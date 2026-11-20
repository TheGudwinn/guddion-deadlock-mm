//! Attachment points of a model (camera, aim and pivot points) and the basis they are written in.
//!
//! Hero models hang their camera points on the root bone, `root_motion`. The game's own models store
//! those points in model axes. Some older skins were exported with the points written in the root
//! bone's own axes, which are turned (x, y, z) -> (y, z, x) compared with the model. With animations
//! that keep the root unturned the camera then sits in the wrong place and does not follow the hero.
//! The fix is a change of basis of every point on the root bone: position and rotation are
//! multiplied by the root bone's rotation. Nothing else in the model is touched.
use super::kv3::{Kv3, Node, Val};

/// Points the camera and the aiming code use. The basis is judged from these.
pub const CAMERA_POINTS: [&str; 13] = [
    "portrait_camera",
    "near_00",
    "near_01",
    "near_02",
    "far_00",
    "far_01",
    "far_02",
    "gunaim_00",
    "gunaim_01",
    "gunaim_02",
    "standing_pivot",
    "crouching_pivot",
    "root_aim",
];

pub const ROOT_BONE: &str = "root_motion";

#[derive(Debug, Clone)]
pub struct Attachment {
    pub name: String,
    pub parent: String,
    pub rot: [f64; 4],
    pub off: [f64; 3],
    rot_val: [Val; 4],
    off_val: [Val; 3],
}

#[derive(Debug, Clone)]
pub struct Rig {
    /// Inverse bind pose of the root bone, 3x4 row-major.
    pub root_bind: [f64; 12],
    pub attachments: Vec<Attachment>,
}

pub fn read_rig(k: &Kv3) -> Result<Rig, String> {
    let tree = k.tree()?;
    let bones = tree.get("m_skeleton").and_then(|s| s.get("m_bones")).ok_or("no skeleton")?;
    let bone = bones.items().iter().find(|b| b.get("m_boneName").and_then(|n| n.str()) == Some(ROOT_BONE)).ok_or("no root_motion bone")?;
    let bind: Vec<f64> = bone.get("m_invBindPose").map(|a| a.items().iter().filter_map(|n| n.val().and_then(|v| k.num(&v))).collect()).unwrap_or_default();
    let root_bind: [f64; 12] = bind.try_into().map_err(|_| "root bind pose is not 12 numbers".to_string())?;
    let mut attachments = Vec::new();
    for item in tree.get("m_attachments").map(|a| a.items()).unwrap_or(&[]) {
        let v = item.get("value").unwrap_or(item);
        let (Some(name), Some(parent)) = (v.get("m_name").and_then(|n| n.str()), v.get("m_influenceNames").and_then(|n| n.items().first()).and_then(|n| n.str())) else { continue };
        let first = |key: &str| v.get(key).and_then(|a| a.items().first()).map(|a| a.items());
        let (Some(r), Some(o)) = (first("m_vInfluenceRotations"), first("m_vInfluenceOffsets")) else { continue };
        let vals = |items: &[super::kv3::Node], n: usize| -> Option<Vec<Val>> {
            let out: Vec<Val> = items.iter().filter_map(|x| x.val()).collect();
            (out.len() == n).then_some(out)
        };
        let (Some(rv), Some(ov)) = (vals(r, 4), vals(o, 3)) else { continue };
        let num = |x: &Val| k.num(x);
        let (Some(rot), Some(off)) = (rv.iter().map(num).collect::<Option<Vec<f64>>>(), ov.iter().map(num).collect::<Option<Vec<f64>>>()) else { continue };
        attachments.push(Attachment {
            name: name.to_string(),
            parent: parent.to_string(),
            rot: rot.try_into().unwrap(),
            off: off.try_into().unwrap(),
            rot_val: rv.try_into().unwrap(),
            off_val: ov.try_into().unwrap(),
        });
    }
    Ok(Rig { root_bind, attachments })
}

/// Rotation of the root bone's axes into model axes (the transpose of the bind rotation).
fn to_model(bind: &[f64; 12]) -> [[f64; 3]; 3] {
    let r = |i: usize, j: usize| bind[i * 4 + j];
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, c) in row.iter_mut().enumerate() {
            *c = r(j, i);
        }
    }
    m
}

fn mul_vec(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// Quaternion (x, y, z, w) of a rotation matrix.
fn quat_of(m: &[[f64; 3]; 3]) -> [f64; 4] {
    let t = m[0][0] + m[1][1] + m[2][2];
    let (x, y, z, w);
    if t > 0.0 {
        let s = (t + 1.0).sqrt() * 2.0;
        w = s / 4.0;
        x = (m[2][1] - m[1][2]) / s;
        y = (m[0][2] - m[2][0]) / s;
        z = (m[1][0] - m[0][1]) / s;
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        w = (m[2][1] - m[1][2]) / s;
        x = s / 4.0;
        y = (m[0][1] + m[1][0]) / s;
        z = (m[0][2] + m[2][0]) / s;
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        w = (m[0][2] - m[2][0]) / s;
        x = (m[0][1] + m[1][0]) / s;
        y = s / 4.0;
        z = (m[1][2] + m[2][1]) / s;
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        w = (m[1][0] - m[0][1]) / s;
        x = (m[0][2] + m[2][0]) / s;
        y = (m[1][2] + m[2][1]) / s;
        z = s / 4.0;
    }
    [x, y, z, w]
}

fn quat_mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let ([ax, ay, az, aw], [bx, by, bz, bw]) = (a, b);
    [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz]
}

fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// Points are in the same axes as the game's own model.
    Same,
    /// Points are written in the root bone's turned axes: the camera does not follow the hero.
    Rotated,
    /// Skeletons differ or the points are not comparable; nothing is judged.
    Unknown,
}

/// Compares the model's camera points with the game's model at the same path.
pub fn basis(model: &Rig, game: &Rig) -> Basis {
    if !close(&model.root_bind, &game.root_bind, 0.01) {
        return Basis::Unknown;
    }
    let m = to_model(&model.root_bind);
    let (mut same, mut rotated) = (0, 0);
    for name in CAMERA_POINTS {
        let find = |r: &'_ Rig| r.attachments.iter().find(|a| a.name == name && a.parent == ROOT_BONE).map(|a| a.off);
        let (Some(a), Some(b)) = (find(model), find(game)) else { continue };
        if close(&a, &b, 0.05) {
            same += 1;
        } else if close(&mul_vec(&m, a), &b, 0.05) {
            rotated += 1;
        }
    }
    match (same, rotated) {
        (0, 0) => Basis::Unknown,
        (_, 0) => Basis::Same,
        (0, _) => Basis::Rotated,
        _ => Basis::Unknown,
    }
}

/// Writes every root-bone point of `k` in model axes. Returns how many points were changed.
pub fn convert_to_model_axes(k: &mut Kv3, rig: &Rig) -> usize {
    let m = to_model(&rig.root_bind);
    let q = quat_of(&m);
    let mut changed = 0;
    for a in rig.attachments.iter().filter(|a| a.parent == ROOT_BONE) {
        if !a.rot_val.iter().chain(a.off_val.iter()).all(|v| k.writable(v)) {
            continue;
        }
        let off = mul_vec(&m, a.off);
        let mut rot = quat_mul(q, a.rot);
        let len = rot.iter().map(|x| x * x).sum::<f64>().sqrt();
        if len > 0.0 {
            rot = rot.map(|x| x / len);
        }
        let ok = a.off_val.iter().zip(off).all(|(v, x)| k.set(v, x)) && a.rot_val.iter().zip(rot).all(|(v, x)| k.set(v, x));
        changed += usize::from(ok);
    }
    changed
}

/// IK target bones (`leg_L_IKTARGET`, ...) are driven by the game's animations in the space of their parent in
/// the game's skeleton. A skin made for an older skeleton hangs them elsewhere (or turned), and the
/// animations then put the legs and arms in the wrong place. Their parent and bind transform are taken from
/// the game's skeleton. These bones carry no mesh, so nothing that is drawn is moved by this.
pub struct IkFix {
    pub name: String,
    new_parent: i32,
    parent_val: Val,
    pos: [f64; 3],
    pos_val: [Val; 3],
    rot: [f64; 4],
    rot_val: [Val; 4],
}

struct Bones {
    names: Vec<String>,
    parents: Vec<i32>,
    parent_vals: Vec<Val>,
    pos: Vec<[f64; 3]>,
    pos_vals: Vec<Vec<Val>>,
    rot: Vec<[f64; 4]>,
    rot_vals: Vec<Vec<Val>>,
}

fn read_bones(k: &Kv3) -> Result<Bones, String> {
    let tree = k.tree()?;
    let ms = tree.get("m_modelSkeleton").ok_or("no model skeleton")?;
    let names: Vec<String> = ms.get("m_boneName").ok_or("no bone names")?.items().iter().map(|n| n.str().unwrap_or("").to_string()).collect();
    let parent_vals: Vec<Val> = ms.get("m_nParent").ok_or("no parents")?.items().iter().filter_map(Node::val).collect();
    let parents = parent_vals.iter().map(|v| k.num(v).map(|x| x as i32).ok_or("parent index")).collect::<Result<Vec<_>, _>>()?;
    let rows = |key: &str, n: usize| -> Result<(Vec<Vec<f64>>, Vec<Vec<Val>>), String> {
        let mut nums = Vec::new();
        let mut valv = Vec::new();
        for item in ms.get(key).ok_or(format!("no {key}"))?.items() {
            let v: Vec<Val> = item.items().iter().filter_map(Node::val).collect();
            let x: Vec<f64> = v.iter().filter_map(|x| k.num(x)).collect();
            if v.len() != n || x.len() != n {
                return Err(format!("{key} entry is not {n} numbers"));
            }
            nums.push(x);
            valv.push(v);
        }
        Ok((nums, valv))
    };
    let (pos, pos_vals) = rows("m_bonePosParent", 3)?;
    let (rot, rot_vals) = rows("m_boneRotParent", 4)?;
    if [parents.len(), pos.len(), rot.len()].iter().any(|n| *n != names.len()) {
        return Err("skeleton channels differ in length".into());
    }
    Ok(Bones {
        names,
        parents,
        parent_vals,
        pos: pos.into_iter().map(|v| [v[0], v[1], v[2]]).collect(),
        pos_vals,
        rot: rot.into_iter().map(|v| [v[0], v[1], v[2], v[3]]).collect(),
        rot_vals,
    })
}

