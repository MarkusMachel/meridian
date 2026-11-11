//! The static world map: provinces, sea zones, tactical cells, sites, and their graphs.
//!
//! The map is read-only data produced offline (see crates/map-builder) and parsed by
//! meridian-data; this module only validates it, indexes it, and answers questions
//! (neighbours, range, paths). Everything is integer math and deterministic.
//!
//! Node numbering in the strategic graph: land provinces first (0..P), then sea zones (P..P+Z).
//! Coordinates are Equal Earth metres (i32), areas are hectares, distances are metres.

use crate::ids::{CellId, ProvinceId};
use crate::sphere::{chord_m, chord_squared, ground_from_chord_squared, ground_m, Place};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::ops::Deref;

pub const NO_ZONE: u32 = u32::MAX;

/// Strategic-graph edge kinds (values match the baked file).
pub mod edge_kind {
    pub const LAND: u8 = 0;
    pub const COAST: u8 = 1;
    pub const STRAIT: u8 = 2;
    pub const SEA: u8 = 3;
}
/// Cell terrain classes (values match the baked file).
pub mod terrain {
    pub const PLAINS: u8 = 0;
    pub const HILLS: u8 = 1;
    pub const MOUNTAINS: u8 = 2;
    pub const HIGH_MOUNTAINS: u8 = 3;
    pub const DESERT: u8 = 4;
    pub const ICE: u8 = 5;
    pub const COUNT: usize = 6;
}
/// Bit flags on cells.
pub mod cell_flag {
    pub const ROAD: u8 = 1;
    pub const RAIL: u8 = 2;
    pub const RIVER: u8 = 4;
    pub const COAST: u8 = 8;
}
/// Bit flags on cell-graph edges.
pub mod cell_edge_flag {
    /// A river runs along the shared border of the two cells: going from one to the other crosses it.
    pub const RIVER: u8 = 1;
    /// A road crosses the shared border. With `RIVER`: a road bridge stood here in 1990.
    pub const ROAD: u8 = 2;
    /// A railway crosses the shared border. With `RIVER`: a railway bridge stood here in 1990.
    pub const RAIL: u8 = 4;
}
pub const SITE_NO_YEAR: i16 = i16::MAX;

/// Parsed arrays exactly as stored in the baked file.
#[derive(Clone, Debug, Default)]
pub struct RawMap {
    pub checksum: u32,
    pub state_tags: Vec<String>,
    pub state_names: Vec<String>,
    pub choke_names: Vec<String>,
    pub choke_built: Vec<i16>,
    pub site_names: Vec<String>,

    pub province_state: Vec<u16>,
    pub province_x: Vec<i32>,
    pub province_y: Vec<i32>,
    pub province_area_ha: Vec<u32>,
    pub province_pop: Vec<u32>,
    pub province_cell_off: Vec<u32>,
    pub province_cells: Vec<u32>,

    pub zone_x: Vec<i32>,
    pub zone_y: Vec<i32>,
    pub zone_area_ha: Vec<u32>,
    pub zone_body: Vec<u16>,

    pub g_off: Vec<u32>,
    pub g_dst: Vec<u32>,
    pub g_kind: Vec<u8>,
    pub g_weight: Vec<u32>,
    pub g_strait: Vec<u16>,
    pub g_cost_m: Vec<u32>,

    pub cell_state: Vec<u16>,
    pub cell_province: Vec<u32>,
    pub cell_x: Vec<i32>,
    pub cell_y: Vec<i32>,
    pub cell_area_ha: Vec<u32>,
    pub cell_landmass: Vec<u32>,
    pub cell_elev_m: Vec<i16>,
    pub cell_relief_m: Vec<u16>,
    pub cell_terrain: Vec<u8>,
    pub cell_urban: Vec<u8>,
    pub cell_flags: Vec<u8>,
    pub cell_zone: Vec<u32>,
    pub cell_pop: Vec<u32>,

    pub c_off: Vec<u32>,
    pub c_dst: Vec<u32>,
    pub c_len_m: Vec<u32>,
    pub c_flags: Vec<u8>,
    pub c_dist_m: Vec<u32>,

    pub site_kind: Vec<u8>,
    pub site_cell: Vec<u32>,
    pub site_province: Vec<u32>,
    pub site_size: Vec<u32>,
    pub site_built: Vec<i16>,
    pub site_destroyed: Vec<i16>,
    pub site_verified: Vec<u8>,

