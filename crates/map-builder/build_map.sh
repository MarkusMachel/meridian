#!/usr/bin/env bash
# Run the whole map-builder pipeline.
#   INTER  working directory (default data/intermediate)
#   BAKED  output file       (default data/baked/meridian-map.bin)
# Needs crates/map-builder/.venv (pip install -r requirements.txt) and downloads the sources.
set -euo pipefail
cd "$(dirname "$0")/../.."
INTER="${INTER:-data/intermediate}"
BAKED="${BAKED:-data/baked/meridian-map.bin}"
PY="crates/map-builder/.venv/bin/python -I -W ignore"
B=crates/map-builder
NE=data/source/natural-earth
$B/fetch_sources.sh
mkdir -p "$INTER"
$PY $B/build_states.py $NE "$INTER"
$PY $B/build_provinces.py "$INTER"
$PY $B/validate_provinces.py "$INTER"
$PY $B/build_cells.py "$INTER"
$PY $B/validate_cells.py "$INTER"
$PY $B/build_sea.py "$INTER" $NE
$PY $B/validate_graph.py "$INTER"
$PY $B/build_terrain.py "$INTER" $NE data/source/etopo/ETOPO_2022_v1_60s_N90W180_surface.tif
$PY $B/build_sites.py "$INTER" $NE
$PY $B/build_population.py "$INTER" $NE data/source/worldbank
$PY $B/bake_map.py "$INTER" "$BAKED"
