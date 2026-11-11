//! Loader for the baked map file written by crates/map-builder/bake_map.py.
//!
//! Container (little-endian): magic "MRMP" | format_version u32 | kind u32 (2) |
//! payload_len u64 | checksum u64 (CRC-32 of the payload, zero-extended).
//! Payload: u32 array count, then per array: name_len u8 | name | dtype u8 | count u64 | data.
//! dtype: 1 u8, 2 u16, 3 u32, 4 i16, 5 i32, 6 u64.
//!
//! Version 2 (map rewrite, phase M1): the cells are hexagons on the globe. Adds `cell_h3`,
//! `cell_xyz` and `node_xyz`; distances are over the ground of the sphere.
//!
//! Version 3: `c_road` and `c_rail`, the level of the road and of the railway on each cell edge.

use crate::error::{DataError, ErrorKind};
use meridian_core::{Map, RawMap};
use std::collections::HashMap;
use std::path::Path;

pub const MAP_FORMAT_VERSION: u32 = 3;
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
    finish(file, parse_raw(file, bytes)?)
}

fn finish(file: &str, raw: RawMap) -> Result<Map, Vec<DataError>> {
    Map::from_raw(raw).map_err(|errs| errs.into_iter().map(|e| DataError::new(file, "map", ErrorKind::BadBinary, e.0)).collect())
}

fn parse_raw(file: &str, bytes: &[u8]) -> Result<RawMap, Vec<DataError>> {
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
            c_road: a.u8s("c_road")?,
            c_rail: a.u8s("c_rail")?,
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
    Ok(raw)
}

/// Load the baked map, with every regional network pack found beside it laid over it: the
/// files `packs/*.network.bin` in the map's folder, in name order. A pack replaces the roads
/// and railways inside its region with finer ones, so the map every tool loads (client,
/// runner, tests) is the same one. The map's checksum takes the packs in, so a save made with
/// a pack does not load without it.
pub fn load_map_file(path: &Path) -> Result<Map, Vec<DataError>> {
    let (name, bytes) = read_file(path)?;
    let mut raw = parse_raw(&name, &bytes)?;
    drop(bytes);
    let mut packs: Vec<std::path::PathBuf> = path
        .parent()
        .map(|dir| dir.join("packs"))
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".network.bin")))
        .collect();
    packs.sort();
    for pack in packs {
        apply_network_pack(&mut raw, &load_network_pack_file(&pack)?);
    }
    finish(&name, raw)
}

// ---- regional network packs: finer roads and railways for the simulation ----

const KIND_NETWORK_PACK: u32 = 7;

/// The roads and railways of a region as links between hexagons, with levels (see
/// `RawMap::c_road`). Unlike the other pack files this one feeds the simulation.
#[derive(Clone, Debug, Default)]
pub struct NetworkPack {
    /// CRC-32 of the file's contents.
    pub checksum: u32,
    /// The H3 numbers of every hexagon in the region, ascending. Links between two of these
    /// are replaced by the pack's (or removed, where the pack has none).
    pub cells: Vec<u64>,
    /// The links, ascending by (a, b) with a < b, as H3 numbers, and their levels.
    pub link_a: Vec<u64>,
    pub link_b: Vec<u64>,
    pub road: Vec<u8>,
    pub rail: Vec<u8>,
}

pub fn load_network_pack_file(path: &Path) -> Result<NetworkPack, Vec<DataError>> {
    let (name, bytes) = read_file(path)?;
    let (a, crc) = open_container(&name, &bytes, KIND_NETWORK_PACK, 1, "network pack")?;
    let read = || -> Result<NetworkPack, DataError> {
        let p = NetworkPack { checksum: crc, cells: a.u64s("cells")?, link_a: a.u64s("link_a")?, link_b: a.u64s("link_b")?, road: a.u8s("road")?, rail: a.u8s("rail")? };
        let n = p.link_a.len();
        let pairs_ascend = (1..n).all(|i| (p.link_a[i - 1], p.link_b[i - 1]) < (p.link_a[i], p.link_b[i]));
        let ok = p.link_b.len() == n && p.road.len() == n && p.rail.len() == n && p.cells.windows(2).all(|w| w[0] < w[1]) && pairs_ascend && (0..n).all(|i| p.link_a[i] < p.link_b[i] && p.road[i] <= 5 && p.rail[i] <= 5);
        if !ok {
            return Err(a.bad("-", "the arrays do not fit together or are not in order".into()));
        }
        Ok(p)
    };
    read().map_err(|e| vec![e])
}

