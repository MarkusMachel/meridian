//! Events plumbing (Phase 4D, step 20): data-defined events with triggers, options,
//! chains, cooldowns and flags. Uses the condition/effect vocabulary from `hooks.rs`, so
//! events and hooks speak the same data language. No scripting language.
//!
//! State (all hashed): pending events waiting for a choice, flags per country, chain
//! stages and cooldowns. The definitions are data held by the ruleset, not state.

use crate::clock::Signals;
use crate::hash::StateHasher;
use crate::hooks::{apply_effect, holds, Condition, Effect};
use crate::ids::CountryId;
use crate::schedule::Rate;
use crate::world::World;

/// One event for one country. Templates for many countries are expanded by the data
/// loader into separate definitions that share a `chain`, which is how linked instances
/// per party are expressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventDef {
    pub id: u32,
    pub country: CountryId,
    pub trigger: Condition,
    /// Each option is a list of effects. A country picks one.
    pub options: Vec<Vec<Effect>>,
    /// Ticks before it can fire again.
    pub cooldown: u64,
    /// Optional chain gate: (chain, stage required, stage set after a choice).
    pub chain: Option<(u16, u16, u16)>,
    /// Severity code for the client (pause, ping, inbox, ticker); the engine only stores it.
    pub severity: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub event: u32,
    pub raised_tick: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventState {
    /// Sorted by event ID; at most one pending instance per event.
    pending: Vec<Pending>,
    /// (country, flag), sorted.
    flags: Vec<(u16, u16)>,
    /// (chain, stage), sorted by chain; absent means stage 0.
    chains: Vec<(u16, u16)>,
    /// (event, tick before which it cannot fire), sorted.
    cooldowns: Vec<(u32, u64)>,
}

impl EventState {
    pub fn is_empty(&self) -> bool {
        *self == EventState::default()
    }
    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }
    pub fn flag(&self, c: CountryId, flag: u16) -> bool {
        self.flags.binary_search(&(c.0, flag)).is_ok()
    }
    pub fn set_flag(&mut self, c: CountryId, flag: u16, on: bool) {
        match (self.flags.binary_search(&(c.0, flag)), on) {
            (Err(i), true) => self.flags.insert(i, (c.0, flag)),
            (Ok(i), false) => {
                self.flags.remove(i);
            }
            _ => {}
        }
    }
    pub fn chain_stage(&self, chain: u16) -> u16 {
        self.chains.binary_search_by_key(&chain, |x| x.0).map_or(0, |i| self.chains[i].1)
    }
    pub fn set_chain_stage(&mut self, chain: u16, stage: u16) {
        match self.chains.binary_search_by_key(&chain, |x| x.0) {
            Ok(i) if stage == 0 => {
                self.chains.remove(i);
            }
            Ok(i) => self.chains[i].1 = stage,
            Err(i) if stage != 0 => self.chains.insert(i, (chain, stage)),
            Err(_) => {}
        }
    }
    fn cooldown_until(&self, event: u32) -> u64 {
        self.cooldowns.binary_search_by_key(&event, |x| x.0).map_or(0, |i| self.cooldowns[i].1)
    }
    fn set_cooldown(&mut self, event: u32, until: u64) {
        match self.cooldowns.binary_search_by_key(&event, |x| x.0) {
            Ok(i) => self.cooldowns[i].1 = until,
            Err(i) => self.cooldowns.insert(i, (event, until)),
        }
    }
    fn is_pending(&self, event: u32) -> bool {
        self.pending.binary_search_by_key(&event, |p| p.event).is_ok()
    }
    fn push_pending(&mut self, p: Pending) {
        if let Err(i) = self.pending.binary_search_by_key(&p.event, |x| x.event) {
            self.pending.insert(i, p);
        }
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.pending.len() as u32);
        for p in &self.pending {
            h.write_u32(p.event);
            h.write_u64(p.raised_tick);
        }
        h.write_u32(self.flags.len() as u32);
        for f in &self.flags {
            h.write_u16(f.0);
            h.write_u16(f.1);
        }
        h.write_u32(self.chains.len() as u32);
        for c in &self.chains {
            h.write_u16(c.0);
            h.write_u16(c.1);
        }
        h.write_u32(self.cooldowns.len() as u32);
        for c in &self.cooldowns {
            h.write_u32(c.0);
            h.write_u64(c.1);
        }
    }
}

/// Event definitions, sorted by ID. Held by the ruleset (data), not by the world.
#[derive(Clone, Debug, Default)]
pub struct EventSet {
    defs: Vec<EventDef>,
    /// How often the polled triggers are checked.
    pub poll_rate: Option<Rate>,
}

impl EventSet {
    /// Fails on duplicate IDs.
    pub fn new(mut defs: Vec<EventDef>, poll_rate: Rate) -> Result<EventSet, u32> {
        defs.sort_by_key(|d| d.id);
        if let Some(w) = defs.windows(2).find(|w| w[0].id == w[1].id) {
            return Err(w[0].id);
        }
        Ok(EventSet { defs, poll_rate: Some(poll_rate) })
    }
    pub fn def(&self, id: u32) -> Option<&EventDef> {
        self.defs.binary_search_by_key(&id, |d| d.id).ok().map(|i| &self.defs[i])
    }

    fn can_fire(&self, d: &EventDef, w: &World, now: u64) -> bool {
        let st = w.event_state();
        !st.is_pending(d.id)
            && now >= st.cooldown_until(d.id)
            && d.chain.is_none_or(|(chain, need, _)| st.chain_stage(chain) == need)
    }

