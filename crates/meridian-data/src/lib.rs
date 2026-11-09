#![allow(clippy::chunks_exact_to_as_chunks)] // chunks_exact reads more plainly than as_chunks

//! Data schemas, loader, validator and baker. Reads files; hands the
//! simulation ready-made world state. The simulation never touches files.

pub mod autosave;
pub mod bake;
pub mod error;
pub mod load;
pub mod map_file;
pub mod render_file;
pub mod replay;
pub mod save;
pub mod schema;

pub use autosave::Autosave;
pub use error::{DataError, ErrorKind};
pub use map_file::{load_map_file, parse_map};
pub use render_file::{load_render_file, parse_render, LineKind, RenderData, RenderLevel};
pub use load::{load_scenario_file, load_scenario_str, validate_scenario_file, validate_scenario_str};
pub use save::{load_world, save_world, SAVE_VERSION};
pub use schema::Scenario;
