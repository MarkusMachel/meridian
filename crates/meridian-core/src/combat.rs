//! Combat plumbing state (Phase 4B, step 14). Tables only: where fights are, how much
//! pressure a cell is under, who holds prisoners, which garrisons exist, doctrine,
//! fortifications and bridges. The formulas and numbers that USE these tables are
//! rules (gameplay backlog G2) and are not in meridian-core.
//!
//! Every table is a vector kept sorted by its key, so iteration order is the same on
//! every run and lookups are binary searches. All of it is hashed (when not empty).

use crate::hash::StateHasher;
use crate::ids::{CellId, CountryId, UnitId};
use crate::modifiers::TraitSet;

/// One unit's condition when a fight began. Rout is decided from this, not from how
/// the fight goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StartSnapshot {
    pub unit: UnitId,
    pub cohesion: u16,
    pub morale: u16,
    pub strength: u32,
}

/// A direct engagement (assault) in progress on one cell.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Engagement {
    pub cell: CellId,
    pub start_tick: u64,
    /// Principals, sorted by unit ID.
    pub attackers: Vec<StartSnapshot>,
    pub defenders: Vec<StartSnapshot>,
}

/// Prisoners held by `captor`, originally from `origin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrisonerPool {
    pub captor: CountryId,
    pub origin: CountryId,
    pub count: u64,
}

pub const DOCTRINE_TRACKS: usize = 3; // land, air, naval; the names live in rules data

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DoctrineState {
    pub current: u16,
    /// Thousandths.
    pub maturity: u16,
    pub target: Option<u16>,
    /// Ticks left in a transition.
    pub timer: u32,
}

/// A bridge across a river edge between two cells (`a < b`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Bridge {
    pub a: u32,
    pub b: u32,
    /// 0 (destroyed) to 1000.
    pub condition: u16,
    /// Built by a unit (can be dismantled) rather than permanent.
    pub laid: bool,
    pub owner: CountryId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CombatState {
    engagements: Vec<Engagement>,
    /// (cell, signed pressure), sorted by cell. Zero entries are removed.
    pressure: Vec<(u32, i16)>,
    prisoners: Vec<PrisonerPool>,
    /// (cell, unit) of fixed garrisons, sorted.
    garrisons: Vec<(u32, u32)>,
    /// Per country, indexed by country ID; grown on demand.
    doctrine: Vec<[DoctrineState; DOCTRINE_TRACKS]>,
    country_traits: Vec<TraitSet>,
    /// (cell, neighbour cell, value 0 to 255): works built in a cell facing a neighbour.
    fortification: Vec<(u32, u32, u8)>,
    bridges: Vec<Bridge>,
}

fn upsert<T, K: Ord>(v: &mut Vec<T>, item: T, key: impl Fn(&T) -> K) {
    let k = key(&item);
    match v.binary_search_by(|x| key(x).cmp(&k)) {
        Ok(i) => v[i] = item,
        Err(i) => v.insert(i, item),
    }
}

impl CombatState {
    pub fn is_empty(&self) -> bool {
        *self == CombatState::default()
    }

    // ---- engagements ----
    pub fn engagements(&self) -> &[Engagement] {
        &self.engagements
    }
    pub fn engagement_at(&self, cell: CellId) -> Option<&Engagement> {
        self.engagements.binary_search_by_key(&cell.0, |e| e.cell.0).ok().map(|i| &self.engagements[i])
    }
    pub fn set_engagement(&mut self, mut e: Engagement) {
        e.attackers.sort_by_key(|s| s.unit);
        e.defenders.sort_by_key(|s| s.unit);
        upsert(&mut self.engagements, e, |e| e.cell.0);
    }
    pub fn end_engagement(&mut self, cell: CellId) -> bool {
        match self.engagements.binary_search_by_key(&cell.0, |e| e.cell.0) {
            Ok(i) => {
                self.engagements.remove(i);
                true
            }
            Err(_) => false,
        }
    }

