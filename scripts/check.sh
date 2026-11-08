#!/usr/bin/env bash
# One-command build and test. Run from anywhere.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
# Release run twice: the hashes must match (determinism across runs).
cargo build --release -p runner
a=$(./target/release/runner 1 8760)
b=$(./target/release/runner 1 8760)
c=$(./target/release/runner 2 8760)
echo "$a"; echo "$c"
[ "$a" = "$b" ] || { echo "FAIL: same seed gave different results"; exit 1; }
[ "${a##*hash=}" != "${c##*hash=}" ] || { echo "FAIL: different seeds gave same hash"; exit 1; }
./target/release/runner validate data/scenarios/sample.ron
# The baked world map (made by crates/map-builder/build_map.sh, not committed).
if [ -f data/baked/meridian-map.bin ]; then
  MERIDIAN_REQUIRE_MAP=1 cargo test --release -p meridian-data --test world_map
  ./target/release/runner map-validate data/baked/meridian-map.bin
  ./target/release/runner map-bench data/baked/meridian-map.bin
else
  echo "NOTE: data/baked/meridian-map.bin not found; skipped map tests (run crates/map-builder/build_map.sh)"
fi
echo "OK"
