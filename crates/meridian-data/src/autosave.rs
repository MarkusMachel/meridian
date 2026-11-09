//! Autosave policy (Phase 5, step 7). No menus yet: this only decides WHEN to save and which
//! old autosaves to drop. The caller (runner now, client later) does the file writing.

#[derive(Clone, Copy, Debug)]
pub struct Autosave {
    /// Save every this many ticks (24 = once per game day). 0 turns autosave off.
    pub every_ticks: u64,
    /// How many autosaves to keep.
    pub keep: usize,
}

impl Autosave {
    /// Should an autosave be written before running `tick`? Tick 0 is not saved.
    pub fn due(&self, tick: u64) -> bool {
        self.every_ticks > 0 && tick > 0 && tick.is_multiple_of(self.every_ticks)
    }

    /// File name for the autosave taken at `tick`. Zero-padded so names sort in tick order.
    pub fn file_name(tick: u64) -> String {
        format!("autosave-{tick:012}.mrsv")
    }

    /// Of the autosave names present, which should be deleted to keep only the newest `keep`.
    pub fn stale(&self, mut names: Vec<String>) -> Vec<String> {
        names.retain(|n| n.starts_with("autosave-") && n.ends_with(".mrsv"));
        names.sort();
        let cut = names.len().saturating_sub(self.keep);
        names.truncate(cut);
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_names_and_rotation() {
        let a = Autosave { every_ticks: 24, keep: 2 };
        assert!(!a.due(0) && !a.due(23) && a.due(24) && a.due(48));
        assert!(!Autosave { every_ticks: 0, keep: 1 }.due(24));
        let names: Vec<String> = [24, 48, 72, 96].iter().map(|&t| Autosave::file_name(t)).chain(["notes.txt".to_string()]).collect();
        assert_eq!(a.stale(names), vec![Autosave::file_name(24), Autosave::file_name(48)]);
    }
}
