"""Step 3 of the map builder: irregular tactical cells (about 225 km2 each).

Usage: python -I build_cells.py <intermediate-dir> [TAG,TAG,...]   (tags = test subset)
Reads  states_1990.geojson, regions_1990.geojson, provinces.csv, province_points_*.npy
Writes cells/ : cell_state.npy cell_province.npy cell_x.npy cell_y.npy cell_area_ha.npy
                cell_landmass.npy cell_edges.npy (a, b, shared length m) cells.wkb states.txt
Also rewrites province areas (exact) into provinces_exact.csv.

Method (deterministic, no random numbers; jitter comes from an integer hash):
  * a global jittered hex lattice supplies seeds; a state keeps the seeds inside it
    (states too small for that get one seed per province);
  * per region (a piece of a state bounded by real admin-1 lines), a Voronoi diagram of its
    seeds is clipped to the region outline, so cells never cross state or region borders and
    follow the coast exactly;
  * cell -> province by nearest labelled province sample point of the same region, then
    repaired so every province has cells and is connected within each landmass;
  * neighbours: Voronoi ridges clipped to land (same state), plus a border pass between
    states and a wrap across the 180-degree seam.
"""
import json
import math
import sys
from pathlib import Path

import numpy as np
import pyproj
import shapely
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components
from scipy.spatial import Voronoi, cKDTree
from shapely.geometry import shape

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_provinces as bp  # noqa: E402

CELL_KM2 = 225.0
SPACING_M = math.sqrt(CELL_KM2 * 1e6 / (math.sqrt(3) / 2))  # hex lattice spacing
JITTER = 0.28
MIN_SHARED_M = 300.0
BORDER_TOL_M = 500.0
SEAM_LON = 179.9999
INV = pyproj.Transformer.from_crs("EPSG:8857", "EPSG:4326", always_xy=True)


def lattice_jitter(i, j):
    """Deterministic pseudo-random pair in [-1, 1) from integer lattice coords."""
    h = (i.astype(np.uint64) * np.uint64(0x9E3779B97F4A7C15)) ^ (j.astype(np.uint64) * np.uint64(0xC2B2AE3D27D4EB4F))
    h ^= h >> np.uint64(29)
    h *= np.uint64(0xBF58476D1CE4E5B9)
    h ^= h >> np.uint64(32)
    u = (h & np.uint64(0xFFFFFFFF)).astype(np.float64) / 2**31 - 1.0
    h2 = h * np.uint64(0x94D049BB133111EB)
    h2 ^= h2 >> np.uint64(31)
    v = (h2 & np.uint64(0xFFFFFFFF)).astype(np.float64) / 2**31 - 1.0
    return u, v


def lattice_seeds(parts):
    """Jittered hex-lattice points that fall inside any of the polygons."""
    xs, ys = [], []
    s = SPACING_M
    row_h = s * math.sqrt(3) / 2
    for g in parts:
        minx, miny, maxx, maxy = g.bounds
        j0, j1 = math.floor(miny / row_h) - 1, math.ceil(maxy / row_h) + 1
        i0, i1 = math.floor(minx / s) - 1, math.ceil(maxx / s) + 1
        I, J = np.meshgrid(np.arange(i0, i1 + 1), np.arange(j0, j1 + 1))
        I, J = I.ravel(), J.ravel()
        u, v = lattice_jitter(I, J)
        X = (I + 0.5 * (J & 1)) * s + u * JITTER * s
        Y = J * row_h + v * JITTER * s
        keep = shapely.contains_xy(g, X, Y)
        xs.append(X[keep]); ys.append(Y[keep])
    if not xs:
        return np.empty(0), np.empty(0)
    return np.concatenate(xs), np.concatenate(ys)