/// Lay a network pack over a map: inside the pack's region the links are the pack's. Whole
/// numbers and sorted lists only, so every machine gets the same map.
pub fn apply_network_pack(raw: &mut RawMap, pack: &NetworkPack) {
    use meridian_core::map::{cell_edge_flag, cell_flag};
    if raw.c_road.len() != raw.c_dst.len() || raw.cell_h3.len() != raw.cell_state.len() {
        return; // a map without link levels or hexagon numbers: nothing to lay it over
    }
    let n = raw.cell_h3.len();
    let covered: Vec<bool> = raw.cell_h3.iter().map(|h| pack.cells.binary_search(h).is_ok()).collect();
    for a in 0..n {
        if !covered[a] {
            continue;
        }
        let (mut any_road, mut any_rail) = (false, false);
        for e in raw.c_off[a] as usize..raw.c_off[a + 1] as usize {
            let b = raw.c_dst[e] as usize;
            if covered[b] {
                let (ha, hb) = (raw.cell_h3[a].min(raw.cell_h3[b]), raw.cell_h3[a].max(raw.cell_h3[b]));
                // find the link in the pack's list, which is sorted by (a, b)
                let at = pack.link_a.partition_point(|&x| x < ha);
                let found = (at..pack.link_a.len()).take_while(|&i| pack.link_a[i] == ha).find(|&i| pack.link_b[i] == hb);
                let (road, rail) = found.map_or((0, 0), |i| (pack.road[i], pack.rail[i]));
                raw.c_road[e] = road;
                raw.c_rail[e] = rail;
                raw.c_flags[e] = (raw.c_flags[e] & !(cell_edge_flag::ROAD | cell_edge_flag::RAIL)) | if road > 0 { cell_edge_flag::ROAD } else { 0 } | if rail > 0 { cell_edge_flag::RAIL } else { 0 };
            }
            any_road |= raw.c_road[e] > 0;
            any_rail |= raw.c_rail[e] > 0;
        }
        raw.cell_flags[a] = (raw.cell_flags[a] & !(cell_flag::ROAD | cell_flag::RAIL)) | if any_road { cell_flag::ROAD } else { 0 } | if any_rail { cell_flag::RAIL } else { 0 };
    }
    // the map is now a different map: mix the pack into its checksum
    let mut h = crc32fast::Hasher::new();
    h.update(&raw.checksum.to_le_bytes());
    h.update(&pack.checksum.to_le_bytes());
    raw.checksum = h.finalize();
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

/// The ground of the globe: two pictures of the world (longitude by latitude, left edge
/// 180° W, top edge 90° N), the height of the land for the lighting and its painted colour.
#[derive(Clone, Debug, Default)]
pub struct GlobeGround {
    pub width: u32,
    pub height: u32,
    /// Metres above the sea, one per pixel.
    pub heights: Vec<u16>,
    /// Red, green, blue, 255: four bytes per pixel.
    pub color: Vec<u8>,
}

/// The ground cover of a regional pack: a fine picture cut in square tiles, each pixel 0 sea,
/// 1 inland water, 2 woods, 3 built-up, 4 industry, 6 other land. A tile that is not stored is
/// all sea.
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

const KIND_GLOBE_GROUND: u32 = 5;
const KIND_PACK_COVER: u32 = 6;

fn read_file(path: &Path) -> Result<(String, Vec<u8>), Vec<DataError>> {
    let name = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|e| vec![DataError::new(&name, "-", ErrorKind::Io, e.to_string())])?;
    Ok((name, bytes))
}

