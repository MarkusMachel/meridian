"""The transport network of the simulation: which neighbouring hexagons a road or a railway
joins, and how good it is (its level). A link is the object the simulation knows: supply and
movement run along links, and a link's level is its capacity class.

Levels (0 = none):
  road  1 local road, 2 secondary road, 3 main road, 4 trunk road, 5 motorway
  rail  1 single-track branch line ... 5 double-track electrified high-speed main line
        (one for being a railway, and one each for: main line, two or more tracks,
        electrified, built for 200 km/h or more)

How a line becomes links: it is walked in steps of under a kilometre, the hexagon under each
step is looked up, and every change of hexagon is a link between the two. A link keeps the
best level of the lines that use it.

Used by build_terrain.py (the world, from Natural Earth) and build_pack.py (a region, from
OpenStreetMap).
"""
import h3
import numpy as np

LEVEL = 6
STEP_DEG = 0.008          # under 0.9 km: a hexagon is at least 5 km across


def line_links(coords):
    """The links a line makes: pairs of H3 numbers (ints) of neighbouring hexagons, in the
    order the line passes them. `coords` is an array of longitude, latitude."""
    xy = np.asarray(coords, dtype=np.float64)[:, :2]
    if len(xy) < 2:
        return []
    seg = np.maximum(np.abs(np.diff(xy[:, 0])), np.abs(np.diff(xy[:, 1])))
    if seg.max() > 180.0:
        return []                     # the source cuts lines at the 180th meridian
    n = np.maximum(1, np.ceil(seg / STEP_DEG)).astype(np.int64)
    idx = np.repeat(np.arange(len(seg)), n)
    t = (np.arange(len(idx)) - np.repeat(np.cumsum(n) - n, n)) / n[idx]
    pts = xy[idx] + (xy[idx + 1] - xy[idx]) * t[:, None]
    pts = np.vstack([pts, xy[-1]])
    out, last = [], None
    for lon, lat in pts.tolist():
        cell = h3.latlng_to_cell(lat, lon, LEVEL)
        if cell == last:
            continue
        if last is not None:
            if h3.are_neighbor_cells(last, cell):
                out.append((h3.str_to_int(last), h3.str_to_int(cell)))
            else:
                # the line cut a corner: fill in the hexagons between
                try:
                    chain = h3.grid_path_cells(last, cell)
                except Exception:      # noqa: BLE001  (no path across a pentagon: leave the gap)
                    chain = []
                for a, b in zip(chain, chain[1:]):
                    out.append((h3.str_to_int(a), h3.str_to_int(b)))
        last = cell
    return out


class Levels:
    """Collects the best level per link, keyed by the two H3 numbers (lower first)."""

    def __init__(self):
        self.road = {}
        self.rail = {}

    def add(self, coords, road=0, rail=0):
        for a, b in line_links(coords):
            key = (a, b) if a < b else (b, a)
            if road and road > self.road.get(key, 0):
                self.road[key] = road
            if rail and rail > self.rail.get(key, 0):
                self.rail[key] = rail

    def on_edges(self, cell_h3, edges):
        """Levels as two u8 arrays, one entry per edge of the map's cell graph (`edges` rows are
        (a, b, ...) cell numbers with a < b). Links to or from a hexagon that is not land are dropped."""
        number = {int(h): i for i, h in enumerate(np.asarray(cell_h3).tolist())}
        n = len(cell_h3)
        edge_key = edges[:, 0].astype(np.int64) * n + edges[:, 1]
        order = np.argsort(edge_key, kind="stable")
        sorted_key = edge_key[order]
        out = []
        for levels in (self.road, self.rail):
            arr = np.zeros(len(edges), dtype=np.uint8)
            for (ha, hb), level in levels.items():
                ia, ib = number.get(ha), number.get(hb)
                if ia is None or ib is None:
                    continue
                key = min(ia, ib) * n + max(ia, ib)
                k = int(np.searchsorted(sorted_key, key))
                if k < len(sorted_key) and sorted_key[k] == key:
                    arr[order[k]] = max(arr[order[k]], level)
            out.append(arr)
        return out


# ---- the world, from Natural Earth (public domain; about 2010, main routes only) ----

NE_ROAD = {"Major Highway": 4, "Beltway": 4, "Secondary Highway": 3, "Bypass": 3, "Road": 2}


def ne_rail_level(rec):
    """Natural Earth's railways carry two codes, `mult_track` and `electric`, each 0, 1 or 2.
    They are taken here as 1 = yes, 2 = no, 0 = not known (two thirds of the lines are 0), which
    fits where the 1s are (Europe, Japan) but is not documented. Level 2 to 4."""
    level = 2                                    # Natural Earth has only the lines that matter
    if str(rec.get("mult_track", "")).strip() == "1":
        level += 1
    if str(rec.get("electric", "")).strip() == "1":
        level += 1
    return level


def world_levels(ne_dir, cell_h3, edges, log=print):
    import shapefile
    from shapely.geometry import shape
    import build_globe_lines as gl

    def records(name):
        r = shapefile.Reader(str(ne_dir / name / name))
        names = [f[0] for f in r.fields[1:]]
        for sr in r.iterShapeRecords():
            if sr.shape.shapeType != 0:
                yield dict(zip(names, sr.record)), shape(sr.shape.__geo_interface__)

    levels = Levels()
    for rec, g in records("ne_10m_roads"):
        kind = str(rec.get("type", ""))
        if kind.startswith("Ferry"):
            continue
        for p in gl.parts(g):
            levels.add(p, road=NE_ROAD.get(kind, 1))
    for rec, g in records("ne_10m_railroads"):
        if "ferry" in str(rec.get("featurecla", "")).lower():
            continue
        for p in gl.parts(g):
            levels.add(p, rail=ne_rail_level(rec))
    road, rail = levels.on_edges(cell_h3, edges)
    log("links by level: road " + ", ".join(f"{k}: {int((road == k).sum()):,}" for k in range(1, 6))
        + "; rail " + ", ".join(f"{k}: {int((rail == k).sum()):,}" for k in range(1, 6)))
    return road, rail
