//! Save files (Phase 5). A save is the whole `World` state plus its state hash.
//!
//! Layout, little-endian, same shape as the other containers:
//!   magic "MRSV" | format_version u32 | kind u32 (3 = save) | payload_len u64 | checksum u64
//! followed by the payload: a postcard encoding of `SaveBody`.
//!
//! - The checksum is the CRC-32 of the payload, zero-extended.
//! - Per-cell state is stored as differences from the starting state of the map, so a save
//!   of a mostly quiet world stays small. The map itself is never inside a save; the save
//!   records its checksum and loading needs the same map.
//! - Derived caches (zones of control and so on) are never saved. They are rebuilt from the
//!   loaded state.
//! - After decoding, the loader recomputes the state hash and refuses the file if it does
//!   not match the one stored at save time.

use crate::error::{DataError, ErrorKind};
use meridian_core::{CellChanges, Map, World};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"MRSV";
pub const SAVE_VERSION: u32 = 3;
/// The oldest save version `migrate` can bring forward.
pub const OLDEST_MIGRATABLE: u32 = 3;
const KIND_SAVE: u32 = 3;
const HEADER_LEN: usize = 28;

#[derive(Serialize, Deserialize)]
struct SaveBody {
    map_checksum: Option<u32>,
    state_hash: u64,
    changes: Option<CellChanges>,
    world: World,
}

fn crc(payload: &[u8]) -> u64 {
    crc32fast::hash(payload) as u64
}

pub fn save_world(world: &World) -> Vec<u8> {
    let body = SaveBody { map_checksum: world.map_checksum(), state_hash: world.state_hash(), changes: world.cell_changes(), world: world.clone() };
    let payload = postcard::to_stdvec(&body).expect("serializing a world cannot fail");
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&SAVE_VERSION.to_le_bytes());
    out.extend_from_slice(&KIND_SAVE.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&crc(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Bring an older payload up to the current version. There is only one version so far, so
/// this is the place a future change adds its step. Older-than-supported and newer-than-known
/// versions fail with a message that says what to do.
fn migrate(version: u32, payload: &[u8]) -> Result<Vec<u8>, String> {
    match version {
        SAVE_VERSION => Ok(payload.to_vec()),
        v if v > SAVE_VERSION => Err(format!("this save was written by a newer build (save version {v}); this build reads up to {SAVE_VERSION}. Update the game to open it")),
        v => Err(format!("save version {v} is older than the oldest this build can migrate ({OLDEST_MIGRATABLE}); it cannot be opened")),
    }
}

/// Decode a save. `map` is needed when the save was made on a map world and must have the
/// same checksum.
pub fn load_world(file: &str, bytes: &[u8], map: Option<Arc<Map>>) -> Result<World, DataError> {
    let bad = |msg: String| DataError::new(file, "-", ErrorKind::BadBinary, msg);
    if bytes.len() < HEADER_LEN || &bytes[0..4] != MAGIC {
        return Err(bad("not a Meridian save file (bad magic number)".into()));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let kind = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if kind != KIND_SAVE {
        return Err(bad(format!("unexpected file kind {kind}")));
    }
    let len = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let sum = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
    let raw = &bytes[HEADER_LEN..];
    if raw.len() as u64 != len {
        return Err(bad(format!("payload is {} bytes, header says {len} (truncated?)", raw.len())));
    }
    if crc(raw) != sum {
        return Err(bad("checksum mismatch (file is corrupt)".into()));
    }
    let payload = migrate(version, raw).map_err(|m| DataError::new(file, "-", ErrorKind::WrongVersion, m))?;
    let body: SaveBody = postcard::from_bytes(&payload).map_err(|e| bad(format!("payload does not decode: {e}")))?;
    let mut world = body.world;
    match (body.map_checksum, map) {
        (None, _) => {}
        (Some(want), Some(m)) if m.checksum == want => {
            let changes = body.changes.ok_or_else(|| bad("map save without cell changes".into()))?;
            world.attach_map(m, &changes).map_err(bad)?;
        }
        (Some(want), Some(m)) => return Err(bad(format!("the save was made on map {want:08x}, but the loaded map is {:08x}", m.checksum))),
        (Some(want), None) => return Err(bad(format!("the save needs map {want:08x}; none was given"))),
    }
    let got = world.state_hash();
    if got != body.state_hash {
        return Err(bad(format!("loaded state hash {got:016x} differs from the saved {:016x}", body.state_hash)));
    }
    Ok(world)
}
