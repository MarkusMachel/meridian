//! Loader for the render-only file written by crates/map-builder/build_render.py.
//!
//! Only the client reads this; the engine never does. Container (little-endian):
//! magic "MRRD" | format_version u32 | kind u32 (4) | payload_len u64 | checksum u64 (CRC-32 of
//! the payload, zero-extended). The payload uses the same array directory as the baked map.
//! Coordinates are Equal Earth metres (EPSG:8857), the same equal-area plane the simulation uses,
//! so the display has no stretch toward the poles. The file carries cell and province centres in
//! this plane too.

use crate::error::{DataError, ErrorKind};
use std::collections::HashMap;
use std::path::Path;

pub const RENDER_FORMAT_VERSION: u32 = 5;
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

/// Shaded relief: a greyscale raster over the Equal Earth plane. Row 0 is the top edge (`y1`).
/// A byte of 128 means "no change"; the client multiplies colours by `byte / 128`.
#[derive(Clone, Debug)]
pub struct Relief {
    pub width: u32,
    pub height: u32,
    /// Edges of the raster in Equal Earth metres.
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub pixels: Vec<u8>,
}

/// A set of polylines with a class each (lower class = more important), sorted by class.
#[derive(Clone, Debug, Default)]
pub struct LineLayer {
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    /// Line `i` is the points `off[i]..off[i + 1]`.
    pub off: Vec<u32>,
    pub class: Vec<u8>,
}

/// Place names with one point each (Equal Earth metres) and the area they cover.
#[derive(Clone, Debug, Default)]
pub struct Labels {
    pub names: Vec<String>,
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    pub area_km2: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct RenderData {
    pub checksum: u32,
    /// Coarsest first.
    pub levels: Vec<RenderLevel>,
    /// Every distinct cell edge once, four values each: x0, y0, x1, y1.
    pub cell_seg: Vec<i32>,
    /// Cell centres in the display projection, one per cell.
    pub cell_mx: Vec<i32>,
    pub cell_my: Vec<i32>,
    /// Each cell's outer ring (no repeated closing point): cell `i` is the points
    /// `cell_ring_off[i]..cell_ring_off[i + 1]`.
    pub cell_ring_off: Vec<u32>,
    pub cell_ring_x: Vec<i32>,
    pub cell_ring_y: Vec<i32>,
    /// Province centres (area-weighted) in the display projection, one per province.
    pub province_mx: Vec<i32>,
    pub province_my: Vec<i32>,
    pub relief: Relief,
    /// River centrelines: line `i` is the points `river_off[i]..river_off[i + 1]`; a lower rank is a bigger river.
    pub river_x: Vec<i32>,
    pub river_y: Vec<i32>,
    pub river_off: Vec<u32>,
    pub river_rank: Vec<u8>,
    /// Roads (class 0 major highway, 1 secondary, 2 other) and railways (0 main, 1 regional, 2 other).
    pub roads: LineLayer,
    pub rails: LineLayer,
    /// Built-up areas as one triangle mesh.
    pub urban_vx: Vec<i32>,
    pub urban_vy: Vec<i32>,
    pub urban_tri: Vec<u32>,
    /// One per country (the map's country order) and one per region of a country.
    pub countries: Labels,
    pub regions: Labels,
}

impl RenderData {
    /// The ring of one cell as (x, y) points.
    pub fn cell_ring(&self, cell: usize) -> impl Iterator<Item = (i32, i32)> + '_ {
        let (a, b) = (self.cell_ring_off[cell] as usize, self.cell_ring_off[cell + 1] as usize);
        (a..b).map(move |i| (self.cell_ring_x[i], self.cell_ring_y[i]))
    }
    /// Bounding box of everything drawn at the finest level: (min_x, min_y, max_x, max_y).
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        let l = self.levels.last().expect("render file has levels");
        let r = |v: &[i32]| (v.iter().copied().min().unwrap_or(0), v.iter().copied().max().unwrap_or(0));
        let (x0, x1) = r(&l.vx);
        let (y0, y1) = r(&l.vy);
        (x0, y0, x1, y1)
    }
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
    let data = RenderData {
        checksum: crc,
        levels,
        cell_seg,
        cell_mx: d.i32s("cell_mx")?,
        cell_my: d.i32s("cell_my")?,
        cell_ring_off: d.u32s("cell_ring_off")?,
        cell_ring_x: d.i32s("cell_ring_x")?,
        cell_ring_y: d.i32s("cell_ring_y")?,
        province_mx: d.i32s("province_mx")?,
        province_my: d.i32s("province_my")?,
        relief: read_relief(&d)?,
        river_x: d.i32s("river_x")?,
        river_y: d.i32s("river_y")?,
        river_off: d.u32s("river_off")?,
        river_rank: d.u8s("river_rank")?,
        roads: read_lines(&d, "road")?,
        rails: read_lines(&d, "rail")?,
        urban_vx: d.i32s("urban_vx")?,
        urban_vy: d.i32s("urban_vy")?,
        urban_tri: d.u32s("urban_tri")?,
        countries: read_labels(&d, "country")?,
        regions: read_labels(&d, "region")?,
    };
    let nu = data.urban_vx.len();
    if data.urban_vy.len() != nu || !data.urban_tri.len().is_multiple_of(3) || data.urban_tri.iter().any(|&i| i as usize >= nu) {
        return Err(d.bad("urban", "urban mesh is inconsistent".into()));
    }
    if data.river_y.len() != data.river_x.len() || data.river_off.len() != data.river_rank.len() + 1 || data.river_off.last().map(|&e| e as usize) != Some(data.river_x.len()) || data.river_off.windows(2).any(|w| w[1] < w[0] + 2) {
        return Err(d.bad("rivers", "river arrays are inconsistent".into()));
    }
    let n = data.cell_mx.len();
    let off = &data.cell_ring_off;
    if data.cell_my.len() != n || off.len() != n + 1 || off.first() != Some(&0) || off.last().map(|&e| e as usize) != Some(data.cell_ring_x.len()) || data.cell_ring_y.len() != data.cell_ring_x.len() || off.windows(2).any(|w| w[1] < w[0] + 3) {
        return Err(d.bad("cell rings", "ring arrays are inconsistent (every cell needs at least 3 points)".into()));
    }
    if data.province_mx.len() != data.province_my.len() {
        return Err(d.bad("province centres", "x and y differ in length".into()));
    }
    Ok(data)
}

