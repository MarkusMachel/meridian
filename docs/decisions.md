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

## Phase 3: Real map, step 1 (1990 states) (2026-11-08)

- **Source data: Natural Earth 5.1.1, 10m admin-0 countries and admin-1 states/provinces**, downloaded from naciscdn.org and pinned by SHA-256 in `crates/map-builder/fetch_sources.sh` (files themselves are not committed). License: the official Terms of Use say all vector data is public domain, free for personal, educational and commercial use, no credit required. (Natural Earth's forum mentions a GADM-derived-columns concern on admin-1; we use only geometry and names, never those columns.)
- **Chosen over** CShapes 2.0 (CC BY-NC, and two catalogs disagree on SA), historical-basemaps (unclear license, coarse), GADM (non-commercial only). CShapes may be used as a private cross-check only; nothing from it ships.
- **Approach:** 1990 states are built from modern entities with a hand-written table (`crates/map-builder/config/states_1990.json`): merges (USSR 15 republics, Yugoslavia 7 entities including Kosovo, Czechoslovakia) and splits (Germany into East and West, Yemen into North and South) done by cutting the admin-0 outline with admin-1 unions so outer borders stay shared. Dependencies follow their 1990 sovereign; 8 tiny or unclaimed areas and Antarctica are excluded, each with a reason in the config.
- **Known simplifications** (also in the config `notes`): Berlin whole to East Germany; Namibia independent (South African-administered until March 1990); Western Sahara to Morocco; Palestine to Israel; Hong Kong to the UK; Macau to Portugal; Palau to the US; Al Bayda' to North Yemen.
- **Result:** 173 states, total land 134.46 million km2 (guide estimate was about 135 million, includes dependencies). Spot checks against reference areas: West Germany 248.6k (real about 248.6k), Yugoslavia 254.8k (real 255.8k), Soviet Union 22.13M (real 22.4M including inland water). Both Yemens together are 453k km2 because Natural Earth's outline stops at the desert border; real figures are larger.
- **Tooling:** Python 3.15 in `crates/map-builder/.venv` (not committed), versions pinned in `requirements.txt` (shapely 2.2.0, pyproj 3.8.0, pyshp 3.1.7, numpy 2.5.4, scipy 1.18.1). Running `build_states.py` twice gives byte-identical GeoJSON (sha256 `16fa95d6...3508a`) on this machine; cross-machine reproducibility depends on GEOS version and is not yet tested.
- **Not yet done in Phase 3:** provinces, cells, adjacency graph, sea zones, sites, 1990 population (Natural Earth's population is 2019, so 1990 values need another source or scaling), baked map file, pathfinding, budgets.

## Phase 3: Real map, step 2 (provinces) (2026-11-08)

- **Result:** 15,240 provinces in 173 states (target was 15,000 to 17,000). Estimated area: median 9,001 km2, mean 8,823 km2, max 28,705 km2; 17 provinces over 15,000 km2, 1 over 20,000; 360 under 3,000 (mostly small states). Run time about 25 seconds.
- **Method** (`crates/map-builder/build_provinces.py`, no random numbers): project each state to Equal Earth (EPSG:8857), split it into landmasses, sample a regular grid (spacing chosen per state so a province gets about 100 points, capped at 10 km), cluster with Lloyd's k-means started from evenly spaced points along a Morton curve, then repair clusters that came out in pieces. Province IDs are ordered by state tag, then north to south and west to east.
- **Province count per state:** `max(round(area / 9000), min(10, max(1, floor(area / 450))))`. This is a change from the guide's flat "minimum 10 per country": provinces are never cut below about 450 km2 (two tactical cells), so Andorra (452 km2) gets 1, Luxembourg gets 5, and states from 4,500 km2 up get the full 10. The allocation across a state's landmasses uses the largest-remainder method so the total is exact.
- **Islands:** landmasses smaller than 0.6 of a province are pooled with neighbours within 1,500 km (single linkage) into island-group provinces, which may be several pieces joined by sea links. Example: Alaska's 191-island chain forms the largest province (28,705 km2). Every landmass above that size gets its own province.
- **Province area is an estimate** (points x spacing squared). Exact areas come from the cells in the next step. States' sampled areas are checked against true area (within 15% for states over 2,000 km2; no warnings).
- **Bug found and fixed during the step:** the fragment repair first ignored neighbours on the edge of a province's bounding box (141 split provinces); fixed by widening the box (14 remain, all thin coastal strips, accepted).
- **Known issues, to handle later:**
  - Natural Earth cuts polygons at the antimeridian, so Chukotka, Fiji and parts of Alaska's Aleutians are separate landmasses with an artificial seam; the adjacency step must wrap across it.
  - Greenland (about 240 provinces) and the Canadian Arctic mostly ice and tundra: a gameplay question, not an engine one.
  - French Guiana is part of the France polygon, so France's 83 provinces include it; same pattern for other overseas parts (logged for the regions layer).
  - Special territories (Berlin, Palestine and similar) get their own pass later, as agreed.
- **Outputs** (all in `data/intermediate/`, not committed): `provinces.csv` (id, state, lon, lat, estimated area, points, landmass count, spacing) and the labelled sample points as `.npy` files. Output is byte-identical across runs.
- **Pipeline:** `crates/map-builder/build_map.sh` runs fetch, states, provinces and the validator.
