//! Loader for the render-only file written by crates/map-builder/build_render.py.
//!
//! Only the client reads this; the engine never does. Container (little-endian):
//! magic "MRRD" | format_version u32 | kind u32 (4) | payload_len u64 | checksum u64 (CRC-32 of
//! the payload, zero-extended). The payload uses the same array directory as the baked map.
//! Coordinates are Equal Earth metres (the same space as `cell_x` and `cell_y` in the map).

use crate::error::{DataError, ErrorKind};
use std::collections::HashMap;
use std::path::Path;

pub const RENDER_FORMAT_VERSION: u32 = 1;
const KIND_RENDER: u32 = 4;
const HEADER_LEN: usize = 28;

/// What a border line separates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LineKind {
    ProvinceBorder = 0,
    StateBorder = 1,
    Coast = 2,
}

/// One zoom level: a triangle mesh of every province plus its border lines.
#[derive(Clone, Debug)]
pub struct RenderLevel {
    /// Simplification tolerance this level was built with, in metres.
    pub tolerance_m: u32,
    pub vx: Vec<i32>,
    pub vy: Vec<i32>,
    /// Province of each vertex. All three vertices of a triangle share one province.
    pub vprov: Vec<u32>,
    /// Three vertex indexes per triangle.
    pub tri: Vec<u32>,
    pub line_x: Vec<i32>,
    pub line_y: Vec<i32>,
    /// Line `i` is the points `line_off[i]..line_off[i + 1]`.
    pub line_off: Vec<u32>,
    pub line_kind: Vec<u8>,
}

