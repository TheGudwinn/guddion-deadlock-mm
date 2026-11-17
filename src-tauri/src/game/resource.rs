//! Just enough of the Source 2 compiled-resource format to look inside a model.
//!
//! A compiled resource (`.vmdl_c` …) is a small header, a table of blocks and the blocks themselves.
//! The `DATA` block of a model is a binary KV3 document; in the version the game uses today (v5)
//! it holds two LZ4 buffers. The first one starts with the string table, which is all the checks need.
//! Nothing here writes anything.

const MAX_BUFFER: usize = 128 << 20;
const KV3_V5: [u8; 4] = [0x05, 0x33, 0x56, 0x4B];
const LZ4: u32 = 1;
/// Fixed v5 header: magic, format GUID, method, dictionary, frame size, counts, sizes.
const V5_SIZES_AT: usize = 72;
const V5_STREAM_AT: usize = 120;

/// The `DATA` block of a compiled resource.
pub fn data_block(res: &[u8]) -> Option<&[u8]> {
    block(res, *b"DATA")
}

/// A block by its four-letter name.
pub fn block(res: &[u8], name: [u8; 4]) -> Option<&[u8]> {
    let count = u32::from_le_bytes(res.get(12..16)?.try_into().ok()?) as usize;
    let table = 8usize.checked_add(u32::from_le_bytes(res.get(8..12)?.try_into().ok()?) as usize)?;
    if count > 4096 {
        return None;
    }
    for i in 0..count {
        let at = table.checked_add(i.checked_mul(12)?)?;
        let head = res.get(at..at + 12)?;
        if head[..4] == name {
            let off = u32::from_le_bytes(head[4..8].try_into().ok()?) as usize;
            let size = u32::from_le_bytes(head[8..12].try_into().ok()?) as usize;
            let start = at.checked_add(4)?.checked_add(off)?;
            return res.get(start..start.checked_add(size)?);
        }
    }
    None
}

/// Why a model could not be looked into. Not an error of the mod, only a limit of this reader.
#[derive(Debug, PartialEq, Eq)]
pub enum Unreadable {
    NotKv3V5,
    Compression(u32),
    Damaged(String),
}

impl std::fmt::Display for Unreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unreadable::NotKv3V5 => write!(f, "other KV3 version"),
            Unreadable::Compression(m) => write!(f, "compression method {m}"),
            Unreadable::Damaged(s) => write!(f, "damaged data: {s}"),
        }
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The first decompressed buffer of a KV3 v5 document: the strings used by the document come first.
pub fn kv3_strings(data: &[u8]) -> Result<Vec<u8>, Unreadable> {
    if data.get(..4) != Some(&KV3_V5[..]) {
        return Err(Unreadable::NotKv3V5);
    }
    let bad = || Unreadable::Damaged("header".into());
    let method = u32_at(data, 20).ok_or_else(bad)?;
    if method != LZ4 {
        return Err(Unreadable::Compression(method));
    }
    let raw = u32_at(data, V5_SIZES_AT).ok_or_else(bad)? as usize;
    let packed = u32_at(data, V5_SIZES_AT + 4).ok_or_else(bad)? as usize;
    if raw == 0 || raw > MAX_BUFFER || packed > MAX_BUFFER {
        return Err(bad());
    }
    let src = data.get(V5_STREAM_AT..V5_STREAM_AT.checked_add(packed).ok_or_else(bad)?).ok_or_else(bad)?;
    lz4_block(src, raw).map_err(Unreadable::Damaged)
}

