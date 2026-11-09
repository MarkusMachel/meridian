//! Trigger-and-effect hooks (Phase 4A, step 5).
//!
//! A rule is data: "at this rate, if this condition holds, apply these effects".
//! The vocabulary is a small fixed set of generic primitives; it knows nothing about
//! gameplay. No scripting language is involved. The data loader (meridian-data) will
//! build `Rule`s from RON files; this module only defines and runs them.
//!
//! Rust note for C# readers: an `enum` with fields is like a tagged union (a sealed
//! class hierarchy); `match` is an exhaustive switch over it.
//!
//! Evaluation is two-phase so rule order cannot change which rules fire: all
//! conditions are checked against the state at the start, then the effects of the
//! rules that fired are applied in rule-ID order.

use crate::clock::Signals;
use crate::ids::{CountryId, ProvinceId};
use crate::schedule::Rate;
use crate::world::{Event, World};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    Always,
    /// The province's value slot is at least `amount`.
    ValueAtLeast { province: ProvinceId, amount: i64 },
    ControllerIs { province: ProvinceId, country: CountryId },
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    AddValue { province: ProvinceId, amount: i64 },
    SetController { province: ProvinceId, country: CountryId },
    RaiseEvent(Event),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub id: u32,
    pub rate: Rate,
    pub condition: Condition,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookError {
    DuplicateId(u32),
    /// A rule refers to a province or country that does not exist.
    BadReference { rule: u32, what: String },
}

impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookError::DuplicateId(id) => write!(f, "rule {id} defined twice"),
            HookError::BadReference { rule, what } => write!(f, "rule {rule}: {what}"),
        }
    }
}

/// Rules sorted by ID.
#[derive(Clone, Debug, Default)]
pub struct HookSet {
    rules: Vec<Rule>,
}

impl HookSet {
    pub fn new(mut rules: Vec<Rule>) -> Result<HookSet, HookError> {
        rules.sort_by_key(|r| r.id);
        if let Some(w) = rules.windows(2).find(|w| w[0].id == w[1].id) {
            return Err(HookError::DuplicateId(w[0].id));
        }
        Ok(HookSet { rules })
    }

