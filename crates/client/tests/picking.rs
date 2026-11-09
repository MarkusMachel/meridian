//! Picking against the real render file: clicking a cell's centre finds that cell, and the cell
//! found lies in the province the province picker names. Skipped when the baked files are missing
//! (unless MERIDIAN_REQUIRE_MAP=1).

use client::{Bounds, CellPicker, Picker};
use std::path::PathBuf;

#[test]
fn cell_and_province_picking_agree_on_the_real_map() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/baked");
    let (rp, mp) = (dir.join("meridian-render.bin"), dir.join("meridian-map.bin"));
    if !rp.exists() || !mp.exists() {
        assert!(std::env::var_os("MERIDIAN_REQUIRE_MAP").is_none(), "baked files required");
        return;
    }
    let r = meridian_data::load_render_file(&rp).unwrap();
    let m = meridian_data::load_map_file(&mp).unwrap();
    let (x0, y0, x1, y1) = r.bounds();
    let bounds = Bounds::around(x0, y0, x1, y1);
    let cells = CellPicker::new(&r, bounds);
    let provinces = Picker::new(r.levels.last().unwrap(), bounds);

    let n = r.cell_mx.len();
    let mut exact = 0;
    let mut agree = 0;
    let mut tested = 0;
    for i in (0..n).step_by(97) {
        let (x, y) = (r.cell_mx[i] as f64, r.cell_my[i] as f64);
        tested += 1;
        if cells.cell_at(&r, x, y) == Some(i as u32) {
            exact += 1;
        }
        // the province under a cell's centre should be the cell's province (the province mesh is
        // simplified, so allow the odd cell on a border to differ)
        if let (Some(c), Some(p)) = (cells.cell_at(&r, x, y), provinces.province_at(x, y)) {
            if m.cell_province[c as usize] == p {
                agree += 1;
            }
        }
    }
    // a centre can fall outside a very concave cell; nearly all must hit their own cell
    assert!(exact * 100 >= tested * 98, "only {exact} of {tested} cell centres picked their own cell");
    assert!(agree * 100 >= tested * 97, "only {agree} of {tested} cell and province picks agree");
    // open sea, far from land (mid Pacific), is no cell and no province
    assert_eq!(cells.cell_at(&r, -15_000_000.0, 0.0), None);
    assert_eq!(provinces.province_at(-15_000_000.0, 0.0), None);
}
