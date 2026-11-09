//! World state: parallel arrays indexed by ID, plus the clock and RNG.

use crate::cells::{CellChanges, CellState};
use crate::combat::CombatState;
use crate::clock::{Clock, Signals};
use crate::command::{Command, Rejection};
use crate::econ::{Economy, ProductionOrder, ProjectSpec, ProjectTemplate};
use crate::events::{EventSet, EventState};
use crate::forces::{Destination, Forces, UnitExt};
use crate::hash::StateHasher;
use crate::intel::Intel;
use crate::polity::{Diplomacy, Polity, MAX_WAR_LEVEL};
use crate::ids::{CountryId, ProvinceId, UnitId};
use crate::map::Map;
use std::sync::Arc;
use crate::rng::Rng;
use crate::tech::Tech;

/// Raw read and write access to world state. Public only with the `omniscient` feature
/// (for rulesets, tools and tests); otherwise crate-private, so code built without the
/// feature, such as the AI crate, can only look at the world through a `CountryView`.
macro_rules! gated {
    ($(#[$m:meta])* fn $($rest:tt)*) => {
        #[cfg(feature = "omniscient")]
        $(#[$m])*
        pub fn $($rest)*
        #[cfg(not(feature = "omniscient"))]
        #[allow(dead_code)]
        $(#[$m])*
        pub(crate) fn $($rest)*
    };
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Unit {
    pub owner: CountryId,
    pub location: ProvinceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum SiteKind {
    City = 0,
    Port = 1,
    Airfield = 2,
    Crossing = 3,
    Industry = 4,
    Base = 5,
}

/// A city, port, airfield, bridge, facility or base. Anchored to a province
/// for now; Phase 3 adds the cell anchor.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Site {
    pub kind: SiteKind,
    pub province: ProvinceId,
    pub owner: CountryId,
    /// 0 (destroyed) to 100 (intact).
    pub condition: u8,
    pub built_year: i16,
    pub destroyed_year: Option<i16>,
}

/// Rules run by the tick, in a fixed order. Implementations must keep all
/// state in the World (a ruleset holding its own state would be missed by the
/// state hash) and obey the determinism rules.
pub trait Ruleset {
    fn run(&self, world: &mut World, signals: Signals);

    /// Called for each queued event, after the systems have run, in the order
    /// (target, source, kind, then raise order). Events raised from here wait for
    /// the next tick. The default ignores events.
    fn on_event(&self, _world: &mut World, _event: &Event) {}

    /// Cost of building something of this kind; `None` means it cannot be built.
    /// Event definitions, if this ruleset has any (used to validate `ChooseEvent`).
    fn event_set(&self) -> Option<&EventSet> {
        None
    }

    // ---- hooks used to validate orders; the defaults refuse (no such capability) ----

    /// How far this unit can bombard, in metres; `None` means it cannot.
    fn bombard_range_m(&self, _world: &World, _unit: UnitId) -> Option<u32> {
        None
    }
    /// 0 means a cell cannot be landed on; higher is better.
    fn landing_suitability(&self, _world: &World, _cell: crate::ids::CellId) -> u8 {
        0
    }
    fn validate_launch(&self, _world: &World, _unit: UnitId, _missile: u16, _target: crate::ids::CellId, _count: u32) -> Result<(), &'static str> {
        Err("this unit cannot launch missiles")
    }
    fn validate_mission(&self, _world: &World, _unit: UnitId, _kind: u8, _target: crate::ids::CellId) -> Result<(), &'static str> {
        Err("this unit cannot fly missions")
    }
    /// Ticks a long move takes; `None` means there is no route.
    fn transit_ticks(&self, _world: &World, _unit: UnitId, _to: Destination) -> Option<u64> {
        None
    }
    /// Ticks a doctrine change takes.
    fn doctrine_transition_ticks(&self, _track: u8, _from: u16, _to: u16) -> u32 {
        720
    }
    fn can_build_bridge(&self, _world: &World, _unit: UnitId) -> bool {
        false
    }

    fn project_template(&self, _kind: u16) -> Option<ProjectTemplate> {
        None
    }
}

/// A deferred effect such as bombardment damage. `kind` is defined by the ruleset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub kind: u16,
    pub target: u32,
    pub source: u32,
    pub amount: i64,
}

/// Something the player should be told about (a city fell, a treaty was signed). `kind` and
/// the two numbers are defined by the ruleset; the client turns them into text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub country: CountryId,
    pub kind: u16,
    pub a: i64,
    pub b: i64,
}

/// Something that happened this tick that the client may want to draw (a hit, a strike, a
/// mission flown, a movement, a battle). Cosmetic only: never read back by the simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClientEvent {
    pub kind: u16,
    /// Cell or other place the event refers to (meaning set by `kind`).
    pub place: u32,
    pub other: u32,
    pub amount: i64,
}

/// Transient outputs of one tick. Not state: empty between ticks, never hashed or saved.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Outbox {
    messages: Vec<Message>,
    events: Vec<ClientEvent>,
}

