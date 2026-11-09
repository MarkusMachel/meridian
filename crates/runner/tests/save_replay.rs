//! Phase 5 done tests: save/load, replay, divergence finder, version handling.

use meridian_core::{
    Command, CountryId, Faction, PopGroup, ProvinceId, Rng, Unit, UnitId, World, ZocCache, Projection,
};
use meridian_data::replay::{first_divergence, hash_trace, play, read_replay, write_replay, Recorder};
use meridian_data::{load_world, save_world, ErrorKind};
use proving_ground::{scenario, ProvingGround};
use std::sync::Arc;

const YEAR: u64 = 8760;

/// Same scripted commands as the runner, so runs exercise the command path.
fn commands_for(tick: u64) -> Vec<Command> {
    if !tick.is_multiple_of(100) {
        return Vec::new();
    }
    let step = (tick / 100) as u32;
    vec![
        Command::MoveUnit { issuer: CountryId(0), unit: UnitId(0), to: ProvinceId(step % 64) },
        Command::SetProvinceController { issuer: CountryId(1), province: ProvinceId((step * 7) % 64), controller: CountryId((step % 3) as u16) },
    ]
}

fn run(world: &mut World, from: u64, to: u64) {
    for t in from..to {
        world.tick(&commands_for(t), &ProvingGround);
    }
}

#[test]
fn saving_at_random_ticks_and_loading_matches_the_uninterrupted_run() {
    let mut straight = scenario(5, 64);
    run(&mut straight, 0, YEAR);
    let want = straight.state_hash();

    let mut rng = Rng::from_seed(77);
    for _ in 0..10 {
        let at = rng.below(YEAR);
        let mut w = scenario(5, 64);
        run(&mut w, 0, at);
        let bytes = save_world(&w);
        let mut loaded = load_world("test.mrsv", &bytes, None).unwrap();
        assert_eq!(loaded.state_hash(), w.state_hash(), "load changed the state at tick {at}");
        assert_eq!(loaded, w);
        run(&mut loaded, at, YEAR);
        assert_eq!(loaded.state_hash(), want, "run continued from a save at tick {at} diverged");
    }
}

#[test]
fn a_recorded_replay_reproduces_the_final_hash_and_survives_a_file_round_trip() {
    let mut w = scenario(9, 64);
    let mut rec = Recorder::new(&w, "proving-ground:64", 240);
    for t in 0..2000 {
        let cmds = commands_for(t);
        // pretend the second command came from an AI
        let (player, ai) = cmds.split_at(cmds.len().min(1));
        rec.step(&mut w, &ProvingGround, player, ai, &[]);
    }
    let replay = rec.finish(&w);
    assert!(replay.commands.iter().any(|e| e.source == meridian_data::replay::Source::Ai), "AI commands are logged");
    let bytes = write_replay(&replay);
    let back = read_replay("r.mrrp", &bytes).unwrap();
    assert_eq!(back, replay);
    let mut fresh = scenario(back.seed, 64);
    assert_eq!(play(&mut fresh, &ProvingGround, &back), Ok(replay.final_hash));
    // corrupt files are refused
    let mut broken = bytes.clone();
    *broken.last_mut().unwrap() ^= 1;
    assert!(read_replay("r.mrrp", &broken).is_err());
}

#[test]
fn replay_playing_names_the_first_bad_checkpoint() {
    let mut w = scenario(9, 64);
    let mut rec = Recorder::new(&w, "x", 100);
    for t in 0..1000 {
        rec.step(&mut w, &ProvingGround, &commands_for(t), &[], &[]);
    }
    let replay = rec.finish(&w);
    // play against a world with a different seed: the very first checkpoint already disagrees
    let mut wrong = scenario(10, 64);
    let d = play(&mut wrong, &ProvingGround, &replay).unwrap_err();
    assert_eq!(d.tick, 0);
    // tamper with a command: the divergence shows up at the next checkpoint after it
    let mut tampered = replay.clone();
    let i = tampered.commands.iter().position(|e| e.tick == 300).unwrap();
    tampered.commands[i].command = Command::SetProvinceController { issuer: CountryId(1), province: ProvinceId(5), controller: CountryId(2) };
    let mut fresh = scenario(9, 64);
    let d = play(&mut fresh, &ProvingGround, &tampered).unwrap_err();
    assert_eq!(d.tick, 400, "checkpoints are every 100 ticks, the change was applied on tick 300");
}