/// Plain LZ4 block decoder with every length and offset checked.
pub fn lz4_block(src: &[u8], out_len: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(out_len);
    let mut i = 0usize;
    let ext = |i: &mut usize, mut n: usize| -> Result<usize, String> {
        loop {
            let b = *src.get(*i).ok_or("lz4: truncated length")?;
            *i += 1;
            n += b as usize;
            if b != 255 {
                return Ok(n);
            }
        }
    };
    while i < src.len() {
        let token = src[i];
        i += 1;
        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            lit = ext(&mut i, lit)?;
        }
        let end = i.checked_add(lit).filter(|e| *e <= src.len()).ok_or("lz4: literal runs past the end")?;
        if out.len() + lit > out_len {
            return Err("lz4: output longer than declared".into());
        }
        out.extend_from_slice(&src[i..end]);
        i = end;
        if i >= src.len() {
            break;
        }
        let off = *src.get(i).ok_or("lz4: truncated offset")? as usize | (*src.get(i + 1).ok_or("lz4: truncated offset")? as usize) << 8;
        i += 2;
        if off == 0 || off > out.len() {
            return Err("lz4: bad match offset".into());
        }
        let mut m = (token & 15) as usize;
        if m == 15 {
            m = ext(&mut i, m)?;
        }
        m += 4;
        if out.len() + m > out_len {
            return Err("lz4: output longer than declared".into());
        }
        for _ in 0..m {
            out.push(out[out.len() - off]);
        }
    }
    if out.len() != out_len {
        return Err(format!("lz4: got {} bytes, expected {out_len}", out.len()));
    }
    Ok(out)
}

/// LZ4 block made only of literals. Larger than a real compression, but valid for every decoder.
pub fn lz4_store(src: &[u8]) -> Vec<u8> {
    let n = src.len();
    let mut out = Vec::with_capacity(n + n / 255 + 16);
    if n < 15 {
        out.push((n as u8) << 4);
    } else {
        out.push(0xF0);
        let mut rest = n - 15;
        while rest >= 255 {
            out.push(255);
            rest -= 255;
        }
        out.push(rest as u8);
    }
    out.extend_from_slice(src);
    out
}

fn align(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}

/// A KV3 v5 `DATA` block taken apart far enough to replace the text of the model's key values
/// (`m_modelInfo.m_keyValueText`), which is where hero models keep camera and scene settings.
struct Doc<'a> {
    data: &'a [u8],
    nb: usize,
    ni: usize,
    ne: usize,
    b1: Vec<u8>,
    c2: &'a [u8],
    u2: usize,
}

const KV_START: &[u8] = b"<!-- kv3 encoding:text:";
const KV_FORMAT: &[u8] = b"format:vmdlkeys";

