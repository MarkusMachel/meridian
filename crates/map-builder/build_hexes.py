"""Map rewrite, phase M1, first step: the land of the world as H3 hexagons.

The simulation's map becomes one level of hexagons on the globe (H3 level 6, about 6.4 km
across; docs/design-notes.md section 25). This step decides which hexagons are land and writes,
for each: its H3 number, its centre as whole metres from the Earth's centre, and its land
neighbours with the ground distance to each.

  input   land polygons in longitude and latitude (Natural Earth 10 m land for now; the 1990
          provinces once the later steps are joined on)
  output  <out>/hex_id.npy      u64   H3 number of each land hexagon, ascending; the index into
                                      this array is the hexagon's number in the game
          <out>/hex_xyz.npy     i32   (n, 3) centre in metres on a sphere of radius EARTH_RADIUS_M
          <out>/hex_off.npy     u32   (n + 1) where each hexagon's neighbours start in hex_dst
          <out>/hex_dst.npy     u32   neighbours (game numbers), ascending per hexagon
          <out>/hex_dist_m.npy  u32   ground distance between the two centres, metres

A hexagon is land when its centre is on land, or when at least 3 of 7 sample points (the centre
and six points part-way to its corners) are, or when it touches land and holds a town (so a
port on a bay or a town on a small island is never left in the sea). Every piece of land of at
least MIN_ISLAND_KM2 keeps at least one hexagon, so small islands do not vanish.

Floating point is used here (it is the builder, not the simulation). The distances are worked
out from the whole-metre centres with integers, exactly as crates/meridian-core/src/sphere.rs
does, so the engine and the file always agree.

Run: python build_hexes.py <natural-earth dir> <out dir> [--level 6] [--south -60]
"""
import math
import sys
import time
from pathlib import Path

import h3
import numpy as np
import pyproj
import shapefile
import shapely
from shapely.geometry import shape

EARTH_RADIUS_M = 6_371_009          # the same value as sphere.rs
LEVEL = 6
MIN_ISLAND_KM2 = 3.0
SAMPLE_REACH = 0.6                  # sample points sit this share of the way from centre to corner
SAMPLES_NEEDED = 3                  # of 7


def load_land(ne_dir):
    """The land polygons (longitude, latitude), biggest first."""
    path = Path(ne_dir) / "ne_10m_land" / "ne_10m_land"
    polys = []
    for s in shapefile.Reader(str(path)).shapes():
        g = shape(s.__geo_interface__)
        polys.extend(g.geoms if g.geom_type == "MultiPolygon" else [g])
    polys = [p if p.is_valid else p.buffer(0) for p in polys]
    return sorted(polys, key=lambda p: -p.area)


def in_tiles(polys, size=5.0):
    """Cut the land into pieces no bigger than `size` degrees a side. Filling a polygon with
    hexagons takes time in proportion to its corners times its hexagons, so a continent in one
    piece (150,000 corners, a million hexagons) takes hours; in pieces it takes minutes."""
    pieces = []
    for p in polys:
        x0, y0, x1, y1 = p.bounds
        if x1 - x0 <= size and y1 - y0 <= size:
            pieces.append(p)
            continue
        for gx in np.arange(math.floor(x0 / size) * size, x1, size):
            for gy in np.arange(math.floor(y0 / size) * size, y1, size):
                part = shapely.clip_by_rect(p, gx, gy, gx + size, gy + size)
                if part.is_empty:
                    continue
                for q in (part.geoms if hasattr(part, "geoms") else [part]):
                    if q.geom_type == "Polygon" and q.area > 0:
                        pieces.append(q)
    return pieces


def load_towns(ne_dir):
    """(lat, lng) of the populated places, or nothing if that layer is not there."""
    path = Path(ne_dir) / "ne_10m_populated_places" / "ne_10m_populated_places"
    if not path.with_suffix(".shp").exists():
        return []
    return [(s.points[0][1], s.points[0][0]) for s in shapefile.Reader(str(path)).shapes()]


def place(lat, lng):
    """Whole-metre position of a latitude and longitude (degrees) on the sphere."""
    la, lo = math.radians(lat), math.radians(lng)
    r = EARTH_RADIUS_M
    return (round(r * math.cos(la) * math.cos(lo)), round(r * math.cos(la) * math.sin(lo)), round(r * math.sin(la)))


