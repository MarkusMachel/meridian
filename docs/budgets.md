# Budgets

Measured on an Apple M4 Pro, release build (`cargo build --release -p runner`, then
`target/release/runner map-bench data/baked/meridian-map.bin`). Budgets are about 3 times the
measurement, so normal noise passes and a real regression fails. `runner map-bench` exits
non-zero when a budget is exceeded; set `MERIDIAN_BUDGET_SCALE` (for example 3) on slower or
emulated machines.

## Map (set in Phase 3, 2026-11-08)

Map size: 4,008 provinces (3,019 regions, one province each unless huge), 862 sea zones, 599,300 cells, 2,492 sites. File 73.3 MB.

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
| Map world, fresh (4,008 provinces, 173 countries, no play) | 35,933 bytes |
| Map world after opening one province and changing 5 cells | 36,039 bytes |

Per-cell state is stored as differences from the starting state, so the 599,261 cells add nothing until they change. Save and replay files are byte-identical on macOS and Linux arm64 for the same run (checked with sha256).

## Not yet set

Per-tick and per-system time budgets and the simulation memory budget are set in Phase 8.

## Phase 6: reads while the sim runs

- A query on the sim thread must finish within `MAX_QUERY_NS` = 2 ms (the sim runs queries between ticks). Measured on the full map: 17 µs for the heaviest ordinary read (a country's provinces), 300 queries during a 20,000-tick run.

## Render file (client data)

- `data/baked/meridian-render.bin` (version 7, Equal Earth): about 353 MB (heights 61 MB, land colours 92 MB, relief 30 MB), built from the intermediate files, byte-identical across runs. Level sizes: coarse 368k triangles, medium 768k, fine 960k. Version 2 (Mercator) was about 292 MB and loaded in about 55 ms; the load time of version 7 is not re-measured.

## Client frame time (offscreen, Apple M4 Pro, 2560x1440)

`client --size 2560x1440 --view CX,CY,M --bench 60` (Europe, centre 1000000,5800000; world 0,0). The figure is GPU throughput: 60 frames submitted back to back with no readback, one wait at the end, best of 5 runs (range in brackets). Timing single frames with a readback each swung by about 40 % between identical runs while other programs used the GPU, so it is no longer the budget figure.

| View | Mode | Level | ms per frame |
| --- | --- | --- | --- |
| World (20 km/px) | flat paper map | 0 | 3.1 (3.1 to 3.3) |
| Regional (3 km/px) | partly tilted | 1 | 5.5 (5.5 to 8.0) |
| Close (600 m/px) | full 3D, trees, buildings | 2 | 7.8 (7.8 to 10.4) |
| Closest (150 m/px) | full 3D, trees, buildings | 2 | 7.7 (7.7 to 8.6) |

Target: 60 fps (16.6 ms) at 2560x1440. Met in every view. Before line culling the close 3D view took 20.2 ms: cell edges alone were 7.5 ms and roads and railways 2.7 ms, measured by switching each off. Lines are now drawn only in the world tiles in view (64 x 32 tiles). Of the remaining close-up cost, the terrain pass is about 4 ms and trees and buildings about 1 ms. Windowed frame time with panels is shown in the top bar and not yet recorded.

## Map rewrite: the level-6 hexagon world (measured 2026-11-10)

Measured on the machine this was written on (a 2-core cloud container, not the M4 Pro), so the times are an upper bound, not the budget. Re-measure on the build machine.

**The builder's first step** (`crates/map-builder/build_hexes.py`, Natural Earth 10 m land, north of 60° S):

| What | Value |
| --- | --- |
| Land hexagons at H3 level 6 | 3,686,556 |
| Their area against the land area of the source | 135.015 against 134.469 million km² (+0.41 %) |
| Neighbour pairs | 10,955,512; 97.4 % of hexagons have all six land neighbours |
| Distance to a neighbour | 4,489 to 7,095 m, mean 6,497 m |
| City-to-city distances (8 cities at hexagon centres) against the great circle | worst error 0.157 % |
| Time | 324 s, single-threaded Python |
| Arrays written (uncompressed) | 264 MB: H3 numbers 29 MB, centres 44 MB, neighbour offsets 15 MB, neighbours 88 MB, distances 88 MB (distances fit 16 bits, which would halve them) |

Filling a whole continent with hexagons in one piece did not finish in 16 minutes; cutting the land into 5-degree pieces first brought the fill down to 22 s.

**The engine's stub world at that size** (`runner probe 8 3686556`; the same probe as Phase 4A, which used 599,261 cells):

| What | 599,261 cells | 3,686,556 hexagons |
| --- | --- | --- |
| State hash over all stub state | 18.1 ms | 110.2 ms |
| Daily pass over every cell and group, one thread | 1.47 ms | 13.71 ms |
| Hourly pass over an active set of about 5 % of cells | 0.030 ms | 0.258 ms |
| Stub state (five byte arrays and one 24-byte population group per cell) | 18.0 MB | 110.6 MB |

So seven times the cells costs about six to nine times as much, as expected: a full daily sweep is still far inside a tick, and the state hash (already not taken every tick) takes about a tenth of a second. Population is planned per province, not per hexagon, which would take most of the 110 MB away.

**The whole map as hexagons (M1, 2026-11-10).** 4,008 provinces, 862 sea zones, 3,685,130 hexagons, 10,953,797 neighbour pairs, 2,492 sites. File 525 MB. Elevation was a stand-in (all zero: the elevation source cannot be downloaded on that machine), so terrain was plains, desert and ice only and path costs will change with real terrain. `runner map-bench`, same 2-core container:

| What | Measured there | Provisional budget |
| --- | --- | --- |
| Load from disk, checksum, parse, validate structure, build indices (median of 5) | 1.3 to 1.4 s | 4.5 s |
| `validate_world()` | 5 ms | not budgeted |
| Memory, peak resident set during load | 1.1 GB | not set |
| Strategic path query, mean over 10,000 (522 nodes expanded) | 102 us | 700 us (unchanged) |
| Strategic path query, 99th percentile | 0.54 ms | 3 ms (unchanged) |
| Local cell path query (within 400 km), mean over 10,000 (1,512 expanded; 360 on the old cells) | 332 to 364 us | 1.1 ms |
| Local cell path query, 99th percentile | 1.7 to 1.9 ms | 6 ms |
| Long cell path query (up to 3,000 km), mean over 100 | 21 to 23 ms (worst 0.54 to 0.62 s) | 70 ms |

The budgets in `runner map-bench` are these provisional ones (about 3 times the container's measurement). **Set them again from a run on the M4 Pro.** A route of 400 km now crosses about 60 hexagons instead of 25, so a local query costs about six times what it did; long routes belong on the strategic graph, as before. Range queries (`cells_within`) on the globe: 10,000 queries of 400 km radius plus 100 of 3,000 km took about 15 s before and under 1 s after comparing straight-line distances first (measured as the whole benchmark going from 113 s to 21 s).

Builder, same container, single-threaded, from the provinces onward: hexagons and provinces 400 s, cell checks 165 s, sea 6 s, terrain 103 s, sites 8 s, population 8 s, bake 7 s: about 12 minutes, peak memory about 3.5 GB (the terrain step works 200,000 cells at a time; in one piece it ran out of 7 GB). Two builds gave the same file (SHA-256 `b4688f9f...199d`). Hexagons cover 134.966 million km2 against 134.465 of real land (+0.37 %); recorded areas add up to the real land (every state within 0.02 % except the Vatican, which is 1 hectare).

**After M2 (rivers on hexagon sides, same day, same container).** 188,795 river sides, 17,663 bridges. `runner map-bench`: local cell query mean 326 us, 99th percentile 1.7 ms; long query mean 8.8 ms, worst 76 ms (down from 21 ms and 0.54 s: a pair with no land route is now refused from a table of connected land made on load, instead of searching the whole landmass). Path fingerprint on Linux with stand-in elevation: `e126a03ea3cca082`.

`runner map-run <map> 1 8760` (a year of the proving ground on the map, 2,601 units, five march orders a day of up to 150 km, recorded and replayed):

| What | Measured |
| --- | --- |
| A tick (no state hash) | mean 44 us, slowest 127 ms |
| A tick with five new march orders | mean 1.0 ms |
| State hash of the map world (taken every 240 ticks by the recorder) | 52 ms |
| The year, with recording | 15.8 s |
| Replay from a fresh world | 37 checkpoints and the final hash match |

Before two fixes the same run took 33 s: every march order made 44 MB of search memory afresh (about 7 ms; the world now keeps it between searches) and an order to a place across the sea searched the whole landmass (up to 1.1 s). No per-tick budget is set yet (Phase 8); these are the numbers to set it from, once measured on the M4 Pro.
