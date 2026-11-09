//! The thread boundary (Phase 6, step 5): run the simulation on its own thread while a UI
//! thread renders.
//!
//! The UI never touches the `World`. It talks to the sim thread through three channels:
//! - commands go in and are applied at the start of the next tick (as always);
//! - one `TickReport` per tick comes back, so a UI redraws only what changed;
//! - a *query* is a closure that runs on the sim thread BETWEEN ticks against the world, and
//!   its answer is sent back. The UI waits for the answer; the sim only pays for the closure.
//!   The sim never waits on the UI: reports go into an unbounded queue.
//!
//! The limit the design sets for reads: a query may cost the sim at most `MAX_QUERY_NS`
//! (2 ms). The sim records the slowest query it served so a test (and later the client) can
//! check it. Queries should return small answers (a country view, a list of cells).

use crate::command::Command;
use crate::world::{Ruleset, TickReport, World};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

/// How long one query may hold up the simulation.
pub const MAX_QUERY_NS: u64 = 2_000_000;

enum Request {
    Commands(Vec<Command>),
    Query(Box<dyn FnOnce(&World) + Send>),
    Run(u64),
    Stop,
}

/// Counters the sim thread keeps for diagnostics. Never part of the world state.
#[derive(Default)]
pub struct SimStats {
    pub ticks: AtomicU64,
    pub queries: AtomicU64,
    /// Slowest query closure so far, in nanoseconds.
    pub max_query_ns: AtomicU64,
}

pub struct SimHandle {
    tx: Sender<Request>,
    reports: Receiver<TickReport>,
    join: Option<JoinHandle<World>>,
    stats: Arc<SimStats>,
}

impl SimHandle {
    /// Move the world and ruleset onto a new thread. The sim idles until told to run.
    pub fn spawn(mut world: World, rules: Box<dyn Ruleset + Send>) -> SimHandle {
        let (tx, rx) = channel::<Request>();
        let (report_tx, reports) = channel::<TickReport>();
        let stats = Arc::new(SimStats::default());
        let thread_stats = stats.clone();
        let join = std::thread::spawn(move || {
            let mut pending: Vec<Command> = Vec::new();
            let mut remaining: u64 = 0;
            'sim: loop {
                // Serve everything that is waiting. When there is nothing to run, block.
                loop {
                    let req = if remaining == 0 {
                        match rx.recv() {
                            Ok(r) => r,
                            Err(_) => break 'sim,
                        }
                    } else {
                        match rx.try_recv() {
                            Ok(r) => r,
                            Err(_) => break,
                        }
                    };
                    match req {
                        Request::Commands(mut c) => pending.append(&mut c),
                        Request::Run(n) => remaining += n,
                        Request::Query(f) => {
                            let t = Instant::now();
                            f(&world);
                            let ns = t.elapsed().as_nanos() as u64;
                            thread_stats.queries.fetch_add(1, Ordering::Relaxed);
                            thread_stats.max_query_ns.fetch_max(ns, Ordering::Relaxed);
                        }
                        Request::Stop => break 'sim,
                    }
                }
                if remaining > 0 {
                    let report = world.tick(&pending, &*rules);
                    pending.clear();
                    remaining -= 1;
                    thread_stats.ticks.fetch_add(1, Ordering::Relaxed);
                    if report_tx.send(report).is_err() {
                        break 'sim;
                    }
                }
            }
            world
        });
        SimHandle { tx, reports, join: Some(join), stats }
    }

    /// Queue commands for the next tick.
    pub fn send_commands(&self, commands: Vec<Command>) {
        let _ = self.tx.send(Request::Commands(commands));
    }

    /// Run this many more ticks as fast as possible.
    pub fn run(&self, ticks: u64) {
        let _ = self.tx.send(Request::Run(ticks));
    }

    /// Reports of the ticks that have finished since the last call, oldest first. Never blocks.
    pub fn drain_reports(&self) -> Vec<TickReport> {
        self.reports.try_iter().collect()
    }

    /// Run `f` on the sim thread between ticks and return its result. Blocks the caller (not
    /// the sim) until the sim gets to it. `None` if the sim thread has ended.
    pub fn query<R: Send + 'static>(&self, f: impl FnOnce(&World) -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = channel();
        self.tx.send(Request::Query(Box::new(move |w| {
            let _ = tx.send(f(w));
        }))).ok()?;
        rx.recv().ok()
    }

    pub fn stats(&self) -> &SimStats {
        &self.stats
    }

    /// Stop the thread and take the world back.
    pub fn stop(mut self) -> World {
        let _ = self.tx.send(Request::Stop);
        self.join.take().expect("joined once").join().expect("sim thread panicked")
    }
}

impl Drop for SimHandle {
    fn drop(&mut self) {
        if let Some(j) = self.join.take() {
            let _ = self.tx.send(Request::Stop);
            let _ = j.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Signals;
    use crate::ids::{CountryId, ProvinceId, UnitId};
    use crate::world::Unit;

    struct Counter;
    impl Ruleset for Counter {
        fn run(&self, w: &mut World, s: Signals) {
            if s.new_day {
                for v in w.values_mut() {
                    *v += 1;
                }
            }
        }
    }

    fn world() -> World {
        let mut w = World::new(1, 2, 4);
        w.add_unit_for_setup(Unit { owner: CountryId(0), location: ProvinceId(0) });
        w
    }

    #[test]
    fn the_threaded_run_matches_a_plain_run_and_queries_never_change_state() {
        let cmds = vec![Command::MoveUnit { issuer: CountryId(0), unit: UnitId(0), to: ProvinceId(2) }];
        let mut plain = world();
        plain.tick(&cmds, &Counter);
        for _ in 0..4999 {
            plain.tick(&[], &Counter);
        }

        let h = SimHandle::spawn(world(), Box::new(Counter));
        h.send_commands(cmds);
        h.run(5000);
        // hammer the sim with queries while it runs
        let mut seen_ticks = 0;
        for _ in 0..500 {
            let v = h.query(|w| w.view(CountryId(0)).own_units().len() + w.clock().tick() as usize).unwrap();
            seen_ticks = seen_ticks.max(v);
        }
        assert!(seen_ticks >= 1);
        // a final query is answered only after the earlier Run request has been queued ahead of it
        while h.stats().ticks.load(Ordering::Relaxed) < 5000 {
            std::thread::yield_now();
        }
        let reports = h.drain_reports();
        assert_eq!(reports.len(), 5000);
        assert!(reports.windows(2).all(|p| p[1].tick == p[0].tick + 1), "one report per tick, in order");
        assert!(h.stats().queries.load(Ordering::Relaxed) >= 500);
        let slowest = h.stats().max_query_ns.load(Ordering::Relaxed);
        assert!(slowest < MAX_QUERY_NS, "slowest query held up the sim for {slowest} ns (limit {MAX_QUERY_NS})");
        let threaded = h.stop();
        assert_eq!(threaded.state_hash(), plain.state_hash());
    }

    #[test]
    fn a_dropped_handle_stops_the_thread() {
        let h = SimHandle::spawn(world(), Box::new(Counter));
        h.run(10);
        drop(h); // must not hang
    }
}
