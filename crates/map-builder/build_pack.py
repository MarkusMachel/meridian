"""A regional detail pack from OpenStreetMap (map rewrite M6, first cut): real roads, railways
and waterways at close zoom, and the ground cover (water, woods, built-up areas) as a fine
picture. Drawing only; the engine never reads a pack.

Usage: python -I build_pack.py <name> <features.geojsonseq> <coverage.poly> <out-dir> [<land_polygons.shp> [<elevation-tiles-dir>]]

`fetch_pack.sh` makes the inputs: it downloads a Geofabrik extract and its coverage outline and
runs the `osmium` tool to keep and export the features used here.

Writes three files:
  <out-dir>/<name>.lines.bin   the globe lines container (see build_globe_lines.py), with two
                               more arrays: `info` (u32: how far a bucket reaches, in millionths
                               of a radian) and the coverage outline `cover` (i32 longitude,
                               latitude in millionths of a degree) with `cover_off` (u32, where
                               each ring starts, rings + 1 entries). Inside the outline the
                               client hides the coarse world roads, railways and small rivers.
  <out-dir>/<name>.cover.bin   container kind 6, version 2: the ground cover picture, cut in
                               square tiles. Arrays: `grid` (i32: west edge and north edge in
                               millionths of a degree, pixel width and height in billionths of a
                               degree, picture width and height in pixels, tile size in pixels,
                               tiles across, tiles down), `tile_layer` (u32 per tile, row by row
                               from the north-west: which stored tile it is, or 0xFFFFFFFF for an
                               all-sea one), `tiles` (u8, stored tiles one after another, each
                               tile * tile bytes, rows north to south).
                               Classes: 0 sea, 1 inland water, 2 woods, 3 built-up, 4 industry,
                               6 other land, 7 land outside the pack's coverage. Land and sea come from the land polygons made from
                               OpenStreetMap's coastline (osmdata.openstreetmap.de); without that
                               file everything is taken for land.

  <out-dir>/<name>.network.bin container kind 7, version 1: the region's roads and railways as
                               links between neighbouring hexagons, with levels (see
                               build_network.py). Unlike the other two this file feeds the
                               simulation: the map loader lays it over the baked map. Arrays:
                               `cells` (u64, the H3 numbers of the hexagons in the region,
                               ascending), `link_a`, `link_b` (u64 H3 numbers, a < b, ascending
                               by pair), `road`, `rail` (u8 levels, 0 = none).

  <out-dir>/<name>.labels.bin  container kind 9, version 1: names to write on the map. Arrays:
                               `label_xyz` (i32, three per label: its place), `label_kind` (u8:
                               0 city, 1 town, 2 village, 3 road number, 4 river name) and the
                               texts as `label_text_blob` (u8) and `label_text_off` (u32).
  <out-dir>/<name>.heights.bin container kind 8, version 1 (only with elevation tiles): the
                               height of the land, finer than the world's, in the same tile
                               layout as the ground cover but as u16 metres and with pixels
                               twice as large. Arrays `grid`, `tile_layer`, `tiles` as above;
                               a tile that is not stored is all at sea level.
                               From the Copernicus DEM GLO-90 (see fetch_pack.sh for the credit).

OpenStreetMap is (c) OpenStreetMap contributors, Open Database License. The data is today's
world, not 1990's: motorways and suburbs built since then are in it.
"""
import json
import sys
from pathlib import Path

import h3
import numpy as np
import shapely
from PIL import Image, ImageDraw
from shapely.geometry import shape

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_globe_lines as gl  # noqa: E402
import build_network  # noqa: E402
import container  # noqa: E402

