# Budgets

Measured on an Apple M4 Pro, release build (`cargo build --release -p runner`, then
`target/release/runner map-bench data/baked/meridian-map.bin`). Budgets are about 3 times the
measurement, so normal noise passes and a real regression fails. `runner map-bench` exits
non-zero when a budget is exceeded; set `MERIDIAN_BUDGET_SCALE` (for example 3) on slower or
emulated machines.

## Map (set in Phase 3, 2026-11-08)

Map size: 16,182 provinces (in 3,019 regions), 862 sea zones, 599,300 cells, 2,492 sites. File 74.7 MB.

| What | Measured | Budget |
| --- | --- | --- |
| Load from disk, checksum, parse, validate structure, build indices (median of 5) | 35 to 38 ms | 120 ms |
| `validate_world()` (whole-world rules) | 1 ms | not budgeted |
| Memory after load (peak resident set, includes the file bytes during parsing) | 160 MB | 400 MB |
| Strategic path query (any two of 16,102 nodes), mean over 10,000 | 219 us | 700 us |
| Strategic path query, 99th percentile | 1.0 ms | 3 ms |
| Local cell path query (target within 400 km, same landmass), mean over 10,000 | 57 us | 200 us |
| Local cell path query, 99th percentile | 263 us | 1 ms |
| Long cell path query (up to 3,000 km), mean over 100 | 2.5 ms | 10 ms |

Notes:
- Strategic queries average about 1,080 nodes expanded; local cell queries about 360.
- A random pair of nodes is always connected (10,000 of 10,000 found), so the world is one graph.
- Long queries have a heavy tail (99th percentile about 52 ms for the longest land crossing). Gameplay should not issue these every tick; the strategic graph is for long routes.
- Costs are set per query (`NodeCosts`, `CellCosts`), so these numbers are for the default cost tables.

## Phase 4A performance probe (2026-11-09, stub state, M4 Pro)

`runner probe [threads]` (release mode) builds a stand-in world: 599,261 cells with a 2-byte controller and four 1-byte fields, 599,261 population group records of 24 bytes, 190 countries. It is a shape check, not the real state: the real systems will do far more work per cell. Stub state is 18 MB.

| What | Measured (8 threads) | Note |
| --- | --- | --- |
| Hash of all stub state | 19.5 ms | Byte-at-a-time FNV. Fine on demand (saves, desync checks); too slow to hash every tick |
| Daily pass over every cell and group, 1 thread | 0.45 ms | Trivial arithmetic, so memory-bound |
| Same pass, 8 threads | 0.41 ms | No gain at this cost per element: thread start-up eats it. Results identical |
| Hourly pass over an active set of 29,216 cells (5%) | 0.015 ms | |
| 190 stub country passes of 2,000 integer ops | 0.58 ms | |

What this tells us:
- The planned state size (about 18 MB for cells and groups) is cheap to sweep. A full pass is well under a millisecond, so active sets are an optimization to add when a real system needs them, not a necessity.
- Parallelism does not help for trivial per-element work. Only use it for systems that measure slower than a few milliseconds single-threaded.
- Hashing is the one real cost: 19.5 ms. Do not hash every tick. If per-tick hashing is wanted for desync checks, switch to a faster hash or hash only changed regions; decide in Phase 5 or 8.
- No budget is set yet. Real budgets come when the real systems exist (Phase 8).

## Phase 5 save sizes (2026-11-09)

Measured, not budgeted (no time budget is set for save and load yet; set in Phase 8 with real systems):

| Save | Size |
| --- | --- |
| Proving-ground world (64 provinces, 6 units), tick 2000 | 441 bytes |
| Map world, fresh (16,182 provinces, 173 countries, no play) | 35,933 bytes |
| Map world after opening one province and changing 5 cells | 36,039 bytes |

Per-cell state is stored as differences from the starting state, so the 599,261 cells add nothing until they change. Save and replay files are byte-identical on macOS and Linux arm64 for the same run (checked with sha256).

## Not yet set

Per-tick and per-system time budgets and the simulation memory budget are set in Phase 8.

## Phase 6: reads while the sim runs

- A query on the sim thread must finish within `MAX_QUERY_NS` = 2 ms (the sim runs queries between ticks). Measured on the full map: 17 µs for the heaviest ordinary read (a country's provinces), 300 queries during a 20,000-tick run.

## Render file (client data)

- `data/baked/meridian-render.bin` (version 2, Mercator, with cell rings): about 191 MB, built in about 40 s from the intermediate files, byte-identical across runs. Loads in about 55 ms (release). Level sizes: coarse 368k triangles, medium 768k, fine 960k.

## Client frame time (offscreen, with readback, Apple M4 Pro, 2560x1440, whole world drawn each frame)

| View | Level | Mean | p50 | p99 | Max |
| --- | --- | --- | --- | --- | --- |
| World | 0 | 8.3 ms | 8.5 | 10.6 | 10.6 |
| Regional (3 km/px) | 1 | 7.4 ms | 9.4 | 11.1 | 11.1 |
| Close (600 m/px) | 2 | 12.0 ms | 10.6 | 16.7 | 16.7 |
| Closest (150 m/px) | 2 | 11.4 ms | 10.0 | 25.3 | 25.3 |

Target: 60 fps (16.6 ms) at 2560x1440 with the whole world on screen. Met on average; the slowest close-up frames (16.7 and 25 ms) miss it. Windowed frame time with panels is shown in the top bar and not yet recorded. Cell edges are not culled; that is the first thing to fix if close-up frames miss 60 fps.
