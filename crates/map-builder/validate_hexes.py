"""Checks for build_hexes.py (map rewrite, phase M1).

  python validate_hexes.py                 the self-test: a small made-up island, no data needed
  python validate_hexes.py <out dir>       check the arrays a real run wrote
  python validate_hexes.py <out dir> <natural-earth dir>   also compare the land area with the source
"""
import math
import sys
from pathlib import Path

import h3
import numpy as np
from shapely.geometry import Polygon, box

import build_hexes as bh


def haversine_m(lat1, lng1, lat2, lng2):
    la1, lo1, la2, lo2 = map(math.radians, (lat1, lng1, lat2, lng2))
    h = math.sin((la2 - la1) / 2) ** 2 + math.cos(la1) * math.cos(la2) * math.sin((lo2 - lo1) / 2) ** 2
    return 2 * bh.EARTH_RADIUS_M * math.asin(math.sqrt(h))


def check_graph(ids, xyz, off, dst, dist):
    """Problems with a hexagon graph, as a list of sentences (empty = fine)."""
    problems = []
    n = len(ids)
    if not (np.diff(ids.astype(np.int64)) > 0).all():
        problems.append("hexagon numbers are not ascending and distinct")
    if len(off) != n + 1 or off[0] != 0 or off[-1] != len(dst) or len(dst) != len(dist):
        problems.append("neighbour arrays have the wrong lengths")
        return problems
    degree = np.diff(off.astype(np.int64))
    if degree.max(initial=0) > 6:
        problems.append("a hexagon has more than six neighbours")
    src = np.repeat(np.arange(n, dtype=np.int64), degree)
    if (dst.astype(np.int64) == src).any():
        problems.append("a hexagon is its own neighbour")
    # every pair is listed from both sides with the same distance
    fwd = np.stack([src, dst.astype(np.int64), dist.astype(np.int64)], axis=1)
    back = np.stack([dst.astype(np.int64), src, dist.astype(np.int64)], axis=1)
    a = fwd[np.lexsort((fwd[:, 1], fwd[:, 0]))]
    b = back[np.lexsort((back[:, 1], back[:, 0]))]
    if not np.array_equal(a, b):
        problems.append("the neighbour lists are not the same from both sides")
    for i in range(n):
        row = dst[off[i]:off[i + 1]]
        if not (np.diff(row.astype(np.int64)) > 0).all():
            problems.append(f"hexagon {i}: neighbours are not ascending")
            break
    r = np.sqrt((xyz.astype(np.float64) ** 2).sum(axis=1))
    if np.abs(r - bh.EARTH_RADIUS_M).max(initial=0) > 2.0:
        problems.append("a centre is not on the sphere")
    return problems


def self_test():
    # the integer ground distance against the haversine formula, near and far
    for a, b in [((52.37, 9.73), (52.27, 10.52)), ((33.31, 44.37), (29.38, 47.98)), ((38.9, -77.04), (55.75, 37.62)), ((-18.14, 178.44), (-13.83, -171.77))]:
        got, want = bh.ground_m(bh.place(*a), bh.place(*b)), haversine_m(*a, *b)
        assert abs(got - want) < 6, (a, b, got, want)
    assert bh.ground_m((1, 2, 3), (1, 2, 3)) == 0

    # a made-up island of 2 by 2 degrees on the equator, and a speck of 4 km2 beside it
    island = box(10.0, -1.0, 12.0, 1.0)
    speck = Polygon([(13.0, 0.0), (13.018, 0.0), (13.018, 0.02), (13.0, 0.02)])
    cells = bh.land_hexagons([island, speck], 6, log=lambda *_: None)
    ids, xyz, off, dst, dist = bh.hexagon_graph(cells)
    assert check_graph(ids, xyz, off, dst, dist) == []
    area = sum(h3.cell_area(c, unit="km^2") for c in cells)
    true_area = (2 * 111.195) ** 2 + 4.0
    assert abs(area / true_area - 1) < 0.03, (area, true_area)          # the coast rule keeps the area about right
    degree = np.diff(off)
    assert (degree == 6).sum() > 0.85 * len(cells)                      # inland hexagons have all six neighbours
    assert 5_000 < dist.min() and dist.max() < 7_800, (dist.min(), dist.max())
    speck_cell = h3.latlng_to_cell(0.01, 13.009, 6)
    assert speck_cell in cells, "a small island keeps one hexagon"
    assert degree[cells.index(speck_cell)] == 0
    # a town on the shore keeps its hexagon even if most of the hexagon is water; one out at sea does not
    shore = next(c for c in sorted(h3.grid_disk(h3.latlng_to_cell(0.0, 12.02, 6), 2)) if c not in cells and h3.cell_to_latlng(c)[1] < 12.06)
    with_town = bh.land_hexagons([island, speck], 6, log=lambda *_: None, towns=[h3.cell_to_latlng(shore), (0.0, 20.0)])
    assert shore in with_town and len(with_town) == len(cells) + 1
    # a hexagon far out at sea is not land; one well inside is
    assert h3.latlng_to_cell(0.0, 20.0, 6) not in cells and h3.latlng_to_cell(0.0, 11.0, 6) in cells
    # the same input gives the same output
    again = bh.hexagon_graph(bh.land_hexagons([speck, island], 6, log=lambda *_: None))
    assert all(np.array_equal(x, y) for x, y in zip((ids, xyz, off, dst, dist), again))
    print(f"self-test passed: {len(cells)} hexagons, {area:.0f} km2 against {true_area:.0f}")


