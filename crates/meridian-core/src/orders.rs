//! Validation and storage of orders to forces (Phase 6, steps 9 and 11). Every command here
//! is checked completely before anything changes, and a refusal carries a fixed reason that
//! never reveals hidden facts. What an order DOES over time is up to rules systems; the
//! engine records it in the unit's `order` field (or the doctrine and bridge tables).

use crate::command::{Command, Rejection};
use crate::combat::{Bridge, DOCTRINE_TRACKS};
use crate::forces::{Destination, UnitOrder};
use crate::ids::{CellId, CountryId, ProvinceId, UnitId};
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

    /// Is there a bridge between these two cells that can carry a load needing `min_condition`?
    fn bridge_usable(&self, a: CellId, b: CellId, min_condition: u16) -> bool {
        self.combat.bridge(a, b).is_some_and(|br| br.condition >= min_condition.max(1))
    }

    /// Ticks a marching unit needs to step from a cell into a neighbour: the step's cost in
    /// metres (terrain, road, river or bridge as they are now) over the unit's pace, at least 1.
    fn march_step_ticks(&self, from: CellId, to: CellId, profile: &crate::world::MarchProfile) -> u64 {
        let map = self.map.as_ref().expect("marching needs a map");
        let bridged = self.bridge_usable(from, to, profile.min_bridge_condition);
        let cost = map.0.cell_step_cost(from, to, &profile.costs, bridged).unwrap_or(u32::MAX);
        (cost as u64).div_ceil(profile.m_per_tick.max(1) as u64).max(1)
    }

    /// Move every marching unit whose step is due into its next cell, in unit order, and set
    /// its next step. A unit that comes to a river with no usable bridge ahead (the bridge may
    /// have been blown since the route was planned) plans its route again from where it stands:
    /// it may go round, or ford and pay for it. A unit the rules no longer let march stops.
    pub(crate) fn advance_marches(&mut self, ruleset: &dyn Ruleset) {
        let Some(map) = self.map.clone() else { return };
        let now = self.clock.tick();
        let mut scratch = self.take_search_memory();
        for i in 0..self.units.len() {
            if self.unit_ext[i].march.as_ref().is_none_or(|m| m.arrive_tick > now) {
                continue;
            }
            let mut march = self.unit_ext[i].march.take().expect("checked above");
            let Some(entered) = march.path.pop() else { continue };
            self.unit_ext[i].cell = Some(CellId(entered));
            if let Some(u) = self.units[i].as_mut() {
                u.location = ProvinceId(map.0.cell_province[entered as usize]);
            }
            let (Some(&next), Some(&goal)) = (march.path.last(), march.path.first()) else { continue };
            let Some(profile) = ruleset.march_profile(self, UnitId(i as u32)).filter(|p| p.m_per_tick > 0) else { continue };
            let (here, mut next) = (CellId(entered), CellId(next));
            if map.0.river_between(here, next) && !self.bridge_usable(here, next, profile.min_bridge_condition) {
                let Some(route) = self.find_cell_path(here, CellId(goal), &profile.costs, profile.min_bridge_condition, &mut scratch) else { continue };
                march.path = route.nodes.iter().skip(1).rev().copied().collect();
                let Some(&n) = march.path.last() else { continue };
                next = CellId(n);
            }
            march.arrive_tick = now + self.march_step_ticks(here, next, &profile);
            self.unit_ext[i].march = Some(march);
        }
        self.return_search_memory(scratch);
    }

    /// Put the permanent bridges of 1990 into the bridge table: one at full condition on every
    /// river edge of the map that a road or a railway crosses (`Map::bridge_edges`), owned by
    /// the country holding the lower-numbered cell. Edges that already have a bridge are left
    /// alone. Returns how many were added. Call once when setting a scenario up; a map baked
    /// before the road and railway edge flags existed gives none.
    pub fn seed_bridges(&mut self) -> usize {
        let Some(map) = self.map.as_ref() else { return 0 };
        let edges = map.0.bridge_edges();
        let mut added = 0;
        for (a, b, _) in edges {
            if self.combat.bridge(CellId(a), CellId(b)).is_none() {
                let owner = CountryId(self.cells.as_ref().map_or(0, |c| c.controller(a as usize)));
                self.combat.set_bridge(Bridge { a, b, condition: 1000, laid: false, owner });
                added += 1;
            }
        }
        added
    }

    /// Cheapest path between two cells, counting the bridges as they stand now: a river edge
    /// with a bridge of at least `min_condition` costs `costs.bridge_extra_m`, any other river
    /// edge `costs.river_extra_m`. The costs and the condition a unit needs are the rules' to
    /// choose (a tank needs a sounder bridge than a rifleman).
    pub fn find_cell_path(&self, from: CellId, to: CellId, costs: &crate::map::CellCosts, min_condition: u16, scratch: &mut crate::map::PathScratch) -> Option<crate::map::Path> {
        let map = self.map.as_ref()?;
        let usable = |a: u32, b: u32| self.combat.bridge(CellId(a), CellId(b)).is_some_and(|br| br.condition >= min_condition.max(1));
        map.0.find_cell_path_bridged(from, to, costs, scratch, &usable)
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
            Command::MoveToCell { issuer, unit, to } => {
                self.owned_unit(issuer, unit)?;
                let pos = self.position_of(unit)?;
                self.check_cell(to)?;
                if self.unit_ext[unit.index()].transit.is_some() {
                    return Err(Rejection::Refused("unit is in transit"));
                }
                let profile = ruleset.march_profile(self, unit).filter(|p| p.m_per_tick > 0).ok_or(Rejection::Refused("unit cannot march"))?;
                if pos == to {
                    self.unit_ext[unit.index()].march = None;
                    return Ok(());
                }
                let mut scratch = self.take_search_memory();
                let route = self.find_cell_path(pos, to, &profile.costs, profile.min_bridge_condition, &mut scratch);
                self.return_search_memory(scratch);
                let route = route.ok_or(Rejection::Refused("no route for that move"))?;
                // the route as the march keeps it: without the cell the unit stands in, next cell last
                let path: Vec<u32> = route.nodes.iter().skip(1).rev().copied().collect();
                let Some(&first) = path.last() else { return Err(Rejection::Refused("no route for that move")) };
                let arrive_tick = self.clock.tick() + self.march_step_ticks(pos, CellId(first), &profile);
                self.unit_ext[unit.index()].march = Some(crate::forces::March { path, arrive_tick });
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
                if pos != a && pos != b {
                    return Err(Rejection::Refused("unit is not at the crossing"));
                }
                if br.laid {
                    if br.owner != issuer {
                        return Err(Rejection::Refused("only the owner can prepare a demolition"));
                    }
                } else if !self.cells.as_ref().is_some_and(|c| c.controller(pos.index()) == issuer.0) {
                    // a permanent bridge belongs to whoever holds the bank the unit stands on
                    return Err(Rejection::Refused("only the country holding this bank can demolish a permanent bridge"));
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
        fn march_profile(&self, _: &World, _: UnitId) -> Option<crate::world::MarchProfile> {
            Some(march_rules())
        }
    }

    /// Fording a river costs as much as 40 km of open ground; a sound bridge costs nothing.
    fn march_rules() -> crate::world::MarchProfile {
        let costs = crate::map::CellCosts { river_extra_m: 40_000, bridge_extra_m: 0, ..Default::default() };
        crate::world::MarchProfile { costs, min_bridge_condition: 500, m_per_tick: 5_000 }
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
    fn permanent_bridges_are_seeded_carry_traffic_and_can_be_blown_by_whoever_holds_the_bank() {
        use crate::map::{CellCosts, PathScratch};
        // the same map, with a road over the river between 40 and 41, and a second river edge
        // (between 56 and 57, one row down) that nothing crosses
        let mut r = test_map(16, 16);
        for (a, b, f) in [(40usize, 41usize, cell_edge_flag::RIVER | cell_edge_flag::ROAD), (56, 57, cell_edge_flag::RIVER)] {
            for (a, b) in [(a, b), (b, a)] {
                for e in r.c_off[a] as usize..r.c_off[a + 1] as usize {
                    if r.c_dst[e] as usize == b {
                        r.c_flags[e] |= f;
                    }
                }
            }
        }
        let mut w = World::from_map(1, Arc::new(Map::from_raw(r).expect("map")));
        let before = w.state_hash();
        assert_eq!(w.seed_bridges(), 1);
        assert_eq!(w.seed_bridges(), 0, "seeding twice adds nothing");
        assert_ne!(w.state_hash(), before, "bridges are part of the state");
        let br = *w.combat().bridge(CellId(41), CellId(40)).unwrap();
        assert_eq!((br.a, br.b, br.condition, br.laid), (40, 41, 1000, false));
        assert!(w.combat().bridge(CellId(56), CellId(57)).is_none());

        // crossing the river costs extra, unless a good enough bridge carries you over
        let costs = CellCosts { river_extra_m: 9_000, bridge_extra_m: 500, ..CellCosts::default() };
        let mut s = PathScratch::new(256);
        let cost = |w: &World, s: &mut PathScratch, min: u16| w.find_cell_path(CellId(40), CellId(41), &costs, min, s).unwrap().cost;
        let dry = w.map().unwrap().find_cell_path(CellId(40), CellId(41), &CellCosts { river_extra_m: 0, ..costs.clone() }, &mut s).unwrap().cost;
        assert_eq!(cost(&w, &mut s, 500), dry + 500);
        // a damaged bridge still carries light traffic but not what needs a sound one
        assert_eq!(w.combat_mut().damage_bridge(CellId(40), CellId(41), 700), Some(300));
        assert_eq!(cost(&w, &mut s, 200), dry + 500);
        let without = cost(&w, &mut s, 500);
        assert!(without > dry + 500 && without <= dry + 9_000, "{without}: ford here or go round");
        assert_eq!(w.combat_mut().repair_bridge(CellId(40), CellId(41), 5_000), Some(1000));

        // demolition: the unit must stand at the crossing, on a bank its country holds
        let c0 = CountryId(w.cells().unwrap().controller(40));
        let other = if c0 == C0 { C1 } else { C0 };
        for (owner, cell) in [(c0, 40u32), (other, 41), (c0, 44)] {
            let id = w.add_unit_for_setup(Unit { owner, location: ProvinceId(0) });
            w.unit_ext_mut(id).unwrap().cell = Some(CellId(cell));
        }
        let blow = |issuer, unit| Command::DemolishBridge { issuer, unit: UnitId(unit), a: CellId(40), b: CellId(41) };
        refused(&w, blow(c0, 2), "not at the crossing");
        if w.cells().unwrap().controller(41) != other.0 {
            refused(&w, blow(other, 1), "holding this bank");
        }
        assert!(w.tick(&[blow(c0, 0)], &Capable).rejected.is_empty());
        assert_eq!(w.combat().bridge(CellId(40), CellId(41)).unwrap().condition, 0);
        assert_eq!(cost(&w, &mut s, 1), without, "a destroyed bridge carries nothing");
        // a permanent bridge cannot be dismantled and taken away
        refused(&w, Command::DismantleBridge { issuer: c0, unit: UnitId(0), a: CellId(40), b: CellId(41) }, "only your own laid");
    }

    #[test]
    fn units_march_cell_by_cell_and_rivers_and_blown_bridges_slow_them() {
        use crate::map::PathScratch;
        // a river down the whole map between columns 8 and 9, so there is no way round it;
        // the only bridge place is in row 2, between cells 40 and 41
        let river_world = || {
            let mut r = test_map(16, 16);
            // open plain everywhere, no roads: the straight row is the one cheapest route
            r.cell_terrain.iter_mut().for_each(|t| *t = 0);
            r.cell_flags.iter_mut().for_each(|f| *f = 0);
            for y in 0..16usize {
                for (a, b) in [(y * 16 + 8, y * 16 + 9), (y * 16 + 9, y * 16 + 8)] {
                    for e in r.c_off[a] as usize..r.c_off[a + 1] as usize {
                        if r.c_dst[e] as usize == b {
                            r.c_flags[e] |= cell_edge_flag::RIVER;
                        }
                    }
                }
            }
            let mut w = World::from_map(1, Arc::new(Map::from_raw(r).expect("map")));
            let id = w.add_unit_for_setup(Unit { owner: C0, location: ProvinceId(0) });
            w.unit_ext_mut(id).unwrap().cell = Some(CellId(38));
            w
        };
        let rules = march_rules();
        // march from cell 38 (two cells west of the bridge) to 43; returns (tick, cell) for each
        // cell entered and whether each step was over a usable bridge at that moment
        let run = |bridge: Option<u16>, blow_at: Option<u64>| -> Vec<(u64, u32)> {
            let mut w = river_world();
            if let Some(condition) = bridge {
                w.combat_mut().set_bridge(Bridge { a: 40, b: 41, condition, laid: false, owner: C0 });
            }
            let order = Command::MoveToCell { issuer: C0, unit: UnitId(0), to: CellId(43) };
            assert!(w.tick(std::slice::from_ref(&order), &Capable).rejected.is_empty());
            let mut seen = vec![(0u64, 38u32)];
            let mut t = 0;
            while w.unit_ext(UnitId(0)).unwrap().march.is_some() {
                t += 1;
                assert!(t < 500, "the march never ends");
                if blow_at == Some(t) {
                    w.combat_mut().damage_bridge(CellId(40), CellId(41), 1000);
                }
                w.tick(&[], &Capable);
                let cell = w.unit_ext(UnitId(0)).unwrap().cell.unwrap().0;
                if cell != seen.last().unwrap().1 {
                    seen.push((t, cell));
                }
            }
            assert_eq!(seen.last().unwrap().1, 43);
            // the unit's province follows its cell
            assert_eq!(w.unit(UnitId(0)).unwrap().location.0, w.map().unwrap().cell_province[43]);
            seen
        };
        // what the pathfinder says: the route and its cost with and without the bridge
        let w = river_world();
        let map = w.map().unwrap();
        let mut s = PathScratch::new(256);
        let with_bridge = map.find_cell_path_bridged(CellId(38), CellId(43), &rules.costs, &mut s, &|a, b| (a.min(b), a.max(b)) == (40, 41)).unwrap();
        let without = map.find_cell_path(CellId(38), CellId(43), &rules.costs, &mut s).unwrap();
        assert!(with_bridge.nodes.windows(2).any(|p| (p[0], p[1]) == (40, 41)), "the bridged route uses the bridge: {:?}", with_bridge.nodes);
        assert!(without.cost > with_bridge.cost, "fording must cost more than the bridge");
        // ticks of a whole march: each step's cost over the pace, rounded up
        let ticks = |route: &[u32], bridged: bool| -> u64 {
            route.windows(2).map(|p| (map.cell_step_cost(CellId(p[0]), CellId(p[1]), &rules.costs, bridged && (p[0], p[1]) == (40, 41)).unwrap() as u64).div_ceil(5_000).max(1)).sum()
        };

        // over a sound bridge: the pathfinder's route, cell by cell, each step taking its time
        let over = run(Some(1000), None);
        assert_eq!(over.iter().map(|x| x.1).collect::<Vec<_>>(), with_bridge.nodes);
        assert_eq!(over.last().unwrap().0, ticks(&with_bridge.nodes, true));
        // no bridge, or one too weak for the unit: the fording route, and it takes longer
        for bridge in [None, Some(300)] {
            let forded = run(bridge, None);
            assert_eq!(forded.iter().map(|x| x.1).collect::<Vec<_>>(), without.nodes);
            assert_eq!(forded.last().unwrap().0, ticks(&without.nodes, false));
            assert!(forded.last().unwrap().0 > over.last().unwrap().0);
        }
        // the bridge is blown while the unit is still walking toward it: it follows the bridged
        // route to the bank (cell 40), plans again there, and arrives later than over the bridge
        let blown = run(Some(1000), Some(1));
        let bank = blown.iter().position(|x| x.1 == 40).expect("it reaches the bank");
        assert_eq!(blown[..=bank].iter().map(|x| x.1).collect::<Vec<_>>(), with_bridge.nodes[..=bank]);
        let onward = map.find_cell_path(CellId(40), CellId(43), &rules.costs, &mut s).unwrap();
        assert_eq!(blown[bank..].iter().map(|x| x.1).collect::<Vec<_>>(), onward.nodes);
        assert!(blown.last().unwrap().0 > over.last().unwrap().0);

        // orders: a unit with no position, an unknown cell, a ruleset that lets nobody march
        let mut w = world();
        let go = |unit, to| Command::MoveToCell { issuer: C0, unit: UnitId(unit), to: CellId(to) };
        assert_eq!(w.dry_run(&go(0, 43), &super::tests::rules()), Err(Rejection::Refused("unit cannot march")));
        assert_eq!(w.dry_run(&go(0, 9_999), &Capable), Err(Rejection::NoSuchCell(CellId(9_999))));
        assert!(matches!(w.dry_run(&go(2, 43), &Capable), Err(Rejection::NotOwner { .. })));
        // a new order replaces the route, and an order to the cell it stands in halts the unit
        assert!(w.tick(&[go(0, 43)], &Capable).rejected.is_empty());
        assert_eq!(w.unit_ext(UnitId(0)).unwrap().march.as_ref().unwrap().destination(), Some(CellId(43)));
        assert!(w.tick(&[go(0, 8)], &Capable).rejected.is_empty());
        assert_eq!(w.unit_ext(UnitId(0)).unwrap().march.as_ref().unwrap().destination(), Some(CellId(8)));
        let here = w.unit_ext(UnitId(0)).unwrap().cell.unwrap();
        let hash_marching = w.state_hash();
        assert!(w.tick(&[go(0, here.0)], &Capable).rejected.is_empty());
        assert!(w.unit_ext(UnitId(0)).unwrap().march.is_none());
        assert_ne!(w.state_hash(), hash_marching, "a march is part of the state");
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