/// What a tick did, for a UI that must not copy the whole world.
#[derive(Debug, Default)]
pub struct TickReport {
    /// The tick that just ran.
    pub tick: u64,
    /// Commands that were refused this tick, in submission order.
    pub rejected: Vec<(Command, Rejection)>,
    /// Cells whose per-cell state changed this tick, ascending.
    pub changed_cells: Vec<u32>,
    /// Draw-me events, in the order they were emitted.
    pub events: Vec<ClientEvent>,
    /// Things to tell the player, in the order they were emitted.
    pub messages: Vec<Message>,
}

/// Shared, read-only map attached to a world. Equality and hashing use the map's file
/// checksum and size, not its contents.
#[derive(Clone)]
pub struct MapRef(pub Arc<Map>);
impl std::fmt::Debug for MapRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Map(crc {:08x}, {} provinces, {} cells)", self.0.checksum, self.0.province_count(), self.0.cell_count())
    }
}
impl PartialEq for MapRef {
    fn eq(&self, o: &MapRef) -> bool {
        Arc::ptr_eq(&self.0, &o.0) || (self.0.checksum == o.0.checksum && self.0.cell_count() == o.0.cell_count() && self.0.province_count() == o.0.province_count())
    }
}
impl Eq for MapRef {}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct World {
    pub(crate) seed: u64,
    pub(crate) clock: Clock,
    pub(crate) rng: Rng,
    pub(crate) country_count: u16,
    /// Per province, indexed by ProvinceId.
    pub(crate) controller: Vec<CountryId>,
    /// Per province generic value slot; meaning is defined by the ruleset.
    pub(crate) value: Vec<i64>,
    /// Indexed by UnitId; None means the slot is empty.
    pub(crate) units: Vec<Option<Unit>>,
    /// Extra unit attributes, always the same length as `units`.
    pub(crate) unit_ext: Vec<UnitExt>,
    pub(crate) forces: Forces,
    pub(crate) combat: CombatState,
    pub(crate) econ: Economy,
    pub(crate) scripted: EventState,
    pub(crate) tech: Tech,
    pub(crate) intel: Intel,
    pub(crate) polity: Polity,
    pub(crate) diplomacy: Diplomacy,
    /// How many factions per treaty approval must pass, in thousandths (a world setting).
    pub(crate) treaty_threshold: u32,
    /// Indexed by SiteId order of creation.
    pub(crate) sites: Vec<Site>,
    /// Events waiting for the next event phase. Normally empty at the end of a tick.
    pub(crate) events: Vec<Event>,
    /// Present only on a world with a map.
    #[serde(skip)]
    pub(crate) outbox: Outbox,
    #[serde(skip)]
    pub(crate) cells: Option<CellState>,
    #[serde(skip)]
    pub(crate) map: Option<MapRef>,
}

impl World {
    pub fn new(seed: u64, country_count: u16, province_count: u32) -> World {
        assert!(country_count > 0, "need at least one country");
        World {
            seed,
            clock: Clock::default(),
            rng: Rng::from_seed(seed),
            country_count,
            controller: vec![CountryId(0); province_count as usize],
            value: vec![0; province_count as usize],
            units: Vec::new(),
            unit_ext: Vec::new(),
            forces: Forces::default(),
            combat: CombatState::default(),
            econ: Economy::default(),
            scripted: EventState::default(),
            tech: Tech::default(),
            intel: Intel::default(),
            polity: Polity::default(),
            diplomacy: Diplomacy::default(),
            treaty_threshold: 500,
            sites: Vec::new(),
            events: Vec::new(),
            outbox: Outbox::default(),
            cells: None,
            map: None,
        }
    }

    /// A world on a real map: one country per map state, every province controlled by
    /// its owning state. The map is shared and never changes.
    pub fn from_map(seed: u64, map: Arc<Map>) -> World {
        let mut w = World::new(seed, map.state_count() as u16, map.province_count());
        for (p, &st) in map.province_state.iter().enumerate() {
            w.controller[p] = CountryId(st);
        }
        w.cells = Some(Self::fresh_cells(&map));
        w.map = Some(MapRef(map));
        w
    }

    gated! {
        /// Per-cell dynamic state, if the world has a map.
        fn cells(&self) -> Option<&CellState> {
            self.cells.as_ref()
        }
    }
    gated! {
        fn cells_mut(&mut self) -> Option<&mut CellState> {
            self.cells.as_mut()
        }
    }

    gated! {
        /// Start simulating a province cell by cell. Every cell takes the province's
        /// controller. Returns false if the world has no map, the province does not exist
        /// or it is already open.
        fn open_province(&mut self, p: ProvinceId) -> bool {
            let (Some(map), Some(cells)) = (self.map.as_ref(), self.cells.as_mut()) else { return false };
            if p.0 >= map.0.province_count() || !cells.opened.insert(p.0) {
                return false;
            }
            let c = self.controller[p.index()].0;
            for &cell in map.0.cells_of(p) {
                cells.set_controller(cell as usize, c);
            }
            true
        }
    }

