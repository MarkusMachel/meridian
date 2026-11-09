//! Phase 1 done tests.
use meridian_core::{Command, CountryId, ProvinceId, UnitId};
use proving_ground::{scenario, ProvingGround};

fn run(seed: u64, ticks: u64) -> u64 {
    let mut w = scenario(seed, 64);
    for t in 0..ticks {
        let cmds = if t.is_multiple_of(100) {
            vec![Command::MoveUnit {
                issuer: CountryId(0),
                unit: UnitId(0),
                to: ProvinceId((t / 100 % 64) as u32),
            }]
        } else {
            vec![]
        };
        w.tick(&cmds, &ProvingGround);
    }
    w.state_hash()
}

#[test]
fn same_seed_same_commands_same_hash_after_one_year() {
    let a = run(1, 8760);
    assert_eq!(a, run(1, 8760));
    assert_eq!(a, run(1, 8760));
}

#[test]
fn different_seeds_differ() {
    assert_ne!(run(1, 8760), run(2, 8760));
}

#[test]
fn rejected_command_leaves_state_unchanged() {
    let mut a = scenario(1, 64);
    let mut b = scenario(1, 64);
    let bad = Command::MoveUnit { issuer: CountryId(0), unit: UnitId(99), to: ProvinceId(0) };
    let r = a.tick(&[bad], &ProvingGround);
    b.tick(&[], &ProvingGround);
    assert_eq!(r.rejected.len(), 1);
    assert_eq!(a.state_hash(), b.state_hash());
}

/// Golden hash: pins the result so any accidental change (or a platform
/// difference when run on Linux) is caught. Update deliberately, with a
/// decision-log entry, if the simulation is changed on purpose.
#[test]
fn golden_hash() {
    assert_eq!(run(1, 8760), GOLDEN);
}
const GOLDEN: u64 = 16117180310011935637;

#[test]
fn registration_order_does_not_change_the_hash() {
    let run = |rs: &dyn meridian_core::Ruleset| {
        let mut w = proving_ground::scenario(7, 64);
        for _ in 0..8760 {
            w.tick(&[], rs);
        }
        w.state_hash()
    };
    let legacy = run(&proving_ground::ProvingGround);
    assert_eq!(run(&proving_ground::schedule(false)), legacy);
    assert_eq!(run(&proving_ground::schedule(true)), legacy);
}
