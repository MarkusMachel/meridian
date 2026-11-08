//! Data schemas, loader, validator and baker. Reads files; hands the
//! simulation ready-made world state. The simulation never touches files.

pub mod bake;
pub mod error;
pub mod load;
pub mod map_file;
pub mod schema;

pub use error::{DataError, ErrorKind};
pub use map_file::{load_map_file, parse_map};
pub use load::{load_scenario_file, load_scenario_str, validate_scenario_file, validate_scenario_str};
pub use schema::Scenario;
