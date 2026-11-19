//! Reader for binary KV3 v5 documents (the `DATA` and `MDAT` blocks of compiled models), able to
//! change a few numbers in place.
//!
//! The document is two buffers, each stored, LZ4- or zstd-compressed. The first one holds the string table and the bulk arrays of numbers
//! ("auxiliary" lanes), the second holds the tree: a table of object sizes, four value lanes
//! (bytes, 16-bit, 32-bit, 64-bit) and the type stream. The reader walks the tree the way the game
//! does and remembers where each value lives, so a number can be rewritten without rebuilding anything.
//! Only values in the first buffer are rewritable; that is where model attachments keep their floats.
use super::resource::{pack, unpack, ZSTD};

const MAX_BUFFER: usize = 128 << 20;
const STREAM_AT: usize = 120;

// type codes
const NULL: u8 = 1;
const BOOL: u8 = 2;
const I64: u8 = 3;
const U64: u8 = 4;
const DBL: u8 = 5;
const STR: u8 = 6;
const ARR: u8 = 8;
const OBJ: u8 = 9;
const ARRT: u8 = 10;
const I32: u8 = 11;
const U32: u8 = 12;
const TRUE: u8 = 13;
const FALSE: u8 = 14;
const I64_0: u8 = 15;
const I64_1: u8 = 16;
const D0: u8 = 17;
const D1: u8 = 18;
const FLT: u8 = 19;
const I16: u8 = 20;
const U16: u8 = 21;
const I32B: u8 = 23;
const ARRB: u8 = 24;
const ARRA: u8 = 25; // array whose numbers live in the auxiliary lanes
const F0: u8 = 26;
const F1: u8 = 27;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loc {
    pub aux: bool,
    /// 0 bytes, 1 shorts, 2 ints, 3 eights
    pub lane: u8,
    /// byte offset inside the lane
    pub off: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Val {
    pub ty: u8,
    pub loc: Option<Loc>,
}

#[derive(Clone, Debug)]
pub enum Node {
    Obj(Vec<(String, Node)>),
    Arr(Vec<Node>),
    Str(String),
    Val(Val),
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn items(&self) -> &[Node] {
        match self {
            Node::Arr(a) => a,
            _ => &[],
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn val(&self) -> Option<Val> {
        match self {
            Node::Val(v) => Some(*v),
            _ => None,
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Pos {
    b: usize,
    s: usize,
    i: usize,
    e: usize,
}

/// Where each lane starts inside its buffer and how long it is.
#[derive(Default, Clone, Copy)]
struct Lanes {
    b: (usize, usize),
    s: (usize, usize),
    i: (usize, usize),
    e: (usize, usize),
}

fn align(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}

fn lanes(start: usize, nb: usize, ns: usize, ni: usize, ne: usize) -> Lanes {
    let b = (start, nb);
    let s = (align(b.0 + b.1, 2), ns * 2);
    let i = (align(s.0 + s.1, 4), ni * 4);
    let e = (align(i.0 + i.1, 8), ne * 8);
    Lanes { b, s, i, e }
}

pub struct Kv3 {
    head: Vec<u8>,
    method: u32,
    b1: Vec<u8>,
    b2: Vec<u8>,
    c2: Vec<u8>,
    l1: Lanes,
    l2: Lanes,
    strings: Vec<String>,
    aux_start: Pos,
    objcounts: Vec<i32>,
    /// Where the type stream may start: after the aligned or the unaligned end of the lanes.
    types: Vec<(usize, usize)>,
}

fn u32_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

impl Kv3 {
    pub fn open(data: &[u8]) -> Result<Kv3, String> {
        let bad = || "KV3 header".to_string();
        if data.get(..4) != Some(&[0x05, 0x33, 0x56, 0x4B][..]) {
            return Err("not a KV3 v5 document".into());
        }
        let method = u32_at(data, 20).ok_or_else(bad)? as u32;
        if method > ZSTD {
            return Err(format!("compression method {method}"));
        }
        if u32_at(data, 56).ok_or_else(bad)? != 0 || u32_at(data, 60).ok_or_else(bad)? != 0 {
            return Err("document has binary blobs".into());
        }
        let g = |o: usize| u32_at(data, o).ok_or_else(bad);
        let (nb, ni, ne, ntypes) = (g(28)?, g(32)?, g(36)?, g(40)?);
        let (u1, c1, u2, c2) = (g(72)?, g(76)?, g(80)?, g(84)?);
        let (nb2, ns2, ni2, ne2, nobj) = (g(88)?, g(92)?, g(96)?, g(100)?, g(108)?);
        if [u1, u2, nb, ni, ne, nb2, ns2, ni2, ne2, nobj, ntypes].iter().any(|v| *v > MAX_BUFFER) || u1 == 0 {
            return Err("implausible sizes".into());
        }
        if data.len() != STREAM_AT + c1 + c2 {
            return Err("unexpected data after the buffers".into());
        }
        let b1 = unpack(method, &data[STREAM_AT..STREAM_AT + c1], u1)?;
        let c2raw = data[STREAM_AT + c1..].to_vec();
        let b2 = unpack(method, &c2raw, u2)?;
        let l1 = lanes(0, nb, 0, ni, ne);
        // a lane with no values takes no space, aligned or not
        let end1 = if ne > 0 { l1.e.0 + l1.e.1 } else { l1.i.0 + l1.i.1 };
        if end1 > u1 {
            return Err("unexpected first buffer layout".into());
        }
        let l2 = lanes(nobj * 4, nb2, ns2, ni2, ne2);
        let plain_end = [l2.b, l2.s, l2.i, l2.e].iter().rev().find(|l| l.1 > 0).map(|l| l.0 + l.1).unwrap_or(l2.b.0);
        let mut types = vec![(l2.e.0 + l2.e.1, ntypes)];
        if ne2 == 0 && plain_end != l2.e.0 + l2.e.1 {
            types.push((plain_end, ntypes));
        }
        types.retain(|(s, n)| s + n <= b2.len());
        if types.is_empty() {
            return Err("unexpected second buffer layout".into());
        }
        let count = i32::from_le_bytes(b1.get(l1.i.0..l1.i.0 + 4).ok_or("no string table")?.try_into().unwrap());
        if count < 0 || count as usize > nb {
            return Err("bad string count".into());
        }
        let mut strings = Vec::with_capacity(count as usize);
        let mut p = 0usize;
        for _ in 0..count {
            let rest = b1.get(p..nb).ok_or("strings run past their buffer")?;
            let n = rest.iter().position(|&c| c == 0).ok_or("unterminated string")?;
            strings.push(String::from_utf8_lossy(&rest[..n]).into_owned());
            p += n + 1;
        }
        let objcounts = (0..nobj).map(|i| i32::from_le_bytes(b2[i * 4..i * 4 + 4].try_into().unwrap())).collect();
        Ok(Kv3 { head: data[..STREAM_AT].to_vec(), method, b1, b2, c2: c2raw, l1, l2, strings, aux_start: Pos { b: p, s: 0, i: 4, e: 0 }, objcounts, types })
    }

    /// The whole tree.
    pub fn tree(&self) -> Result<Node, String> {
        let mut last = Err("no type stream".to_string());
        for t in 0..self.types.len() {
            let mut w = Walk { k: self, main: Pos::default(), aux: self.aux_start, tp: 0, oi: 0, depth: 0, types: self.types[t] };
            last = w.node().and_then(|root| {
                let l = &self.l2;
                let done = w.tp == w.types.1 && w.oi == self.objcounts.len() && w.main.b == l.b.1 && w.main.s == l.s.1 && w.main.i == l.i.1 && w.main.e == l.e.1;
                if done {
                    Ok(root)
                } else {
                    Err("the tree does not use all of its data".to_string())
                }
            });
            if last.is_ok() {
                break;
            }
        }
        last
    }

    fn lane_bytes(&self, l: Loc, n: usize) -> Option<&[u8]> {
        let (buf, lanes) = if l.aux { (&self.b1, &self.l1) } else { (&self.b2, &self.l2) };
        let base = match l.lane {
            0 => lanes.b.0,
            1 => lanes.s.0,
            2 => lanes.i.0,
            _ => lanes.e.0,
        };
        buf.get(base + l.off..base + l.off + n)
    }

    /// A number as f64, for float, double, integer and constant values.
    pub fn num(&self, v: &Val) -> Option<f64> {
        let get = |n: usize| v.loc.and_then(|l| self.lane_bytes(l, n));
        Some(match v.ty {
            FLT => f32::from_le_bytes(get(4)?.try_into().ok()?) as f64,
            DBL => f64::from_le_bytes(get(8)?.try_into().ok()?),
            D0 | F0 | I64_0 => 0.0,
            D1 | F1 | I64_1 => 1.0,
            I32 => i32::from_le_bytes(get(4)?.try_into().ok()?) as f64,
            U32 => u32::from_le_bytes(get(4)?.try_into().ok()?) as f64,
            _ => return None,
        })
    }

    /// Whether `set` can rewrite this value.
    pub fn writable(&self, v: &Val) -> bool {
        matches!((v.ty, v.loc), (FLT, Some(Loc { aux: true, lane: 2, .. })) | (DBL, Some(Loc { aux: true, lane: 3, .. })))
    }

    pub fn set(&mut self, v: &Val, x: f64) -> bool {
        let Some(l) = v.loc.filter(|l| l.aux) else { return false };
        let base = if l.lane == 2 { self.l1.i.0 } else { self.l1.e.0 };
        match (v.ty, l.lane) {
            (FLT, 2) => match self.b1.get_mut(base + l.off..base + l.off + 4) {
                Some(s) => {
                    s.copy_from_slice(&(x as f32).to_le_bytes());
                    true
                }
                None => false,
            },
            (DBL, 3) => match self.b1.get_mut(base + l.off..base + l.off + 8) {
                Some(s) => {
                    s.copy_from_slice(&x.to_le_bytes());
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    /// The block with the changed first buffer; everything else is byte for byte as it was.
    pub fn finish(&self) -> Vec<u8> {
        let packed = pack(self.method, &self.b1);
        let mut out = self.head.clone();
        let c2 = self.c2.len();
        out[52..56].copy_from_slice(&((packed.len() + c2) as u32).to_le_bytes());
        out[76..80].copy_from_slice(&(packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        out.extend_from_slice(&self.c2);
        out
    }
}

struct Walk<'a> {
    k: &'a Kv3,
    main: Pos,
    aux: Pos,
    tp: usize,
    oi: usize,
    depth: usize,
    types: (usize, usize),
}

impl Walk<'_> {
    fn tbyte(&mut self) -> Result<u8, String> {
        let (start, len) = self.types;
        if self.tp >= len {
            return Err("type stream ended early".into());
        }
        let v = self.k.b2[start + self.tp];
        self.tp += 1;
        Ok(v)
    }

    /// A type byte; a set high bit means a flag byte follows.
    fn ty(&mut self) -> Result<u8, String> {
        let tb = self.tbyte()?;
        if tb & 0x80 != 0 {
            self.tbyte()?;
        }
        Ok(tb & 0x7F)
    }

    fn take(&mut self, aux: bool, lane: u8, n: usize) -> Result<Loc, String> {
        let (lanes, pos) = if aux { (&self.k.l1, &mut self.aux) } else { (&self.k.l2, &mut self.main) };
        let (len, cur) = match lane {
            0 => (lanes.b.1, &mut pos.b),
            1 => (lanes.s.1, &mut pos.s),
            2 => (lanes.i.1, &mut pos.i),
            _ => (lanes.e.1, &mut pos.e),
        };
        let off = *cur;
        let end = off.checked_add(n).filter(|e| *e <= len).ok_or("value lane ran out")?;
        *cur = end;
        Ok(Loc { aux, lane, off })
    }

    fn int(&mut self, aux: bool) -> Result<i32, String> {
        let l = self.take(aux, 2, 4)?;
        Ok(i32::from_le_bytes(self.k.lane_bytes(l, 4).ok_or("lane")?.try_into().unwrap()))
    }

    fn byte(&mut self, aux: bool) -> Result<u8, String> {
        let l = self.take(aux, 0, 1)?;
        Ok(self.k.lane_bytes(l, 1).ok_or("lane")?[0])
    }

    fn prim(&mut self, t: u8, aux: bool) -> Result<Node, String> {
        let (lane, n) = match t {
            BOOL | I32B => (0, 1),
            I16 | U16 => (1, 2),
            I32 | U32 | FLT | STR => (2, 4),
            I64 | U64 | DBL => (3, 8),
            NULL | TRUE | FALSE | I64_0 | I64_1 | D0 | D1 | F0 | F1 => return Ok(Node::Val(Val { ty: t, loc: None })),
            _ => return Err(format!("unknown value type {t}")),
        };
        let loc = self.take(aux, lane, n)?;
        if t == STR {
            let i = i32::from_le_bytes(self.k.lane_bytes(loc, 4).ok_or("lane")?.try_into().unwrap());
            return Ok(Node::Str(if i < 0 { String::new() } else { self.k.strings.get(i as usize).cloned().ok_or("string index")? }));
        }
        Ok(Node::Val(Val { ty: t, loc: Some(loc) }))
    }

    fn node(&mut self) -> Result<Node, String> {
        let t = self.ty()?;
        self.value(t)
    }

    fn value(&mut self, t: u8) -> Result<Node, String> {
        self.depth += 1;
        if self.depth > 64 {
            return Err("tree too deep".into());
        }
        let r = self.value_inner(t);
        self.depth -= 1;
        r
    }

    fn value_inner(&mut self, t: u8) -> Result<Node, String> {
        match t {
            OBJ => {
                let n = *self.k.objcounts.get(self.oi).ok_or("object table ran out")?;
                self.oi += 1;
                let mut m = Vec::new();
                for _ in 0..n.max(0) {
                    let i = self.int(false)?;
                    let name = self.k.strings.get(i as usize).cloned().ok_or("member name index")?;
                    m.push((name, self.node()?));
                }
                Ok(Node::Obj(m))
            }
            ARR => {
                let n = self.int(false)?;
                (0..n.max(0)).map(|_| self.node()).collect::<Result<Vec<_>, _>>().map(Node::Arr)
            }
            ARRT | ARRB => {
                let n = if t == ARRT { self.int(false)? } else { self.byte(false)? as i32 };
                let st = self.ty()?;
                let mut out = Vec::new();
                for _ in 0..n.max(0) {
                    out.push(match st {
                        OBJ | ARR | ARRT | ARRB => self.value(st)?,
                        ARRA => {
                            let m = self.byte(false)?;
                            let sub = self.ty()?;
                            Node::Arr((0..m).map(|_| self.prim(sub, true)).collect::<Result<_, _>>()?)
                        }
                        _ => self.prim(st, false)?,
                    });
                }
                Ok(Node::Arr(out))
            }
            ARRA => {
                let n = self.byte(false)?;
                let st = self.ty()?;
                (0..n).map(|_| self.prim(st, true)).collect::<Result<Vec<_>, _>>().map(Node::Arr)
            }
            _ => self.prim(t, false),
        }
    }
}
