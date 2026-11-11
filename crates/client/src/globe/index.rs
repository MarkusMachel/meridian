//! Finding hexagons and building their triangles. The map file names each cell by its H3
//! number; this file is the only place that knows what such a number means.

use super::camera::EARTH_RADIUS_M;
use super::{add, dir_of, dot, lat_lon_of, scale, sub, unit, V3};
use bytemuck::{Pod, Zeroable};
use h3o::{CellIndex, LatLng, Resolution};
use meridian_core::{CellId, Map};

/// The level of the map's hexagons, and of the tiles they are drawn in (a level-3 hexagon
/// holds 343 level-6 ones and is about 120 km across).
const CELL_LEVEL: Resolution = Resolution::Six;
const TILE_LEVEL: Resolution = Resolution::Three;
/// No tile reaches farther than this from its centre, as an angle (about 110 km).
pub const TILE_REACH: f64 = 0.0175;
pub const NO_CELL: u32 = u32::MAX;

/// What runs along one side of a hexagon (bits; the shader draws a line for each).
pub mod side {
    pub const PROVINCE: u32 = 1;
    pub const COUNTRY: u32 = 2;
    pub const RIVER: u32 = 4;
    pub const COAST: u32 = 8;
}

/// One corner of a triangle as the graphics card gets it: 24 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    /// Metres from the tile's anchor.
    pub pos: [f32; 3],
    pub cell: u32,
    /// 1 at the hexagon's centre, 0 on its rim: how far into the hexagon a pixel is.
    pub inward: f32,
    /// `side` bits for the side this triangle stands on.
    pub side: u32,
}

/// A group of hexagons drawn together: the cells under one level-3 hexagon.
#[derive(Clone, Debug)]
pub struct Tile {
    /// Direction from the Earth's centre to the middle of the tile.
    pub center: V3,
    /// Where its cells are in `HexIndex::tile_cells`.
    cells: std::ops::Range<usize>,
}

pub struct HexIndex {
    /// (H3 number, cell) ascending by number, to find a cell by its hexagon.
    by_number: Vec<(u64, u32)>,
    pub tiles: Vec<Tile>,
    tile_cells: Vec<u32>,
}

fn lat_lng(d: V3) -> LatLng {
    let (lat, lon) = lat_lon_of(d);
    LatLng::from_radians(lat, lon).expect("finite angles")
}

fn dir_of_lat_lng(ll: LatLng) -> V3 {
    dir_of(ll.lat_radians(), ll.lng_radians())
}

impl HexIndex {
    pub fn new(map: &Map) -> HexIndex {
        let mut by_number: Vec<(u64, u32)> = map.cell_h3.iter().enumerate().map(|(i, &h)| (h, i as u32)).collect();
        by_number.sort_unstable();
        // group the cells by the level-3 hexagon they lie in
        let mut by_tile: Vec<(u64, u32)> = map
            .cell_h3
            .iter()
            .enumerate()
            .map(|(i, &h)| {
                let cell = CellIndex::try_from(h).expect("the map's hexagon numbers are valid");
                (u64::from(cell.parent(TILE_LEVEL).expect("level 6 has a level 3 parent")), i as u32)
            })
            .collect();
        by_tile.sort_unstable();
        let mut tiles = Vec::new();
        let mut start = 0;
        while start < by_tile.len() {
            let key = by_tile[start].0;
            let end = start + by_tile[start..].iter().take_while(|t| t.0 == key).count();
            let center = dir_of_lat_lng(LatLng::from(CellIndex::try_from(key).expect("a parent is valid")));
            tiles.push(Tile { center, cells: start..end });
            start = end;
        }
        HexIndex { by_number, tiles, tile_cells: by_tile.into_iter().map(|t| t.1).collect() }
    }

    fn cell_of_number(&self, h: u64) -> Option<u32> {
        self.by_number.binary_search_by_key(&h, |x| x.0).ok().map(|i| self.by_number[i].1)
    }

    /// The land cell under a point on the ground, if there is one.
    pub fn cell_at(&self, d: V3) -> Option<CellId> {
        self.cell_of_number(u64::from(lat_lng(d).to_cell(CELL_LEVEL))).map(CellId)
    }

