//! Intelligence plumbing (Phase 4E, step 18): agencies with stored capability, networks,
//! operations, per-country contacts and a deterministic estimate function for views.
//! What each capability or operation does is rules data.

use crate::hash::StateHasher;
use crate::ids::CountryId;

/// Capability disciplines (human sources, signals, imagery, open sources,
/// counterintelligence, covert action by convention).
pub const DISCIPLINES: usize = 6;

pub const AGENCY_FOREIGN: u8 = 1;
pub const AGENCY_DOMESTIC: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Agency {
    pub country: CountryId,
    pub flags: u8,
    /// 0 to 100 per discipline.
    pub capability: [u8; DISCIPLINES],
}

/// An abstract network of sources inside a target country.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Network {
    pub owner: CountryId,
    pub target: CountryId,
    pub strength: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operation {
    pub id: u32,
    pub owner: CountryId,
    pub target: CountryId,
    pub kind: u16,
    pub progress: u16,
    pub exposed: bool,
    /// Who the target believes did it. Can be wrong, and is stored separately from `owner`.
    pub attributed_to: Option<CountryId>,
}

/// What `observer` knows about a thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contact {
    pub observer: CountryId,
    pub target_kind: u8,
    pub target: u32,
    pub last_seen: u64,
    /// 0 unknown, 1 detected, 2 identified, 3 detailed.
    pub knowledge: u8,
    pub cell: u32,
    /// Estimated strength, coarse.
    pub strength_est: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Intel {
    agencies: Vec<Agency>,
    networks: Vec<Network>,
    operations: Vec<Operation>,
    contacts: Vec<Contact>,
}

impl Intel {
    pub fn is_empty(&self) -> bool {
        *self == Intel::default()
    }

    // agencies (one per country and flag set; sorted by country then flags)
    pub fn set_agency(&mut self, a: Agency) {
        match self.agencies.binary_search_by_key(&(a.country.0, a.flags), |x| (x.country.0, x.flags)) {
            Ok(i) => self.agencies[i] = a,
            Err(i) => self.agencies.insert(i, a),
        }
    }
    pub fn agencies(&self) -> &[Agency] {
        &self.agencies
    }
    /// Best capability a country has in a discipline across its agencies.
    pub fn capability(&self, c: CountryId, discipline: usize) -> u8 {
        self.agencies.iter().filter(|a| a.country == c).map(|a| a.capability[discipline]).max().unwrap_or(0)
    }

    // networks
    pub fn set_network(&mut self, n: Network) {
        match self.networks.binary_search_by_key(&(n.owner.0, n.target.0), |x| (x.owner.0, x.target.0)) {
            Ok(i) if n.strength == 0 => {
                self.networks.remove(i);
            }
            Ok(i) => self.networks[i] = n,
            Err(i) if n.strength > 0 => self.networks.insert(i, n),
            Err(_) => {}
        }
    }
    pub fn network(&self, owner: CountryId, target: CountryId) -> u16 {
        self.networks.binary_search_by_key(&(owner.0, target.0), |x| (x.owner.0, x.target.0)).map_or(0, |i| self.networks[i].strength)
    }

    // operations
    pub fn start_operation(&mut self, owner: CountryId, target: CountryId, kind: u16) -> u32 {
        let id = self.operations.len() as u32;
        self.operations.push(Operation { id, owner, target, kind, progress: 0, exposed: false, attributed_to: None });
        id
    }
    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }
    pub fn operation_mut(&mut self, id: u32) -> Option<&mut Operation> {
        self.operations.get_mut(id as usize)
    }

    // contacts
    pub fn set_contact(&mut self, c: Contact) {
        let key = (c.observer.0, c.target_kind, c.target);
        match self.contacts.binary_search_by_key(&key, |x| (x.observer.0, x.target_kind, x.target)) {
            Ok(i) => self.contacts[i] = c,
            Err(i) => self.contacts.insert(i, c),
        }
    }
    pub fn contact(&self, observer: CountryId, target_kind: u8, target: u32) -> Option<&Contact> {
        self.contacts.binary_search_by_key(&(observer.0, target_kind, target), |x| (x.observer.0, x.target_kind, x.target)).ok().map(|i| &self.contacts[i])
    }
    pub fn contacts_of(&self, observer: CountryId) -> &[Contact] {
        let start = self.contacts.partition_point(|c| c.observer.0 < observer.0);
        let n = self.contacts[start..].iter().take_while(|c| c.observer == observer).count();
        &self.contacts[start..start + n]
    }
    /// Drop contacts last seen before `cutoff_tick`. Returns how many.
    pub fn expire_contacts(&mut self, cutoff_tick: u64) -> usize {
        let before = self.contacts.len();
        self.contacts.retain(|c| c.last_seen >= cutoff_tick);
        before - self.contacts.len()
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.agencies.len() as u32);
        for a in &self.agencies {
            h.write_u16(a.country.0);
            h.write_u8(a.flags);
            h.write_bytes(&a.capability);
        }
        h.write_u32(self.networks.len() as u32);
        for n in &self.networks {
            h.write_u16(n.owner.0);
            h.write_u16(n.target.0);
            h.write_u16(n.strength);
        }
        h.write_u32(self.operations.len() as u32);
        for o in &self.operations {
            h.write_u32(o.id);
            h.write_u16(o.owner.0);
            h.write_u16(o.target.0);
            h.write_u16(o.kind);
            h.write_u16(o.progress);
            h.write_u8(o.exposed as u8);
            h.write_u32(o.attributed_to.map_or(u32::MAX, |c| c.0 as u32));
        }
        h.write_u32(self.contacts.len() as u32);
        for c in &self.contacts {
            h.write_u16(c.observer.0);
            h.write_u8(c.target_kind);
            h.write_u32(c.target);
            h.write_u64(c.last_seen);
            h.write_u8(c.knowledge);
            h.write_u32(c.cell);
            h.write_u16(c.strength_est);
        }
    }
}

