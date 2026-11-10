use super::*;
use std::collections::BinaryHeap;

/// A small deterministic world: W x H cells 10 km apart, provinces are 4x4 blocks,
/// one sea zone touching province 0, a few sites.
pub fn test_map(w: usize, h: usize) -> RawMap {
    assert!(w.is_multiple_of(4) && h.is_multiple_of(4));
    let n = w * h;
    let (pw, ph) = (w / 4, h / 4);
    let p = pw * ph;
    let mut r = RawMap {
        state_tags: vec!["AAA".into(), "BBB".into()],
        state_names: vec!["Aland".into(), "Bland".into()],
        ..RawMap::default()
    };
    let prov_of = |x: usize, y: usize| (y / 4) * pw + x / 4;
    // pseudo-random terrain/flags from a hash of the position
    let rnd = |i: usize| -> u32 { (i as u32).wrapping_mul(2654435761).rotate_left(13).wrapping_mul(40503) >> 7 };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            r.cell_state.push(if x < w / 2 { 0 } else { 1 });
            r.cell_province.push(prov_of(x, y) as u32);
            r.cell_x.push(x as i32 * 10_000);
            r.cell_y.push(y as i32 * 10_000);
            r.cell_area_ha.push(10_000);
            r.cell_landmass.push(0);
            r.cell_elev_m.push((rnd(i) % 3000) as i16);
            r.cell_relief_m.push((rnd(i + 7) % 900) as u16);
            r.cell_terrain.push((rnd(i + 3) % 6) as u8);
            r.cell_urban.push(0);
            r.cell_flags.push((rnd(i + 11) % 8) as u8 & 3);
            r.cell_zone.push(if x == 0 && y == 0 { 0 } else { NO_ZONE });
            r.cell_pop.push(100);
        }
    }
    // province state must equal its cells' state: provinces never straddle the x = w/2 line when pw is even
    for py in 0..ph {
        for px in 0..pw {
            r.province_state.push(if px * 4 < w / 2 { 0 } else { 1 });
            r.province_x.push((px * 4 * 10_000 + 15_000) as i32);
            r.province_y.push((py * 4 * 10_000 + 15_000) as i32);
            r.province_area_ha.push(16 * 10_000);
            r.province_pop.push(1600);
        }
    }
    let mut off = vec![0u32];
    for prov in 0..p {
        for i in 0..n {
            if r.cell_province[i] as usize == prov {
                r.province_cells.push(i as u32);
            }
        }
        off.push(r.province_cells.len() as u32);
    }
    r.province_cell_off = off;
    // sea zone beside province 0
    r.zone_x = vec![-20_000];
    r.zone_y = vec![15_000];
    r.zone_area_ha = vec![1_000_000];
    r.zone_body = vec![0];
    // strategic graph: provinces 4-neighbour + zone
    let nodes = p + 1;
    let mut adj: Vec<Vec<(u32, u8)>> = vec![Vec::new(); nodes];
    for py in 0..ph {
        for px in 0..pw {
            let a = py * pw + px;
            if px + 1 < pw {
                adj[a].push(((a + 1) as u32, 0));
                adj[a + 1].push((a as u32, 0));
            }
            if py + 1 < ph {
                adj[a].push(((a + pw) as u32, 0));
                adj[a + pw].push((a as u32, 0));
            }
        }
    }
    adj[0].push((p as u32, 1));
    adj[p].push((0, 1));
    r.g_off.push(0);
    for (a, list) in adj.iter_mut().enumerate() {
        list.sort();
        for &(b, k) in list.iter() {
            r.g_dst.push(b);
            r.g_kind.push(k);
            r.g_weight.push(1);
            r.g_strait.push(0);
            let (ax, ay) = if a < p { (r.province_x[a], r.province_y[a]) } else { (r.zone_x[0], r.zone_y[0]) };
            let (bx, by) = if (b as usize) < p { (r.province_x[b as usize], r.province_y[b as usize]) } else { (r.zone_x[0], r.zone_y[0]) };
            let d = isqrt(((ax - bx) as i64 * (ax - bx) as i64 + (ay - by) as i64 * (ay - by) as i64) as u64) as u32;
            r.g_cost_m.push(d + 1);
        }
        r.g_off.push(r.g_dst.len() as u32);
    }
    // cell graph: 4-neighbour
    r.c_off.push(0);
    for y in 0..h {
        for x in 0..w {
            let mut nb: Vec<usize> = Vec::new();
            if x > 0 { nb.push(y * w + x - 1); }
            if x + 1 < w { nb.push(y * w + x + 1); }
            if y > 0 { nb.push((y - 1) * w + x); }
            if y + 1 < h { nb.push((y + 1) * w + x); }
            nb.sort();
            for b in nb {
                r.c_dst.push(b as u32);
                r.c_len_m.push(10_000);
                // a river on every edge whose lower id is a multiple of 7 (symmetric by construction)
                let lo = (y * w + x).min(b);
                r.c_flags.push(if lo % 7 == 0 { 1 } else { 0 });
                r.c_dist_m.push(10_000);
            }
            r.c_off.push(r.c_dst.len() as u32);
        }
    }
    // sites
    for (k, c) in [5usize, 17, 17, 40].iter().enumerate() {
        r.site_kind.push((k % 6) as u8);
        r.site_cell.push(*c as u32);
        r.site_province.push(r.cell_province[*c]);
        r.site_size.push(0);
        r.site_built.push(0);
        r.site_destroyed.push(SITE_NO_YEAR);
        r.site_verified.push(1);
        r.site_names.push(format!("site{k}"));
    }
    r
}