#[test]
fn the_divergence_finder_names_the_first_differing_tick() {
    let log = |change_at: Option<u64>| {
        let mut entries = Vec::new();
        for t in 0..500 {
            for command in commands_for(t) {
                entries.push(meridian_data::replay::Entry { tick: t, source: meridian_data::replay::Source::Player, command });
            }
        }
        if let Some(at) = change_at {
            entries.retain(|e| e.tick != at);
        }
        entries
    };
    let mut a = scenario(3, 64);
    let mut b = scenario(3, 64);
    let ta = hash_trace(&mut a, &ProvingGround, &log(None), 500);
    let tb = hash_trace(&mut b, &ProvingGround, &log(Some(300)), 500);
    // the command stamped tick 300 is applied during tick 300, so the hash AFTER it (index 301) differs first
    assert_eq!(first_divergence(&ta, &tb), Some(301));
    assert_eq!(first_divergence(&ta, &ta), None);
    assert_eq!(first_divergence(&ta[..10], &ta), Some(10));
}

#[test]
fn old_new_and_damaged_saves_fail_with_clear_messages() {
    let bytes = save_world(&scenario(1, 8));
    let with_version = |v: u32| {
        let mut b = bytes.clone();
        b[4..8].copy_from_slice(&v.to_le_bytes());
        b
    };
    let e = load_world("s", &with_version(0), None).unwrap_err();
    assert_eq!(e.kind, ErrorKind::WrongVersion);
    assert!(e.to_string().contains("older than the oldest"), "{e}");
    let e = load_world("s", &with_version(99), None).unwrap_err();
    assert!(e.to_string().contains("newer build"), "{e}");
    let mut flipped = bytes.clone();
    *flipped.last_mut().unwrap() ^= 0xff;
    assert!(load_world("s", &flipped, None).unwrap_err().to_string().contains("checksum"));
    assert!(load_world("s", &bytes[..bytes.len() - 3], None).unwrap_err().to_string().contains("truncated"));
    assert!(load_world("s", b"not a save at all, no", None).unwrap_err().to_string().contains("magic"));
    assert!(load_world("s", &bytes, None).is_ok());
}

/// A world with every kind of dynamic state populated survives a save and load unchanged.
#[test]
fn every_table_round_trips() {
    use meridian_core::{Agency, Bridge, Contact, Destination, Decision, Hull, Officer, Pilot, Vote};
    let mut w = scenario(2, 16);
    w.econ_mut().configure(3, 4);
    w.econ_mut().stocks_of_mut(1)[2] = 55;
    w.econ_mut().pop.add_for_setup(PopGroup { cell: 3, culture: 4, religion: 1, heads: 900, support: [1, 2, 3, 4] });
    w.econ_mut().finance_mut(CountryId(2)).treasury = -7;
    w.econ_mut().add_equipment(CountryId(1), 9, 12);
    w.polity_mut().set_faction(Faction { country: CountryId(0), kind: 1, support: 300, cohesion: 800 });
    w.polity_mut().set_war_level(CountryId(2), CountryId(0), 3);
    w.polity_mut().set_unit_loyalty(1, 250);
    let a = w.diplomacy_mut().add_alliance(0, vec![0, 2]);
    let call = w.diplomacy_mut().call_alliance(a, CountryId(0), CountryId(1)).unwrap();
    w.diplomacy_mut().decide(call, CountryId(2), true);
    w.diplomacy_mut().set_council(vec![(0, true), (1, false)]);
    let _ = (Decision::Join, Vote::Yes);
    w.tech_mut().configure(3);
    w.tech_mut().set_level(CountryId(1), 2, 40);
    w.tech_mut().unlock(CountryId(1), 7);
    w.intel_mut().set_agency(Agency { country: CountryId(0), flags: 1, capability: [1, 2, 3, 4, 5, 6] });
    w.intel_mut().set_contact(Contact { observer: CountryId(0), target_kind: 0, target: 3, last_seen: 12, knowledge: 2, cell: 8, strength_est: 70 });
    w.event_state_mut().set_flag(CountryId(1), 5, true);
    w.event_state_mut().set_chain_stage(2, 3);
    w.combat_mut().add_pressure(meridian_core::CellId(4), -9);
    w.combat_mut().set_bridge(Bridge { a: 1, b: 2, condition: 500, laid: true, owner: CountryId(1) });
    w.combat_mut().add_prisoners(CountryId(0), CountryId(1), 33);
    w.forces_mut().add_officer(Officer { country: CountryId(0), birth_days_before_start: 15000, rank: 3, experience: 40, status: 0, assignment: Some(UnitId(0)), traits: Default::default() });
    w.forces_mut().add_pilot(Pilot { country: CountryId(1), wing: None, birth_days_before_start: 9000, rank: 1, experience: 2, kills: 1, status: 0 });
    w.forces_mut().add_hull(Hull { model: 4, owner: CountryId(2), location: 11, parent: None, condition: 900, fuel: 100, status: 0 });
    w.unit_ext_mut(UnitId(0)).unwrap().traits.add(3);
    w.start_transit(UnitId(1), Destination::Province(ProvinceId(9)), 500);
    w.raise_event(meridian_core::Event { kind: 3, target: 1, source: 2, amount: -5 });
    w.set_treaty_threshold(650);
    let before = w.state_hash();
    let loaded = load_world("t", &save_world(&w), None).unwrap();
    assert_eq!(loaded, w);
    assert_eq!(loaded.state_hash(), before);
}

