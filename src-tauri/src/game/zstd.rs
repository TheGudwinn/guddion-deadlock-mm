//! Small zstd (RFC 8878) frame decoder and a "store only" encoder.
//!
//! Some compiled models keep their mesh data in zstd frames. The reader needs to look inside those, and
//! a repair rewrites only the first buffer, which is written back as raw zstd blocks: bigger than a real
//! compression, valid for every decoder. Everything is bounds-checked; the content checksum, when the
//! frame has one, is verified, so a wrong decode never passes silently.

const MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const MAX_BLOCK: usize = 1 << 17;

const LL_BASE: [u32; 36] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32, 40, 48, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536];
const LL_BITS: [u32; 36] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
const ML_BASE: [u32; 53] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 37, 39, 41, 43, 47, 51, 59, 67, 83, 99, 131, 259, 515, 1027, 2051, 4099, 8195, 16387, 32771, 65539,
];
const ML_BITS: [u32; 53] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

const LL_DEFAULT: [i16; 36] = [4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1, -1, -1, -1, -1];
const ML_DEFAULT: [i16; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1,
];
const OF_DEFAULT: [i16; 29] = [1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1];

fn err<T>(s: &str) -> Result<T, String> {
    Err(format!("zstd: {s}"))
}

/// Bits read from the start, least significant first.
struct Fwd<'a> {
    d: &'a [u8],
    bit: usize,
}

impl Fwd<'_> {
    fn peek(&self, n: u32) -> u32 {
        let mut v = 0u64;
        for i in 0..8 {
            if let Some(&b) = self.d.get(self.bit / 8 + i) {
                v |= (b as u64) << (8 * i);
            }
        }
        ((v >> (self.bit % 8)) & ((1u64 << n) - 1)) as u32
    }
    fn read(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.bit += n as usize;
        v
    }
}

/// Bits read from the end: the last byte holds a marker bit, the stream is read downwards.
struct Back<'a> {
    d: &'a [u8],
    /// Unread bits; negative once more was read than there is.
    pos: i64,
}

impl<'a> Back<'a> {
    fn new(d: &'a [u8]) -> Result<Self, String> {
        let last = *d.last().ok_or("zstd: empty bit stream")?;
        if last == 0 {
            return err("bit stream has no end marker");
        }
        let top = 7 - last.leading_zeros() as i64;
        Ok(Back { d, pos: (d.len() as i64 - 1) * 8 + top })
    }
    fn bits(&self, start: i64, n: u32) -> u32 {
        // bits [start, start + n) of the stream; positions below 0 do not exist and read as zeros
        let (first, miss) = if start < 0 { (0usize, (-start) as u32) } else { (start as usize, 0) };
        if miss >= n {
            return 0;
        }
        let have = n - miss;
        let mut v = 0u64;
        for i in 0..8 {
            if let Some(&b) = self.d.get(first / 8 + i) {
                v |= (b as u64) << (8 * i);
            }
        }
        (((v >> (first % 8)) & ((1u64 << have) - 1)) << miss) as u32
    }
    fn peek(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        self.bits(self.pos - n as i64, n)
    }
    fn read(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.pos -= n as i64;
        v
    }
}

fn log2(x: u32) -> u32 {
    31 - x.leading_zeros()
}

#[derive(Clone)]
struct Fse {
    al: u32,
    /// (symbol, bits to read, baseline) per state
    ent: Vec<(u8, u8, u16)>,
}

impl Fse {
    fn build(norm: &[i16], al: u32) -> Result<Fse, String> {
        let size = 1usize << al;
        if norm.iter().map(|&n| if n < 0 { 1 } else { n as usize }).sum::<usize>() != size {
            return err("distribution does not add up");
        }
        let mut ent = vec![(0u8, 0u8, 0u16); size];
        let mut high = size;
        for (s, &n) in norm.iter().enumerate() {
            if n == -1 {
                high -= 1;
                ent[high].0 = s as u8;
            }
        }
        let (step, mask) = ((size >> 1) + (size >> 3) + 3, size - 1);
        let mut pos = 0usize;
        for (s, &n) in norm.iter().enumerate() {
            for _ in 0..n.max(0) {
                ent[pos].0 = s as u8;
                pos = (pos + step) & mask;
                while pos >= high {
                    pos = (pos + step) & mask;
                }
            }
        }
        if pos != 0 {
            return err("symbols do not fill the table");
        }
        let mut next: Vec<u32> = norm.iter().map(|&n| if n < 0 { 1 } else { n as u32 }).collect();
        for e in ent.iter_mut() {
            let x = next[e.0 as usize];
            next[e.0 as usize] += 1;
            let nb = al - log2(x);
            e.1 = nb as u8;
            e.2 = ((x << nb) - size as u32) as u16;
        }
        Ok(Fse { al, ent })
    }

