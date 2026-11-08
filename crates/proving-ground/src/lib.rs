//! THROWAWAY test ruleset. Exists only to give the engine something to hash.
//! meridian-core must never depend on this crate. Delete when real gameplay starts.

use meridian_core::{CountryId, ProvinceId, Ruleset, Signals, Unit, World};

pub struct ProvingGround;

impl Ruleset for ProvingGround {
    fn run(&self, world: &mut World, signals: Signals) {
        // Daily: every province counter grows by 1 (stands in for an economy).
        if signals.new_day {
            for v in world.values_mut() {
                *v += 1;
            }
        }
        // Monthly: one random event bumps a random province (stands in for events).
        if signals.new_month {
            let n = world.province_count() as u64;
            let p = world.rng_mut().below(n) as usize;
            world.values_mut()[p] += 100;
        }
    }
}

/// 3 countries, `provinces` provinces split evenly, 2 units per country.
pub fn scenario(seed: u64, provinces: u32) -> World {
    let mut w = World::new(seed, 3, provinces);
    for p in 0..provinces {
        w.set_controller(ProvinceId(p), CountryId((p % 3) as u16));
    }
    for c in 0..3u16 {
        for k in 0..2u32 {
            w.add_unit_for_setup(Unit {
                owner: CountryId(c),
                location: ProvinceId((c as u32 + 3 * k) % provinces),
            });
        }
    }
    w
}
