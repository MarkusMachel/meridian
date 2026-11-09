#![allow(clippy::chunks_exact_to_as_chunks)] // chunks_exact reads more plainly than as_chunks
//! Client shell (Phase 7). Everything here except `render` and the binary is plain logic with
//! tests and no graphics: a camera, a picker, map-mode colours and a session that drives the
//! simulation thread. Floats are fine here because nothing in this crate feeds the simulation.

pub mod camera;
pub mod modes;
pub mod offscreen;
pub mod picker;
pub mod render;
pub mod session;

pub use camera::Camera;
pub use modes::{Mode, Rgba};
pub use picker::Picker;
pub use session::{Session, Snapshot, Speed};
