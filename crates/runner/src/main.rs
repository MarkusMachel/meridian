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
