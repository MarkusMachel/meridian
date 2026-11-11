//! Map rewrite M3 on the real hexagon map: picking and the hexagon triangles. Skipped when no
//! baked map is present (set MERIDIAN_REQUIRE_MAP to make that a failure).

use client::globe::index::side;
use client::globe::{GlobeCamera, HexIndex, V3};
use meridian_core::map::cell_edge_flag;
use meridian_core::{CellId, Map};

fn load() -> Option<Map> {
    let path = std::env::var_os("MERIDIAN_MAP").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/baked/meridian-map.bin"));
    if !path.exists() {
        assert!(std::env::var_os("MERIDIAN_REQUIRE_MAP").is_none(), "map file {} is required but missing", path.display());
        eprintln!("skipping: no baked map at {}", path.display());
        return None;
    }
    let map = meridian_data::load_map_file(&path).unwrap();
    map.is_globe().then_some(map)
}

fn dir(map: &Map, cell: u32) -> V3 {
    map.cell_place(CellId(cell)).unwrap().map(|v| v as f64)
}

/// M3 done test: a hexagon's centre, put on the screen and picked again, is the same hexagon,
/// from the whole globe down to the closest zoom, at the equator, in the far north and across
/// the 180th meridian.
#[test]
fn a_hexagon_shown_on_screen_is_picked_again() {
    let Some(map) = load() else { return };
    let index = HexIndex::new(&map);
    // every cell is found by its own centre
    for c in (0..map.cell_count()).step_by(1777) {
        assert_eq!(index.cell_at(dir(&map, c)), Some(CellId(c)));
    }
    let mut checked = 0;
    for (lat, lon) in [(0.3, 32.6), (69.65, 18.96), (-17.8, 178.0), (64.7, 177.5), (52.37, 9.73), (-16.5, -179.9)] {
        for km in [40_000.0, 5_000.0, 800.0, 150.0, 30.0] {
            let camera = GlobeCamera::looking_at(2560.0, 1440.0, lat, lon, km);
            // land hexagons near the middle of the screen and toward its edges
            for (px, py) in [(1280.0, 720.0), (700.0, 400.0), (1900.0, 1100.0), (300.0, 1300.0)] {
                let Some(cell) = camera.screen_to_dir(px, py).and_then(|d| index.cell_at(d)) else { continue };
                let (x, y) = camera.dir_to_screen(dir(&map, cell.0)).expect("its centre is on the near side");
                let again = index.cell_at(camera.screen_to_dir(x, y).unwrap());
                assert_eq!(again, Some(cell), "at {lat},{lon}, {km} km across");
                checked += 1;
            }
        }
    }
    assert!(checked > 40, "only {checked} picks landed on land");
}

/// Each hexagon is a fan of triangles, one per side, and each side knows what runs along it:
/// the counts of river sides, borders and coast agree with the map's cell graph.
#[test]
fn hexagon_triangles_carry_the_right_sides() {
    let Some(map) = load() else { return };
    let index = HexIndex::new(&map);
    assert!(index.tiles.len() > 10_000);
    let (mut cells, mut rivers) = (0, 0);
    for tile in (0..index.tiles.len() as u32).step_by(97) {
        let (_, vertices) = index.tile_mesh(&map, tile);
        assert_eq!(vertices.len() % 3, 0);
        let mut at = 0;
        while at < vertices.len() {
            let cell = vertices[at].cell;
            let n = vertices[at..].iter().take_while(|v| v.cell == cell).count() / 3;
            assert!(n == 6 || n == 5, "a hexagon has six sides (a pentagon five), not {n}");
            let sides: Vec<u32> = (0..n).map(|i| vertices[at + 3 * i].side).collect();
            let count = |bit: u32| sides.iter().filter(|&&s| s & bit != 0).count();
            let edges: Vec<usize> = map.cell_edges(CellId(cell)).collect();
            let river = edges.iter().filter(|&&e| map.c_flags[e] & cell_edge_flag::RIVER != 0).count();
            let abroad = edges.iter().filter(|&&e| map.cell_state[map.c_dst[e] as usize] != map.cell_state[cell as usize]).count();
            assert_eq!(count(side::RIVER), river, "river sides of cell {cell}");
            assert_eq!(count(side::COUNTRY), abroad, "country borders of cell {cell}");
            assert_eq!(count(side::COAST), n - edges.len(), "coast sides of cell {cell}");
            // triangles are small and near their anchor (camera-relative positions stay exact)
            assert!(vertices[at..at + 3 * n].iter().all(|v| v.pos.iter().all(|p| p.abs() < 130_000.0)));
            cells += 1;
            rivers += river;
            at += 3 * n;
        }
    }
    assert!(cells > 20_000 && rivers > 500, "{cells} cells, {rivers} river sides");
}

/// The tiles in view are found, and the picture of the world for the far view agrees with picking.
#[test]
fn tiles_in_view_and_the_world_picture() {
    let Some(map) = load() else { return };
    let index = HexIndex::new(&map);
    let camera = GlobeCamera::looking_at(2560.0, 1440.0, 52.37, 9.73, 600.0);
    let near = index.tiles_near(client::globe::dir_of(camera.lat, camera.lon), camera.view_radius());
    assert!((10..200).contains(&near.len()), "{} tiles for a 600 km view", near.len());
    let (w, h) = (720, 360);
    let picture = index.world_picture(w, h);
    let land = picture.iter().filter(|&&c| c != u32::MAX).count() as f64 / picture.len() as f64;
    assert!((0.15..0.30).contains(&land), "land is {land:.2} of a longitude-latitude picture");
    // Hannover's pixel is West German
    let (col, row) = (((9.73 + 180.0) / 360.0 * w as f64) as usize, ((90.0 - 52.37) / 180.0 * h as f64) as usize);
    let cell = picture[row * w as usize + col];
    assert_eq!(map.state_tags[map.cell_state[cell as usize] as usize], "BRD");
}
