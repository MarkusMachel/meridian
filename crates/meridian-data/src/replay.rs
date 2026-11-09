//! Replays and divergence finding (Phase 5).
//!
//! A replay is: which scenario, the seed, the map checksum, and a log of every command with
//! the tick it was applied on and who issued it (player, AI, or a delegated headquarters).
//! Because AI commands are logged, replaying never runs the AI, so a replay does not depend on
//! the AI code of the build that plays it. Checkpoint hashes let the player stop at the first
//! tick that disagrees.
//!
//! File layout, little-endian: magic "MRRP" | version u32 | kind u32 (4 = replay) |
//! payload_len u64 | checksum u64 (CRC-32 of the payload) | payload (postcard of `Replay`).

use crate::error::{DataError, ErrorKind};
use meridian_core::{Command, Ruleset, World};
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 4] = b"MRRP";
pub const REPLAY_VERSION: u32 = 1;
const KIND_REPLAY: u32 = 4;
const HEADER_LEN: usize = 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Player,
    Ai,
    Headquarters,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub tick: u64,
    pub source: Source,
    pub command: Command,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replay {
    /// A name or path the caller can resolve to a starting world.
    pub scenario: String,
    pub seed: u64,
    pub map_checksum: Option<u32>,
    /// Total ticks run.
    pub ticks: u64,
    /// In application order; ticks ascending.
    pub commands: Vec<Entry>,
    /// (tick, hash of the state before that tick runs), ascending.
    pub checkpoints: Vec<(u64, u64)>,
    pub final_hash: u64,
}

/// Records a run as it happens.
pub struct Recorder {
    replay: Replay,
    every: u64,
}

impl Recorder {
    /// Checkpoint every `every` ticks (and once at the start).
    pub fn new(world: &World, scenario: &str, every: u64) -> Recorder {
        Recorder {
            replay: Replay {
                scenario: scenario.to_string(),
                seed: world.seed(),
                map_checksum: world.map_checksum(),
                ticks: 0,
                commands: Vec::new(),
                checkpoints: Vec::new(),
                final_hash: 0,
            },
            every: every.max(1),
        }
    }

    /// Run one tick with the given commands, logging them. Player commands go first, then
    /// AI, then headquarters, which is also the order they are applied in.
    pub fn step(&mut self, world: &mut World, rules: &dyn Ruleset, player: &[Command], ai: &[Command], hq: &[Command]) {
        let tick = world.clock().tick();
        if tick.is_multiple_of(self.every) {
            self.replay.checkpoints.push((tick, world.state_hash()));
        }
        let mut all = Vec::with_capacity(player.len() + ai.len() + hq.len());
        for (src, list) in [(Source::Player, player), (Source::Ai, ai), (Source::Headquarters, hq)] {
            for c in list {
                self.replay.commands.push(Entry { tick, source: src, command: c.clone() });
                all.push(c.clone());
            }
        }
        world.tick(&all, rules);
    }

    pub fn finish(mut self, world: &World) -> Replay {
        self.replay.ticks = world.clock().tick();
        self.replay.final_hash = world.state_hash();
        self.replay
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    pub tick: u64,
    pub expected: u64,
    pub got: u64,
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "diverged at tick {}: expected hash {:016x}, got {:016x}", self.tick, self.expected, self.got)
    }
}

/// Re-run a replay from a fresh starting world, checking every checkpoint and the final hash.
/// Returns the final hash on success.
pub fn play(world: &mut World, rules: &dyn Ruleset, r: &Replay) -> Result<u64, Divergence> {
    let mut next_cmd = 0;
    let mut next_cp = 0;
    while world.clock().tick() < r.ticks {
        let tick = world.clock().tick();
        if next_cp < r.checkpoints.len() && r.checkpoints[next_cp].0 == tick {
            let got = world.state_hash();
            if got != r.checkpoints[next_cp].1 {
                return Err(Divergence { tick, expected: r.checkpoints[next_cp].1, got });
            }
            next_cp += 1;
        }
        let mut cmds = Vec::new();
        while next_cmd < r.commands.len() && r.commands[next_cmd].tick == tick {
            cmds.push(r.commands[next_cmd].command.clone());
            next_cmd += 1;
        }
        world.tick(&cmds, rules);
    }
    let got = world.state_hash();
    if got != r.final_hash {
        return Err(Divergence { tick: r.ticks, expected: r.final_hash, got });
    }
    Ok(got)
}

/// Hash after every tick (index 0 is the state before tick 0), for comparing two runs.
pub fn hash_trace(world: &mut World, rules: &dyn Ruleset, commands: &[Entry], ticks: u64) -> Vec<u64> {
    let mut out = vec![world.state_hash()];
    let mut next = 0;
    for tick in world.clock().tick()..ticks {
        let mut cmds = Vec::new();
        while next < commands.len() && commands[next].tick == tick {
            cmds.push(commands[next].command.clone());
            next += 1;
        }
        world.tick(&cmds, rules);
        out.push(world.state_hash());
    }
    out
}

/// The first position where two hash traces differ (the tick, if both start at tick 0).
/// A trace that ends early counts as differing where it stops.
pub fn first_divergence(a: &[u64], b: &[u64]) -> Option<usize> {
    let n = a.len().min(b.len());
    (0..n).find(|&i| a[i] != b[i]).or(if a.len() != b.len() { Some(n) } else { None })
}

pub fn write_replay(r: &Replay) -> Vec<u8> {
    let payload = postcard::to_stdvec(r).expect("serializing a replay cannot fail");
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&REPLAY_VERSION.to_le_bytes());
    out.extend_from_slice(&KIND_REPLAY.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&(crc32fast::hash(&payload) as u64).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

pub fn read_replay(file: &str, bytes: &[u8]) -> Result<Replay, DataError> {
    let bad = |msg: String| DataError::new(file, "-", ErrorKind::BadBinary, msg);
    if bytes.len() < HEADER_LEN || &bytes[0..4] != MAGIC {
        return Err(bad("not a Meridian replay file (bad magic number)".into()));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != REPLAY_VERSION {
        return Err(DataError::new(file, "-", ErrorKind::WrongVersion, format!("replay version {version}, this build reads {REPLAY_VERSION}")));
    }
    if u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != KIND_REPLAY {
        return Err(bad("unexpected file kind".into()));
    }
    let len = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let sum = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
    let payload = &bytes[HEADER_LEN..];
    if payload.len() as u64 != len {
        return Err(bad(format!("payload is {} bytes, header says {len} (truncated?)", payload.len())));
    }
    if crc32fast::hash(payload) as u64 != sum {
        return Err(bad("checksum mismatch (file is corrupt)".into()));
    }
    postcard::from_bytes(payload).map_err(|e| bad(format!("payload does not decode: {e}")))
}
