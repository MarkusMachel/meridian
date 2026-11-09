//! Politics and diplomacy plumbing (Phase 4D, steps 16 and 17). Tables, votes and
//! validation only. Which regimes, laws, factions, treaties and resolutions exist, and
//! every threshold, is rules data (gameplay backlog G3).
//!
//! All tables are sorted vectors; all votes are integer sums with ties resolved by ID.

use crate::hash::StateHasher;
use crate::ids::CountryId;

// ------------------------------------------------------------------ politics

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountryPolity {
    pub regime: u16,
    /// 0 to 100.
    pub legitimacy: u8,
    /// 0 to 100.
    pub stability: u8,
}

/// A faction inside a country. `support` is its share of influence in thousandths;
/// `cohesion` how united it is (0 to 1000).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Faction {
    pub country: CountryId,
    pub kind: u16,
    pub support: u16,
    pub cohesion: u16,
}

/// A rebel or political movement; becomes a country when it wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Movement {
    pub id: u32,
    pub origin: CountryId,
    /// Fixed at formation.
    pub goal: u16,
    pub strength: u32,
    pub cohesion: u16,
    pub became: Option<CountryId>,
}

/// War level of one side against another: 0 none, then rules-defined steps
/// (insurgency, limited, total, nuclear by convention).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct War {
    /// `a < b`.
    pub a: CountryId,
    pub b: CountryId,
    pub level_a: u8,
    pub level_b: u8,
}

pub const MAX_WAR_LEVEL: u8 = 4;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Polity {
    countries: Vec<CountryPolity>,
    factions: Vec<Faction>,
    /// (country, law, value), sorted.
    laws: Vec<(u16, u16, u16)>,
    /// (cell, culture, religion, unrest 0..=1000), sorted by the first three.
    unrest: Vec<(u32, u16, u8, u16)>,
    movements: Vec<Movement>,
    /// (unit, loyalty 0..=1000) and (officer, loyalty), sorted.
    unit_loyalty: Vec<(u32, u16)>,
    officer_loyalty: Vec<(u32, u16)>,
    wars: Vec<War>,
    /// (country, faction kind, treaty kind): this faction opposes treaties of this kind.
    opposes: Vec<(u16, u16, u16)>,
}

fn set_pair(v: &mut Vec<(u32, u16)>, key: u32, value: u16) {
    match v.binary_search_by_key(&key, |x| x.0) {
        Ok(i) => v[i].1 = value,
        Err(i) => v.insert(i, (key, value)),
    }
}

impl Polity {
    pub fn is_empty(&self) -> bool {
        *self == Polity::default()
    }

    pub fn country(&self, c: CountryId) -> CountryPolity {
        self.countries.get(c.index()).copied().unwrap_or_default()
    }
    pub fn country_mut(&mut self, c: CountryId) -> &mut CountryPolity {
        if self.countries.len() <= c.index() {
            self.countries.resize(c.index() + 1, CountryPolity::default());
        }
        &mut self.countries[c.index()]
    }

    // factions
    pub fn set_faction(&mut self, f: Faction) {
        match self.factions.binary_search_by_key(&(f.country.0, f.kind), |x| (x.country.0, x.kind)) {
            Ok(i) => self.factions[i] = f,
            Err(i) => self.factions.insert(i, f),
        }
    }
    pub fn factions_of(&self, c: CountryId) -> &[Faction] {
        let start = self.factions.partition_point(|f| f.country.0 < c.0);
        let n = self.factions[start..].iter().take_while(|f| f.country == c).count();
        &self.factions[start..start + n]
    }

    /// Does a measure pass? Each faction's weight is `support * cohesion / 1000`;
    /// `favors(kind)` says which factions are in favor. Passes if the favoring weight is
    /// at least `threshold_permille` of the total weight. No weight at all means it stalls.
    pub fn faction_vote(&self, c: CountryId, favors: impl Fn(u16) -> bool, threshold_permille: u32) -> bool {
        let (mut yes, mut total) = (0u64, 0u64);
        for f in self.factions_of(c) {
            let w = f.support as u64 * f.cohesion as u64 / 1000;
            total += w;
            if favors(f.kind) {
                yes += w;
            }
        }
        total > 0 && yes * 1000 >= total * threshold_permille as u64
    }

    // stances on treaties
    pub fn set_opposes(&mut self, country: CountryId, faction: u16, treaty_kind: u16, opposes: bool) {
        let key = (country.0, faction, treaty_kind);
        match (self.opposes.binary_search(&key), opposes) {
            (Err(i), true) => self.opposes.insert(i, key),
            (Ok(i), false) => {
                self.opposes.remove(i);
            }
            _ => {}
        }
    }
    pub fn opposes(&self, country: CountryId, faction: u16, treaty_kind: u16) -> bool {
        self.opposes.binary_search(&(country.0, faction, treaty_kind)).is_ok()
    }

