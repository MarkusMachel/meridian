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
