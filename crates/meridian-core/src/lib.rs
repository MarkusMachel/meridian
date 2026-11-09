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
pub mod econ;
pub mod events;
pub mod fixed;
pub mod flows;
pub mod forces;
pub mod hash;
pub mod hooks;
pub mod map;
pub mod intel;
pub mod modifiers;
pub mod parallel;
pub mod ids;
pub mod polity;
pub mod rng;
pub mod schedule;
pub mod tech;
pub mod view;
pub mod world;

pub use active::ActiveSet;
pub use cells::CellState;
pub use clock::{Clock, Date, Signals};
pub use command::{Command, Rejection};
pub use fixed::Fixed;
pub use combat::{garrison_size, Bridge, CombatState, DoctrineState, Engagement, PrisonerPool, StartSnapshot};
pub use derived::{front_segments, CellControl, FrontSegment, Projection, UnitsByCell, ZocCache};
pub use econ::{apply_recipe, banded_price, staffing_permille, Building, Economy, Finance, PopGroup, PopTable, ProductionOrder, Project, ProjectSpec, ProjectTemplate, Recipe, TradeRoute};
pub use events::{EventDef, EventSet, EventState, Pending};
pub use flows::{apply_flows, propagate_supply, split_shortage, CostGraph, Flow, Source, SupplyResult};
pub use forces::{Destination, Forces, Hull, Officer, Pilot, Transit, UnitExt};
pub use hooks::{Condition, Effect, HookError, HookSet, Rule};
pub use ids::{CellId, CountryId, HullId, OfficerId, PilotId, ProvinceId, SiteId, UnitId};
pub use map::{CellCosts, Map, NodeCosts, Path, PathScratch, RawMap};
pub use intel::{estimate_range, Agency, Contact, Intel, Network, Operation};
pub use modifiers::{apply_modifiers, CategoryRules, Modifier, Scope, TraitDef, TraitSet, TraitTable};
pub use polity::{Alliance, AllianceCall, CountryPolity, Decision, Diplomacy, Faction, Movement, Polity, Treaty, Vote, War, MAX_WAR_LEVEL};
pub use rng::Rng;
pub use schedule::{Rate, Schedule, ScheduleBuilder, ScheduleError, System, Timing};
pub use tech::{Program, Tech};
pub use view::{ContactId, CountryView, KnownUnit};
pub use world::{Event, MapRef, Ruleset, Site, SiteKind, TickReport, Unit, World};
