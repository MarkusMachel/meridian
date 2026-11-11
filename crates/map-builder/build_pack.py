"""A regional detail pack from OpenStreetMap (map rewrite M6, first cut): real roads, railways
and waterways at close zoom, and the ground cover (water, woods, built-up areas) as a fine
picture. Drawing only; the engine never reads a pack.

Usage: python -I build_pack.py <name> <features.geojsonseq> <coverage.poly> <out-dir> [<land_polygons.shp>]

`fetch_pack.sh` makes the inputs: it downloads a Geofabrik extract and its coverage outline and
runs the `osmium` tool to keep and export the features used here.

Writes two files:
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
import container  # noqa: E402

# line styles (the client's colours and widths) and detail tiers, by OpenStreetMap tag
ROADS = {
    "motorway": (4, 1), "trunk": (4, 1), "motorway_link": (4, 2), "trunk_link": (4, 2),
    "primary": (5, 2), "primary_link": (5, 3), "secondary": (5, 3),
    "tertiary": (8, 4), "unclassified": (8, 4),
}
RAIL = (7, 3)
WATERWAYS = {"river": (3, 2), "canal": (3, 3)}
SIMPLIFY_DEG = 0.00008          # about 8 m: under half a pixel at the closest zoom
BUCKET_LEVEL = 4                # hexagons about 45 km across
BUCKET_REACH_MICRORAD = 6_500
# the ground cover picture: pixel size in degrees (about 79 m by 83 m at 45 N) and tile size
PIXEL_LON, PIXEL_LAT = 0.001, 0.00075
TILE = 2048
SEA, WATER, WOODS, BUILT, INDUSTRY, LAND, OUTSIDE = 0, 1, 2, 3, 4, 6, 7
MIN_AREA_DEG2 = 4e-7            # areas under about 0.4 hectare are left out


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


def main(name, features_file, poly_file, out_dir, land_file=None):
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
            if kind in ("LineString", "MultiLineString"):
                found = ROADS.get(tags.get("highway")) or (RAIL if tags.get("railway") == "rail" and tags.get("service") is None else None) or WATERWAYS.get(tags.get("waterway"))
                if found is None or tags.get("tunnel") in ("yes", "building_passage"):
                    continue
                simple = shapely.simplify(geom, SIMPLIFY_DEG)
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
    across, down = -(-width // TILE), -(-height // TILE)
    tile_layer = np.full(across * down, 0xFFFFFFFF, dtype=np.uint32)
    stored = []
    for ty in range(down):
        for tx in range(across):
            part = grid[ty * TILE:(ty + 1) * TILE, tx * TILE:(tx + 1) * TILE]
            if not part.any():
                continue
            full = np.zeros((TILE, TILE), dtype=np.uint8)
            full[:part.shape[0], :part.shape[1]] = part
            tile_layer[ty * across + tx] = len(stored)
            stored.append(full)
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


if __name__ == "__main__":
    if len(sys.argv) not in (5, 6):
        sys.exit(__doc__)
    main(*sys.argv[1:])
