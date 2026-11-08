//! Meridian simulation core. Deterministic, headless, no I/O.
//!
//! Rules (see ENGINE_BUILD_GUIDE.md section 1.2): no floats in simulation
//! state, one seeded RNG, no hash-map iteration, commands applied at the
//! start of a tick.

pub mod clock;
pub mod command;
pub mod fixed;
pub mod hash;
pub mod ids;
pub mod rng;
pub mod world;

pub use clock::{Clock, Date, Signals};
pub use command::{Command, Rejection};
pub use fixed::Fixed;
pub use ids::{CountryId, ProvinceId, UnitId};
pub use rng::Rng;
pub use world::{Ruleset, Site, SiteKind, TickReport, Unit, World};
