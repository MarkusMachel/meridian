//! Baker stub: scenario -> binary container. The payload encoding is a
//! placeholder (postcard); the real baked map format comes from the
//! benchmark (tools/format-bench). Container header, little-endian:
//!   magic "MRDN" | format_version u32 | kind u32 | payload_len u64 | checksum u64

use crate::error::{DataError, ErrorKind};
use crate::schema::Scenario;
use meridian_core::hash::StateHasher;

const MAGIC: &[u8; 4] = b"MRDN";
pub const FORMAT_VERSION: u32 = 1;
const KIND_SCENARIO: u32 = 1;
const HEADER_LEN: usize = 28;

fn checksum(payload: &[u8]) -> u64 {
    let mut h = StateHasher::default();
    h.write_bytes(payload);
    h.finish()
}

pub fn bake_scenario(s: &Scenario) -> Vec<u8> {
    let payload = postcard::to_stdvec(s).expect("serializing a scenario cannot fail");
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&KIND_SCENARIO.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&checksum(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

pub fn unbake_scenario(file: &str, bytes: &[u8]) -> Result<Scenario, DataError> {
    let bad = |msg: String| DataError::new(file, "-", ErrorKind::BadBinary, msg);
    if bytes.len() < HEADER_LEN || &bytes[0..4] != MAGIC {
        return Err(bad("not a Meridian baked file (bad magic number)".into()));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != FORMAT_VERSION {
        return Err(DataError::new(file, "-", ErrorKind::WrongVersion, format!("baked format version {version}, this build reads {FORMAT_VERSION}")));
    }
    let kind = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if kind != KIND_SCENARIO {
        return Err(bad(format!("unexpected file kind {kind}")));
    }
    let len = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let sum = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
    let payload = &bytes[HEADER_LEN..];
    if payload.len() as u64 != len {
        return Err(bad(format!("payload is {} bytes, header says {len} (truncated?)", payload.len())));
    }
    if checksum(payload) != sum {
        return Err(bad("checksum mismatch (file is corrupt)".into()));
    }
    postcard::from_bytes(payload).map_err(|e| bad(format!("payload does not decode: {e}")))
}
