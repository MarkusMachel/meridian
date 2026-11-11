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

# The export is made again when the extract is newer or when this script asks for other
# features than last time (EXPORT below is raised whenever the lists change).
EXPORT=2
if [ ! -f "$SRC/$NAME.geojsonseq" ] || [ "$PBF" -nt "$SRC/$NAME.geojsonseq" ] || [ "$(cat "$SRC/$NAME.export" 2>/dev/null)" != "$EXPORT" ]; then
echo "keeping roads, railways, waterways and ground cover..."
osmium tags-filter "$PBF" \
  w/highway=motorway,trunk,primary,secondary,tertiary,unclassified,motorway_link,trunk_link,primary_link \
  w/railway=rail \
  w/waterway=river,canal,stream \
  wr/natural=water,wood \
  wr/landuse=forest,residential,industrial,commercial,retail,reservoir,basin \
  wr/waterway=riverbank \
  --overwrite -o "$SRC/$NAME-kept.osm.pbf"
echo "exporting geometry..."
osmium export "$SRC/$NAME-kept.osm.pbf" -f geojsonseq -x print_record_separator=false \
  --geometry-types=linestring,polygon --overwrite -o "$SRC/$NAME.geojsonseq"
# the names of cities, towns and villages, as points (kept apart so that the many other
# tagged points of the extract are not exported)
osmium tags-filter "$PBF" n/place=city,town,village --omit-referenced --overwrite -o "$SRC/$NAME-places.osm.pbf"
osmium export "$SRC/$NAME-places.osm.pbf" -f geojsonseq -x print_record_separator=false \
  --geometry-types=point --overwrite -o "$SRC/$NAME-places.geojsonseq"
cat "$SRC/$NAME-places.geojsonseq" >> "$SRC/$NAME.geojsonseq"
echo "$EXPORT" > "$SRC/$NAME.export"
fi

# Finer heights than the world's, for the hills of the close zoom: Copernicus DEM GLO-90, one
# file per degree square (about 5 MB each; a square that is all sea has none). Credit, which
# the licence requires wherever the data is used: "(c) DLR e.V. 2010-2014 and (c) Airbus Defence
# and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights
# reserved". It is in docs/attributions.md and in the client's Settings window.
DEM=data/source/copernicus-dem-90
mkdir -p "$DEM"
echo "fetching elevation tiles..."
got=0
for tile in $($PY -c "
import math, sys
lon, lat = [], []
for line in open(sys.argv[1], encoding='utf-8').read().splitlines()[1:]:
    w = line.split()
    if len(w) >= 2:
        try:
            lon.append(float(w[0])); lat.append(float(w[1]))
        except ValueError:
            pass
for la in range(math.floor(min(lat)), math.ceil(max(lat))):
    for lo in range(math.floor(min(lon)), math.ceil(max(lon))):
        print('Copernicus_DSM_COG_30_%s%02d_00_%s%03d_00_DEM' % ('N' if la >= 0 else 'S', abs(la), 'E' if lo >= 0 else 'W', abs(lo)))
" "$POLY"); do
  if [ ! -f "$DEM/$tile.tif" ]; then
    # A square the server does not have (the sea: curl's exit code 22) is remembered as an
    # empty file so it is not asked for again. Any other failure leaves nothing behind, and
    # the square is tried again on the next run.
    code=0
    curl -sfL --retry 3 -o "$DEM/$tile.part" "https://copernicus-dem-90m.s3.amazonaws.com/$tile/$tile.tif" || code=$?
    if [ "$code" -eq 0 ]; then
      mv "$DEM/$tile.part" "$DEM/$tile.tif"
    else
      rm -f "$DEM/$tile.part"
      if [ "$code" -eq 22 ]; then : > "$DEM/$tile.tif"; else echo "  could not fetch $tile (curl exit $code)"; fi
    fi
  fi
  [ -s "$DEM/$tile.tif" ] && got=$((got + 1))
done
echo "elevation tiles on disk: $got"
[ "$got" -gt 0 ] || echo "WARNING: no elevation tile could be downloaded; the pack is built without fine heights"

echo "building the pack..."
$PY crates/map-builder/build_pack.py "$NAME" "$SRC/$NAME.geojsonseq" "$POLY" "$OUT" "$LAND" "$DEM"
ls -la "$OUT"
