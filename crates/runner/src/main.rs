//! Headless runner: `runner [seed] [ticks]` prints the state hash.

use meridian_core::{Command, CountryId, ProvinceId, UnitId};
use proving_ground::{scenario, ProvingGround};

/// A fixed command script so runs exercise the command path.
fn commands_for(tick: u64, provinces: u32) -> Vec<Command> {
    if !tick.is_multiple_of(100) {
        return Vec::new();
    }
    let step = (tick / 100) as u32;
    vec![
        Command::MoveUnit {
            issuer: CountryId(0),
            unit: UnitId(0),
            to: ProvinceId(step % provinces),
        },
        Command::SetProvinceController {
            issuer: CountryId(1),
            province: ProvinceId((step * 7) % provinces),
            controller: CountryId((step % 3) as u16),
        },
        // Deliberately invalid: unit 99 does not exist.
        Command::MoveUnit { issuer: CountryId(2), unit: UnitId(99), to: ProvinceId(0) },
    ]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("validate") => std::process::exit(cmd_validate(&args[1..])),
        Some("bake") => std::process::exit(cmd_bake(&args[1..])),
        Some("run") => std::process::exit(cmd_run(&args[1..])),
        Some("map-validate") => std::process::exit(cmd_map_validate(&args[1..])),
        Some("map-bench") => std::process::exit(cmd_map_bench(&args[1..])),
        Some("record") => std::process::exit(cmd_record(&args[1..])),
        Some("replay") => std::process::exit(cmd_replay(&args[1..])),
        Some("diff") => std::process::exit(cmd_diff(&args[1..])),
        Some("save") => std::process::exit(cmd_save(&args[1..])),
        Some("load") => std::process::exit(cmd_load(&args[1..])),
        Some("probe") => std::process::exit(cmd_probe(&args[1..])),
        _ => {}
    }
    let mut args = args.into_iter();
    let seed: u64 = args.next().map_or(1, |s| s.parse().expect("seed must be an integer"));
    let ticks: u64 = args.next().map_or(8760, |s| s.parse().expect("ticks must be an integer"));
    let provinces = 64;

    let mut world = scenario(seed, provinces);
    let mut rejected = 0usize;
    for t in 0..ticks {
        let report = world.tick(&commands_for(t, provinces), &ProvingGround);
        rejected += report.rejected.len();
    }
    let d = world.clock().date();
    println!(
        "seed={seed} ticks={ticks} date={:04}-{:02}-{:02}T{:02} rejected={rejected} hash={:016x}",
        d.year, d.month, d.day, d.hour,
        world.state_hash()
    );
}

/// `runner validate <file.ron>...` : report every error in every file.
fn cmd_validate(files: &[String]) -> i32 {
    if files.is_empty() {
        eprintln!("usage: runner validate <scenario.ron>...");
        return 2;
    }
    let mut bad = 0;
    for f in files {
        match meridian_data::validate_scenario_file(std::path::Path::new(f)) {
            Ok(_) => println!("{f}: ok"),
            Err(errors) => {
                bad += 1;
                for e in &errors {
                    eprintln!("{e}");
                }
                eprintln!("{f}: {} error(s)", errors.len());
            }
        }
    }
    (bad > 0) as i32
}

/// `runner bake <in.ron> <out.bin>` : validate, then write the baked file.
fn cmd_bake(args: &[String]) -> i32 {
    let [input, output] = args else {
        eprintln!("usage: runner bake <scenario.ron> <out.bin>");
        return 2;
    };
    match meridian_data::validate_scenario_file(std::path::Path::new(input)) {
        Ok(s) => {
            let bytes = meridian_data::bake::bake_scenario(&s);
            std::fs::write(output, &bytes).expect("cannot write output file");
            println!("{output}: {} bytes", bytes.len());
            0
        }
        Err(errors) => {
            for e in &errors {
                eprintln!("{e}");
            }
            1
        }
    }
}

/// `runner run <scenario.ron> [ticks]` : load a scenario and tick it.
fn cmd_run(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("usage: runner run <scenario.ron> [ticks]");
        return 2;
    };
    let ticks: u64 = args.get(1).map_or(8760, |s| s.parse().expect("ticks must be an integer"));
    let mut world = match meridian_data::load_scenario_file(std::path::Path::new(path)) {
        Ok(w) => w,
        Err(errors) => {
            for e in &errors {
                eprintln!("{e}");
            }
            return 1;
        }
    };
    for _ in 0..ticks {
        world.tick(&[], &ProvingGround);
    }
    println!("scenario={path} ticks={ticks} hash={:016x}", world.state_hash());
    0
}

