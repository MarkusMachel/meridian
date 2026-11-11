#!/usr/bin/env bash
# Build a regional detail pack from OpenStreetMap (map rewrite M6, first cut).
#
#   crates/map-builder/fetch_pack.sh europe/italy
#
# Needs the `osmium` tool once:   brew install osmium-tool
# Downloads the Geofabrik extract (Italy: 2.1 GB), its coverage outline and, once, the world's
# land polygons (about 0.9 GB) into
# data/source/osm/, keeps the roads, railways, waterways and ground cover, and writes
# data/baked/packs/<name>.lines.bin and <name>.cover.bin. The download is today's map, so two
# runs on different days differ; the file's date and SHA-256 are written next to it.
#
# OpenStreetMap data is (c) OpenStreetMap contributors, Open Database License.
set -euo pipefail
cd "$(dirname "$0")/../.."
REGION="${1:?usage: fetch_pack.sh <geofabrik region, e.g. europe/italy>}"
NAME="$(basename "$REGION")"
SRC=data/source/osm
OUT=data/baked/packs
PY="crates/map-builder/.venv/bin/python -I -W ignore"
command -v osmium >/dev/null || { echo "the osmium tool is missing: brew install osmium-tool"; exit 1; }
$PY -c "import PIL" 2>/dev/null || { echo "Pillow is missing: crates/map-builder/.venv/bin/python -m pip install -r crates/map-builder/requirements.txt"; exit 1; }
mkdir -p "$SRC" "$OUT"

PBF="$SRC/$NAME-latest.osm.pbf"
POLY="$SRC/$NAME.poly"
[ -f "$PBF" ] || curl -fL --retry 3 -o "$PBF" "https://download.geofabrik.de/$REGION-latest.osm.pbf"
[ -f "$POLY" ] || curl -fL --retry 3 -o "$POLY" "https://download.geofabrik.de/$REGION.poly"
{ date -u +"downloaded or first used %Y-%m-%dT%H:%MZ"; shasum -a 256 "$PBF"; } > "$SRC/$NAME.source.txt"

# Land and sea: polygons made from OpenStreetMap's coastline for the whole world (about 0.9 GB,
# downloaded once and shared by every pack).
LAND="$SRC/land-polygons-split-4326/land_polygons.shp"
if [ ! -f "$LAND" ]; then
  [ -f "$SRC/land-polygons-split-4326.zip" ] || curl -fL --retry 3 -o "$SRC/land-polygons-split-4326.zip" "https://osmdata.openstreetmap.de/download/land-polygons-split-4326.zip"
  unzip -q -o "$SRC/land-polygons-split-4326.zip" -d "$SRC"
fi

if [ ! -f "$SRC/$NAME.geojsonseq" ] || [ "$PBF" -nt "$SRC/$NAME.geojsonseq" ]; then
echo "keeping roads, railways, waterways and ground cover..."
osmium tags-filter "$PBF" \
  w/highway=motorway,trunk,primary,secondary,tertiary,unclassified,motorway_link,trunk_link,primary_link \
  w/railway=rail \
  w/waterway=river,canal \
  wr/natural=water,wood \
  wr/landuse=forest,residential,industrial,commercial,retail,reservoir,basin \
  wr/waterway=riverbank \
  --overwrite -o "$SRC/$NAME-kept.osm.pbf"
echo "exporting geometry..."
osmium export "$SRC/$NAME-kept.osm.pbf" -f geojsonseq -x print_record_separator=false \
  --geometry-types=linestring,polygon --overwrite -o "$SRC/$NAME.geojsonseq"
fi
echo "building the pack..."
$PY crates/map-builder/build_pack.py "$NAME" "$SRC/$NAME.geojsonseq" "$POLY" "$OUT" "$LAND"
ls -la "$OUT"