    // The globe (map rewrite, phase M1). All three are empty on a flat map, which the small
    // test maps still are; on a globe map distances come from these, not from x and y.
    /// The H3 number of each hexagon (level 6). The engine never interprets it: it is the
    /// cell's permanent name, so other data can be joined to it.
    pub cell_h3: Vec<u64>,
    /// Each cell's centre on the globe as a `sphere::Place`: three numbers per cell.
    pub cell_xyz: Vec<i32>,
    /// The same for every node of the strategic graph (provinces, then sea zones).
    pub node_xyz: Vec<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapError(pub String);
impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Uniform grid over cell centres for range queries. Built deterministically at load.
#[derive(Clone, Debug, Default)]
struct Grid {
    /// Set on a globe map: there the buckets are boxes in three dimensions.
    globe: Option<Grid3>,
    bucket_m: i32,
    x0: i32,
    y0: i32,
    cols: u32,
    rows: u32,
    off: Vec<u32>,
    items: Vec<u32>,
}

const BUCKET_M: i32 = 64_000;

impl Grid {
    fn build(xs: &[i32], ys: &[i32]) -> Grid {
        if xs.is_empty() {
            return Grid::default();
        }
        let x0 = *xs.iter().min().unwrap();
        let y0 = *ys.iter().min().unwrap();
        let cols = ((*xs.iter().max().unwrap() - x0) / BUCKET_M + 1) as u32;
        let rows = ((*ys.iter().max().unwrap() - y0) / BUCKET_M + 1) as u32;
        let mut off = vec![0u32; (cols * rows) as usize + 1];
        let bucket = |i: usize| -> usize { (((ys[i] - y0) / BUCKET_M) as u32 * cols + ((xs[i] - x0) / BUCKET_M) as u32) as usize };
        for i in 0..xs.len() {
            off[bucket(i) + 1] += 1;
        }
        for b in 0..(cols * rows) as usize {
            off[b + 1] += off[b];
        }
        let mut fill = off.clone();
        let mut items = vec![0u32; xs.len()];
        for i in 0..xs.len() {
            let b = bucket(i);
            items[fill[b] as usize] = i as u32;
            fill[b] += 1;
        }
        Grid { globe: None, bucket_m: BUCKET_M, x0, y0, cols, rows, off, items }
    }

