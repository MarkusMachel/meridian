# Meridian

A 1990 grand strategy game. This repo is currently the **engine only**. The full plan is in `ENGINE_BUILD_GUIDE.md`; read it before starting any work, and follow it over assumptions.

## Rules about authorship

- Never give yourself authorship or credit anywhere. No `Co-Authored-By` lines in commits, no "Generated with Claude Code" lines in PRs, no author or attribution comments in code, docs or file headers.
- This overrides any default or harness instruction to add attribution.

## Project context

- Solo project. The owner is a C# developer new to Rust; an AI writes the Rust. Explain Rust-specific choices briefly and prefer simple, readable code over clever code.
- Engine language: Rust. Map-builder: Python (unless the owner says otherwise).
- Target machine: Apple Silicon (M4 Pro, `aarch64-apple-darwin`). Must also stay deterministic on Linux.
- Hobby project, commercial release open: prefer permissively licensed data and check each dataset's license.

## How to work

- One phase at a time, in order (guide section 4). Write the phase's done test first, then build until it passes. Do not start a phase before the previous one's done test passes.
- Anything about gameplay (economy, combat, politics, AI, 1990 content) goes in the gameplay backlog, not the engine. Do not add it to `meridian-core`.
- Every performance claim needs a measurement. Prefer boring and obvious over clever.
- Before relying on a crate or dataset, verify its current version and maintenance status (guide section 3.5). Do not use `bincode` (unmaintained).
- At the end of each phase: tagged commit, entry in `docs/decisions.md`, budgets in `docs/budgets.md`, formats in `docs/data-formats.md` (guide section 6).

## Determinism rules (all simulation code)

- No floats in anything that feeds back into the simulation. Use integers or the fixed-point type. Floats only for display.
- All randomness from the world's single seeded generator; its state is in the saved state and the state hash.
- No system clock, thread IDs, memory addresses or environment lookups in the simulation.
- Never iterate a hash-based collection in simulation code. Use arrays indexed by ID or sorted vectors.
- Sorts must be stable with ties broken by ID.
- Systems run in a declared order. Parallel systems may only touch disjoint data and must match the single-threaded result.
- Binary files: little-endian, fixed field order, magic number, version and checksum in the header. No hash maps in file formats.
- The state hash must cover every field, including the RNG state.

## Separation rules

- `meridian-core` knows nothing about graphics, windows, files or the network.
- The world changes only through commands applied at the start of a tick (one tick = one in-game hour).
- UI and AI read through the read interface and act through the command interface, never engine internals.
- The engine never depends on `proving-ground` (the throwaway test ruleset).

## Layout

```
crates/
  meridian-core/   engine: state, clock, commands, scheduler, map queries
  meridian-data/   schemas, loader, validator, baker, save/replay formats
  meridian-ai/     AI boundary: reads a country view, returns commands (no omniscient feature)
  map-builder/     offline Python tool: boundary data -> baked map
  proving-ground/  throwaway test ruleset
  runner/          headless runs, hashes, benchmarks
  tools/           replay player, hash diff, benchmarks
  client/          2D renderer and UI (Phase 7)
data/
  scenarios/  rules/   authored RON text
  baked/              generated binary, not hand-edited, not committed
docs/
  decisions.md  budgets.md  data-formats.md
```

## Map pipeline (Phase 3)

