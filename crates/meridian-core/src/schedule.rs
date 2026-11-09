//! System scheduler (Phase 4A, steps 1 to 3 and 7).
//!
//! A *system* is a function over the world with a name, a rate (how often it runs)
//! and a list of systems it must run after. The `ScheduleBuilder` sorts them once at
//! startup. The result depends only on names and declared dependencies, never on the
//! order you registered them in, so the same systems give the same hash.
//!
//! Rust note for C# readers: `fn(&mut World, Signals)` is a plain function pointer,
//! like a static method group. Systems keep no state of their own; all state is in
//! the `World`, so the state hash sees it.

use crate::clock::Signals;
use crate::hooks::HookSet;
use crate::world::{Event, Ruleset, World};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// How often a system runs, driven by the clock's rollover signals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    Hourly,
    Daily,
    Monthly,
    Yearly,
}

impl Rate {
    fn due(self, s: Signals) -> bool {
        match self {
            Rate::Hourly => true,
            Rate::Daily => s.new_day,
            Rate::Monthly => s.new_month,
            Rate::Yearly => s.new_year,
        }
    }
}

pub type SystemFn = fn(&mut World, Signals);
pub type EventFn = fn(&mut World, &Event);

/// A registered system. `after` names systems that must run earlier in the same tick.
#[derive(Clone, Debug)]
pub struct System {
    pub name: &'static str,
    pub rate: Rate,
    pub after: &'static [&'static str],
    pub run: SystemFn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    DuplicateName(&'static str),
    DuplicateHandler(u16),
    UnknownDependency { system: &'static str, missing: &'static str },
    /// The systems still unsorted when no progress was possible, in name order.
    Cycle(Vec<&'static str>),
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::DuplicateName(n) => write!(f, "system '{n}' registered twice"),
            ScheduleError::DuplicateHandler(k) => write!(f, "two handlers for event kind {k}"),
            ScheduleError::UnknownDependency { system, missing } => {
                write!(f, "system '{system}' runs after '{missing}', which is not registered")
            }
            ScheduleError::Cycle(names) => write!(f, "ordering cycle among systems: {}", names.join(", ")),
        }
    }
}

#[derive(Default)]
pub struct ScheduleBuilder {
    systems: Vec<System>,
    handlers: Vec<(u16, EventFn)>,
    hooks: HookSet,
}

impl ScheduleBuilder {
    pub fn new() -> ScheduleBuilder {
        ScheduleBuilder::default()
    }

    pub fn system(mut self, system: System) -> ScheduleBuilder {
        self.systems.push(system);
        self
    }

    /// Data-driven rules, run after all systems each tick.
    pub fn hooks(mut self, hooks: HookSet) -> ScheduleBuilder {
        self.hooks = hooks;
        self
    }

    /// Handle events of one kind. At most one handler per kind.
    pub fn on_event(mut self, kind: u16, handler: EventFn) -> ScheduleBuilder {
        self.handlers.push((kind, handler));
        self
    }

    /// Sort into a fixed order. Among systems that are ready at the same time the
    /// one with the smaller name goes first, so the result never depends on the order
    /// of `add` calls.
    pub fn build(self) -> Result<Schedule, ScheduleError> {
        let mut handlers = self.handlers;
        handlers.sort_by_key(|h| h.0);
        if let Some(w) = handlers.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(ScheduleError::DuplicateHandler(w[0].0));
        }
        let mut systems = self.systems;
        systems.sort_by(|a, b| a.name.cmp(b.name));
        for w in systems.windows(2) {
            if w[0].name == w[1].name {
                return Err(ScheduleError::DuplicateName(w[0].name));
            }
        }
        // Dependency indexes into the name-sorted list.
        let mut deps: Vec<Vec<usize>> = Vec::with_capacity(systems.len());
        for s in &systems {
            let mut d = Vec::new();
            for &dep in s.after {
                match systems.binary_search_by(|x| x.name.cmp(dep)) {
                    Ok(i) => d.push(i),
                    Err(_) => return Err(ScheduleError::UnknownDependency { system: s.name, missing: dep }),
                }
            }
            deps.push(d);
        }
        // Repeatedly take the first (smallest-name) system whose dependencies are done.
        let n = systems.len();
        let mut done = vec![false; n];
        let mut order = Vec::with_capacity(n);
        while order.len() < n {
            let next = (0..n).find(|&i| !done[i] && deps[i].iter().all(|&d| done[d]));
            match next {
                Some(i) => {
                    done[i] = true;
                    order.push(i);
                }
                None => {
                    let stuck = (0..n).filter(|&i| !done[i]).map(|i| systems[i].name).collect();
                    return Err(ScheduleError::Cycle(stuck));
                }
            }
        }
        let ordered: Vec<System> = order.iter().map(|&i| systems[i].clone()).collect();
        let nanos = (0..ordered.len()).map(|_| AtomicU64::new(0)).collect();
        let runs = (0..ordered.len()).map(|_| AtomicU64::new(0)).collect();
        Ok(Schedule { systems: ordered, handlers, hooks: self.hooks, nanos, runs })
    }
}

/// A sorted list of systems. Implements `Ruleset`, so `World::tick` runs it directly.
pub struct Schedule {
    systems: Vec<System>,
    /// Sorted by event kind.
    handlers: Vec<(u16, EventFn)>,
    hooks: HookSet,
    /// Diagnostics only: total time and run count per system. Never read by the
    /// simulation and never hashed.
    nanos: Vec<AtomicU64>,
    runs: Vec<AtomicU64>,
}

