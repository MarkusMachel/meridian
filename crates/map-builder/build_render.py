"""Step 8 of the map builder: the render-only file for the client.

Usage: python -I build_render.py <intermediate-dir> <out-file> --ne <natural-earth-dir> [--levels 8000,2500,800]
Reads  cells/cells.wkb, cells/cell_province.npy, cells/cell_state.npy, provinces.csv
Writes the render file (container "MRRD", kind 4; same layout as the baked map, see
docs/data-formats.md). The engine never reads this file; only the client does.

Method (deterministic, no random numbers):
  * cells are merged into one polygon per province;
  * the province polygons are simplified as one *coverage* (shared borders are simplified
    together, so neighbours still fit) at each tolerance, one level per tolerance;
  * per level: a triangle mesh of every province (fill) and noded border lines, each
    classified as province border, state border or coast;
  * one extra set of cell outline segments at full detail.
Coordinates are Equal Earth metres (EPSG:8857), the same equal-area plane the simulation uses, so
the display has no area stretch toward the poles. They are rounded to whole metres (i32).
Version 3 and earlier used Web Mercator; that is no longer read.
"""
import csv
import struct
import sys
import zlib
from pathlib import Path

import numpy as np
import pyproj
import shapely
from shapely import STRtree

sys.path.insert(0, str(Path(__file__).resolve().parent))
import render_extras  # noqa: E402

FORMAT_VERSION = 7
KIND_RENDER = 4
LEVELS_M = [8000.0, 2500.0, 800.0]
KIND_PROVINCE, KIND_STATE, KIND_COAST = 0, 1, 2

def to_display(coords):
    """(n, 2) simulation coordinates -> display coordinates. The display plane is the simulation's
    own Equal Earth plane, so this is the identity (kept as one place to change the projection)."""
    return coords


def read_wkb(path):
    data = Path(path).read_bytes()
    out, i = [], 0
    while i < len(data):
        n = int.from_bytes(data[i:i + 4], "little")
        out.append(data[i + 4:i + 4 + n])
        i += 4 + n
    return shapely.from_wkb(np.array(out, dtype=object))


def province_polygons(cells, cell_prov, n_prov):
    order = np.argsort(cell_prov, kind="stable")
    bounds = np.searchsorted(cell_prov[order], np.arange(n_prov + 1))
    polys = []
    for p in range(n_prov):
        group = cells[order[bounds[p]:bounds[p + 1]]]
        polys.append(shapely.union_all(group) if len(group) else shapely.Polygon())
    return np.array(polys, dtype=object)


def rounded(geoms):
    """Snap to whole metres so shared vertices stay identical after rounding."""
    return shapely.set_precision(geoms, 1.0)


def project(geoms):
    """Rounded to whole metres vertex by vertex (so a vertex shared by two
    provinces stays shared), then repair any polygon the rounding made invalid."""
    out = shapely.transform(geoms, lambda c: np.rint(to_display(c)))
    bad = ~shapely.is_valid(out)
    if bad.any():
        out[bad] = shapely.make_valid(out[bad])
    return out


def fill_mesh(polys):
    """Triangles of every polygon: (vx, vy, vprov, tri). One vertex list, shared vertices merged."""
    tris = shapely.constrained_delaunay_triangles(polys)
    parts, owner = shapely.get_parts(tris, return_index=True)
    # each triangle is a closed ring of 4 coordinates; keep the first three
    c = shapely.get_coordinates(parts).reshape(-1, 4, 2)[:, :3, :]
    c = np.rint(c).astype(np.int64)
    prov = np.repeat(owner.astype(np.int64), 3)
    flat = c.reshape(-1, 2)
    key = np.column_stack([flat, prov])
    uniq, inverse = np.unique(key, axis=0, return_inverse=True)
    return (uniq[:, 0].astype(np.int32), uniq[:, 1].astype(np.int32), uniq[:, 2].astype(np.uint32),
            inverse.astype(np.uint32).reshape(-1))


def border_lines(polys, prov_state):
    """Noded boundary lines of a polygon set. Returns (x, y, off, kind) with polylines in x/y."""
    boundaries = shapely.boundary(polys)
    noded = shapely.union_all(boundaries)
    pieces = shapely.get_parts(noded)
    pieces = pieces[~shapely.is_empty(pieces)]
    mids = shapely.line_interpolate_point(pieces, 0.5, normalized=True)
    tree = STRtree(polys)
    q_mid, q_prov = tree.query(mids, predicate="dwithin", distance=2.0)
    touching = [[] for _ in range(len(pieces))]
    for m, p in zip(q_mid.tolist(), q_prov.tolist()):
        touching[m].append(p)
    kinds = np.empty(len(pieces), dtype=np.uint8)
    for i, t in enumerate(touching):
        if len(t) <= 1:
            kinds[i] = KIND_COAST
        else:
            states = {int(prov_state[p]) for p in t}
            kinds[i] = KIND_STATE if len(states) > 1 else KIND_PROVINCE
    # deterministic order: kind, then first coordinate
    first = np.array([g.coords[0] for g in pieces]) if len(pieces) else np.zeros((0, 2))
    order = np.lexsort((first[:, 1], first[:, 0], kinds)) if len(pieces) else np.zeros(0, dtype=np.int64)
    xs, ys, off, out_kind = [], [], [0], []
    for i in order:
        c = np.rint(np.asarray(pieces[i].coords)).astype(np.int32)
        xs.append(c[:, 0]); ys.append(c[:, 1]); off.append(off[-1] + len(c)); out_kind.append(kinds[i])
    return (np.concatenate(xs).astype(np.int32), np.concatenate(ys).astype(np.int32),
            np.array(off, dtype=np.uint32), np.array(out_kind, dtype=np.uint8))


