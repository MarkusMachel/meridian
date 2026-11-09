//! Dynamic per-cell state (Phase 4B, step 11). The map file holds the static facts
//! about each cell; this holds what changes during play. Dense arrays indexed by
//! cell ID, one byte or so per field, all hashed.
//!
//! Population is not here: it is stored as group records (design notes section 14),
//! added in Phase 4C.

use crate::active::ActiveSet;
use crate::hash::StateHasher;
use crate::ids::CountryId;

/// `last_fought` value meaning "never".
pub const NEVER_FOUGHT: u32 = u32::MAX;

/// Hazard flag bits (meaning of the bits beyond these is up to the rules).
pub mod hazard {
    pub const MINES: u8 = 1;
    pub const FALLOUT: u8 = 2;
    pub const FIRE: u8 = 4;
    pub const FLOOD: u8 = 8;
}

#[derive(Clone, Debug)]
pub struct CellState {
    controller: Vec<u16>,
    /// 0 (none) to 255 (flattened).
    damage: Vec<u8>,
    hazards: Vec<u8>,
    /// Tick of the last fight in the cell, or `NEVER_FOUGHT`.
    last_fought: Vec<u32>,
    hunger: Vec<u8>,
    /// Remaining local food, 0 to 255.
    food: Vec<u8>,
    /// Mud, snow depth or frozen ground; meaning set by rules.
    ground: Vec<u8>,
    /// Provinces currently simulated cell by cell. Sorted.
    pub opened: ActiveSet,
    /// Cells changed since the last `take_dirty`. Diagnostic for the read interface; not state.
    dirty: Vec<u32>,
}

impl PartialEq for CellState {
    fn eq(&self, o: &CellState) -> bool {
        self.controller == o.controller
            && self.damage == o.damage
            && self.hazards == o.hazards
            && self.last_fought == o.last_fought
            && self.hunger == o.hunger
            && self.food == o.food
            && self.ground == o.ground
            && self.opened == o.opened
    }
}
impl Eq for CellState {}

/// One cell whose state differs from the starting state.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CellDiff {
    pub cell: u32,
    pub controller: u16,
    pub damage: u8,
    pub hazards: u8,
    pub last_fought: u32,
    pub hunger: u8,
    pub food: u8,
    pub ground: u8,
}

/// Per-cell state saved as differences from the starting state of the map.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CellChanges {
    /// Ascending by cell.
    pub cells: Vec<CellDiff>,
    pub opened: Vec<u32>,
}

impl CellState {
    fn row(&self, i: usize) -> CellDiff {
        CellDiff {
            cell: i as u32,
            controller: self.controller[i],
            damage: self.damage[i],
            hazards: self.hazards[i],
            last_fought: self.last_fought[i],
            hunger: self.hunger[i],
            food: self.food[i],
            ground: self.ground[i],
        }
    }

    /// The cells that differ from `base` (the state a fresh map world starts with).
    pub fn changes_from(&self, base: &CellState) -> CellChanges {
        let cells = (0..self.len()).filter(|&i| self.row(i) != base.row(i)).map(|i| self.row(i)).collect();
        CellChanges { cells, opened: self.opened.iter().collect() }
    }

    /// Apply saved differences on top of a fresh state. Fails if a cell is out of range
    /// or the list is not strictly ascending.
    pub fn apply_changes(&mut self, ch: &CellChanges) -> Result<(), String> {
        let mut last: Option<u32> = None;
        for d in &ch.cells {
            if d.cell as usize >= self.len() || last.is_some_and(|l| d.cell <= l) {
                return Err(format!("cell change for cell {} is out of range or out of order", d.cell));
            }
            last = Some(d.cell);
            let i = d.cell as usize;
            self.controller[i] = d.controller;
            self.damage[i] = d.damage;
            self.hazards[i] = d.hazards;
            self.last_fought[i] = d.last_fought;
            self.hunger[i] = d.hunger;
            self.food[i] = d.food;
            self.ground[i] = d.ground;
        }
        self.opened = ActiveSet::new();
        self.opened.extend(&ch.opened);
        Ok(())
    }
}

impl CellState {
    /// All cells start controlled by `controller[cell]`.
    pub fn new(controller: Vec<u16>) -> CellState {
        let n = controller.len();
        CellState {
            controller,
            damage: vec![0; n],
            hazards: vec![0; n],
            last_fought: vec![NEVER_FOUGHT; n],
            hunger: vec![0; n],
            food: vec![255; n],
            ground: vec![0; n],
            opened: ActiveSet::new(),
            dirty: Vec::new(),
        }
    }

