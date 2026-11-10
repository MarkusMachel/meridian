//! Per-country view of the world (Phase 4F, steps 21 and 22). The UI and the AI read the
//! world only through this, so they see what their country knows, not the true state.
//!
//! Own forces and own provinces are exact. Other countries' units appear only through the
//! country's contacts, with fields filled in up to the knowledge level. A debug Cargo
//! feature, `omniscient`, adds raw access to the whole world for tests and debug builds;
//! the AI crate is built without it.

use crate::command::{Command, Rejection};
use crate::ids::{CountryId, ProvinceId, UnitId};
use crate::intel::{estimate_range, Contact};
use crate::world::{Ruleset, Unit, World};

/// A handle for a contact, valid for the view it came from (position in the country's
/// contact list). Orders against an enemy name a contact or a place, never a real unit ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContactId(pub u32);

/// What a country knows about someone else's unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownUnit {
    pub id: ContactId,
    pub knowledge: u8,
    pub last_seen: u64,
    pub last_cell: u32,
    /// Only from "identified" up.
    pub owner: Option<CountryId>,
    /// Estimated strength range, only from "identified" up.
    pub strength: Option<(i64, i64)>,
}

pub struct CountryView<'a> {
    world: &'a World,
    country: CountryId,
}

impl World {
    pub fn view(&self, country: CountryId) -> CountryView<'_> {
        CountryView { world: self, country }
    }
}

impl<'a> CountryView<'a> {
    pub fn country(&self) -> CountryId {
        self.country
    }
    pub fn tick(&self) -> u64 {
        self.world.clock().tick()
    }

    /// The country's own units, exact.
    pub fn own_units(&self) -> Vec<(UnitId, &'a Unit)> {
        (0..self.world.unit_count()).filter_map(|i| self.world.unit(UnitId(i)).filter(|u| u.owner == self.country).map(|u| (UnitId(i), u))).collect()
    }

    /// Provinces the country controls, exact.
    pub fn own_provinces(&self) -> Vec<ProvinceId> {
        (0..self.world.province_count()).map(ProvinceId).filter(|&p| self.world.controller(p) == self.country).collect()
    }

    /// Who controls a province, as far as this country can tell: its own provinces and
    /// provinces where it has a unit are known; anywhere else it is not.
    pub fn province_controller(&self, p: ProvinceId) -> Option<CountryId> {
        let c = self.world.controller(p);
        if c == self.country || self.own_units().iter().any(|(_, u)| u.location == p) {
            Some(c)
        } else {
            None
        }
    }

    /// Every bridge with its condition. Bridges are landmarks: any country knows where they are
    /// and whether they stand.
    pub fn bridges(&self) -> &'a [crate::combat::Bridge] {
        self.world.combat().bridges()
    }

    /// Units of other countries this country knows about, from its contact list.
    pub fn known_units(&self) -> Vec<KnownUnit> {
        self.world
            .intel()
            .contacts_of(self.country)
            .iter()
            .enumerate()
            .filter(|(_, c)| c.target_kind == 0 && c.knowledge >= 1)
            .map(|(i, c)| self.known(i, c))
            .collect()
    }

    fn known(&self, i: usize, c: &Contact) -> KnownUnit {
        let identified = c.knowledge >= 2;
        let key = (self.country.0 as u64) << 40 ^ (c.target as u64) << 8 ^ (self.tick() / 24);
        KnownUnit {
            id: ContactId(i as u32),
            knowledge: c.knowledge,
            last_seen: c.last_seen,
            last_cell: c.cell,
            owner: identified.then(|| self.world.unit(UnitId(c.target)).map(|u| u.owner)).flatten(),
            strength: identified.then(|| estimate_range(c.strength_est as i64, c.knowledge, key)),
        }
    }

    /// Would this command be accepted right now? Changes nothing. See `World::dry_run`.
    pub fn check(&self, cmd: &Command, ruleset: &dyn Ruleset) -> Result<(), Rejection> {
        self.world.dry_run(cmd, ruleset)
    }

    /// Raw access to the true world. Debug builds and tests only.
    #[cfg(feature = "omniscient")]
    pub fn omniscient(&self) -> &'a World {
        self.world
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::Contact;
    use crate::schedule::ScheduleBuilder;

    fn world() -> World {
        let mut w = World::new(1, 3, 4);
        w.add_unit_for_setup(Unit { owner: CountryId(0), location: ProvinceId(0) });
        w.add_unit_for_setup(Unit { owner: CountryId(1), location: ProvinceId(2) });
        w.set_controller_for_setup(ProvinceId(2), CountryId(1));
        w
    }

    #[test]
    fn a_country_sees_its_own_forces_exactly_and_others_only_through_contacts() {
        let mut w = world();
        let v = w.view(CountryId(0));
        assert_eq!(v.own_units().len(), 1);
        assert!(v.known_units().is_empty(), "the enemy unit is invisible without a contact");
        assert_eq!(v.province_controller(ProvinceId(2)), None);
        assert_eq!(v.province_controller(ProvinceId(0)), Some(CountryId(0)));

        for (level, expect_owner) in [(1u8, false), (2, true), (3, true)] {
            w.intel_mut().set_contact(Contact { observer: CountryId(0), target_kind: 0, target: 1, last_seen: 0, knowledge: level, cell: 7, strength_est: 800 });
            let k = w.view(CountryId(0)).known_units();
            assert_eq!(k.len(), 1);
            assert_eq!(k[0].owner.is_some(), expect_owner);
            assert_eq!(k[0].strength.is_some(), expect_owner);
            assert_eq!(k[0].last_cell, 7);
            if level == 3 {
                assert_eq!(k[0].strength, Some((800, 800)));
            }
        }
        // country 2 knows nothing of that contact
        assert!(w.view(CountryId(2)).known_units().is_empty());
    }

    #[test]
    fn checking_a_command_does_not_change_the_world() {
        let w = world();
        let rules = ScheduleBuilder::new().build().unwrap();
        let before = w.state_hash();
        let ok = Command::MoveUnit { issuer: CountryId(0), unit: UnitId(0), to: ProvinceId(1) };
        let bad = Command::MoveUnit { issuer: CountryId(0), unit: UnitId(1), to: ProvinceId(1) };
        assert!(w.view(CountryId(0)).check(&ok, &rules).is_ok());
        assert_eq!(w.view(CountryId(0)).check(&bad, &rules), Err(Rejection::NotOwner { issuer: CountryId(0) }));
        assert_eq!(w.state_hash(), before);
    }
}