pub fn load_globe_ground_file(path: &Path) -> Result<GlobeGround, Vec<DataError>> {
    let (name, bytes) = read_file(path)?;
    let (a, _) = open_container(&name, &bytes, KIND_GLOBE_GROUND, 2, "globe ground file")?;
    let read = || -> Result<GlobeGround, DataError> {
        let size = a.u32s("size")?;
        let (heights, color) = (a.u16s("height")?, a.u8s("color")?);
        let pixels = if size.len() == 2 { size[0] as u64 * size[1] as u64 } else { 0 };
        if pixels == 0 || heights.len() as u64 != pixels || color.len() as u64 != 4 * pixels {
            return Err(a.bad("-", "the pictures are not the size the file says".into()));
        }
        Ok(GlobeGround { width: size[0], height: size[1], heights, color })
    };
    read().map_err(|e| vec![e])
}

pub fn load_pack_cover_file(path: &Path) -> Result<PackCover, Vec<DataError>> {
    let (name, bytes) = read_file(path)?;
    let (a, _) = open_container(&name, &bytes, KIND_PACK_COVER, 2, "pack ground cover file")?;
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

#[cfg(test)]
mod network_tests {
    use super::*;
    use meridian_core::map::{cell_edge_flag, cell_flag};

    /// Three cells in a row (0 - 1 - 2) with hexagon numbers 10, 20, 30; a level-2 road on both
    /// links and a railway on the first.
    fn row() -> RawMap {
        RawMap {
            checksum: 7,
            cell_h3: vec![10, 20, 30],
            cell_state: vec![0, 0, 0],
            cell_flags: vec![cell_flag::ROAD | cell_flag::RAIL, cell_flag::ROAD | cell_flag::RAIL, cell_flag::ROAD],
            c_off: vec![0, 1, 3, 4],
            c_dst: vec![1, 0, 2, 1],
            c_flags: vec![cell_edge_flag::ROAD | cell_edge_flag::RAIL, cell_edge_flag::ROAD | cell_edge_flag::RAIL, cell_edge_flag::ROAD | cell_edge_flag::RIVER, cell_edge_flag::ROAD | cell_edge_flag::RIVER],
            c_road: vec![2, 2, 2, 2],
            c_rail: vec![1, 1, 0, 0],
            ..Default::default()
        }
    }

    #[test]
    fn a_pack_replaces_the_links_inside_its_region_and_only_there() {
        // the region is cells 10 and 20: their link becomes a motorway with no railway;
        // the link from 20 to 30 leaves the region and stays as it was
        let pack = NetworkPack { checksum: 99, cells: vec![10, 20], link_a: vec![10], link_b: vec![20], road: vec![5], rail: vec![0] };
        let mut m = row();
        apply_network_pack(&mut m, &pack);
        assert_eq!(m.c_road, vec![5, 5, 2, 2]);
        assert_eq!(m.c_rail, vec![0, 0, 0, 0]);
        assert_eq!(m.c_flags[0], cell_edge_flag::ROAD, "the railway flag went with the railway");
        assert_eq!(m.c_flags[2], cell_edge_flag::ROAD | cell_edge_flag::RIVER, "the river and the outside link are untouched");
        assert_eq!(m.cell_flags, vec![cell_flag::ROAD, cell_flag::ROAD, cell_flag::ROAD]);
        assert_ne!(m.checksum, 7, "a map with a pack is a different map");

        // a region the pack has no links for loses its links
        let empty = NetworkPack { checksum: 5, cells: vec![10, 20, 30], ..Default::default() };
        let mut m = row();
        apply_network_pack(&mut m, &empty);
        assert_eq!((m.c_road.clone(), m.c_rail.clone()), (vec![0; 4], vec![0; 4]));
        assert_eq!(m.cell_flags, vec![0, 0, 0]);
        assert_eq!(m.c_flags[2], cell_edge_flag::RIVER);

        // the same pack on the same map always gives the same checksum
        let (mut a, mut b) = (row(), row());
        apply_network_pack(&mut a, &pack);
        apply_network_pack(&mut b, &pack);
        assert_eq!(a.checksum, b.checksum);
    }
}
