//! Derived caches (Phase 4B, step 13): zones of control and front segments.
//!
//! These are rebuilt from stored state, are never hashed and are never saved. They
//! live outside `World`, so they cannot leak into the state hash by accident. The
//! tests check that rebuilding always gives identical results.
//!
//! Visibility and air-defense coverage caches (also listed in step 13) need sensor
//! and weapon data that does not exist yet; they follow the same pattern and come
//! with the rules that define them.

use crate::ids::{CellId, CountryId, UnitId};
use crate::world::World;

/// What a unit projects: reach in metres and strength, both supplied by the rules.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    pub radius_m: u32,
    pub strength: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellControl {
    /// Neither side projects into it; the stored controller stands.
    Empty,
    /// Only one country projects into it.
    Held(CountryId),
    /// Two or more countries project into it.
    Contested,
}

/// Zone-of-control strengths per cell and country, as a sorted list of
/// `(cell, country, strength)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZocCache {
    entries: Vec<(u32, u16, u64)>,
}

impl ZocCache {
    /// Rebuild from the units on the map. `project` decides, per unit, whether and how
    /// far it projects; `terrain_permille` scales strength per cell (1000 = no change).
    /// Units without a cell position are skipped.
    pub fn rebuild(world: &World, project: impl Fn(UnitId) -> Option<Projection>, terrain_permille: impl Fn(CellId) -> u32) -> ZocCache {
        let Some(map) = world.map() else { return ZocCache::default() };
        let mut raw: Vec<(u32, u16, u64)> = Vec::new();
        for i in 0..world.unit_count() {
            let u = UnitId(i);
            let (Some(unit), Some(ext)) = (world.unit(u), world.unit_ext(u)) else { continue };
            let (Some(cell), Some(p)) = (ext.cell, project(u)) else { continue };
            for c in map.cells_within(cell, p.radius_m) {
                let s = p.strength as u64 * terrain_permille(c) as u64 / 1000;
                if s > 0 {
                    raw.push((c.0, unit.owner.0, s));
                }
            }
        }
        // Sort, then merge equal (cell, country) pairs. Addition is order-independent.
        raw.sort_unstable();
        let mut entries: Vec<(u32, u16, u64)> = Vec::with_capacity(raw.len());
        for e in raw {
            match entries.last_mut() {
                Some(last) if last.0 == e.0 && last.1 == e.1 => last.2 += e.2,
                _ => entries.push(e),
            }
        }
        ZocCache { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `(country, strength)` pairs for a cell, by country ID.
    pub fn at(&self, cell: CellId) -> impl Iterator<Item = (CountryId, u64)> + '_ {
        let start = self.entries.partition_point(|e| e.0 < cell.0);
        self.entries[start..].iter().take_while(move |e| e.0 == cell.0).map(|e| (CountryId(e.1), e.2))
    }

    pub fn strength(&self, cell: CellId, country: CountryId) -> u64 {
        self.at(cell).find(|(c, _)| *c == country).map_or(0, |(_, s)| s)
    }

    pub fn control(&self, cell: CellId) -> CellControl {
        let mut it = self.at(cell);
        match (it.next(), it.next()) {
            (None, _) => CellControl::Empty,
            (Some((c, _)), None) => CellControl::Held(c),
            _ => CellControl::Contested,
        }
    }
}

/// Which units stand in which cell, as a sorted list of `(cell, unit)`. Derived from
/// the units' cell positions; the spatial index behind "units within N cells".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitsByCell {
    pairs: Vec<(u32, u32)>,
}

impl UnitsByCell {
    pub fn rebuild(world: &World) -> UnitsByCell {
        let mut pairs: Vec<(u32, u32)> = (0..world.unit_count())
            .filter_map(|i| world.unit_ext(UnitId(i)).and_then(|e| e.cell).map(|c| (c.0, i)))
            .collect();
        pairs.sort_unstable();
        UnitsByCell { pairs }
    }

