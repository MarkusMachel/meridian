#!/usr/bin/env bash
# Phase 3 done test: building the map twice from the same inputs gives byte-identical output.
set -euo pipefail
cd "$(dirname "$0")/../.."
T="${1:?usage: check_determinism.sh <scratch-dir>}"
rm -rf "$T/runA" "$T/runB"; mkdir -p "$T"
INTER="$T/runA/inter" BAKED="$T/runA/map.bin" crates/map-builder/build_map.sh > "$T/runA.log" 2>&1
INTER="$T/runB/inter" BAKED="$T/runB/map.bin" crates/map-builder/build_map.sh > "$T/runB.log" 2>&1
a=$(shasum -a 256 "$T/runA/map.bin" | cut -c1-64); b=$(shasum -a 256 "$T/runB/map.bin" | cut -c1-64)
echo "run A: $a"; echo "run B: $b"
[ "$a" = "$b" ] && echo "IDENTICAL" || { echo "DIFFERENT"; exit 1; }
