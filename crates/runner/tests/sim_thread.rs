//! Phase 6 done test on the real map: reading while the sim runs on another thread never holds
//! the sim up beyond `MAX_QUERY_NS`.

use meridian_core::{CountryId, Ruleset, Signals, SimHandle, World, MAX_QUERY_NS};
use std::sync::atomic::Ordering;
use std::sync::Arc;

struct NoRules;
impl Ruleset for NoRules {
    fn run(&self, _: &mut World, _: Signals) {}
}

#[test]
fn queries_on_the_full_map_stay_inside_the_stall_limit() {
    let path = std::env::var_os("MERIDIAN_MAP").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/baked/meridian-map.bin"));
    if !path.exists() {
        assert!(std::env::var_os("MERIDIAN_REQUIRE_MAP").is_none(), "map file {} is required but missing", path.display());
        eprintln!("skipping: no baked map at {}", path.display());
        return;
    }
    let map = Arc::new(meridian_data::load_map_file(&path).unwrap());
    let h = SimHandle::spawn(World::from_map(1, map), Box::new(NoRules));
    h.run(20_000);
    for i in 0..300u16 {
        // the heaviest ordinary read: every province a country holds
        let n = h.query(move |w| w.view(CountryId(i % 170)).own_provinces().len()).unwrap();
        assert!(n < 15_240);
    }
    while h.stats().ticks.load(Ordering::Relaxed) < 20_000 {
        std::thread::yield_now();
    }
    let slowest = h.stats().max_query_ns.load(Ordering::Relaxed);
    eprintln!("slowest of 300 own_provinces queries on the full map: {slowest} ns (limit {MAX_QUERY_NS})");
    assert!(slowest < MAX_QUERY_NS);
}