    gated! {
        /// Stop simulating a province cell by cell. The province goes to whoever controls
        /// most of its cells (ties to the lower country ID) and every cell follows it.
        /// Returns false if the province was not open.
        fn close_province(&mut self, p: ProvinceId) -> bool {
            let (Some(map), Some(cells)) = (self.map.as_ref(), self.cells.as_mut()) else { return false };
            if !cells.opened.remove(p.0) {
                return false;
            }
            let list = map.0.cells_of(p);
            if let Some(winner) = cells.majority_controller(list) {
                self.controller[p.index()] = winner;
            }
            let c = self.controller[p.index()].0;
            for &cell in list {
                cells.set_controller(cell as usize, c);
            }
            true
        }
    }

    /// The checksum of the attached map, if any (saves record it so they load on the right map).
    pub fn map_checksum(&self) -> Option<u32> {
        self.map.as_ref().map(|m| m.0.checksum)
    }

    /// Per-cell state as differences from a fresh world on the same map.
    pub fn cell_changes(&self) -> Option<CellChanges> {
        let (map, cells) = (self.map.as_ref()?, self.cells.as_ref()?);
        Some(cells.changes_from(&Self::fresh_cells(&map.0)))
    }

    fn fresh_cells(map: &Map) -> CellState {
        CellState::new(map.cell_province.iter().map(|&p| map.province_state[p as usize]).collect())
    }

    /// Re-attach a map to a world that was loaded without one (cells and map are not part
    /// of the serialized body), then apply the saved cell differences.
    pub fn attach_map(&mut self, map: Arc<Map>, changes: &CellChanges) -> Result<(), String> {
        if map.province_count() != self.province_count() || map.state_count() > self.country_count as u32 {
            return Err("the save does not fit this map (province or country counts differ)".into());
        }
        let mut cells = Self::fresh_cells(&map);
        cells.apply_changes(changes)?;
        self.cells = Some(cells);
        self.map = Some(MapRef(map));
        Ok(())
    }

    pub fn map(&self) -> Option<&Map> {
        self.map.as_ref().map(|m| &*m.0)
    }

    // ---- setup (scenario building, before the first tick) ----

    pub fn set_controller_for_setup(&mut self, p: ProvinceId, c: CountryId) {
        self.controller[p.index()] = c;
    }

    pub fn add_unit_for_setup(&mut self, unit: Unit) -> UnitId {
        let id = UnitId(self.units.len() as u32);
        self.units.push(Some(unit));
        self.unit_ext.push(UnitExt::default());
        id
    }

    pub fn add_site_for_setup(&mut self, site: Site) {
        self.sites.push(site);
    }

    // ---- read access ----