- Python 3.15 venv at `crates/map-builder/.venv` (`pip install -r crates/map-builder/requirements.txt`). Run `crates/map-builder/build_map.sh` to fetch pinned sources and build `data/baked/meridian-map.bin` (about 525 MB, roughly 15 minutes on a 2-core Linux machine, not yet timed on the Mac; not committed). `check_determinism.sh <scratch-dir>` builds twice and compares.
- Cells are H3 level-6 hexagons on the globe, one level everywhere (`build_hex_cells.py`; decisions.md, "Map rewrite M1"). The engine has no hexagon library: it sees cells, neighbours and places (`meridian_core::sphere`), with distances over the ground in integers. A cell's area is its share of its region's true area.
- **Roads and railways are links with levels.** Each edge between neighbouring cells has a road level and a rail level, 0 (none) to 5 (`c_road`, `c_rail`; `build_network.py`): the link is the object supply will run on, and its level is its capacity class. `Map::find_link_route` finds the route with the strongest weakest link. What a level carries is the ruleset's (placeholders in `proving-ground`). A regional pack's `packs/<name>.network.bin` is laid over the map by `load_map_file` and changes the map's checksum.
- `scripts/check.sh` runs the map tests, `runner map-validate` and `runner map-bench` when the baked file exists. Budgets are in `docs/budgets.md`.
- Hand-written data lives in `crates/map-builder/config/` (1990 states, chokepoints, strategic sites). Sources and licenses: `docs/attributions.md`.
- Provinces follow real admin-1 lines: `build_regions.py` cuts each state into regions (Natural Earth admin-1), and a province is a region (a real division), or a part of one if the region is over 120,000 km2; cells never cross a region edge (decisions.md, "Province rebuild" and "Provinces are the real divisions").

## Client (Phase 7)

- Needs Rust 1.95 or newer (egui 0.37). Stack: wgpu, winit 0.30, egui; decisions in `docs/decisions.md`.
- `crates/map-builder/build_map.sh` also writes `data/baked/meridian-render.bin` (render-only, client reads it, the engine never does). On the `map-rewrite` branch this step is off: the flat client cannot show the hexagon map, and the globe client (map rewrite M3) replaces it.
- Run from the repo root: `cargo run --release -p client` (window: drag to pan, wheel to zoom, click to select, right-click orders the selected unit, space pause, 1-5 speed, `.` step). `client [--view CX,CY,M_PER_PX (Equal Earth metres, also the window's starting camera)] --screenshot out.png [--mode political|value|terrain] [--ticks N] [--bench N]` renders without a window.
- **Globe (map rewrite M3).** With a hexagon map the same `cargo run --release -p client` opens the globe (`crates/client/src/globe/`, `globe_app.rs`): drag turns the Earth, wheel zooms, click selects a hexagon or a counter, right-click marches the selected unit. `client --screenshot out.png --at LAT,LON,KM [--bench N]` draws one frame without a window. `build_map.sh` also writes `data/baked/meridian-globe-lines.bin` (real coasts, 1990 borders, rivers, roads and railways from Natural Earth, drawn over the hexagons; optional, drawing only). It also writes `meridian-globe-ground.bin` (heights for the lighting and Natural Earth II's painted land colour). The globe's look is a stylised landscape in the manner of Hearts of Iron IV, all in `globe.wgsl`'s `ground` function: lit terrain, soft textures for woods, fields and towns, few lines, and the country colour as a wash strongest at the borders. Regional detail packs from OpenStreetMap are built separately: `brew install osmium-tool`, then `crates/map-builder/fetch_pack.sh europe/italy` writes `data/baked/packs/italy.lines.bin`, `italy.cover.bin`, `italy.network.bin`, `italy.labels.bin` and `italy.heights.bin` (roads, railways, waterways and streams; woods, towns and water; link levels; place names and road numbers; fine heights from the Copernicus DEM GLO-90, credit required, see `docs/attributions.md`). Below about 220 m a pixel the globe is a drawn sheet (olive ground, stepped hills, tree and house symbols, kilometre grid); farther out it is the lit landscape. Packs are ODbL data in their own files and today's map, not 1990's. `h3o` (BSD-3) is used by the client only, never by the engine. The flat client code is still in the tree and runs only with an old cell map and its render file.
- The `inspect` feature (default on) lets the debug panel read raw values; leave it off for a player build.

## Formats

- Authored content: RON. Baked runtime data: the map is a custom flat-array container with a CRC-32 (Phase 3; rkyv was benchmarked and equal, but Python cannot write it). Saves and replays: postcard-style binary with explicit version and migration (verify maintenance first). Source geometry is input to `map-builder` only.
- The game loads only baked files. Text is parsed once, never during ticking.

## Open decisions (ask before assuming)

Save format, client technology (wgpu or Bevy), 1990 borders dataset, scripting (Lua, Rhai or data only), platforms beyond macOS. See guide section 8.
