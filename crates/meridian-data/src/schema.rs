//! Authored-file schema. Kept separate from engine types so the file format
//! can change without touching the engine. Every file carries a version.

use serde::{Deserialize, Serialize};

pub const SCENARIO_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub version: u32,
    pub name: String,
    pub seed: u64,
    pub countries: Vec<CountryDef>,
    pub provinces: Vec<ProvinceDef>,
    #[serde(default)]
    pub units: Vec<UnitDef>,
    #[serde(default)]
    pub sites: Vec<SiteDef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountryDef {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvinceDef {
    pub id: u32,
    pub name: String,
    pub controller: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDef {
    pub id: u32,
    pub owner: u32,
    pub location: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SiteKindDef {
    City,
    Port,
    Airfield,
    Crossing,
    Industry,
    Base,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteDef {
    pub id: u32,
    pub name: String,
    pub kind: SiteKindDef,
    pub province: u32,
    pub owner: u32,
    /// 0 to 100.
    pub condition: u32,
    pub built_year: i32,
    pub destroyed_year: Option<i32>,
}
