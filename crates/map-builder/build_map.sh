#!/usr/bin/env bash
# Run the whole map-builder pipeline.
#   INTER  working directory (default data/intermediate)
#   BAKED  output file       (default data/baked/meridian-map.bin)
#   RENDER render-only file for the client (default data/baked/meridian-render.bin)
# Needs crates/map-builder/.venv (pip install -r requirements.txt) and downloads the sources.
set -euo pipefail
cd "$(dirname "$0")/../.."
INTER="${INTER:-data/intermediate}"
BAKED="${BAKED:-data/baked/meridian-map.bin}"
RENDER="${RENDER:-data/baked/meridian-render.bin}"
PY="crates/map-builder/.venv/bin/python -I -W ignore"
B=crates/map-builder
NE=data/source/natural-earth
$B/fetch_sources.sh
mkdir -p "$INTER"
$PY $B/build_states.py $NE "$INTER"
$PY $B/build_regions.py $NE "$INTER"
$PY $B/build_provinces.py "$INTER"
$PY $B/validate_provinces.py "$INTER"
# Map rewrite M1: the cells are H3 level-6 hexagons on the globe (build_cells.py made the old
# irregular cells and is kept only for the helpers the other steps import from it).
$PY $B/build_hex_cells.py "$INTER" $NE
$PY $B/validate_cells.py "$INTER"
$PY $B/build_sea.py "$INTER" $NE
$PY $B/validate_graph.py "$INTER"
$PY $B/build_terrain.py "$INTER" $NE data/source/etopo/ETOPO_2022_v1_60s_N90W180_surface.tif
$PY $B/build_sites.py "$INTER" $NE
$PY $B/build_population.py "$INTER" $NE data/source/worldbank
$PY $B/bake_map.py "$INTER" "$BAKED"
# The real coasts, borders, rivers, roads and railways for the globe view (drawing only).
$PY $B/build_globe_lines.py "$INTER" $NE "$(dirname "$BAKED")/meridian-globe-lines.bin"
# Shaded relief for the globe view, from the elevation grid the terrain step keeps.
$PY $B/build_globe_relief.py "$INTER" "$(dirname "$BAKED")/meridian-globe-relief.bin"
# The render file is for the flat client, which cannot show the hexagon world; the globe client
# (map rewrite M3) replaces it. Set RENDER_OLD=1 to try the old step anyway.
if [ "${RENDER_OLD:-0}" = 1 ]; then
  $PY $B/build_render.py "$INTER" "$RENDER" --ne $NE
  $PY $B/validate_render.py "$INTER" "$RENDER"
fi