    /// Check every polled trigger, in event ID order, against the state at the start of
    /// the pass, then raise the ones that hold.
    pub fn poll(&self, world: &mut World, signals: Signals) {
        let due = match self.poll_rate {
            Some(Rate::Hourly) => true,
            Some(Rate::Daily) => signals.new_day,
            Some(Rate::Monthly) => signals.new_month,
            Some(Rate::Yearly) => signals.new_year,
            None => false,
        };
        if !due {
            return;
        }
        let now = world.clock().tick();
        let firing: Vec<u32> = self.defs.iter().filter(|d| self.can_fire(d, world, now) && holds(&d.trigger, world)).map(|d| d.id).collect();
        for id in firing {
            self.raise_checked(world, id, now);
        }
    }

    /// Raise an event from a system or an effect, ignoring its trigger but not its chain
    /// gate, cooldown or pending state. Returns whether it was raised.
    pub fn raise(&self, world: &mut World, id: u32) -> bool {
        let now = world.clock().tick();
        match self.def(id) {
            Some(d) if self.can_fire(d, world, now) => self.raise_checked(world, id, now),
            _ => false,
        }
    }

    fn raise_checked(&self, world: &mut World, id: u32, now: u64) -> bool {
        let Some(d) = self.def(id) else { return false };
        let st = world.event_state_mut();
        st.push_pending(Pending { event: id, raised_tick: now });
        st.set_cooldown(id, now + d.cooldown);
        true
    }

    /// The country picks an option of a pending event: effects apply, the chain advances,
    /// the event is no longer pending. `Err` gives a fixed reason.
    pub fn choose(&self, world: &mut World, country: CountryId, event: u32, option: usize) -> Result<(), &'static str> {
        let d = self.def(event).ok_or("no such event")?;
        if d.country != country {
            return Err("this event is not yours");
        }
        let st = world.event_state();
        let i = st.pending.binary_search_by_key(&event, |p| p.event).map_err(|_| "event is not pending")?;
        let effects = d.options.get(option).ok_or("no such option")?;
        world.event_state_mut().pending.remove(i);
        for e in effects {
            apply_effect(e, world);
        }
        if let Some((chain, _, next)) = d.chain {
            world.event_state_mut().set_chain_stage(chain, next);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ProvinceId;

    fn def(id: u32, trigger: Condition, chain: Option<(u16, u16, u16)>) -> EventDef {
        EventDef {
            id,
            country: CountryId(0),
            trigger,
            options: vec![vec![Effect::AddValue { province: ProvinceId(0), amount: 10 }], vec![Effect::SetFlag { country: CountryId(0), flag: 7 }]],
            cooldown: 100,
            chain,
            severity: 1,
        }
    }
    const DAY: Signals = Signals { new_day: true, new_month: false, new_year: false };

    #[test]
    fn polled_triggers_raise_once_and_respect_cooldown() {
        let set = EventSet::new(vec![def(1, Condition::ValueAtLeast { province: ProvinceId(0), amount: 5 }, None)], Rate::Daily).unwrap();
        let mut w = World::new(1, 1, 1);
        set.poll(&mut w, DAY);
        assert!(w.event_state().pending().is_empty(), "trigger not met");
        w.values_mut()[0] = 5;
        set.poll(&mut w, Signals::default());
        assert!(w.event_state().pending().is_empty(), "daily poll does not run on an ordinary tick");
        set.poll(&mut w, DAY);
        assert_eq!(w.event_state().pending().len(), 1);
        set.poll(&mut w, DAY);
        assert_eq!(w.event_state().pending().len(), 1, "no duplicate while pending");
        assert_eq!(set.choose(&mut w, CountryId(0), 1, 0), Ok(()));
        assert_eq!(w.value(ProvinceId(0)), 15);
        set.poll(&mut w, DAY);
        assert!(w.event_state().pending().is_empty(), "on cooldown");
    }

    #[test]
    fn choosing_applies_effects_flags_and_advances_chains() {
        let set = EventSet::new(vec![def(1, Condition::Always, Some((3, 0, 1))), def(2, Condition::FlagSet { country: CountryId(0), flag: 7 }, Some((3, 1, 2)))], Rate::Daily).unwrap();
        let mut w = World::new(1, 1, 1);
        set.poll(&mut w, DAY);
        assert_eq!(w.event_state().pending().iter().map(|p| p.event).collect::<Vec<_>>(), vec![1], "event 2 waits for stage 1");
        assert_eq!(set.choose(&mut w, CountryId(0), 1, 1), Ok(()));
        assert!(w.event_state().flag(CountryId(0), 7));
        assert_eq!(w.event_state().chain_stage(3), 1);
        set.poll(&mut w, DAY);
        assert_eq!(w.event_state().pending().iter().map(|p| p.event).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn bad_choices_are_refused_without_changes() {
        let set = EventSet::new(vec![def(1, Condition::Always, None)], Rate::Daily).unwrap();
        let mut w = World::new(1, 2, 1);
        assert_eq!(set.choose(&mut w, CountryId(0), 1, 0), Err("event is not pending"));
        set.poll(&mut w, DAY);
        let before = w.state_hash();
        assert_eq!(set.choose(&mut w, CountryId(1), 1, 0), Err("this event is not yours"));
        assert_eq!(set.choose(&mut w, CountryId(0), 1, 9), Err("no such option"));
        assert_eq!(set.choose(&mut w, CountryId(0), 8, 0), Err("no such event"));
        assert_eq!(w.state_hash(), before);
        assert!(!set.raise(&mut w, 1), "already pending");
        assert_eq!(EventSet::new(vec![def(1, Condition::Always, None), def(1, Condition::Always, None)], Rate::Daily).err(), Some(1));
    }
}
