//! Map rewrite M2 done tests on the real hexagon map: rivers run along hexagon sides without
//! gaps, and a unit crosses by the bridge, or goes round or fords when the bridge is down.
//! Skipped when no baked map is present (set MERIDIAN_REQUIRE_MAP to make that a failure).

use meridian_core::map::{CellCosts, PathScratch};
use meridian_core::sphere::{ground_m, place_from_micro_degrees};
use meridian_core::{CellId, Command, CountryId, Map, UnitId, World};
use std::sync::Arc;

fn load() -> Option<Arc<Map>> {
    let path = std::env::var_os("MERIDIAN_MAP").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/baked/meridian-map.bin"));
    if !path.exists() {
        assert!(std::env::var_os("MERIDIAN_REQUIRE_MAP").is_none(), "map file {} is required but missing", path.display());
        eprintln!("skipping: no baked map at {}", path.display());
        return None;
    }
    Some(Arc::new(meridian_data::load_map_file(&path).unwrap()))
}

/// The hexagon at a latitude and longitude in degrees.
fn cell_at(m: &Map, lat: f64, lon: f64) -> CellId {
    let (cell, away) = m.nearest_cell_to_place(place_from_micro_degrees((lat * 1e6) as i32, (lon * 1e6) as i32)).expect("a globe map");
    assert!(away < 7_000, "no land hexagon at {lat},{lon}");
    cell
}

/// Two places on opposite banks of a great river, a short way apart. With fording made to
/// cost 5,000 km and no bridges, the cheapest route between them must either pay that or go
/// a long way round (the source). A gap in the river would let it through cheaply.
#[test]
fn great_rivers_have_no_gaps() {
    let Some(m) = load() else { return };
    let pairs = [
        ("Rhine near Cologne", (50.80, 6.48), (50.80, 7.21)),
        ("Rhine near Karlsruhe", (49.00, 8.05), (48.95, 8.65)),
        ("Danube south of Budapest", (47.19, 18.41), (46.91, 19.69)),
        ("Vistula south of Warsaw", (51.40, 21.15), (51.25, 22.57)),
        ("Volga at Saratov", (51.50, 45.50), (51.50, 47.00)),
        ("Tigris north of Baghdad", (33.30, 44.07), (33.75, 44.64)),
        ("Euphrates at Fallujah", (33.20, 43.40), (33.40, 44.05)),
        ("Nile at Asyut", (27.20, 30.70), (27.20, 31.60)),
        ("Mississippi at Memphis", (35.84, -90.70), (35.61, -88.81)),
        ("Yangtze at Wuhan", (31.00, 114.00), (30.10, 114.40)),
    ];
    let open = CellCosts::default();
    let closed = CellCosts { river_extra_m: 5_000_000, ..CellCosts::default() };
    let mut s = PathScratch::new(m.cell_count() as usize);
    for (name, a, b) in pairs {
        let (ca, cb) = (cell_at(&m, a.0, a.1), cell_at(&m, b.0, b.1));
        let direct = m.cell_distance_m(ca, cb);
        let easy = m.find_cell_path_bridged(ca, cb, &open, &mut s, &|_, _| true).expect("a route").cost;
        let hard = m.find_cell_path_bridged(ca, cb, &closed, &mut s, &|_, _| false).expect("a route").cost;
        eprintln!("{name}: {} km apart, {} km by road and bridge, {} km with no crossing allowed", direct / 1000, easy / 1000, hard / 1000);
        // going round the source is allowed and for some rivers not very far (the Vistula);
        // a gap would make the closed route about as cheap as the open one
        assert!(hard >= 4 * easy, "{name}: the river can be passed without crossing it ({hard} against {easy})");
    }
}

/// Rivers run along sides: a river edge is flagged the same from both hexagons, a bridge is a
/// river side that a road or railway passes, and the scenario seeds every one of them.
#[test]
fn bridges_are_river_sides_with_a_road_or_railway() {
    let Some(m) = load() else { return };
    let bridges = m.bridge_edges();
    assert!(bridges.len() > 2_000, "bridges: {}", bridges.len());
    for &(a, b, _) in bridges.iter().take(5_000) {
        assert!(m.river_between(CellId(a), CellId(b)) && m.river_between(CellId(b), CellId(a)));
    }
    let w = proving_ground::scenario_on_map(1, m.clone());
    assert_eq!(w.combat().bridges().len(), bridges.len());
}

