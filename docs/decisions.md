# Decision log

## Phase 1: Core loop (2026-11-08)

- **Fixed point:** Q32.32 in an i64, i128 intermediates for mul/div, saturating on overflow. Division by zero panics (caller bug).
- **RNG:** xoshiro256** seeded via splitmix64, verified against an independent Python implementation. Golden values pinned in tests.
- **Calendar:** tick 0 = 1990-01-01 00:00; date derived from tick count by integer civil-calendar math. Rollover signals are for the tick about to run (tick 0 counts as new day/month/year).
- **Hash:** FNV-1a 64 over explicit little-endian bytes, no dependencies. Not cryptographic; fine for divergence detection. A test mutates every field and checks the hash changes.
- **Tick order:** apply commands (in submission order) -> run ruleset -> advance clock.
- **Commands:** validated fully before mutating; a rejection changes nothing. Placeholder commands: MoveUnit, SetProvinceController. Issuer must own the unit.
- **Ruleset:** a trait in meridian-core (`Ruleset`); the proving-ground crate implements it, so the dependency points the right way. Rulesets keep no state of their own.
- **Rejected:** a float-free `f32` display helper in core (kept one `to_f64_for_display` only).
- **Linux verified (2026-11-08):** `scripts/check.sh` passes in Docker on linux/arm64 and linux/amd64 (rust:1.91). Seed 1 hash `2f1e797ab5e8d8f5` and seed 2 hash `490c68f0ed77447b` are identical to macOS.

## Phase 2: Data and scenarios (2026-11-08)

- **Crate versions checked on crates.io (2026-11-08):** ron 0.12.3 (updated 2026-10-28), postcard 1.1.3 (last release 2025-07-24, about 38M recent downloads, stable), rkyv 0.8.18 (updated 2026-08-05). All MIT or Apache-2.0, so fine for a possible commercial release. This supersedes the stale rkyv note in guide section 3.5.
- **Authored text format: RON**, as decided earlier. Files use anonymous structs `( ... )` with a top-level `version`. Schema types (`meridian-data/src/schema.rs`) are separate from engine types; the loader converts. `deny_unknown_fields` is on, so typos are errors.
- **Loader flow:** read version first (so an old file gets a version error, not a field error) -> full parse -> semantic checks collecting every error -> build world only if there were none. A bad file therefore cannot yield a partly built world; the API returns `Result<World, Vec<DataError>>`.
- **IDs:** unique and contiguous from 0 per table (so ID = array index). Gaps and duplicates are errors.
- **Errors** carry file, item (e.g. `units[2] (id 2)`), kind and problem. Classes: Io, Malformed (with line/column), MissingField, WrongVersion, DuplicateId, NonContiguousId, UnknownReference, BadValue, BadBinary.
- **Sites** added to the world (kind, province, owner, condition 0-100, built year, destroyed year) and to the state hash. Anchored to a province for now; the cell anchor arrives with cells in Phase 3.
- **Baker stub:** container header `MRDN | format_version u32 | kind u32 | payload_len u64 | checksum u64` (little-endian, FNV-1a checksum) around a postcard payload. Deterministic, round-trip tested, corruption and truncation detected. Scenarios are tiny, so the payload encoding does not matter yet.
- **Format benchmark** (`cargo run --release -p tools --bin format-bench`, Apple M4 Pro, median of 7 runs, synthetic map of 16,000 provinces, 600,000 cells, 3.6M adjacency entries):

  | Format | Size | Load |
  | --- | --- | --- |
  | RON text | 49.1 MB | 563 ms |
  | postcard to owned arrays | 21.6 MB | 12.6 ms |
  | custom flat arrays to owned | 28.3 MB | 0.6 ms |
  | rkyv validated zero-copy access | 28.3 MB | about 0 ms |
  | rkyv validate and convert to owned | 28.3 MB | 0.5 ms |

- **Decision: baked map data uses rkyv.** It is as fast as the hand-rolled flat format, validates on load, and spares us maintaining a custom container. Its layout is tied to type definitions, which is fine for data we regenerate. Revisit in Phase 3 if the real map (nested site tables) behaves differently. The custom flat format stays the fallback.
- **Decision: saves stay on postcard** (12.6 ms for 21.6 MB, small, and the format is under our version control). Needs a version number and migration step in Phase 5.
- **Rejected:** RON for the map (about 45x slower than postcard, 2x larger); bincode (unmaintained).
- **Caveat:** the benchmark map is synthetic random data. Real maps compress differently but the array sizes are the same, so load times should be comparable. rkyv zero-copy access needs a 16-byte-aligned buffer (use `rkyv::util::AlignedVec` when reading a file).
- **Deferred:** a stale-looking guide item "postcard actively maintained" is resolved as above (stable 1.x, heavy use, but the last release is over a year old; acceptable for a format we can also hand-roll).
- **Intentional hash change:** adding the site table to the state hash changed the Phase 1 golden hash (`9773012133288501445` -> `16117180310011935637`) and the runner hashes, even with zero sites, because the site count is hashed. Golden values re-pinned. The Phase 1 Linux hashes quoted above are historical.
- **Linux verified for Phase 2 (2026-11-08):** `scripts/check.sh` passes on linux/arm64 and linux/amd64. Runner hashes (seed 1 `9c4a31d740efcec5`, seed 2 `f99a6164964f1dab`) and `runner run data/scenarios/sample.ron` (`caded2dcbee2f0a5`) match macOS.