impl<'a> Doc<'a> {
    fn open(data: &'a [u8]) -> Result<Doc<'a>, String> {
        let at = |o: usize| u32_at(data, o).map(|v| v as usize).ok_or_else(|| "header is cut short".to_string());
        if data.get(..4) != Some(&KV3_V5[..]) {
            return Err("not a KV3 v5 document".into());
        }
        if at(20)? as u32 != LZ4 {
            return Err(format!("compression method {}", at(20)?));
        }
        if at(56)? != 0 || at(60)? != 0 {
            return Err("document has binary blobs".into());
        }
        let (nb, ni, ne) = (at(28)?, at(32)?, at(36)?);
        let (u1, c1, u2, c2) = (at(72)?, at(76)?, at(80)?, at(84)?);
        if [u1, u2, nb, ni, ne].iter().any(|v| *v > MAX_BUFFER) || u1 == 0 {
            return Err("implausible sizes".into());
        }
        if data.len() != V5_STREAM_AT + c1 + c2 {
            return Err("unexpected data after the buffers".into());
        }
        let b1 = lz4_block(&data[V5_STREAM_AT..V5_STREAM_AT + c1], u1)?;
        let ints = align(nb, 4);
        if align(ints + ni * 4, 8) + ne * 8 != u1 {
            return Err("unexpected buffer layout".into());
        }
        Ok(Doc { data, nb, ni, ne, b1, c2: &data[V5_STREAM_AT + c1..], u2 })
    }

    /// (start, end) of the key-values string inside the first buffer.
    fn text_span(&self) -> Result<(usize, usize), String> {
        let ints = align(self.nb, 4);
        let count = i32::from_le_bytes(self.b1.get(ints..ints + 4).ok_or("no string table")?.try_into().unwrap());
        if count < 0 || count as usize > self.nb {
            return Err("bad string count".into());
        }
        let (mut p, mut found) = (0usize, None);
        for _ in 0..count {
            let rest = self.b1.get(p..self.nb).ok_or("strings run past their buffer")?;
            let n = rest.iter().position(|&c| c == 0).ok_or("unterminated string")?;
            let s = &rest[..n];
            if s.starts_with(KV_START) && contains(&s[..s.len().min(400)], KV_FORMAT) {
                if found.is_some() {
                    return Err("more than one key-values text".into());
                }
                found = Some((p, p + n));
            }
            p += n + 1;
        }
        found.ok_or_else(|| "no key-values text".to_string())
    }

    fn text(&self) -> Result<String, String> {
        let (s, e) = self.text_span()?;
        String::from_utf8(self.b1[s..e].to_vec()).map_err(|_| "key-values text is not UTF-8".to_string())
    }

    fn with_text(&self, new: &str) -> Result<Vec<u8>, String> {
        let (s, e) = self.text_span()?;
        let ints = align(self.nb, 4);
        let eights = align(ints + self.ni * 4, 8);
        let mut b = Vec::with_capacity(self.b1.len() + new.len());
        b.extend_from_slice(&self.b1[..s]);
        b.extend_from_slice(new.as_bytes());
        b.extend_from_slice(&self.b1[e..self.nb]);
        let nb = b.len();
        b.resize(align(nb, 4), 0);
        b.extend_from_slice(&self.b1[ints..ints + self.ni * 4]);
        b.resize(align(b.len(), 8), 0);
        b.extend_from_slice(&self.b1[eights..eights + self.ne * 8]);
        let packed = lz4_store(&b);
        let mut out = self.data[..V5_STREAM_AT].to_vec();
        let put = |out: &mut Vec<u8>, at: usize, v: usize| out[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
        put(&mut out, 28, nb);
        put(&mut out, 48, b.len() + self.u2);
        put(&mut out, 52, packed.len() + self.c2.len());
        put(&mut out, 72, b.len());
        put(&mut out, 76, packed.len());
        out.extend_from_slice(&packed);
        out.extend_from_slice(self.c2);
        Ok(out)
    }
}

/// The key-values text of a model's `DATA` block (camera, scene and other settings written as text).
pub fn key_values(data: &[u8]) -> Result<String, String> {
    Doc::open(data)?.text()
}

/// A new `DATA` block that is the same as `data` except for the key-values text.
pub fn with_key_values(data: &[u8], text: &str) -> Result<Vec<u8>, String> {
    Doc::open(data)?.with_text(text)
}

/// The resource file with its `DATA` block swapped for `new`. Later blocks keep their 16-byte alignment.
pub fn replace_data_block(res: &[u8], new: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "resource header".to_string();
    let count = u32_at(res, 12).ok_or_else(bad)? as usize;
    let table = 8 + u32_at(res, 8).ok_or_else(bad)? as usize;
    if count > 4096 {
        return Err(bad());
    }
    let entry = |i: usize| table + i * 12;
    let (mut idx, mut start, mut size) = (None, 0usize, 0usize);
    for i in 0..count {
        let at = entry(i);
        let head = res.get(at..at + 12).ok_or_else(bad)?;
        if head[..4] == *b"DATA" {
            start = at + 4 + u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
            size = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
            idx = Some(i);
        }
    }
    let idx = idx.ok_or("no DATA block")?;
    let end = start.checked_add(size).filter(|e| *e <= res.len()).ok_or_else(bad)?;
    let tail = &res[end..];
    let mut pad = 0;
    if !tail.is_empty() {
        pad = (16 - (new.len() as i64 - size as i64).rem_euclid(16) as usize) % 16;
    }
    let shift = new.len() as i64 + pad as i64 - size as i64;
    let mut out = Vec::with_capacity(res.len() + new.len());
    out.extend_from_slice(&res[..start]);
    out.extend_from_slice(new);
    out.resize(out.len() + pad, 0);
    out.extend_from_slice(tail);
    out[entry(idx) + 8..entry(idx) + 12].copy_from_slice(&(new.len() as u32).to_le_bytes());
    for i in 0..count {
        let at = entry(i);
        let off = u32::from_le_bytes(res[at + 4..at + 8].try_into().unwrap()) as usize;
        if at + 4 + off > start && i != idx {
            let moved = (off as i64 + shift) as u32;
            out[at + 4..at + 8].copy_from_slice(&moved.to_le_bytes());
        }
    }
    let total = out.len() as u32;
    out[0..4].copy_from_slice(&total.to_le_bytes());
    Ok(out)
}

pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lz4_literals_and_matches() {
        assert_eq!(lz4_block(&[0x50, b'h', b'e', b'l', b'l', b'o'], 5).unwrap(), b"hello");
        // "abc" then a match of 5 bytes at distance 3 ("abcab") and nothing after it
        let src = [0x31, b'a', b'b', b'c', 3, 0];
        assert_eq!(lz4_block(&src, 8).unwrap(), b"abcabcab");
    }

    #[test]
    fn lz4_refuses_nonsense() {
        assert!(lz4_block(&[0x50, b'h'], 5).is_err());
        assert!(lz4_block(&[0x31, b'a', b'b', b'c', 9, 0], 8).is_err());
        assert!(lz4_block(&[0x50, 1, 2, 3, 4, 5], 4).is_err());
        assert!(lz4_block(&[0x31, b'a', b'b', b'c', 3, 0], 7).is_err());
    }

    #[test]
    fn finds_blocks() {
        // header: size, hver, ver, blockoffset(=8 -> table at 16), count 1
        let mut r = vec![0u8; 16];
        r[8..12].copy_from_slice(&8u32.to_le_bytes());
        r[12..16].copy_from_slice(&1u32.to_le_bytes());
        r.extend_from_slice(b"DATA");
        r.extend_from_slice(&8u32.to_le_bytes()); // relative to this field (at 20) -> 28
        r.extend_from_slice(&3u32.to_le_bytes());
        r.extend_from_slice(b"xyz");
        assert_eq!(data_block(&r), Some(&b"xyz"[..]));
        assert_eq!(block(&r, *b"NOPE"), None);
        assert_eq!(data_block(&r[..10]), None);
    }

    #[test]
    fn stored_lz4_round_trips() {
        for n in [1usize, 14, 15, 16, 269, 270, 271, 5000] {
            let src: Vec<u8> = (0..n).map(|i| (i * 7 % 251) as u8).collect();
            assert_eq!(lz4_block(&lz4_store(&src), n).unwrap(), src, "n={n}");
        }
    }

    #[test]
    fn data_block_swap_keeps_later_blocks_in_place() {
        // blocks: DATA (5 bytes), then TAIL (3 bytes) starting at the next 16-byte boundary
        let mut r = vec![0u8; 16];
        r[8..12].copy_from_slice(&8u32.to_le_bytes());
        r[12..16].copy_from_slice(&2u32.to_le_bytes());
        // entry 0 at 16: offset field at 20 -> data at 40; entry 1 at 28: offset field at 32 -> tail at 48
        r.extend_from_slice(b"DATA");
        r.extend_from_slice(&20u32.to_le_bytes());
        r.extend_from_slice(&5u32.to_le_bytes());
        r.extend_from_slice(b"TAIL");
        r.extend_from_slice(&16u32.to_le_bytes());
        r.extend_from_slice(&3u32.to_le_bytes());
        assert_eq!(r.len(), 40);
        r.extend_from_slice(b"12345");
        r.resize(48, 0);
        r.extend_from_slice(b"xyz");
        assert_eq!(data_block(&r), Some(&b"12345"[..]));
        let out = replace_data_block(&r, b"0123456789ABCDEFG").unwrap();
        assert_eq!(data_block(&out), Some(&b"0123456789ABCDEFG"[..]));
        assert_eq!(block(&out, *b"TAIL"), Some(&b"xyz"[..]));
        assert_eq!(u32::from_le_bytes(out[0..4].try_into().unwrap()) as usize, out.len());
        assert_eq!((out.len() - 3) % 16, 0);
    }

    #[test]
    fn other_kv3_versions_are_reported_not_misread() {
        assert_eq!(kv3_strings(&[4, 0x33, 0x56, 0x4B, 0, 0]), Err(Unreadable::NotKv3V5));
    }
}
