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
    // Budgets from docs/budgets.md (about 3x the M4 Pro measurements). MERIDIAN_BUDGET_SCALE
    // loosens them on slower machines, e.g. 3 for emulated Linux.
    let scale: f64 = std::env::var("MERIDIAN_BUDGET_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let budgets = [
        ("load ms", load_ms, 120.0),
        ("strategic mean us", node_stats.0, 700.0),
        ("strategic p99 us", node_stats.2, 3_000.0),
        ("local cell mean us", cell_stats.0, 200.0),
        ("local cell p99 us", cell_stats.2, 1_000.0),
        ("long cell mean us", long_stats.0, 10_000.0),
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
