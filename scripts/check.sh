#!/usr/bin/env bash
# One-command build and test. Run from anywhere.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
# Phase 6: the AI and client crates must compile without the `omniscient` feature. A workspace
# build unifies features and hides a violation, so check each crate on its own.
cargo check -p meridian-ai
cargo check -p test-client
cargo test -p test-client
# Release run twice: the hashes must match (determinism across runs).
cargo build --release -p runner
a=$(./target/release/runner 1 8760)
b=$(./target/release/runner 1 8760)
c=$(./target/release/runner 2 8760)
echo "$a"; echo "$c"
[ "$a" = "$b" ] || { echo "FAIL: same seed gave different results"; exit 1; }
[ "${a##*hash=}" != "${c##*hash=}" ] || { echo "FAIL: different seeds gave same hash"; exit 1; }
./target/release/runner validate data/scenarios/sample.ron
# Phase 5: save/load and replay round trips (scratch files go in a temp directory).
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
./target/release/runner record 4 3000 "$tmp/a.mrrp"
./target/release/runner replay "$tmp/a.mrrp"
./target/release/runner save 4 2000 "$tmp/s.mrsv"
l=$(./target/release/runner load "$tmp/s.mrsv" 1000)
s4=$(./target/release/runner 4 3000)
echo "$l"
[ "${l##*hash=}" = "${s4##*hash=}" ] || { echo "FAIL: save at tick 2000 + 1000 ticks differs from an uninterrupted run"; exit 1; }
# The baked world map (made by crates/map-builder/build_map.sh, not committed).
if [ -f data/baked/meridian-map.bin ]; then
  MERIDIAN_REQUIRE_MAP=1 cargo test --release -p meridian-data --test world_map
  ./target/release/runner map-validate data/baked/meridian-map.bin
  ./target/release/runner map-bench data/baked/meridian-map.bin
  # Map rewrite M2: rivers, bridges and marches on the hexagon map; a year with marching units replays to the same hashes.
  MERIDIAN_REQUIRE_MAP=1 cargo test --release -p runner --test hex_world
  ./target/release/runner map-run data/baked/meridian-map.bin 1 8760
  # Map rewrite M3: the globe. Picking and hexagon triangles on the real map, and one frame
  # drawn without a window.
  MERIDIAN_REQUIRE_MAP=1 cargo test --release -p client --test globe
  cargo build --release -p client
  ./target/release/client --screenshot "$tmp/globe.png" --size 800x450 --at 52.4,9.7,400
  [ -s "$tmp/globe.png" ] || { echo "FAIL: client wrote no picture of the globe"; exit 1; }
  if [ -f data/baked/meridian-render.bin ]; then
    # Client smoke test: draw one frame without a window and check that a picture was written.
    cargo build --release -p client
    ./target/release/client --screenshot "$tmp/shot.png" --size 800x450 --ticks 24
    [ -s "$tmp/shot.png" ] || { echo "FAIL: client wrote no picture"; exit 1; }
    crates/map-builder/.venv/bin/python -I -W ignore crates/map-builder/validate_render.py data/intermediate data/baked/meridian-render.bin
  fi
else
  echo "NOTE: data/baked/meridian-map.bin not found; skipped map tests (run crates/map-builder/build_map.sh)"
fi
echo "OK"