    /// Units in a cell, by unit ID.
    pub fn in_cell(&self, cell: CellId) -> impl Iterator<Item = UnitId> + '_ {
        let start = self.pairs.partition_point(|p| p.0 < cell.0);
        self.pairs[start..].iter().take_while(move |p| p.0 == cell.0).map(|p| UnitId(p.1))
    }

    /// Cells that hold at least one unit, ascending: the active set for supply and
    /// attrition systems.
    pub fn occupied_cells(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.pairs.iter().map(|p| p.0).collect();
        v.dedup();
        v
    }

    /// Total demand per occupied cell, where `demand(unit)` is what each unit needs.
    /// Returns `(cell, total)` ascending by cell.
    pub fn aggregate_demand(&self, demand: impl Fn(UnitId) -> u64) -> Vec<(u32, u64)> {
        let mut out: Vec<(u32, u64)> = Vec::new();
        for &(cell, unit) in &self.pairs {
            let d = demand(UnitId(unit));
            match out.last_mut() {
                Some(last) if last.0 == cell => last.1 += d,
                _ => out.push((cell, d)),
            }
        }
        out
    }

    /// Split a cell's available supply among its units with the shortage rule
    /// (proportional, remainders by unit ID). Returns `(unit, share)`.
    pub fn split_supply(&self, cell: CellId, available: i64, demand: impl Fn(UnitId) -> i64) -> Vec<(UnitId, i64)> {
        let units: Vec<UnitId> = self.in_cell(cell).collect();
        let demands: Vec<i64> = units.iter().map(|&u| demand(u)).collect();
        let shares = crate::flows::split_shortage(available, &demands);
        units.into_iter().zip(shares).collect()
    }
}

/// A connected stretch of front between two countries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontSegment {
    /// Boundary edges `(cell of a, cell of b)`, sorted.
    pub edges: Vec<(u32, u32)>,
}