    // laws
    pub fn law(&self, c: CountryId, law: u16) -> u16 {
        self.laws.binary_search_by_key(&(c.0, law), |x| (x.0, x.1)).map_or(0, |i| self.laws[i].2)
    }
    pub fn set_law(&mut self, c: CountryId, law: u16, value: u16) {
        match self.laws.binary_search_by_key(&(c.0, law), |x| (x.0, x.1)) {
            Ok(i) => self.laws[i].2 = value,
            Err(i) => self.laws.insert(i, (c.0, law, value)),
        }
    }
    /// Change a law only if the factions pass it. Returns whether it changed.
    pub fn try_change_law(&mut self, c: CountryId, law: u16, value: u16, favors: impl Fn(u16) -> bool, threshold_permille: u32) -> bool {
        if self.faction_vote(c, favors, threshold_permille) {
            self.set_law(c, law, value);
            true
        } else {
            false
        }
    }

    // unrest per population group
    pub fn unrest(&self, cell: u32, culture: u16, religion: u8) -> u16 {
        self.unrest.binary_search_by_key(&(cell, culture, religion), |x| (x.0, x.1, x.2)).map_or(0, |i| self.unrest[i].3)
    }
    pub fn set_unrest(&mut self, cell: u32, culture: u16, religion: u8, value: u16) {
        match self.unrest.binary_search_by_key(&(cell, culture, religion), |x| (x.0, x.1, x.2)) {
            Ok(i) if value == 0 => {
                self.unrest.remove(i);
            }
            Ok(i) => self.unrest[i].3 = value,
            Err(i) if value != 0 => self.unrest.insert(i, (cell, culture, religion, value)),
            Err(_) => {}
        }
    }

    // movements
    pub fn add_movement(&mut self, origin: CountryId, goal: u16, strength: u32, cohesion: u16) -> u32 {
        let id = self.movements.len() as u32;
        self.movements.push(Movement { id, origin, goal, strength, cohesion, became: None });
        id
    }
    pub fn movements(&self) -> &[Movement] {
        &self.movements
    }
    pub fn movement_became(&mut self, id: u32, country: CountryId) {
        if let Some(m) = self.movements.get_mut(id as usize) {
            m.became = Some(country);
        }
    }

    // loyalty
    pub fn unit_loyalty(&self, unit: u32) -> u16 {
        self.unit_loyalty.binary_search_by_key(&unit, |x| x.0).map_or(1000, |i| self.unit_loyalty[i].1)
    }
    pub fn set_unit_loyalty(&mut self, unit: u32, v: u16) {
        set_pair(&mut self.unit_loyalty, unit, v);
    }
    pub fn officer_loyalty(&self, o: u32) -> u16 {
        self.officer_loyalty.binary_search_by_key(&o, |x| x.0).map_or(1000, |i| self.officer_loyalty[i].1)
    }
    pub fn set_officer_loyalty(&mut self, o: u32, v: u16) {
        set_pair(&mut self.officer_loyalty, o, v);
    }

