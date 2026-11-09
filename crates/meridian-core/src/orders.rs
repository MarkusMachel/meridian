//! Validation and storage of orders to forces (Phase 6, steps 9 and 11). Every command here
//! is checked completely before anything changes, and a refusal carries a fixed reason that
//! never reveals hidden facts. What an order DOES over time is up to rules systems; the
//! engine records it in the unit's `order` field (or the doctrine and bridge tables).

use crate::command::{Command, Rejection};
use crate::combat::{Bridge, DOCTRINE_TRACKS};
use crate::forces::{Destination, UnitOrder};
use crate::ids::{CellId, CountryId, UnitId};
use crate::map::{cell_edge_flag, cell_flag};
use crate::world::{Ruleset, World};

type Res<T> = Result<T, Rejection>;

impl World {
    fn owned_unit(&self, issuer: CountryId, unit: UnitId) -> Res<()> {
        if issuer.0 >= self.country_count {
            return Err(Rejection::NoSuchCountry(issuer));
        }
        match self.units.get(unit.index()).and_then(|u| u.as_ref()) {
            None => Err(Rejection::NoSuchUnit(unit)),
            Some(u) if u.owner != issuer => Err(Rejection::NotOwner { issuer }),
            Some(_) => Ok(()),
        }
    }

    fn position_of(&self, unit: UnitId) -> Res<CellId> {
        self.unit_ext[unit.index()].cell.ok_or(Rejection::Refused("unit has no position on the map"))
    }

    fn check_cell(&self, c: CellId) -> Res<()> {
        match self.map.as_ref() {
            Some(m) if c.0 < m.0.cell_count() => Ok(()),
            _ => Err(Rejection::NoSuchCell(c)),
        }
    }

    /// Do two cells share a border?
    fn adjacent(&self, a: CellId, b: CellId) -> bool {
        let Some(m) = self.map.as_ref() else { return false };
        m.0.cell_edges(a).any(|e| m.0.c_dst[e] == b.0)
    }

    /// Can this country see the cell? Its own cells, cells where it has a unit or next to
    /// one, and cells where it holds a contact of at least "detected" knowledge.
    pub(crate) fn cell_visible_to(&self, country: CountryId, cell: CellId) -> bool {
        if self.cells.as_ref().is_some_and(|c| c.controller(cell.index()) == country.0) {
            return true;
        }
        if self.intel.contacts_of(country).iter().any(|c| c.cell == cell.0 && c.knowledge >= 1) {
            return true;
        }
        self.units.iter().enumerate().any(|(i, u)| {
            u.as_ref().is_some_and(|u| u.owner == country) && self.unit_ext[i].cell.is_some_and(|p| p == cell || self.adjacent(p, cell))
        })
    }

    fn set_unit_order(&mut self, unit: UnitId, order: Option<UnitOrder>) {
        self.unit_ext[unit.index()].order = order;
    }