fn map_path() -> std::path::PathBuf {
    std::env::var_os("MERIDIAN_MAP").map(Into::into).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/baked/meridian-map.bin"))
}

/// On the real map: per-cell state is saved as differences, derived caches are rebuilt on load.
#[test]
fn a_map_world_saves_cell_differences_and_caches_rebuild_identically() {
    let path = map_path();
    if !path.exists() {
        assert!(std::env::var_os("MERIDIAN_REQUIRE_MAP").is_none(), "map file {} is required but missing", path.display());
        eprintln!("skipping: no baked map at {}", path.display());
        return;
    }
    let map = Arc::new(meridian_data::load_map_file(&path).unwrap());
    let mut w = World::from_map(3, map.clone());
    let fresh_size = save_world(&w).len();
    // some play: open a province, change cells, place units with cell positions
    w.open_province(ProvinceId(100));
    let cell_ids: Vec<u32> = w.map().unwrap().cells_of(ProvinceId(100)).to_vec();
    {
        let c = w.cells_mut().unwrap();
        for &cell in &cell_ids[..5] {
            c.damage[cell as usize] = 77;
            c.controller[cell as usize] = 9;
        }
        c.hunger[cell_ids[0] as usize] = 3;
    }
    for (k, owner) in [(0usize, 0u16), (3, 9)] {
        let id = w.add_unit_for_setup(Unit { owner: CountryId(owner), location: ProvinceId(100) });
        w.unit_ext_mut(id).unwrap().cell = Some(meridian_core::CellId(cell_ids[k]));
    }
    let bytes = save_world(&w);
    eprintln!("map-world save sizes: fresh {fresh_size} bytes, after play {} bytes", bytes.len());
    assert!(bytes.len() < fresh_size + 4000, "cell state must be saved as differences ({} vs {fresh_size} bytes)", bytes.len());
    assert!(bytes.len() < 200_000, "a mostly quiet world saves small: {} bytes", bytes.len());

    let loaded = load_world("m", &bytes, Some(map.clone())).unwrap();
    assert_eq!(loaded.state_hash(), w.state_hash());
    assert_eq!(loaded.cells(), w.cells());
    let proj = |_: UnitId| Some(Projection { radius_m: 40_000, strength: 100 });
    let z1 = ZocCache::rebuild(&w, proj, |_| 1000);
    let z2 = ZocCache::rebuild(&loaded, proj, |_| 1000);
    assert!(!z1.is_empty());
    assert_eq!(z1, z2, "derived cache must rebuild to identical values after a load");
    // wrong or missing map
    assert!(load_world("m", &bytes, None).unwrap_err().to_string().contains("needs map"));
}
