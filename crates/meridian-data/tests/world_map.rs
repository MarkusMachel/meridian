//! Phase 3 done tests against the real baked world map.
//! The file is produced by crates/map-builder (not committed). If it is missing the tests
//! print a notice and pass, unless MERIDIAN_REQUIRE_MAP=1 (set by scripts/check.sh).
use meridian_core::{CellCosts, CellId, NodeCosts, PathScratch, ProvinceId};
use meridian_data::load_map_file;
use std::path::PathBuf;

fn map_path() -> PathBuf {
    std::env::var_os("MERIDIAN_MAP").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/baked/meridian-map.bin"))
}

fn load() -> Option<meridian_core::Map> {
    let p = map_path();
    if !p.exists() {
        assert!(std::env::var_os("MERIDIAN_REQUIRE_MAP").is_none(), "map file {} is required but missing", p.display());
        eprintln!("NOTE: {} not found, skipping world-map test", p.display());
        return None;
    }
    Some(load_map_file(&p).unwrap_or_else(|e| panic!("{}", e.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("\n"))))
}

#[test]
fn world_map_loads_and_validates_with_zero_errors() {
    let Some(m) = load() else { return };
    let problems = m.validate_world();
    assert!(problems.is_empty(), "{} problems, first: {:?}", problems.len(), &problems[..problems.len().min(5)]);
    assert!((2_500..=4_500).contains(&m.province_count()), "provinces: {}", m.province_count());
    assert!((3_500_000..=3_900_000).contains(&m.cell_count()), "hexagons: {}", m.cell_count());
    assert!(m.is_globe(), "the baked map is hexagons on the globe");
    assert_eq!(m.state_count(), 173);
}

#[test]
fn every_cell_belongs_to_exactly_one_province_and_areas_match() {
    let Some(m) = load() else { return };
    let mut count = vec![0u8; m.cell_count() as usize];
    for p in 0..m.province_count() {
        for &c in m.cells_of(ProvinceId(p)) {
            count[c as usize] += 1;
        }
    }
    assert!(count.iter().all(|&c| c == 1));
    let total_ha: u64 = m.cell_area_ha.iter().map(|&a| a as u64).sum();
    let km2 = total_ha / 100;
    assert!((133_000_000..136_000_000).contains(&km2), "land area {km2} km2");
}

#[test]
fn known_chokepoints_exist_as_named_strait_edges() {
    let Some(m) = load() else { return };
    let mut named = std::collections::BTreeSet::new();
    for &s in &m.g_strait {
        if s > 0 {
            named.insert(m.choke_names[s as usize - 1].clone());
        }
    }
    for want in ["Strait of Gibraltar", "Turkish Straits", "Suez Canal", "Strait of Hormuz", "Strait of Malacca", "Panama Canal"] {
        assert!(named.contains(want), "{want} missing; have {named:?}");
    }
}

/// A place from latitude and longitude in degrees (floats are fine in a test).
fn place(lat: f64, lon: f64) -> meridian_core::sphere::Place {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let r = meridian_core::sphere::EARTH_RADIUS_M as f64;
    [(r * la.cos() * lo.cos()).round() as i32, (r * la.cos() * lo.sin()).round() as i32, (r * la.sin()).round() as i32]
}

/// Map rewrite M1: known cities sit on a hexagon of the right country, and the distance
/// between their hexagons is the real distance to within 1 % (plus one hexagon each end,
/// because a city is somewhere inside its hexagon, not at the centre).
#[test]
fn cities_are_where_they_should_be_on_the_globe() {
    let Some(m) = load() else { return };
    let cities = [
        ("Hannover", 52.37, 9.73, "BRD"),
        ("Leipzig", 51.34, 12.37, "DDR"),
        ("Baghdad", 33.31, 44.37, "IRQ"),
        ("Kuwait City", 29.38, 47.98, "KWT"),
        ("Moscow", 55.75, 37.62, "SUN"),
        ("Washington", 38.90, -77.04, "USA"),
        ("Singapore", 1.35, 103.82, "SGP"),
        ("Tromso", 69.65, 18.96, "NOR"),
        ("Wellington", -41.29, 174.78, "NZL"),
        ("Suva", -18.14, 178.44, "FJI"),
        ("Anadyr", 64.73, 177.51, "SUN"),
    ];
    let cells: Vec<CellId> = cities
        .iter()
        .map(|&(name, lat, lon, tag)| {
            let (cell, away) = m.nearest_cell_to_place(place(lat, lon)).unwrap();
            assert!(away < 7_000, "{name}: the nearest hexagon centre is {away} m away");
            assert_eq!(m.state_tags[m.cell_state[cell.index()] as usize], tag, "{name}");
            cell
        })
        .collect();
    for (i, a) in cities.iter().enumerate() {
        for (j, b) in cities.iter().enumerate() {
            let real = meridian_core::sphere::ground_m(place(a.1, a.2), place(b.1, b.2)) as f64;
            let got = m.cell_distance_m(cells[i], cells[j]) as f64;
            assert!((got - real).abs() <= real * 0.01 + 8_000.0, "{} to {}: {got} m, really {real} m", a.0, b.0);
        }
    }
    // every edge is about one hexagon long (level 6: 4.4 to 7.2 km between centres)
    for a in 0..m.cell_count() {
        for e in m.cell_edges(CellId(a)) {
            assert!((4_300..=7_300).contains(&m.c_dist_m[e]), "edge from cell {a}: {} m", m.c_dist_m[e]);
        }
    }
}

