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
echo "OK"
