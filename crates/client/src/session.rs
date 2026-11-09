//! A client session: owns the simulation thread, decides how many ticks to run per frame, and
//! keeps an exact log of the commands it sent and the tick each one was applied on.
//!
//! The log is what makes a client run reproducible. The session only sends commands and a
//! run request while the sim is idle, so a command always lands on a known tick: the one
//! after the last finished tick. `replay_headless` applies the same log with plain
//! `World::tick` calls and must end with the same state hash.

use meridian_core::{Command, CountryId, ProvinceId, Rejection, Ruleset, SimHandle, TickReport, World};

/// How fast time runs. Speeds are ticks per frame (a tick is one in-game hour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Paused,
    /// 1 to 5 ticks per frame.
    Run(u8),
}

impl Speed {
    pub fn ticks_per_frame(self) -> u64 {
        match self {
            Speed::Paused => 0,
            Speed::Run(n) => n.clamp(1, 5) as u64,
        }
    }
}

/// What the renderer and panels need, copied out of the world between ticks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub tick: u64,
    pub date: String,
    /// Controlling country per province; `u16::MAX` where the viewer cannot tell.
    pub controllers: Vec<u16>,
    /// Raw per-province counter (zeros unless built with `inspect`).
    pub values: Vec<i64>,
    /// (unit id, owner, province) of each unit the viewer can see.
    pub units: Vec<(u32, u16, u32)>,
}

pub struct Session {
    sim: SimHandle,
    viewer: CountryId,
    speed: Speed,
    /// The world's tick when the session began.
    base_tick: u64,
    /// Absolute tick the sim has been asked to reach (finished or not).
    requested: u64,
    step_once: bool,
    /// Never ask for ticks beyond this one (run-until-date, and exact stops in tests).
    stop_at: Option<u64>,
    queued: Vec<Command>,
    log: Vec<(u64, Command)>,
    last_refused: Vec<(Command, Rejection)>,
    snapshot_tick: Option<u64>,
    snapshot: Snapshot,
}

impl Session {
    pub fn new(world: World, rules: Box<dyn Ruleset + Send>, viewer: CountryId) -> Session {
        let requested = world.clock().tick();
        Session {
            sim: SimHandle::spawn(world, rules),
            viewer,
            speed: Speed::Paused,
            base_tick: requested,
            requested,
            step_once: false,
            stop_at: None,
            queued: Vec::new(),
            log: Vec::new(),
            last_refused: Vec::new(),
            snapshot_tick: None,
            snapshot: Snapshot::default(),
        }
    }

    pub fn speed(&self) -> Speed {
        self.speed
    }
    pub fn set_speed(&mut self, s: Speed) {
        self.speed = s;
    }
    /// Run exactly one tick at the next frame, whatever the speed.
    pub fn step(&mut self) {
        self.step_once = true;
    }
    /// Pause for good at this tick (the sim is asked for no more), or `None` to run on.
    pub fn set_stop_at(&mut self, tick: Option<u64>) {
        self.stop_at = tick;
    }
    /// Orders to send with the next tick.
    pub fn issue(&mut self, c: Command) {
        self.queued.push(c);
    }
    pub fn log(&self) -> &[(u64, Command)] {
        &self.log
    }
    /// Commands the sim refused most recently, with the reason.
    pub fn last_refused(&self) -> &[(Command, Rejection)] {
        &self.last_refused
    }
    pub fn sim(&self) -> &SimHandle {
        &self.sim
    }

