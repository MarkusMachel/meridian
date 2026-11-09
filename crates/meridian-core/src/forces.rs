//! Military tables (Phase 4B, step 12): extra unit attributes, officers, pilots, hulls
//! and transit states. Only plain fields and stable IDs here; what the numbers mean
//! (ranks, statuses, experience thresholds) is defined by rules data.
//!
//! IDs are positions in append-only vectors and are never reused. A dead officer or a
//! sunk ship keeps its slot with a status field, so the history stays.
//!
//! Everything is stored in whole numbers: morale, cohesion and readiness are in
//! thousandths (0 to 1000), so no floats and no fixed-point needed.

use crate::hash::StateHasher;
use crate::ids::{CellId, CountryId, HullId, OfficerId, PilotId, UnitId};
use crate::modifiers::TraitSet;

/// Where a unit in transit will arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Destination {
    Cell(CellId),
    Province(crate::ids::ProvinceId),
}

/// A move in progress (by air, sea or other long hop): not a tracked flight or ship,
/// just a destination and an arrival tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Transit {
    pub destination: Destination,
    pub arrive_tick: u64,
}

/// A standing order held by a unit (or a headquarters). The engine validates and stores it;
/// rules systems carry it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UnitOrder {
    /// Hold at all costs: cannot retreat in order. The player can cancel it.
    Hold,
    /// Break off from a fight as a defender.
    Withdraw,
    Fortify { facing: CellId },
    JoinAssault { cell: CellId },
    AmphibiousAssault { cell: CellId },
    Bombard { target: CellId },
    AirMission { kind: u8, target: CellId },
    Launch { missile: u16, target: CellId, count: u32 },
    Mobilize,
    Demobilize,
    BuildBridge { a: CellId, b: CellId },
}

/// Attributes of a unit that live beside `Unit { owner, location }`, indexed by UnitId.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnitExt {
    /// Command chain (brigade -> division -> corps ...). The engine gives it no meaning.
    pub parent: Option<UnitId>,
    pub cell: Option<CellId>,
    /// Rules-defined status code (0 = normal).
    pub status: u8,
    /// Rules-defined stance code (0 = defend by convention).
    pub stance: u8,
    /// Thousandths, 0 to 1000.
    pub morale: u16,
    pub cohesion: u16,
    pub readiness: u16,
    pub veterancy: u8,
    pub commander: Option<OfficerId>,
    /// Days the unit has been encircled (no retreat cell). Rules decide what it means.
    pub encircled_days: u16,
    pub traits: TraitSet,
    pub transit: Option<Transit>,
    pub order: Option<UnitOrder>,
}