impl RenderLevel {
    pub fn line_count(&self) -> usize {
        self.line_kind.len()
    }
    pub fn kind(&self, line: usize) -> LineKind {
        match self.line_kind[line] {
            0 => LineKind::ProvinceBorder,
            1 => LineKind::StateBorder,
            _ => LineKind::Coast,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderData {
    pub checksum: u32,
    /// Coarsest first.
    pub levels: Vec<RenderLevel>,
    /// Every distinct cell edge once, four values each: x0, y0, x1, y1.
    pub cell_seg: Vec<i32>,
}

struct Dir<'a> {
    file: &'a str,
    by_name: HashMap<&'a str, (u8, &'a [u8])>,
}

impl Dir<'_> {
    fn bad(&self, item: &str, msg: String) -> DataError {
        DataError::new(self.file, item, ErrorKind::BadBinary, msg)
    }
    fn raw(&self, name: &str, dtype: u8) -> Result<&[u8], DataError> {
        let (dt, bytes) = *self.by_name.get(name).ok_or_else(|| self.bad(name, "array is missing from the file".into()))?;
        if dt != dtype {
            return Err(self.bad(name, format!("array has dtype {dt}, expected {dtype}")));
        }
        Ok(bytes)
    }
    fn u8s(&self, name: &str) -> Result<Vec<u8>, DataError> {
        Ok(self.raw(name, 1)?.to_vec())
    }
    fn u32s(&self, name: &str) -> Result<Vec<u32>, DataError> {
        Ok(self.raw(name, 3)?.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
    fn i32s(&self, name: &str) -> Result<Vec<i32>, DataError> {
        Ok(self.raw(name, 5)?.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
}

/// Parse a render file from bytes. `file` only labels errors.
pub fn parse_render(file: &str, bytes: &[u8]) -> Result<RenderData, DataError> {
    let bad = |msg: &str| DataError::new(file, "-", ErrorKind::BadBinary, msg);
    if bytes.len() < HEADER_LEN || &bytes[0..4] != b"MRRD" {
        return Err(bad("not a Meridian render file (bad magic number)"));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != RENDER_FORMAT_VERSION {
        return Err(DataError::new(file, "-", ErrorKind::WrongVersion, format!("render format version {version}, this build reads {RENDER_FORMAT_VERSION}")));
    }
    if u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != KIND_RENDER {
        return Err(bad("file is not a render file (wrong kind)"));
    }
    let len = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let sum = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
    let payload = &bytes[HEADER_LEN..];
    if payload.len() as u64 != len {
        return Err(DataError::new(file, "-", ErrorKind::BadBinary, format!("payload is {} bytes, header says {len} (truncated?)", payload.len())));
    }
    let crc = crc32fast::hash(payload);
    if crc as u64 != sum {
        return Err(bad("checksum mismatch (file is corrupt)"));
    }

    let mut by_name = HashMap::new();
    let mut pos = 0usize;
    let take = |pos: &mut usize, n: usize| -> Result<&[u8], DataError> {
        let end = pos.checked_add(n).filter(|&e| e <= payload.len()).ok_or_else(|| bad("array table runs past the end of the file"))?;
        let s = &payload[*pos..end];
        *pos = end;
        Ok(s)
    };
    let count = u32::from_le_bytes(take(&mut pos, 4)?.try_into().unwrap());
    for _ in 0..count {
        let nl = take(&mut pos, 1)?[0] as usize;
        let name = std::str::from_utf8(take(&mut pos, nl)?).map_err(|_| bad("array name is not text"))?;
        let dtype = take(&mut pos, 1)?[0];
        let n = u64::from_le_bytes(take(&mut pos, 8)?.try_into().unwrap()) as usize;
        let width = match dtype {
            1 => 1,
            2 | 4 => 2,
            3 | 5 => 4,
            other => return Err(DataError::new(file, name, ErrorKind::BadBinary, format!("unknown dtype {other}"))),
        };
        let nbytes = n.checked_mul(width).ok_or_else(|| bad("array size overflows"))?;
        let data = take(&mut pos, nbytes)?;
        if by_name.insert(name, (dtype, data)).is_some() {
            return Err(DataError::new(file, name, ErrorKind::BadBinary, "array appears twice"));
        }
    }
    if pos != payload.len() {
        return Err(bad("extra bytes after the last array"));
    }

    let d = Dir { file, by_name };
    let tolerances = d.u32s("levels_tolerance_m")?;
    let mut levels = Vec::with_capacity(tolerances.len());
    for (k, &tolerance_m) in tolerances.iter().enumerate() {
        let n = |s: &str| format!("l{k}_{s}");
        let level = RenderLevel {
            tolerance_m,
            vx: d.i32s(&n("vx"))?,
            vy: d.i32s(&n("vy"))?,
            vprov: d.u32s(&n("vprov"))?,
            tri: d.u32s(&n("tri"))?,
            line_x: d.i32s(&n("line_x"))?,
            line_y: d.i32s(&n("line_y"))?,
            line_off: d.u32s(&n("line_off"))?,
            line_kind: d.u8s(&n("line_kind"))?,
        };
        check_level(&d, k, &level)?;
        levels.push(level);
    }
    let cell_seg = d.i32s("cell_seg")?;
    if !cell_seg.len().is_multiple_of(4) {
        return Err(d.bad("cell_seg", "length is not a multiple of 4".into()));
    }
    Ok(RenderData { checksum: crc, levels, cell_seg })
}

fn check_level(d: &Dir, k: usize, l: &RenderLevel) -> Result<(), DataError> {
    let item = format!("level {k}");
    let fail = |msg: &str| Err(d.bad(&item, msg.to_string()));
    let nv = l.vx.len();
    if l.vy.len() != nv || l.vprov.len() != nv {
        return fail("vertex arrays differ in length");
    }
    if !l.tri.len().is_multiple_of(3) || l.tri.iter().any(|&i| i as usize >= nv) {
        return fail("triangle index out of range");
    }
    if l.tri.chunks_exact(3).any(|t| l.vprov[t[0] as usize] != l.vprov[t[1] as usize] || l.vprov[t[0] as usize] != l.vprov[t[2] as usize]) {
        return fail("a triangle spans two provinces");
    }
    if l.line_x.len() != l.line_y.len() || l.line_off.len() != l.line_kind.len() + 1 {
        return fail("line arrays differ in length");
    }
    if l.line_off.first() != Some(&0) || l.line_off.last().map(|&e| e as usize) != Some(l.line_x.len()) || l.line_off.windows(2).any(|w| w[1] < w[0] + 2) {
        return fail("line offsets are not increasing, or a line has fewer than two points");
    }
    if l.line_kind.iter().any(|&k| k > 2) {
        return fail("unknown line kind");
    }
    Ok(())
}

pub fn load_render_file(path: &Path) -> Result<RenderData, DataError> {
    let name = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|e| DataError::new(&name, "-", ErrorKind::Io, e.to_string()))?;
    parse_render(&name, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn array(out: &mut Vec<u8>, name: &str, dtype: u8, data: &[u8], count: u64) {
        out.push(name.len() as u8);
        out.extend_from_slice(name.as_bytes());
        out.push(dtype);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(data);
    }
    fn le32(v: &[i32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// One level: one square province made of two triangles, one coast line.
    fn tiny(tri: [u32; 6]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&10u32.to_le_bytes());
        let u = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        array(&mut p, "levels_tolerance_m", 3, &u(&[1000]), 1);
        array(&mut p, "l0_vx", 5, &le32(&[0, 10, 10, 0]), 4);
        array(&mut p, "l0_vy", 5, &le32(&[0, 0, 10, 10]), 4);
        array(&mut p, "l0_vprov", 3, &u(&[0, 0, 0, 0]), 4);
        array(&mut p, "l0_tri", 3, &u(&tri), 6);
        array(&mut p, "l0_line_x", 5, &le32(&[0, 10, 10]), 3);
        array(&mut p, "l0_line_y", 5, &le32(&[0, 0, 10]), 3);
        array(&mut p, "l0_line_off", 3, &u(&[0, 3]), 2);
        array(&mut p, "l0_line_kind", 1, &[2], 1);
        array(&mut p, "cell_seg", 5, &le32(&[0, 0, 10, 0]), 4);
        let mut f = b"MRRD".to_vec();
        f.extend_from_slice(&1u32.to_le_bytes());
        f.extend_from_slice(&4u32.to_le_bytes());
        f.extend_from_slice(&(p.len() as u64).to_le_bytes());
        f.extend_from_slice(&(crc32fast::hash(&p) as u64).to_le_bytes());
        f.extend_from_slice(&p);
        f
    }

    #[test]
    fn a_small_file_loads() {
        let r = parse_render("t", &tiny([0, 1, 2, 0, 2, 3])).unwrap();
        assert_eq!(r.levels.len(), 1);
        assert_eq!(r.levels[0].tri.len(), 6);
        assert_eq!(r.levels[0].kind(0), LineKind::Coast);
        assert_eq!(r.cell_seg.len(), 4);
    }

    #[test]
    fn damaged_files_are_refused_with_a_reason() {
        let good = tiny([0, 1, 2, 0, 2, 3]);
        let msg = |b: &[u8]| parse_render("t", b).unwrap_err().to_string();
        assert!(msg(&tiny([0, 1, 2, 0, 2, 9])).contains("triangle index out of range"));
        let mut flipped = good.clone();
        *flipped.last_mut().unwrap() ^= 1;
        assert!(msg(&flipped).contains("checksum"));
        assert!(msg(&good[..good.len() - 3]).contains("truncated"));
        let mut other = good.clone();
        other[0..4].copy_from_slice(b"MRMP");
        assert!(msg(&other).contains("bad magic"));
        let mut v2 = good.clone();
        v2[4] = 2;
        assert!(msg(&v2).contains("version 2"));
    }
}
