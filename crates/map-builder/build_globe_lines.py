"""The real lines for the globe view (map rewrite M3): coasts, the 1990 borders, rivers, roads
and railways as they run on the ground, not as the hexagon diagram. Drawing only; the engine
never reads this file.

Usage: python -I build_globe_lines.py <intermediate-dir> <natural-earth-dir> <out.bin>

Every line is cut into short straight pieces between whole-metre places on the sphere. The
pieces are sorted into buckets (a detail tier and a level-2 hexagon, about 300 km across) so the
client draws only the buckets in view.

Container: the same as the map file (bake_map.py), kind 4, version 1. Arrays:
  seg_a, seg_b   i32, three per piece: the two ends
  seg_style      u8 per piece (STYLES below)
  bucket_off     u32, buckets + 1 entries
  bucket_tier    u8 per bucket: 0 whole globe, 1 continent, 2 country, 3 region
  bucket_xyz     i32, three per bucket: the middle of its hexagon

Honest limits: Natural Earth at 1:10 million. Roads and railways are roughly 2010, not 1990,
and only the main ones. Finer maps need finer sources (map rewrite M4 and M6).
"""
import json
import struct
import sys
import zlib
from pathlib import Path

import h3
import numpy as np
import shapely
from shapely.geometry import shape

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_hexes as bh  # noqa: E402
import build_terrain as bt  # noqa: E402

VERSION = 1
KIND = 4
# style code -> (name, tier)
STYLES = {
    0: ("coast", 0),
    1: ("border of 1990", 0),
    2: ("great river", 0),
    3: ("river", 1),
    4: ("major highway", 1),
    5: ("highway or road", 2),
    6: ("minor road", 3),
    7: ("railway", 3),
}
MAX_PIECE_DEG = 0.25     # longer pieces are cut, so a long straight border still follows the globe
BUCKET_LEVEL = 2


def parts(geom):
    """The plain lines of any geometry (rings of polygons included), as coordinate arrays."""
    if geom is None or geom.is_empty:
        return []
    t = geom.geom_type
    if t in ("LineString", "LinearRing"):
        return [np.asarray(geom.coords)[:, :2]]
    if t == "Polygon":
        return [np.asarray(r.coords)[:, :2] for r in [geom.exterior, *geom.interiors]]
    return [p for g in geom.geoms for p in parts(g)]


def cut(line):
    """Insert points so no piece is longer than MAX_PIECE_DEG; pieces across the 180th
    meridian are dropped (the source cuts lines there)."""
    a, b = line[:-1], line[1:]
    keep = np.abs(a[:, 0] - b[:, 0]) < 180.0
    a, b = a[keep], b[keep]
    n = np.maximum(1, np.ceil(np.maximum(np.abs(a[:, 0] - b[:, 0]), np.abs(a[:, 1] - b[:, 1])) / MAX_PIECE_DEG)).astype(np.int64)
    idx = np.repeat(np.arange(len(a)), n)
    step = np.arange(len(idx)) - np.repeat(np.cumsum(n) - n, n)
    t0 = (step / n[idx])[:, None]; t1 = ((step + 1) / n[idx])[:, None]
    return a[idx] + (b[idx] - a[idx]) * t0, a[idx] + (b[idx] - a[idx]) * t1


def places(lonlat):
    lon, lat = np.radians(lonlat[:, 0]), np.radians(lonlat[:, 1])
    v = np.stack([np.cos(lat) * np.cos(lon), np.cos(lat) * np.sin(lon), np.sin(lat)], axis=1) * bh.EARTH_RADIUS_M
    return np.rint(v).astype(np.int32)