def cell_segments(cells):
    """Every distinct cell edge once, as x0,y0,x1,y1 (whole metres), in sorted order."""
    rings = shapely.get_rings(cells)
    coords, idx = shapely.get_coordinates(rings, return_index=True)
    c = np.rint(coords).astype(np.int64)
    same = idx[1:] == idx[:-1]
    a, b = c[:-1][same], c[1:][same]
    swap = (a[:, 0] > b[:, 0]) | ((a[:, 0] == b[:, 0]) & (a[:, 1] > b[:, 1]))
    lo = np.where(swap[:, None], b, a); hi = np.where(swap[:, None], a, b)
    seg = np.unique(np.hstack([lo, hi]), axis=0)
    return seg.astype(np.int32).reshape(-1)


def cell_rings(cells):
    """Outer ring of each cell (its largest part), without the repeated closing point.
    Returns (off, x, y) with the points of cell i at off[i]..off[i+1]."""
    parts, owner = shapely.get_parts(cells, return_index=True)
    area = shapely.area(parts)
    best = np.full(len(cells), -1, dtype=np.int64)
    order = np.lexsort((-area, owner))  # per cell, largest part first
    first = np.ones(len(order), dtype=bool)
    first[1:] = owner[order][1:] != owner[order][:-1]
    best[owner[order][first]] = order[first]
    assert (best >= 0).all(), "a cell has no geometry"
    ext = shapely.get_exterior_ring(parts[best])
    coords, idx = shapely.get_coordinates(ext, return_index=True)
    drop_last = np.ones(len(coords), dtype=bool)
    ends = np.flatnonzero(np.r_[idx[1:] != idx[:-1], True])
    drop_last[ends] = False
    coords, idx = coords[drop_last], idx[drop_last]
    off = np.zeros(len(cells) + 1, dtype=np.uint32)
    np.cumsum(np.bincount(idx, minlength=len(cells)), out=off[1:])
    c = np.rint(coords).astype(np.int32)
    return off, c[:, 0].copy(), c[:, 1].copy()


