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
    assert!(!s.log().is_empty());
    let (done, log) = s.finish();
    let total = done.clock().tick();
    assert!(total >= 400);
    let headless = replay_headless(w2, &proving_ground::schedule(false), &log, total);
    assert_eq!(done.state_hash(), headless.state_hash());
    // the orders really did something: without them the run ends elsewhere
    let without = replay_headless(w3, &proving_ground::schedule(false), &[], total);
    assert_ne!(done.state_hash(), without.state_hash());
}

#[test]
fn saving_and_loading_in_the_client_continues_exactly() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/baked/meridian-map.bin");
    let map = path.exists().then(|| Arc::new(meridian_data::load_map_file(&path).unwrap()));
    let make = || match &map {
        Some(m) => proving_ground::scenario_on_map(7, m.clone()),
        None => proving_ground::scenario(7, 60),
    };
    // uninterrupted: 600 ticks
    let mut straight = Session::new(make(), Box::new(proving_ground::schedule(false)), CountryId(0));
    straight.set_speed(Speed::Run(5));
    straight.set_stop_at(Some(600));
    run_until(&mut straight, 600);
    let (straight_world, _) = straight.finish();

    // 250 ticks, save, load into a NEW session, run to 600
    let mut a = Session::new(make(), Box::new(proving_ground::schedule(false)), CountryId(0));
    a.set_speed(Speed::Run(5));
    a.set_stop_at(Some(250));
    run_until(&mut a, 250);
    a.set_speed(Speed::Paused);
    let bytes = a.save_bytes().expect("save");
    drop(a);
    let loaded = meridian_data::load_world("quicksave", &bytes, map.clone()).unwrap_or_else(|e| panic!("{e}"));
    let mut b = Session::new(loaded, Box::new(proving_ground::schedule(false)), CountryId(0));
    b.set_speed(Speed::Run(5));
    b.set_stop_at(Some(600));
    run_until(&mut b, 600);
    let (b_world, _) = b.finish();
    assert_eq!(straight_world.clock().tick(), 600);
    assert_eq!(b_world.clock().tick(), 600);
    assert_eq!(straight_world.state_hash(), b_world.state_hash());
}