/// Margin (thousandths of the true value) by knowledge level: unknown, detected,
/// identified, detailed.
pub const MARGIN_PERMILLE: [i64; 4] = [1000, 400, 150, 0];

/// A deterministic estimate range of a true value. The margin shrinks as knowledge rises;
/// the true value is always inside the range; the range is centred off the truth by an
/// amount derived from `key` (so the same observer, target and day always see the same
/// range, and no random numbers are consumed). Returns `(low, high)`.
pub fn estimate_range(true_value: i64, knowledge: u8, key: u64) -> (i64, i64) {
    let m = MARGIN_PERMILLE[(knowledge as usize).min(3)];
    if m == 0 {
        return (true_value, true_value);
    }
    // splitmix64 finalizer as a stateless mixer
    let mut z = key.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let shift_permille = (z % (2 * m as u64 + 1)) as i64 - m;
    let width = true_value.abs() * m / 1000;
    let centre = true_value + true_value * shift_permille / 1000;
    ((centre - width).max(0).min(true_value), (centre + width).max(true_value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_contain_the_truth_and_tighten_with_knowledge() {
        for key in 0..2000u64 {
            for level in 0..4u8 {
                let (lo, hi) = estimate_range(1000, level, key);
                assert!(lo <= 1000 && 1000 <= hi, "level {level} key {key}: {lo}..{hi}");
                assert_eq!(estimate_range(1000, level, key), (lo, hi), "deterministic");
                if level == 3 {
                    assert_eq!((lo, hi), (1000, 1000));
                }
            }
        }
        // on average the range narrows with each level
        let avg = |level| (0..500u64).map(|k| { let (l, h) = estimate_range(1000, level, k); h - l }).sum::<i64>();
        assert!(avg(0) > avg(1) && avg(1) > avg(2) && avg(2) > 0);
        assert_eq!(estimate_range(0, 1, 5), (0, 0));
    }

    #[test]
    fn contacts_are_sorted_per_observer_and_expire() {
        let mut i = Intel::default();
        let c = |o: u16, t: u32, seen| Contact { observer: CountryId(o), target_kind: 0, target: t, last_seen: seen, knowledge: 1, cell: 0, strength_est: 5 };
        i.set_contact(c(2, 9, 10));
        i.set_contact(c(1, 4, 50));
        i.set_contact(c(2, 3, 20));
        assert_eq!(i.contacts_of(CountryId(2)).iter().map(|x| x.target).collect::<Vec<_>>(), vec![3, 9]);
        assert!(i.contact(CountryId(1), 0, 4).is_some() && i.contact(CountryId(1), 0, 5).is_none());
        assert_eq!(i.expire_contacts(15), 1, "only the contact last seen at tick 10 expires");
        assert_eq!(i.contacts_of(CountryId(2)).len(), 1);
    }

    #[test]
    fn agencies_networks_and_operations() {
        let mut i = Intel::default();
        i.set_agency(Agency { country: CountryId(0), flags: AGENCY_FOREIGN, capability: [10, 20, 30, 40, 50, 60] });
        i.set_agency(Agency { country: CountryId(0), flags: AGENCY_DOMESTIC, capability: [90, 0, 0, 0, 0, 0] });
        assert_eq!((i.capability(CountryId(0), 0), i.capability(CountryId(0), 5), i.capability(CountryId(1), 0)), (90, 60, 0));
        i.set_network(Network { owner: CountryId(0), target: CountryId(3), strength: 70 });
        assert_eq!(i.network(CountryId(0), CountryId(3)), 70);
        i.set_network(Network { owner: CountryId(0), target: CountryId(3), strength: 0 });
        assert_eq!(i.network(CountryId(0), CountryId(3)), 0);
        let op = i.start_operation(CountryId(0), CountryId(3), 2);
        let o = i.operation_mut(op).unwrap();
        o.exposed = true;
        o.attributed_to = Some(CountryId(5));
        assert_eq!(i.operations()[0].owner, CountryId(0), "attribution is separate from the true owner");
    }
}