/// Boundary edges between cells stored as controlled by `a` and by `b`, grouped into
/// segments. Two edges are in the same segment if they share a cell or their cells
/// touch. Segments are ordered by their smallest edge. Needs cell state (a map world).
pub fn front_segments(world: &World, a: CountryId, b: CountryId) -> Vec<FrontSegment> {
    let (Some(map), Some(cells)) = (world.map(), world.cells()) else { return Vec::new() };
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for c in 0..map.cell_count() {
        if cells.controller(c as usize) != a.0 {
            continue;
        }
        for e in map.cell_edges(CellId(c)) {
            let n = map.c_dst[e];
            if cells.controller(n as usize) == b.0 {
                edges.push((c, n));
            }
        }
    }
    edges.sort_unstable();
    // Union-find over edge indexes; link edges that share a cell on either side or whose a-cells are neighbours.
    let n = edges.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut by_a: Vec<(u32, usize)> = edges.iter().enumerate().map(|(i, e)| (e.0, i)).collect();
    by_a.sort_unstable();
    let mut by_b: Vec<(u32, usize)> = edges.iter().enumerate().map(|(i, e)| (e.1, i)).collect();
    by_b.sort_unstable();
    for group in [&by_a, &by_b] {
        for w in group.windows(2) {
            if w[0].0 == w[1].0 {
                let (x, y) = (find(&mut parent, w[0].1), find(&mut parent, w[1].1));
                parent[x.max(y)] = x.min(y);
            }
        }
    }
    // a-cells that are neighbours of each other
    for &(cell, i) in &by_a {
        for e in map.cell_edges(CellId(cell)) {
            let nb = map.c_dst[e];
            if let Ok(pos) = by_a.binary_search_by_key(&nb, |x| x.0) {
                // binary_search may land on any edge of that cell; all edges of one cell are already joined
                let (x, y) = (find(&mut parent, i), find(&mut parent, by_a[pos].1));
                parent[x.max(y)] = x.min(y);
            }
        }
    }
    let mut groups: Vec<Vec<(u32, u32)>> = Vec::new();
    let mut root_slot: Vec<Option<usize>> = vec![None; n];
    for (i, &edge) in edges.iter().enumerate() {
        let r = find(&mut parent, i);
        let slot = *root_slot[r].get_or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[slot].push(edge);
    }
    groups.into_iter().map(|edges| FrontSegment { edges }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::tests::test_map;
    use crate::map::Map;
    use crate::world::Unit;
    use crate::ids::ProvinceId;
    use std::sync::Arc;

    fn world() -> World {
        World::from_map(1, Arc::new(Map::from_raw(test_map(16, 16)).unwrap()))
    }

    fn place(w: &mut World, owner: u16, cell: u32) -> UnitId {
        let id = w.add_unit_for_setup(Unit { owner: CountryId(owner), location: ProvinceId(0) });
        w.unit_ext_mut(id).unwrap().cell = Some(CellId(cell));
        id
    }

    fn proj(_: UnitId) -> Option<Projection> {
        Some(Projection { radius_m: 15_000, strength: 100 })
    }

    #[test]
    fn zoc_is_held_contested_or_empty() {
        let mut w = world();
        // 16x16 cells, 10 km apart: cell index = y*16 + x
        place(&mut w, 0, 5 * 16 + 3);
        place(&mut w, 1, 5 * 16 + 5);
        let z = ZocCache::rebuild(&w, proj, |_| 1000);
        assert_eq!(z.control(CellId(5 * 16 + 2)), CellControl::Held(CountryId(0)));
        assert_eq!(z.control(CellId(5 * 16 + 6)), CellControl::Held(CountryId(1)));
        assert_eq!(z.control(CellId(5 * 16 + 4)), CellControl::Contested);
        assert_eq!(z.control(CellId(15 * 16 + 15)), CellControl::Empty);
        assert_eq!(z.strength(CellId(5 * 16 + 3), CountryId(0)), 100);
    }

    #[test]
    fn rebuild_is_identical_and_independent_of_unit_order() {
        let mut a = world();
        place(&mut a, 0, 20);
        place(&mut a, 0, 21);
        place(&mut a, 1, 100);
        let mut b = world();
        place(&mut b, 1, 100);
        place(&mut b, 0, 21);
        place(&mut b, 0, 20);
        let za = ZocCache::rebuild(&a, proj, |c| 500 + c.0 % 400);
        assert_eq!(za, ZocCache::rebuild(&a, proj, |c| 500 + c.0 % 400));
        assert_eq!(za, ZocCache::rebuild(&b, proj, |c| 500 + c.0 % 400));
        assert!(!za.is_empty());
        // stacked units add up
        let mut s = world();
        place(&mut s, 0, 20);
        place(&mut s, 0, 20);
        assert_eq!(ZocCache::rebuild(&s, proj, |_| 1000).strength(CellId(20), CountryId(0)), 200);
    }

    #[test]
    fn units_by_cell_aggregates_demand_and_splits_supply() {
        let mut w = world();
        let a = place(&mut w, 0, 7);
        let b = place(&mut w, 0, 7);
        place(&mut w, 1, 40);
        let idx = UnitsByCell::rebuild(&w);
        assert_eq!(idx.in_cell(CellId(7)).collect::<Vec<_>>(), vec![a, b]);
        assert_eq!(idx.occupied_cells(), vec![7, 40]);
        assert_eq!(idx.aggregate_demand(|u| 10 + u.0 as u64), vec![(7, 21), (40, 12)]);
        // 9 units of supply for demands 10 and 11 -> proportional 4 and 5 (floor 4,4 + 1 spare to unit a)
        let split = idx.split_supply(CellId(7), 9, |u| 10 + u.0 as i64);
        assert_eq!(split.iter().map(|s| s.1).sum::<i64>(), 9);
        assert_eq!(idx, UnitsByCell::rebuild(&w));
    }

    #[test]
    fn caches_do_not_touch_the_hash() {
        let mut w = world();
        place(&mut w, 0, 20);
        let before = w.state_hash();
        let _ = ZocCache::rebuild(&w, proj, |_| 1000);
        let _ = front_segments(&w, CountryId(0), CountryId(1));
        assert_eq!(w.state_hash(), before);
    }

    #[test]
    fn front_runs_along_the_border_between_countries() {
        let mut w = world();
        // make cells with x < 8 country 0 and the rest country 1 (the test map already splits at x = w/2)
        let c = w.cells_mut().unwrap();
        for i in 0..256 {
            c.set_controller(i, if i % 16 < 8 { 0 } else { 1 });
        }
        let segs = front_segments(&w, CountryId(0), CountryId(1));
        assert_eq!(segs.len(), 1, "one straight border is one segment");
        // each of the 16 rows contributes at least the edge (x=7) -> (x=8)
        assert!(segs[0].edges.len() >= 16);
        assert!(segs[0].edges.iter().all(|&(a, b)| a % 16 < 8 && b % 16 >= 8));
        // two separate pockets give two segments
        let c = w.cells_mut().unwrap();
        for i in 0..256 {
            c.set_controller(i, 0);
        }
        c.set_controller(0, 1);
        c.set_controller(16 * 15 + 15, 1);
        assert_eq!(front_segments(&w, CountryId(0), CountryId(1)).len(), 2);
    }
}