def ground_m(a, b):
    """Ground distance in metres between two whole-metre places: the integer method of sphere.rs."""
    one = 1 << 60
    two_r = 2 * EARTH_RADIUS_M
    chord2 = (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2 + (a[2] - b[2]) ** 2
    s2 = (chord2 << 60) // (two_r * two_r)
    if s2 >= one:
        return EARTH_RADIUS_M * 3_141_592_654 // 1_000_000_000
    s = math.isqrt(s2 << 60)
    cos = math.isqrt((one - s2) << 60)
    for _ in range(5):
        s = s * one // math.isqrt(2 * one * (one + cos))
        cos = math.isqrt(one * one - s * s)
    small = s + (s * s // one * s // one) // 6
    return ((small << 6) * EARTH_RADIUS_M) >> 60


def sample_points(cell):
    """Seven (lng, lat) points of a hexagon: its centre and a point part-way to each corner.
    Longitudes are kept close to the centre's, so a hexagon on the 180th meridian stays whole."""
    clat, clng = h3.cell_to_latlng(cell)
    pts = [(clng, clat)]
    for lat, lng in h3.cell_to_boundary(cell):
        d = (lng - clng + 180.0) % 360.0 - 180.0
        pts.append((clng + d * SAMPLE_REACH, clat + (lat - clat) * SAMPLE_REACH))
    return pts[:7] if len(pts) >= 7 else pts + [pts[0]] * (7 - len(pts))   # a pentagon has five corners


def land_hexagons(polys, level=LEVEL, south=-90.0, log=print, towns=()):
    """The H3 numbers of the land hexagons, ascending."""
    geod = pyproj.Geod(ellps="sphere")
    centre, touch, islands = set(), set(), []
    t = time.time()
    for p in polys:
        if p.bounds[3] <= south:
            continue
        inside = set()
        for piece in in_tiles([p]):
            inside.update(h3.geo_to_cells(piece, level))
            touch.update(h3.h3shape_to_cells_experimental(h3.geo_to_h3shape(piece), level, contain="overlap"))
        centre.update(inside)
        if not inside and abs(geod.geometry_area_perimeter(p)[0]) >= MIN_ISLAND_KM2 * 1e6:
            c = p.representative_point()
            islands.append(h3.latlng_to_cell(c.y, c.x, level))
    log(f"  centres on land {len(centre)}, touching land {len(touch)}, small islands {len(islands)} ({time.time() - t:.0f} s)")
    # hexagons that only touch land: keep those that are land at enough of their sample points
    edge = sorted(touch - centre)
    pts = np.array([q for c in edge for q in sample_points(c)], dtype=np.float64)
    pts[:, 0] = (pts[:, 0] + 180.0) % 360.0 - 180.0
    tree = shapely.STRtree(polys)
    hit, _ = tree.query(shapely.points(pts), predicate="within")
    on_land = np.zeros(len(pts), dtype=bool)
    on_land[np.unique(hit)] = True
    share = on_land.reshape(-1, 7).sum(axis=1)
    kept = {c for c, k in zip(edge, share) if k >= SAMPLES_NEEDED}
    log(f"  coastal hexagons kept {len(kept)} of {len(edge)}")
    town_cells = {h3.latlng_to_cell(lat, lng, level) for lat, lng in towns}
    by_town = (town_cells & touch) - centre - kept
    log(f"  hexagons kept for a town on the coast {len(by_town)}; towns with no land under them {len(town_cells - touch)}")
    out = centre | kept | set(islands) | by_town
    if south > -90.0:
        out = {c for c in out if h3.cell_to_latlng(c)[0] > south}
    return sorted(out, key=h3.str_to_int)


def hexagon_graph(cells):
    """Centres, and the neighbour lists with distances, for hexagons in ascending H3 order."""
    ids = np.array([h3.str_to_int(c) for c in cells], dtype=np.uint64)
    number = {c: i for i, c in enumerate(cells)}
    xyz = np.array([place(*h3.cell_to_latlng(c)) for c in cells], dtype=np.int32)
    off, dst, dist = [0], [], []
    for i, c in enumerate(cells):
        near = sorted(number[n] for n in h3.grid_ring(c, 1) if n in number)
        a = xyz[i].tolist()
        for j in near:
            dst.append(j)
            dist.append(ground_m(a, xyz[j].tolist()))
        off.append(len(dst))
    return ids, xyz, np.array(off, dtype=np.uint32), np.array(dst, dtype=np.uint32), np.array(dist, dtype=np.uint32)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    opt = {sys.argv[i]: sys.argv[i + 1] for i in range(1, len(sys.argv) - 1) if sys.argv[i].startswith("--")}
    ne_dir, out = Path(args[0]), Path(args[1])
    level, south = int(opt.get("--level", LEVEL)), float(opt.get("--south", -60.0))
    out.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    polys = load_land(ne_dir)
    print(f"land polygons: {len(polys)}")
    cells = land_hexagons(polys, level, south, towns=load_towns(ne_dir))
    ids, xyz, off, dst, dist = hexagon_graph(cells)
    for name, a in [("hex_id", ids), ("hex_xyz", xyz), ("hex_off", off), ("hex_dst", dst), ("hex_dist_m", dist)]:
        np.save(out / f"{name}.npy", a)
    area = sum(h3.cell_area(c, unit="km^2") for c in cells)
    degree = np.diff(off)
    print(f"level {level}: {len(cells)} land hexagons, {area / 1e6:.2f} million km2, "
          f"{len(dst) // 2} neighbour pairs, neighbours per hexagon {np.bincount(degree).tolist()}, "
          f"distance to a neighbour {dist.min()} to {dist.max()} m (mean {dist.mean():.0f}); {time.time() - t0:.0f} s")


if __name__ == "__main__":
    main()