    fn single(sym: u8) -> Fse {
        Fse { al: 0, ent: vec![(sym, 0, 0)] }
    }

    fn step(&self, state: usize, b: &mut Back) -> usize {
        let (_, nb, base) = self.ent[state];
        base as usize + b.read(nb as u32) as usize
    }
}

/// Distribution table description: (counts, accuracy log, bytes used).
fn read_ncount(d: &[u8], max_sym: usize, max_al: u32) -> Result<(Vec<i16>, u32, usize), String> {
    let mut r = Fwd { d, bit: 0 };
    let al = 5 + r.read(4);
    if al > max_al {
        return err("accuracy log too large");
    }
    let mut remaining: i32 = (1 << al) + 1;
    let mut threshold: i32 = 1 << al;
    let mut nbits = al + 1;
    let mut norm: Vec<i16> = Vec::new();
    let mut prev0 = false;
    while remaining > 1 {
        if prev0 {
            loop {
                let rep = r.read(2);
                norm.extend(std::iter::repeat_n(0, rep as usize));
                if rep != 3 {
                    break;
                }
                if norm.len() > max_sym + 1 {
                    return err("too many symbols");
                }
            }
        }
        if norm.len() > max_sym || r.bit > d.len() * 8 {
            return err("distribution is cut short");
        }
        let max = 2 * threshold - 1 - remaining;
        let low = r.peek(nbits - 1) as i32 & (threshold - 1);
        let mut count;
        if low < max {
            count = low;
            r.bit += nbits as usize - 1;
        } else {
            count = r.peek(nbits) as i32 & (2 * threshold - 1);
            if count >= threshold {
                count -= max;
            }
            r.bit += nbits as usize;
        }
        count -= 1;
        remaining -= count.abs();
        if remaining < 1 {
            return err("distribution overshoots");
        }
        norm.push(count as i16);
        prev0 = count == 0;
        while remaining < threshold {
            nbits -= 1;
            threshold >>= 1;
        }
    }
    if norm.len() > max_sym + 1 || r.bit > d.len() * 8 {
        return err("distribution is cut short");
    }
    Ok((norm, al, r.bit.div_ceil(8)))
}

#[derive(Clone)]
struct Huf {
    bits: u32,
    /// (symbol, code length) indexed by the next `bits` bits of the stream
    table: Vec<(u8, u8)>,
}

impl Huf {
    fn from_weights(mut w: Vec<u8>) -> Result<Huf, String> {
        if w.len() > 255 || w.iter().any(|&x| x > 12) {
            return err("bad Huffman weights");
        }
        let sum: u32 = w.iter().map(|&x| if x > 0 { 1u32 << (x - 1) } else { 0 }).sum();
        if sum == 0 {
            return err("empty Huffman tree");
        }
        let bits = log2(sum) + 1;
        if bits > 12 {
            return err("Huffman codes too long");
        }
        let rest = (1u32 << bits) - sum;
        if !rest.is_power_of_two() {
            return err("Huffman weights do not complete the tree");
        }
        w.push((log2(rest) + 1) as u8);
        let mut table = vec![(0u8, 0u8); 1 << bits];
        let mut pos = 0usize;
        for weight in 1..=bits as u8 {
            for (sym, &x) in w.iter().enumerate() {
                if x == weight {
                    let n = 1usize << (weight - 1);
                    for e in &mut table[pos..pos + n] {
                        *e = (sym as u8, (bits + 1 - weight as u32) as u8);
                    }
                    pos += n;
                }
            }
        }
        Ok(Huf { bits, table })
    }