    /// Report every reference to a province or country that does not exist.
    pub fn check(&self, provinces: u32, countries: u16) -> Vec<HookError> {
        let mut errs = Vec::new();
        for r in &self.rules {
            let mut bad = |what: String| errs.push(HookError::BadReference { rule: r.id, what });
            check_condition(&r.condition, provinces, countries, &mut bad);
            for e in &r.effects {
                match e {
                    Effect::AddValue { province, .. } if province.0 >= provinces => bad(format!("province {} does not exist", province.0)),
                    Effect::SetController { province, country } => {
                        if province.0 >= provinces {
                            bad(format!("province {} does not exist", province.0));
                        }
                        if country.0 >= countries {
                            bad(format!("country {} does not exist", country.0));
                        }
                    }
                    _ => {}
                }
            }
        }
        errs
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Run every rule whose rate is due. Call `check` first; effects that point at
    /// missing things are skipped rather than crashing.
    pub fn run(&self, world: &mut World, signals: Signals) {
        let fired: Vec<&Rule> = self.rules.iter().filter(|r| r.rate_due(signals) && holds(&r.condition, world)).collect();
        for r in fired {
            for e in &r.effects {
                apply(e, world);
            }
        }
    }
}

impl Rule {
    fn rate_due(&self, s: Signals) -> bool {
        match self.rate {
            Rate::Hourly => true,
            Rate::Daily => s.new_day,
            Rate::Monthly => s.new_month,
            Rate::Yearly => s.new_year,
        }
    }
}

fn check_condition(c: &Condition, provinces: u32, countries: u16, bad: &mut dyn FnMut(String)) {
    match c {
        Condition::Always => {}
        Condition::ValueAtLeast { province, .. } => {
            if province.0 >= provinces {
                bad(format!("province {} does not exist", province.0));
            }
        }
        Condition::ControllerIs { province, country } => {
            if province.0 >= provinces {
                bad(format!("province {} does not exist", province.0));
            }
            if country.0 >= countries {
                bad(format!("country {} does not exist", country.0));
            }
        }
        Condition::All(v) | Condition::Any(v) => v.iter().for_each(|c| check_condition(c, provinces, countries, bad)),
        Condition::Not(c) => check_condition(c, provinces, countries, bad),
    }
}

fn holds(c: &Condition, w: &World) -> bool {
    match c {
        Condition::Always => true,
        Condition::ValueAtLeast { province, amount } => province.0 < w.province_count() && w.value(*province) >= *amount,
        Condition::ControllerIs { province, country } => province.0 < w.province_count() && w.controller(*province) == *country,
        Condition::All(v) => v.iter().all(|c| holds(c, w)),
        Condition::Any(v) => v.iter().any(|c| holds(c, w)),
        Condition::Not(c) => !holds(c, w),
    }
}

fn apply(e: &Effect, w: &mut World) {
    match e {
        Effect::AddValue { province, amount } => {
            if let Some(v) = w.values_mut().get_mut(province.index()) {
                *v = v.saturating_add(*amount);
            }
        }
        Effect::SetController { province, country } => {
            if province.0 < w.province_count() && country.0 < w.country_count() {
                w.set_controller(*province, *country);
            }
        }
        Effect::RaiseEvent(ev) => w.raise_event(*ev),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(i: u32) -> ProvinceId {
        ProvinceId(i)
    }

    #[test]
    fn conditions_are_checked_before_any_effect_applies() {
        // Rule 1 raises province 0 to 5. Rule 2 fires only if province 0 >= 5.
        // With two-phase evaluation rule 2 must NOT fire in the same pass.
        let rules = vec![
            Rule { id: 2, rate: Rate::Hourly, condition: Condition::ValueAtLeast { province: p(0), amount: 5 }, effects: vec![Effect::AddValue { province: p(1), amount: 1 }] },
            Rule { id: 1, rate: Rate::Hourly, condition: Condition::Always, effects: vec![Effect::AddValue { province: p(0), amount: 5 }] },
        ];
        let hooks = HookSet::new(rules).unwrap();
        let mut w = World::new(1, 2, 2);
        hooks.run(&mut w, Signals::default());
        assert_eq!((w.value(p(0)), w.value(p(1))), (5, 0));
        hooks.run(&mut w, Signals::default());
        assert_eq!((w.value(p(0)), w.value(p(1))), (10, 1));
    }

    #[test]
    fn logic_and_rates() {
        let both = Condition::All(vec![Condition::Always, Condition::Not(Box::new(Condition::ControllerIs { province: p(0), country: CountryId(1) }))]);
        let hooks = HookSet::new(vec![Rule { id: 1, rate: Rate::Daily, condition: both, effects: vec![Effect::SetController { province: p(0), country: CountryId(1) }] }]).unwrap();
        let mut w = World::new(1, 2, 1);
        hooks.run(&mut w, Signals::default());
        assert_eq!(w.controller(p(0)), CountryId(0), "daily rule must not run on a non-day tick");
        hooks.run(&mut w, Signals { new_day: true, ..Signals::default() });
        assert_eq!(w.controller(p(0)), CountryId(1));
    }

    #[test]
    fn duplicate_ids_and_bad_references_are_reported() {
        let r = |id| Rule { id, rate: Rate::Hourly, condition: Condition::Always, effects: vec![] };
        assert_eq!(HookSet::new(vec![r(1), r(1)]).err(), Some(HookError::DuplicateId(1)));
        let bad = Rule { id: 3, rate: Rate::Hourly, condition: Condition::ControllerIs { province: p(9), country: CountryId(0) }, effects: vec![Effect::SetController { province: p(0), country: CountryId(7) }] };
        let errs = HookSet::new(vec![bad]).unwrap().check(2, 2);
        assert_eq!(errs.len(), 2);
    }
}