    pub fn controller(&self, i: usize) -> u16 {
        self.controller[i]
    }
    /// All controllers, for bulk scans (front detection).
    pub fn controllers(&self) -> &[u16] {
        &self.controller
    }
    pub fn damage(&self, i: usize) -> u8 {
        self.damage[i]
    }
    pub fn hazards(&self, i: usize) -> u8 {
        self.hazards[i]
    }
    pub fn last_fought(&self, i: usize) -> u32 {
        self.last_fought[i]
    }
    pub fn hunger(&self, i: usize) -> u8 {
        self.hunger[i]
    }
    pub fn food(&self, i: usize) -> u8 {
        self.food[i]
    }
    pub fn ground(&self, i: usize) -> u8 {
        self.ground[i]
    }
    pub fn set_controller(&mut self, i: usize, v: u16) {
        if self.controller[i] != v {
            self.controller[i] = v;
            self.dirty.push(i as u32);
        }
    }
    pub fn set_damage(&mut self, i: usize, v: u8) {
        if self.damage[i] != v {
            self.damage[i] = v;
            self.dirty.push(i as u32);
        }
    }
    pub fn set_hazards(&mut self, i: usize, v: u8) {
        if self.hazards[i] != v {
            self.hazards[i] = v;
            self.dirty.push(i as u32);
        }
    }
    pub fn set_last_fought(&mut self, i: usize, v: u32) {
        if self.last_fought[i] != v {
            self.last_fought[i] = v;
            self.dirty.push(i as u32);
        }
    }
    pub fn set_hunger(&mut self, i: usize, v: u8) {
        if self.hunger[i] != v {
            self.hunger[i] = v;
            self.dirty.push(i as u32);
        }
    }
    pub fn set_food(&mut self, i: usize, v: u8) {
        if self.food[i] != v {
            self.food[i] = v;
            self.dirty.push(i as u32);
        }
    }
    pub fn set_ground(&mut self, i: usize, v: u8) {
        if self.ground[i] != v {
            self.ground[i] = v;
            self.dirty.push(i as u32);
        }
    }

    /// The cells changed since the last call, ascending and without repeats. Clears the list.
    pub fn take_dirty(&mut self) -> Vec<u32> {
        let mut d = std::mem::take(&mut self.dirty);
        d.sort_unstable();
        d.dedup();
        d
    }

    pub fn len(&self) -> usize {
        self.controller.len()
    }
    pub fn is_empty(&self) -> bool {
        self.controller.is_empty()
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.controller.len() as u32);
        for i in 0..self.controller.len() {
            h.write_u16(self.controller[i]);
            h.write_u8(self.damage[i]);
            h.write_u8(self.hazards[i]);
            h.write_u32(self.last_fought[i]);
            h.write_u8(self.hunger[i]);
            h.write_u8(self.food[i]);
            h.write_u8(self.ground[i]);
        }
        self.opened.hash_into(h);
    }

    /// The country controlling most of these cells; ties go to the lower country ID.
    pub fn majority_controller(&self, cells: &[u32]) -> Option<CountryId> {
        let mut owners: Vec<u16> = cells.iter().map(|&c| self.controller[c as usize]).collect();
        owners.sort_unstable();
        let (mut best, mut best_n) = (None, 0usize);
        let mut i = 0;
        while i < owners.len() {
            let j = owners[i..].iter().take_while(|&&o| o == owners[i]).count();
            if j > best_n {
                best = Some(CountryId(owners[i]));
                best_n = j;
            }
            i += j;
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use crate::ids::{CellId, CountryId, ProvinceId};
    use crate::map::tests::test_map;
    use crate::map::Map;
    use crate::world::World;
    use std::sync::Arc;

    fn world() -> World {
        World::from_map(1, Arc::new(Map::from_raw(test_map(16, 16)).unwrap()))
    }

    #[test]
    fn cells_start_under_their_province_controller_and_are_hashed() {
        let mut w = world();
        let base = w.state_hash();
        assert_eq!(w.cells().unwrap().len(), 256);
        w.cells_mut().unwrap().damage[10] = 3;
        assert_ne!(w.state_hash(), base, "damage not hashed");
        for edit in [
            |c: &mut super::CellState| c.hazards[0] = 1,
            |c: &mut super::CellState| c.last_fought[0] = 5,
            |c: &mut super::CellState| c.hunger[0] = 5,
            |c: &mut super::CellState| c.food[0] = 5,
            |c: &mut super::CellState| c.ground[0] = 5,
            |c: &mut super::CellState| c.controller[0] = 1,
        ] {
            let mut v = world();
            edit(v.cells_mut().unwrap());
            assert_ne!(v.state_hash(), base);
        }
    }

    #[test]
    fn open_and_close_follow_the_majority() {
        let mut w = world();
        let p = ProvinceId(0);
        let cells: Vec<u32> = w.map().unwrap().cells_of(p).to_vec();
        assert!(w.open_province(p));
        assert!(!w.open_province(p), "already open");
        // country 1 takes 10 of 16 cells
        let c = w.cells_mut().unwrap();
        for &cell in &cells[..10] {
            c.controller[cell as usize] = 1;
        }
        assert_eq!(c.majority_controller(&cells), Some(CountryId(1)));
        assert!(w.close_province(p));
        assert!(!w.close_province(p), "already closed");
        assert_eq!(w.controller(p), CountryId(1));
        assert!(w.cells().unwrap().controller.iter().enumerate().all(|(i, &c)| {
            let prov = w.map().unwrap().cell_province[i];
            c == w.controller(ProvinceId(prov)).0 || prov != 0
        }));
        assert!(w.cells().unwrap().opened.is_empty());
        let _ = CellId(0);
    }

    #[test]
    fn tie_goes_to_the_lower_country_id() {
        let mut w = world();
        let p = ProvinceId(0);
        let cells: Vec<u32> = w.map().unwrap().cells_of(p).to_vec();
        w.open_province(p);
        let c = w.cells_mut().unwrap();
        for &cell in &cells[..8] {
            c.controller[cell as usize] = 1;
        }
        for &cell in &cells[8..] {
            c.controller[cell as usize] = 0;
        }
        w.close_province(p);
        assert_eq!(w.controller(p), CountryId(0));
    }
}