/// M2 done test: a unit ordered across a great river takes the bridge; with the bridge
/// destroyed it goes round or fords, and arrives later.
#[test]
fn a_unit_crosses_by_the_bridge_and_is_slowed_when_it_is_blown() {
    let Some(m) = load() else { return };
    let rules = proving_ground::rules(false);
    // the Rhine bridge nearest Cologne
    let cologne = place_from_micro_degrees(50_940_000, 6_960_000);
    let &(a, b, _) = m.bridge_edges().iter().min_by_key(|&&(a, _, _)| (ground_m(m.cell_place(CellId(a)).unwrap(), cologne), a)).unwrap();
    assert!(ground_m(m.cell_place(CellId(a)).unwrap(), cologne) < 40_000, "no bridge near Cologne");

    // returns the tick on which the unit stood on the far bank, and the cells it passed
    let run = |blow: bool| -> (u64, Vec<u32>) {
        let mut w: World = proving_ground::scenario_on_map(1, m.clone());
        let owner = CountryId(m.cell_state[a as usize]);
        let unit: UnitId = w.view(owner).own_units()[0].0;
        w.unit_ext_mut(unit).unwrap().cell = Some(CellId(a));
        if blow {
            let order = Command::DemolishBridge { issuer: owner, unit, a: CellId(a), b: CellId(b) };
            let report = w.tick(&[order], &rules);
            assert!(report.rejected.is_empty(), "{:?}", report.rejected);
            assert_eq!(w.combat().bridges().iter().find(|x| (x.a, x.b) == (a.min(b), a.max(b))).unwrap().condition, 0);
        }
        let start = w.clock().tick();
        let report = w.tick(&[Command::MoveToCell { issuer: owner, unit, to: CellId(b) }], &rules);
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
        let mut passed = vec![a];
        for _ in 0..2_000 {
            let here = w.unit_ext(unit).unwrap().cell.unwrap().0;
            if *passed.last().unwrap() != here {
                passed.push(here);
            }
            if here == b {
                return (w.clock().tick() - start, passed);
            }
            w.tick(&[], &rules);
        }
        panic!("the unit never arrived; it passed {passed:?}");
    };
    let (quick, over) = run(false);
    let (slow, round) = run(true);
    eprintln!("over the bridge: {quick} ticks {over:?}; bridge blown: {slow} ticks {round:?}");
    assert_eq!(over, vec![a, b], "with the bridge standing the unit goes straight over");
    assert!(slow > quick, "a blown bridge must cost time ({slow} against {quick} ticks)");
}

/// Units are set up by latitude and longitude: the 1st Armoured Division stands at its towns.
#[test]
fn the_division_is_placed_by_latitude_and_longitude() {
    let Some(m) = load() else { return };
    let w = proving_ground::scenario_on_map(1, m.clone());
    let brd = CountryId(m.state_tags.iter().position(|t| t == "BRD").unwrap() as u16);
    let view = w.view(brd);
    let hq = view.own_units().into_iter().map(|(id, _)| id).find(|&id| view.own_unit_command(id).is_some_and(|c| c.0 == "1. Panzerdivision")).expect("the division exists");
    let cell = view.own_unit_place(hq).unwrap().0.unwrap();
    let hannover = place_from_micro_degrees(52_375_900, 9_732_000);
    assert!(ground_m(m.cell_place(cell).unwrap(), hannover) < 5_000);
    let brigades = view.own_units().into_iter().filter(|&(id, _)| view.own_unit_command(id).is_some_and(|c| c.1 == Some(hq))).count();
    assert_eq!(brigades, 4, "three brigades and the artillery regiment report to the division");
}

/// Roads and railways are links with levels: a supply route follows the strongest chain.
#[test]
fn supply_routes_follow_the_road_and_rail_links() {
    use meridian_core::map::LinkKind;
    let Some(m) = load() else { return };
    assert_eq!(m.c_road.len(), m.c_dst.len(), "the baked map has a road level for every cell edge");
    let mut s = PathScratch::new(m.cell_count() as usize);
    let as_built = |_: u32, _: u32, built: u8| built;
    let (hamburg, munich) = (cell_at(&m, 53.55, 10.0), cell_at(&m, 48.14, 11.58));
    let direct = m.cell_distance_m(hamburg, munich);
    for kind in [LinkKind::Road, LinkKind::Rail] {
        let route = m.find_link_route(hamburg, munich, kind, &as_built, &mut s).unwrap_or_else(|| panic!("no {kind:?} route from Hamburg to Munich"));
        eprintln!("Hamburg to Munich by {kind:?}: level {}, {} km ({} km in a straight line), {} hexagons", route.level, route.length_m / 1000, direct / 1000, route.cells.len());
        // (the level is that of the weakest link, and the links at a city's own hexagon are
        // often the weakest: Natural Earth marks many city approaches as plain roads)
        assert!(route.level >= 2, "a real route joins the two cities");
        assert!(route.length_m >= direct && route.length_m < direct * 2);
        // every step of the route really is a link of at least the route's level
        for w in route.cells.windows(2) {
            assert!(m.link_level(CellId(w[0]), CellId(w[1]), kind) >= route.level);
        }
        // cutting a link in the middle forces another way, never a better one
        let (ca, cb) = (route.cells[route.cells.len() / 2], route.cells[route.cells.len() / 2 + 1]);
        let cut = |a: u32, b: u32, built: u8| if (a.min(b), a.max(b)) == (ca.min(cb), ca.max(cb)) { 0 } else { built };
        if let Some(other) = m.find_link_route(hamburg, munich, kind, &cut, &mut s) {
            assert!(other.level < route.level || other.length_m >= route.length_m);
        }
    }
    // no road crosses the sea
    assert!(m.find_link_route(cell_at(&m, 40.1, 9.0), cell_at(&m, 41.9, 12.5), LinkKind::Road, &as_built, &mut s).is_none(), "Sardinia is an island");
}
