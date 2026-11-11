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

/// One unit of a named formation: what it is called, its type and size, where it stands (latitude and
/// longitude in millionths of a degree, as whole numbers so every machine picks the same
/// hexagon), and which entry of the same list it reports to.
pub struct FormationUnit {
    pub name: &'static str,
    pub kind: u16,
    pub echelon: u8,
    pub lat: i32,
    pub lon: i32,
    pub reports_to: Option<usize>,
}

/// The West German 1st Armoured Division of 1990 under I Corps, as a first try at a real order
/// of battle: the corps and division headquarters, the division's three brigades and its
/// artillery regiment, each at its garrison town (Münster, Hannover, Hildesheim, Braunschweig,
/// Nienburg). Written from general knowledge of the Bundeswehr's Army Structure 4; names and
/// garrisons are not checked against a source, and battalions are left out.
pub const PANZERDIVISION_1: [FormationUnit; 6] = [
    FormationUnit { name: "I. Korps", kind: unit_kind::HEADQUARTERS, echelon: echelon::CORPS, lat: 51_960_700, lon: 7_626_100, reports_to: None },
    FormationUnit { name: "1. Panzerdivision", kind: unit_kind::HEADQUARTERS, echelon: echelon::DIVISION, lat: 52_375_900, lon: 9_732_000, reports_to: Some(0) },
    FormationUnit { name: "Panzergrenadierbrigade 1", kind: unit_kind::MECHANISED, echelon: echelon::BRIGADE, lat: 52_150_800, lon: 9_951_100, reports_to: Some(1) },
    FormationUnit { name: "Panzerbrigade 2", kind: unit_kind::ARMOUR, echelon: echelon::BRIGADE, lat: 52_268_900, lon: 10_526_800, reports_to: Some(1) },
    FormationUnit { name: "Panzerbrigade 3", kind: unit_kind::ARMOUR, echelon: echelon::BRIGADE, lat: 52_646_100, lon: 9_220_800, reports_to: Some(1) },
    FormationUnit { name: "Artillerieregiment 1", kind: unit_kind::SP_ARTILLERY, echelon: echelon::REGIMENT, lat: 52_430_000, lon: 9_600_000, reports_to: Some(1) },
];

/// A formation is only placed when the map has a cell within this distance of its first unit
/// (so a small test map that does not reach Germany simply gets none).
const FORMATION_REACH_M: u32 = 60_000;

/// Put a named formation on the map: each unit in the cell nearest its place, owned by whoever
/// holds the cell the first unit stands in, with the chain of command set (`UnitExt::parent`).
/// Returns the new units' IDs, or nothing if the map has no cell near the formation.
pub fn add_formation(w: &mut World, formation: &[FormationUnit]) -> Vec<UnitId> {
    // only a globe map has places; a flat test map gets no formation
    let place = |w: &World, u: &FormationUnit| w.map().and_then(|m| m.nearest_cell_to_place(meridian_core::sphere::place_from_micro_degrees(u.lat, u.lon)));
    let Some(first) = formation.first() else { return Vec::new() };
    let Some((home, away)) = place(w, first) else { return Vec::new() };
    if away > FORMATION_REACH_M {
        return Vec::new();
    }
    let owner = CountryId(w.cells().map_or(0, |c| c.controller(home.0 as usize)));
    let mut ids: Vec<UnitId> = Vec::new();
    for u in formation {
        let Some((cell, _)) = place(w, u) else { continue };
        let province = w.map().map_or(ProvinceId(0), |m| ProvinceId(m.cell_province[cell.0 as usize]));
        let id = w.add_unit_for_setup(Unit { owner, location: province });
        let parent = u.reports_to.and_then(|k| ids.get(k).copied());
        if let Some(e) = w.unit_ext_mut(id) {
            e.cell = Some(cell);
            e.kind = u.kind;
            e.echelon = u.echelon;
            e.name = u.name.to_string();
            e.parent = parent;
        }
        ids.push(id);
    }
    ids
}

/// Placeholder: what a road or a railway of each level carries, in tonnes a day (index = level,
/// 0 = no link). Invented round numbers so the client has something to show; the real values
/// belong to the rules data and the supply system that will use them.
pub const ROAD_TONNES_PER_DAY: [u32; 6] = [0, 300, 800, 2_000, 5_000, 12_000];
pub const RAIL_TONNES_PER_DAY: [u32; 6] = [0, 1_500, 4_000, 9_000, 18_000, 30_000];

pub fn link_capacity(kind: meridian_core::map::LinkKind, level: u8) -> u32 {
    let table = match kind {
        meridian_core::map::LinkKind::Road => &ROAD_TONNES_PER_DAY,
        meridian_core::map::LinkKind::Rail => &RAIL_TONNES_PER_DAY,
    };
    table[(level as usize).min(5)]
}

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
/// first province, one unit to a cell while the province has cells to spare. West Germany also
/// gets a real formation with its chain of command (`PANZERDIVISION_1`).
pub fn scenario_on_map(seed: u64, map: std::sync::Arc<meridian_core::Map>) -> World {
    let mut first: Vec<Option<u32>> = vec![None; map.state_count() as usize];
    for (p, &st) in map.province_state.iter().enumerate() {
        first[st as usize].get_or_insert(p as u32);
    }
    let mut w = World::from_map(seed, map);
    // the permanent bridges of 1990 (none on a map baked before roads and railways were marked on cell edges)
    w.seed_bridges();
    add_formation(&mut w, &PANZERDIVISION_1);
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