    gated! {
        fn sites(&self) -> &[Site] {
            &self.sites
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }
    pub fn clock(&self) -> &Clock {
        &self.clock
    }
    pub fn country_count(&self) -> u16 {
        self.country_count
    }
    pub fn province_count(&self) -> u32 {
        self.controller.len() as u32
    }
    gated! {
        fn controller(&self, p: ProvinceId) -> CountryId {
            self.controller[p.index()]
        }
    }
    gated! {
        fn value(&self, p: ProvinceId) -> i64 {
            self.value[p.index()]
        }
    }
    gated! {
        fn unit(&self, u: UnitId) -> Option<&Unit> {
            self.units.get(u.index()).and_then(|x| x.as_ref())
        }
    }
    gated! {
        fn unit_ext(&self, u: UnitId) -> Option<&UnitExt> {
            self.units.get(u.index()).and_then(|x| x.as_ref()).map(|_| &self.unit_ext[u.index()])
        }
    }
    gated! {
        fn unit_ext_mut(&mut self, u: UnitId) -> Option<&mut UnitExt> {
            if self.units.get(u.index()).is_some_and(|x| x.is_some()) { Some(&mut self.unit_ext[u.index()]) } else { None }
        }
    }
    gated! {
        fn forces(&self) -> &Forces {
            &self.forces
        }
    }
    gated! {
        fn forces_mut(&mut self) -> &mut Forces {
            &mut self.forces
        }
    }
    gated! {
        fn combat(&self) -> &CombatState {
            &self.combat
        }
    }
    gated! {
        fn combat_mut(&mut self) -> &mut CombatState {
            &mut self.combat
        }
    }
    gated! {
        fn tech(&self) -> &Tech {
            &self.tech
        }
    }
    gated! {
        fn tech_mut(&mut self) -> &mut Tech {
            &mut self.tech
        }
    }
    gated! {
        fn intel(&self) -> &Intel {
            &self.intel
        }
    }
    gated! {
        fn intel_mut(&mut self) -> &mut Intel {
            &mut self.intel
        }
    }
    gated! {
        fn event_state(&self) -> &EventState {
            &self.scripted
        }
    }
    gated! {
        fn event_state_mut(&mut self) -> &mut EventState {
            &mut self.scripted
        }
    }
    gated! {
        fn polity(&self) -> &Polity {
            &self.polity
        }
    }
    gated! {
        fn polity_mut(&mut self) -> &mut Polity {
            &mut self.polity
        }
    }
    gated! {
        fn diplomacy(&self) -> &Diplomacy {
            &self.diplomacy
        }
    }
    gated! {
        fn diplomacy_mut(&mut self) -> &mut Diplomacy {
            &mut self.diplomacy
        }
    }
    pub fn set_treaty_threshold(&mut self, permille: u32) {
        self.treaty_threshold = permille;
    }

    gated! {
        /// Add a country at run time (a breakaway state or a successor). IDs are never reused.
        fn create_country(&mut self) -> CountryId {
            let id = CountryId(self.country_count);
            self.country_count += 1;
            id
        }
    }

    gated! {
        /// A unit changes sides (a defection or a transfer). Returns false if it does not exist.
        fn set_unit_owner(&mut self, u: UnitId, to: CountryId) -> bool {
            match self.units.get_mut(u.index()).and_then(|x| x.as_mut()) {
                Some(unit) if to.0 < self.country_count => {
                    unit.owner = to;
                    true
                }
                _ => false,
            }
        }
    }

    gated! {
        /// Split a country in two (the civil war outcome). The original government keeps its
        /// ID; a new country takes the listed provinces and units plus `share_permille` of the
        /// treasury, debt and equipment stockpile. Nothing is created or destroyed: people,
        /// goods and units stay where they are, and the money and equipment shares add back
        /// up to the original totals. Returns the new country, or None if the origin is unknown.
        fn split_country(&mut self, origin: CountryId, provinces: &[ProvinceId], units: &[UnitId], share_permille: u32) -> Option<CountryId> {
            if origin.0 >= self.country_count || share_permille > 1000 {
                return None;
            }
            let new = self.create_country();
            for &p in provinces {
                if p.0 >= self.province_count() || self.controller[p.index()] != origin {
                    continue;
                }
                self.controller[p.index()] = new;
                if let (Some(map), Some(cells)) = (self.map.as_ref(), self.cells.as_mut()) {
                    for &c in map.0.cells_of(p) {
                        cells.set_controller(c as usize, new.0);
                    }
                    for b in self.econ.buildings().to_vec() {
                        if b.owner == origin && map.0.cell_province[b.cell as usize] == p.0 {
                            self.econ.set_building(crate::econ::Building { owner: new, ..b });
                        }
                    }
                }
                for s in self.sites.iter_mut() {
                    if s.province == p && s.owner == origin {
                        s.owner = new;
                    }
                }
            }
            for &u in units {
                if self.unit(u).is_some_and(|x| x.owner == origin) {
                    self.set_unit_owner(u, new);
                }
            }
            let f = self.econ.finance(origin);
            let (t, d) = (f.treasury * share_permille as i64 / 1000, f.debt * share_permille as i64 / 1000);
            self.econ.finance_mut(origin).treasury -= t;
            self.econ.finance_mut(origin).debt -= d;
            self.econ.finance_mut(new).treasury += t;
            self.econ.finance_mut(new).debt += d;
            for model in self.econ.models_held_by(origin) {
                let give = self.econ.equipment(origin, model) * share_permille as u64 / 1000;
                self.econ.take_equipment(origin, model, give);
                self.econ.add_equipment(new, model, give);
            }
            let c = self.polity.country(origin);
            *self.polity.country_mut(new) = c;
            Some(new)
        }
    }

    /// Check a command against the current state without changing it. Works on a copy of
    /// the world, so it is exact but not free; use it for player previews, not every tick.
    pub fn dry_run(&self, cmd: &Command, ruleset: &dyn Ruleset) -> Result<(), Rejection> {
        self.clone().apply(cmd, ruleset)
    }

    gated! {
        fn econ(&self) -> &Economy {
            &self.econ
        }
    }
    gated! {
        fn econ_mut(&mut self) -> &mut Economy {
            &mut self.econ
        }
    }

    gated! {
        /// Put a unit in transit: it arrives at `destination` on `arrive_tick`.
        /// Returns false if the unit does not exist or is already in transit.
        fn start_transit(&mut self, u: UnitId, destination: Destination, arrive_tick: u64) -> bool {
            match self.unit_ext_mut(u) {
                Some(e) if e.transit.is_none() => {
                    e.transit = Some(crate::forces::Transit { destination, arrive_tick });
                    true
                }
                _ => false,
            }
        }
    }

    gated! {
        fn unit_count(&self) -> u32 {
            self.units.len() as u32
        }
    }

    // ---- access for rulesets ----

    gated! {
        fn values_mut(&mut self) -> &mut [i64] {
            &mut self.value
        }
    }
    gated! {
        /// Queue an event for the next event phase of the tick.
        fn raise_event(&mut self, event: Event) {
            self.events.push(event);
        }
    }
    gated! {
        fn pending_events(&self) -> &[Event] {
            &self.events
        }
    }
    gated! {
        /// Queue a message for the player. It appears in this tick's report.
        fn message(&mut self, country: CountryId, kind: u16, a: i64, b: i64) {
            self.outbox.messages.push(Message { country, kind, a, b });
        }
    }
    gated! {
        /// Queue a draw-me event for the client. It appears in this tick's report.
        fn emit(&mut self, event: ClientEvent) {
            self.outbox.events.push(event);
        }
    }
    gated! {
        fn rng_mut(&mut self) -> &mut Rng {
            &mut self.rng
        }
    }
    gated! {
        fn set_controller(&mut self, p: ProvinceId, c: CountryId) {
            self.controller[p.index()] = c;
        }
    }

    // ---- the tick ----

    /// One tick: apply commands, run the ruleset, advance the clock.
    pub fn tick(&mut self, commands: &[Command], ruleset: &dyn Ruleset) -> TickReport {
        let signals = self.clock.signals();
        let mut report = TickReport::default();
        for cmd in commands {
            if let Err(why) = self.apply(cmd, ruleset) {
                report.rejected.push((cmd.clone(), why));
            }
        }
        self.complete_transits();
        ruleset.run(self, signals);
        // Event phase: everything raised so far is applied in a fixed order.
        // `sort_by_key` is stable, so equal keys keep their raise order.
        let mut batch = std::mem::take(&mut self.events);
        batch.sort_by_key(|e| (e.target, e.source, e.kind));
        for e in &batch {
            ruleset.on_event(self, e);
        }
        report.tick = self.clock.tick();
        self.clock.advance();
        report.events = std::mem::take(&mut self.outbox.events);
        report.messages = std::mem::take(&mut self.outbox.messages);
        if let Some(c) = self.cells.as_mut() {
            report.changed_cells = c.take_dirty();
        }
        report
    }

    /// Units whose arrival tick has come land now, in unit ID order.
    fn complete_transits(&mut self) {
        let now = self.clock.tick();
        for i in 0..self.units.len() {
            let Some(t) = self.unit_ext[i].transit else { continue };
            if t.arrive_tick > now {
                continue;
            }
            self.unit_ext[i].transit = None;
            match t.destination {
                Destination::Province(p) => {
                    if let Some(u) = self.units[i].as_mut() {
                        u.location = p;
                    }
                    self.unit_ext[i].cell = None;
                }
                Destination::Cell(c) => {
                    if let (Some(map), Some(u)) = (self.map.as_ref(), self.units[i].as_mut()) {
                        if c.0 < map.0.cell_count() {
                            u.location = ProvinceId(map.0.cell_province[c.index()]);
                            self.unit_ext[i].cell = Some(c);
                        }
                    }
                }
            }
        }
    }

    /// Validate fully, then mutate. On Err the world is untouched.
    fn apply(&mut self, cmd: &Command, ruleset: &dyn Ruleset) -> Result<(), Rejection> {
        match *cmd {
            Command::MoveUnit { issuer, unit, to } => {
                self.check_country(issuer)?;
                self.check_province(to)?;
                let u = self.unit(unit).ok_or(Rejection::NoSuchUnit(unit))?;
                if u.owner != issuer {
                    return Err(Rejection::NotOwner { issuer });
                }
                if let Some(Some(u)) = self.units.get_mut(unit.index()) {
                    u.location = to;
                }
                Ok(())
            }
            Command::SetProvinceController { issuer, province, controller } => {
                self.check_country(issuer)?;
                self.check_country(controller)?;
                self.check_province(province)?;
                self.controller[province.index()] = controller;
                Ok(())
            }
            Command::StartConstruction { issuer, region, cell, kind, level } => {
                self.check_country(issuer)?;
                let owner = {
                    let (Some(map), Some(cells)) = (self.map.as_ref(), self.cells.as_ref()) else { return Err(Rejection::NoSuchCell(cell)) };
                    if cell.0 >= map.0.cell_count() {
                        return Err(Rejection::NoSuchCell(cell));
                    }
                    cells.controller(cell.index())
                };
                if owner != issuer.0 {
                    return Err(Rejection::Refused("cell is not under your control"));
                }
                if region >= self.econ.regions {
                    return Err(Rejection::Refused("no such region"));
                }
                if level == 0 {
                    return Err(Rejection::Refused("target level must be above zero"));
                }
                let t = ruleset.project_template(kind).ok_or(Rejection::Refused("cannot be built"))?;
                let now = self.clock.tick();
                self.econ.start_project(ProjectSpec { owner: issuer, region, cell: cell.0, kind, target_level: level }, &t, now);
                Ok(())
            }
            Command::SetWarLevel { issuer, against, level } => {
                self.check_country(issuer)?;
                self.check_country(against)?;
                if issuer == against {
                    return Err(Rejection::Refused("cannot be at war with yourself"));
                }
                if level > MAX_WAR_LEVEL {
                    return Err(Rejection::Refused("no such war level"));
                }
                if level < self.polity.war_level(issuer, against) {
                    return Err(Rejection::Refused("war levels are lowered by treaty"));
                }
                self.polity.set_war_level(issuer, against, level);
                Ok(())
            }
            Command::HostileAct { issuer, against, required_level } => {
                self.check_country(issuer)?;
                self.check_country(against)?;
                if self.polity.war_level(issuer, against) < required_level {
                    return Err(Rejection::Refused("not permitted at your current war level"));
                }
                Ok(())
            }
            Command::SignTreaty { issuer, treaty } => {
                self.check_country(issuer)?;
                let t = self.diplomacy.treaty(treaty).ok_or(Rejection::Refused("no such treaty"))?;
                if !t.parties.contains(&issuer.0) {
                    return Err(Rejection::Refused("you are not a party to this treaty"));
                }
                let now = self.clock.tick();
                let (polity, threshold) = (&self.polity, self.treaty_threshold);
                // A country approves when the factions that do not oppose this kind of treaty
                // carry the world's threshold of weighted support. No factions: it stalls.
                let kind = t.kind;
                let approved = |c: CountryId| polity.faction_vote(c, |f| !polity.opposes(c, f, kind), threshold);
                if self.diplomacy.sign(treaty, now, approved) { Ok(()) } else { Err(Rejection::Refused("treaty not signed yet")) }
            }
            Command::SetStance { .. } | Command::Mobilize { .. } | Command::Demobilize { .. } | Command::Appoint { .. } | Command::Launch { .. }
            | Command::AirMission { .. } | Command::Transit { .. } | Command::SetDoctrine { .. } | Command::Withdraw { .. } | Command::Hold { .. }
            | Command::CancelOrder { .. } | Command::Fortify { .. } | Command::AmphibiousAssault { .. } | Command::JoinAssault { .. }
            | Command::Bombard { .. } | Command::BuildBridge { .. } | Command::DismantleBridge { .. } | Command::DemolishBridge { .. } => self.apply_order(cmd, ruleset),
            Command::ChooseEvent { issuer, event, option } => {
                self.check_country(issuer)?;
                let set = ruleset.event_set().ok_or(Rejection::Refused("no events in this ruleset"))?;
                set.choose(self, issuer, event, option as usize).map_err(Rejection::Refused)
            }
            Command::SetProductionOrder { issuer, model, quantity, priority } => {
                self.check_country(issuer)?;
                self.econ.set_order(ProductionOrder { country: issuer, model, quantity, priority });
                Ok(())
            }
        }
    }

    fn check_country(&self, c: CountryId) -> Result<(), Rejection> {
        if c.0 < self.country_count { Ok(()) } else { Err(Rejection::NoSuchCountry(c)) }
    }
    fn check_province(&self, p: ProvinceId) -> Result<(), Rejection> {
        if p.0 < self.province_count() { Ok(()) } else { Err(Rejection::NoSuchProvince(p)) }
    }

    // ---- state hash ----

    /// Hash of EVERY field of the world, including the RNG state.
    /// If you add a field to World, add it here. `hash_covers_every_field`
    /// in the tests guards against forgetting.
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::default();
        h.write_u64(self.seed);
        h.write_u64(self.clock.tick());
        for w in self.rng.state() {
            h.write_u64(w);
        }
        h.write_u16(self.country_count);
        h.write_u32(self.controller.len() as u32);
        for c in &self.controller {
            h.write_u16(c.0);
        }
        for v in &self.value {
            h.write_i64(*v);
        }
        h.write_u32(self.units.len() as u32);
        for u in &self.units {
            match u {
                None => h.write_u8(0),
                Some(u) => {
                    h.write_u8(1);
                    h.write_u16(u.owner.0);
                    h.write_u32(u.location.0);
                }
            }
        }
        // Military extras are hashed only once something non-default exists, so worlds
        // that never use them keep their old hashes.
        if !self.forces.is_empty() || self.unit_ext.iter().any(|e| *e != UnitExt::default()) {
            h.write_u8(4);
            for e in &self.unit_ext {
                e.hash_into(&mut h);
            }
            self.forces.hash_into(&mut h);
        }
        if self.treaty_threshold != 500 {
            h.write_u8(12);
            h.write_u32(self.treaty_threshold);
        }
        if !self.tech.is_empty() {
            h.write_u8(10);
            self.tech.hash_into(&mut h);
        }
        if !self.intel.is_empty() {
            h.write_u8(11);
            self.intel.hash_into(&mut h);
        }
        if !self.scripted.is_empty() {
            h.write_u8(9);
            self.scripted.hash_into(&mut h);
        }
        if !self.polity.is_empty() {
            h.write_u8(7);
            self.polity.hash_into(&mut h);
        }
        if !self.diplomacy.is_empty() {
            h.write_u8(8);
            self.diplomacy.hash_into(&mut h);
        }
        if !self.econ.is_empty() {
            h.write_u8(6);
            self.econ.hash_into(&mut h);
        }
        if !self.combat.is_empty() {
            h.write_u8(5);
            self.combat.hash_into(&mut h);
        }
        h.write_u32(self.sites.len() as u32);
        for site in &self.sites {
            h.write_u8(site.kind as u8);
            h.write_u32(site.province.0);
            h.write_u16(site.owner.0);
            h.write_u8(site.condition);
            h.write_bytes(&site.built_year.to_le_bytes());
            match site.destroyed_year {
                None => h.write_u8(0),
                Some(y) => {
                    h.write_u8(1);
                    h.write_bytes(&y.to_le_bytes());
                }
            }
        }
        // Pending events are hashed only when present, so event-free hashes are unchanged.
        if !self.events.is_empty() {
            h.write_u8(2);
            h.write_u32(self.events.len() as u32);
            for e in &self.events {
                h.write_u16(e.kind);
                h.write_u32(e.target);
                h.write_u32(e.source);
                h.write_i64(e.amount);
            }
        }
        if let Some(c) = &self.cells {
            h.write_u8(3);
            c.hash_into(&mut h);
        }
        // Only worlds with a map hash it, so map-less hashes are unchanged.
        if let Some(m) = &self.map {
            h.write_u8(1);
            h.write_u32(m.0.checksum);
            h.write_u32(m.0.province_count());
            h.write_u32(m.0.cell_count());
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Nothing;
    impl Ruleset for Nothing {
        fn run(&self, _: &mut World, _: Signals) {}
    }

    fn world() -> World {
        let mut w = World::new(1, 2, 4);
        w.add_unit_for_setup(Unit { owner: CountryId(0), location: ProvinceId(0) });
        w
    }

    #[test]
    fn hash_covers_every_field() {
        let base = world();
        let h0 = base.state_hash();
        let mut variants: Vec<World> = Vec::new();
        let mut w = base.clone(); w.seed += 1; variants.push(w);
        let mut w = base.clone(); w.clock.advance(); variants.push(w);
        let mut w = base.clone(); w.rng.next_u64(); variants.push(w);
        let mut w = base.clone(); w.country_count += 1; variants.push(w);
        let mut w = base.clone(); w.controller[3] = CountryId(1); variants.push(w);
        let mut w = base.clone(); w.value[2] = 5; variants.push(w);
        let mut w = base.clone(); w.units[0].as_mut().unwrap().location = ProvinceId(1); variants.push(w);
        let mut w = base.clone(); w.units[0].as_mut().unwrap().owner = CountryId(1); variants.push(w);
        let mut w = base.clone(); w.units.push(None); variants.push(w);
        let mut w = base.clone(); w.raise_event(Event { kind: 1, target: 2, source: 3, amount: 4 }); variants.push(w);
        let mut w = base.clone(); w.unit_ext_mut(UnitId(0)).unwrap().morale = 5; variants.push(w);
        let mut w = base.clone(); w.unit_ext_mut(UnitId(0)).unwrap().encircled_days = 2; variants.push(w);
        let mut w = base.clone(); w.econ_mut().pop.add_for_setup(crate::econ::PopGroup { cell: 0, culture: 0, religion: 0, heads: 1, support: [0; 4] }); variants.push(w);
        let mut w = base.clone(); w.econ_mut().finance_mut(CountryId(0)).treasury = 5; variants.push(w);
        let mut w = base.clone(); w.set_treaty_threshold(700); variants.push(w);
        let mut w = base.clone(); w.polity_mut().set_law(CountryId(0), 1, 2); variants.push(w);
        let mut w = base.clone(); w.diplomacy_mut().add_alliance(0, vec![0, 1]); variants.push(w);
        let mut w = base.clone(); w.event_state_mut().set_flag(CountryId(0), 3, true); variants.push(w);
        let mut w = base.clone(); w.tech_mut().configure(2); w.tech_mut().set_level(CountryId(0), 1, 5); variants.push(w);
        let mut w = base.clone(); w.intel_mut().set_network(crate::intel::Network { owner: CountryId(0), target: CountryId(1), strength: 3 }); variants.push(w);
        let mut w = base.clone(); w.combat_mut().add_pressure(crate::ids::CellId(1), 4); variants.push(w);
        let mut w = base.clone(); w.combat_mut().add_prisoners(CountryId(0), CountryId(1), 3); variants.push(w);
        let mut w = base.clone(); w.combat_mut().set_fortification(crate::ids::CellId(1), crate::ids::CellId(2), 3); variants.push(w);
        let mut w = base.clone(); w.combat_mut().country_traits_mut(CountryId(1)).add(1); variants.push(w);
        let mut w = base.clone(); w.unit_ext_mut(UnitId(0)).unwrap().parent = Some(UnitId(0)); variants.push(w);
        let mut w = base.clone(); w.unit_ext_mut(UnitId(0)).unwrap().traits.add(3); variants.push(w);
        let mut w = base.clone(); w.start_transit(UnitId(0), Destination::Province(ProvinceId(1)), 9); variants.push(w);
        let mut w = base.clone();
        w.forces_mut().add_officer(crate::forces::Officer { country: CountryId(0), birth_days_before_start: 9000, rank: 1, experience: 0, status: 0, assignment: None, traits: Default::default() });
        let h_off = w.state_hash();
        variants.push(w.clone());
        w.forces_mut().officers[0].experience = 1;
        assert_ne!(w.state_hash(), h_off, "officer experience not hashed");
        let mut w = base.clone();
        w.forces_mut().add_hull(crate::forces::Hull { model: 1, owner: CountryId(0), location: 3, parent: None, condition: 1000, fuel: 5, status: 0 });
        variants.push(w);
        let mut w = base.clone();
        w.forces_mut().add_pilot(crate::forces::Pilot { country: CountryId(0), wing: None, birth_days_before_start: 1, rank: 0, experience: 0, kills: 0, status: 0 });
        variants.push(w);
        let site = Site { kind: SiteKind::City, province: ProvinceId(1), owner: CountryId(0), condition: 100, built_year: 1900, destroyed_year: None };
        let mut w = base.clone(); w.sites.push(site.clone()); variants.push(w);
        let mut w = base.clone(); w.sites.push(site.clone()); let h1 = w.state_hash();
        let mut w2 = w.clone(); w2.sites[0].kind = SiteKind::Port; assert_ne!(h1, w2.state_hash());
        let mut w2 = w.clone(); w2.sites[0].province = ProvinceId(2); assert_ne!(h1, w2.state_hash());
        let mut w2 = w.clone(); w2.sites[0].owner = CountryId(1); assert_ne!(h1, w2.state_hash());
        let mut w2 = w.clone(); w2.sites[0].condition = 50; assert_ne!(h1, w2.state_hash());
        let mut w2 = w.clone(); w2.sites[0].built_year = 1901; assert_ne!(h1, w2.state_hash());
        let mut w2 = w.clone(); w2.sites[0].destroyed_year = Some(1950); assert_ne!(h1, w2.state_hash());
        for (i, v) in variants.iter().enumerate() {
            assert_ne!(v.state_hash(), h0, "variant {i} not reflected in hash");
        }
    }

    #[test]
    fn the_tick_report_lists_changed_cells_events_and_messages() {
        use crate::map::tests::test_map;
        struct Reporter;
        impl Ruleset for Reporter {
            fn run(&self, w: &mut World, s: Signals) {
                if s.new_day {
                    w.cells_mut().unwrap().set_damage(5, 9);
                    w.cells_mut().unwrap().set_damage(2, 9);
                    w.cells_mut().unwrap().set_damage(5, 9); // unchanged value: not a change
                    w.emit(ClientEvent { kind: 1, place: 5, other: 0, amount: 9 });
                    w.message(CountryId(0), 4, 5, 0);
                }
            }
        }
        let mut w = World::from_map(1, Arc::new(Map::from_raw(test_map(16, 16)).unwrap()));
        let r = w.tick(&[], &Reporter);
        assert_eq!((r.tick, r.changed_cells.clone()), (0, vec![2, 5]));
        assert_eq!((r.events.len(), r.messages.len()), (1, 1));
        let r = w.tick(&[], &Reporter);
        assert!(r.changed_cells.is_empty() && r.events.is_empty() && r.messages.is_empty(), "nothing left over, and tick 1 is not a new day");
        assert_eq!(r.tick, 1);
    }

    #[test]
    fn units_in_transit_land_on_their_arrival_tick_in_order() {
        let mut w = world();
        w.add_unit_for_setup(Unit { owner: CountryId(1), location: ProvinceId(0) });
        assert!(w.start_transit(UnitId(0), Destination::Province(ProvinceId(2)), 3));
        assert!(!w.start_transit(UnitId(0), Destination::Province(ProvinceId(3)), 5), "already moving");
        assert!(!w.start_transit(UnitId(7), Destination::Province(ProvinceId(3)), 5), "no such unit");
        for _ in 0..3 {
            w.tick(&[], &Nothing);
            assert_eq!(w.unit(UnitId(0)).unwrap().location, ProvinceId(0));
        }
        w.tick(&[], &Nothing); // tick 3
        assert_eq!(w.unit(UnitId(0)).unwrap().location, ProvinceId(2));
        assert!(w.unit_ext(UnitId(0)).unwrap().transit.is_none());
    }

    #[test]
    fn rejected_commands_leave_state_unchanged() {
        let bad = [
            Command::MoveUnit { issuer: CountryId(0), unit: UnitId(99), to: ProvinceId(1) },
            Command::MoveUnit { issuer: CountryId(0), unit: UnitId(0), to: ProvinceId(99) },
            Command::MoveUnit { issuer: CountryId(1), unit: UnitId(0), to: ProvinceId(1) },
            Command::MoveUnit { issuer: CountryId(9), unit: UnitId(0), to: ProvinceId(1) },
            Command::SetProvinceController { issuer: CountryId(0), province: ProvinceId(99), controller: CountryId(1) },
            Command::SetProvinceController { issuer: CountryId(0), province: ProvinceId(0), controller: CountryId(9) },
        ];
        for cmd in bad {
            let mut w = world();
            // Compare against a world that just ticked with no commands.
            let mut expected = world();
            expected.tick(&[], &Nothing);
            let report = w.tick(std::slice::from_ref(&cmd), &Nothing);
            assert_eq!(report.rejected.len(), 1, "{cmd:?} should be rejected");
            assert_eq!(w, expected, "{cmd:?} changed state");
        }
    }

    #[test]
    fn valid_commands_apply() {
        let mut w = world();
        let r = w.tick(
            &[
                Command::MoveUnit { issuer: CountryId(0), unit: UnitId(0), to: ProvinceId(2) },
                Command::SetProvinceController { issuer: CountryId(1), province: ProvinceId(3), controller: CountryId(1) },
            ],
            &Nothing,
        );
        assert!(r.rejected.is_empty());
        assert_eq!(w.unit(UnitId(0)).unwrap().location, ProvinceId(2));
        assert_eq!(w.controller(ProvinceId(3)), CountryId(1));
    }
}