    // ---- pressure ----
    pub fn pressure(&self, cell: CellId) -> i16 {
        self.pressure.binary_search_by_key(&cell.0, |p| p.0).map_or(0, |i| self.pressure[i].1)
    }
    pub fn add_pressure(&mut self, cell: CellId, delta: i16) -> i16 {
        let v = self.pressure(cell).saturating_add(delta);
        self.set_pressure(cell, v);
        v
    }
    pub fn set_pressure(&mut self, cell: CellId, value: i16) {
        match self.pressure.binary_search_by_key(&cell.0, |p| p.0) {
            Ok(i) if value == 0 => {
                self.pressure.remove(i);
            }
            Ok(i) => self.pressure[i].1 = value,
            Err(i) if value != 0 => self.pressure.insert(i, (cell.0, value)),
            Err(_) => {}
        }
    }
    pub fn pressured_cells(&self) -> impl Iterator<Item = (CellId, i16)> + '_ {
        self.pressure.iter().map(|p| (CellId(p.0), p.1))
    }

    // ---- prisoners ----
    pub fn add_prisoners(&mut self, captor: CountryId, origin: CountryId, count: u64) {
        let key = (captor.0, origin.0);
        match self.prisoners.binary_search_by_key(&key, |p| (p.captor.0, p.origin.0)) {
            Ok(i) => self.prisoners[i].count += count,
            Err(i) => self.prisoners.insert(i, PrisonerPool { captor, origin, count }),
        }
    }
    pub fn prisoners(&self) -> &[PrisonerPool] {
        &self.prisoners
    }
    pub fn total_prisoners(&self) -> u64 {
        self.prisoners.iter().map(|p| p.count).sum()
    }

    // ---- garrisons ----
    pub fn set_garrison(&mut self, cell: CellId, unit: UnitId) {
        upsert(&mut self.garrisons, (cell.0, unit.0), |g| g.0);
    }
    pub fn garrison_at(&self, cell: CellId) -> Option<UnitId> {
        self.garrisons.binary_search_by_key(&cell.0, |g| g.0).ok().map(|i| UnitId(self.garrisons[i].1))
    }
    pub fn garrison_count(&self) -> usize {
        self.garrisons.len()
    }

    // ---- doctrine and country traits ----
    pub fn doctrine(&self, country: CountryId, track: usize) -> DoctrineState {
        self.doctrine.get(country.index()).map_or(DoctrineState::default(), |d| d[track])
    }
    pub fn set_doctrine(&mut self, country: CountryId, track: usize, state: DoctrineState) {
        if self.doctrine.len() <= country.index() {
            self.doctrine.resize(country.index() + 1, [DoctrineState::default(); DOCTRINE_TRACKS]);
        }
        self.doctrine[country.index()][track] = state;
    }
    pub fn country_traits(&self, country: CountryId) -> Option<&TraitSet> {
        self.country_traits.get(country.index())
    }
    pub fn country_traits_mut(&mut self, country: CountryId) -> &mut TraitSet {
        if self.country_traits.len() <= country.index() {
            self.country_traits.resize(country.index() + 1, TraitSet::new());
        }
        &mut self.country_traits[country.index()]
    }

    // ---- fortification (directional) ----
    pub fn fortification(&self, cell: CellId, facing: CellId) -> u8 {
        self.fortification.binary_search_by_key(&(cell.0, facing.0), |f| (f.0, f.1)).map_or(0, |i| self.fortification[i].2)
    }
    pub fn set_fortification(&mut self, cell: CellId, facing: CellId, value: u8) {
        let key = (cell.0, facing.0);
        match self.fortification.binary_search_by_key(&key, |f| (f.0, f.1)) {
            Ok(i) if value == 0 => {
                self.fortification.remove(i);
            }
            Ok(i) => self.fortification[i].2 = value,
            Err(i) if value != 0 => self.fortification.insert(i, (cell.0, facing.0, value)),
            Err(_) => {}
        }
    }
    pub fn fortified_cells(&self) -> usize {
        let mut n = 0;
        let mut last = None;
        for f in &self.fortification {
            if last != Some(f.0) {
                n += 1;
                last = Some(f.0);
            }
        }
        n
    }

    // ---- bridges ----
    pub fn set_bridge(&mut self, mut b: Bridge) {
        if b.a > b.b {
            std::mem::swap(&mut b.a, &mut b.b);
        }
        upsert(&mut self.bridges, b, |b| (b.a, b.b));
    }
    pub fn bridge(&self, a: CellId, b: CellId) -> Option<&Bridge> {
        let key = (a.0.min(b.0), a.0.max(b.0));
        self.bridges.binary_search_by_key(&key, |x| (x.a, x.b)).ok().map(|i| &self.bridges[i])
    }
    /// All bridges, ascending by (a, b).
    pub fn bridges(&self) -> &[Bridge] {
        &self.bridges
    }
    /// Lower a bridge's condition by `amount` (not below 0, which means destroyed). Returns the
    /// new condition, or `None` if there is no bridge there.
    pub fn damage_bridge(&mut self, a: CellId, b: CellId, amount: u16) -> Option<u16> {
        let key = (a.0.min(b.0), a.0.max(b.0));
        let i = self.bridges.binary_search_by_key(&key, |x| (x.a, x.b)).ok()?;
        self.bridges[i].condition = self.bridges[i].condition.saturating_sub(amount);
        Some(self.bridges[i].condition)
    }
    /// Raise a bridge's condition by `amount` (not above 1000). Returns the new condition.
    pub fn repair_bridge(&mut self, a: CellId, b: CellId, amount: u16) -> Option<u16> {
        let key = (a.0.min(b.0), a.0.max(b.0));
        let i = self.bridges.binary_search_by_key(&key, |x| (x.a, x.b)).ok()?;
        self.bridges[i].condition = self.bridges[i].condition.saturating_add(amount).min(1000);
        Some(self.bridges[i].condition)
    }
    pub fn remove_bridge(&mut self, a: CellId, b: CellId) -> bool {
        let key = (a.0.min(b.0), a.0.max(b.0));
        match self.bridges.binary_search_by_key(&key, |x| (x.a, x.b)) {
            Ok(i) => {
                self.bridges.remove(i);
                true
            }
            Err(_) => false,
        }
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.engagements.len() as u32);
        for e in &self.engagements {
            h.write_u32(e.cell.0);
            h.write_u64(e.start_tick);
            for side in [&e.attackers, &e.defenders] {
                h.write_u32(side.len() as u32);
                for s in side.iter() {
                    h.write_u32(s.unit.0);
                    h.write_u16(s.cohesion);
                    h.write_u16(s.morale);
                    h.write_u32(s.strength);
                }
            }
        }
        h.write_u32(self.pressure.len() as u32);
        for p in &self.pressure {
            h.write_u32(p.0);
            h.write_bytes(&p.1.to_le_bytes());
        }
        h.write_u32(self.prisoners.len() as u32);
        for p in &self.prisoners {
            h.write_u16(p.captor.0);
            h.write_u16(p.origin.0);
            h.write_u64(p.count);
        }
        h.write_u32(self.garrisons.len() as u32);
        for g in &self.garrisons {
            h.write_u32(g.0);
            h.write_u32(g.1);
        }
        h.write_u32(self.doctrine.len() as u32);
        for d in &self.doctrine {
            for t in d {
                h.write_u16(t.current);
                h.write_u16(t.maturity);
                h.write_u32(t.target.map_or(u32::MAX, |x| x as u32));
                h.write_u32(t.timer);
            }
        }
        h.write_u32(self.country_traits.len() as u32);
        for t in &self.country_traits {
            h.write_u16(t.len() as u16);
            for id in t.iter() {
                h.write_u16(id);
            }
        }
        h.write_u32(self.fortification.len() as u32);
        for f in &self.fortification {
            h.write_u32(f.0);
            h.write_u32(f.1);
            h.write_u8(f.2);
        }
        h.write_u32(self.bridges.len() as u32);
        for b in &self.bridges {
            h.write_u32(b.a);
            h.write_u32(b.b);
            h.write_u16(b.condition);
            h.write_u8(b.laid as u8);
            h.write_u16(b.owner.0);
        }
    }
}