def main(inter, ne_dir, out_file):
    inter, ne = Path(inter), Path(ne_dir)

    def layer(name, keep=None):
        return bt.load_layer(ne / name / name, keep=keep)

    def records(name):
        import shapefile
        r = shapefile.Reader(str(ne / name / name))
        names = [f[0] for f in r.fields[1:]]
        for sr in r.iterShapeRecords():
            if sr.shape.shapeType != 0:      # a few records have no shape
                yield dict(zip(names, sr.record)), shape(sr.shape.__geo_interface__)

    lines = []   # (style, coordinate array)
    land = shapely.unary_union(layer("ne_10m_land"))
    for p in parts(land):
        lines.append((0, p))
    # borders: the outlines of the 1990 states, without the parts that are coast
    states = [shape(f["geometry"]) for f in json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))["features"]]
    outlines = shapely.unary_union([s.boundary for s in states])
    inland = shapely.difference(outlines, shapely.buffer(land.boundary, 0.03))
    for p in parts(inland):
        lines.append((1, p))
    for rec, g in records("ne_10m_rivers_lake_centerlines"):
        great = int(rec.get("scalerank") or 9) <= 4
        for p in parts(g):
            lines.append((2 if great else 3, p))
    for extra in ("ne_10m_rivers_europe", "ne_10m_rivers_north_america", "ne_10m_rivers_australia"):
        if (ne / extra).exists():
            for _, g in records(extra):
                for p in parts(g):
                    lines.append((3, p))
    for rec, g in records("ne_10m_roads"):
        kind = str(rec.get("type", ""))
        if kind.startswith("Ferry"):
            continue
        style = 4 if kind in ("Major Highway", "Beltway") else 5 if kind in ("Secondary Highway", "Road", "Bypass") else 6
        for p in parts(g):
            lines.append((style, p))
    for rec, g in records("ne_10m_railroads"):
        if "ferry" in str(rec.get("featurecla", "")).lower():
            continue          # a train ferry is not track
        for p in parts(g):
            lines.append((7, p))

    a_all, b_all, style_all = [], [], []
    for style, line in lines:
        if len(line) < 2:
            continue
        a, b = cut(line)
        a_all.append(a); b_all.append(b); style_all.append(np.full(len(a), style, dtype=np.uint8))
    a = np.concatenate(a_all); b = np.concatenate(b_all); style = np.concatenate(style_all)
    # buckets: detail tier, then the level-2 hexagon the middle of the piece is in
    mid = (a + b) / 2.0
    cells = np.array([h3.str_to_int(h3.latlng_to_cell(la, lo, BUCKET_LEVEL)) for lo, la in mid], dtype=np.uint64)
    tier = np.array([STYLES[s][1] for s in range(len(STYLES))], dtype=np.uint8)[style]
    order = np.lexsort((style, cells, tier))          # stable: the source order breaks ties
    a, b, style, cells, tier = a[order], b[order], style[order], cells[order], tier[order]
    change = np.flatnonzero((np.diff(cells) != 0) | (np.diff(tier) != 0)) + 1
    starts = np.concatenate([[0], change])
    bucket_off = np.concatenate([starts, [len(style)]]).astype(np.uint32)
    centers = np.array([h3.cell_to_latlng(h3.int_to_str(int(c))) for c in cells[starts]])
    arrays = {
        "seg_a": (places(a).reshape(-1), "<i4", 5),
        "seg_b": (places(b).reshape(-1), "<i4", 5),
        "seg_style": (style, "u1", 1),
        "bucket_off": (bucket_off, "<u4", 3),
        "bucket_tier": (tier[starts], "u1", 1),
        "bucket_xyz": (places(centers[:, ::-1]).reshape(-1), "<i4", 5),
    }
    payload = bytearray(struct.pack("<I", len(arrays)))
    for name, (arr, dt, code) in arrays.items():
        nb = name.encode("ascii")
        payload += struct.pack("<B", len(nb)) + nb + struct.pack("<BQ", code, arr.size) + np.ascontiguousarray(arr).astype(dt, copy=False).tobytes()
    header = b"MRMP" + struct.pack("<IIQQ", VERSION, KIND, len(payload), zlib.crc32(bytes(payload)))
    Path(out_file).parent.mkdir(parents=True, exist_ok=True)
    Path(out_file).write_bytes(header + bytes(payload))
    counts = np.bincount(style, minlength=len(STYLES))
    print(f"{out_file}: {len(header) + len(payload):,} bytes; {len(style):,} pieces in {len(starts):,} buckets; "
          + ", ".join(f"{STYLES[s][0]} {counts[s]:,}" for s in STYLES))


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
