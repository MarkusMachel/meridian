"""Step 2 of the map builder: split each 1990 state into provinces.

Usage: python -I build_provinces.py <intermediate-dir>
Reads  states_1990.geojson
Writes provinces.csv, provinces_report.txt, province_points_{x,y,label}.npy

Method (deterministic, no random numbers):
  1. Project to Equal Earth (equal-area, metres).
  2. Split each state into landmasses (polygon parts). Parts smaller than 0.6 of a
     province are pooled with near neighbours (within 1,500 km) into island groups.
  3. Sample each landmass (or group) on a regular grid, spacing chosen per state.
  4. Cluster the grid points into k equal-area provinces with Lloyd's k-means,
     initialised from evenly spaced points along a Morton curve.
  5. Repair clusters that came out in several pieces (fragments join the neighbour
     they touch most).
Province area here is an estimate (points x spacing^2). Exact areas come from
the cell step.
"""
import json
import math
import sys
from pathlib import Path

import numpy as np
import pyproj
import shapely
from scipy import ndimage
from scipy.cluster.hierarchy import fcluster, linkage
from scipy.spatial import cKDTree
from shapely.geometry import shape

TARGET_KM2 = 9000.0       # average province area
MIN_PROVINCES = 10        # per state, where the state is big enough
MIN_PROVINCE_KM2 = 450.0  # ... but never cut finer than about 2 tactical cells
POOL_DISTANCE_M = 1_500_000
SMALL_PART_FRACTION = 0.6
POINTS_PER_PROVINCE = 100  # grid sampling density
MAX_SPACING_M = 10_000
LLOYD_ITERATIONS = 25

PROJ = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:8857", always_xy=True)


def project(geom):
    return shapely.transform(geom, lambda c: np.column_stack(PROJ.transform(c[:, 0], c[:, 1])))


