#!/usr/bin/env bash
# Run the map-builder steps in order. Needs crates/map-builder/.venv (see requirements.txt)
# and the sources from fetch_sources.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."
PY="crates/map-builder/.venv/bin/python -I -W ignore"
B=crates/map-builder
$B/fetch_sources.sh
$PY $B/build_states.py data/source/natural-earth data/intermediate
$PY $B/build_provinces.py data/intermediate
$PY $B/validate_provinces.py data/intermediate
$PY $B/build_cells.py data/intermediate
$PY $B/validate_cells.py data/intermediate
$PY $B/build_sea.py data/intermediate data/source/natural-earth
$PY $B/validate_graph.py data/intermediate
$PY $B/build_terrain.py data/intermediate data/source/natural-earth data/source/etopo/ETOPO_2022_v1_60s_N90W180_surface.tif
$PY $B/build_sites.py data/intermediate data/source/natural-earth
$PY $B/build_population.py data/intermediate data/source/natural-earth data/source/worldbank