fn load_map_or_report(path: &str) -> Option<meridian_core::Map> {
    match meridian_data::load_map_file(std::path::Path::new(path)) {
        Ok(m) => Some(m),
        Err(errors) => {
            for e in errors.iter().take(20) {
                eprintln!("{e}");
            }
            eprintln!("{path}: {} error(s)", errors.len());
            None
        }
    }
}

/// `runner map-validate <map.bin>` : structure checks (on load) then whole-world checks.
fn cmd_map_validate(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("usage: runner map-validate <map.bin>");
        return 2;
    };
    let Some(m) = load_map_or_report(path) else { return 1 };
    let problems = m.validate_world();
    for p in problems.iter().take(50) {
        eprintln!("{path}: {p}");
    }
    println!(
        "{path}: {} provinces, {} sea zones, {} cells, {} sites, {} states: {} problem(s)",
        m.province_count(), m.zone_count(), m.cell_count(), m.site_count(), m.state_count(), problems.len()
    );
    (!problems.is_empty()) as i32
}

/// `runner map-bench <map.bin>` : load time and path-query timings (run in release mode).
/// Exits 1 if a budget from docs/budgets.md is exceeded.
fn cmd_map_bench(args: &[String]) -> i32 {
    use meridian_core::{CellCosts, CellId, NodeCosts, PathScratch, Rng};
    use std::time::Instant;
    let Some(path) = args.first() else {
        eprintln!("usage: runner map-bench <map.bin>");
        return 2;
    };
    // load: median of 5
    let mut loads = Vec::new();
    let mut map = None;
    for _ in 0..5 {
        let t = Instant::now();
        map = load_map_or_report(path);
        loads.push(t.elapsed().as_secs_f64() * 1000.0);
        if map.is_none() {
            return 1;
        }
    }
    loads.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let load_ms = loads[2];
    let m = map.unwrap();
    let t = Instant::now();
    let problems = m.validate_world();
    let validate_ms = t.elapsed().as_secs_f64() * 1000.0;

    let mut rng = Rng::from_seed(2026);
    let mut fingerprint = meridian_core::hash::StateHasher::default();
    // 10,000 random strategic-graph queries between any two nodes
    let mut ns = PathScratch::new(m.node_count() as usize);
    let nc = NodeCosts::default();
    let mut times = Vec::new();
    let (mut found, mut exp_total) = (0u32, 0u64);
    for _ in 0..10_000 {
        let a = rng.below(m.node_count() as u64) as u32;
        let b = rng.below(m.node_count() as u64) as u32;
        let t = Instant::now();
        let p = m.find_node_path(a, b, &nc, &mut ns);
        times.push(t.elapsed().as_secs_f64() * 1e6);
        if let Some(p) = p {
            found += 1;
            exp_total += p.expanded as u64;
            fingerprint.write_u32(p.cost);
            fingerprint.write_u32(p.nodes.len() as u32);
            fingerprint.write_u32(p.expanded);
        }
    }
    let node_stats = stats(&mut times);
    let node_found = found;
    let node_exp = exp_total as f64 / found.max(1) as f64;

    // 10,000 local tactical queries: target within 400 km on the same landmass
    let mut cs = PathScratch::new(m.cell_count() as usize);
    let cc = CellCosts::default();
    let mut times = Vec::new();
    let (mut found, mut exp_total, mut asked) = (0u32, 0u64, 0u32);
    while asked < 10_000 {
        let a = CellId(rng.below(m.cell_count() as u64) as u32);
        let near = m.cells_within(a, 400_000);
        let pool: Vec<CellId> = near.into_iter().filter(|c| m.cell_landmass[c.index()] == m.cell_landmass[a.index()]).collect();
        if pool.len() < 2 {
            continue;
        }
        let b = pool[rng.below(pool.len() as u64) as usize];
        asked += 1;
        let t = Instant::now();
        let p = m.find_cell_path(a, b, &cc, &mut cs);
        times.push(t.elapsed().as_secs_f64() * 1e6);
        if let Some(p) = p {
            found += 1;
            exp_total += p.expanded as u64;
            fingerprint.write_u32(p.cost);
            fingerprint.write_u32(p.nodes.len() as u32);
            fingerprint.write_u32(*p.nodes.get(p.nodes.len() / 2).unwrap_or(&0));
        }
    }
    let cell_stats = stats(&mut times);
    let cell_found = found;
    let cell_exp = exp_total as f64 / found.max(1) as f64;

    // 100 long tactical queries across one landmass (Eurasia-scale): first cell of two far states
    let mut times = Vec::new();
    let mut asked = 0;
    while asked < 100 {
        let a = CellId(rng.below(m.cell_count() as u64) as u32);
        let near = m.cells_within(a, 3_000_000);
        let pool: Vec<CellId> = near.into_iter().filter(|c| m.cell_landmass[c.index()] == m.cell_landmass[a.index()] || m.cell_state[c.index()] == m.cell_state[a.index()]).collect();
        if pool.len() < 2 {
            continue;
        }
        let b = pool[rng.below(pool.len() as u64) as usize];
        asked += 1;
        let t = Instant::now();
        let _ = m.find_cell_path(a, b, &cc, &mut cs);
        times.push(t.elapsed().as_secs_f64() * 1e6);
    }
    let long_stats = stats(&mut times);

    println!("map: {} provinces, {} zones, {} cells; validate_world {:.0} ms ({} problems)", m.province_count(), m.zone_count(), m.cell_count(), validate_ms, problems.len());
    println!("load (median of 5): {load_ms:.1} ms");
    println!("10,000 strategic queries ({node_found} found, {node_exp:.0} expanded avg): mean {:.1} us, p50 {:.1}, p99 {:.1}, max {:.1}", node_stats.0, node_stats.1, node_stats.2, node_stats.3);
    println!("10,000 local cell queries ({cell_found} found, {cell_exp:.0} expanded avg): mean {:.1} us, p50 {:.1}, p99 {:.1}, max {:.1}", cell_stats.0, cell_stats.1, cell_stats.2, cell_stats.3);
    println!("100 long cell queries (up to 3,000 km): mean {:.0} us, p50 {:.0}, p99 {:.0}, max {:.0}", long_stats.0, long_stats.1, long_stats.2, long_stats.3);
    println!("path fingerprint (must match on every platform): {:016x}", fingerprint.finish());
    // Budgets from docs/budgets.md. For the hexagon world they are provisional: about 3x what
    // a 2-core Linux container measured on 2026-11-10, to be set again from a run on the
    // M4 Pro. MERIDIAN_BUDGET_SCALE loosens them on slower machines, e.g. 3 for emulated Linux.
    let scale: f64 = std::env::var("MERIDIAN_BUDGET_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let budgets = [
        ("load ms", load_ms, 4_500.0),
        ("strategic mean us", node_stats.0, 700.0),
        ("strategic p99 us", node_stats.2, 3_000.0),
        ("local cell mean us", cell_stats.0, 1_100.0),
        ("local cell p99 us", cell_stats.2, 6_000.0),
        ("long cell mean us", long_stats.0, 70_000.0),
    ];
    let mut over = false;
    for (name, got, limit) in budgets {
        let limit = limit * scale;
        if got > limit {
            eprintln!("BUDGET EXCEEDED: {name} {got:.1} > {limit}");
            over = true;
        }
    }
    (over || !problems.is_empty()) as i32
}