fn read_relief(d: &Dir) -> Result<Relief, DataError> {
    let info = d.i32s("relief_info")?;
    if info.len() != 6 || info[0] <= 0 || info[1] <= 0 || info[4] <= info[2] || info[5] <= info[3] {
        return Err(d.bad("relief_info", "expected width, height and a non-empty box".into()));
    }
    let pixels = d.u8s("relief")?;
    if pixels.len() != info[0] as usize * info[1] as usize {
        return Err(d.bad("relief", "raster size does not match relief_info".into()));
    }
    Ok(Relief { width: info[0] as u32, height: info[1] as u32, x0: info[2], y0: info[3], x1: info[4], y1: info[5], pixels })
}

fn read_lines(d: &Dir, prefix: &str) -> Result<LineLayer, DataError> {
    let l = LineLayer {
        x: d.i32s(&format!("{prefix}_x"))?,
        y: d.i32s(&format!("{prefix}_y"))?,
        off: d.u32s(&format!("{prefix}_off"))?,
        class: d.u8s(&format!("{prefix}_class"))?,
    };
    if l.y.len() != l.x.len() || l.off.len() != l.class.len() + 1 || l.off.first() != Some(&0) || l.off.last().map(|&e| e as usize) != Some(l.x.len()) || l.off.windows(2).any(|w| w[1] < w[0] + 2) || l.class.windows(2).any(|w| w[1] < w[0]) {
        return Err(d.bad(prefix, "line arrays are inconsistent or not sorted by class".into()));
    }
    Ok(l)
}

