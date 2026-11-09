//! Meridian simulation core. Deterministic, headless, no I/O.
//!
//! Rules (see ENGINE_BUILD_GUIDE.md section 1.2): no floats in simulation
//! state, one seeded RNG, no hash-map iteration, commands applied at the
//! start of a tick.

pub mod active;
pub mod cells;
pub mod combat;
pub mod clock;
pub mod command;
pub mod derived;
pub mod fixed;
pub mod flows;
pub mod forces;
pub mod hash;
pub mod hooks;
pub mod map;
pub mod modifiers;
pub mod parallel;
pub mod ids;
pub mod rng;
pub mod schedule;
pub mod world;

pub use active::ActiveSet;
pub use cells::CellState;
pub use clock::{Clock, Date, Signals};
pub use command::{Command, Rejection};
pub use fixed::Fixed;
pub use combat::{garrison_size, Bridge, CombatState, DoctrineState, Engagement, PrisonerPool, StartSnapshot};
pub use derived::{front_segments, CellControl, FrontSegment, Projection, UnitsByCell, ZocCache};
pub use flows::{apply_flows, propagate_supply, split_shortage, CostGraph, Flow, Source, SupplyResult};
pub use forces::{Destination, Forces, Hull, Officer, Pilot, Transit, UnitExt};
pub use hooks::{Condition, Effect, HookError, HookSet, Rule};
pub use ids::{CellId, CountryId, HullId, OfficerId, PilotId, ProvinceId, SiteId, UnitId};
pub use map::{CellCosts, Map, NodeCosts, Path, PathScratch, RawMap};
pub use modifiers::{apply_modifiers, CategoryRules, Modifier, Scope, TraitDef, TraitSet, TraitTable};
pub use rng::Rng;
pub use schedule::{Rate, Schedule, ScheduleBuilder, ScheduleError, System, Timing};
pub use world::{Event, MapRef, Ruleset, Site, SiteKind, TickReport, Unit, World};
