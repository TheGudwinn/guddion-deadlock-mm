//! Reader for the VPK directory index (versions 1 and 2). File data is never read.
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

const SIGNATURE: u32 = 0x55AA_1234;
const EMBEDDED: u16 = 0x7FFF;
const MAX_TREE: u32 = 64 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpkFile {
    pub path: String,
    pub crc: u32,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct VpkInfo {
    pub version: u32,
    pub files: Vec<VpkFile>,
    /// True when some entries live in separate `_NNN.vpk` chunk files.
    pub external_archives: bool,
}

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.p.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("unexpected end of tree")?;
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn cstr(&mut self) -> Result<String, String> {
        let rest = &self.b[self.p..];
        let n = rest.iter().position(|&c| c == 0).ok_or("unterminated string")?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.p += n + 1;
        Ok(s)
    }
}

pub fn read_vpk_index(path: &Path) -> Result<VpkInfo, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).map_err(|_| "file too small".to_string())?;
    let sig = u32::from_le_bytes(head[0..4].try_into().unwrap());
    let version = u32::from_le_bytes(head[4..8].try_into().unwrap());
    let tree_size = u32::from_le_bytes(head[8..12].try_into().unwrap());
    if sig != SIGNATURE {
        return Err("bad signature (not a VPK)".into());
    }
    match version {
        1 => {}
        2 => {
            let mut extra = [0u8; 16];
            f.read_exact(&mut extra).map_err(|_| "truncated v2 header".to_string())?;
        }
        v => return Err(format!("unsupported VPK version {v}")),
    }
    if tree_size == 0 || tree_size > MAX_TREE {
        return Err(format!("implausible tree size {tree_size}"));
    }
    let mut tree = vec![0u8; tree_size as usize];
    f.read_exact(&mut tree).map_err(|_| "truncated directory tree".to_string())?;

    let mut c = Cur { b: &tree, p: 0 };
    let mut files = Vec::new();
    let mut external = false;
    loop {
        let ext = c.cstr()?;
        if ext.is_empty() {
            break;
        }
        loop {
            let dir = c.cstr()?;
            if dir.is_empty() {
                break;
            }
            loop {
                let name = c.cstr()?;
                if name.is_empty() {
                    break;
                }
                let crc = c.u32()?;
                let preload = c.u16()? as u64;
                let archive = c.u16()?;
                let _offset = c.u32()?;
                let len = c.u32()? as u64;
                if c.u16()? != 0xFFFF {
                    return Err("bad entry terminator".into());
                }
                c.take(preload as usize)?;
                if archive != EMBEDDED && len > 0 {
                    external = true;
                }
                let mut p = String::new();
                if dir != " " {
                    p.push_str(&dir);
                    p.push('/');
                }
                p.push_str(&name);
                if ext != " " {
                    p.push('.');
                    p.push_str(&ext);
                }
                files.push(VpkFile { path: p, crc, size: preload + len });
            }
        }
    }
    Ok(VpkInfo { version, files, external_archives: external })
}

/// Build a minimal v2 VPK with all data embedded. Used by tests and fixtures.
#[doc(hidden)]
pub fn build_vpk(files: &[(&str, &[u8])]) -> Vec<u8> {
    // ext -> dir -> [(name, data)]
    let mut tree: BTreeMap<String, BTreeMap<String, Vec<(String, &[u8])>>> = BTreeMap::new();
    for (path, data) in files {
        let (dir, file) = path.rsplit_once('/').unwrap_or((" ", path));
        let (name, ext) = file.rsplit_once('.').unwrap_or((file, " "));
        tree.entry(ext.to_string()).or_default().entry(dir.to_string()).or_default().push((name.to_string(), *data));
    }
    let mut t = Vec::new();
    let mut data_section = Vec::new();
    let put = |t: &mut Vec<u8>, s: &str| {
        t.extend_from_slice(s.as_bytes());
        t.push(0);
    };
    for (ext, dirs) in &tree {
        put(&mut t, ext);
        for (dir, names) in dirs {
            put(&mut t, dir);
            for (name, data) in names {
                put(&mut t, name);
                t.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
                t.extend_from_slice(&0u16.to_le_bytes());
                t.extend_from_slice(&EMBEDDED.to_le_bytes());
                t.extend_from_slice(&(data_section.len() as u32).to_le_bytes());
                t.extend_from_slice(&(data.len() as u32).to_le_bytes());
                t.extend_from_slice(&0xFFFFu16.to_le_bytes());
                data_section.extend_from_slice(data);
            }
            t.push(0);
        }
        t.push(0);
    }
    t.push(0);
    let mut out = Vec::new();
    out.extend_from_slice(&SIGNATURE.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(t.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data_section.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 12]); // md5 sections + signature section sizes
    out.extend_from_slice(&t);
    out.extend_from_slice(&data_section);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_paths_and_crc() {
        let bytes = build_vpk(&[("materials/a/b.vmat_c", b"hello"), ("top.txt", b"x")]);
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("pak01_dir.vpk");
        std::fs::write(&p, bytes).unwrap();
        let info = read_vpk_index(&p).unwrap();
        assert_eq!(info.version, 2);
        assert!(!info.external_archives);
        let mut paths: Vec<_> = info.files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, ["materials/a/b.vmat_c", "top.txt"]);
        assert_eq!(info.files.iter().find(|f| f.path == "top.txt").unwrap().crc, crc32fast::hash(b"x"));
    }

    #[test]
    fn rejects_garbage_and_truncation() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.vpk");
        std::fs::write(&p, b"not a vpk at all, definitely").unwrap();
        assert!(read_vpk_index(&p).is_err());
        let mut v = build_vpk(&[("a.txt", b"1")]);
        v.truncate(34);
        std::fs::write(&p, v).unwrap();
        assert!(read_vpk_index(&p).is_err());
    }
}
