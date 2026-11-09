//! Active sets (Phase 4A, step 8): a sorted list of IDs that a system must process.
//! Quiet parts of the world are simply absent, so they cost nothing. Iteration is
//! always in ascending ID order, so it is deterministic.
//!
//! It is a sorted `Vec<u32>` rather than a hash set, because hash sets must never be
//! iterated in simulation code (CLAUDE.md). Insert and remove are O(n) in the worst
//! case; that is fine for sets of thousands, and `extend` merges a batch in one pass.

use crate::hash::StateHasher;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveSet {
    ids: Vec<u32>,
}

impl ActiveSet {
    pub fn new() -> ActiveSet {
        ActiveSet::default()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn contains(&self, id: u32) -> bool {
        self.ids.binary_search(&id).is_ok()
    }
    /// Ascending ID order.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.ids.iter().copied()
    }
    pub fn as_slice(&self) -> &[u32] {
        &self.ids
    }

    /// Returns true if the ID was not already present.
    pub fn insert(&mut self, id: u32) -> bool {
        match self.ids.binary_search(&id) {
            Ok(_) => false,
            Err(pos) => {
                self.ids.insert(pos, id);
                true
            }
        }
    }

    /// Returns true if the ID was present.
    pub fn remove(&mut self, id: u32) -> bool {
        match self.ids.binary_search(&id) {
            Ok(pos) => {
                self.ids.remove(pos);
                true
            }
            Err(_) => false,
        }
    }

    /// Add many IDs (any order, duplicates allowed) with one sort and merge.
    pub fn extend(&mut self, new: &[u32]) {
        if new.is_empty() {
            return;
        }
        self.ids.extend_from_slice(new);
        self.ids.sort_unstable();
        self.ids.dedup();
    }

    /// Keep only IDs for which `keep` returns true; order is preserved. Systems use
    /// this to drop cells that have gone quiet.
    pub fn retain(&mut self, mut keep: impl FnMut(u32) -> bool) {
        self.ids.retain(|&id| keep(id));
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.ids.len() as u32);
        for &id in &self.ids {
            h.write_u32(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;
    use std::collections::BTreeSet;

    #[test]
    fn matches_a_btreeset_under_random_operations() {
        let mut rng = Rng::from_seed(11);
        let mut set = ActiveSet::new();
        let mut model = BTreeSet::new();
        for _ in 0..20_000 {
            let id = rng.below(500) as u32;
            match rng.below(4) {
                0 | 1 => assert_eq!(set.insert(id), model.insert(id)),
                2 => assert_eq!(set.remove(id), model.remove(&id)),
                _ => {
                    let batch: Vec<u32> = (0..5).map(|_| rng.below(500) as u32).collect();
                    set.extend(&batch);
                    model.extend(batch);
                }
            }
        }
        assert_eq!(set.iter().collect::<Vec<_>>(), model.iter().copied().collect::<Vec<_>>());
        set.retain(|id| id % 2 == 0);
        model.retain(|id| id % 2 == 0);
        assert_eq!(set.as_slice(), model.iter().copied().collect::<Vec<_>>().as_slice());
        assert!(set.iter().all(|id| set.contains(id)));
    }
}