    // war levels
    pub fn war_level(&self, side: CountryId, against: CountryId) -> u8 {
        let (a, b) = (side.0.min(against.0), side.0.max(against.0));
        match self.wars.binary_search_by_key(&(a, b), |w| (w.a.0, w.b.0)) {
            Ok(i) => {
                if side.0 == a { self.wars[i].level_a } else { self.wars[i].level_b }
            }
            Err(_) => 0,
        }
    }
    pub fn set_war_level(&mut self, side: CountryId, against: CountryId, level: u8) {
        let (a, b) = (side.0.min(against.0), side.0.max(against.0));
        let idx = self.wars.binary_search_by_key(&(a, b), |w| (w.a.0, w.b.0));
        let i = idx.unwrap_or_else(|i| {
            self.wars.insert(i, War { a: CountryId(a), b: CountryId(b), level_a: 0, level_b: 0 });
            i
        });
        if side.0 == a { self.wars[i].level_a = level } else { self.wars[i].level_b = level }
    }
    pub fn wars(&self) -> &[War] {
        &self.wars
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.countries.len() as u32);
        for c in &self.countries {
            h.write_u16(c.regime);
            h.write_u8(c.legitimacy);
            h.write_u8(c.stability);
        }
        h.write_u32(self.factions.len() as u32);
        for f in &self.factions {
            h.write_u16(f.country.0);
            h.write_u16(f.kind);
            h.write_u16(f.support);
            h.write_u16(f.cohesion);
        }
        h.write_u32(self.laws.len() as u32);
        for l in &self.laws {
            h.write_u16(l.0);
            h.write_u16(l.1);
            h.write_u16(l.2);
        }
        h.write_u32(self.unrest.len() as u32);
        for u in &self.unrest {
            h.write_u32(u.0);
            h.write_u16(u.1);
            h.write_u8(u.2);
            h.write_u16(u.3);
        }
        h.write_u32(self.movements.len() as u32);
        for m in &self.movements {
            h.write_u32(m.id);
            h.write_u16(m.origin.0);
            h.write_u16(m.goal);
            h.write_u32(m.strength);
            h.write_u16(m.cohesion);
            h.write_u32(m.became.map_or(u32::MAX, |c| c.0 as u32));
        }
        for v in [&self.unit_loyalty, &self.officer_loyalty] {
            h.write_u32(v.len() as u32);
            for x in v.iter() {
                h.write_u32(x.0);
                h.write_u16(x.1);
            }
        }
        h.write_u32(self.opposes.len() as u32);
        for o in &self.opposes {
            h.write_u16(o.0);
            h.write_u16(o.1);
            h.write_u16(o.2);
        }
        h.write_u32(self.wars.len() as u32);
        for w in &self.wars {
            h.write_u16(w.a.0);
            h.write_u16(w.b.0);
            h.write_u8(w.level_a);
            h.write_u8(w.level_b);
        }
    }
}

// ------------------------------------------------------------------ diplomacy

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Treaty {
    pub id: u32,
    pub kind: u16,
    /// Sorted country IDs.
    pub parties: Vec<u16>,
    /// Major treaties need each party's factions to approve.
    pub major: bool,
    pub signed_tick: Option<u64>,
    pub expires: Option<u64>,
}

/// An alliance in a tier (0 historic, 1 regional by convention).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alliance {
    pub id: u32,
    pub tier: u8,
    pub members: Vec<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Pending,
    Join,
    Decline,
}