/// One line of the timing report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timing {
    pub name: &'static str,
    pub runs: u64,
    pub total_ns: u64,
}

impl Schedule {
    /// Names in the order systems run each tick.
    pub fn order(&self) -> Vec<&'static str> {
        self.systems.iter().map(|s| s.name).collect()
    }

    /// Per-system cost since the schedule was built, in running order.
    pub fn timing_report(&self) -> Vec<Timing> {
        self.systems
            .iter()
            .enumerate()
            .map(|(i, s)| Timing {
                name: s.name,
                runs: self.runs[i].load(Ordering::Relaxed),
                total_ns: self.nanos[i].load(Ordering::Relaxed),
            })
            .collect()
    }
}

impl Ruleset for Schedule {
    fn run(&self, world: &mut World, signals: Signals) {
        for (i, s) in self.systems.iter().enumerate() {
            if !s.rate.due(signals) {
                continue;
            }
            // The clock reading goes only into the diagnostics counters.
            let t = Instant::now();
            (s.run)(world, signals);
            self.nanos[i].fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            self.runs[i].fetch_add(1, Ordering::Relaxed);
        }
        self.hooks.run(world, signals);
    }

    fn on_event(&self, world: &mut World, event: &Event) {
        if let Ok(i) = self.handlers.binary_search_by_key(&event.kind, |h| h.0) {
            (self.handlers[i].1)(world, event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nop(_: &mut World, _: Signals) {}
    fn sys(name: &'static str, after: &'static [&'static str]) -> System {
        System { name, rate: Rate::Hourly, after, run: nop }
    }

    #[test]
    fn order_ignores_registration_order() {
        let a = ScheduleBuilder::new().system(sys("c", &["b"])).system(sys("a", &[])).system(sys("b", &["a"])).build().unwrap();
        let b = ScheduleBuilder::new().system(sys("b", &["a"])).system(sys("a", &[])).system(sys("c", &["b"])).build().unwrap();
        assert_eq!(a.order(), vec!["a", "b", "c"]);
        assert_eq!(a.order(), b.order());
    }

    #[test]
    fn independent_systems_go_by_name() {
        let s = ScheduleBuilder::new().system(sys("z", &[])).system(sys("m", &[])).build().unwrap();
        assert_eq!(s.order(), vec!["m", "z"]);
    }

    #[test]
    fn cycle_unknown_and_duplicate_are_errors() {
        let e = ScheduleBuilder::new().system(sys("a", &["b"])).system(sys("b", &["a"])).build().err().unwrap();
        assert_eq!(e, ScheduleError::Cycle(vec!["a", "b"]));
        let e = ScheduleBuilder::new().system(sys("a", &["nope"])).build().err().unwrap();
        assert!(matches!(e, ScheduleError::UnknownDependency { .. }));
        let e = ScheduleBuilder::new().system(sys("a", &[])).system(sys("a", &[])).build().err().unwrap();
        assert_eq!(e, ScheduleError::DuplicateName("a"));
    }

    // Event kind 1 adds `amount` to province `target`.
    fn apply_damage(w: &mut World, e: &Event) {
        w.values_mut()[e.target as usize] += e.amount;
    }
    fn halve(w: &mut World, e: &Event) {
        w.values_mut()[e.target as usize] /= 2;
        let _ = e;
    }
    fn raiser(w: &mut World, _: Signals) {
        w.raise_event(Event { kind: 1, target: 0, source: 9, amount: 10 });
        w.raise_event(Event { kind: 2, target: 0, source: 5, amount: 0 });
    }

    #[test]
    fn events_run_after_systems_in_target_source_order() {
        let build = || {
            ScheduleBuilder::new()
                .system(System { name: "raiser", rate: Rate::Hourly, after: &[], run: raiser })
                .on_event(1, apply_damage)
                .on_event(2, halve)
                .build()
                .unwrap()
        };
        let mut w = World::new(1, 1, 1);
        w.tick(&[], &build());
        // source 5 (halve 0 -> 0) runs before source 9 (add 10): value 10, not 5.
        assert_eq!(w.value(crate::ids::ProvinceId(0)), 10);
        assert!(w.pending_events().is_empty());
        let e = ScheduleBuilder::new().on_event(1, apply_damage).on_event(1, halve).build().err().unwrap();
        assert_eq!(e, ScheduleError::DuplicateHandler(1));
    }

    fn add_one(w: &mut World, _: Signals) {
        w.values_mut()[0] += 1;
    }
    fn double(w: &mut World, _: Signals) {
        w.values_mut()[0] *= 2;
    }

    #[test]
    fn rates_follow_the_clock_and_order_matters() {
        let s = ScheduleBuilder::new()
            .system(System { name: "double", rate: Rate::Daily, after: &["add"], run: double })
            .system(System { name: "add", rate: Rate::Daily, after: &[], run: add_one })
            .build()
            .unwrap();
        let mut w = World::new(1, 1, 1);
        for _ in 0..48 {
            w.tick(&[], &s);
        }
        // two days: ((0+1)*2 + 1) * 2 = 6
        assert_eq!(w.value(crate::ids::ProvinceId(0)), 6);
        let report = s.timing_report();
        assert_eq!(report[0].name, "add");
        assert_eq!(report[0].runs, 2);
    }
}