    /// Reads the tree description; returns the tree and the bytes it took.
    fn read(d: &[u8]) -> Result<(Huf, usize), String> {
        let hb = *d.first().ok_or("zstd: empty Huffman header")? as usize;
        if hb >= 128 {
            let n = hb - 127;
            let used = 1 + n.div_ceil(2);
            let raw = d.get(1..used).ok_or("zstd: Huffman header cut short")?;
            let w = (0..n).map(|i| if i % 2 == 0 { raw[i / 2] >> 4 } else { raw[i / 2] & 15 }).collect();
            return Ok((Huf::from_weights(w)?, used));
        }
        let body = d.get(1..1 + hb).ok_or("zstd: Huffman header cut short")?;
        let (norm, al, used) = read_ncount(body, 12, 6)?;
        let fse = Fse::build(&norm, al)?;
        let mut b = Back::new(body.get(used..).ok_or("zstd: Huffman header cut short")?)?;
        let mut s1 = b.read(al) as usize;
        let mut s2 = b.read(al) as usize;
        let mut w = Vec::new();
        loop {
            if w.len() > 255 {
                return err("too many Huffman weights");
            }
            w.push(fse.ent[s1].0);
            s1 = fse.step(s1, &mut b);
            if b.pos < 0 {
                w.push(fse.ent[s2].0);
                break;
            }
            w.push(fse.ent[s2].0);
            s2 = fse.step(s2, &mut b);
            if b.pos < 0 {
                w.push(fse.ent[s1].0);
                break;
            }
        }
        Ok((Huf::from_weights(w)?, 1 + hb))
    }

    fn stream(&self, d: &[u8], n: usize, out: &mut Vec<u8>) -> Result<(), String> {
        let mut b = Back::new(d)?;
        for _ in 0..n {
            let (sym, nb) = self.table[b.peek(self.bits) as usize];
            b.pos -= nb as i64;
            out.push(sym);
        }
        if b.pos != 0 {
            return err("literal stream is not used exactly");
        }
        Ok(())
    }
}

struct Dec {
    out: Vec<u8>,
    limit: usize,
    huf: Option<Huf>,
    ll: Option<Fse>,
    of: Option<Fse>,
    ml: Option<Fse>,
    rep: [usize; 3],
}

fn tables(mode: u8, d: &[u8], p: &mut usize, prev: &Option<Fse>, default: &[i16], default_al: u32, max_sym: usize, max_al: u32) -> Result<Fse, String> {
    match mode {
        0 => Fse::build(default, default_al),
        1 => {
            let s = *d.get(*p).ok_or("zstd: table cut short")?;
            *p += 1;
            if s as usize > max_sym {
                return err("symbol out of range");
            }
            Ok(Fse::single(s))
        }
        2 => {
            let (norm, al, used) = read_ncount(d.get(*p..).ok_or("zstd: table cut short")?, max_sym, max_al)?;
            *p += used;
            Fse::build(&norm, al)
        }
        _ => prev.clone().ok_or_else(|| "zstd: repeated table without a previous one".to_string()),
    }
}

