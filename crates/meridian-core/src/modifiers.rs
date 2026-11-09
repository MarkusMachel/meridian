//! Generic modifiers and traits (Phase 4A, step 9).
//!
//! A *modifier* says "add this fraction to this key in this category". Keys and
//! categories are numbers whose meaning lives in rules data (for example key 3 might
//! mean "movement cost"); the engine never names them.
//!
//! Stacking rule (design notes section 12): modifiers in the same category ADD up
//! (and the sum is clamped to that category's cap); then the categories MULTIPLY in
//! ascending category number. So +25% and +25% in one category is +50%, but one
//! +25% in each of two categories is x1.5625.
//!
//! A *trait* is a named bundle of modifiers with a scope. An entity (unit, officer,
//! country) holds a small sorted list of trait IDs. All of it is fixed-point.

use crate::fixed::Fixed;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Modifier {
    /// Which quantity this changes. Meaning is defined by rules data.
    pub key: u16,
    /// Stacking group. Must be below `CategoryRules::len()`.
    pub category: u8,
    /// Fractional change: 0.25 means +25%, -0.5 means -50%.
    pub value: Fixed,
}

/// Per-category clamp on the summed value.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CategoryRules {
    caps: Vec<(Fixed, Fixed)>,
}

impl CategoryRules {
    /// One `(min, max)` pair per category, in category order.
    pub fn new(caps: Vec<(Fixed, Fixed)>) -> CategoryRules {
        assert!(caps.iter().all(|c| c.0 <= c.1), "category cap has min above max");
        CategoryRules { caps }
    }
    pub fn len(&self) -> usize {
        self.caps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.caps.is_empty()
    }
}

/// Apply every modifier with this `key` to `base`. Modifiers in a category outside
/// the rules are ignored (the data validator should reject them earlier).
pub fn apply_modifiers<'a>(base: Fixed, key: u16, mods: impl Iterator<Item = &'a Modifier>, rules: &CategoryRules) -> Fixed {
    let mut sums = vec![Fixed::ZERO; rules.len()];
    for m in mods.filter(|m| m.key == key) {
        if let Some(s) = sums.get_mut(m.category as usize) {
            *s = s.saturating_add(m.value);
        }
    }
    let mut out = base;
    for (sum, &(lo, hi)) in sums.iter().zip(&rules.caps) {
        let clamped = (*sum).max(lo).min(hi);
        let factor = Fixed::ONE.saturating_add(clamped).max(Fixed::ZERO);
        out = out * factor;
    }
    out
}

/// Which holders a trait's modifiers reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Scope {
    /// Only the entity holding the trait.
    Holder,
    /// The holder and everything below it in the command chain.
    HolderAndBelow,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TraitDef {
    pub scope: Scope,
    pub modifiers: Vec<Modifier>,
}

/// Trait definitions indexed by trait ID (`u16`).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct TraitTable {
    defs: Vec<TraitDef>,
}

impl TraitTable {
    pub fn new(defs: Vec<TraitDef>) -> TraitTable {
        TraitTable { defs }
    }
    pub fn get(&self, id: u16) -> Option<&TraitDef> {
        self.defs.get(id as usize)
    }
    pub fn len(&self) -> usize {
        self.defs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// The modifiers that reach an entity: all of its own traits, plus the
    /// `HolderAndBelow` traits of each ancestor (nearest first). Order is fixed:
    /// own traits by ID, then ancestors in the order given, each by ID. Within one
    /// category the sum does not depend on order, but a fixed order keeps any future
    /// order-sensitive use deterministic too.
    pub fn effective(&self, own: &TraitSet, ancestors: &[&TraitSet]) -> Vec<Modifier> {
        let mut out = Vec::new();
        for id in own.iter() {
            if let Some(d) = self.get(id) {
                out.extend_from_slice(&d.modifiers);
            }
        }
        for a in ancestors {
            for id in a.iter() {
                if let Some(d) = self.get(id) {
                    if d.scope == Scope::HolderAndBelow {
                        out.extend_from_slice(&d.modifiers);
                    }
                }
            }
        }
        out
    }
}

/// A sorted list of trait IDs held by one entity.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TraitSet {
    ids: Vec<u16>,
}

