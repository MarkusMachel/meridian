//! Loader for the baked map file written by crates/map-builder/bake_map.py.
//!
//! Container (little-endian): magic "MRMP" | format_version u32 | kind u32 (2) |
//! payload_len u64 | checksum u64 (CRC-32 of the payload, zero-extended).
//! Payload: u32 array count, then per array: name_len u8 | name | dtype u8 | count u64 | data.
//! dtype: 1 u8, 2 u16, 3 u32, 4 i16, 5 i32, 6 u64.
//!
//! Version 2 (map rewrite, phase M1): the cells are hexagons on the globe. Adds `cell_h3`,
//! `cell_xyz` and `node_xyz`; distances are over the ground of the sphere.

use crate::error::{DataError, ErrorKind};
use meridian_core::{Map, RawMap};
use std::collections::HashMap;
use std::path::Path;

pub const MAP_FORMAT_VERSION: u32 = 2;
const KIND_MAP: u32 = 2;
const HEADER_LEN: usize = 28;

struct Arrays<'a> {
    file: &'a str,
    by_name: HashMap<&'a str, (u8, usize, &'a [u8])>, // name -> (dtype, count, bytes)
}

impl<'a> Arrays<'a> {
    fn bad(&self, item: &str, msg: String) -> DataError {
        DataError::new(self.file, item, ErrorKind::BadBinary, msg)
    }
    fn raw(&self, name: &str, dtype: u8, width: usize) -> Result<(usize, &'a [u8]), DataError> {
        let (dt, count, bytes) = *self.by_name.get(name).ok_or_else(|| self.bad(name, "array is missing from the file".into()))?;
        if dt != dtype {
            return Err(self.bad(name, format!("array has dtype {dt}, expected {dtype}")));
        }
        debug_assert_eq!(bytes.len(), count * width);
        Ok((count, bytes))
    }
    fn u8s(&self, name: &str) -> Result<Vec<u8>, DataError> {
        Ok(self.raw(name, 1, 1)?.1.to_vec())
    }
    fn u16s(&self, name: &str) -> Result<Vec<u16>, DataError> {
        Ok(self.raw(name, 2, 2)?.1.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
    }
    fn u32s(&self, name: &str) -> Result<Vec<u32>, DataError> {
        Ok(self.raw(name, 3, 4)?.1.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
    fn i16s(&self, name: &str) -> Result<Vec<i16>, DataError> {
        Ok(self.raw(name, 4, 2)?.1.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
    }
    fn i32s(&self, name: &str) -> Result<Vec<i32>, DataError> {
        Ok(self.raw(name, 5, 4)?.1.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
    fn u64s(&self, name: &str) -> Result<Vec<u64>, DataError> {
        Ok(self.raw(name, 6, 8)?.1.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect())
    }
    /// A string table stored as `<base>_blob` (u8) and `<base>_off` (u32).
    fn strings(&self, base: &str) -> Result<Vec<String>, DataError> {
        let blob = self.u8s(&format!("{base}_blob"))?;
        let off = self.u32s(&format!("{base}_off"))?;
        let mut out = Vec::with_capacity(off.len().saturating_sub(1));
        for w in off.windows(2) {
            let (a, b) = (w[0] as usize, w[1] as usize);
            if a > b || b > blob.len() {
                return Err(self.bad(base, "string offsets are out of range".into()));
            }
            out.push(String::from_utf8(blob[a..b].to_vec()).map_err(|_| self.bad(base, "string is not valid UTF-8".into()))?);
        }
        Ok(out)
    }
}

/// Parse a baked map from bytes. `file` is only used to label errors.
pub fn parse_map(file: &str, bytes: &[u8]) -> Result<Map, Vec<DataError>> {
    let one = |e: DataError| vec![e];
    let bad = |msg: String| one(DataError::new(file, "-", ErrorKind::BadBinary, msg));
    if bytes.len() < HEADER_LEN || &bytes[0..4] != b"MRMP" {
        return Err(bad("not a Meridian map file (bad magic number)".into()));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != MAP_FORMAT_VERSION {
        return Err(one(DataError::new(file, "-", ErrorKind::WrongVersion, format!("map format version {version}, this build reads {MAP_FORMAT_VERSION}"))));
    }
    if u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != KIND_MAP {
        return Err(bad("file is not a map (wrong kind)".into()));
    }
    let len = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let sum = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
    let payload = &bytes[HEADER_LEN..];
    if payload.len() as u64 != len {
        return Err(bad(format!("payload is {} bytes, header says {len} (truncated?)", payload.len())));
    }
    let crc = crc32fast::hash(payload);
    if crc as u64 != sum {
        return Err(bad("checksum mismatch (file is corrupt)".into()));
    }

    // directory of arrays
    let mut by_name = HashMap::new();
    let mut pos = 0usize;
    let take = |pos: &mut usize, n: usize| -> Result<&[u8], DataError> {
        let end = pos.checked_add(n).filter(|&e| e <= payload.len()).ok_or_else(|| DataError::new(file, "-", ErrorKind::BadBinary, "array table runs past the end of the file"))?;
        let s = &payload[*pos..end];
        *pos = end;
        Ok(s)
    };
    let count = u32::from_le_bytes(take(&mut pos, 4).map_err(one)?.try_into().unwrap());
    for _ in 0..count {
        let nl = take(&mut pos, 1).map_err(one)?[0] as usize;
        let name = std::str::from_utf8(take(&mut pos, nl).map_err(one)?).map_err(|_| bad("array name is not text".into()))?;
        let dtype = take(&mut pos, 1).map_err(one)?[0];
        let n = u64::from_le_bytes(take(&mut pos, 8).map_err(one)?.try_into().unwrap()) as usize;
        let width = match dtype {
            1 => 1,
            2 | 4 => 2,
            3 | 5 => 4,
            6 => 8,
            other => return Err(bad(format!("array {name}: unknown dtype {other}"))),
        };
        let nbytes = n.checked_mul(width).ok_or_else(|| bad(format!("array {name}: size overflows")))?;
        let data = take(&mut pos, nbytes).map_err(one)?;
        if by_name.insert(name, (dtype, n, data)).is_some() {
            return Err(bad(format!("array {name} appears twice")));
        }
    }
    if pos != payload.len() {
        return Err(bad("extra bytes after the last array".into()));
    }

    let a = Arrays { file, by_name };
    let raw = (|| -> Result<RawMap, DataError> {
        Ok(RawMap {
            checksum: crc,
            state_tags: a.strings("state_tags")?,
            state_names: a.strings("state_names")?,
            choke_names: a.strings("choke_names")?,
            choke_built: a.i16s("choke_built")?,
            site_names: a.strings("site_names")?,
            province_state: a.u16s("province_state")?,
            province_x: a.i32s("province_x")?,
            province_y: a.i32s("province_y")?,
            province_area_ha: a.u32s("province_area_ha")?,
            province_pop: a.u32s("province_pop")?,
            province_cell_off: a.u32s("province_cell_off")?,
            province_cells: a.u32s("province_cells")?,
            zone_x: a.i32s("zone_x")?,
            zone_y: a.i32s("zone_y")?,
            zone_area_ha: a.u32s("zone_area_ha")?,
            zone_body: a.u16s("zone_body")?,
            g_off: a.u32s("g_off")?,
            g_dst: a.u32s("g_dst")?,
            g_kind: a.u8s("g_kind")?,
            g_weight: a.u32s("g_weight")?,
            g_strait: a.u16s("g_strait")?,
            g_cost_m: a.u32s("g_cost_m")?,
            cell_state: a.u16s("cell_state")?,
            cell_province: a.u32s("cell_province")?,
            cell_x: a.i32s("cell_x")?,
            cell_y: a.i32s("cell_y")?,
            cell_area_ha: a.u32s("cell_area_ha")?,
            cell_landmass: a.u32s("cell_landmass")?,
            cell_elev_m: a.i16s("cell_elev_m")?,
            cell_relief_m: a.u16s("cell_relief_m")?,
            cell_terrain: a.u8s("cell_terrain")?,
            cell_urban: a.u8s("cell_urban")?,
            cell_flags: a.u8s("cell_flags")?,
            cell_zone: a.u32s("cell_zone")?,
            cell_pop: a.u32s("cell_pop")?,
            c_off: a.u32s("c_off")?,
            c_dst: a.u32s("c_dst")?,
            c_len_m: a.u32s("c_len_m")?,
            c_flags: a.u8s("c_flags")?,
            c_dist_m: a.u32s("c_dist_m")?,
            site_kind: a.u8s("site_kind")?,
            site_cell: a.u32s("site_cell")?,
            site_province: a.u32s("site_province")?,
            site_size: a.u32s("site_size")?,
            site_built: a.i16s("site_built")?,
            site_destroyed: a.i16s("site_destroyed")?,
            site_verified: a.u8s("site_verified")?,
            cell_h3: a.u64s("cell_h3")?,
            cell_xyz: a.i32s("cell_xyz")?,
            node_xyz: a.i32s("node_xyz")?,
        })
    })()
    .map_err(one)?;
    Map::from_raw(raw).map_err(|errs| errs.into_iter().map(|e| DataError::new(file, "map", ErrorKind::BadBinary, e.0)).collect())
}

pub fn load_map_file(path: &Path) -> Result<Map, Vec<DataError>> {
    let name = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|e| vec![DataError::new(&name, "-", ErrorKind::Io, e.to_string())])?;
    parse_map(&name, &bytes)
}