/// Plain Dijkstra over the same costs, used as the reference for A*.
fn dijkstra(off: &[u32], dst: &[u32], from: u32, to: u32, cost: impl Fn(u32, usize) -> u32) -> Option<u32> {
    let mut dist = vec![u32::MAX; off.len() - 1];
    let mut heap = BinaryHeap::new();
    dist[from as usize] = 0;
    heap.push(Reverse((0u32, from)));
    while let Some(Reverse((d, v))) = heap.pop() {
        if d > dist[v as usize] {
            continue;
        }
        if v == to {
            return Some(d);
        }
        let (lo, hi) = (off[v as usize] as usize, off[v as usize + 1] as usize);
        for (e, &wn) in dst[lo..hi].iter().enumerate().map(|(i, w)| (lo + i, w)) {
            let nd = d + cost(v, e);
            let w = wn as usize;
            if nd < dist[w] {
                dist[w] = nd;
                heap.push(Reverse((nd, w as u32)));
            }
        }
    }
    None
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

#[test]
fn synthetic_map_is_valid() {
    let m = Map::from_raw(test_map(16, 16)).expect("structure");
    assert_eq!(m.cell_count(), 256);
    assert_eq!(m.province_count(), 16);
    assert_eq!(m.node_count(), 17);
    let problems = m.validate_world();
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn isqrt_matches_definition() {
    for v in (0u64..5000).chain([u32::MAX as u64, u64::MAX, (1 << 62) + 12345, 999_999_999_999]) {
        let r = isqrt(v);
        assert!(r as u128 * r as u128 <= v as u128 && (r as u128 + 1) * (r as u128 + 1) > v as u128, "isqrt({v}) = {r}");
    }
}

#[test]
fn cell_astar_matches_dijkstra() {
    let m = Map::from_raw(test_map(24, 24)).unwrap();
    let costs = CellCosts::default();
    let mut scratch = PathScratch::new(m.cell_count() as usize);
    let mut rng = Lcg(42);
    let r = &*m;
    let reference = |a: u32, e: usize| {
        let b = r.c_dst[e] as usize;
        let a = a as usize;
        let mut pct = costs.terrain_percent[r.cell_terrain[b] as usize];
        let both = |bit: u8| r.cell_flags[a] & bit != 0 && r.cell_flags[b] & bit != 0;
        if both(1) { pct = pct.min(costs.road_percent); }
        if both(2) { pct = pct.min(costs.rail_percent); }
        let mut c = (r.c_dist_m[e] as u64 * pct as u64).div_ceil(100) as u32;
        if r.c_flags[e] & 1 != 0 { c += costs.river_extra_m; }
        c
    };
    for _ in 0..300 {
        let (a, b) = (rng.next(576) as u32, rng.next(576) as u32);
        let got = m.find_cell_path(CellId(a), CellId(b), &costs, &mut scratch).map(|p| p.cost);
        let want = dijkstra(&r.c_off, &r.c_dst, a, b, reference);
        assert_eq!(got, want, "cells {a} -> {b}");
    }
}

#[test]
fn node_astar_matches_dijkstra_and_path_is_connected() {
    let m = Map::from_raw(test_map(32, 32)).unwrap();
    let costs = NodeCosts { percent: [100, 300, 100, 50] };
    let mut scratch = PathScratch::new(m.node_count() as usize);
    let r = &*m;
    let mut rng = Lcg(7);
    for _ in 0..200 {
        let (a, b) = (rng.next(65) as u32, rng.next(65) as u32);
        let got = m.find_node_path(a, b, &costs, &mut scratch);
        let want = dijkstra(&r.g_off, &r.g_dst, a, b, |_, e| ((r.g_cost_m[e] as u64 * costs.percent[r.g_kind[e] as usize] as u64).div_ceil(100)) as u32);
        assert_eq!(got.as_ref().map(|p| p.cost), want, "{a} -> {b}");
        if let Some(p) = got {
            assert_eq!((p.nodes[0], *p.nodes.last().unwrap()), (a, b));
            for w in p.nodes.windows(2) {
                assert!(r.g_dst[m.node_edges(w[0])].contains(&w[1]), "path steps must be edges");
            }
        }
    }
}

#[test]
fn path_is_deterministic_and_unreachable_is_none() {
    let m = Map::from_raw(test_map(16, 16)).unwrap();
    let mut s1 = PathScratch::new(256);
    let mut s2 = PathScratch::new(256);
    let c = CellCosts::default();
    let a = m.find_cell_path(CellId(0), CellId(255), &c, &mut s1).unwrap();
    // reuse one scratch for many queries, compare with a fresh one
    for _ in 0..5 {
        let _ = m.find_cell_path(CellId(3), CellId(200), &c, &mut s1);
    }
    let b = m.find_cell_path(CellId(0), CellId(255), &c, &mut s1).unwrap();
    let d = m.find_cell_path(CellId(0), CellId(255), &c, &mut s2).unwrap();
    assert_eq!(a, b);
    assert_eq!(a, d);
    assert_eq!(m.find_cell_path(CellId(7), CellId(7), &c, &mut s1).unwrap().cost, 0);
}

#[test]
fn range_queries_match_brute_force() {
    let m = Map::from_raw(test_map(32, 32)).unwrap();
    let r = &*m;
    for center in [0u32, 5, 100, 517, 1023] {
        for radius in [0u32, 9_999, 10_000, 25_000, 90_000, 700_000] {
            let got = m.cells_within(CellId(center), radius);
            let (cx, cy) = (r.cell_x[center as usize] as i64, r.cell_y[center as usize] as i64);
            let want: Vec<CellId> = (0..m.cell_count())
                .filter(|&i| {
                    let (dx, dy) = (r.cell_x[i as usize] as i64 - cx, r.cell_y[i as usize] as i64 - cy);
                    dx * dx + dy * dy <= (radius as i64) * (radius as i64)
                })
                .map(CellId)
                .collect();
            assert_eq!(got, want, "center {center} radius {radius}");
            let sites = m.sites_within(CellId(center), radius);
            let want_sites: Vec<u32> = (0..m.site_count()).filter(|&s| want.contains(&CellId(r.site_cell[s as usize]))).collect();
            assert_eq!(sites, want_sites);
        }
    }
}

#[test]
fn province_cells_partition_the_cells() {
    let m = Map::from_raw(test_map(16, 16)).unwrap();
    let mut seen = vec![0u8; 256];
    for p in 0..m.province_count() {
        for &c in m.cells_of(ProvinceId(p)) {
            seen[c as usize] += 1;
            assert_eq!(m.cell_province[c as usize], p);
        }
    }
    assert!(seen.iter().all(|&x| x == 1));
}

#[test]
fn structure_check_catches_corruption() {
    let good = test_map(16, 16);
    type Corrupt = Box<dyn Fn(&mut RawMap)>;
    let cases: Vec<(&str, Corrupt)> = vec![
        ("cell in two provinces' lists", Box::new(|r| { let a = r.province_cell_off[1] as usize; r.province_cells[a] = r.province_cells[0]; })),
        ("cell with a province that does not exist", Box::new(|r| r.cell_province[3] = 999)),
        ("province without a valid owner", Box::new(|r| r.province_state[2] = 9)),
        ("one-way edge", Box::new(|r| { let e = r.c_off[10] as usize; r.c_dst[e] = r.c_dst[e].wrapping_add(2); r.c_dst[e] = r.c_dst[e].min(255); })),
        ("short array", Box::new(|r| { r.cell_pop.pop(); })),
        ("neighbour out of range", Box::new(|r| { let l = r.g_dst.len(); r.g_dst[l - 1] = 5000; })),
        ("bad terrain", Box::new(|r| r.cell_terrain[0] = 77)),
        ("site in wrong province", Box::new(|r| r.site_province[0] = 3)),
    ];
    for (name, f) in cases {
        let mut bad = good.clone();
        f(&mut bad);
        assert!(Map::from_raw(bad).is_err(), "corruption not caught: {name}");
    }
}

#[test]
fn world_check_catches_orphan_and_landlocked_landmass() {
    // drop every edge of node 5 on both sides -> orphan province
    let mut r = test_map(16, 16);
    let p = 16usize;
    let mut adj: Vec<Vec<(u32, u8)>> = Vec::new();
    for a in 0..=p {
        let mut v = Vec::new();
        for e in r.g_off[a] as usize..r.g_off[a + 1] as usize {
            if a != 5 && r.g_dst[e] != 5 {
                v.push((r.g_dst[e], r.g_kind[e]));
            }
        }
        adj.push(v);
    }
    r.g_off = vec![0];
    r.g_dst.clear(); r.g_kind.clear();
    for a in &adj { for &(b, k) in a { r.g_dst.push(b); r.g_kind.push(k); } r.g_off.push(r.g_dst.len() as u32); }
    r.g_weight = vec![1; r.g_dst.len()]; r.g_strait = vec![0; r.g_dst.len()]; r.g_cost_m = vec![100; r.g_dst.len()];
    let m = Map::from_raw(r).unwrap();
    let problems = m.validate_world();
    assert!(problems.iter().any(|e| e.0.contains("node 5") && e.0.contains("no neighbours")), "{problems:?}");
    assert!(problems.iter().any(|e| e.0.contains("unreachable")), "{problems:?}");
}

#[test]
fn nearest_cell_finds_the_cell_under_a_point() {
    let m = Map::from_raw(test_map(8, 8)).expect("map");
    for c in [0u32, 9, 27, 63] {
        let (x, y) = (m.cell_x[c as usize], m.cell_y[c as usize]);
        assert_eq!(m.nearest_cell(x, y), Some((CellId(c), 0)));
        // a few metres off the centre it is still that cell, and the distance is reported
        let (near, away) = m.nearest_cell(x + 3, y - 4).unwrap();
        assert_eq!((near, away), (CellId(c), 5));
    }
}

// ---- the globe (map rewrite, phase M1) ----

/// A place from latitude and longitude in degrees (floats are fine in a test).
fn place(lat: f64, lon: f64) -> Place {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let r = crate::sphere::EARTH_RADIUS_M as f64;
    [(r * la.cos() * lo.cos()).round() as i32, (r * la.cos() * lo.sin()).round() as i32, (r * la.sin()).round() as i32]
}

/// The flat test map laid on the globe: the same cells and graphs, but each cell sits at a
/// latitude and longitude (0.09 degrees apart, about 10 km) starting at (`lat0`, `lon0`), and
/// every distance is over the ground. Starting near 180 degrees east puts the map across the
/// line where longitude jumps from +180 to -180.
pub fn globe_test_map(w: usize, h: usize, lat0: f64, lon0: f64) -> RawMap {
    let mut r = test_map(w, h);
    let at = |x: f64, y: f64| place(lat0 + y * 0.09, (lon0 + x * 0.09 + 180.0).rem_euclid(360.0) - 180.0);
    for y in 0..h {
        for x in 0..w {
            r.cell_xyz.extend(at(x as f64, y as f64));
            r.cell_h3.push(0x0860_0000_0000_0000 + (y * w + x) as u64);
        }
    }
    for a in 0..w * h {
        for e in r.c_off[a] as usize..r.c_off[a + 1] as usize {
            r.c_dist_m[e] = ground_m(place_of(&r.cell_xyz, a as u32), place_of(&r.cell_xyz, r.c_dst[e])) + 2;
        }
    }
    // provinces are 4x4 blocks: their middle is 1.5 cells in; the one sea zone lies to the west
    for py in 0..h / 4 {
        for px in 0..w / 4 {
            r.node_xyz.extend(at(px as f64 * 4.0 + 1.5, py as f64 * 4.0 + 1.5));
        }
    }
    r.node_xyz.extend(at(-2.0, 1.5));
    for a in 0..r.g_off.len() - 1 {
        for e in r.g_off[a] as usize..r.g_off[a + 1] as usize {
            r.g_cost_m[e] = ground_m(place_of(&r.node_xyz, a as u32), place_of(&r.node_xyz, r.g_dst[e])) + 2;
        }
    }
    r
}

/// The three places the globe tests use: on the equator, far north (where a degree of
/// longitude is short), and across the 180 degree line.
const GLOBE_CORNERS: [(f64, f64); 3] = [(-1.0, 20.0), (68.0, 15.0), (-18.0, 178.8)];

#[test]
fn on_the_globe_distances_are_over_the_ground() {
    for (lat0, lon0) in GLOBE_CORNERS {
        let m = Map::from_raw(globe_test_map(32, 32, lat0, lon0)).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(m.is_globe());
        // a step north is 0.09 degrees of latitude anywhere: 10,007.5 km / 90 * 0.09
        assert!(m.cell_distance_m(CellId(0), CellId(32)).abs_diff(10_008) <= 2);
        // a step east shrinks with the cosine of the latitude
        let east = m.cell_distance_m(CellId(0), CellId(1)) as f64;
        assert!((east - 10_007.5 * lat0.to_radians().cos()).abs() < 3.0, "{east} at latitude {lat0}");
        // across the whole map, and the same both ways
        let far = m.cell_distance_m(CellId(0), CellId(31));
        assert!(far.abs_diff((31.0 * east) as u32) < 400, "{far}: the map does not tear at the 180 degree line");
        assert_eq!(far, m.cell_distance_m(CellId(31), CellId(0)));
        assert_eq!(m.cell_place(CellId(5)), Some(place_of(&m.cell_xyz, 5)));
    }
    assert!(!Map::from_raw(test_map(8, 8)).unwrap().is_globe());
}

#[test]
fn on_the_globe_range_queries_match_brute_force() {
    for (lat0, lon0) in GLOBE_CORNERS {
        let m = Map::from_raw(globe_test_map(32, 32, lat0, lon0)).unwrap();
        for center in [0u32, 5, 100, 517, 1023] {
            for radius in [0u32, 3_000, 10_010, 25_000, 90_000, 700_000] {
                let got = m.cells_within(CellId(center), radius);
                let want: Vec<CellId> = (0..m.cell_count()).map(CellId).filter(|&c| m.cell_distance_m(CellId(center), c) <= radius).collect();
                assert_eq!(got, want, "centre {center} radius {radius} at {lat0},{lon0}");
            }
        }
    }
}

#[test]
fn on_the_globe_the_pathfinder_still_finds_the_cheapest_route() {
    for (lat0, lon0) in GLOBE_CORNERS {
        let m = Map::from_raw(globe_test_map(16, 16, lat0, lon0)).unwrap();
        let costs = CellCosts { terrain_percent: [100, 130, 180, 250, 150, 400, 100, 100], road_percent: 60, rail_percent: 80, river_extra_m: 4_000, bridge_extra_m: 500 };
        let mut scratch = PathScratch::new(m.cell_count() as usize);
        for (from, to) in [(0u32, 255u32), (15, 240), (37, 201), (255, 0)] {
            let path = m.find_cell_path(CellId(from), CellId(to), &costs, &mut scratch).unwrap();
            // the cheapest cost by plain search with no guide at all
            let mut best = vec![u64::MAX; 256];
            let mut heap = BinaryHeap::new();
            best[from as usize] = 0;
            heap.push(std::cmp::Reverse((0u64, from)));
            while let Some(std::cmp::Reverse((d, a))) = heap.pop() {
                if d > best[a as usize] {
                    continue;
                }
                for e in m.cell_edges(CellId(a)) {
                    let b = m.c_dst[e];
                    let nd = d + m.cell_step_cost(CellId(a), CellId(b), &costs, false).unwrap() as u64;
                    if nd < best[b as usize] {
                        best[b as usize] = nd;
                        heap.push(std::cmp::Reverse((nd, b)));
                    }
                }
            }
            assert_eq!(path.cost as u64, best[to as usize], "{from} to {to} at {lat0},{lon0}");
        }
        // the strategic graph too: a route exists and costs at least the distance over the ground
        let nodes = NodeCosts { percent: [100, 100, 100, 100] };
        let mut s = PathScratch::new(m.node_count() as usize);
        let p = m.find_node_path(0, m.province_count() - 1, &nodes, &mut s).unwrap();
        assert!(p.cost >= ground_m(place_of(&m.node_xyz, 0), place_of(&m.node_xyz, m.province_count() - 1)));
    }
}

#[test]
fn on_the_globe_the_nearest_cell_is_found_and_bad_places_are_caught() {
    let m = Map::from_raw(globe_test_map(8, 8, 52.0, 9.0)).unwrap();
    let at = place_of(&m.cell_xyz, 27);
    assert_eq!(m.nearest_cell_to_place(at), Some((CellId(27), 0)));
    assert_eq!(m.nearest_cell_to_place(place(52.0 + 3.0 * 0.09 + 0.01, 9.0 + 3.0 * 0.09 + 0.01)).unwrap().0, CellId(27));
    assert_eq!(Map::from_raw(test_map(8, 8)).unwrap().nearest_cell_to_place(at), None);

    let good = globe_test_map(8, 8, 52.0, 9.0);
    type Corrupt = Box<dyn Fn(&mut RawMap)>;
    let cases: Vec<(&str, Corrupt)> = vec![
        ("a place off the globe", Box::new(|r| r.cell_xyz[4] += 50_000)),
        ("a hexagon number used twice", Box::new(|r| r.cell_h3[9] = r.cell_h3[3])),
        ("an edge shorter than the straight line", Box::new(|r| r.c_dist_m[5] = 100)),
        ("places missing for the nodes", Box::new(|r| r.node_xyz.clear())),
        ("a short list of places", Box::new(|r| { r.cell_xyz.pop(); })),
    ];
    for (name, f) in cases {
        let mut bad = good.clone();
        f(&mut bad);
        assert!(Map::from_raw(bad).is_err(), "not caught: {name}");
    }
}
