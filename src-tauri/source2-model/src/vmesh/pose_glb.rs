use serde_json::Value;

use crate::error::{Result, Source2Error};
use crate::skeleton::BoneTransform;
use crate::skeleton::Skeleton;

use super::ModelGlb;

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| Source2Error::Decode("GLB header is truncated".into()))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn pad_four(bytes: &mut Vec<u8>, value: u8) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(value);
    }
}

/// Replace the GLB skeleton's local bind transforms with a sampled pose.
/// Geometry, materials and skin weights remain untouched, so the browser and
/// the native renderer evaluate exactly the same skinning inputs.
pub fn pose_model_glb(
    model: &ModelGlb,
    skeleton: &Skeleton,
    pose: &[BoneTransform],
) -> Result<ModelGlb> {
    if u32_at(&model.glb, 0)? != 0x4654_6c67 || u32_at(&model.glb, 4)? != 2 {
        return Err(Source2Error::Decode("invalid GLB header".into()));
    }
    let json_len = u32_at(&model.glb, 12)? as usize;
    if u32_at(&model.glb, 16)? != 0x4e4f_534a {
        return Err(Source2Error::Decode("GLB has no JSON chunk".into()));
    }
    let json_end = 20usize
        .checked_add(json_len)
        .filter(|end| *end <= model.glb.len())
        .ok_or_else(|| Source2Error::Decode("GLB JSON chunk is truncated".into()))?;
    let bin_len = u32_at(&model.glb, json_end)? as usize;
    if u32_at(&model.glb, json_end + 4)? != 0x004e_4942 {
        return Err(Source2Error::Decode("GLB has no BIN chunk".into()));
    }
    let bin_start = json_end + 8;
    let bin = model
        .glb
        .get(bin_start..bin_start + bin_len)
        .ok_or_else(|| Source2Error::Decode("GLB BIN chunk is truncated".into()))?;

    let mut document: Value = serde_json::from_slice(&model.glb[20..json_end])
        .map_err(|error| Source2Error::Decode(format!("invalid GLB JSON: {error}")))?;
    let nodes = document
        .get_mut("nodes")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| Source2Error::Decode("GLB has no nodes".into()))?;
    for (bone, transform) in skeleton.bones.iter().zip(pose) {
        let Some(node) = nodes
            .iter_mut()
            .find(|node| node.get("name").and_then(Value::as_str) == Some(bone.name.as_str()))
        else {
            continue;
        };
        node["translation"] = serde_json::json!(transform.translation);
        node["rotation"] = serde_json::json!(transform.rotation);
        node["scale"] = serde_json::json!(transform.scale);
    }

    let mut json = serde_json::to_vec(&document)
        .map_err(|error| Source2Error::Decode(format!("GLB JSON encode failed: {error}")))?;
    pad_four(&mut json, b' ');
    let total_len = 12 + 8 + json.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total_len);
    glb.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_len as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004e_4942u32.to_le_bytes());
    glb.extend_from_slice(bin);
    Ok(ModelGlb {
        vertex_count: model.vertex_count,
        index_count: model.index_count,
        glb,
    })
}

fn same(a: &[f32], b: &[f32], eps: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= eps)
}