fn is_ik_target(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with("_iktarget")
}

/// IK target bones of `model` whose parent or bind transform is not the game's.
pub fn ik_fixes(model: &Kv3, game: &Kv3) -> Result<Vec<IkFix>, String> {
    let (m, g) = (read_bones(model)?, read_bones(game)?);
    let parent_name = |b: &Bones, i: usize| (b.parents[i] >= 0).then(|| b.names.get(b.parents[i] as usize).cloned()).flatten();
    let mut out = Vec::new();
    for (i, name) in m.names.iter().enumerate().filter(|(_, n)| is_ik_target(n)) {
        let Some(j) = g.names.iter().position(|n| n == name) else { continue };
        let Some(gp) = parent_name(&g, j) else { continue };
        let Some(new_parent) = m.names.iter().position(|n| *n == gp) else { continue };
        let same_parent = parent_name(&m, i).as_deref() == Some(gp.as_str());
        let dot: f64 = m.rot[i].iter().zip(&g.rot[j]).map(|(a, b)| a * b).sum();
        // q and -q are the same rotation
        let same_rot = dot.abs() > 0.9999;
        let same_pos = close(&m.pos[i], &g.pos[j], 0.01);
        if same_parent && same_rot && same_pos {
            continue;
        }
        // only what can be rewritten in place counts; the rest is left as the mod made it
        if m.pos_vals[i].len() != 3 || m.rot_vals[i].len() != 4 || !model.writable_i32(&m.parent_vals[i]) || !m.pos_vals[i].iter().chain(&m.rot_vals[i]).all(|v| model.writable(v)) {
            continue;
        }
        out.push(IkFix {
            name: name.clone(),
            new_parent: new_parent as i32,
            parent_val: m.parent_vals[i],
            pos: g.pos[j],
            pos_val: [m.pos_vals[i][0], m.pos_vals[i][1], m.pos_vals[i][2]],
            rot: g.rot[j],
            rot_val: [m.rot_vals[i][0], m.rot_vals[i][1], m.rot_vals[i][2], m.rot_vals[i][3]],
        });
    }
    Ok(out)
}

/// Applies the fixes; the number of bones changed.
pub fn apply_ik_fixes(k: &mut Kv3, fixes: &[IkFix]) -> usize {
    let mut changed = 0;
    for f in fixes {
        if !k.writable_i32(&f.parent_val) || !f.pos_val.iter().chain(f.rot_val.iter()).all(|v| k.writable(v)) {
            continue;
        }
        let ok = k.set_i32(&f.parent_val, f.new_parent) && f.pos_val.iter().zip(f.pos).all(|(v, x)| k.set(v, x)) && f.rot_val.iter().zip(f.rot).all(|(v, x)| k.set(v, x));
        changed += usize::from(ok);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIND: [f64; 12] = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.5];

    #[test]
    fn turned_axes_become_model_axes() {
        let m = to_model(&BIND);
        // what the game stores as (-18.8, -20.2, 87.4) is written (-20.2, 87.4, -18.8) in the turned axes
        let v = mul_vec(&m, [-20.16, 87.36, -18.82]);
        assert!(close(&v, &[-18.82, -20.16, 87.36], 1e-9), "{v:?}");
        let q = quat_of(&m);
        assert!(close(&q, &[0.5, 0.5, 0.5, 0.5], 1e-9), "{q:?}");
        // the turned rotation of such a point is cancelled
        let r = quat_mul(q, [-0.5, -0.5, -0.5, 0.5]);
        assert!(close(&r, &[0.0, 0.0, 0.0, 1.0], 1e-9), "{r:?}");
    }

    fn rig(off: [f64; 3]) -> Rig {
        let v = Val { ty: 19, loc: None };
        Rig {
            root_bind: BIND,
            attachments: vec![Attachment { name: "near_00".into(), parent: ROOT_BONE.into(), rot: [0.0, 0.0, 0.0, 1.0], off, rot_val: [v; 4], off_val: [v; 3] }],
        }
    }

    #[test]
    fn basis_is_judged_from_camera_points() {
        let game = rig([-18.82, -20.16, 87.36]);
        assert_eq!(basis(&rig([-18.82, -20.16, 87.36]), &game), Basis::Same);
        assert_eq!(basis(&rig([-20.16, 87.36, -18.82]), &game), Basis::Rotated);
        assert_eq!(basis(&rig([1.0, 2.0, 3.0]), &game), Basis::Unknown);
        let mut other = rig([-18.82, -20.16, 87.36]);
        other.root_bind[11] = 9.0;
        assert_eq!(basis(&other, &game), Basis::Unknown);
    }
}