def voronoi_cells(sx, sy):
    """Finite Voronoi polygons for each seed, plus ridge info (point pairs, vertex pairs)."""
    n = len(sx)
    if n == 1:
        return None, None, None
    ext = max(np.ptp(sx), np.ptp(sy), 1e5) * 50
    cx, cy = sx.mean(), sy.mean()
    far = np.array([[cx - ext, cy - ext], [cx + ext, cy - ext], [cx + ext, cy + ext], [cx - ext, cy + ext]])
    pts = np.vstack([np.column_stack([sx, sy]), far])
    vor = Voronoi(pts)
    polys = []
    for i in range(n):
        reg = vor.regions[vor.point_region[i]]
        polys.append(shapely.Polygon(vor.vertices[reg]))
    ridges = [(a, b, va, vb) for (a, b), (va, vb) in zip(vor.ridge_points, vor.ridge_vertices) if a < n and b < n and va >= 0 and vb >= 0]
    return np.array(polys, dtype=object), vor, ridges


BIG_PART_M2 = 20e6      # landmasses at least this big get their own cells
ISLET_ATTACH_M = 40_000  # smaller ones join a cell this close, else are binned in 100 km squares


def cells_for_part(p):
    """Cells of one landmass: (polys, seed x, seed y, edges[local a, b, length])."""
    sx, sy = lattice_seeds([p])
    if len(sx) == 0:
        rp = p.representative_point()
        sx, sy = np.array([rp.x]), np.array([rp.y])
    order = np.lexsort((sx, np.round(sy / 1000)))
    sx, sy = sx[order], sy[order]
    n = len(sx)
    if n == 1:
        return np.array([p], dtype=object), sx, sy, np.empty((0, 3), dtype=np.int64)
    shapely.prepare(p)
    polys, vor, ridges = voronoi_cells(sx, sy)
    inside = shapely.contains(p, polys)
    todo = np.where(~inside)[0]
    if len(todo):
        polys[todo] = shapely.intersection(polys[todo], p)
    polys = np.array([shapely.make_valid(q) if not q.is_valid else q for q in polys], dtype=object)
    if not ridges:
        return polys, sx, sy, np.empty((0, 3), dtype=np.int64)
    a = np.array([r[0] for r in ridges]); b = np.array([r[1] for r in ridges])
    va = vor.vertices[[r[2] for r in ridges]]; vb = vor.vertices[[r[3] for r in ridges]]
    segs = shapely.linestrings(np.stack([va, vb], axis=1))
    length = shapely.length(segs)
    part = ~shapely.contains(p, segs)
    if part.any():
        length[part] = shapely.length(shapely.intersection(segs[part], p))
    keep = length >= MIN_SHARED_M
    return polys, sx, sy, np.column_stack([a[keep], b[keep], np.rint(length[keep])]).astype(np.int64)


def valid_polygons(g):
    """A valid MultiPolygon: repair if needed and keep only the polygonal parts."""
    if not g.is_valid:
        g = shapely.make_valid(g)
    polys = [q for part in shapely.get_parts(g) for q in shapely.get_parts(part) if q.geom_type == "Polygon" and not q.is_empty]
    return shapely.MultiPolygon(polys)


