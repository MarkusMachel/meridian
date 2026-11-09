#![allow(clippy::chunks_exact_to_as_chunks)] // chunks_exact reads more plainly than as_chunks
//! Render-file checks against the real baked world (client data, not used by the engine).
//! Files come from crates/map-builder (not committed). If the render file is missing the test
//! prints a notice and passes, unless MERIDIAN_REQUIRE_MAP=1 (set by scripts/check.sh).
use meridian_data::{load_map_file, load_render_file, LineKind};
use std::path::PathBuf;
use std::time::Instant;

fn baked(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/baked").join(name)
}

#[test]
fn render_file_fits_the_baked_map() {
    let (rp, mp) = (baked("meridian-render.bin"), baked("meridian-map.bin"));
    if !rp.exists() || !mp.exists() {
        assert!(std::env::var_os("MERIDIAN_REQUIRE_MAP").is_none(), "{} is required but missing", rp.display());
        eprintln!("NOTE: {} not found, skipping render-file test", rp.display());
        return;
    }
    let t = Instant::now();
    let r = load_render_file(&rp).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("render file loaded in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    let m = load_map_file(&mp).unwrap_or_else(|e| panic!("{}", e[0]));
    let provinces = m.province_count() as usize;

    assert_eq!(r.levels.len(), 3);
    assert!(r.levels.windows(2).all(|w| w[0].tolerance_m > w[1].tolerance_m), "coarsest level must come first");
    for (k, l) in r.levels.iter().enumerate() {
        // every vertex belongs to a real province, and every province has at least one triangle
        let mut has = vec![false; provinces];
        for t in l.tri.chunks_exact(3) {
            let p = l.vprov[t[0] as usize] as usize;
            assert!(p < provinces, "level {k}: province {p} does not exist");
            has[p] = true;
        }
        let missing = has.iter().filter(|&&h| !h).count();
        assert_eq!(missing, 0, "level {k}: {missing} provinces have no fill");
        // all three line kinds present, and every point lies in Web Mercator range (x within +-20,038 km)
        for kind in [LineKind::ProvinceBorder, LineKind::StateBorder, LineKind::Coast] {
            assert!((0..l.line_count()).any(|i| l.kind(i) == kind), "level {k}: no {kind:?} lines");
        }
        assert!(l.line_x.iter().chain(&l.vx).all(|x| x.abs() <= 20_037_509), "level {k}: x out of range");
        assert!(l.line_y.iter().chain(&l.vy).all(|y| (-9_000_000..19_000_000).contains(y)), "level {k}: y out of range");
    }
    assert!(r.cell_seg.len() / 4 > 1_000_000);
    assert_eq!(r.cell_mx.len(), m.cell_count() as usize);
    assert_eq!(r.province_mx.len(), provinces);
}
