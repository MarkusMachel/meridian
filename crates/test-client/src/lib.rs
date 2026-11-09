//! Phase 6 done tests: a client that uses only the public interface, a stub AI against a stub
//! player, run headless and deterministic, over a thread boundary, with refusals that explain
//! themselves and change nothing.

use meridian_ai::Ai;
use meridian_core::{Command, CountryId, CountryView, ProvinceId, Ruleset, Signals, World};

/// No rules at all: the world only changes through commands.
pub struct NoRules;
impl Ruleset for NoRules {
    fn run(&self, _world: &mut World, _signals: Signals) {}
}

pub const PROVINCES: u32 = 8;

pub fn world() -> World {
    let mut w = World::new(7, 2, PROVINCES);
    for owner in 0..2u16 {
        for k in 0..2u32 {
            w.add_unit_for_setup(meridian_core::Unit { owner: CountryId(owner), location: ProvinceId(owner as u32 * 4 + k) });
        }
    }
    w
}

/// The "player": walks its first unit forward one province per tick.
pub struct StubPlayer;
impl StubPlayer {
    pub fn orders(&self, view: &CountryView) -> Vec<Command> {
        match view.own_units().first() {
            Some(&(unit, u)) => vec![Command::MoveUnit { issuer: view.country(), unit, to: ProvinceId((u.location.0 + 1) % PROVINCES) }],
            None => Vec::new(),
        }
    }
}

/// The stub AI: walks its first unit backward, and every 5th turn also tries something
/// illegal (moving a unit it does not own) to exercise refusals.
#[derive(Default)]
pub struct StubAi {
    turns: u32,
}
impl Ai for StubAi {
    fn decide(&mut self, view: &CountryView) -> Vec<Command> {
        self.turns += 1;
        let mut out = Vec::new();
        if let Some(&(unit, u)) = view.own_units().first() {
            out.push(Command::MoveUnit { issuer: view.country(), unit, to: ProvinceId((u.location.0 + PROVINCES - 1) % PROVINCES) });
        }
        if self.turns.is_multiple_of(5) {
            // unit 0 belongs to country 0, not to us
            out.push(Command::MoveUnit { issuer: view.country(), unit: meridian_core::UnitId(0), to: ProvinceId(0) });
        }
        out
    }
    fn save_section(&self) -> Vec<u8> {
        self.turns.to_le_bytes().to_vec()
    }
    fn load_section(&mut self, bytes: &[u8]) {
        if let Ok(b) = bytes.try_into() {
            self.turns = u32::from_le_bytes(b);
        }
    }
}

/// Run player and AI against each other for `ticks` ticks on the calling thread.
/// Returns the final hash and the number of refusals.
pub fn headless_match(ticks: u64) -> (u64, usize) {
    let mut w = world();
    let (player, mut ai) = (StubPlayer, StubAi::default());
    let mut refused = 0;
    for _ in 0..ticks {
        let mut cmds = player.orders(&w.view(CountryId(0)));
        cmds.extend(ai.decide(&w.view(CountryId(1))));
        refused += w.tick(&cmds, &NoRules).rejected.len();
    }
    (w.state_hash(), refused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use meridian_core::{Rejection, SimHandle, UnitId};

    #[test]
    fn stub_ai_and_stub_player_are_deterministic_by_hash() {
        let (h1, r1) = headless_match(2000);
        let (h2, r2) = headless_match(2000);
        assert_eq!(h1, h2);
        assert_eq!(r1, r2);
        assert_eq!(r1, 400, "the AI's illegal order is refused every fifth turn");
        assert_ne!(headless_match(1999).0, h1, "and the hash does move with the run");
    }

    #[test]
    fn the_same_match_over_the_thread_boundary_gives_the_same_hash() {
        let direct = headless_match(300).0;
        let h = SimHandle::spawn(world(), Box::new(NoRules));
        let mut ai = StubAi::default();
        let mut refused = 0;
        for _ in 0..300 {
            // read through a query, then give orders through the command channel
            let (p, a) = h
                .query(move |w| {
                    // the closure gets the world, but this client only calls `view`
                    (StubPlayer.orders(&w.view(CountryId(0))), w.view(CountryId(1)).own_units().len())
                })
                .unwrap();
            assert_eq!(a, 2);
            let mut cmds = p;
            // the AI needs a view, which only lives inside a query; run its decision there
            let mut ai_for_query = std::mem::take(&mut ai);
            let (ai_back, ai_cmds) = h.query(move |w| {
                let c = ai_for_query.decide(&w.view(CountryId(1)));
                (ai_for_query, c)
            }).unwrap();
            ai = ai_back;
            cmds.extend(ai_cmds);
            h.send_commands(cmds);
            h.run(1);
            let report = loop {
                if let Some(r) = h.drain_reports().pop() {
                    break r;
                }
                std::thread::yield_now();
            };
            refused += report.rejected.len();
        }
        assert_eq!(refused, 60);
        let w = h.stop();
        assert_eq!(w.state_hash(), direct);
    }

    #[test]
    fn a_refusal_names_a_reason_and_changes_nothing() {
        let mut w = world();
        let mut reference = world();
        let bad = Command::MoveUnit { issuer: CountryId(1), unit: UnitId(0), to: ProvinceId(3) };
        // the preview says no, and says why, without touching the world
        let before = w.state_hash();
        let why = w.view(CountryId(1)).check(&bad, &NoRules).unwrap_err();
        assert_eq!(why, Rejection::NotOwner { issuer: CountryId(1) });
        assert!(!why.to_string().is_empty());
        assert_eq!(w.state_hash(), before);
        // and so does the real tick
        let report = w.tick(std::slice::from_ref(&bad), &NoRules);
        assert_eq!(report.rejected.len(), 1);
        reference.tick(&[], &NoRules);
        assert_eq!(w.state_hash(), reference.state_hash());
    }
}