impl Default for UnitExt {
    fn default() -> UnitExt {
        UnitExt {
            parent: None,
            cell: None,
            status: 0,
            stance: 0,
            morale: 1000,
            cohesion: 1000,
            readiness: 1000,
            veterancy: 0,
            commander: None,
            encircled_days: 0,
            traits: TraitSet::new(),
            transit: None,
            order: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Officer {
    pub country: CountryId,
    /// Days before 1990-01-01 (so age is derived from the clock).
    pub birth_days_before_start: i32,
    pub rank: u8,
    pub experience: u16,
    /// 0 active, 1 wounded, 2 retired, 3 dead by convention; the engine does not care.
    pub status: u8,
    pub assignment: Option<UnitId>,
    pub traits: TraitSet,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Pilot {
    pub country: CountryId,
    pub wing: Option<UnitId>,
    pub birth_days_before_start: i32,
    pub rank: u8,
    pub experience: u16,
    pub kills: u16,
    pub status: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hull {
    /// Catalog model (index into rules data).
    pub model: u16,
    pub owner: CountryId,
    /// Sea zone or port node ID in the strategic graph.
    pub location: u32,
    /// Task force this hull belongs to.
    pub parent: Option<UnitId>,
    /// 0 (sunk) to 1000.
    pub condition: u16,
    pub fuel: u32,
    /// 0 active, 1 in repair, 2 sunk, 3 decommissioned by convention.
    pub status: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Forces {
    pub officers: Vec<Officer>,
    pub pilots: Vec<Pilot>,
    pub hulls: Vec<Hull>,
}

impl Forces {
    pub fn add_officer(&mut self, o: Officer) -> OfficerId {
        self.officers.push(o);
        OfficerId(self.officers.len() as u32 - 1)
    }
    pub fn add_pilot(&mut self, p: Pilot) -> PilotId {
        self.pilots.push(p);
        PilotId(self.pilots.len() as u32 - 1)
    }
    pub fn add_hull(&mut self, h: Hull) -> HullId {
        self.hulls.push(h);
        HullId(self.hulls.len() as u32 - 1)
    }
    pub fn is_empty(&self) -> bool {
        self.officers.is_empty() && self.pilots.is_empty() && self.hulls.is_empty()
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.officers.len() as u32);
        for o in &self.officers {
            h.write_u16(o.country.0);
            h.write_bytes(&o.birth_days_before_start.to_le_bytes());
            h.write_u8(o.rank);
            h.write_u16(o.experience);
            h.write_u8(o.status);
            write_opt_unit(h, o.assignment);
            write_traits(h, &o.traits);
        }
        h.write_u32(self.pilots.len() as u32);
        for p in &self.pilots {
            h.write_u16(p.country.0);
            write_opt_unit(h, p.wing);
            h.write_bytes(&p.birth_days_before_start.to_le_bytes());
            h.write_u8(p.rank);
            h.write_u16(p.experience);
            h.write_u16(p.kills);
            h.write_u8(p.status);
        }
        h.write_u32(self.hulls.len() as u32);
        for x in &self.hulls {
            h.write_u16(x.model);
            h.write_u16(x.owner.0);
            h.write_u32(x.location);
            write_opt_unit(h, x.parent);
            h.write_u16(x.condition);
            h.write_u32(x.fuel);
            h.write_u8(x.status);
        }
    }
}

fn write_opt_unit(h: &mut StateHasher, u: Option<UnitId>) {
    match u {
        None => h.write_u8(0),
        Some(u) => {
            h.write_u8(1);
            h.write_u32(u.0);
        }
    }
}

fn write_traits(h: &mut StateHasher, t: &TraitSet) {
    h.write_u16(t.len() as u16);
    for id in t.iter() {
        h.write_u16(id);
    }
}

impl UnitExt {
    pub fn hash_into(&self, h: &mut StateHasher) {
        write_opt_unit(h, self.parent);
        match self.cell {
            None => h.write_u8(0),
            Some(c) => {
                h.write_u8(1);
                h.write_u32(c.0);
            }
        }
        h.write_u8(self.status);
        h.write_u8(self.stance);
        h.write_u16(self.morale);
        h.write_u16(self.cohesion);
        h.write_u16(self.readiness);
        h.write_u8(self.veterancy);
        match self.commander {
            None => h.write_u8(0),
            Some(o) => {
                h.write_u8(1);
                h.write_u32(o.0);
            }
        }
        h.write_u16(self.encircled_days);
        write_traits(h, &self.traits);
        match self.order {
            None => h.write_u8(0),
            Some(o) => {
                h.write_u8(1);
                match o {
                    UnitOrder::Hold => h.write_u8(0),
                    UnitOrder::Withdraw => h.write_u8(1),
                    UnitOrder::Fortify { facing } => {
                        h.write_u8(2);
                        h.write_u32(facing.0);
                    }
                    UnitOrder::JoinAssault { cell } => {
                        h.write_u8(3);
                        h.write_u32(cell.0);
                    }
                    UnitOrder::AmphibiousAssault { cell } => {
                        h.write_u8(4);
                        h.write_u32(cell.0);
                    }
                    UnitOrder::Bombard { target } => {
                        h.write_u8(5);
                        h.write_u32(target.0);
                    }
                    UnitOrder::AirMission { kind, target } => {
                        h.write_u8(6);
                        h.write_u8(kind);
                        h.write_u32(target.0);
                    }
                    UnitOrder::Launch { missile, target, count } => {
                        h.write_u8(7);
                        h.write_u16(missile);
                        h.write_u32(target.0);
                        h.write_u32(count);
                    }
                    UnitOrder::Mobilize => h.write_u8(8),
                    UnitOrder::Demobilize => h.write_u8(9),
                    UnitOrder::BuildBridge { a, b } => {
                        h.write_u8(10);
                        h.write_u32(a.0);
                        h.write_u32(b.0);
                    }
                }
            }
        }
        match self.transit {
            None => h.write_u8(0),
            Some(t) => {
                h.write_u8(1);
                match t.destination {
                    Destination::Cell(c) => {
                        h.write_u8(0);
                        h.write_u32(c.0);
                    }
                    Destination::Province(p) => {
                        h.write_u8(1);
                        h.write_u32(p.0);
                    }
                }
                h.write_u64(t.arrive_tick);
            }
        }
    }
}
