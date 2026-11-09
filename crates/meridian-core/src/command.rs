//! Commands are the only way to change the world from outside. They are
//! applied at the start of a tick. A rejected command changes nothing.

use crate::ids::{CellId, CountryId, ProvinceId, UnitId};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Command {
    /// Order a unit to a province. (Placeholder: teleports. Pathing is Phase 3.)
    MoveUnit { issuer: CountryId, unit: UnitId, to: ProvinceId },
    /// Change a province's controller. (Placeholder for occupation.)
    SetProvinceController { issuer: CountryId, province: ProvinceId, controller: CountryId },
    /// Start a construction project in a cell the issuer controls. Cost comes from the ruleset.
    StartConstruction { issuer: CountryId, region: u32, cell: CellId, kind: u16, level: u8 },
    /// Set (or with quantity 0 and priority 0, clear) a production order.
    SetProductionOrder { issuer: CountryId, model: u16, quantity: u32, priority: u8 },
    /// Raise the issuer's own war level against another country (up to the maximum).
    /// Lowering a level is a peace treaty's job, not a command.
    SetWarLevel { issuer: CountryId, against: CountryId, level: u8 },
    /// A hostile act that needs the issuer's war level against `against` to be at least
    /// `required_level`. The engine only checks the level; what the act does is a rule.
    HostileAct { issuer: CountryId, against: CountryId, required_level: u8 },
    /// Sign a proposed treaty. Major treaties also need every party's factions to approve.
    SignTreaty { issuer: CountryId, treaty: u32 },
    /// Pick an option of a pending event.
    ChooseEvent { issuer: CountryId, event: u32, option: u8 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    NoSuchUnit(UnitId),
    NoSuchProvince(ProvinceId),
    NoSuchCountry(CountryId),
    /// The issuer does not own the thing it is ordering.
    NotOwner { issuer: CountryId },
    NoSuchCell(CellId),
    /// Refused by a rule, with a fixed reason that never reveals hidden information.
    Refused(&'static str),
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rejection::NoSuchUnit(u) => write!(f, "unit {} does not exist", u.0),
            Rejection::NoSuchProvince(p) => write!(f, "province {} does not exist", p.0),
            Rejection::NoSuchCountry(c) => write!(f, "country {} does not exist", c.0),
            Rejection::NoSuchCell(c) => write!(f, "cell {} does not exist", c.0),
            Rejection::Refused(why) => write!(f, "refused: {why}"),
            Rejection::NotOwner { issuer } => write!(f, "country {} does not own the target", issuer.0),
        }
    }
}