impl Dec {
    fn literals(&mut self, b: &[u8]) -> Result<(Vec<u8>, usize), String> {
        let b0 = *b.first().ok_or("zstd: empty block")?;
        let (typ, sf) = (b0 & 3, (b0 >> 2) & 3);
        let at = |i: usize| b.get(i).map(|&x| x as usize).ok_or_else(|| "zstd: literals header cut short".to_string());
        if typ < 2 {
            let (size, head) = match sf {
                0 | 2 => ((b0 >> 3) as usize, 1),
                1 => ((b0 >> 4) as usize | at(1)? << 4, 2),
                _ => ((b0 >> 4) as usize | at(1)? << 4 | at(2)? << 12, 3),
            };
            if size > MAX_BLOCK {
                return err("literals too long");
            }
            return if typ == 0 {
                Ok((b.get(head..head + size).ok_or("zstd: literals cut short")?.to_vec(), head + size))
            } else {
                Ok((vec![at(head)? as u8; size], head + 1))
            };
        }
        let (streams4, bits, head) = match sf {
            0 => (false, 10, 3),
            1 => (true, 10, 3),
            2 => (true, 14, 4),
            _ => (true, 18, 5),
        };
        let mut v = 0u64;
        for i in 0..head {
            v |= (at(i)? as u64) << (8 * i);
        }
        v >>= 4;
        let (regen, comp) = ((v & ((1 << bits) - 1)) as usize, ((v >> bits) & ((1 << bits) - 1)) as usize);
        if regen > MAX_BLOCK {
            return err("literals too long");
        }
        let data = b.get(head..head + comp).ok_or("zstd: literals cut short")?;
        let mut p = 0;
        if typ == 2 {
            let (h, used) = Huf::read(data)?;
            self.huf = Some(h);
            p = used;
        }
        let huf = self.huf.as_ref().ok_or("zstd: literals without a Huffman tree")?;
        let body = &data[p..];
        let mut out = Vec::with_capacity(regen);
        if !streams4 {
            huf.stream(body, regen, &mut out)?;
        } else {
            let sz = |i: usize| -> Result<usize, String> { Ok(u16::from_le_bytes(body.get(i * 2..i * 2 + 2).ok_or("zstd: jump table cut short")?.try_into().unwrap()) as usize) };
            let (s1, s2, s3) = (sz(0)?, sz(1)?, sz(2)?);
            let rest = body.get(6..).ok_or("zstd: jump table cut short")?;
            if s1 + s2 + s3 > rest.len() {
                return err("literal streams run past their data");
            }
            let part = regen.div_ceil(4);
            let cuts = [(0, s1, part), (s1, s1 + s2, part), (s1 + s2, s1 + s2 + s3, part), (s1 + s2 + s3, rest.len(), regen.saturating_sub(3 * part))];
            for (a, z, n) in cuts {
                huf.stream(&rest[a..z], n, &mut out)?;
            }
        }
        Ok((out, head + comp))
    }

    fn block(&mut self, b: &[u8]) -> Result<(), String> {
        let (lit, mut p) = self.literals(b)?;
        let n0 = *b.get(p).ok_or("zstd: block has no sequence section")? as usize;
        p += 1;
        let nseq = if n0 == 0 {
            0
        } else if n0 < 128 {
            n0
        } else if n0 < 255 {
            let v = ((n0 - 128) << 8) + *b.get(p).ok_or("zstd: sequence count cut short")? as usize;
            p += 1;
            v
        } else {
            let v = *b.get(p).ok_or("zstd: sequence count cut short")? as usize + ((*b.get(p + 1).ok_or("zstd: sequence count cut short")? as usize) << 8) + 0x7F00;
            p += 2;
            v
        };
        if nseq == 0 {
            if self.out.len() + lit.len() > self.limit {
                return err("output longer than declared");
            }
            self.out.extend_from_slice(&lit);
            return Ok(());
        }
        let modes = *b.get(p).ok_or("zstd: sequence modes cut short")?;
        p += 1;
        let ll = tables(modes >> 6, b, &mut p, &self.ll, &LL_DEFAULT, 6, 35, 9)?;
        let of = tables((modes >> 4) & 3, b, &mut p, &self.of, &OF_DEFAULT, 5, 31, 8)?;
        let ml = tables((modes >> 2) & 3, b, &mut p, &self.ml, &ML_DEFAULT, 6, 52, 9)?;
        let mut r = Back::new(b.get(p..).ok_or("zstd: sequences cut short")?)?;
        let (mut sl, mut so, mut sm) = (r.read(ll.al) as usize, r.read(of.al) as usize, r.read(ml.al) as usize);
        let mut used = 0usize;
        for i in 0..nseq {
            let (llc, ofc, mlc) = (ll.ent[sl].0 as usize, of.ent[so].0 as u32, ml.ent[sm].0 as usize);
            let value = (1usize << ofc) + r.read(ofc) as usize;
            let mlen = (ML_BASE[mlc] + r.read(ML_BITS[mlc])) as usize;
            let llen = (LL_BASE[llc] + r.read(LL_BITS[llc])) as usize;
            let offset = if value > 3 {
                self.rep = [value - 3, self.rep[0], self.rep[1]];
                value - 3
            } else {
                let idx = if llen == 0 { value } else { value - 1 };
                let o = match idx {
                    0 => self.rep[0],
                    1 => {
                        self.rep = [self.rep[1], self.rep[0], self.rep[2]];
                        self.rep[0]
                    }
                    2 => {
                        self.rep = [self.rep[2], self.rep[0], self.rep[1]];
                        self.rep[0]
                    }
                    _ => {
                        let o = self.rep[0].checked_sub(1).filter(|o| *o > 0).ok_or("zstd: bad repeated offset")?;
                        self.rep = [o, self.rep[0], self.rep[1]];
                        o
                    }
                };
                o
            };
            let end = used + llen;
            if end > lit.len() || self.out.len() + llen + mlen > self.limit {
                return err("sequence runs past its data");
            }
            self.out.extend_from_slice(&lit[used..end]);
            used = end;
            if offset == 0 || offset > self.out.len() {
                return err("match offset out of range");
            }
            for _ in 0..mlen {
                let c = self.out[self.out.len() - offset];
                self.out.push(c);
            }
            if i + 1 < nseq {
                sl = ll.step(sl, &mut r);
                sm = ml.step(sm, &mut r);
                so = of.step(so, &mut r);
            }
            if r.pos < 0 {
                return err("sequences ran past their stream");
            }
        }
        if self.out.len() + lit.len() - used > self.limit {
            return err("output longer than declared");
        }
        self.out.extend_from_slice(&lit[used..]);
        self.ll = Some(ll);
        self.of = Some(of);
        self.ml = Some(ml);
        Ok(())
    }
}

