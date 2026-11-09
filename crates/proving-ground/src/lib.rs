//! THROWAWAY test ruleset. Exists only to give the engine something to hash.
//! meridian-core must never depend on this crate. Delete when real gameplay starts.

use meridian_core::{CountryId, ProvinceId, Rate, Ruleset, Schedule, ScheduleBuilder, Signals, System, Unit, World};

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

/// The proving-ground rules on the real map: every state is a country that controls its own
/// provinces, and each country that owns land gets one unit on its first province.
pub fn scenario_on_map(seed: u64, map: std::sync::Arc<meridian_core::Map>) -> World {
    let mut first: Vec<Option<u32>> = vec![None; map.state_count() as usize];
    for (p, &st) in map.province_state.iter().enumerate() {
        first[st as usize].get_or_insert(p as u32);
    }
    let mut w = World::from_map(seed, map);
    for (c, p) in first.iter().enumerate() {
        if let Some(p) = p {
            w.add_unit_for_setup(Unit { owner: CountryId(c as u16), location: ProvinceId(*p) });
        }
    }
    w
}

fn daily_counter(world: &mut World, _: Signals) {
    for v in world.values_mut() {
        *v += 1;
    }
}

fn monthly_event(world: &mut World, _: Signals) {
    let n = world.province_count() as u64;
    let p = world.rng_mut().below(n) as usize;
    world.values_mut()[p] += 100;
}

/// The same rules as `ProvingGround`, as registered systems. `reversed` registers them
/// in the opposite source order, to show that registration order does not matter.
pub fn schedule(reversed: bool) -> Schedule {
    let a = System { name: "daily_counter", rate: Rate::Daily, after: &[], run: daily_counter };
    let b = System { name: "monthly_event", rate: Rate::Monthly, after: &["daily_counter"], run: monthly_event };
    let builder = ScheduleBuilder::new();
    let builder = if reversed { builder.system(b).system(a) } else { builder.system(a).system(b) };
    builder.build().expect("proving-ground schedule is valid")
}
