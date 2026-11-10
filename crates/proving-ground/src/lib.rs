//! THROWAWAY test ruleset. Exists only to give the engine something to hash.
//! meridian-core must never depend on this crate. Delete when real gameplay starts.

use meridian_core::{CellCosts, CellId, CountryId, MarchProfile, ProvinceId, Rate, Ruleset, Schedule, ScheduleBuilder, Signals, System, Unit, UnitId, World};

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
    fn march_profile(&self, _: &World, _: UnitId) -> Option<MarchProfile> {
        Some(march_profile())
    }
}

/// Unit type codes of the proving ground (`UnitExt::kind`). Placeholders for trying the map and
/// the counters out; the real unit types will come from the rules data.
pub mod unit_kind {
    pub const INFANTRY: u16 = 1;
    pub const ARMOUR: u16 = 2;
    pub const MECHANISED: u16 = 3;
    pub const MOTORISED: u16 = 4;
    pub const ARTILLERY: u16 = 5;
    pub const SP_ARTILLERY: u16 = 6;
    pub const RECON: u16 = 7;
    pub const ARMOURED_RECON: u16 = 8;
    pub const ENGINEER: u16 = 9;
    pub const AIR_DEFENCE: u16 = 10;
    pub const ANTI_TANK: u16 = 11;
    pub const AIRBORNE: u16 = 12;
    pub const MOUNTAIN: u16 = 13;
    pub const SUPPLY: u16 = 14;
    pub const HEADQUARTERS: u16 = 15;
    pub const COUNT: u16 = 15;
}
/// Unit size codes (`UnitExt::echelon`).
pub mod echelon {
    pub const BATTALION: u8 = 1;
    pub const REGIMENT: u8 = 2;
    pub const BRIGADE: u8 = 3;
    pub const DIVISION: u8 = 4;
    pub const CORPS: u8 = 5;
}

/// The test formation every country gets on the map: a corps headquarters, two divisions and
/// a spread of brigades, regiments and battalions of every type, so each counter can be seen.
pub const TEST_FORMATION: [(u16, u8); 15] = [
    (unit_kind::HEADQUARTERS, echelon::CORPS),
    (unit_kind::ARMOUR, echelon::DIVISION),
    (unit_kind::MECHANISED, echelon::DIVISION),
    (unit_kind::INFANTRY, echelon::BRIGADE),
    (unit_kind::MOTORISED, echelon::BRIGADE),
    (unit_kind::AIRBORNE, echelon::BRIGADE),
    (unit_kind::MOUNTAIN, echelon::BRIGADE),
    (unit_kind::ARTILLERY, echelon::REGIMENT),
    (unit_kind::SP_ARTILLERY, echelon::REGIMENT),
    (unit_kind::AIR_DEFENCE, echelon::REGIMENT),
    (unit_kind::ARMOURED_RECON, echelon::REGIMENT),
    (unit_kind::RECON, echelon::BATTALION),
    (unit_kind::ENGINEER, echelon::BATTALION),
    (unit_kind::ANTI_TANK, echelon::BATTALION),
    (unit_kind::SUPPLY, echelon::BATTALION),
];

/// Placeholder marching numbers so units can be moved about on the map: every unit covers 12 km
/// of open ground an hour, fording a river costs as much as 30 km, a bridge in at least half
/// condition costs nothing. Real values per unit type belong to the rules data.
pub fn march_profile() -> MarchProfile {
    MarchProfile { costs: CellCosts { river_extra_m: 30_000, bridge_extra_m: 0, ..CellCosts::default() }, min_bridge_condition: 500, m_per_tick: 12_000 }
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
/// provinces, and each country that owns land gets the test formation (`TEST_FORMATION`) on its
/// first province, one unit to a cell while the province has cells to spare.
pub fn scenario_on_map(seed: u64, map: std::sync::Arc<meridian_core::Map>) -> World {
    let mut first: Vec<Option<u32>> = vec![None; map.state_count() as usize];
    for (p, &st) in map.province_state.iter().enumerate() {
        first[st as usize].get_or_insert(p as u32);
    }
    let mut w = World::from_map(seed, map);
    // the permanent bridges of 1990 (none on a map baked before roads and railways were marked on cell edges)
    w.seed_bridges();
    for (c, p) in first.iter().enumerate() {
        if let Some(p) = p {
            // units stand in a cell and march cell by cell: spread the formation over the province
            let cells: Vec<u32> = w.map().map(|m| m.cells_of(ProvinceId(*p)).to_vec()).unwrap_or_default();
            for (k, &(kind, size)) in TEST_FORMATION.iter().enumerate() {
                let id = w.add_unit_for_setup(Unit { owner: CountryId(c as u16), location: ProvinceId(*p) });
                if let Some(e) = w.unit_ext_mut(id) {
                    e.cell = if cells.is_empty() { None } else { Some(CellId(cells[k % cells.len()])) };
                    e.kind = kind;
                    e.echelon = size;
                }
            }
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
/// The proving-ground schedule together with the rule answers a schedule cannot give (how
/// units march). This is what a client or a headless run on the map should use.
pub struct Rules(pub Schedule);

impl Ruleset for Rules {
    fn run(&self, world: &mut World, signals: Signals) {
        self.0.run(world, signals);
    }
    fn on_event(&self, world: &mut World, event: &meridian_core::Event) {
        self.0.on_event(world, event);
    }
    fn event_set(&self) -> Option<&meridian_core::EventSet> {
        self.0.event_set()
    }
    fn march_profile(&self, _: &World, _: UnitId) -> Option<MarchProfile> {
        Some(march_profile())
    }
}

pub fn rules(reversed: bool) -> Rules {
    Rules(schedule(reversed))
}

pub fn schedule(reversed: bool) -> Schedule {
    let a = System { name: "daily_counter", rate: Rate::Daily, after: &[], run: daily_counter };
    let b = System { name: "monthly_event", rate: Rate::Monthly, after: &["daily_counter"], run: monthly_event };
    let builder = ScheduleBuilder::new();
    let builder = if reversed { builder.system(b).system(a) } else { builder.system(a).system(b) };
    builder.build().expect("proving-ground schedule is valid")
}