impl TraitSet {
    pub fn new() -> TraitSet {
        TraitSet::default()
    }
    pub fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        self.ids.iter().copied()
    }
    pub fn contains(&self, id: u16) -> bool {
        self.ids.binary_search(&id).is_ok()
    }
    pub fn add(&mut self, id: u16) -> bool {
        match self.ids.binary_search(&id) {
            Ok(_) => false,
            Err(p) => {
                self.ids.insert(p, id);
                true
            }
        }
    }
    pub fn remove(&mut self, id: u16) -> bool {
        match self.ids.binary_search(&id) {
            Ok(p) => {
                self.ids.remove(p);
                true
            }
            Err(_) => false,
        }
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pct(n: i64) -> Fixed {
        Fixed::from_ratio(n, 100)
    }
    fn m(key: u16, category: u8, v: Fixed) -> Modifier {
        Modifier { key, category, value: v }
    }
    fn rules() -> CategoryRules {
        // category 0 capped to [-50%, +100%]; category 1 uncapped-ish
        CategoryRules::new(vec![(pct(-50), pct(100)), (pct(-90), pct(1000))])
    }

    #[test]
    fn same_category_adds_different_categories_multiply() {
        let r = rules();
        let same = [m(1, 0, pct(25)), m(1, 0, pct(25))];
        assert_eq!(apply_modifiers(Fixed::from_int(100), 1, same.iter(), &r), Fixed::from_int(150));
        let diff = [m(1, 0, pct(25)), m(1, 1, pct(25))];
        // 100 * 1.25 * 1.25 = 156.25
        assert_eq!(apply_modifiers(Fixed::from_int(100), 1, diff.iter(), &r), Fixed::from_ratio(15625, 100));
    }

    #[test]
    fn caps_and_keys_are_respected() {
        let r = rules();
        let big = [m(1, 0, pct(80)), m(1, 0, pct(80)), m(2, 0, pct(500))];
        // key 1 sum 160% clamps to +100%; key 2 is ignored
        assert_eq!(apply_modifiers(Fixed::from_int(10), 1, big.iter(), &r), Fixed::from_int(20));
        let low = [m(1, 0, pct(-80)), m(1, 0, pct(-80))];
        assert_eq!(apply_modifiers(Fixed::from_int(10), 1, low.iter(), &r), Fixed::from_int(5));
        assert_eq!(apply_modifiers(Fixed::from_int(10), 1, [].iter(), &r), Fixed::from_int(10));
    }

    #[test]
    fn result_does_not_depend_on_modifier_order() {
        let r = rules();
        let a = [m(1, 0, pct(10)), m(1, 1, pct(30)), m(1, 0, pct(-5)), m(1, 1, pct(7))];
        let mut b = a;
        b.reverse();
        assert_eq!(apply_modifiers(Fixed::from_int(1000), 1, a.iter(), &r), apply_modifiers(Fixed::from_int(1000), 1, b.iter(), &r));
    }

    #[test]
    fn traits_respect_scope() {
        let table = TraitTable::new(vec![
            TraitDef { scope: Scope::Holder, modifiers: vec![m(1, 0, pct(10))] },
            TraitDef { scope: Scope::HolderAndBelow, modifiers: vec![m(1, 0, pct(20))] },
        ]);
        let mut army = TraitSet::new();
        army.add(1);
        army.add(0);
        assert!(!army.add(0));
        let mut brigade = TraitSet::new();
        brigade.add(0);
        // brigade: own trait 0 (+10%) plus the army's below-reaching trait 1 (+20%); army's trait 0 does not reach down
        let eff = table.effective(&brigade, &[&army]);
        assert_eq!(eff.len(), 2);
        // 0.1 and 0.2 are not exact in binary fixed-point, so allow a few raw units (about 1e-7).
        let got = apply_modifiers(Fixed::from_int(100), 1, eff.iter(), &rules());
        assert!((got.raw() - Fixed::from_int(130).raw()).abs() < 1024, "{got:?}");
        assert!(army.remove(1) && !army.contains(1));
    }
}