/// Pose the skeleton with the first frame and add an animation that plays all the frames.
/// `frames[i]` is the pose at `times[i]` seconds; only bones that actually move get channels,
/// and the first frame is repeated at `end` so the loop closes without a jump.
pub fn animate_model_glb(
    model: &ModelGlb,
    skeleton: &Skeleton,
    frames: &[Vec<BoneTransform>],
    times: &[f32],
    end: f32,
) -> Result<ModelGlb> {
    let posed = pose_model_glb(model, skeleton, &frames[0])?;
    if frames.len() < 2 {
        return Ok(posed);
    }
    let glb = &posed.glb;
    let json_len = u32_at(glb, 12)? as usize;
    let json_end = 20 + json_len;
    let bin_len = u32_at(glb, json_end)? as usize;
    let mut bin = glb[json_end + 8..json_end + 8 + bin_len].to_vec();
    let mut document: Value = serde_json::from_slice(&glb[20..json_end])
        .map_err(|error| Source2Error::Decode(format!("invalid GLB JSON: {error}")))?;

    let mut node_of = std::collections::HashMap::new();
    if let Some(nodes) = document.get("nodes").and_then(Value::as_array) {
        for (index, node) in nodes.iter().enumerate() {
            if let Some(name) = node.get("name").and_then(Value::as_str) {
                node_of.entry(name.to_string()).or_insert(index);
            }
        }
    }

    let mut all_times: Vec<f32> = times.to_vec();
    all_times.push(end.max(times.last().copied().unwrap_or(0.0) + 1e-3));
    let count = all_times.len();

    pad_four(&mut bin, 0);
    let mut views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();
    let base_view = document.get("bufferViews").and_then(Value::as_array).map_or(0, Vec::len);
    let base_accessor = document.get("accessors").and_then(Value::as_array).map_or(0, Vec::len);
    let mut push = |bin: &mut Vec<u8>, data: &[f32], kind: &str, extra: Option<(f32, f32)>| -> usize {
        let offset = bin.len();
        for value in data {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        views.push(serde_json::json!({"buffer": 0, "byteOffset": offset, "byteLength": data.len() * 4}));
        let mut accessor = serde_json::json!({
            "bufferView": base_view + views.len() - 1, "componentType": 5126, "count": count, "type": kind
        });
        if let Some((min, max)) = extra {
            accessor["min"] = serde_json::json!([min]);
            accessor["max"] = serde_json::json!([max]);
        }
        accessors.push(accessor);
        base_accessor + accessors.len() - 1
    };
    let time_accessor = push(&mut bin, &all_times, "SCALAR", Some((all_times[0], *all_times.last().unwrap())));

    let mut channels = Vec::new();
    let mut samplers = Vec::new();
    for (bone_index, bone) in skeleton.bones.iter().enumerate() {
        let Some(&node) = node_of.get(bone.name.as_str()) else { continue };
        let Some(first) = frames[0].get(bone_index) else { continue };
        let moves = frames.iter().skip(1).any(|f| {
            f.get(bone_index).is_some_and(|t| {
                !same(&t.translation, &first.translation, 1e-4) || !same(&t.rotation, &first.rotation, 1e-5)
            })
        });
        if !moves {
            continue;
        }
        let at = |i: usize| frames[i.min(frames.len() - 1)].get(bone_index).unwrap_or(first);
        let mut translation = Vec::with_capacity(count * 3);
        let mut rotation: Vec<f32> = Vec::with_capacity(count * 4);
        for i in 0..count {
            let t = at(if i == count - 1 { 0 } else { i });
            translation.extend_from_slice(&t.translation);
            // keep neighbouring quaternions on the same side so interpolation takes the short way
            let mut q = t.rotation;
            if let Some(prev) = rotation.chunks(4).last() {
                if prev.iter().zip(&q).map(|(a, b)| a * b).sum::<f32>() < 0.0 {
                    q = [-q[0], -q[1], -q[2], -q[3]];
                }
            }
            rotation.extend_from_slice(&q);
        }
        let ta = push(&mut bin, &translation, "VEC3", None);
        let ra = push(&mut bin, &rotation, "VEC4", None);
        for (path, output) in [("translation", ta), ("rotation", ra)] {
            samplers.push(serde_json::json!({"input": time_accessor, "output": output, "interpolation": "LINEAR"}));
            channels.push(serde_json::json!({"sampler": samplers.len() - 1, "target": {"node": node, "path": path}}));
        }
    }
    if channels.is_empty() {
        return Ok(posed);
    }

    let extend = |document: &mut Value, key: &str, items: Vec<Value>| {
        if let Some(array) = document.get_mut(key).and_then(Value::as_array_mut) {
            array.extend(items);
        } else {
            document[key] = Value::Array(items);
        }
    };
    extend(&mut document, "bufferViews", views);
    extend(&mut document, "accessors", accessors);
    extend(&mut document, "animations", vec![serde_json::json!({"name": "idle", "channels": channels, "samplers": samplers})]);
    document["buffers"][0]["byteLength"] = serde_json::json!(bin.len());

    let mut json = serde_json::to_vec(&document)
        .map_err(|error| Source2Error::Decode(format!("GLB JSON encode failed: {error}")))?;
    pad_four(&mut json, b' ');
    let total_len = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total_len as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004e_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    Ok(ModelGlb { vertex_count: model.vertex_count, index_count: model.index_count, glb: out })
}
