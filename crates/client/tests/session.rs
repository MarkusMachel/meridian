//! Phase 7 done test: a client session and a headless run of the same command log end with the
//! same hash. Uses the proving-ground ruleset on the real map when the baked map exists.

use client::session::replay_headless;
use client::{Session, Speed};
use meridian_core::{Command, CountryId, ProvinceId, Ruleset, UnitId, World};
use std::sync::Arc;

fn world_and_rules() -> (World, Box<dyn Ruleset + Send>) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/baked/meridian-map.bin");
    if path.exists() {
        let map = Arc::new(meridian_data::load_map_file(&path).unwrap());
        (proving_ground::scenario_on_map(7, map), Box::new(proving_ground::schedule(false)))
    } else {
        assert!(std::env::var_os("MERIDIAN_REQUIRE_MAP").is_none(), "baked map required");
        (proving_ground::scenario(7, 60), Box::new(proving_ground::schedule(false)))
    }
}

/// Run frames until `ticks` have finished.
fn run_until(s: &mut Session, ticks: u64) {
    while s.refresh().tick < ticks {
        s.frame();
        std::thread::yield_now();
    }
}

#[test]
fn client_session_matches_a_headless_run_of_the_same_log() {
    let (w, rules) = world_and_rules();
    let (w2, _) = world_and_rules();
    let (w3, _) = world_and_rules();
    let mut s = Session::new(w, rules, CountryId(0));
    // every speed, then single steps, with orders given along the way
    let plan: [(Speed, u64); 6] = [(Speed::Run(1), 10), (Speed::Run(2), 30), (Speed::Run(3), 60), (Speed::Run(4), 100), (Speed::Run(5), 200), (Speed::Run(5), 400)];
    for (i, (speed, upto)) in plan.iter().enumerate() {
        s.set_speed(*speed);
        run_until(&mut s, *upto);
        s.set_speed(Speed::Paused);
        s.issue(Command::SetProvinceController { issuer: CountryId(0), province: ProvinceId(i as u32), controller: CountryId(0) });
        s.issue(Command::MoveUnit { issuer: CountryId(0), unit: UnitId(0), to: ProvinceId((i * 3) as u32 % 60) });
        for _ in 0..3 {
            s.step();
            let before = s.refresh().tick;
            while s.refresh().tick == before {
                s.frame();
                std::thread::yield_now();
            }
        }
    }
    let total = s.refresh().tick;
    assert!(total >= 400);
    assert!(!s.log().is_empty());
    let (done, log) = s.finish();
    let headless = replay_headless(w2, &proving_ground::schedule(false), &log, total);
    assert_eq!(done.state_hash(), headless.state_hash());
    // the orders really did something: without them the run ends elsewhere
    let without = replay_headless(w3, &proving_ground::schedule(false), &[], total);
    assert_ne!(done.state_hash(), without.state_hash());
}