def process_state(tag, geom, prov_ids, prov_pts, cell_budget_min):
    """Cells for one state. Returns dict of arrays (local indices)."""
    parts = sorted(shapely.get_parts(geom), key=lambda p: (-p.area, p.bounds))
    big_limit = min(BIG_PART_M2, parts[0].area)
    big = [(i, p) for i, p in enumerate(parts) if p.area >= big_limit]
    small = [(i, p) for i, p in enumerate(parts) if p.area < big_limit]

    polys, sxs, sys_, land, edges = [], [], [], [], []
    base = 0
    for li, p in big:
        pl, sx, sy, e = cells_for_part(p)
        polys.append(pl); sxs.append(sx); sys_.append(sy)
        land.append(np.full(len(pl), li, dtype=np.int32))
        if len(e):
            e = e.copy(); e[:, 0] += base; e[:, 1] += base; edges.append(e)
        base += len(pl)
    polys = np.concatenate(polys); sx = np.concatenate(sxs); sy = np.concatenate(sys_); land = np.concatenate(land)

    # islets: join the nearest cell if close to it (measured to the original cell), else
    # islets are binned into 100 km squares, one multi-piece cell per square.
    if small:
        original = polys.copy()
        tree = shapely.STRtree(original)
        bins = {}
        for li, q in small:
            rp = q.representative_point()
            j = int(tree.nearest(rp))
            if shapely.distance(original[j], rp) <= ISLET_ATTACH_M:
                polys[j] = shapely.union(polys[j], q)
            else:
                bins.setdefault((int(rp.x // 100_000), int(rp.y // 100_000)), []).append((li, q))
        extra_polys, extra_sx, extra_sy, extra_land = [], [], [], []
        for key in sorted(bins):
            members = bins[key]
            u = shapely.union_all([q for _, q in members])
            rp = max(members, key=lambda m: m[1].area)[1].representative_point()
            extra_polys.append(u); extra_sx.append(rp.x); extra_sy.append(rp.y); extra_land.append(members[0][0])
        if extra_polys:
            polys = np.concatenate([polys, np.array(extra_polys, dtype=object)])
            sx = np.concatenate([sx, extra_sx]); sy = np.concatenate([sy, extra_sy])
            land = np.concatenate([land, np.array(extra_land, dtype=np.int32)])
    n = len(polys)
    edges = np.vstack(edges) if edges else np.empty((0, 3), dtype=np.int64)
    cxy = shapely.get_coordinates(shapely.centroid(polys))
    area = shapely.area(polys)
    return dict(sx=sx, sy=sy, polys=polys, cx=cxy[:, 0], cy=cxy[:, 1], area=area, land=land, edges=edges, nparts=len(parts))


def main(inter, only=None):
    inter = Path(inter)
    out = inter / "cells"
    out.mkdir(exist_ok=True)
    fc = json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))
    import csv
    prow = list(csv.DictReader(open(inter / "provinces.csv", encoding="utf-8")))
    px = np.load(inter / "province_points_x.npy"); py = np.load(inter / "province_points_y.npy"); pl = np.load(inter / "province_points_label.npy")
    order = np.argsort(pl, kind="stable")
    bounds = np.searchsorted(pl[order], np.arange(len(prow) + 1))
    prov_pts = {pid: (px[order[bounds[pid]:bounds[pid + 1]]], py[order[bounds[pid]:bounds[pid + 1]]]) for pid in range(len(prow))}
    prov_by_region = {}
    for r in prow:
        prov_by_region.setdefault(int(r["region"]), []).append(int(r["id"]))

    sfc = json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))
    rfc = json.loads((inter / "regions_1990.geojson").read_text(encoding="utf-8"))
    tags = sorted(f["properties"]["tag"] for f in sfc["features"])
    regions = sorted(rfc["features"], key=lambda f: f["properties"]["region"])
    if only:
        tags = [t for t in tags if t in only]
    tag_index = {t: i for i, t in enumerate(tags)}
    # cells are numbered state by state, region by region (regions are already sorted by state tag)
    regions = sorted((f for f in regions if f["properties"]["tag"] in tag_index), key=lambda f: (f["properties"]["tag"], f["properties"]["region"]))

    C = {k: [] for k in ["state", "region", "prov", "x", "y", "area", "land", "seedx", "seedy"]}
    polys_all, edges_all, offsets = [], [], {}
    base = 0
    land_base = 0
    for feat in regions:
        tag, rid = feat["properties"]["tag"], feat["properties"]["region"]
        si = tag_index[tag]
        geom = valid_polygons(bp.project(shape(feat["geometry"])))
        r = process_state(tag, geom, None, None, None)
        n = len(r["sx"])
        # sort cells by (rounded y, rounded x) of centroid for stable ids
        idx = np.lexsort((np.round(r["cx"] / 1000), np.round(r["cy"] / 1000)))
        inv = np.empty(n, dtype=np.int64); inv[idx] = np.arange(n)
        # nearest province point, among the provinces of this region only
        pids = prov_by_region[rid]
        ppts = np.vstack([np.column_stack(prov_pts[p]) for p in pids])
        plab = np.concatenate([np.full(len(prov_pts[p][0]), p) for p in pids])
        _, nn = cKDTree(ppts).query(np.column_stack([r["cx"], r["cy"]]))
        prov = plab[nn]
        C["state"].append(np.full(n, si, dtype=np.int32)); C["region"].append(np.full(n, rid, dtype=np.int32)); C["prov"].append(prov[idx])
        C["x"].append(r["cx"][idx]); C["y"].append(r["cy"][idx]); C["area"].append(r["area"][idx])
        C["land"].append(r["land"][idx] + land_base); C["seedx"].append(r["sx"][idx]); C["seedy"].append(r["sy"][idx])
        polys_all.append(r["polys"][idx])
        e = r["edges"].copy()
        if len(e):
            e[:, 0] = inv[e[:, 0]] + base; e[:, 1] = inv[e[:, 1]] + base
            edges_all.append(e)
        offsets[rid] = (base, base + n)
        base += n
        land_base += r["nparts"]

    cat = {k: np.concatenate(v) for k, v in C.items()}
    polys = np.concatenate(polys_all)
    edges = np.concatenate(edges_all) if edges_all else np.empty((0, 3), dtype=np.int64)
    ncell = len(polys)

    # ---- province repair: every province has a cell, connected within each landmass ----
    prov = cat["prov"].copy()
    prov = repair_provinces(prov, cat, edges, prov_by_region, offsets, prow)

    # ---- cross-state borders and the 180-degree seam ----
    cross = cross_edges(polys, cat["region"])
    seam = seam_edges(polys)
    edges = np.vstack([edges, cross, seam]) if len(cross) or len(seam) else edges
    # canonical order: a<b, sorted, merged duplicates keep the longest
    lo = np.minimum(edges[:, 0], edges[:, 1]); hi = np.maximum(edges[:, 0], edges[:, 1])
    ordr = np.lexsort((-edges[:, 2], hi, lo))
    lo, hi, ln = lo[ordr], hi[ordr], edges[ordr, 2]
    first = np.ones(len(lo), dtype=bool); first[1:] = (lo[1:] != lo[:-1]) | (hi[1:] != hi[:-1])
    edges = np.column_stack([lo[first], hi[first], ln[first]]).astype(np.int64)

    # ---- write ----
    np.save(out / "cell_state.npy", cat["state"].astype(np.uint16))
    np.save(out / "cell_region.npy", cat["region"].astype(np.uint32))
    np.save(out / "cell_province.npy", prov.astype(np.uint32))
    np.save(out / "cell_x.npy", np.rint(cat["x"]).astype(np.int32))
    np.save(out / "cell_y.npy", np.rint(cat["y"]).astype(np.int32))
    np.save(out / "cell_area_ha.npy", np.rint(cat["area"] / 1e4).astype(np.uint32))
    np.save(out / "cell_landmass.npy", cat["land"].astype(np.uint32))
    np.save(out / "cell_edges.npy", edges)
    wkb = shapely.to_wkb(polys)
    with open(out / "cells.wkb", "wb") as f:
        for b in wkb:
            f.write(len(b).to_bytes(4, "little")); f.write(b)
    (out / "states.txt").write_text("\n".join(tags) + "\n", encoding="utf-8")

    # exact province areas
    exact = np.bincount(prov, weights=cat["area"] / 1e6, minlength=len(prow))
    with open(inter / "provinces_exact.csv", "w", encoding="utf-8", newline="\n") as f:
        f.write("id,cells,area_km2\n")
        cnt = np.bincount(prov, minlength=len(prow))
        for i in range(len(prow)):
            f.write(f"{i},{cnt[i]},{exact[i]:.2f}\n")
    print(f"{ncell} cells, {len(edges)} edges; mean cell area {cat['area'].mean()/1e6:.1f} km2; "
          f"cells/province mean {ncell/ max(1,len(set(prov.tolist()))):.1f}")