const P1: u64 = 11400714785074694791;
const P2: u64 = 14029467366897019727;
const P3: u64 = 1609587929392839161;
const P4: u64 = 9650029242287828579;
const P5: u64 = 2870177450012600261;

fn round(acc: u64, x: u64) -> u64 {
    acc.wrapping_add(x.wrapping_mul(P2)).rotate_left(31).wrapping_mul(P1)
}

pub fn xxh64(d: &[u8]) -> u64 {
    let rd8 = |i: usize| u64::from_le_bytes(d[i..i + 8].try_into().unwrap());
    let mut i = 0;
    let mut h;
    if d.len() >= 32 {
        let mut v = [P1.wrapping_add(P2), P2, 0, 0u64.wrapping_sub(P1)];
        while i + 32 <= d.len() {
            for (k, x) in v.iter_mut().enumerate() {
                *x = round(*x, rd8(i + k * 8));
            }
            i += 32;
        }
        h = v[0].rotate_left(1).wrapping_add(v[1].rotate_left(7)).wrapping_add(v[2].rotate_left(12)).wrapping_add(v[3].rotate_left(18));
        for x in v {
            h = (h ^ round(0, x)).wrapping_mul(P1).wrapping_add(P4);
        }
    } else {
        h = P5;
    }
    h = h.wrapping_add(d.len() as u64);
    while i + 8 <= d.len() {
        h = (h ^ round(0, rd8(i))).rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        i += 8;
    }
    if i + 4 <= d.len() {
        h = (h ^ (u32::from_le_bytes(d[i..i + 4].try_into().unwrap()) as u64).wrapping_mul(P1)).rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        i += 4;
    }
    while i < d.len() {
        h = (h ^ (d[i] as u64).wrapping_mul(P5)).rotate_left(11).wrapping_mul(P1);
        i += 1;
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^ (h >> 32)
}

/// Decodes one frame that must hold exactly `want` bytes.
pub fn decompress(src: &[u8], want: usize) -> Result<Vec<u8>, String> {
    if src.get(..4) != Some(&MAGIC[..]) {
        return err("not a zstd frame");
    }
    let desc = *src.get(4).ok_or("zstd: cut short")?;
    let (single, checksum) = (desc & 0x20 != 0, desc & 4 != 0);
    if desc & 8 != 0 || desc & 3 != 0 {
        return err("reserved bit or dictionary");
    }
    let mut p = 5 + usize::from(!single);
    let fcs_len = match desc >> 6 {
        0 => usize::from(single),
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let fcs = src.get(p..p + fcs_len).ok_or("zstd: cut short")?;
    p += fcs_len;
    if fcs_len > 0 {
        let mut v = 0u64;
        for (i, b) in fcs.iter().enumerate() {
            v |= (*b as u64) << (8 * i);
        }
        if fcs_len == 2 {
            v += 256;
        }
        if v != want as u64 {
            return err("content size differs from the declared one");
        }
    }
    if want > (128 << 20) {
        return err("implausible size");
    }
    let mut d = Dec { out: Vec::with_capacity(want), limit: want, huf: None, ll: None, of: None, ml: None, rep: [1, 4, 8] };
    loop {
        let h = src.get(p..p + 3).ok_or("zstd: cut short")?;
        let h = h[0] as usize | (h[1] as usize) << 8 | (h[2] as usize) << 16;
        p += 3;
        let (last, typ, size) = (h & 1 != 0, (h >> 1) & 3, h >> 3);
        if size > MAX_BLOCK {
            return err("block too large");
        }
        match typ {
            0 => {
                if d.out.len() + size > want {
                    return err("output longer than declared");
                }
                d.out.extend_from_slice(src.get(p..p + size).ok_or("zstd: block cut short")?);
                p += size;
            }
            1 => {
                if d.out.len() + size > want {
                    return err("output longer than declared");
                }
                let b = *src.get(p).ok_or("zstd: block cut short")?;
                d.out.resize(d.out.len() + size, b);
                p += 1;
            }
            2 => {
                d.block(src.get(p..p + size).ok_or("zstd: block cut short")?)?;
                p += size;
            }
            _ => return err("reserved block type"),
        }
        if last {
            break;
        }
    }
    if checksum {
        let want_sum = u32::from_le_bytes(src.get(p..p + 4).ok_or("zstd: checksum cut short")?.try_into().unwrap());
        p += 4;
        if xxh64(&d.out) as u32 != want_sum {
            return err("content checksum differs");
        }
    }
    if p != src.len() {
        return err("data after the frame");
    }
    if d.out.len() != want {
        return err("output shorter than declared");
    }
    Ok(d.out)
}

/// A valid frame that holds `raw` in uncompressed blocks.
pub fn store(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / MAX_BLOCK * 3 + 16);
    out.extend_from_slice(&MAGIC);
    out.push(0xA0); // 4-byte content size, single segment, no checksum
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    let mut chunks = raw.chunks(MAX_BLOCK).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0]);
    }
    while let Some(c) = chunks.next() {
        let h = (c.len() << 3) | usize::from(chunks.peek().is_none());
        out.extend_from_slice(&[h as u8, (h >> 8) as u8, (h >> 16) as u8]);
        out.extend_from_slice(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xxh64_known_values() {
        assert_eq!(xxh64(b""), 0xEF46DB3751D8E999);
        assert_eq!(xxh64(b"a"), 0xD24EC4F1A98C6E5B);
    }

    #[test]
    fn stored_frames_round_trip() {
        for n in [0usize, 1, 100, MAX_BLOCK, MAX_BLOCK + 1, 3 * MAX_BLOCK + 17] {
            let raw: Vec<u8> = (0..n).map(|i| (i * 31 % 251) as u8).collect();
            assert_eq!(decompress(&store(&raw), n).unwrap(), raw, "{n}");
        }
        assert!(decompress(&store(b"abc"), 4).is_err());
    }

    #[test]
    fn rle_block_frame() {
        // single segment with a 1-byte content size of 5, one last RLE block of 5 bytes
        let h = (5u32 << 3) | (1 << 1) | 1;
        let f = [0x28, 0xB5, 0x2F, 0xFD, 0x20, 5, h as u8, 0, 0, b'x'];
        assert_eq!(decompress(&f, 5).unwrap(), b"xxxxx");
    }

    #[test]
    fn refuses_damaged_frames() {
        assert!(decompress(b"not a frame", 3).is_err());
        let mut f = store(b"hello world");
        f.truncate(f.len() - 1);
        assert!(decompress(&f, 11).is_err());
    }
}
