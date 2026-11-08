//! World state: parallel arrays indexed by ID, plus the clock and RNG.

use crate::clock::{Clock, Signals};
use crate::command::{Command, Rejection};
use crate::hash::StateHasher;
use crate::ids::{CountryId, ProvinceId, UnitId};
use crate::rng::Rng;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub owner: CountryId,
    pub location: ProvinceId,
}

/// Rules run by the tick, in a fixed order. Implementations must keep all
/// state in the World (a ruleset holding its own state would be missed by the
/// state hash) and obey the determinism rules.
pub trait Ruleset {
    fn run(&self, world: &mut World, signals: Signals);
}

#[derive(Debug, Default)]
pub struct TickReport {
    /// Commands that were refused this tick, in submission order.
    pub rejected: Vec<(Command, Rejection)>,
}

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
        }
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

    // ---- read access ----

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