    /// Orders that are not handled directly in `World::apply`.
    pub(crate) fn apply_order(&mut self, cmd: &Command, ruleset: &dyn Ruleset) -> Res<()> {
        match *cmd {
            Command::SetStance { issuer, unit, stance } => {
                self.owned_unit(issuer, unit)?;
                self.unit_ext[unit.index()].stance = stance;
            }
            Command::Mobilize { issuer, unit } => {
                self.owned_unit(issuer, unit)?;
                self.set_unit_order(unit, Some(UnitOrder::Mobilize));
            }
            Command::Demobilize { issuer, unit } => {
                self.owned_unit(issuer, unit)?;
                self.set_unit_order(unit, Some(UnitOrder::Demobilize));
            }
            Command::Hold { issuer, unit } => {
                self.owned_unit(issuer, unit)?;
                self.set_unit_order(unit, Some(UnitOrder::Hold));
            }
            Command::CancelOrder { issuer, unit } => {
                self.owned_unit(issuer, unit)?;
                if self.unit_ext[unit.index()].order.is_none() {
                    return Err(Rejection::Refused("unit has no order to cancel"));
                }
                self.set_unit_order(unit, None);
            }
            Command::Appoint { issuer, officer, unit } => {
                self.owned_unit(issuer, unit)?;
                let o = self.forces.officers.get(officer.index()).ok_or(Rejection::Refused("no such officer"))?;
                if o.country != issuer {
                    return Err(Rejection::NotOwner { issuer });
                }
                if o.status != 0 {
                    return Err(Rejection::Refused("officer is not available"));
                }
                // free the unit's old commander and the officer's old post
                if let Some(old) = self.unit_ext[unit.index()].commander {
                    self.forces.officers[old.index()].assignment = None;
                }
                if let Some(prev) = self.forces.officers[officer.index()].assignment {
                    self.unit_ext[prev.index()].commander = None;
                }
                self.forces.officers[officer.index()].assignment = Some(unit);
                self.unit_ext[unit.index()].commander = Some(officer);
            }
            Command::Withdraw { issuer, unit } => {
                self.owned_unit(issuer, unit)?;
                let pos = self.position_of(unit)?;
                let defending = self.combat.engagement_at(pos).is_some_and(|e| e.defenders.iter().any(|s| s.unit == unit));
                if !defending {
                    return Err(Rejection::Refused("unit is not defending in a fight"));
                }
                self.set_unit_order(unit, Some(UnitOrder::Withdraw));
            }
            Command::Fortify { issuer, unit, facing } => {
                self.owned_unit(issuer, unit)?;
                let pos = self.position_of(unit)?;
                self.check_cell(facing)?;
                if !self.adjacent(pos, facing) {
                    return Err(Rejection::Refused("that direction is not next to the unit"));
                }
                self.set_unit_order(unit, Some(UnitOrder::Fortify { facing }));
            }
            Command::JoinAssault { issuer, unit, cell } => {
                self.owned_unit(issuer, unit)?;
                let pos = self.position_of(unit)?;
                self.check_cell(cell)?;
                if self.combat.engagement_at(cell).is_none() {
                    return Err(Rejection::Refused("there is no assault there"));
                }
                if !self.adjacent(pos, cell) {
                    return Err(Rejection::Refused("unit is too far from the assault"));
                }
                self.set_unit_order(unit, Some(UnitOrder::JoinAssault { cell }));
            }
            Command::AmphibiousAssault { issuer, unit, target } => {
                self.owned_unit(issuer, unit)?;
                self.check_cell(target)?;
                let map = self.map.as_ref().expect("check_cell proved a map exists");
                if map.0.cell_flags[target.index()] & cell_flag::COAST == 0 {
                    return Err(Rejection::Refused("the target is not on the coast"));
                }
                if self.cells.as_ref().is_some_and(|c| c.controller(target.index()) == issuer.0) {
                    return Err(Rejection::Refused("that is already your territory"));
                }
                if ruleset.landing_suitability(self, target) == 0 {
                    return Err(Rejection::Refused("no suitable landing there"));
                }
                self.set_unit_order(unit, Some(UnitOrder::AmphibiousAssault { cell: target }));
            }
            Command::Bombard { issuer, unit, target } => {
                self.owned_unit(issuer, unit)?;
                let pos = self.position_of(unit)?;
                self.check_cell(target)?;
                let range = ruleset.bombard_range_m(self, unit).ok_or(Rejection::Refused("unit cannot bombard"))?;
                let map = self.map.as_ref().expect("check_cell proved a map exists");
                if map.0.cell_distance_m(pos, target) > range {
                    return Err(Rejection::Refused("target is out of range"));
                }
                if !self.cell_visible_to(issuer, target) {
                    return Err(Rejection::Refused("target is not visible"));
                }
                self.set_unit_order(unit, Some(UnitOrder::Bombard { target }));
            }
            Command::Launch { issuer, unit, missile, target, count } => {
                self.owned_unit(issuer, unit)?;
                self.check_cell(target)?;
                if count == 0 {
                    return Err(Rejection::Refused("a launch needs at least one missile"));
                }
                ruleset.validate_launch(self, unit, missile, target, count).map_err(Rejection::Refused)?;
                self.set_unit_order(unit, Some(UnitOrder::Launch { missile, target, count }));
            }
            Command::AirMission { issuer, unit, kind, target } => {
                self.owned_unit(issuer, unit)?;
                self.check_cell(target)?;
                ruleset.validate_mission(self, unit, kind, target).map_err(Rejection::Refused)?;
                self.set_unit_order(unit, Some(UnitOrder::AirMission { kind, target }));
            }
            Command::Transit { issuer, unit, to } => {
                self.owned_unit(issuer, unit)?;
                self.check_cell(to)?;
                let ticks = ruleset.transit_ticks(self, unit, Destination::Cell(to)).ok_or(Rejection::Refused("no route for that move"))?;
                let arrive = self.clock.tick() + ticks;
                if !self.start_transit(unit, Destination::Cell(to), arrive) {
                    return Err(Rejection::Refused("unit is already moving"));
                }
            }
            Command::SetDoctrine { issuer, track, target } => {
                if issuer.0 >= self.country_count {
                    return Err(Rejection::NoSuchCountry(issuer));
                }
                if track as usize >= DOCTRINE_TRACKS {
                    return Err(Rejection::Refused("no such doctrine track"));
                }
                let mut d = self.combat.doctrine(issuer, track as usize);
                if d.current == target || d.target == Some(target) {
                    return Err(Rejection::Refused("that doctrine is already adopted or in progress"));
                }
                d.timer = ruleset.doctrine_transition_ticks(track, d.current, target);
                d.target = Some(target);
                self.combat.set_doctrine(issuer, track as usize, d);
            }
            Command::BuildBridge { issuer, unit, a, b } => {
                self.owned_unit(issuer, unit)?;
                let pos = self.position_of(unit)?;
                self.check_cell(a)?;
                self.check_cell(b)?;
                if !self.adjacent(a, b) {
                    return Err(Rejection::Refused("the two cells are not neighbours"));
                }
                if pos != a && pos != b {
                    return Err(Rejection::Refused("unit is not at the crossing"));
                }
                let map = self.map.as_ref().expect("check_cell proved a map exists");
                let river = map.0.cell_edges(a).any(|e| map.0.c_dst[e] == b.0 && map.0.c_flags[e] & cell_edge_flag::RIVER != 0);
                if !river {
                    return Err(Rejection::Refused("there is no river to bridge"));
                }
                if self.combat.bridge(a, b).is_some() {
                    return Err(Rejection::Refused("there is already a bridge there"));
                }
                if !ruleset.can_build_bridge(self, unit) {
                    return Err(Rejection::Refused("unit has no bridging equipment"));
                }
                // starts at condition 0; a rules system raises it while the unit works
                self.combat.set_bridge(Bridge { a: a.0, b: b.0, condition: 0, laid: true, owner: issuer });
                self.set_unit_order(unit, Some(UnitOrder::BuildBridge { a, b }));
            }
            Command::DismantleBridge { issuer, unit, a, b } => {
                self.owned_unit(issuer, unit)?;
                let pos = self.position_of(unit)?;
                let br = *self.combat.bridge(a, b).ok_or(Rejection::Refused("there is no bridge there"))?;
                if !br.laid || br.owner != issuer {
                    return Err(Rejection::Refused("only your own laid bridges can be dismantled"));
                }
                if pos != a && pos != b {
                    return Err(Rejection::Refused("unit is not at the crossing"));
                }
                self.combat.remove_bridge(a, b);
            }
            Command::DemolishBridge { issuer, unit, a, b } => {
                self.owned_unit(issuer, unit)?;
                let pos = self.position_of(unit)?;
                let br = *self.combat.bridge(a, b).ok_or(Rejection::Refused("there is no bridge there"))?;
                if br.owner != issuer {
                    return Err(Rejection::Refused("only the owner can prepare a demolition"));
                }
                if pos != a && pos != b {
                    return Err(Rejection::Refused("unit is not at the crossing"));
                }
                self.combat.set_bridge(Bridge { condition: 0, ..br });
            }
            _ => unreachable!("handled in World::apply"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::{Engagement, StartSnapshot};
    use crate::forces::Officer;
    use crate::ids::OfficerId;
    use crate::intel::Contact;
    use crate::map::tests::test_map;
    use crate::map::Map;
    use crate::schedule::ScheduleBuilder;
    use crate::world::{Ruleset, Unit};
    use crate::ids::ProvinceId;
    use std::sync::Arc;

    /// A ruleset that allows everything the tests probe.
    struct Capable;
    impl Ruleset for Capable {
        fn run(&self, _: &mut World, _: crate::clock::Signals) {}
        fn bombard_range_m(&self, _: &World, _: UnitId) -> Option<u32> {
            Some(25_000)
        }
        fn landing_suitability(&self, _: &World, _: CellId) -> u8 {
            2
        }
        fn validate_launch(&self, _: &World, _: UnitId, missile: u16, _: CellId, _: u32) -> Result<(), &'static str> {
            if missile == 1 { Ok(()) } else { Err("no such missile in stock") }
        }
        fn validate_mission(&self, _: &World, _: UnitId, kind: u8, _: CellId) -> Result<(), &'static str> {
            if kind < 3 { Ok(()) } else { Err("that mission is not available") }
        }
        fn transit_ticks(&self, _: &World, _: UnitId, _: Destination) -> Option<u64> {
            Some(6)
        }
        fn can_build_bridge(&self, _: &World, _: UnitId) -> bool {
            true
        }
    }

    const C0: CountryId = CountryId(0);
    const C1: CountryId = CountryId(1);

    /// 16x16 map; cell (x, y) = y*16 + x. A river on the edge between cells 40 and 41,
    /// and cell 255 on the coast.
    fn world() -> World {
        let mut r = test_map(16, 16);
        r.cell_flags[255] |= cell_flag::COAST;
        for (a, b) in [(40usize, 41usize), (41, 40)] {
            for e in r.c_off[a] as usize..r.c_off[a + 1] as usize {
                if r.c_dst[e] as usize == b {
                    r.c_flags[e] |= cell_edge_flag::RIVER;
                }
            }
        }
        let mut w = World::from_map(1, Arc::new(Map::from_raw(r).expect("map")));
        for (owner, cell) in [(0u16, 40u32), (0, 42), (1, 200)] {
            let id = w.add_unit_for_setup(Unit { owner: CountryId(owner), location: ProvinceId(0) });
            w.unit_ext_mut(id).unwrap().cell = Some(CellId(cell));
        }
        w
    }

    fn rules() -> crate::schedule::Schedule {
        ScheduleBuilder::new().build().unwrap()
    }

    fn refused(w: &World, c: Command, why: &str) {
        let before = w.state_hash();
        match w.dry_run(&c, &Capable) {
            Err(Rejection::Refused(r)) => assert!(r.contains(why), "{c:?}: got '{r}', wanted '{why}'"),
            other => panic!("{c:?}: expected refusal '{why}', got {other:?}"),
        }
        assert_eq!(w.state_hash(), before);
    }

    #[test]
    fn stance_hold_cancel_and_ownership() {
        let mut w = world();
        let r = w.tick(&[Command::SetStance { issuer: C0, unit: UnitId(0), stance: 1 }, Command::Hold { issuer: C0, unit: UnitId(0) }], &Capable);
        assert!(r.rejected.is_empty());
        assert_eq!(w.unit_ext(UnitId(0)).unwrap().stance, 1);
        assert_eq!(w.unit_ext(UnitId(0)).unwrap().order, Some(UnitOrder::Hold));
        // not yours / no such unit / nothing to cancel
        assert_eq!(w.dry_run(&Command::SetStance { issuer: C1, unit: UnitId(0), stance: 1 }, &Capable), Err(Rejection::NotOwner { issuer: C1 }));
        assert_eq!(w.dry_run(&Command::Hold { issuer: C0, unit: UnitId(9) }, &Capable), Err(Rejection::NoSuchUnit(UnitId(9))));
        assert!(w.tick(&[Command::CancelOrder { issuer: C0, unit: UnitId(0) }], &Capable).rejected.is_empty());
        refused(&w, Command::CancelOrder { issuer: C0, unit: UnitId(0) }, "no order to cancel");
    }

    #[test]
    fn appointing_moves_an_officer_between_posts() {
        let mut w = world();
        let off = |c| Officer { country: c, birth_days_before_start: 15000, rank: 3, experience: 10, status: 0, assignment: None, traits: Default::default() };
        let a = w.forces_mut().add_officer(off(C0));
        let b = w.forces_mut().add_officer(off(C0));
        let foreign = w.forces_mut().add_officer(off(C1));
        w.forces_mut().officers[b.index()].status = 3;
        assert!(w.tick(&[Command::Appoint { issuer: C0, officer: a, unit: UnitId(0) }], &Capable).rejected.is_empty());
        assert_eq!(w.unit_ext(UnitId(0)).unwrap().commander, Some(a));
        // the same officer takes a second post: the first one is left without a commander
        assert!(w.tick(&[Command::Appoint { issuer: C0, officer: a, unit: UnitId(1) }], &Capable).rejected.is_empty());
        assert_eq!(w.unit_ext(UnitId(0)).unwrap().commander, None);
        assert_eq!(w.forces().officers[a.index()].assignment, Some(UnitId(1)));
        refused(&w, Command::Appoint { issuer: C0, officer: b, unit: UnitId(0) }, "not available");
        refused(&w, Command::Appoint { issuer: C0, officer: OfficerId(77), unit: UnitId(0) }, "no such officer");
        assert_eq!(w.dry_run(&Command::Appoint { issuer: C0, officer: foreign, unit: UnitId(0) }, &Capable), Err(Rejection::NotOwner { issuer: C0 }));
    }

    #[test]
    fn withdraw_join_assault_and_fortify_need_the_right_situation() {
        let mut w = world();
        refused(&w, Command::Withdraw { issuer: C0, unit: UnitId(0) }, "not defending");
        let snap = |u| StartSnapshot { unit: UnitId(u), cohesion: 900, morale: 900, strength: 50 };
        w.combat_mut().set_engagement(Engagement { cell: CellId(40), start_tick: 0, attackers: vec![snap(2)], defenders: vec![snap(0)] });
        assert!(w.tick(&[Command::Withdraw { issuer: C0, unit: UnitId(0) }], &Capable).rejected.is_empty());
        assert_eq!(w.unit_ext(UnitId(0)).unwrap().order, Some(UnitOrder::Withdraw));
        // unit 1 (cell 42) is two cells from the fight at 40; it must step to 41 first
        refused(&w, Command::JoinAssault { issuer: C0, unit: UnitId(1), cell: CellId(40) }, "too far");
        refused(&w, Command::JoinAssault { issuer: C0, unit: UnitId(1), cell: CellId(100) }, "no assault");
        w.unit_ext_mut(UnitId(1)).unwrap().cell = Some(CellId(41));
        assert!(w.dry_run(&Command::JoinAssault { issuer: C0, unit: UnitId(1), cell: CellId(40) }, &Capable).is_ok());
        assert!(w.dry_run(&Command::Fortify { issuer: C0, unit: UnitId(0), facing: CellId(41) }, &Capable).is_ok());
        refused(&w, Command::Fortify { issuer: C0, unit: UnitId(0), facing: CellId(44) }, "not next to");
        assert_eq!(w.dry_run(&Command::Fortify { issuer: C0, unit: UnitId(0), facing: CellId(9999) }, &Capable), Err(Rejection::NoSuchCell(CellId(9999))));
    }

    #[test]
    fn bombard_needs_range_and_visibility() {
        let mut w = world();
        // unit 0 at cell 40 (x=8,y=2). Cell 42 is 20 km away (own unit there, so visible).
        assert!(w.dry_run(&Command::Bombard { issuer: C0, unit: UnitId(0), target: CellId(42) }, &Capable).is_ok());
        // enemy unit at cell 200 (x=8,y=12): 100 km, out of the 25 km range
        refused(&w, Command::Bombard { issuer: C0, unit: UnitId(0), target: CellId(200) }, "out of range");
        // in range but unseen: cell 24 (x=8,y=1) is 10 km away and next to the unit at 40? Not adjacent (y differs by 1: cell 40-16=24 yes adjacent).
        // so use a cell in range that is neither adjacent to a unit nor known: x=8,y=4 (cell 72) is 20 km from unit 0
        refused(&w, Command::Bombard { issuer: C0, unit: UnitId(0), target: CellId(72) }, "not visible");
        w.intel_mut().set_contact(Contact { observer: C0, target_kind: 0, target: 2, last_seen: 0, knowledge: 1, cell: 72, strength_est: 10 });
        assert!(w.dry_run(&Command::Bombard { issuer: C0, unit: UnitId(0), target: CellId(72) }, &Capable).is_ok());
        // the default ruleset has no bombardment at all
        assert_eq!(w.dry_run(&Command::Bombard { issuer: C0, unit: UnitId(0), target: CellId(42) }, &rules()), Err(Rejection::Refused("unit cannot bombard")));
    }

    #[test]
    fn launches_missions_and_transits_are_checked_by_the_ruleset() {
        let mut w = world();
        let launch = |m| Command::Launch { issuer: C0, unit: UnitId(0), missile: m, target: CellId(200), count: 2 };
        assert!(w.dry_run(&launch(1), &Capable).is_ok());
        refused(&w, launch(5), "no such missile");
        refused(&w, Command::Launch { issuer: C0, unit: UnitId(0), missile: 1, target: CellId(200), count: 0 }, "at least one");
        refused(&w, Command::AirMission { issuer: C0, unit: UnitId(0), kind: 9, target: CellId(5) }, "not available");
        assert!(w.tick(&[Command::AirMission { issuer: C0, unit: UnitId(0), kind: 1, target: CellId(5) }], &Capable).rejected.is_empty());
        assert_eq!(w.unit_ext(UnitId(0)).unwrap().order, Some(UnitOrder::AirMission { kind: 1, target: CellId(5) }));
        // transit: arrives six ticks later, and a second order while moving is refused
        assert!(w.tick(&[Command::Transit { issuer: C0, unit: UnitId(1), to: CellId(100) }], &Capable).rejected.is_empty());
        refused(&w, Command::Transit { issuer: C0, unit: UnitId(1), to: CellId(101) }, "already moving");
        for _ in 0..6 {
            w.tick(&[], &Capable);
        }
        assert_eq!(w.unit_ext(UnitId(1)).unwrap().cell, Some(CellId(100)));
        assert_eq!(w.dry_run(&Command::Transit { issuer: C0, unit: UnitId(1), to: CellId(101) }, &rules()), Err(Rejection::Refused("no route for that move")));
    }

    #[test]
    fn doctrine_changes_start_a_timed_transition() {
        let mut w = world();
        assert!(w.tick(&[Command::SetDoctrine { issuer: C0, track: 0, target: 3 }], &Capable).rejected.is_empty());
        let d = w.combat().doctrine(C0, 0);
        assert_eq!((d.target, d.timer), (Some(3), 720));
        refused(&w, Command::SetDoctrine { issuer: C0, track: 0, target: 3 }, "already adopted or in progress");
        refused(&w, Command::SetDoctrine { issuer: C0, track: 7, target: 1 }, "no such doctrine track");
    }

    #[test]
    fn bridges_are_built_dismantled_and_demolished_by_their_owner() {
        let mut w = world();
        let build = Command::BuildBridge { issuer: C0, unit: UnitId(0), a: CellId(40), b: CellId(41) };
        refused(&w, Command::BuildBridge { issuer: C0, unit: UnitId(1), a: CellId(40), b: CellId(41) }, "not at the crossing");
        refused(&w, Command::BuildBridge { issuer: C0, unit: UnitId(0), a: CellId(40), b: CellId(24) }, "no river");
        assert_eq!(w.dry_run(&build, &rules()), Err(Rejection::Refused("unit has no bridging equipment")));
        assert!(w.tick(std::slice::from_ref(&build), &Capable).rejected.is_empty());
        let b = *w.combat().bridge(CellId(40), CellId(41)).unwrap();
        assert!(b.laid && b.condition == 0 && b.owner == C0);
        refused(&w, build.clone(), "already a bridge");
        // the enemy cannot dismantle or demolish it
        w.unit_ext_mut(UnitId(2)).unwrap().cell = Some(CellId(41));
        refused(&w, Command::DismantleBridge { issuer: C1, unit: UnitId(2), a: CellId(40), b: CellId(41) }, "only your own");
        refused(&w, Command::DemolishBridge { issuer: C1, unit: UnitId(2), a: CellId(41), b: CellId(40) }, "only the owner");
        w.combat_mut().set_bridge(Bridge { condition: 900, ..b });
        assert!(w.tick(&[Command::DemolishBridge { issuer: C0, unit: UnitId(0), a: CellId(40), b: CellId(41) }], &Capable).rejected.is_empty());
        assert_eq!(w.combat().bridge(CellId(40), CellId(41)).unwrap().condition, 0);
        assert!(w.tick(&[Command::DismantleBridge { issuer: C0, unit: UnitId(0), a: CellId(40), b: CellId(41) }], &Capable).rejected.is_empty());
        assert!(w.combat().bridge(CellId(40), CellId(41)).is_none());
    }

    #[test]
    fn amphibious_assaults_need_a_hostile_coast_and_a_landing_site() {
        let w = world();
        let go = |t| Command::AmphibiousAssault { issuer: C0, unit: UnitId(0), target: CellId(t) };
        refused(&w, go(100), "not on the coast");
        // cell 255 is in country 1's half (x=15)
        assert!(w.dry_run(&go(255), &Capable).is_ok());
        assert_eq!(w.dry_run(&go(255), &rules()), Err(Rejection::Refused("no suitable landing there")));
    }

    #[test]
    fn a_rejected_command_never_changes_state() {
        let mut w = world();
        let before = w.state_hash();
        let bad = [
            Command::Bombard { issuer: C0, unit: UnitId(0), target: CellId(200) },
            Command::Withdraw { issuer: C0, unit: UnitId(0) },
            Command::Hold { issuer: C1, unit: UnitId(0) },
            Command::Transit { issuer: C0, unit: UnitId(0), to: CellId(99999) },
        ];
        let r = w.tick(&bad, &Capable);
        assert_eq!(r.rejected.len(), 4);
        let mut fresh = world();
        fresh.tick(&[], &Capable);
        assert_eq!(w.state_hash(), fresh.state_hash(), "state differs from a tick with no commands (hash before was {before:016x})");
    }
}