/// A call to arms: `caller` was attacked by `enemy`; each other member decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllianceCall {
    pub alliance: u32,
    pub caller: CountryId,
    pub enemy: CountryId,
    /// (member, decision), sorted by member.
    pub decisions: Vec<(u16, Decision)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vote {
    Yes,
    No,
    Abstain,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diplomacy {
    treaties: Vec<Treaty>,
    alliances: Vec<Alliance>,
    calls: Vec<AllianceCall>,
    /// Security Council: (country, has_veto), sorted by country.
    council: Vec<(u16, bool)>,
    assembly: Vec<u16>,
}

impl Diplomacy {
    pub fn is_empty(&self) -> bool {
        *self == Diplomacy::default()
    }

    pub fn propose_treaty(&mut self, kind: u16, mut parties: Vec<u16>, major: bool, expires: Option<u64>) -> u32 {
        parties.sort_unstable();
        parties.dedup();
        let id = self.treaties.len() as u32;
        self.treaties.push(Treaty { id, kind, parties, major, signed_tick: None, expires });
        id
    }
    pub fn treaty(&self, id: u32) -> Option<&Treaty> {
        self.treaties.get(id as usize)
    }
    pub fn treaties(&self) -> &[Treaty] {
        &self.treaties
    }
    /// Sign a treaty. For a major treaty `approved(party)` must be true for every party;
    /// otherwise it stalls. Returns false if unsigned still (stalled, unknown or already signed).
    pub fn sign(&mut self, id: u32, now: u64, approved: impl Fn(CountryId) -> bool) -> bool {
        let Some(t) = self.treaties.get_mut(id as usize) else { return false };
        if t.signed_tick.is_some() || (t.major && !t.parties.iter().all(|&p| approved(CountryId(p)))) {
            return false;
        }
        t.signed_tick = Some(now);
        true
    }
    pub fn in_force(&self, id: u32, now: u64) -> bool {
        self.treaty(id).is_some_and(|t| t.signed_tick.is_some() && t.expires.is_none_or(|e| now < e))
    }

    pub fn add_alliance(&mut self, tier: u8, mut members: Vec<u16>) -> u32 {
        members.sort_unstable();
        members.dedup();
        let id = self.alliances.len() as u32;
        self.alliances.push(Alliance { id, tier, members });
        id
    }
    pub fn alliance(&self, id: u32) -> Option<&Alliance> {
        self.alliances.get(id as usize)
    }

    /// Open a call: every other member gets a pending decision. None if the caller is not a member.
    pub fn call_alliance(&mut self, alliance: u32, caller: CountryId, enemy: CountryId) -> Option<usize> {
        let a = self.alliances.get(alliance as usize)?;
        if !a.members.contains(&caller.0) {
            return None;
        }
        let decisions = a.members.iter().filter(|&&m| m != caller.0).map(|&m| (m, Decision::Pending)).collect();
        self.calls.push(AllianceCall { alliance, caller, enemy, decisions });
        Some(self.calls.len() - 1)
    }
    pub fn calls(&self) -> &[AllianceCall] {
        &self.calls
    }
    /// A member answers a call. Each member may answer once. Returns false if it cannot.
    pub fn decide(&mut self, call: usize, member: CountryId, join: bool) -> bool {
        let Some(c) = self.calls.get_mut(call) else { return false };
        match c.decisions.binary_search_by_key(&member.0, |d| d.0) {
            Ok(i) if c.decisions[i].1 == Decision::Pending => {
                c.decisions[i].1 = if join { Decision::Join } else { Decision::Decline };
                true
            }
            _ => false,
        }
    }

    pub fn set_council(&mut self, mut seats: Vec<(u16, bool)>) {
        seats.sort_unstable();
        self.council = seats;
    }
    pub fn set_assembly(&mut self, mut members: Vec<u16>) {
        members.sort_unstable();
        self.assembly = members;
    }
    pub fn council(&self) -> &[(u16, bool)] {
        &self.council
    }

    /// Security Council vote: binding. Needs at least `needed` Yes votes from seated
    /// members and no No vote from a seat with a veto. Votes from non-members are ignored.
    pub fn council_vote(&self, votes: &[(CountryId, Vote)], needed: usize) -> bool {
        let mut yes = 0;
        for &(c, v) in votes {
            if let Ok(i) = self.council.binary_search_by_key(&c.0, |s| s.0) {
                match v {
                    Vote::Yes => yes += 1,
                    Vote::No if self.council[i].1 => return false,
                    _ => {}
                }
            }
        }
        yes >= needed
    }

    /// Assembly vote: non-binding. Passes with a simple majority of those voting Yes or No.
    pub fn assembly_vote(&self, votes: &[(CountryId, Vote)]) -> bool {
        let (mut yes, mut no) = (0, 0);
        for &(c, v) in votes {
            if self.assembly.binary_search(&c.0).is_ok() {
                match v {
                    Vote::Yes => yes += 1,
                    Vote::No => no += 1,
                    Vote::Abstain => {}
                }
            }
        }
        yes > no
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.treaties.len() as u32);
        for t in &self.treaties {
            h.write_u32(t.id);
            h.write_u16(t.kind);
            h.write_u8(t.major as u8);
            h.write_u64(t.signed_tick.unwrap_or(u64::MAX));
            h.write_u64(t.expires.unwrap_or(u64::MAX));
            for p in &t.parties {
                h.write_u16(*p);
            }
        }
        h.write_u32(self.alliances.len() as u32);
        for a in &self.alliances {
            h.write_u8(a.tier);
            for m in &a.members {
                h.write_u16(*m);
            }
        }
        h.write_u32(self.calls.len() as u32);
        for c in &self.calls {
            h.write_u32(c.alliance);
            h.write_u16(c.caller.0);
            h.write_u16(c.enemy.0);
            for d in &c.decisions {
                h.write_u16(d.0);
                h.write_u8(d.1 as u8);
            }
        }
        for s in &self.council {
            h.write_u16(s.0);
            h.write_u8(s.1 as u8);
        }
        for m in &self.assembly {
            h.write_u16(*m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(n: u16) -> CountryId {
        CountryId(n)
    }

    fn polity() -> Polity {
        let mut p = Polity::default();
        // kind 0 (army) strong and united, kind 1 (workers) larger but divided, kind 2 small
        p.set_faction(Faction { country: c(0), kind: 0, support: 400, cohesion: 1000 });
        p.set_faction(Faction { country: c(0), kind: 1, support: 500, cohesion: 400 });
        p.set_faction(Faction { country: c(0), kind: 2, support: 100, cohesion: 1000 });
        p
    }

    #[test]
    fn laws_pass_or_stall_by_faction_weight() {
        let mut p = polity();
        // weights: 400, 200, 100 = 700. Army alone is 571 permille.
        assert!(!p.try_change_law(c(0), 5, 1, |k| k == 1, 500), "divided workers cannot carry it");
        assert_eq!(p.law(c(0), 5), 0);
        assert!(p.try_change_law(c(0), 5, 1, |k| k == 0, 500));
        assert_eq!(p.law(c(0), 5), 1);
        assert!(!p.faction_vote(c(9), |_| true, 1), "no factions: stalls");
        assert_eq!(p.factions_of(c(0)).len(), 3);
    }

    #[test]
    fn war_levels_are_per_side() {
        let mut p = Polity::default();
        p.set_war_level(c(5), c(2), 3);
        assert_eq!((p.war_level(c(5), c(2)), p.war_level(c(2), c(5))), (3, 0));
        p.set_war_level(c(2), c(5), 1);
        assert_eq!((p.war_level(c(5), c(2)), p.war_level(c(2), c(5))), (3, 1));
        assert_eq!(p.wars().len(), 1);
    }

    #[test]
    fn major_treaties_need_every_partys_approval() {
        let mut d = Diplomacy::default();
        let minor = d.propose_treaty(1, vec![3, 1], false, None);
        let major = d.propose_treaty(2, vec![1, 3], true, Some(1000));
        assert!(d.sign(minor, 5, |_| false), "minor needs no approval");
        assert!(!d.sign(major, 5, |c| c.0 == 1), "country 3 withholds approval: stalls");
        assert!(!d.in_force(major, 6));
        assert!(d.sign(major, 7, |_| true));
        assert!(d.in_force(major, 999) && !d.in_force(major, 1000));
        assert!(!d.sign(major, 8, |_| true), "already signed");
        assert_eq!(d.treaty(major).unwrap().parties, vec![1, 3]);
    }

    #[test]
    fn each_alliance_member_decides_for_itself() {
        let mut d = Diplomacy::default();
        let a = d.add_alliance(0, vec![4, 2, 9]);
        assert!(d.call_alliance(a, c(7), c(1)).is_none(), "not a member");
        let call = d.call_alliance(a, c(2), c(1)).unwrap();
        assert!(d.decide(call, c(4), true));
        assert!(!d.decide(call, c(4), false), "only one answer");
        assert!(d.decide(call, c(9), false));
        assert!(!d.decide(call, c(2), true), "the caller does not answer");
        assert_eq!(d.calls()[call].decisions, vec![(4, Decision::Join), (9, Decision::Decline)]);
    }

    #[test]
    fn the_council_respects_vetoes_and_the_assembly_is_a_majority() {
        let mut d = Diplomacy::default();
        d.set_council(vec![(1, true), (2, true), (10, false), (11, false), (12, false)]);
        d.set_assembly(vec![1, 2, 3, 4, 5, 6]);
        let yes = |ids: &[u16]| ids.iter().map(|&i| (c(i), Vote::Yes)).collect::<Vec<_>>();
        assert!(d.council_vote(&yes(&[1, 10, 11]), 3));
        assert!(!d.council_vote(&yes(&[1, 10]), 3), "too few");
        let mut v = yes(&[1, 10, 11, 12]);
        v.push((c(2), Vote::No));
        assert!(!d.council_vote(&v, 3), "veto");
        let mut v = yes(&[1, 10, 11]);
        v.push((c(12), Vote::No));
        assert!(d.council_vote(&v, 3), "a No without veto does not block");
        assert!(d.council_vote(&[(c(1), Vote::Yes), (c(30), Vote::No), (c(10), Vote::Yes), (c(11), Vote::Yes)], 3), "outsiders ignored");
        assert!(d.assembly_vote(&[(c(3), Vote::Yes), (c(4), Vote::No), (c(5), Vote::Yes), (c(6), Vote::Abstain)]));
        assert!(!d.assembly_vote(&[(c(3), Vote::Yes), (c(4), Vote::No)]), "a tie fails");
    }

    #[test]
    fn insertion_order_does_not_change_the_hash() {
        let h = |p: &Polity| {
            let mut s = StateHasher::default();
            p.hash_into(&mut s);
            s.finish()
        };
        let mut a = Polity::default();
        a.set_law(c(1), 2, 3);
        a.set_law(c(0), 9, 1);
        a.set_unit_loyalty(7, 100);
        a.set_unit_loyalty(2, 50);
        let mut b = Polity::default();
        b.set_unit_loyalty(2, 50);
        b.set_unit_loyalty(7, 100);
        b.set_law(c(0), 9, 1);
        b.set_law(c(1), 2, 3);
        assert_eq!(h(&a), h(&b));
        a.set_unrest(4, 1, 1, 300);
        assert_eq!(a.unrest(4, 1, 1), 300);
        a.set_unrest(4, 1, 1, 0);
        assert_eq!(h(&a), h(&b), "zero unrest is removed");
    }
}