def main(inter, out_file, levels, ne_dir):
    inter = Path(inter)
    c = inter / "cells"
    cells = read_wkb(c / "cells.wkb")
    cell_prov = np.load(c / "cell_province.npy").astype(np.int64)
    cell_state = np.load(c / "cell_state.npy")
    rows = list(csv.DictReader(open(inter / "provinces.csv", encoding="utf-8")))
    P = len(rows)
    prov_state = np.zeros(P, dtype=np.int64)
    prov_state[cell_prov] = cell_state  # every cell of a province has the province's state
    print(f"{len(cells)} cells, {P} provinces")

    full = province_polygons(cells, cell_prov, P)
    cells_m = shapely.transform(cells, to_display)
    # the unsimplified area of each province (km2): what the validator holds the fills to
    A = {}
    A["province_ref_area_km2"] = np.rint(shapely.area(project(full)) / 1e6).astype("<u4")
    A["levels_tolerance_m"] = np.array(levels, dtype="<u4")
    for k, tol in enumerate(levels):
        simple = shapely.coverage_simplify(full, tol)
        simple = project(simple)
        # a province must never vanish; fall back to the unsimplified shape
        empty = shapely.is_empty(simple)
        if empty.any():
            simple[empty] = project(full[empty])
        vx, vy, vp, tri = fill_mesh(simple)
        lx, ly, loff, lkind = border_lines(simple, prov_state)
        A[f"l{k}_vx"], A[f"l{k}_vy"], A[f"l{k}_vprov"], A[f"l{k}_tri"] = vx, vy, vp, tri
        A[f"l{k}_line_x"], A[f"l{k}_line_y"], A[f"l{k}_line_off"], A[f"l{k}_line_kind"] = lx, ly, loff, lkind
        print(f"level {k} ({tol:.0f} m): {len(vx)} vertices, {len(tri)//3} triangles, {len(loff)-1} lines "
              f"({np.bincount(lkind, minlength=3).tolist()} province/state/coast)")
    A["cell_seg"] = cell_segments(cells_m)
    print(f"cell outlines: {len(A['cell_seg'])//4} segments")
    A["cell_ring_off"], A["cell_ring_x"], A["cell_ring_y"] = cell_rings(cells_m)
    # centres in the display projection (area-weighted province centres, as in the baked map)
    cx = np.load(c / "cell_x.npy").astype(np.float64); cy = np.load(c / "cell_y.npy").astype(np.float64)
    carea = np.load(c / "cell_area_ha.npy").astype(np.float64)
    cm = np.rint(to_display(np.column_stack([cx, cy]))).astype(np.int32)
    A["cell_mx"], A["cell_my"] = cm[:, 0].copy(), cm[:, 1].copy()
    wsum = np.maximum(np.bincount(cell_prov, weights=carea, minlength=P), 1)
    pe = np.column_stack([np.bincount(cell_prov, weights=cx * carea, minlength=P) / wsum, np.bincount(cell_prov, weights=cy * carea, minlength=P) / wsum])
    pm = np.rint(to_display(pe)).astype(np.int32)
    A["province_mx"], A["province_my"] = pm[:, 0].copy(), pm[:, 1].copy()
    print(f"cell rings: {len(A['cell_ring_x'])} points; x range {cm[:,0].min()}..{cm[:,0].max()}, y range {cm[:,1].min()}..{cm[:,1].max()}")
    # shaded relief, rivers and place names
    A["relief"], A["relief_info"] = render_extras.relief(inter, (int(cm[:, 0].min()), int(cm[:, 0].max()), int(cm[:, 1].min()), int(cm[:, 1].max())))
    print(f"relief: {A['relief_info'][0]} x {A['relief_info'][1]} pixels")
    A["height_m"] = render_extras.heights(inter, (int(cm[:, 0].min()), int(cm[:, 0].max()), int(cm[:, 1].min()), int(cm[:, 1].max())))
    print(f"heights: {A['height_m'].min()} .. {A['height_m'].max()} m")
    A["land_rgb"] = render_extras.land_colors(ne_dir, (int(cm[:, 0].min()), int(cm[:, 0].max()), int(cm[:, 1].min()), int(cm[:, 1].max())))
    print(f"land colours: {len(A['land_rgb']) // 3} pixels (Natural Earth II)")
    A["river_x"], A["river_y"], A["river_off"], A["river_rank"] = render_extras.rivers(ne_dir)
    print(f"rivers: {len(A['river_off']) - 1} lines, {len(A['river_x'])} points")
    A["road_x"], A["road_y"], A["road_off"], A["road_class"] = render_extras.roads(ne_dir)
    print(f"roads: {len(A['road_off']) - 1} lines, {len(A['road_x'])} points, by class {np.bincount(A['road_class'], minlength=3).tolist()}")
    A["rail_x"], A["rail_y"], A["rail_off"], A["rail_class"] = render_extras.rails(ne_dir)
    print(f"railways: {len(A['rail_off']) - 1} lines, {len(A['rail_x'])} points, by class {np.bincount(A['rail_class'], minlength=3).tolist()}")
    A["urban_vx"], A["urban_vy"], A["urban_tri"] = render_extras.urban(ne_dir)
    print(f"urban areas: {len(A['urban_vx'])} vertices, {len(A['urban_tri']) // 3} triangles")
    render_extras.labels(inter, A)
    print(f"labels: {len(A['country_x'])} countries, {len(A['region_x'])} regions")
    write(out_file, A)


DTYPE_CODES = {np.dtype("u1"): 1, np.dtype("<u2"): 2, np.dtype("<u4"): 3, np.dtype("<i2"): 4, np.dtype("<i4"): 5}


def write(out_file, A):
    payload = bytearray(struct.pack("<I", len(A)))
    for name, arr in A.items():
        arr = np.ascontiguousarray(arr)
        dt = arr.dtype.newbyteorder("<")
        nb = name.encode()
        payload += struct.pack("<B", len(nb)) + nb + struct.pack("<BQ", DTYPE_CODES[np.dtype(dt)], arr.size)
        payload += arr.astype(dt, copy=False).tobytes()
    header = b"MRRD" + struct.pack("<IIQQ", FORMAT_VERSION, KIND_RENDER, len(payload), zlib.crc32(bytes(payload)))
    Path(out_file).write_bytes(header + bytes(payload))
    print(f"{out_file}: {len(header) + len(payload):,} bytes, {len(A)} arrays")


if __name__ == "__main__":
    args = sys.argv[1:]
    levels = LEVELS_M
    if "--levels" in args:
        i = args.index("--levels")
        levels = [float(x) for x in args[i + 1].split(",")]
        del args[i:i + 2]
    ne = "data/source/natural-earth"
    if "--ne" in args:
        i = args.index("--ne")
        ne = args[i + 1]
        del args[i:i + 2]
    main(args[0], args[1], levels, ne)