#[test]
fn paths_work_across_the_world() {
    let Some(m) = load() else { return };
    // Paris <-> Moscow style check by state: find the first provinces of FRA and SUN
    let find = |tag: &str| -> u32 {
        let s = m.state_tags.iter().position(|t| t == tag).unwrap() as u16;
        m.province_state.iter().position(|&x| x == s).unwrap() as u32
    };
    let mut scratch = PathScratch::new(m.node_count() as usize);
    let p = m.find_node_path(find("FRA"), find("SUN"), &NodeCosts::default(), &mut scratch).expect("France to the Soviet Union");
    assert!(p.nodes.len() > 3 && p.cost > 1_000_000, "{p:?}");
    // sea route Americas <-> Asia exists too
    let q = m.find_node_path(find("USA"), find("JPN"), &NodeCosts::default(), &mut scratch).expect("US to Japan");
    assert!(q.nodes.iter().any(|&n| m.is_sea_node(n)), "must cross the sea");
    // cell path inside one province-neighbourhood
    let mut cs = PathScratch::new(m.cell_count() as usize);
    let cells = m.cells_of(ProvinceId(find("BRD")));
    let (a, b) = (CellId(cells[0]), CellId(*cells.last().unwrap()));
    let path = m.find_cell_path(a, b, &CellCosts::default(), &mut cs);
    assert!(path.is_some() || m.cell_landmass[a.index()] != m.cell_landmass[b.index()]);
}

#[test]
fn corrupt_map_files_are_refused() {
    let p = map_path();
    if !p.exists() {
        return;
    }
    let bytes = std::fs::read(&p).unwrap();
    use meridian_data::parse_map;
    assert!(parse_map("t", &bytes[..bytes.len() - 5]).is_err());
    let mut flipped = bytes.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 1;
    assert!(parse_map("t", &flipped).is_err());
    assert!(parse_map("t", b"MRMP").is_err());
    let mut v = bytes.clone();
    v[4] = 9;
    assert_eq!(parse_map("t", &v).unwrap_err()[0].kind, meridian_data::ErrorKind::WrongVersion);
}

#[test]
fn world_on_the_real_map_is_deterministic_and_hash_covers_the_map() {
    use meridian_core::{Command, CountryId, Ruleset, Signals, World};
    use std::sync::Arc;
    struct Nothing;
    impl Ruleset for Nothing {
        fn run(&self, _: &mut World, _: Signals) {}
    }
    let Some(m) = load() else { return };
    let m = Arc::new(m);
    let run = |seed: u64| {
        let mut w = World::from_map(seed, m.clone());
        assert_eq!(w.country_count() as u32, m.state_count());
        assert_eq!(w.province_count(), m.province_count());
        for p in [0u32, 2000, 4000] {
            assert_eq!(w.controller(ProvinceId(p)).0, m.province_state[p as usize]);
        }
        for _ in 0..48 {
            w.tick(&[], &Nothing);
        }
        // a command on a real province works and changes the hash
        let before = w.state_hash();
        let owner = w.controller(ProvinceId(7));
        let r = w.tick(&[Command::SetProvinceController { issuer: owner, province: ProvinceId(7), controller: CountryId(1) }], &Nothing);
        assert!(r.rejected.is_empty());
        assert_ne!(before, w.state_hash());
        w.state_hash()
    };
    assert_eq!(run(5), run(5));
    assert_ne!(run(5), run(6));
    // the map is part of the hash: same seed, no map, different hash
    let mapless = World::new(5, m.state_count() as u16, m.province_count());
    assert_ne!(mapless.state_hash(), World::from_map(5, m.clone()).state_hash());
}