    /// The tiles that may be in view: those within `radius` (an angle) of the direction `f`.
    pub fn tiles_near(&self, f: V3, radius: f64) -> Vec<u32> {
        let limit = (radius + TILE_REACH).min(std::f64::consts::PI).cos();
        (0..self.tiles.len() as u32).filter(|&t| dot(self.tiles[t as usize].center, f) >= limit).collect()
    }

    /// A picture of the whole world for the far view: for each pixel of a `width` by `height`
    /// grid of longitude (left edge 180° W) and latitude (top edge 90° N), the cell there or
    /// `NO_CELL`. Worked out on all processor cores.
    pub fn world_picture(&self, width: u32, height: u32) -> Vec<u32> {
        let mut out = vec![NO_CELL; (width * height) as usize];
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        let rows_each = (height as usize).div_ceil(threads);
        std::thread::scope(|s| {
            for (k, part) in out.chunks_mut(rows_each * width as usize).enumerate() {
                s.spawn(move || {
                    let mut last = (0u64, NO_CELL); // neighbouring pixels are mostly the same hexagon
                    for (i, px) in part.iter_mut().enumerate() {
                        let (row, col) = (k * rows_each + i / width as usize, i % width as usize);
                        let lat = 90.0 - (row as f64 + 0.5) * 180.0 / height as f64;
                        let lon = (col as f64 + 0.5) * 360.0 / width as f64 - 180.0;
                        let h = u64::from(LatLng::new(lat, lon).expect("in range").to_cell(CELL_LEVEL));
                        if h != last.0 {
                            last = (h, self.cell_of_number(h).unwrap_or(NO_CELL));
                        }
                        *px = last.1;
                    }
                });
            }
        });
        out
    }

    /// The triangles of one tile: each hexagon is a fan of triangles from its centre, one per
    /// side, and each triangle knows what runs along its side. Positions are relative to the
    /// returned anchor (the tile's centre on the ground), which keeps them small and exact.
    pub fn tile_mesh(&self, map: &Map, tile: u32) -> (V3, Vec<Vertex>) {
        let t = &self.tiles[tile as usize];
        let anchor = scale(t.center, EARTH_RADIUS_M);
        let mut out = Vec::with_capacity(t.cells.len() * 18);
        for &cell in &self.tile_cells[t.cells.clone()] {
            let index = CellIndex::try_from(map.cell_h3[cell as usize]).expect("valid");
            let corners: Vec<V3> = index.boundary().iter().map(|&ll| dir_of_lat_lng(ll)).collect();
            let n = corners.len();
            let middle = unit(corners.iter().fold([0.0; 3], |a, &c| add(a, c)));
            // which side faces which neighbour: the side whose middle points most nearly at it
            let mut sides = vec![side::COAST; n];
            for e in map.cell_edges(CellId(cell)) {
                let other = map.c_dst[e] as usize;
                let to = map.cell_place(CellId(other as u32)).expect("a globe map").map(|v| v as f64);
                let toward = unit(sub(unit(to), middle));
                let facing = (0..n).max_by(|&a, &b| {
                    let mid = |i: usize| dot(sub(unit(add(corners[i], corners[(i + 1) % n])), middle), toward);
                    mid(a).total_cmp(&mid(b))
                });
                let mut bits = 0;
                if map.cell_province[other] != map.cell_province[cell as usize] {
                    bits |= side::PROVINCE;
                }
                if map.cell_state[other] != map.cell_state[cell as usize] {
                    bits |= side::COUNTRY;
                }
                if map.c_flags[e] & meridian_core::map::cell_edge_flag::RIVER != 0 {
                    bits |= side::RIVER;
                }
                sides[facing.expect("a hexagon has sides")] = bits;
            }
            let local = |d: V3| sub(scale(d, EARTH_RADIUS_M), anchor).map(|v| v as f32);
            for i in 0..n {
                out.push(Vertex { pos: local(middle), cell, inward: 1.0, side: sides[i] });
                out.push(Vertex { pos: local(corners[i]), cell, inward: 0.0, side: sides[i] });
                out.push(Vertex { pos: local(corners[(i + 1) % n]), cell, inward: 0.0, side: sides[i] });
            }
        }
        (anchor, out)
    }
}