def check_run(out, ne_dir=None):
    out = Path(out)
    ids, xyz, off, dst, dist = (np.load(out / f"{n}.npy") for n in ("hex_id", "hex_xyz", "hex_off", "hex_dst", "hex_dist_m"))
    problems = check_graph(ids, xyz, off, dst, dist)
    cells = [h3.int_to_str(int(i)) for i in ids]
    level = h3.get_resolution(cells[0])
    area = sum(h3.cell_area(c, unit="km^2") for c in cells)
    degree = np.diff(off)
    print(f"{len(cells)} level-{level} hexagons, {area / 1e6:.3f} million km2, neighbours per hexagon {np.bincount(degree).tolist()}, "
          f"pentagons {sum(h3.is_pentagon(c) for c in cells)}")
    # known cities are on land, and the distances between them are right to 1 %
    cities = {"Hannover": (52.37, 9.73), "Baghdad": (33.31, 44.37), "Kuwait City": (29.38, 47.98), "Moscow": (55.75, 37.62),
              "Washington": (38.90, -77.04), "Quito": (-0.18, -78.47), "Singapore": (1.35, 103.82), "Tromso": (69.65, 18.96)}
    number = {c: i for i, c in enumerate(cells)}
    at = {}
    for name, (lat, lng) in cities.items():
        c = h3.latlng_to_cell(lat, lng, level)
        if c not in number:
            problems.append(f"{name} is not on a land hexagon")
        else:
            at[name] = xyz[number[c]].tolist()
    names = sorted(at)
    worst = 0.0
    for i, a in enumerate(names):
        for b in names[i + 1:]:
            got, want = bh.ground_m(at[a], at[b]), haversine_m(*cities[a], *cities[b])
            worst = max(worst, abs(got / want - 1))
    print(f"city-to-city distances: worst error {worst:.3%} (the cities sit at hexagon centres, up to {h3.average_hexagon_edge_length(level, 'km'):.1f} km off)")
    if worst > 0.01:
        problems.append("a distance between cities is more than 1 % out")
    if ne_dir:
        import pyproj
        geod = pyproj.Geod(ellps="sphere")
        south = min(h3.cell_to_latlng(c)[0] for c in cells[:: max(1, len(cells) // 5000)])
        true_area = sum(abs(geod.geometry_area_perimeter(p)[0]) for p in bh.load_land(ne_dir) if p.bounds[1] > south - 1.0) / 1e6
        print(f"land area of the source north of {south:.0f} degrees: {true_area / 1e6:.3f} million km2; hexagons {area / true_area - 1:+.2%}")
        if abs(area / true_area - 1) > 0.01:
            problems.append("the hexagons' area is more than 1 % from the land area of the source")
    for p in problems:
        print("PROBLEM:", p)
    return not problems


if __name__ == "__main__":
    if len(sys.argv) == 1:
        self_test()
    else:
        sys.exit(0 if check_run(*sys.argv[1:3]) else 1)