# line styles (the client's colours and widths) and detail tiers, by OpenStreetMap tag
# A map shows few lines: motorways from far off, main roads from a region down, secondary
# roads close up. Tier 5 (small roads, canals) is only drawn when the player asks for it.
ROADS = {
    "motorway": (4, 1), "trunk": (4, 2), "motorway_link": (4, 4), "trunk_link": (4, 4),
    "primary": (5, 3), "primary_link": (5, 4), "secondary": (6, 4),
    "tertiary": (8, 5), "unclassified": (8, 5),
}
RAIL = (7, 3)
WATERWAYS = {"river": (3, 2), "canal": (3, 5), "stream": (9, 4)}      # style 9: a stream, a thin line close up
SIMPLIFY_DEG = 0.00008          # about 8 m: under half a pixel at the closest zoom
BUCKET_LEVEL = 4                # hexagons about 45 km across
BUCKET_REACH_MICRORAD = 6_500
# the ground cover picture: pixel size in degrees (about 79 m by 83 m at 45 N) and tile size
PIXEL_LON, PIXEL_LAT = 0.001, 0.00075
TILE = 2048
SEA, WATER, WOODS, BUILT, INDUSTRY, LAND, OUTSIDE = 0, 1, 2, 3, 4, 6, 7
MIN_AREA_DEG2 = 4e-7            # areas under about 0.4 hectare are left out
# the heights picture: pixels twice the ground cover's (about 160 m)
HEIGHT_PIXEL_LON, HEIGHT_PIXEL_LAT = 2 * PIXEL_LON, 2 * PIXEL_LAT
PLACES = {"city": 0, "town": 1, "village": 2}
ROAD_NUMBER, RIVER_NAME = 3, 4
LABEL_SPREAD_LEVEL = 5          # one road number or river name per hexagon of about 17 km


# The simulation's links (build_network.py): a road's level by its class, a railway's by what
# kind of line it is.
ROAD_LEVEL = {"motorway": 5, "trunk": 4, "primary": 3, "secondary": 2, "tertiary": 1, "unclassified": 1}


def number(text, default=0.0):
    try:
        return float(str(text).split()[0].split(";")[0])
    except (ValueError, IndexError):
        return default


def rail_level(tags):
    """1 for being a railway, and one more each for: a main line, two or more tracks,
    electrified, built for 200 km/h or more."""
    level = 1
    level += tags.get("usage") == "main"
    level += number(tags.get("tracks", tags.get("passenger_lines", 1)), 1.0) >= 2
    level += tags.get("electrified", "no") not in ("no", "")
    level += number(tags.get("maxspeed", 0)) >= 200 or tags.get("highspeed") == "yes"
    return int(level)


def cover_class(tags):
    if tags.get("natural") == "water" or tags.get("waterway") == "riverbank" or tags.get("landuse") in ("reservoir", "basin"):
        return WATER
    if tags.get("natural") == "wood" or tags.get("landuse") == "forest":
        return WOODS
    if tags.get("landuse") == "residential":
        return BUILT
    if tags.get("landuse") in ("industrial", "commercial", "retail"):
        return INDUSTRY
    return 0


def read_poly(path):
    """Rings of an Osmosis .poly file (Geofabrik's coverage outlines); holes are skipped."""
    rings, ring, skip = [], None, False
    for line in Path(path).read_text(encoding="utf-8").splitlines()[1:]:
        word = line.strip()
        if not word:
            continue
        if word == "END":
            if ring is not None:
                if not skip and len(ring) >= 3:
                    rings.append(np.array(ring))
                ring = None
            continue
        if ring is None:
            ring, skip = [], word.startswith("!")
            continue
        lon, lat = word.split()[:2]
        ring.append((float(lon), float(lat)))
    if not rings:
        sys.exit(f"{path}: no outline found")
    return rings


def polygons(geom):
    if geom.geom_type == "Polygon":
        return [geom]
    if geom.geom_type == "MultiPolygon":
        return list(geom.geoms)
    return []


def draw_land(land_file, pen, to_px, box):
    """Draw the land polygons that touch the box (west, south, east, north) onto a sheet."""
    import shapefile
    r = shapefile.Reader(str(land_file))
    n = 0
    for s in r.iterShapes():
        if s.shapeType == 0:
            continue
        w, so, e, no = s.bbox
        if e < box[0] or w > box[2] or no < box[1] or so > box[3]:
            continue
        for poly in polygons(shape(s.__geo_interface__)):
            pen.polygon(to_px(poly.exterior.coords), fill=255)
            for hole in poly.interiors:
                pen.polygon(to_px(hole.coords), fill=0)
            n += 1
    return n