/// (mean, p50, p99, max)
fn stats(t: &mut [f64]) -> (f64, f64, f64, f64) {
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = t.len();
    (t.iter().sum::<f64>() / n as f64, t[n / 2], t[n * 99 / 100], t[n - 1])
}

/// `runner probe [threads]` : Phase 4A performance probe on a stub world (release mode).
/// Stand-ins only: 600,000 cells with five one-byte-ish fields, 600,000 population
/// group records of 24 bytes, 190 countries. Not the real state; it tells us whether
/// the planned shapes are affordable before we build them.
fn cmd_probe(args: &[String]) -> i32 {
    use meridian_core::hash::StateHasher;
    use meridian_core::parallel::for_each_chunk_mut;
    use meridian_core::{ActiveSet, Rng};
    use std::time::Instant;
    const COUNTRIES: usize = 190;
    let threads: usize = args.first().map_or(8, |s| s.parse().expect("threads must be an integer"));
    // the number of cells: today's map by default; pass 3686556 for the level-6 hexagon world
    let cells: usize = args.get(1).map_or(599_261, |s| s.parse().expect("cells must be an integer"));
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;

    let mut rng = Rng::from_seed(1);
    let mut controller: Vec<u16> = (0..cells).map(|_| rng.below(COUNTRIES as u64) as u16).collect();
    let mut hunger = vec![0u8; cells];
    let food: Vec<u8> = (0..cells).map(|_| rng.below(256) as u8).collect();
    let mut ground = vec![0u8; cells];
    let mut damage = vec![0u8; cells];
    // Population group record, 24 bytes: cell, head count, culture, religion, 4 support shares, pad.
    #[derive(Clone, Copy)]
    struct Group { cell: u32, heads: u32, culture: u16, religion: u8, support: [u8; 4], _pad: [u8; 9] }
    let mut groups: Vec<Group> = (0..cells)
        .map(|c| Group { cell: c as u32, heads: 1000 + rng.below(9000) as u32, culture: rng.below(300) as u16, religion: rng.below(20) as u8, support: [64; 4], _pad: [0; 9] })
        .collect();
    println!("stub world: {cells} cells, {} groups ({} bytes each), {COUNTRIES} countries, {threads} threads", groups.len(), std::mem::size_of::<Group>());

    // 1. State hash over everything.
    let t = Instant::now();
    let mut h = StateHasher::default();
    for i in 0..cells {
        h.write_u16(controller[i]);
        h.write_u8(hunger[i]);
        h.write_u8(food[i]);
        h.write_u8(ground[i]);
        h.write_u8(damage[i]);
    }
    for g in &groups {
        h.write_u32(g.cell);
        h.write_u32(g.heads);
        h.write_u16(g.culture);
        h.write_u8(g.religion);
        h.write_bytes(&g.support);
    }
    let hash_ms = ms(t);
    println!("state hash over all stub state: {hash_ms:.1} ms (hash {:016x})", h.finish());

    // 2. Daily full pass over cells and groups: single thread vs N threads, results must match.
    let daily = |threads: usize, hunger: &mut [u8], ground: &mut [u8], groups: &mut [Group], food: &[u8]| {
        for_each_chunk_mut(hunger, threads, |first, part| {
            for (k, v) in part.iter_mut().enumerate() {
                *v = v.saturating_add(food[first + k] & 1).saturating_sub(food[first + k] >> 7);
            }
        });
        for_each_chunk_mut(ground, threads, |first, part| {
            for (k, v) in part.iter_mut().enumerate() {
                *v = (*v).wrapping_mul(3).wrapping_add(((first + k) & 7) as u8);
            }
        });
        for_each_chunk_mut(groups, threads, |_, part| {
            for g in part.iter_mut() {
                g.heads = g.heads + (g.heads >> 12) - (g.heads >> 13);
                g.support[(g.culture & 3) as usize] = g.support[(g.culture & 3) as usize].wrapping_add(1);
            }
        });
    };
    let (mut h1, mut g1, mut p1) = (hunger.clone(), ground.clone(), groups.clone());
    let t = Instant::now();
    for _ in 0..10 {
        daily(1, &mut h1, &mut g1, &mut p1, &food);
    }
    let single = ms(t) / 10.0;
    let t = Instant::now();
    for _ in 0..10 {
        daily(threads, &mut hunger, &mut ground, &mut groups, &food);
    }
    let multi = ms(t) / 10.0;
    let same = h1 == hunger && g1 == ground && p1.iter().zip(&groups).all(|(a, b)| a.heads == b.heads && a.support == b.support);
    println!("daily cell+group pass: 1 thread {single:.2} ms, {threads} threads {multi:.2} ms, identical results: {same}");

    // 3. Active set: 5% of cells busy, touched per tick (hourly systems).
    let mut active = ActiveSet::new();
    let ids: Vec<u32> = (0..cells as u32 / 20).map(|_| rng.below(cells as u64) as u32).collect();
    let t = Instant::now();
    active.extend(&ids);
    let build = ms(t);
    let t = Instant::now();
    for _ in 0..100 {
        for id in active.iter() {
            let i = id as usize;
            damage[i] = damage[i].saturating_add(1);
            controller[i] = controller[i].wrapping_add(0);
        }
    }
    let per_tick = ms(t) / 100.0;
    println!("active set of {} cells: build {build:.2} ms, hourly pass {per_tick:.3} ms", active.len());

    // 4. 190 countries, each doing a monthly-style pass of 2,000 integer ops (stub for AI/economy).
    let t = Instant::now();
    let mut acc = 0u64;
    for c in 0..COUNTRIES as u64 {
        let mut x = c + 1;
        for _ in 0..2_000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            acc ^= x >> 33;
        }
    }
    println!("190 stub country passes: {:.3} ms (checksum {acc:x})", ms(t));

    // 5. Memory estimate.
    let bytes = cells * (2 + 4) + groups.len() * std::mem::size_of::<Group>();
    println!("stub state size: {:.1} MB", bytes as f64 / 1e6);
    (!same) as i32
}

