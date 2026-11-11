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
    fn has(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
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
/// Check a container's header and checksum and list its arrays. `kind` and `version` are the
/// ones the caller reads; `what` names the kind of file in error messages.
fn open_container<'a>(file: &'a str, bytes: &'a [u8], kind: u32, version_wanted: u32, what: &str) -> Result<(Arrays<'a>, u32), Vec<DataError>> {
    let one = |e: DataError| vec![e];
    let bad = |msg: String| one(DataError::new(file, "-", ErrorKind::BadBinary, msg));
    if bytes.len() < HEADER_LEN || &bytes[0..4] != b"MRMP" {
        return Err(bad("not a Meridian map file (bad magic number)".into()));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != version_wanted {
        return Err(one(DataError::new(file, "-", ErrorKind::WrongVersion, format!("{what} format version {version}, this build reads {version_wanted}"))));
    }
    if u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != kind {
        return Err(bad(format!("file is not a {what} (wrong kind)")));
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

    Ok((Arrays { file, by_name }, crc))
}

pub fn parse_map(file: &str, bytes: &[u8]) -> Result<Map, Vec<DataError>> {
    let one = |e: DataError| vec![e];
    let (a, crc) = open_container(file, bytes, KIND_MAP, MAP_FORMAT_VERSION, "map")?;
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

// ---- the globe's drawn lines (map rewrite M3) ----

pub const GLOBE_LINES_VERSION: u32 = 1;
const KIND_GLOBE_LINES: u32 = 4;

/// Real coastlines, borders, rivers, roads and railways for the globe view, as short straight
/// pieces between places on the sphere. Drawing only: the engine never reads this file.
#[derive(Clone, Debug, Default)]
pub struct GlobeLines {
    /// Both ends of each piece: three whole metres from the Earth's centre each.
    pub seg_a: Vec<i32>,
    pub seg_b: Vec<i32>,
    /// What each piece is (see `crates/map-builder/build_globe_lines.py`).
    pub seg_style: Vec<u8>,
    /// The pieces are grouped in buckets: bucket `k` is pieces `bucket_off[k]..bucket_off[k + 1]`,
    /// all of one detail tier and around one place.
    pub bucket_off: Vec<u32>,
    /// 0 = shown from the whole globe down, 1 = from a continent, 2 = from a country, 3 = from a region.
    pub bucket_tier: Vec<u8>,
    /// The middle of each bucket's area on the sphere (three numbers each).
    pub bucket_xyz: Vec<i32>,
    /// How far a bucket's pieces reach from its middle, in millionths of a radian.
    pub bucket_reach_microrad: u32,
    /// A regional pack only: the outline of the area it covers, as longitude and latitude in
    /// millionths of a degree, ring after ring; ring `k` is points `cover_off[k]..cover_off[k + 1]`.
    /// Inside it the pack's lines replace the world's coarse roads, railways and small rivers.
    pub cover: Vec<i32>,
    pub cover_off: Vec<u32>,
}

/// Shaded relief for the globe: a picture of the world (longitude by latitude, left edge
/// 180° W, top edge 90° N), 128 for flat ground, lighter facing the light.
#[derive(Clone, Debug, Default)]
pub struct GlobeRelief {
    pub width: u32,
    pub height: u32,
    pub shade: Vec<u8>,
}

/// The ground cover of a regional pack: a fine picture cut in square tiles, each pixel 0
/// nothing, 1 water, 2 woods, 3 built-up, 4 industry.
#[derive(Clone, Debug, Default)]
pub struct PackCover {
    /// West and north edge in millionths of a degree.
    pub west_e6: i32,
    pub north_e6: i32,
    /// Pixel width and height in billionths of a degree.
    pub pixel_lon_e9: i32,
    pub pixel_lat_e9: i32,
    pub width: u32,
    pub height: u32,
    /// Tile size in pixels, and how many tiles across and down.
    pub tile: u32,
    pub across: u32,
    pub down: u32,
    /// For each tile, row by row from the north-west: which stored tile it is, or `u32::MAX`.
    pub tile_layer: Vec<u32>,
    /// The stored tiles one after another, `tile * tile` bytes each.
    pub tiles: Vec<u8>,
}

const KIND_GLOBE_RELIEF: u32 = 5;
const KIND_PACK_COVER: u32 = 6;

fn read_file(path: &Path) -> Result<(String, Vec<u8>), Vec<DataError>> {
    let name = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|e| vec![DataError::new(&name, "-", ErrorKind::Io, e.to_string())])?;
    Ok((name, bytes))
}

pub fn load_globe_relief_file(path: &Path) -> Result<GlobeRelief, Vec<DataError>> {
    let (name, bytes) = read_file(path)?;
    let (a, _) = open_container(&name, &bytes, KIND_GLOBE_RELIEF, 1, "globe relief file")?;
    let read = || -> Result<GlobeRelief, DataError> {
        let size = a.u32s("size")?;
        let shade = a.u8s("shade")?;
        if size.len() != 2 || size[0] == 0 || shade.len() as u64 != size[0] as u64 * size[1] as u64 {
            return Err(a.bad("-", "the picture is not the size the file says".into()));
        }
        Ok(GlobeRelief { width: size[0], height: size[1], shade })
    };
    read().map_err(|e| vec![e])
}

pub fn load_pack_cover_file(path: &Path) -> Result<PackCover, Vec<DataError>> {
    let (name, bytes) = read_file(path)?;
    let (a, _) = open_container(&name, &bytes, KIND_PACK_COVER, 1, "pack ground cover file")?;
    let read = || -> Result<PackCover, DataError> {
        let g = a.i32s("grid")?;
        if g.len() != 9 || g[2..].iter().any(|&v| v <= 0) {
            return Err(a.bad("grid", "nine positive numbers expected".into()));
        }
        let c = PackCover { west_e6: g[0], north_e6: g[1], pixel_lon_e9: g[2], pixel_lat_e9: g[3], width: g[4] as u32, height: g[5] as u32, tile: g[6] as u32, across: g[7] as u32, down: g[8] as u32, tile_layer: a.u32s("tile_layer")?, tiles: a.u8s("tiles")? };
        let per_tile = c.tile as usize * c.tile as usize;
        let stored = c.tiles.len() / per_tile;
        let ok = c.tile_layer.len() == (c.across * c.down) as usize && c.tiles.len() == stored * per_tile && stored > 0 && c.tile_layer.iter().all(|&l| l == u32::MAX || (l as usize) < stored) && c.across * c.tile >= c.width && c.down * c.tile >= c.height;
        if !ok {
            return Err(a.bad("-", "the arrays do not fit together".into()));
        }
        Ok(c)
    };
    read().map_err(|e| vec![e])
}

pub fn load_globe_lines_file(path: &Path) -> Result<GlobeLines, Vec<DataError>> {
    let (name, bytes) = read_file(path)?;
    let (a, _) = open_container(&name, &bytes, KIND_GLOBE_LINES, GLOBE_LINES_VERSION, "globe lines file")?;
    let read = || -> Result<GlobeLines, DataError> {
        let g = GlobeLines { seg_a: a.i32s("seg_a")?, seg_b: a.i32s("seg_b")?, seg_style: a.u8s("seg_style")?, bucket_off: a.u32s("bucket_off")?, bucket_tier: a.u8s("bucket_tier")?,
            bucket_xyz: a.i32s("bucket_xyz")?,
            // the world file has none of these three
            bucket_reach_microrad: if a.has("info") { a.u32s("info")?.first().copied().unwrap_or(40_000) } else { 40_000 },
            cover: if a.has("cover") { a.i32s("cover")? } else { Vec::new() },
            cover_off: if a.has("cover_off") { a.u32s("cover_off")? } else { Vec::new() },
        };
        let (n, b) = (g.seg_style.len(), g.bucket_tier.len());
        let ok = g.seg_a.len() == 3 * n && g.seg_b.len() == 3 * n && g.bucket_xyz.len() == 3 * b && g.bucket_off.len() == b + 1 && g.bucket_off.windows(2).all(|w| w[0] <= w[1]) && g.bucket_off.last().is_some_and(|&e| e as usize == n)
            && g.cover.len().is_multiple_of(2)
            && g.cover_off.windows(2).all(|w| w[0] <= w[1])
            && g.cover_off.last().is_none_or(|&e| e as usize * 2 == g.cover.len());
        if !ok {
            return Err(a.bad("-", "the arrays do not fit together".into()));
        }
        Ok(g)
    };
    read().map_err(|e| vec![e])
}