    /// Indices within `radius` of (x, y), ascending by index.
    fn within(&self, xs: &[i32], ys: &[i32], x: i32, y: i32, radius: u32) -> Vec<u32> {
        let mut out = Vec::new();
        if self.cols == 0 {
            return out;
        }
        let r = radius as i64;
        let lo_x = ((x as i64 - r - self.x0 as i64).div_euclid(self.bucket_m as i64)).clamp(0, self.cols as i64 - 1);
        let hi_x = ((x as i64 + r - self.x0 as i64).div_euclid(self.bucket_m as i64)).clamp(0, self.cols as i64 - 1);
        let lo_y = ((y as i64 - r - self.y0 as i64).div_euclid(self.bucket_m as i64)).clamp(0, self.rows as i64 - 1);
        let hi_y = ((y as i64 + r - self.y0 as i64).div_euclid(self.bucket_m as i64)).clamp(0, self.rows as i64 - 1);
        for by in lo_y..=hi_y {
            for bx in lo_x..=hi_x {
                let b = (by as u32 * self.cols + bx as u32) as usize;
                for &i in &self.items[self.off[b] as usize..self.off[b + 1] as usize] {
                    let dx = xs[i as usize] as i64 - x as i64;
                    let dy = ys[i as usize] as i64 - y as i64;
                    if dx * dx + dy * dy <= r * r {
                        out.push(i);
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }
}

/// Integer square root (floor), integer arithmetic only (Newton's method).
pub fn isqrt(v: u64) -> u64 {
    if v < 2 {
        return v;
    }
    let mut x = 1u64 << (64 - v.leading_zeros()).div_ceil(2); // a power of two at or above sqrt(v)
    loop {
        let y = (x + v / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

fn dist_m(ax: i32, ay: i32, bx: i32, by: i32) -> u32 {
    let dx = (ax as i64 - bx as i64).unsigned_abs();
    let dy = (ay as i64 - by as i64).unsigned_abs();
    isqrt(dx * dx + dy * dy) as u32
}

/// The same idea as `Grid` in three dimensions, for places on the globe. The boxes that hold
/// at least one cell are kept in a sorted list and found by binary search (no hash map, so the
/// order never depends on the machine).
#[derive(Clone, Debug, Default)]
struct Grid3 {
    /// Box key (see `box_key`), ascending, with where its cells start in `items`.
    boxes: Vec<(u64, u32)>,
    items: Vec<u32>,
}

/// Which box a coordinate falls in along one axis (0 to about 200).
fn box_index(v: i64) -> i64 {
    (v + crate::sphere::EARTH_RADIUS_M + BUCKET_M as i64).div_euclid(BUCKET_M as i64)
}
fn box_key(ix: i64, iy: i64, iz: i64) -> u64 {
    ((ix as u64) << 32) | ((iy as u64) << 16) | iz as u64
}

impl Grid3 {
    fn build(xyz: &[i32]) -> Grid3 {
        let key = |i: usize| box_key(box_index(xyz[3 * i] as i64), box_index(xyz[3 * i + 1] as i64), box_index(xyz[3 * i + 2] as i64));
        let mut keyed: Vec<(u64, u32)> = (0..xyz.len() / 3).map(|i| (key(i), i as u32)).collect();
        keyed.sort_unstable(); // by box, then by cell number: no ties
        let mut g = Grid3::default();
        for (i, &(k, cell)) in keyed.iter().enumerate() {
            if g.boxes.last().is_none_or(|b| b.0 != k) {
                g.boxes.push((k, i as u32));
            }
            g.items.push(cell);
        }
        g
    }

    /// Cells whose centre is within `radius` metres over the ground of `at`, ascending.
    fn within(&self, xyz: &[i32], at: Place, radius: u32) -> Vec<u32> {
        let mut out = Vec::new();
        // the straight line to a place is never longer than the way over the ground, so every
        // cell in range lies in the cube of boxes `radius` each way
        let r = radius as i64;
        // The straight line that spans an arc of `radius`, found by halving: `line` ends as
        // the longest with an arc within the radius. Cells clearly nearer than it are in,
        // cells clearly farther are out (8 m either side covers every rounding).
        let (mut line, mut step) = (0u64, 1u64 << 24);
        while step > 0 {
            let longer = (line + step).min(2 * crate::sphere::EARTH_RADIUS_M as u64);
            if ground_from_chord_squared(longer * longer) <= radius {
                line = longer;
            }
            step /= 2;
        }
        let sure_in = line.saturating_sub(8).pow(2);
        let maybe_in = (line + 8).pow(2);
        let range = |v: i32| box_index((v as i64 - r).max(-crate::sphere::EARTH_RADIUS_M - 1))..=box_index((v as i64 + r).min(crate::sphere::EARTH_RADIUS_M + 1));
        for ix in range(at[0]) {
            for iy in range(at[1]) {
                for iz in range(at[2]) {
                    let key = box_key(ix, iy, iz);
                    let Ok(b) = self.boxes.binary_search_by_key(&key, |b| b.0) else { continue };
                    let end = self.boxes.get(b + 1).map_or(self.items.len(), |n| n.1 as usize);
                    for &i in &self.items[self.boxes[b].1 as usize..end] {
                        let c2 = chord_squared(place_of(xyz, i), at);
                        // most cells are settled by the straight line alone; only those within
                        // a few metres of the edge of the range need the exact arc
                        if c2 <= sure_in || (c2 <= maybe_in && ground_from_chord_squared(c2) <= radius) {
                            out.push(i);
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }
}

/// Number the connected pieces of a graph: each node gets the lowest node number of its piece.
fn connected_groups(off: &[u32], dst: &[u32]) -> Vec<u32> {
    let n = off.len() - 1;
    let mut group = vec![u32::MAX; n];
    let mut stack = Vec::new();
    for start in 0..n {
        if group[start] != u32::MAX {
            continue;
        }
        group[start] = start as u32;
        stack.push(start as u32);
        while let Some(v) = stack.pop() {
            for &w in &dst[off[v as usize] as usize..off[v as usize + 1] as usize] {
                if group[w as usize] == u32::MAX {
                    group[w as usize] = start as u32;
                    stack.push(w);
                }
            }
        }
    }
    group
}

/// The place stored at position `i` of a list of three numbers per entry.
fn place_of(xyz: &[i32], i: u32) -> Place {
    let k = 3 * i as usize;
    [xyz[k], xyz[k + 1], xyz[k + 2]]
}

#[derive(Debug)]
pub struct Map {
    raw: RawMap,
    cell_grid: Grid,
    /// Which connected piece of land each cell is in (worked out on load, not stored): two
    /// cells in different pieces have no land route, which is known without searching.
    cell_group: Vec<u32>,
    /// Sites grouped by cell: site_by_cell_off[c]..[c+1] indexes site_by_cell.
    site_by_cell_off: Vec<u32>,
    site_by_cell: Vec<u32>,
}

impl Deref for Map {
    type Target = RawMap;
    fn deref(&self) -> &RawMap {
        &self.raw
    }
}

impl Map {
    /// Validate structure (lengths, ranges, symmetry) and build indices.
    pub fn from_raw(raw: RawMap) -> Result<Map, Vec<MapError>> {
        let errors = check_structure(&raw);
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut cell_grid = Grid::build(&raw.cell_x, &raw.cell_y);
        if !raw.cell_xyz.is_empty() {
            cell_grid.globe = Some(Grid3::build(&raw.cell_xyz));
        }
        let n = raw.cell_x.len();
        let mut off = vec![0u32; n + 1];
        for &c in &raw.site_cell {
            off[c as usize + 1] += 1;
        }
        for i in 0..n {
            off[i + 1] += off[i];
        }
        let mut fill = off.clone();
        let mut by_cell = vec![0u32; raw.site_cell.len()];
        for (s, &c) in raw.site_cell.iter().enumerate() {
            by_cell[fill[c as usize] as usize] = s as u32;
            fill[c as usize] += 1;
        }
        let cell_group = connected_groups(&raw.c_off, &raw.c_dst);
        Ok(Map { raw, cell_grid, cell_group, site_by_cell_off: off, site_by_cell: by_cell })
    }

    pub fn province_count(&self) -> u32 {
        self.raw.province_state.len() as u32
    }
    pub fn zone_count(&self) -> u32 {
        self.raw.zone_x.len() as u32
    }
    pub fn node_count(&self) -> u32 {
        self.province_count() + self.zone_count()
    }
    pub fn cell_count(&self) -> u32 {
        self.raw.cell_x.len() as u32
    }
    pub fn site_count(&self) -> u32 {
        self.raw.site_kind.len() as u32
    }
    pub fn state_count(&self) -> u32 {
        self.raw.state_tags.len() as u32
    }
    pub fn is_sea_node(&self, node: u32) -> bool {
        node >= self.province_count()
    }

    /// Neighbours of a strategic-graph node as (neighbour, edge index).
    pub fn node_edges(&self, node: u32) -> std::ops::Range<usize> {
        self.raw.g_off[node as usize] as usize..self.raw.g_off[node as usize + 1] as usize
    }
    pub fn cell_edges(&self, cell: CellId) -> std::ops::Range<usize> {
        self.raw.c_off[cell.index()] as usize..self.raw.c_off[cell.index() + 1] as usize
    }
    pub fn cells_of(&self, p: ProvinceId) -> &[u32] {
        &self.raw.province_cells[self.raw.province_cell_off[p.index()] as usize..self.raw.province_cell_off[p.index() + 1] as usize]
    }
    pub fn sites_in_cell(&self, c: CellId) -> &[u32] {
        &self.site_by_cell[self.site_by_cell_off[c.index()] as usize..self.site_by_cell_off[c.index() + 1] as usize]
    }

    /// Is this map on the globe (hexagons with places in three dimensions)? The small test
    /// maps are flat; a baked map is on the globe.
    pub fn is_globe(&self) -> bool {
        !self.raw.cell_xyz.is_empty()
    }

    /// Where a cell's centre is on the globe; `None` on a flat map.
    pub fn cell_place(&self, c: CellId) -> Option<Place> {
        self.is_globe().then(|| place_of(&self.raw.cell_xyz, c.0))
    }

    /// Distance between two cell centres in metres (floor): over the ground on the globe.
    pub fn cell_distance_m(&self, a: CellId, b: CellId) -> u32 {
        if self.is_globe() {
            return ground_m(place_of(&self.raw.cell_xyz, a.0), place_of(&self.raw.cell_xyz, b.0));
        }
        dist_m(self.raw.cell_x[a.index()], self.raw.cell_y[a.index()], self.raw.cell_x[b.index()], self.raw.cell_y[b.index()])
    }

    /// All cells whose centre is within `radius_m` of the centre of `center`, ascending by ID.
    pub fn cells_within(&self, center: CellId, radius_m: u32) -> Vec<CellId> {
        if let Some(g) = &self.cell_grid.globe {
            return g.within(&self.raw.cell_xyz, place_of(&self.raw.cell_xyz, center.0), radius_m).into_iter().map(CellId).collect();
        }
        let (x, y) = (self.raw.cell_x[center.index()], self.raw.cell_y[center.index()]);
        self.cell_grid.within(&self.raw.cell_x, &self.raw.cell_y, x, y, radius_m).into_iter().map(CellId).collect()
    }

    /// All sites within `radius_m` of the centre of `center` (by the site's cell centre), ascending by site ID.
    pub fn sites_within(&self, center: CellId, radius_m: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for c in self.cells_within(center, radius_m) {
            out.extend_from_slice(self.sites_in_cell(c));
        }
        out.sort_unstable();
        out
    }

    // ---- pathfinding ----

    /// Cheapest path between two strategic-graph nodes (provinces or sea zones).
    pub fn find_node_path(&self, from: u32, to: u32, costs: &NodeCosts, scratch: &mut PathScratch) -> Option<Path> {
        let r = &self.raw;
        let (gx, gy) = self.node_xy(to);
        let min_pct = costs.percent.iter().copied().min().unwrap().max(1);
        let globe = !r.node_xyz.is_empty();
        let goal = if globe { place_of(&r.node_xyz, to) } else { [0; 3] };
        astar(
            &r.g_off,
            &r.g_dst,
            from,
            to,
            |_a, e| scaled(r.g_cost_m[e], costs.percent[(r.g_kind[e] & 3) as usize]),
            |n| {
                // never more than the real cost: on the globe the straight line through the
                // Earth, which is shorter than any way over the ground
                if globe {
                    return scaled_floor(chord_m(place_of(&r.node_xyz, n), goal), min_pct);
                }
                let (x, y) = self.node_xy(n);
                scaled_floor(dist_m(x, y, gx, gy), min_pct)
            },
            scratch,
        )
    }

    fn node_xy(&self, n: u32) -> (i32, i32) {
        let p = self.province_count();
        if n < p {
            (self.raw.province_x[n as usize], self.raw.province_y[n as usize])
        } else {
            (self.raw.zone_x[(n - p) as usize], self.raw.zone_y[(n - p) as usize])
        }
    }

    /// Cheapest path between two cells over the tactical cell graph, with no bridges: every
    /// river edge costs `river_extra_m`.
    pub fn find_cell_path(&self, from: CellId, to: CellId, costs: &CellCosts, scratch: &mut PathScratch) -> Option<Path> {
        self.find_cell_path_bridged(from, to, costs, scratch, &|_, _| false)
    }

    /// Cost in metres of going from cell `a` along its edge `e` (an index into the cell graph):
    /// the distance scaled by the terrain of the cell entered, or by road or rail when both
    /// cells have one, plus the river or bridge extra when a river runs across the edge.
    fn edge_cost(&self, a: u32, e: usize, costs: &CellCosts, bridged: &dyn Fn(u32, u32) -> bool) -> u32 {
        let r = &self.raw;
        let b = r.c_dst[e] as usize;
        let a = a as usize;
        let mut pct = costs.terrain_percent[(r.cell_terrain[b] as usize).min(7)];
        let both = |bit: u8| r.cell_flags[a] & bit != 0 && r.cell_flags[b] & bit != 0;
        if both(cell_flag::ROAD) {
            pct = pct.min(costs.road_percent);
        }
        if both(cell_flag::RAIL) {
            pct = pct.min(costs.rail_percent);
        }
        let mut c = scaled(r.c_dist_m[e], pct);
        if r.c_flags[e] & cell_edge_flag::RIVER != 0 {
            c = c.saturating_add(if bridged(a as u32, b as u32) { costs.bridge_extra_m } else { costs.river_extra_m });
        }
        c
    }

    /// Cost in metres of one step between two neighbouring cells, exactly as the pathfinder
    /// counts it; `None` if they are not neighbours.
    pub fn cell_step_cost(&self, a: CellId, b: CellId, costs: &CellCosts, bridged: bool) -> Option<u32> {
        let e = self.cell_edges(a).find(|&e| self.raw.c_dst[e] == b.0)?;
        Some(self.edge_cost(a.0, e, costs, &|_, _| bridged))
    }

    /// The cell whose centre is nearest to a point in world metres, and how far away it is in
    /// metres (ties go to the lower cell number). Looks at every cell: for setting a scenario
    /// up, not for use every tick.
    pub fn nearest_cell(&self, x: i32, y: i32) -> Option<(CellId, u32)> {
        let r = &self.raw;
        (0..self.cell_count()).map(|c| (dist_m(r.cell_x[c as usize], r.cell_y[c as usize], x, y), c)).min().map(|(d, c)| (CellId(c), d))
    }

    /// The cell whose centre is nearest to a place on the globe, and how far away it is over
    /// the ground (ties go to the lower cell number); `None` on a flat map. Looks at every
    /// cell, like `nearest_cell`.
    pub fn nearest_cell_to_place(&self, at: Place) -> Option<(CellId, u32)> {
        if !self.is_globe() {
            return None;
        }
        // nearest by the straight line is nearest over the ground too, and cheaper to compare
        let d2 = |c: u32| -> u64 {
            let p = place_of(&self.raw.cell_xyz, c);
            (0..3).map(|k| (p[k] as i64 - at[k] as i64).pow(2) as u64).sum()
        };
        let c = (0..self.cell_count()).map(|c| (d2(c), c)).min()?.1;
        Some((CellId(c), ground_m(place_of(&self.raw.cell_xyz, c), at)))
    }

    /// Are two cells joined by land (whatever the terrain)? Known without a search.
    pub fn same_landmass(&self, a: CellId, b: CellId) -> bool {
        self.cell_group[a.index()] == self.cell_group[b.index()]
    }

    /// Does a river run across the shared border of these two neighbouring cells?
    pub fn river_between(&self, a: CellId, b: CellId) -> bool {
        self.cell_edges(a).any(|e| self.raw.c_dst[e] == b.0 && self.raw.c_flags[e] & cell_edge_flag::RIVER != 0)
    }

    /// River edges where a road or a railway crosses (`a < b`, with the edge's flags): the
    /// places that had a permanent bridge in 1990. Ascending by (a, b).
    pub fn bridge_edges(&self) -> Vec<(u32, u32, u8)> {
        let r = &self.raw;
        let mut out = Vec::new();
        for a in 0..self.cell_count() {
            for e in self.cell_edges(CellId(a)) {
                let (b, f) = (r.c_dst[e], r.c_flags[e]);
                if a < b && f & cell_edge_flag::RIVER != 0 && f & (cell_edge_flag::ROAD | cell_edge_flag::RAIL) != 0 {
                    out.push((a, b, f));
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// Cheapest path between two cells. `bridged(a, b)` says whether the river edge between
    /// two cells has a usable bridge right now; then it costs `bridge_extra_m` instead of
    /// `river_extra_m`. Bridges change during play, so they live in the world, not in the map
    /// (see `World::find_cell_path`).
    pub fn find_cell_path_bridged(&self, from: CellId, to: CellId, costs: &CellCosts, scratch: &mut PathScratch, bridged: &dyn Fn(u32, u32) -> bool) -> Option<Path> {
        let r = &self.raw;
        if self.cell_group[from.index()] != self.cell_group[to.index()] {
            return None; // across the sea: a search would visit the whole landmass to learn this
        }
        let (gx, gy) = (r.cell_x[to.index()], r.cell_y[to.index()]);
        let min_pct = costs.min_percent();
        let globe = self.is_globe();
        let goal = if globe { place_of(&r.cell_xyz, to.0) } else { [0; 3] };
        astar(
            &r.c_off,
            &r.c_dst,
            from.0,
            to.0,
            |a, e| self.edge_cost(a, e, costs, bridged),
            |n| {
                if globe {
                    return scaled_floor(chord_m(place_of(&r.cell_xyz, n), goal), min_pct);
                }
                scaled_floor(dist_m(r.cell_x[n as usize], r.cell_y[n as usize], gx, gy), min_pct)
            },
            scratch,
        )
    }

    // ---- global checks (beyond structure) ----

    /// Whole-world rules: no orphan provinces, everything reachable, every landmass has a sea link,
    /// every cell in exactly one province, areas add up. Returns all problems found.
    pub fn validate_world(&self) -> Vec<MapError> {
        let r = &self.raw;
        let mut e = Vec::new();
        let p = self.province_count() as usize;
        let n = self.node_count() as usize;
        for node in 0..n {
            if r.g_off[node] == r.g_off[node + 1] {
                e.push(MapError(format!("node {node} ({}) has no neighbours", if node < p { "province" } else { "sea zone" })));
            }
        }
        // every province has an owner: ensured by structure check (state index in range); also each has cells
        for prov in 0..p {
            if r.province_cell_off[prov] == r.province_cell_off[prov + 1] {
                e.push(MapError(format!("province {prov} has no cells")));
            }
            let sum: u64 = self.cells_of(ProvinceId(prov as u32)).iter().map(|&c| r.cell_area_ha[c as usize] as u64).sum();
            if sum.abs_diff(r.province_area_ha[prov] as u64) > 1 {
                e.push(MapError(format!("province {prov}: cell areas sum to {sum} ha but the province says {}", r.province_area_ha[prov])));
            }
        }
        // connectivity over the whole strategic graph
        let mut seen = vec![false; n];
        let mut stack = vec![0u32];
        seen[0] = true;
        let mut reached = 1;
        while let Some(v) = stack.pop() {
            for ed in self.node_edges(v) {
                let w = r.g_dst[ed];
                if !seen[w as usize] {
                    seen[w as usize] = true;
                    reached += 1;
                    stack.push(w);
                }
            }
        }
        if reached != n {
            let first = seen.iter().position(|s| !s).unwrap();
            e.push(MapError(format!("{} of {n} nodes are unreachable from node 0 (first: {first})", n - reached)));
        }
        // landmasses (groups of provinces joined by land edges) must touch the sea
        let mut group = vec![u32::MAX; p];
        let mut next = 0u32;
        let mut has_sea = Vec::new();
        for start in 0..p {
            if group[start] != u32::MAX {
                continue;
            }
            let mut sea = false;
            group[start] = next;
            let mut st = vec![start as u32];
            while let Some(v) = st.pop() {
                for ed in self.node_edges(v) {
                    let w = r.g_dst[ed];
                    match r.g_kind[ed] {
                        edge_kind::LAND => {
                            if group[w as usize] == u32::MAX {
                                group[w as usize] = next;
                                st.push(w);
                            }
                        }
                        edge_kind::COAST => sea = true,
                        _ => {}
                    }
                }
            }
            has_sea.push(sea);
            next += 1;
        }
        for (g, ok) in has_sea.iter().enumerate() {
            if !ok {
                let first = group.iter().position(|&x| x == g as u32).unwrap();
                e.push(MapError(format!("landmass containing province {first} has no sea link")));
            }
        }
        e
    }
}

fn scaled(dist: u32, pct: u32) -> u32 {
    ((dist as u64 * pct as u64).div_ceil(100)).min(u32::MAX as u64) as u32
}
fn scaled_floor(dist: u32, pct: u32) -> u32 {
    ((dist as u64 * pct as u64) / 100).min(u32::MAX as u64) as u32
}

/// Travel cost percentages for the strategic graph, by edge kind
/// [land, coast, strait, sea]. 100 = cost equals distance. Each must be at least 1.
#[derive(Clone, Debug)]
pub struct NodeCosts {
    pub percent: [u32; 4],
}
impl Default for NodeCosts {
    fn default() -> Self {
        NodeCosts { percent: [100, 100, 100, 100] }
    }
}

/// Travel cost percentages for the cell graph. Each must be at least 1.
#[derive(Clone, Debug)]
pub struct CellCosts {
    pub terrain_percent: [u32; 8],
    pub road_percent: u32,
    pub rail_percent: u32,
    /// Added to the cost of an edge a river runs across, when there is no usable bridge.
    pub river_extra_m: u32,
    /// Added instead when the river is crossed by a usable bridge (a bridge is a bottleneck).
    pub bridge_extra_m: u32,
}
impl Default for CellCosts {
    fn default() -> Self {
        CellCosts { terrain_percent: [100, 150, 250, 400, 180, 500, 100, 100], road_percent: 60, rail_percent: 40, river_extra_m: 4000, bridge_extra_m: 0 }
    }
}
impl CellCosts {
    fn min_percent(&self) -> u32 {
        let t = self.terrain_percent.iter().copied().min().unwrap();
        t.min(self.road_percent).min(self.rail_percent).max(1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    pub nodes: Vec<u32>,
    pub cost: u32,
    /// Nodes taken off the queue (a measure of search effort).
    pub expanded: u32,
}

/// Reusable search memory so a query does not clear millions of entries.
pub struct PathScratch {
    stamp: Vec<u32>,
    generation: u32,
    g: Vec<u32>,
    parent: Vec<u32>,
    heap: BinaryHeap<Reverse<(u32, u32)>>,
}
impl PathScratch {
    pub fn new(capacity: usize) -> PathScratch {
        PathScratch { stamp: vec![0; capacity], generation: 0, g: vec![0; capacity], parent: vec![0; capacity], heap: BinaryHeap::new() }
    }
}

fn astar(
    off: &[u32],
    dst: &[u32],
    from: u32,
    to: u32,
    edge_cost: impl Fn(u32, usize) -> u32,
    heuristic: impl Fn(u32) -> u32,
    s: &mut PathScratch,
) -> Option<Path> {
    assert!(s.stamp.len() >= off.len() - 1, "PathScratch too small for this graph");
    s.generation = s.generation.wrapping_add(1);
    if s.generation == 0 {
        s.stamp.iter_mut().for_each(|x| *x = 0);
        s.generation = 1;
    }
    let gen = s.generation;
    s.heap.clear();
    s.stamp[from as usize] = gen;
    s.g[from as usize] = 0;
    s.parent[from as usize] = u32::MAX;
    s.heap.push(Reverse((heuristic(from), from)));
    let mut expanded = 0u32;
    while let Some(Reverse((f, v))) = s.heap.pop() {
        let gv = s.g[v as usize];
        if f > gv.saturating_add(heuristic(v)) {
            continue; // stale entry
        }
        expanded += 1;
        if v == to {
            let mut nodes = vec![v];
            let mut cur = v;
            while s.parent[cur as usize] != u32::MAX {
                cur = s.parent[cur as usize];
                nodes.push(cur);
            }
            nodes.reverse();
            return Some(Path { nodes, cost: gv, expanded });
        }
        let (lo, hi) = (off[v as usize] as usize, off[v as usize + 1] as usize);
        for (e, &w) in dst[lo..hi].iter().enumerate().map(|(i, w)| (lo + i, w)) {
            let ng = gv.saturating_add(edge_cost(v, e));
            if s.stamp[w as usize] != gen || ng < s.g[w as usize] {
                s.stamp[w as usize] = gen;
                s.g[w as usize] = ng;
                s.parent[w as usize] = v;
                s.heap.push(Reverse((ng.saturating_add(heuristic(w)), w)));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------

fn check_structure(r: &RawMap) -> Vec<MapError> {
    let mut e = Vec::new();
    let mut err = |s: String| e.push(MapError(s));
    let p = r.province_state.len();
    let z = r.zone_x.len();
    let n = r.cell_x.len();
    let s = r.site_kind.len();
    let states = r.state_tags.len();
    macro_rules! len {
        ($name:literal, $v:expr, $want:expr) => {
            if $v.len() != $want {
                err(format!("array {} has {} entries, expected {}", $name, $v.len(), $want));
            }
        };
    }
    len!("state_names", r.state_names, states);
    len!("choke_built", r.choke_built, r.choke_names.len());
    len!("province_x", r.province_x, p);
    len!("province_y", r.province_y, p);
    len!("province_area_ha", r.province_area_ha, p);
    len!("province_pop", r.province_pop, p);
    len!("province_cell_off", r.province_cell_off, p + 1);
    len!("province_cells", r.province_cells, n);
    len!("zone_y", r.zone_y, z);
    len!("zone_area_ha", r.zone_area_ha, z);
    len!("zone_body", r.zone_body, z);
    len!("g_off", r.g_off, p + z + 1);
    for (name, l) in [("g_kind", r.g_kind.len()), ("g_weight", r.g_weight.len()), ("g_strait", r.g_strait.len()), ("g_cost_m", r.g_cost_m.len())] {
        if l != r.g_dst.len() {
            err(format!("array {name} has {l} entries, expected {}", r.g_dst.len()));
        }
    }
    len!("cell_state", r.cell_state, n);
    len!("cell_province", r.cell_province, n);
    len!("cell_y", r.cell_y, n);
    len!("cell_area_ha", r.cell_area_ha, n);
    len!("cell_landmass", r.cell_landmass, n);
    len!("cell_elev_m", r.cell_elev_m, n);
    len!("cell_relief_m", r.cell_relief_m, n);
    len!("cell_terrain", r.cell_terrain, n);
    len!("cell_urban", r.cell_urban, n);
    len!("cell_flags", r.cell_flags, n);
    len!("cell_zone", r.cell_zone, n);
    len!("cell_pop", r.cell_pop, n);
    len!("c_off", r.c_off, n + 1);
    for (name, l) in [("c_len_m", r.c_len_m.len()), ("c_flags", r.c_flags.len()), ("c_dist_m", r.c_dist_m.len())] {
        if l != r.c_dst.len() {
            err(format!("array {name} has {l} entries, expected {}", r.c_dst.len()));
        }
    }
    if !r.cell_xyz.is_empty() || !r.cell_h3.is_empty() || !r.node_xyz.is_empty() {
        len!("cell_h3", r.cell_h3, n);
        len!("cell_xyz", r.cell_xyz, 3 * n);
        len!("node_xyz", r.node_xyz, 3 * (p + z));
    }
    len!("site_cell", r.site_cell, s);
    len!("site_province", r.site_province, s);
    len!("site_size", r.site_size, s);
    len!("site_built", r.site_built, s);
    len!("site_destroyed", r.site_destroyed, s);
    len!("site_verified", r.site_verified, s);
    len!("site_names", r.site_names, s);
    if !e.is_empty() || p == 0 || n == 0 {
        if p == 0 || n == 0 {
            e.push(MapError("map has no provinces or no cells".into()));
        }
        return e; // later checks index these arrays
    }
    let mut err = |s: String| e.push(MapError(s));

    // value ranges
    for (i, &st) in r.province_state.iter().enumerate() {
        if st as usize >= states {
            err(format!("province {i}: owner state {st} does not exist"));
        }
    }
    for i in 0..n {
        if r.cell_province[i] as usize >= p {
            err(format!("cell {i}: province {} does not exist", r.cell_province[i]));
            return e;
        }
        if r.cell_state[i] != r.province_state[r.cell_province[i] as usize] {
            err(format!("cell {i}: state {} differs from its province's state", r.cell_state[i]));
        }
        if r.cell_zone[i] != NO_ZONE && r.cell_zone[i] as usize >= z {
            err(format!("cell {i}: sea zone {} does not exist", r.cell_zone[i]));
        }
        if r.cell_terrain[i] as usize >= terrain::COUNT {
            err(format!("cell {i}: unknown terrain class {}", r.cell_terrain[i]));
        }
    }
    for i in 0..s {
        let c = r.site_cell[i] as usize;
        if c >= n {
            err(format!("site {i}: cell {c} does not exist"));
            return e;
        }
        if r.site_province[i] != r.cell_province[c] {
            err(format!("site {i}: province differs from its cell's province"));
        }
        if r.site_kind[i] > 5 {
            err(format!("site {i}: unknown kind {}", r.site_kind[i]));
        }
    }
    // province -> cells is a partition: a permutation where each cell appears under its own province
    let off = &r.province_cell_off;
    if off[0] != 0 || off[p] as usize != n || off.windows(2).any(|w| w[0] > w[1]) {
        err("province_cell_off is not a non-decreasing run from 0 to the cell count".into());
        return e;
    }
    let mut seen = vec![false; n];
    for prov in 0..p {
        for &c in &r.province_cells[off[prov] as usize..off[prov + 1] as usize] {
            let c = c as usize;
            if c >= n || seen[c] {
                err(format!("province {prov} lists cell {c} which is out of range or listed twice"));
                return e;
            }
            seen[c] = true;
            if r.cell_province[c] as usize != prov {
                err(format!("cell {c} is listed under province {prov} but belongs to {}", r.cell_province[c]));
            }
        }
    }
    if seen.iter().any(|x| !x) {
        err("some cell is not listed under any province".into());
    }
    // graphs: offsets, ranges, sorted, symmetric
    check_graph("strategic graph", &r.g_off, &r.g_dst, p + z, Some(&r.g_kind), &mut e);
    check_graph("cell graph", &r.c_off, &r.c_dst, n, None, &mut e);
    if e.is_empty() && !r.cell_xyz.is_empty() {
        check_globe(r, &mut e);
    }
    e
}

/// Globe maps: every place is on the sphere, every hexagon number is used once, and no edge
/// is recorded as shorter than the straight line between its ends (the pathfinder relies on
/// that to find the cheapest route).
fn check_globe(r: &RawMap, e: &mut Vec<MapError>) {
    for (name, xyz) in [("cell", &r.cell_xyz), ("node", &r.node_xyz)] {
        for i in 0..(xyz.len() / 3) as u32 {
            let from_centre = chord_m(place_of(xyz, i), [0, 0, 0]) as i64;
            if (from_centre - crate::sphere::EARTH_RADIUS_M).abs() > 3 {
                e.push(MapError(format!("{name} {i}: its place is {from_centre} m from the Earth's centre, not on the globe")));
                return;
            }
        }
    }
    let mut numbers = r.cell_h3.clone();
    numbers.sort_unstable();
    if numbers.windows(2).any(|w| w[0] == w[1]) {
        e.push(MapError("two cells have the same hexagon number".into()));
    }
    for (name, off, dst, cost, xyz) in [("cell graph", &r.c_off, &r.c_dst, &r.c_dist_m, &r.cell_xyz), ("strategic graph", &r.g_off, &r.g_dst, &r.g_cost_m, &r.node_xyz)] {
        for a in 0..off.len() - 1 {
            for ed in off[a] as usize..off[a + 1] as usize {
                let line = chord_m(place_of(xyz, a as u32), place_of(xyz, dst[ed]));
                if cost[ed] < line {
                    e.push(MapError(format!("{name}: edge {a}->{} is recorded as {} m, shorter than the straight line ({line} m)", dst[ed], cost[ed])));
                    return;
                }
            }
        }
    }
}

fn check_graph(name: &str, off: &[u32], dst: &[u32], nodes: usize, kind: Option<&[u8]>, e: &mut Vec<MapError>) {
    if off[0] != 0 || off[nodes] as usize != dst.len() || off.windows(2).any(|w| w[0] > w[1]) {
        e.push(MapError(format!("{name}: offsets are not a non-decreasing run from 0 to the edge count")));
        return;
    }
    for v in 0..nodes {
        let list = &dst[off[v] as usize..off[v + 1] as usize];
        for (k, &w) in list.iter().enumerate() {
            if w as usize >= nodes {
                e.push(MapError(format!("{name}: node {v} has neighbour {w} which does not exist")));
                return;
            }
            if w as usize == v {
                e.push(MapError(format!("{name}: node {v} is its own neighbour")));
            }
            if k > 0 && list[k - 1] >= w {
                e.push(MapError(format!("{name}: neighbours of node {v} are not strictly increasing")));
                return;
            }
        }
    }
    for v in 0..nodes {
        for ed in off[v] as usize..off[v + 1] as usize {
            let w = dst[ed] as usize;
            let back = &dst[off[w] as usize..off[w + 1] as usize];
            match back.binary_search(&(v as u32)) {
                Err(_) => {
                    e.push(MapError(format!("{name}: edge {v}->{w} has no reverse edge")));
                    return;
                }
                Ok(i) => {
                    if let Some(k) = kind {
                        if k[ed] != k[off[w] as usize + i] {
                            e.push(MapError(format!("{name}: edge {v}<->{w} has different kinds in its two directions")));
                            return;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