    /// Ticks the sim thread has finished, counted on the world's own clock.
    fn done(&self) -> u64 {
        self.base_tick + self.sim.stats().ticks.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn idle(&self) -> bool {
        self.done() == self.requested
    }

    /// Call once per frame. Returns the number of ticks asked of the sim this frame (0 if paused,
    /// or if the sim has not finished the last request yet).
    pub fn frame(&mut self) -> u64 {
        for r in self.sim.drain_reports() {
            self.take_report(r);
        }
        if !self.idle() {
            return 0;
        }
        let mut n = if self.step_once { 1 } else { self.speed.ticks_per_frame() };
        self.step_once = false;
        if let Some(stop) = self.stop_at {
            n = n.min(stop.saturating_sub(self.requested));
        }
        if n == 0 {
            // Orders given while paused wait for the next tick; they are not applied yet.
            return 0;
        }
        if !self.queued.is_empty() {
            let cmds = std::mem::take(&mut self.queued);
            for c in &cmds {
                self.log.push((self.requested, c.clone()));
            }
            self.sim.send_commands(cmds);
        }
        self.sim.run(n);
        self.requested += n;
        n
    }

    fn take_report(&mut self, r: TickReport) {
        if !r.rejected.is_empty() {
            self.last_refused = r.rejected;
        }
    }

    /// Copy what the UI shows out of the world, if a tick has finished since the last copy.
    /// Waits for the sim only if it is mid-request (the query runs between ticks).
    pub fn refresh(&mut self) -> &Snapshot {
        let viewer = self.viewer;
        let tick_now = self.done();
        if self.snapshot_tick != Some(tick_now) {
            if let Some(s) = self.sim.query(move |w| snapshot_of(w, viewer)) {
                self.snapshot_tick = Some(s.tick);
                self.snapshot = s;
            }
        }
        &self.snapshot
    }

    /// Block until the sim has finished every tick asked of it (at most a frame's worth).
    pub fn wait_idle(&self) {
        while !self.idle() {
            std::thread::yield_now();
        }
    }

    /// A save of the world between ticks, after the requested ticks have finished, so the saved
    /// tick is exactly the one the session was asked to reach.
    pub fn save_bytes(&self) -> Option<Vec<u8>> {
        self.wait_idle();
        self.sim.query(meridian_data::save_world)
    }

    /// Finish: stop the thread and hand back the world and the command log.
    pub fn finish(self) -> (World, Vec<(u64, Command)>) {
        self.wait_idle();
        let log = self.log;
        (self.sim.stop(), log)
    }
}

fn snapshot_of(w: &World, viewer: CountryId) -> Snapshot {
    let n = w.province_count();
    let d = w.clock().date();
    let date = format!("{:04}-{:02}-{:02} {:02}:00", d.year, d.month, d.day, d.hour);
    let view = w.view(viewer);
    #[cfg(feature = "inspect")]
    let (controllers, values, units) = {
        let controllers = (0..n).map(|p| w.controller(ProvinceId(p)).0).collect();
        let values = (0..n).map(|p| w.value(ProvinceId(p))).collect();
        let units = (0..w.unit_count()).filter_map(|id| w.unit(meridian_core::UnitId(id)).map(|u| (id, u.owner.0, u.location.0))).collect();
        (controllers, values, units)
    };
    #[cfg(not(feature = "inspect"))]
    let (controllers, values, units) = {
        let controllers = (0..n).map(|p| view.province_controller(ProvinceId(p)).map_or(u16::MAX, |c| c.0)).collect();
        let units = view.own_units().iter().map(|(id, u)| (id.0, u.owner.0, u.location.0)).collect();
        (controllers, vec![0; n as usize], units)
    };
    let _ = &view;
    Snapshot { tick: w.clock().tick(), date, controllers, values, units }
}

/// Apply a command log with plain `World::tick` calls, the way a headless run would.
/// `log` is `(tick, command)` in order; a command is applied at the start of that tick.
pub fn replay_headless(mut world: World, rules: &dyn Ruleset, log: &[(u64, Command)], total_ticks: u64) -> World {
    let mut next = 0;
    while world.clock().tick() < total_ticks {
        let t = world.clock().tick();
        let mut cmds = Vec::new();
        while next < log.len() && log[next].0 == t {
            cmds.push(log[next].1.clone());
            next += 1;
        }
        world.tick(&cmds, rules);
    }
    world
}
