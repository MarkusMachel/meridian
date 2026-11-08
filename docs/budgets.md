# Budgets

Measured on an Apple M4 Pro, release build (`cargo build --release -p runner`, then
`target/release/runner map-bench data/baked/meridian-map.bin`). Budgets are about 3 times the
measurement, so normal noise passes and a real regression fails. `runner map-bench` exits
non-zero when a budget is exceeded; set `MERIDIAN_BUDGET_SCALE` (for example 3) on slower or
emulated machines.

## Map (set in Phase 3, 2026-11-08)

Map size: 15,240 provinces, 862 sea zones, 599,261 cells, 2,492 sites. File 74.5 MB.

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

## Not yet set

Per-tick and per-system time budgets and the simulation memory budget are set in Phase 8.
