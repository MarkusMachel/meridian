//! World state: parallel arrays indexed by ID, plus the clock and RNG.

use crate::clock::{Clock, Signals};
use crate::command::{Command, Rejection};
use crate::hash::StateHasher;
use crate::ids::{CountryId, ProvinceId, UnitId};
use crate::map::Map;
use std::sync::Arc;
use crate::rng::Rng;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub owner: CountryId,
    pub location: ProvinceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
}

/// A deferred effect such as bombardment damage. `kind` is defined by the ruleset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: u16,
    pub target: u32,
    pub source: u32,
    pub amount: i64,
}

#[derive(Debug, Default)]
pub struct TickReport {
    /// Commands that were refused this tick, in submission order.
    pub rejected: Vec<(Command, Rejection)>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct World {
    seed: u64,
    clock: Clock,
    rng: Rng,
    country_count: u16,
    /// Per province, indexed by ProvinceId.
    controller: Vec<CountryId>,
    /// Per province generic value slot; meaning is defined by the ruleset.
    value: Vec<i64>,
    /// Indexed by UnitId; None means the slot is empty.
    units: Vec<Option<Unit>>,
    /// Indexed by SiteId order of creation.
    sites: Vec<Site>,
    /// Events waiting for the next event phase. Normally empty at the end of a tick.
    events: Vec<Event>,
    map: Option<MapRef>,
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
            sites: Vec::new(),
            events: Vec::new(),
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
        w.map = Some(MapRef(map));
        w
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
        id
    }

    pub fn add_site_for_setup(&mut self, site: Site) {
        self.sites.push(site);
    }

    // ---- read access ----

    pub fn sites(&self) -> &[Site] {
        &self.sites
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
    pub fn controller(&self, p: ProvinceId) -> CountryId {
        self.controller[p.index()]
    }
    pub fn value(&self, p: ProvinceId) -> i64 {
        self.value[p.index()]
    }
    pub fn unit(&self, u: UnitId) -> Option<&Unit> {
        self.units.get(u.index()).and_then(|x| x.as_ref())
    }
    pub fn unit_count(&self) -> u32 {
        self.units.len() as u32
    }

    // ---- access for rulesets ----

    pub fn values_mut(&mut self) -> &mut [i64] {
        &mut self.value
    }
    /// Queue an event for the next event phase of the tick.
    pub fn raise_event(&mut self, event: Event) {
        self.events.push(event);
    }
    pub fn pending_events(&self) -> &[Event] {
        &self.events
    }
    pub fn rng_mut(&mut self) -> &mut Rng {
        &mut self.rng
    }
    pub fn set_controller(&mut self, p: ProvinceId, c: CountryId) {
        self.controller[p.index()] = c;
    }

    // ---- the tick ----

    /// One tick: apply commands, run the ruleset, advance the clock.
    pub fn tick(&mut self, commands: &[Command], ruleset: &dyn Ruleset) -> TickReport {
        let signals = self.clock.signals();
        let mut report = TickReport::default();
        for cmd in commands {
            if let Err(why) = self.apply(cmd) {
                report.rejected.push((cmd.clone(), why));
            }
        }
        ruleset.run(self, signals);
        // Event phase: everything raised so far is applied in a fixed order.
        // `sort_by_key` is stable, so equal keys keep their raise order.
        let mut batch = std::mem::take(&mut self.events);
        batch.sort_by_key(|e| (e.target, e.source, e.kind));
        for e in &batch {
            ruleset.on_event(self, e);
        }
        self.clock.advance();
        report
    }

    /// Validate fully, then mutate. On Err the world is untouched.
    fn apply(&mut self, cmd: &Command) -> Result<(), Rejection> {
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