const PG_SCENARIO: &str = "proving-ground:64";

fn read_file(path: &str) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("{path}: {e}");
            None
        }
    }
}

/// `runner record <seed> <ticks> <out.mrrp>` : run the proving ground, log every command, write a replay.
/// Every third command is logged as coming from the AI, to exercise the log.
fn cmd_record(args: &[String]) -> i32 {
    use meridian_data::replay::Recorder;
    let [seed, ticks, out] = args else {
        eprintln!("usage: runner record <seed> <ticks> <out.mrrp>");
        return 2;
    };
    let (seed, ticks): (u64, u64) = (seed.parse().expect("seed must be an integer"), ticks.parse().expect("ticks must be an integer"));
    let mut world = scenario(seed, 64);
    let mut rec = Recorder::new(&world, PG_SCENARIO, 240);
    for t in 0..ticks {
        let cmds = commands_for(t, 64);
        let (player, ai) = cmds.split_at(cmds.len().min(1));
        rec.step(&mut world, &ProvingGround, player, ai, &[]);
    }
    let replay = rec.finish(&world);
    let bytes = meridian_data::replay::write_replay(&replay);
    std::fs::write(out, &bytes).expect("cannot write output file");
    println!("{out}: {} commands, {} checkpoints, {} bytes, final hash {:016x}", replay.commands.len(), replay.checkpoints.len(), bytes.len(), replay.final_hash);
    0
}