def provinces_for_area(area_km2):
    floor_rule = min(MIN_PROVINCES, max(1, int(area_km2 // MIN_PROVINCE_KM2)))
    return max(round(area_km2 / TARGET_KM2), floor_rule, 1)


def allocate(areas, total):
    """Split `total` provinces over groups in proportion to area, at least 1 each
    (largest-remainder method; ties go to the earlier group)."""
    n = len(areas)
    if total <= n:
        return [1] * n
    share = [a / sum(areas) * total for a in areas]
    ks = [max(1, int(sh)) for sh in share]
    while sum(ks) < total:
        i = max(range(n), key=lambda j: (share[j] - ks[j], -j))
        ks[i] += 1
    while sum(ks) > total:
        i = max((j for j in range(n) if ks[j] > 1), key=lambda j: (ks[j] - share[j], -j))
        ks[i] -= 1
    return ks


def morton_order(x, y):
    """Indices sorting points along a Z-order curve (deterministic spatial order)."""
    xi = ((x - x.min()) / 100.0).astype(np.uint64)
    yi = ((y - y.min()) / 100.0).astype(np.uint64)

    def spread(v):
        v = v & np.uint64(0xFFFFFFFF)
        v = (v | (v << np.uint64(16))) & np.uint64(0x0000FFFF0000FFFF)
        v = (v | (v << np.uint64(8))) & np.uint64(0x00FF00FF00FF00FF)
        v = (v | (v << np.uint64(4))) & np.uint64(0x0F0F0F0F0F0F0F0F)
        v = (v | (v << np.uint64(2))) & np.uint64(0x3333333333333333)
        v = (v | (v << np.uint64(1))) & np.uint64(0x5555555555555555)
        return v

    key = spread(xi) | (spread(yi) << np.uint64(1))
    return np.argsort(key, kind="stable")


def grid_points(geoms, spacing):
    """Regular grid points (aligned to a global lattice) inside the union of geoms."""
    xs, ys = [], []
    for g in geoms:
        minx, miny, maxx, maxy = g.bounds
        gx = np.arange(math.floor(minx / spacing) * spacing + spacing / 2, maxx, spacing)
        gy = np.arange(math.floor(miny / spacing) * spacing + spacing / 2, maxy, spacing)
        if len(gx) == 0 or len(gy) == 0:
            continue
        X, Y = np.meshgrid(gx, gy)
        X, Y = X.ravel(), Y.ravel()
        inside = shapely.contains_xy(g, X, Y)
        xs.append(X[inside])
        ys.append(Y[inside])
    if not xs:
        return np.empty(0), np.empty(0)
    return np.concatenate(xs), np.concatenate(ys)


def kmeans(x, y, k):
    n = len(x)
    if k <= 1:
        return np.zeros(n, dtype=np.int32)
    order = morton_order(x, y)
    init = order[((np.arange(k) + 0.5) * n / k).astype(np.int64)]
    centers = np.column_stack([x[init], y[init]])
    pts = np.column_stack([x, y])
    for _ in range(LLOYD_ITERATIONS):
        _, lab = cKDTree(centers).query(pts)
        sums = np.zeros((k, 2))
        np.add.at(sums, lab, pts)
        cnt = np.bincount(lab, minlength=k)
        nonempty = cnt > 0
        new = centers.copy()
        new[nonempty] = sums[nonempty] / cnt[nonempty, None]
        if np.array_equal(new, centers):
            break
        centers = new
    _, lab = cKDTree(centers).query(pts)
    return lab.astype(np.int32)


def repair_fragments(x, y, lab, spacing):
    """Make every label one connected piece (on the sampling grid). Returns new labels."""
    ix = np.rint((x - x.min()) / spacing).astype(np.int64)
    iy = np.rint((y - y.min()) / spacing).astype(np.int64)
    grid = np.full((iy.max() + 3, ix.max() + 3), -1, dtype=np.int32)  # 1-cell border of -1
    iy1, ix1 = iy + 1, ix + 1
    grid[iy1, ix1] = lab
    struct = np.ones((3, 3), dtype=bool)
    for _ in range(3):  # a few sweeps are enough in practice
        changed = False
        slices = ndimage.find_objects(grid + 1)
        for l, sl in enumerate(slices):
            if sl is None:
                continue
            # widen the box by one cell so neighbours on its edge are visible
            sl = tuple(slice(max(s.start - 1, 0), min(s.stop + 1, n)) for s, n in zip(sl, grid.shape))
            view = grid[sl]
            sub = view == l
            comp, n = ndimage.label(sub, structure=struct)
            if n <= 1:
                continue
            sizes = np.bincount(comp.ravel())[1:]
            keep = int(np.argmax(sizes)) + 1
            for c in range(1, n + 1):
                if c == keep:
                    continue
                piece = comp == c
                ring = ndimage.binary_dilation(piece, structure=struct) & ~piece
                nb = view[ring]
                nb = nb[(nb >= 0) & (nb != l)]
                if len(nb) == 0:
                    continue  # isolated by water: leave as a second piece
                view[piece] = int(np.bincount(nb).argmax())
                changed = True
        if not changed:
            break
    lab = grid[iy1, ix1]
    return lab


def main(inter):
    inter = Path(inter)
    fc = json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))
    rows = []          # (tag, name, cx, cy, area_km2, npts, nparts)
    all_x, all_y, all_l = [], [], []
    report = []
    gid = 0
    for feat in sorted(fc["features"], key=lambda f: f["properties"]["tag"]):
        tag, name = feat["properties"]["tag"], feat["properties"]["name"]
        geom = project(shape(feat["geometry"]))
        parts = sorted(shapely.get_parts(geom), key=lambda p: (-p.area, p.bounds))
        area_km2 = sum(p.area for p in parts) / 1e6
        n_state = provinces_for_area(area_km2)
        target_m2 = (area_km2 * 1e6) / n_state

        large = [p for p in parts if p.area >= SMALL_PART_FRACTION * target_m2]
        small = [p for p in parts if p.area < SMALL_PART_FRACTION * target_m2]
        if not large and small:  # state made only of small parts: treat the biggest as a group seed
            large, small = [], small
        groups = [[p] for p in large]
        if small:
            cents = np.array([[p.centroid.x, p.centroid.y] for p in small])
            if len(small) == 1:
                groups.append(small)
            else:
                cl = fcluster(linkage(cents, "single"), POOL_DISTANCE_M, criterion="distance")
                for c in sorted(set(cl)):
                    groups.append([small[i] for i in range(len(small)) if cl[i] == c])

        spacing = min(MAX_SPACING_M, math.sqrt(target_m2 / POINTS_PER_PROVINCE))
        state_rows = []
        ks = allocate([sum(p.area for p in g) for g in groups], n_state)
        for g, k in zip(groups, ks):
            x, y = grid_points(g, spacing)
            if len(x) == 0:  # tiny group the grid missed: one province at a representative point
                rp = shapely.union_all(g).representative_point()
                x, y = np.array([rp.x]), np.array([rp.y])
            k = min(k, len(x))
            lab = kmeans(x, y, k)
            if k > 1:
                lab = repair_fragments(x, y, lab, spacing)
            for l in sorted(set(lab.tolist())):
                m = lab == l
                state_rows.append([x[m].mean(), y[m].mean(), m.sum() * spacing * spacing / 1e6, int(m.sum()), len(g), x[m], y[m], spacing])
        # deterministic ids: sort by (rounded y, rounded x)
        state_rows.sort(key=lambda r: (round(r[1] / 1000), round(r[0] / 1000)))
        for r in state_rows:
            rows.append((tag, name, r[0], r[1], r[2], r[3], r[4], r[7]))
            all_x.append(r[5]); all_y.append(r[6]); all_l.append(np.full(len(r[5]), gid, dtype=np.int32))
            gid += 1
        areas = [r[2] for r in state_rows]
        report.append(f"{tag:4} {name[:30]:30} area {area_km2:12,.0f}  provinces {len(state_rows):4}  target {n_state:4}  size min/avg/max {min(areas):8,.0f} {np.mean(areas):8,.0f} {max(areas):8,.0f}")

    inv = pyproj.Transformer.from_crs("EPSG:8857", "EPSG:4326", always_xy=True)
    with open(inter / "provinces.csv", "w", encoding="utf-8", newline="\n") as f:
        f.write("id,state,lon,lat,area_km2_est,points,landmasses,spacing_m\n")
        for i, r in enumerate(rows):
            lon, lat = inv.transform(r[2], r[3])
            f.write(f"{i},{r[0]},{lon:.4f},{lat:.4f},{r[4]:.0f},{r[5]},{r[6]},{r[7]:.1f}\n")
    np.save(inter / "province_points_x.npy", np.concatenate(all_x).astype(np.float64))
    np.save(inter / "province_points_y.npy", np.concatenate(all_y).astype(np.float64))
    np.save(inter / "province_points_label.npy", np.concatenate(all_l))

    sizes = np.array([r[4] for r in rows])
    summary = [
        f"{len(rows)} provinces in {len(set(r[0] for r in rows))} states",
        f"estimated province area km2: min {sizes.min():,.0f}  median {np.median(sizes):,.0f}  mean {sizes.mean():,.0f}  max {sizes.max():,.0f}",
        f"provinces over 15,000 km2: {(sizes > 15000).sum()}; under 3,000 km2: {(sizes < 3000).sum()}",
        "",
    ]
    (inter / "provinces_report.txt").write_text("\n".join(summary + report) + "\n", encoding="utf-8")
    print("\n".join(summary))


if __name__ == "__main__":
    main(sys.argv[1])