def repair_provinces(prov, cat, edges, prov_by_region, offsets, prow):
    n = len(prov)
    area = cat["area"]
    # 1. give every province at least one cell (take the nearest cell from a donor with spare cells)
    for rid in sorted(offsets):
        lo, hi = offsets[rid]
        for pid in prov_by_region[rid]:
            if (prov[lo:hi] == pid).any():
                continue
            mx, my = float(prow[pid]["lon"]), float(prow[pid]["lat"])
            x, y = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:8857", always_xy=True).transform(mx, my)
            d = (cat["x"][lo:hi] - x) ** 2 + (cat["y"][lo:hi] - y) ** 2
            for k in np.argsort(d, kind="stable"):
                donor = prov[lo + k]
                if (prov[lo:hi] == donor).sum() > 1:
                    prov[lo + k] = pid
                    break
    # 2. connectivity within (province, landmass)
    for _ in range(6):
        a, b = edges[:, 0], edges[:, 1]
        same = (prov[a] == prov[b]) & (cat["land"][a] == cat["land"][b])
        g = coo_matrix((np.ones(same.sum()), (a[same], b[same])), shape=(n, n))
        ncomp, comp = connected_components(g, directed=False)
        key = prov.astype(np.int64) * 10_000_000 + cat["land"]
        # component size by area
        csize = np.bincount(comp, weights=area, minlength=ncomp)
        # largest component per key
        order = np.lexsort((-csize[comp], key))
        seen = {}
        main = {}
        for i in order:
            k = key[i]
            if k not in main:
                main[k] = comp[i]
        moved = 0
        for c in np.unique(comp):
            members = np.where(comp == c)[0]
            k = key[members[0]]
            if main[k] == c:
                continue
            # neighbour province with the longest shared edge, within the same landmass
            ms = set(members.tolist())
            votes = {}
            sel = np.isin(a, members) | np.isin(b, members)
            for e in edges[sel]:
                u, v, ln = int(e[0]), int(e[1]), int(e[2])
                if (u in ms) == (v in ms):
                    continue
                o = v if u in ms else u
                if cat["land"][o] != cat["land"][members[0]] or cat["region"][o] != cat["region"][members[0]]:
                    continue
                votes[prov[o]] = votes.get(prov[o], 0) + ln
            if votes:
                best = max(sorted(votes), key=lambda p: votes[p])
                prov[members] = best
                moved += 1
        if not moved:
            break
    return prov


