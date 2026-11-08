//! Commands are the only way to change the world from outside. They are
//! applied at the start of a tick. A rejected command changes nothing.

use crate::ids::{CountryId, ProvinceId, UnitId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Order a unit to a province. (Placeholder: teleports. Pathing is Phase 3.)
    MoveUnit { issuer: CountryId, unit: UnitId, to: ProvinceId },
    /// Change a province's controller. (Placeholder for occupation.)
    SetProvinceController { issuer: CountryId, province: ProvinceId, controller: CountryId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    NoSuchUnit(UnitId),
    NoSuchProvince(ProvinceId),
    NoSuchCountry(CountryId),
    /// The issuer does not own the thing it is ordering.
    NotOwner { issuer: CountryId },
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rejection::NoSuchUnit(u) => write!(f, "unit {} does not exist", u.0),
            Rejection::NoSuchProvince(p) => write!(f, "province {} does not exist", p.0),
            Rejection::NoSuchCountry(c) => write!(f, "country {} does not exist", c.0),
            Rejection::NotOwner { issuer } => write!(f, "country {} does not own the target", issuer.0),
        }
    }
}
