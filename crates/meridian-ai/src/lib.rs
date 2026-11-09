//! AI crate boundary (Phase 4F, step 21). The AI reads a `CountryView` and returns
//! commands, exactly like a player. It has no access to the true world.
//!
//! AI memory lives in its own save section (`save_section`), outside the world state hash:
//! commands the AI issues are logged in the replay like any command, so a replay never
//! re-runs the AI. The AI itself (utility scoring, planners) is built after the systems
//! it decides on exist (gameplay backlog G4).

use meridian_core::{Command, CountryId, CountryView};
use std::time::Instant;

pub trait Ai {
    /// Look at the world through the view and decide this turn's commands.
    fn decide(&mut self, view: &CountryView) -> Vec<Command>;
    /// The AI's private memory, saved beside (not inside) the world save.
    fn save_section(&self) -> Vec<u8>;
    fn load_section(&mut self, bytes: &[u8]);
}

/// Does nothing. Counts its turns so the save section has something to carry.
#[derive(Default)]
pub struct IdleAi {
    pub turns: u32,
}

impl Ai for IdleAi {
    fn decide(&mut self, _view: &CountryView) -> Vec<Command> {
        self.turns += 1;
        Vec::new()
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

/// How often an AI thinks, by importance tier: tier 0 every tick of its period table.
#[derive(Clone, Debug)]
pub struct AiSchedule {
    /// Ticks between turns for each tier (tier index into this list).
    pub period_by_tier: Vec<u64>,
    /// Tier per country, indexed by country ID.
    pub tier: Vec<u8>,
}

impl AiSchedule {
    /// Countries whose AI should act on this tick, ascending by country ID. A country's
    /// phase is offset by its ID so turns spread across ticks instead of bunching up.
    pub fn due(&self, tick: u64) -> Vec<CountryId> {
        (0..self.tier.len())
            .filter(|&c| {
                let period = self.period_by_tier[self.tier[c] as usize].max(1);
                (tick + c as u64).is_multiple_of(period)
            })
            .map(|c| CountryId(c as u16))
            .collect()
    }
}

/// Per-AI cost, for finding a planner that is too slow.
#[derive(Clone, Debug, Default)]
pub struct AiTiming {
    pub turns: u64,
    pub total_ns: u64,
}

/// Run every due AI against the world and return all commands in country ID order.
/// Timing is recorded per country in `timing` (diagnostics only, never hashed).
pub fn run_ais(world: &meridian_core::World, schedule: &AiSchedule, ais: &mut [Box<dyn Ai>], timing: &mut [AiTiming]) -> Vec<Command> {
    let mut out = Vec::new();
    for c in schedule.due(world.clock().tick()) {
        let view = world.view(c);
        let t = Instant::now();
        out.extend(ais[c.index()].decide(&view));
        timing[c.index()].turns += 1;
        timing[c.index()].total_ns += t.elapsed().as_nanos() as u64;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use meridian_core::{ProvinceId, ScheduleBuilder, Unit, UnitId, World};

    /// Moves its first unit one province along each turn, through the command interface only.
    struct Walker;
    impl Ai for Walker {
        fn decide(&mut self, view: &CountryView) -> Vec<Command> {
            let Some((unit, u)) = view.own_units().first().copied() else { return Vec::new() };
            vec![Command::MoveUnit { issuer: view.country(), unit, to: ProvinceId((u.location.0 + 1) % 4) }]
        }
        fn save_section(&self) -> Vec<u8> {
            Vec::new()
        }
        fn load_section(&mut self, _: &[u8]) {}
    }

    #[test]
    fn ais_act_only_through_commands_and_memory_stays_out_of_the_hash() {
        let mut w = World::new(1, 3, 4);
        for c in 0..3u16 {
            w.add_unit_for_setup(Unit { owner: CountryId(c), location: ProvinceId(c as u32) });
        }
        let rules = ScheduleBuilder::new().build().unwrap();
        let sched = AiSchedule { period_by_tier: vec![1, 24], tier: vec![0, 1, 1] };
        // tier 1 thinks every 24 ticks, offset by country ID so turns do not bunch up
        assert_eq!(sched.due(0), vec![CountryId(0)]);
        assert_eq!(sched.due(22), vec![CountryId(0), CountryId(2)]);
        assert_eq!(sched.due(23), vec![CountryId(0), CountryId(1)]);
        let mut ais: Vec<Box<dyn Ai>> = vec![Box::new(Walker), Box::new(IdleAi::default()), Box::new(Walker)];
        let mut timing = vec![AiTiming::default(); 3];
        let before = w.state_hash();
        let cmds = run_ais(&w, &sched, &mut ais, &mut timing);
        assert_eq!(w.state_hash(), before, "deciding changes nothing");
        assert_eq!(cmds.len(), 1, "only country 0 (walker, due) moves at tick 0");
        w.tick(&cmds, &rules);
        assert_eq!(w.unit(UnitId(0)).unwrap().location, ProvinceId(1));
        assert_eq!(timing[0].turns, 1);

        // AI memory round-trips through its own section and is not part of the world
        let idle = IdleAi { turns: 41 };
        let bytes = idle.save_section();
        let mut again = IdleAi::default();
        again.load_section(&bytes);
        assert_eq!(again.turns, 41);
    }
}