def cross_edges(polys, state):
    """Cells of different regions (so also of different states) that touch along a border."""
    bnd = shapely.boundary(polys)
    tree = shapely.STRtree(polys)
    qa, qb = tree.query(polys, predicate="dwithin", distance=BORDER_TOL_M)
    m = (qa < qb) & (state[qa] != state[qb])
    qa, qb = qa[m], qb[m]
    if not len(qa):
        return np.empty((0, 3), dtype=np.int64)
    shared = shapely.length(shapely.intersection(bnd[qa], shapely.buffer(polys[qb], BORDER_TOL_M)))
    keep = shared >= 1000.0
    return np.column_stack([qa[keep], qb[keep], np.rint(shared[keep])]).astype(np.int64)


def seam_edges(polys):
    """Pair cells touching the 180-degree seam from the east and west sides."""
    east, west = [], []
    bounds = shapely.bounds(polys)
    # a cell can only touch the seam if it is near the projection's edge; test candidates by lon
    cand = np.where((np.abs(bounds[:, 0]) > 5e6) | (np.abs(bounds[:, 2]) > 5e6))[0]
    for i in cand:
        c = shapely.get_coordinates(polys[i])
        lon, _ = INV.transform(c[:, 0], c[:, 1])
        lon = np.asarray(lon)
        on_e = lon >= SEAM_LON
        on_w = lon <= -SEAM_LON
        if on_e.any():
            east.append((i, c[on_e, 1].min(), c[on_e, 1].max()))
        if on_w.any():
            west.append((i, c[on_w, 1].min(), c[on_w, 1].max()))
    out = []
    for i, a0, a1 in east:
        for j, b0, b1 in west:
            ov = min(a1, b1) - max(a0, b0)
            if ov > 1000.0:
                out.append((i, j, int(ov)))
    return np.array(out, dtype=np.int64).reshape(-1, 3)


if __name__ == "__main__":
    only = set(sys.argv[2].split(",")) if len(sys.argv) > 2 else None
    main(sys.argv[1], only)
