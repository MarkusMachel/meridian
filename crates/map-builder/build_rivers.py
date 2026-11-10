"""Rivers on hexagon edges (map rewrite M2).

A river does not run through hexagons, it runs along the borders between them, so that a unit
going from one hexagon to its neighbour either crosses the river or does not. Each river line
is turned into a chain of hexagon sides: the cheapest chain, on the web of hexagon corners and
sides, from the corner nearest the line's start to the corner nearest its end, where a side
costs more the farther it lies from the line. A chain is unbroken by construction.

Used by build_terrain.py; `snap_rivers` returns one boolean per cell edge.
"""
import heapq

import h3
import numpy as np
import shapely

LEVEL = 6
SAMPLE_M = 1_500.0     # the line is walked in steps of this length to find the hexagons it passes
OFF_LINE_M = 1_500.0   # a side this far from the line costs twice its length, at twice this five times


def line_parts(geom):
    if geom.geom_type == "LineString":
        return [geom]
    if hasattr(geom, "geoms"):
        return [p for g in geom.geoms for p in line_parts(g)]
    return []


def snap_part(part, part_xy, proj, corner_xy):
    """One river line (lon/lat `part`, the same line projected `part_xy`) as a chain of hexagon
    sides. Returns a list of (hexagon, hexagon) pairs (H3 strings) whose shared side the river
    runs along. `corner_xy` caches the projected position of every corner seen."""
    n = max(2, int(part_xy.length / SAMPLE_M) + 1)
    pts = shapely.get_coordinates(shapely.line_interpolate_point(part, np.linspace(0.0, 1.0, n), normalized=True))
    passed = list(dict.fromkeys(h3.latlng_to_cell(lat, lng, LEVEL) for lng, lat in pts))
    corridor = sorted({c for p in passed for c in h3.grid_disk(p, 2)})
    sides = {}                                     # (corner, corner) -> the hexagons on each side
    for c in corridor:
        v = h3.cell_to_vertexes(c)
        for i in range(len(v)):
            a, b = v[i], v[(i + 1) % len(v)]
            sides.setdefault((a, b) if a < b else (b, a), []).append(c)
    corners = sorted({v for s in sides for v in s})
    new = [v for v in corners if v not in corner_xy]
    if new:
        ll = np.array([h3.vertex_to_latlng(v) for v in new])
        lng = ll[:, 1]
        if np.ptp(lng) > 180.0:                    # a hexagon on the 180 degree line: keep its corners together
            lng = np.where(lng < 0, lng + 360.0, lng)
        xs, ys = proj.transform(np.clip(lng, -180.0, 180.0), ll[:, 0])
        corner_xy.update(zip(new, zip(np.asarray(xs).tolist(), np.asarray(ys).tolist())))
    xy = np.array([corner_xy[v] for v in corners])
    index = {v: i for i, v in enumerate(corners)}
    keys = sorted(sides)
    a = np.array([index[s[0]] for s in keys]); b = np.array([index[s[1]] for s in keys])
    length = np.hypot(xy[a, 0] - xy[b, 0], xy[a, 1] - xy[b, 1])
    off = shapely.distance(shapely.points((xy[a] + xy[b]) / 2.0), part_xy)
    cost = np.rint(length * (1.0 + (off / OFF_LINE_M) ** 2)).astype(np.int64) + 1
    near = {}
    for k in range(len(keys)):
        near.setdefault(int(a[k]), []).append((int(b[k]), int(cost[k]), k))
        near.setdefault(int(b[k]), []).append((int(a[k]), int(cost[k]), k))
    ends = shapely.get_coordinates(part_xy)[[0, -1]]
    start = int(np.argmin(np.hypot(xy[:, 0] - ends[0, 0], xy[:, 1] - ends[0, 1])))
    goal = int(np.argmin(np.hypot(xy[:, 0] - ends[1, 0], xy[:, 1] - ends[1, 1])))
    if start == goal:
        return []
    # cheapest chain of sides; ties go to the lower corner number, so the result is always the same
    best = {start: 0}; came = {}
    heap = [(0, start)]
    while heap:
        d, v = heapq.heappop(heap)
        if v == goal:
            break
        if d > best[v]:
            continue
        for w, c, k in near[v]:
            if d + c < best.get(w, 1 << 62):
                best[w] = d + c; came[w] = (v, k)
                heapq.heappush(heap, (d + c, w))
    out = []
    v = goal
    while v != start:
        v, k = came[v]
        pair = sides[keys[k]]
        if len(pair) == 2:
            out.append((pair[0], pair[1]))
    return out


def snap_rivers(lines, lines_xy, cell_h3, edges, proj, log=print):
    """`lines` are the river lines in longitude and latitude, `lines_xy` the same projected.
    Returns a boolean array, one per cell edge: the river runs along that shared side."""
    number = {h3.int_to_str(int(c)): i for i, c in enumerate(cell_h3.tolist())}
    n = len(cell_h3)
    edge_key = edges[:, 0].astype(np.int64) * n + edges[:, 1]      # the edge list is sorted by (a, b), a < b
    out = np.zeros(len(edges), dtype=bool)
    corner_xy = {}
    chains = sides_all = 0
    for geom, geom_xy in zip(lines, lines_xy):
        for part, part_xy in zip(line_parts(geom), line_parts(geom_xy)):
            if part_xy.length < SAMPLE_M:
                continue
            pairs = snap_part(part, part_xy, proj, corner_xy)
            chains += 1; sides_all += len(pairs)
            for ca, cb in pairs:
                ia, ib = number.get(ca), number.get(cb)
                if ia is None or ib is None:       # sea on one side: nothing to cross there
                    continue
                key = min(ia, ib) * n + max(ia, ib)
                k = int(np.searchsorted(edge_key, key))
                if k < len(edge_key) and edge_key[k] == key:
                    out[k] = True
    log(f"rivers: {chains} lines snapped to {sides_all} hexagon sides, {int(out.sum())} of them between two land hexagons")
    return out