/// How many garrison troops a settlement gets on first contact. Pure integer function of
/// the cell's population and built-up level and the country's template (permille of the
/// base rate), so no random numbers are needed and every run agrees.
/// Base rule: one soldier per 200 inhabitants, plus 1% extra per urban level step (0..=255).
pub fn garrison_size(population: u32, urban_level: u8, template_permille: u32) -> u32 {
    let base = population as u64 / 200;
    let with_urban = base * (100 + urban_level as u64 / 5) / 100;
    (with_urban * template_permille as u64 / 1000).min(u32::MAX as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_stay_sorted_and_zero_entries_vanish() {
        let mut c = CombatState::default();
        assert!(c.is_empty());
        c.add_pressure(CellId(9), 5);
        c.add_pressure(CellId(2), -3);
        c.add_pressure(CellId(9), -5);
        assert_eq!(c.pressured_cells().collect::<Vec<_>>(), vec![(CellId(2), -3)]);
        c.set_fortification(CellId(4), CellId(5), 40);
        c.set_fortification(CellId(4), CellId(3), 10);
        c.set_fortification(CellId(1), CellId(0), 7);
        assert_eq!((c.fortification(CellId(4), CellId(5)), c.fortification(CellId(4), CellId(9))), (40, 0));
        assert_eq!(c.fortified_cells(), 2);
        c.set_fortification(CellId(1), CellId(0), 0);
        assert_eq!(c.fortified_cells(), 1);
        c.add_prisoners(CountryId(1), CountryId(0), 100);
        c.add_prisoners(CountryId(1), CountryId(0), 50);
        c.add_prisoners(CountryId(0), CountryId(1), 7);
        assert_eq!(c.total_prisoners(), 157);
        assert_eq!(c.prisoners()[0].captor, CountryId(0));
    }

    #[test]
    fn insertion_order_does_not_matter_to_the_hash() {
        let hash = |c: &CombatState| {
            let mut h = StateHasher::default();
            c.hash_into(&mut h);
            h.finish()
        };
        let mut a = CombatState::default();
        a.set_pressure(CellId(5), 2);
        a.set_pressure(CellId(1), 9);
        a.set_garrison(CellId(7), UnitId(3));
        a.set_garrison(CellId(2), UnitId(8));
        a.set_bridge(Bridge { a: 9, b: 4, condition: 1000, laid: false, owner: CountryId(0) });
        let mut b = CombatState::default();
        b.set_bridge(Bridge { a: 4, b: 9, condition: 1000, laid: false, owner: CountryId(0) });
        b.set_garrison(CellId(2), UnitId(8));
        b.set_garrison(CellId(7), UnitId(3));
        b.set_pressure(CellId(1), 9);
        b.set_pressure(CellId(5), 2);
        assert_eq!(hash(&a), hash(&b));
        assert_eq!(a, b);
        assert!(a.bridge(CellId(9), CellId(4)).is_some());
        assert!(a.remove_bridge(CellId(4), CellId(9)) && a.bridge(CellId(4), CellId(9)).is_none());
    }

    #[test]
    fn engagements_keep_sides_sorted_and_snapshot_is_stored() {
        let mut c = CombatState::default();
        let s = |u| StartSnapshot { unit: UnitId(u), cohesion: 800, morale: 700, strength: 90 };
        c.set_engagement(Engagement { cell: CellId(8), start_tick: 5, attackers: vec![s(4), s(2)], defenders: vec![s(1)] });
        c.set_engagement(Engagement { cell: CellId(3), start_tick: 6, attackers: vec![], defenders: vec![] });
        assert_eq!(c.engagements()[0].cell, CellId(3));
        let e = c.engagement_at(CellId(8)).unwrap();
        assert_eq!(e.attackers.iter().map(|a| a.unit.0).collect::<Vec<_>>(), vec![2, 4]);
        assert!(c.end_engagement(CellId(8)) && !c.end_engagement(CellId(8)));
    }

    #[test]
    fn doctrine_and_traits_grow_on_demand() {
        let mut c = CombatState::default();
        assert_eq!(c.doctrine(CountryId(40), 1), DoctrineState::default());
        c.set_doctrine(CountryId(40), 1, DoctrineState { current: 2, maturity: 300, target: Some(3), timer: 90 });
        assert_eq!(c.doctrine(CountryId(40), 1).timer, 90);
        c.country_traits_mut(CountryId(5)).add(2);
        assert!(c.country_traits(CountryId(5)).unwrap().contains(2));
    }

    #[test]
    fn garrisons_scale_with_population_urbanity_and_template() {
        assert_eq!(garrison_size(0, 0, 1000), 0);
        assert_eq!(garrison_size(20_000, 0, 1000), 100);
        assert!(garrison_size(20_000, 200, 1000) > 100);
        assert_eq!(garrison_size(20_000, 0, 2000), 200);
        assert_eq!(garrison_size(20_000, 255, 1000), garrison_size(20_000, 255, 1000));
    }
}
