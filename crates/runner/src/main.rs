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
    let mut args = std::env::args().skip(1);
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