def tiles_of(grid, tile):
    """Cut a picture into square tiles; all-zero tiles are not stored. Returns the stored
    tiles, the table saying which stored tile each one is, and how many across and down."""
    height, width = grid.shape
    across, down = -(-width // tile), -(-height // tile)
    table = np.full(across * down, 0xFFFFFFFF, dtype=np.uint32)
    stored = []
    for ty in range(down):
        for tx in range(across):
            part = grid[ty * tile:(ty + 1) * tile, tx * tile:(tx + 1) * tile]
            if not part.any():
                continue
            full = np.zeros((tile, tile), dtype=grid.dtype)
            full[:part.shape[0], :part.shape[1]] = part
            table[ty * across + tx] = len(stored)
            stored.append(full)
    return stored, table, across, down


def heights_picture(dem_dir, west, south, east, north):
    """The land's height in metres on a grid of HEIGHT_PIXEL degrees, from one-degree elevation
    tiles (Copernicus DEM GLO-90 as cloud-optimised GeoTIFFs). A missing tile is sea."""
    import tifffile
    width = int(np.ceil((east - west) / HEIGHT_PIXEL_LON)); height = int(np.ceil((north - south) / HEIGHT_PIXEL_LAT))
    out = np.zeros((height, width), dtype=np.uint16)
    used = 0
    for lat0 in range(int(np.floor(south)), int(np.ceil(north))):
        for lon0 in range(int(np.floor(west)), int(np.ceil(east))):
            ns = f"{'N' if lat0 >= 0 else 'S'}{abs(lat0):02d}"; ew = f"{'E' if lon0 >= 0 else 'W'}{abs(lon0):03d}"
            path = Path(dem_dir) / f"Copernicus_DSM_COG_30_{ns}_00_{ew}_00_DEM.tif"
            if not path.exists() or path.stat().st_size == 0:
                continue
            src = np.asarray(tifffile.imread(str(path)), dtype=np.float32)
            # the picture's pixels are about two of the source's across: average each source pixel
            # with its eight neighbours first, so single rough pixels do not become false hills
            edge = np.pad(src, 1, mode="edge")
            src = sum(edge[dy:dy + src.shape[0], dx:dx + src.shape[1]] for dy in range(3) for dx in range(3)) / 9.0
            rows, cols = src.shape[:2]
            # the picture's pixels whose middles fall in this tile
            x0 = max(0, int(np.ceil((lon0 - west) / HEIGHT_PIXEL_LON - 0.5))); x1 = min(width, int(np.ceil((lon0 + 1 - west) / HEIGHT_PIXEL_LON - 0.5)))
            y0 = max(0, int(np.ceil((north - (lat0 + 1)) / HEIGHT_PIXEL_LAT - 0.5))); y1 = min(height, int(np.ceil((north - lat0) / HEIGHT_PIXEL_LAT - 0.5)))
            if x1 <= x0 or y1 <= y0:
                continue
            lon = west + (np.arange(x0, x1) + 0.5) * HEIGHT_PIXEL_LON
            lat = north - (np.arange(y0, y1) + 0.5) * HEIGHT_PIXEL_LAT
            cx = np.clip((lon - lon0) * cols - 0.5, 0, cols - 1); cy = np.clip((lat0 + 1 - lat) * rows - 0.5, 0, rows - 1)
            ix = np.minimum(cx.astype(np.int64), cols - 2); iy = np.minimum(cy.astype(np.int64), rows - 2)
            fx = (cx - ix)[None, :]; fy = (cy - iy)[:, None]
            a = src[np.ix_(iy, ix)]; b = src[np.ix_(iy, ix + 1)]; c = src[np.ix_(iy + 1, ix)]; d = src[np.ix_(iy + 1, ix + 1)]
            h = (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy
            out[y0:y1, x0:x1] = np.clip(np.rint(h), 0, 65535).astype(np.uint16)
            used += 1
    return out, used


def main(name, features_file, poly_file, out_dir, land_file=None, dem_dir=None):
    out_dir = Path(out_dir)
    rings = read_poly(poly_file)
    west = min(r[:, 0].min() for r in rings); east = max(r[:, 0].max() for r in rings)
    south = min(r[:, 1].min() for r in rings); north = max(r[:, 1].max() for r in rings)
    width = int(np.ceil((east - west) / PIXEL_LON)); height = int(np.ceil((north - south) / PIXEL_LAT))
    print(f"{name}: {west:.3f} to {east:.3f} E, {south:.3f} to {north:.3f} N; ground cover picture {width} x {height}")
    # one drawing sheet per class; later classes are laid over earlier ones
    sheets = {c: Image.new("L", (width, height), 0) for c in (WOODS, BUILT, INDUSTRY, WATER)}
    pens = {c: ImageDraw.Draw(img) for c, img in sheets.items()}

    def to_px(coords):
        xy = np.asarray(coords)[:, :2]
        return list(zip(((xy[:, 0] - west) / PIXEL_LON).tolist(), ((north - xy[:, 1]) / PIXEL_LAT).tolist()))

    if land_file:
        land_sheet = Image.new("L", (width, height), 0)
        n_land = draw_land(land_file, ImageDraw.Draw(land_sheet), to_px, (west, south, east, north))
        print(f"land polygons drawn: {n_land:,}")
        if n_land == 0:
            sys.exit(f"{land_file}: no land polygon touches the region")
    else:
        land_sheet = None
        print("no land polygons given: everything inside the picture is taken for land")

    lines, counts, areas, bad = [], {}, {}, 0
    labels = {}        # (kind, text, hexagon) -> (lon, lat): one label of a kind and text per hexagon
    network = build_network.Levels()
    with open(features_file, encoding="utf-8") as f:
        for n, row in enumerate(f):
            row = row.strip().lstrip("\x1e")
            if not row:
                continue
            try:
                feature = json.loads(row)
                tags = feature.get("properties") or {}
                geom = shape(feature["geometry"])
            except (ValueError, KeyError, TypeError, AttributeError):
                bad += 1
                continue
            kind = geom.geom_type
            if kind == "Point":
                place = PLACES.get(tags.get("place"))
                if place is not None and tags.get("name"):
                    labels[(place, str(tags["name"]), len(labels))] = (geom.x, geom.y)
                continue
            if kind in ("LineString", "MultiLineString"):
                # the simulation's links first: tunnels and bridges carry traffic like any other stretch
                road = ROAD_LEVEL.get(tags.get("highway"), 0)
                rail = rail_level(tags) if tags.get("railway") == "rail" and tags.get("service") is None else 0
                if road or rail:
                    for p in gl.parts(geom):
                        network.add(p, road=road, rail=rail)
                waterway = tags.get("waterway")
                if waterway in ("river", "canal") and not tags.get("name"):
                    # OpenStreetMap tags countless small, unnamed watercourses as "river" in
                    # hilly country; without a name it is almost never one, so it is drawn (if
                    # at all) as a stream, close up only, not as a major river everywhere.
                    waterway = "stream"
                found = ROADS.get(tags.get("highway")) or (RAIL if tags.get("railway") == "rail" and tags.get("service") is None else None) or WATERWAYS.get(waterway)
                if found is None or tags.get("tunnel") in ("yes", "building_passage"):
                    continue
                # a number for the bigger roads and a name for the rivers, once per hexagon
                text = (tags.get("ref") if found[0] in (4, 5) and "_link" not in str(tags.get("highway")) else tags.get("name") if tags.get("waterway") == "river" else None)
                if text and kind == "LineString" and len(geom.coords) >= 2:
                    at = geom.interpolate(0.5, normalized=True)
                    text = str(text).split(";")[0].strip()
                    key = (RIVER_NAME if tags.get("waterway") else ROAD_NUMBER, text, h3.latlng_to_cell(at.y, at.x, LABEL_SPREAD_LEVEL))
                    if text and len(text) <= 40:
                        labels.setdefault(key, (at.x, at.y))
                simple = shapely.simplify(geom, SIMPLIFY_DEG * (2.5 if found[0] == 9 else 1.0))
                for p in gl.parts(simple):
                    if len(p) >= 2:
                        lines.append((found, p))
                key = tags.get("highway") or ("rail" if tags.get("railway") else tags.get("waterway"))
                counts[key] = counts.get(key, 0) + 1
            elif kind in ("Polygon", "MultiPolygon"):
                c = cover_class(tags)
                if c == 0:
                    continue
                for poly in polygons(geom):
                    if poly.area < MIN_AREA_DEG2:
                        continue
                    pens[c].polygon(to_px(poly.exterior.coords), fill=255)
                    for hole in poly.interiors:
                        pens[c].polygon(to_px(hole.coords), fill=0)
                    areas[c] = areas.get(c, 0) + 1
            if n % 500_000 == 0 and n:
                print(f"  {n:,} features read")
    print(f"lines kept by tag: {dict(sorted(counts.items(), key=lambda kv: -kv[1]))}")
    print(f"areas drawn: water {areas.get(WATER, 0):,}, woods {areas.get(WOODS, 0):,}, built-up {areas.get(BUILT, 0):,}, industry {areas.get(INDUSTRY, 0):,}; unreadable rows {bad}")
    if not lines:
        sys.exit("no roads, railways or waterways were found: was the export run with the tags as properties?")

    # ---- the lines file ----
    a_all, b_all, style_all, tier_all = [], [], [], []
    for (style, tier), line in lines:
        a, b = gl.cut(line)
        a_all.append(a); b_all.append(b)
        style_all.append(np.full(len(a), style, dtype=np.uint8)); tier_all.append(np.full(len(a), tier, dtype=np.uint8))
    a = np.concatenate(a_all); b = np.concatenate(b_all); style = np.concatenate(style_all); tier = np.concatenate(tier_all)
    mid = (a + b) / 2.0
    cells = np.array([h3.str_to_int(h3.latlng_to_cell(la, lo, BUCKET_LEVEL)) for lo, la in mid], dtype=np.uint64)
    order = np.lexsort((style, cells, tier))
    a, b, style, cells, tier = a[order], b[order], style[order], cells[order], tier[order]
    starts = np.concatenate([[0], np.flatnonzero((np.diff(cells) != 0) | (np.diff(tier) != 0)) + 1])
    centers = np.array([h3.cell_to_latlng(h3.int_to_str(int(c))) for c in cells[starts]])
    cover = np.concatenate(rings)
    cover_off = np.cumsum([0] + [len(r) for r in rings])
    size = container.write(out_dir / f"{name}.lines.bin", gl.KIND, gl.VERSION, {
        "seg_a": (gl.places(a).reshape(-1), "<i4"),
        "seg_b": (gl.places(b).reshape(-1), "<i4"),
        "seg_style": (style, "u1"),
        "bucket_off": (np.concatenate([starts, [len(style)]]), "<u4"),
        "bucket_tier": (tier[starts], "u1"),
        "bucket_xyz": (gl.places(centers[:, ::-1]).reshape(-1), "<i4"),
        "info": (np.array([BUCKET_REACH_MICRORAD]), "<u4"),
        "cover": (np.rint(cover * 1e6).reshape(-1), "<i4"),
        "cover_off": (cover_off, "<u4"),
    })
    print(f"{out_dir / (name + '.lines.bin')}: {size:,} bytes; {len(style):,} pieces in {len(starts):,} buckets")

    # ---- the network file ----
    region = set()
    for ring in rings:
        region.update(h3.polygon_to_cells(h3.LatLngPoly([(lat, lon) for lon, lat in ring.tolist()]), build_network.LEVEL))
    region_ids = np.array(sorted(h3.str_to_int(c) for c in region), dtype=np.uint64)
    keys = sorted(set(network.road) | set(network.rail))
    road_levels = np.array([network.road.get(k, 0) for k in keys], dtype=np.uint8)
    rail_levels = np.array([network.rail.get(k, 0) for k in keys], dtype=np.uint8)
    size = container.write(out_dir / f"{name}.network.bin", 7, 1, {
        "cells": (region_ids, "<u8"),
        "link_a": (np.array([k[0] for k in keys], dtype=np.uint64), "<u8"),
        "link_b": (np.array([k[1] for k in keys], dtype=np.uint64), "<u8"),
        "road": (road_levels, "u1"),
        "rail": (rail_levels, "u1"),
    })
    print(f"{out_dir / (name + '.network.bin')}: {size:,} bytes; {len(region_ids):,} hexagons in the region, {len(keys):,} links; "
          + "road by level " + ", ".join(f"{k}: {int((road_levels == k).sum()):,}" for k in range(1, 6))
          + "; rail by level " + ", ".join(f"{k}: {int((rail_levels == k).sum()):,}" for k in range(1, 6)))

    # ---- the ground cover file ----
    grid = np.full((height, width), LAND, dtype=np.uint8)
    if land_sheet is not None:
        grid[np.asarray(land_sheet) == 0] = SEA
        land_sheet.close()
    sea = grid == SEA
    for c in (WOODS, BUILT, INDUSTRY, WATER):
        grid[np.asarray(sheets[c]) > 0] = c
        sheets[c].close()
    grid[sea] = SEA                        # nothing grows or is built past the coastline
    # land inside the picture but outside the pack's coverage (a neighbouring country) has no
    # detail here: marked so the client draws it as it would without a pack
    inside = Image.new("L", (width, height), 0)
    pen = ImageDraw.Draw(inside)
    for ring in rings:
        pen.polygon(to_px(ring), fill=255)
    grid[(np.asarray(inside) == 0) & ~sea] = OUTSIDE
    inside.close()
    stored, tile_layer, across, down = tiles_of(grid, TILE)
    if not stored:
        sys.exit("the picture is all sea")
    info = [round(west * 1e6), round(north * 1e6), round(PIXEL_LON * 1e9), round(PIXEL_LAT * 1e9), width, height, TILE, across, down]
    size = container.write(out_dir / f"{name}.cover.bin", 6, 2, {
        "grid": (np.array(info), "<i4"),
        "tile_layer": (tile_layer, "<u4"),
        "tiles": (np.stack(stored).reshape(-1), "u1"),
    })
    share = np.bincount(grid.reshape(-1), minlength=7) / grid.size
    print(f"{out_dir / (name + '.cover.bin')}: {size:,} bytes; {len(stored)} of {across * down} tiles stored; "
          f"of the picture: sea {share[0]:.1%}, water {share[1]:.1%}, woods {share[2]:.1%}, built-up {share[3]:.1%}, industry {share[4]:.1%}")


    # ---- the labels file ----
    keys = sorted(labels, key=lambda k: (k[0], k[1], str(k[2])))
    if keys:
        where = np.array([labels[k] for k in keys])
        texts = [k[1].encode("utf-8") for k in keys]
        offsets = np.zeros(len(texts) + 1, dtype=np.uint32)
        np.cumsum([len(t) for t in texts], out=offsets[1:])
        size = container.write(out_dir / f"{name}.labels.bin", 9, 1, {
            "label_xyz": (gl.places(where).reshape(-1), "<i4"),
            "label_kind": (np.array([k[0] for k in keys], dtype=np.uint8), "u1"),
            "label_text_blob": (np.frombuffer(b"".join(texts), dtype=np.uint8), "u1"),
            "label_text_off": (offsets, "<u4"),
        })
        by_kind = np.bincount([k[0] for k in keys], minlength=5)
        print(f"{out_dir / (name + '.labels.bin')}: {size:,} bytes; cities {by_kind[0]:,}, towns {by_kind[1]:,}, villages {by_kind[2]:,}, road numbers {by_kind[3]:,}, river names {by_kind[4]:,}")
    else:
        print("no names found (was the export run with points?): no labels file")

    # ---- the heights file ----
    if dem_dir:
        heights, used = heights_picture(dem_dir, west, south, east, north)
        if used == 0:
            print(f"{dem_dir}: no elevation tile for this region: no heights file")
        else:
            stored, table, across, down = tiles_of(heights, TILE)
            info = [round(west * 1e6), round(north * 1e6), round(HEIGHT_PIXEL_LON * 1e9), round(HEIGHT_PIXEL_LAT * 1e9), heights.shape[1], heights.shape[0], TILE, across, down]
            size = container.write(out_dir / f"{name}.heights.bin", 8, 1, {"grid": (np.array(info), "<i4"), "tile_layer": (table, "<u4"), "tiles": (np.stack(stored).reshape(-1), "<u2")})
            print(f"{out_dir / (name + '.heights.bin')}: {size:,} bytes; {used} elevation tiles, {len(stored)} of {across * down} tiles stored; highest {int(heights.max())} m")


if __name__ == "__main__":
    if len(sys.argv) not in (5, 6, 7):
        sys.exit(__doc__)
    main(*sys.argv[1:])