fn read_labels(d: &Dir, prefix: &str) -> Result<Labels, DataError> {
    let off = d.u32s(&format!("{prefix}_name_off"))?;
    let bytes = d.u8s(&format!("{prefix}_name_bytes"))?;
    let x = d.i32s(&format!("{prefix}_x"))?;
    let y = d.i32s(&format!("{prefix}_y"))?;
    let area_km2 = d.u32s(&format!("{prefix}_area_km2"))?;
    let n = x.len();
    if y.len() != n || area_km2.len() != n || off.len() != n + 1 || off.first() != Some(&0) || off.last().map(|&e| e as usize) != Some(bytes.len()) || off.windows(2).any(|w| w[1] < w[0]) {
        return Err(d.bad(prefix, "label arrays are inconsistent".into()));
    }
    let mut names = Vec::with_capacity(n);
    for w in off.windows(2) {
        let s = std::str::from_utf8(&bytes[w[0] as usize..w[1] as usize]).map_err(|_| d.bad(prefix, "a name is not valid UTF-8".into()))?;
        names.push(s.to_string());
    }
    Ok(Labels { names, x, y, area_km2 })
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
        p.extend_from_slice(&44u32.to_le_bytes());
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
        // one cell: the square, centre (5, 5)
        array(&mut p, "cell_mx", 5, &le32(&[5]), 1);
        array(&mut p, "cell_my", 5, &le32(&[5]), 1);
        array(&mut p, "cell_ring_off", 3, &u(&[0, 4]), 2);
        array(&mut p, "cell_ring_x", 5, &le32(&[0, 10, 10, 0]), 4);
        array(&mut p, "cell_ring_y", 5, &le32(&[0, 0, 10, 10]), 4);
        array(&mut p, "province_mx", 5, &le32(&[5]), 1);
        array(&mut p, "province_my", 5, &le32(&[5]), 1);
        array(&mut p, "relief_info", 5, &le32(&[2, 1, -10, -10, 10, 10]), 6);
        array(&mut p, "relief", 1, &[128, 140], 2);
        array(&mut p, "river_x", 5, &le32(&[0, 5]), 2);
        array(&mut p, "river_y", 5, &le32(&[0, 5]), 2);
        array(&mut p, "river_off", 3, &u(&[0, 2]), 2);
        array(&mut p, "river_rank", 1, &[3], 1);
        for pre in ["road", "rail"] {
            array(&mut p, &format!("{pre}_x"), 5, &le32(&[0, 5]), 2);
            array(&mut p, &format!("{pre}_y"), 5, &le32(&[0, 5]), 2);
            array(&mut p, &format!("{pre}_off"), 3, &u(&[0, 2]), 2);
            array(&mut p, &format!("{pre}_class"), 1, &[0], 1);
        }
        array(&mut p, "urban_vx", 5, &le32(&[0, 1, 0]), 3);
        array(&mut p, "urban_vy", 5, &le32(&[0, 0, 1]), 3);
        array(&mut p, "urban_tri", 3, &u(&[0, 1, 2]), 3);
        for pre in ["country", "region"] {
            array(&mut p, &format!("{pre}_name_off"), 3, &u(&[0, 3]), 2);
            array(&mut p, &format!("{pre}_name_bytes"), 1, b"Abc", 3);
            array(&mut p, &format!("{pre}_x"), 5, &le32(&[5]), 1);
            array(&mut p, &format!("{pre}_y"), 5, &le32(&[5]), 1);
            array(&mut p, &format!("{pre}_area_km2"), 3, &u(&[100]), 1);
        }
        let mut f = b"MRRD".to_vec();
        f.extend_from_slice(&5u32.to_le_bytes());
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
        assert_eq!(r.cell_ring(0).collect::<Vec<_>>(), vec![(0, 0), (10, 0), (10, 10), (0, 10)]);
        assert_eq!(r.bounds(), (0, 0, 10, 10));
        assert_eq!(r.relief.pixels, vec![128, 140]);
        assert_eq!(r.river_off, vec![0, 2]);
        assert_eq!(r.countries.names, vec!["Abc".to_string()]);
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
        v2[4] = 9;
        assert!(msg(&v2).contains("version 9"));
    }
}