/// `runner replay <file.mrrp>` : re-run a recorded run and compare every checkpoint.
fn cmd_replay(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("usage: runner replay <file.mrrp>");
        return 2;
    };
    let Some(bytes) = read_file(path) else { return 1 };
    let replay = match meridian_data::replay::read_replay(path, &bytes) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if replay.scenario != PG_SCENARIO {
        eprintln!("{path}: this runner can only replay scenario '{PG_SCENARIO}', not '{}'", replay.scenario);
        return 1;
    }
    let mut world = scenario(replay.seed, 64);
    match meridian_data::replay::play(&mut world, &ProvingGround, &replay) {
        Ok(h) => {
            println!("{path}: replayed {} ticks, final hash {h:016x} matches", replay.ticks);
            0
        }
        Err(d) => {
            eprintln!("{path}: {d}");
            1
        }
    }
}

/// `runner diff <a.mrrp> <b.mrrp>` : replay both from their starts and name the first tick whose hashes differ.
fn cmd_diff(args: &[String]) -> i32 {
    let [a, b] = args else {
        eprintln!("usage: runner diff <a.mrrp> <b.mrrp>");
        return 2;
    };
    let mut traces = Vec::new();
    for path in [a, b] {
        let Some(bytes) = read_file(path) else { return 1 };
        let r = match meridian_data::replay::read_replay(path, &bytes) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        };
        let mut w = scenario(r.seed, 64);
        traces.push(meridian_data::replay::hash_trace(&mut w, &ProvingGround, &r.commands, r.ticks));
    }
    match meridian_data::replay::first_divergence(&traces[0], &traces[1]) {
        None => {
            println!("identical for {} ticks", traces[0].len() - 1);
            0
        }
        Some(i) => {
            if i == 0 {
                println!("the runs differ before tick 0 (different starting state)");
            } else {
                println!("first difference is in the state after tick {} (the state hash at tick {i} is the first that differs)", i - 1);
            }
            1
        }
    }
}

/// `runner save <seed> <ticks> <out.mrsv>` : run the proving ground and write a save.
fn cmd_save(args: &[String]) -> i32 {
    let [seed, ticks, out] = args else {
        eprintln!("usage: runner save <seed> <ticks> <out.mrsv>");
        return 2;
    };
    let (seed, ticks): (u64, u64) = (seed.parse().expect("seed must be an integer"), ticks.parse().expect("ticks must be an integer"));
    let mut world = scenario(seed, 64);
    for t in 0..ticks {
        world.tick(&commands_for(t, 64), &ProvingGround);
    }
    let bytes = meridian_data::save_world(&world);
    std::fs::write(out, &bytes).expect("cannot write output file");
    println!("{out}: tick {ticks}, {} bytes, hash {:016x}", bytes.len(), world.state_hash());
    0
}

/// `runner load <file.mrsv> <more ticks>` : load a save (checking its hash) and keep running.
fn cmd_load(args: &[String]) -> i32 {
    let [path, more] = args else {
        eprintln!("usage: runner load <file.mrsv> <more ticks>");
        return 2;
    };
    let more: u64 = more.parse().expect("ticks must be an integer");
    let Some(bytes) = read_file(path) else { return 1 };
    let mut world = match meridian_data::load_world(path, &bytes, None) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let start = world.clock().tick();
    for t in start..start + more {
        world.tick(&commands_for(t, 64), &ProvingGround);
    }
    println!("{path}: loaded at tick {start}, ran {more} more, hash={:016x}", world.state_hash());
    0
}
