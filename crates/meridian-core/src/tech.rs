//! Technology plumbing (Phase 4E, step 19). Per-country field levels, development
//! programs that unlock specific models, and a generic prerequisite check. Which fields,
//! models, costs and caps exist is rules data.

use crate::hash::StateHasher;
use crate::ids::CountryId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub id: u32,
    pub country: CountryId,
    pub model: u16,
    pub cost: u32,
    pub progress: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tech {
    fields: u8,
    /// `[country * fields + field]`, 0 to 100.
    levels: Vec<u8>,
    programs: Vec<Program>,
    next_program: u32,
    /// (country, model) pairs the country can build, sorted.
    unlocked: Vec<(u16, u16)>,
}

impl Tech {
    pub fn is_empty(&self) -> bool {
        *self == Tech::default()
    }

    /// Declare how many fields exist. Existing levels are cleared.
    pub fn configure(&mut self, fields: u8) {
        self.fields = fields;
        self.levels.clear();
    }

    pub fn level(&self, c: CountryId, field: u8) -> u8 {
        self.levels.get(c.index() * self.fields as usize + field as usize).copied().unwrap_or(0)
    }
    pub fn set_level(&mut self, c: CountryId, field: u8, level: u8) {
        assert!(field < self.fields, "field out of range");
        let need = (c.index() + 1) * self.fields as usize;
        if self.levels.len() < need {
            self.levels.resize(need, 0);
        }
        self.levels[c.index() * self.fields as usize + field as usize] = level.min(100);
    }

    /// Learning by doing: raise a field, but never above `cap` (the diffusion limit the
    /// rules compute from what other countries already know). Returns the new level.
    pub fn raise(&mut self, c: CountryId, field: u8, by: u8, cap: u8) -> u8 {
        let cur = self.level(c, field);
        let new = cur.saturating_add(by).min(cap.min(100)).max(cur);
        self.set_level(c, field, new);
        new
    }

    /// Does the country meet every `(field, minimum level)` requirement?
    pub fn meets(&self, c: CountryId, prereqs: &[(u8, u8)]) -> bool {
        prereqs.iter().all(|&(f, l)| self.level(c, f) >= l)
    }

    pub fn has_model(&self, c: CountryId, model: u16) -> bool {
        self.unlocked.binary_search(&(c.0, model)).is_ok()
    }
    pub fn unlock(&mut self, c: CountryId, model: u16) {
        if let Err(i) = self.unlocked.binary_search(&(c.0, model)) {
            self.unlocked.insert(i, (c.0, model));
        }
    }

    /// Start developing a model. Needs the prerequisites and a model not yet owned.
    pub fn start_program(&mut self, c: CountryId, model: u16, prereqs: &[(u8, u8)], cost: u32) -> Result<u32, &'static str> {
        if self.has_model(c, model) {
            return Err("model already known");
        }
        if !self.meets(c, prereqs) {
            return Err("prerequisite fields too low");
        }
        if self.programs.iter().any(|p| p.country == c && p.model == model) {
            return Err("already in development");
        }
        let id = self.next_program;
        self.next_program += 1;
        self.programs.push(Program { id, country: c, model, cost, progress: 0 });
        Ok(id)
    }

    /// Add progress; finishing a program unlocks its model. Returns true when it finished.
    pub fn advance_program(&mut self, id: u32, points: u32) -> bool {
        let Some(i) = self.programs.iter().position(|p| p.id == id) else { return false };
        self.programs[i].progress = self.programs[i].progress.saturating_add(points);
        if self.programs[i].progress >= self.programs[i].cost {
            let p = self.programs.remove(i);
            self.unlock(p.country, p.model);
            true
        } else {
            false
        }
    }
    pub fn programs(&self) -> &[Program] {
        &self.programs
    }

    /// Hand a finished model to another country. The receiver gets the model but none of
    /// the field levels behind it. False if the giver does not have it.
    pub fn transfer_model(&mut self, from: CountryId, to: CountryId, model: u16) -> bool {
        if !self.has_model(from, model) {
            return false;
        }
        self.unlock(to, model);
        true
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u8(self.fields);
        for l in &self.levels {
            h.write_u8(*l);
        }
        h.write_u32(self.programs.len() as u32);
        for p in &self.programs {
            h.write_u32(p.id);
            h.write_u16(p.country.0);
            h.write_u16(p.model);
            h.write_u32(p.cost);
            h.write_u32(p.progress);
        }
        h.write_u32(self.next_program);
        h.write_u32(self.unlocked.len() as u32);
        for u in &self.unlocked {
            h.write_u16(u.0);
            h.write_u16(u.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_need_prerequisites_and_unlock_models() {
        let mut t = Tech::default();
        t.configure(4);
        let (a, b) = (CountryId(0), CountryId(1));
        t.set_level(a, 1, 60);
        t.set_level(a, 2, 30);
        assert_eq!(t.start_program(a, 9, &[(1, 70)], 100), Err("prerequisite fields too low"));
        let id = t.start_program(a, 9, &[(1, 50), (2, 30)], 100).unwrap();
        assert_eq!(t.start_program(a, 9, &[], 100), Err("already in development"));
        assert!(!t.advance_program(id, 60));
        assert!(t.advance_program(id, 60));
        assert!(t.has_model(a, 9) && t.programs().is_empty());
        assert_eq!(t.start_program(a, 9, &[], 1), Err("model already known"));
        // a transfer gives the model, not the fields
        assert!(t.transfer_model(a, b, 9));
        assert!(t.has_model(b, 9));
        assert_eq!(t.level(b, 1), 0);
        assert!(!t.transfer_model(b, a, 77));
    }

    #[test]
    fn learning_is_capped_by_diffusion_and_never_lowers() {
        let mut t = Tech::default();
        t.configure(2);
        let c = CountryId(3);
        assert_eq!(t.raise(c, 0, 10, 25), 10);
        assert_eq!(t.raise(c, 0, 20, 25), 25);
        assert_eq!(t.raise(c, 0, 5, 25), 25, "at the cap");
        t.set_level(c, 1, 90);
        assert_eq!(t.raise(c, 1, 5, 40), 90, "a cap below the current level never lowers it");
        assert_eq!(t.level(CountryId(50), 0), 0);
    }
}
