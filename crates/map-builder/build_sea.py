"""Step 4 of the map builder: sea zones, coast links, chokepoints, province graph.

Usage: python -I build_sea.py <intermediate-dir> <natural-earth-dir>
Reads  states_1990.geojson, cells/*, provinces.csv, config/chokepoints.json
Writes sea/ : zones.csv zone_pixels (labels.npy), cell_zone.npy,
              strategic_edges.npy (a, b, kind, weight)   nodes 0..P-1 provinces, P.. zones
              chokepoints.txt report

Zones are cut from a 20 km raster of the water in Equal Earth coordinates. Water bodies
(connected pieces of raster water) below 100,000 km2 are lakes and get no zones; bigger
ones get zones on a jittered lattice that is finer near coasts (shells below).
Edge kinds: 0 land border, 1 coast-to-sea link, 2 strait/canal, 3 sea-to-sea.
"""
import csv
import json
import math
import sys
from pathlib import Path

import numpy as np
import pyproj
import shapefile
import shapely
from scipy import ndimage
from scipy.spatial import cKDTree
from shapely.geometry import shape

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_provinces as bp  # noqa: E402
from build_cells import lattice_jitter  # noqa: E402

RES = 20_000.0
MIN_BODY_KM2 = 100_000.0
SHELLS = [(0.0, 250_000.0, 450_000.0), (250_000.0, 1_500_000.0, 800_000.0), (1_500_000.0, 1e12, 1_400_000.0)]
MIN_ZONE_PIXELS = 25
COAST_LINK_M = 30_000.0
FWD = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:8857", always_xy=True)
INV = pyproj.Transformer.from_crs("EPSG:8857", "EPSG:4326", always_xy=True)
LAND, SEA, STRAIT_K, SEAK = 0, 1, 2, 3


def build_grid():
    x0 = FWD.transform(-180, 0)[0]
    y1 = FWD.transform(0, 90)[1]
    xs = np.arange(-abs(x0) + RES / 2, abs(x0), RES)
    ys = np.arange(-y1 + RES / 2, y1, RES)
    X, Y = np.meshgrid(xs, ys)
    lon, lat = INV.transform(X.ravel(), Y.ravel(), errcheck=False)
    lon = np.asarray(lon).reshape(X.shape); lat = np.asarray(lat).reshape(X.shape)
    valid = np.isfinite(lon) & np.isfinite(lat) & (np.abs(lon) <= 180) & (np.abs(lat) <= 90)
    # outside the map outline the inverse returns values that do not round-trip
    x2, y2 = FWD.transform(np.where(valid, lon, 0.0), np.where(valid, lat, 0.0))
    valid &= (np.abs(np.asarray(x2).reshape(X.shape) - X) < 1000.0) & (np.abs(np.asarray(y2).reshape(X.shape) - Y) < 1000.0)
    return xs, ys, X, Y, valid


def rasterize(geoms, X, Y, xs, ys):
    mask = np.zeros(X.shape, dtype=bool)
    for g in geoms:
        for p in shapely.get_parts(g):
            minx, miny, maxx, maxy = p.bounds
            c0 = max(0, int((minx - xs[0]) // RES)); c1 = min(len(xs), int((maxx - xs[0]) // RES) + 2)
            r0 = max(0, int((miny - ys[0]) // RES)); r1 = min(len(ys), int((maxy - ys[0]) // RES) + 2)
            if c1 <= c0 or r1 <= r0:
                continue
            sx = X[r0:r1, c0:c1]; sy = Y[r0:r1, c0:c1]
            mask[r0:r1, c0:c1] |= shapely.contains_xy(p, sx.ravel(), sy.ravel()).reshape(sx.shape)
    return mask


def main(inter, ne_dir):
    inter, ne_dir = Path(inter), Path(ne_dir)
    out = inter / "sea"; out.mkdir(exist_ok=True)
    xs, ys, X, Y, valid = build_grid()
    H, W = X.shape
    fc = json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))
    land = rasterize([bp.project(shape(f["geometry"])) for f in sorted(fc["features"], key=lambda f: f["properties"]["tag"])], X, Y, xs, ys)
    r = shapefile.Reader(str(ne_dir / "ne_10m_admin_0_countries" / "ne_10m_admin_0_countries"))
    ata = [shape(sr.shape.__geo_interface__) for sr in r.shapeRecords() if dict(zip([f[0] for f in r.fields[1:]], sr.record))["ADM0_A3"] == "ATA"]
    blocked = rasterize([bp.project(g) for g in ata], X, Y, xs, ys)
    water = valid & ~land & ~blocked
    print(f"grid {W}x{H}; land {land.sum()*RES*RES/1e12:.1f}M km2 (raster), water {water.sum()*RES*RES/1e12:.1f}M km2", flush=True)

    # ---- water bodies (with the 180-degree seam joined) ----
    lab, nb = ndimage.label(water)
    parent = list(range(nb + 1))
    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]; a = parent[a]
        return a
    seam_pairs = []
    for row in range(H):
        cols = np.where(valid[row])[0]
        if len(cols) < 2:
            continue
        L, R = cols[0], cols[-1]
        if water[row, L] and water[row, R]:
            seam_pairs.append((row, L, R))
            a, b = find(lab[row, L]), find(lab[row, R])
            if a != b:
                parent[b] = a
    root = np.array([find(i) for i in range(nb + 1)])
    body = np.where(lab > 0, root[lab], 0)
    sizes = np.bincount(body.ravel())
    body_ok = np.zeros(len(sizes), dtype=bool)
    body_ok[1:] = sizes[1:] * RES * RES / 1e6 >= MIN_BODY_KM2
    sea = body_ok[body] & (body > 0)
    bodies = [b for b in range(1, len(sizes)) if body_ok[b]]
    print(f"{len(bodies)} water bodies large enough for zones (of {len(np.unique(body))-1}); lakes ignored: {(water & ~sea).sum()*RES*RES/1e6:,.0f} km2", flush=True)

    # ---- seeds ----
    dist = ndimage.distance_transform_edt(~(land | blocked)) * RES
    seeds = []
    for lo, hi, sp in SHELLS:
        row_h = sp * math.sqrt(3) / 2
        j0, j1 = int(ys[0] // row_h) - 1, int(ys[-1] // row_h) + 1
        i0, i1 = int(xs[0] // sp) - 1, int(xs[-1] // sp) + 1
        I, J = np.meshgrid(np.arange(i0, i1 + 1), np.arange(j0, j1 + 1)); I, J = I.ravel(), J.ravel()
        u, v = lattice_jitter(I, J)
        sx = (I + 0.5 * (J & 1)) * sp + u * 0.25 * sp
        sy = J * row_h + v * 0.25 * sp
        c = np.rint((sx - xs[0]) / RES).astype(int); rr = np.rint((sy - ys[0]) / RES).astype(int)
        ok = (c >= 0) & (c < W) & (rr >= 0) & (rr < H)
        c, rr, sx, sy = c[ok], rr[ok], sx[ok], sy[ok]
        ok = sea[rr, c] & (dist[rr, c] >= lo) & (dist[rr, c] < hi)
        seeds.append(np.column_stack([sx[ok], sy[ok], rr[ok], c[ok]]))
    seeds = np.vstack(seeds)
    seed_body = body[seeds[:, 2].astype(int), seeds[:, 3].astype(int)]
    # every body needs a seed
    extra = []
    for b in bodies:
        if not (seed_body == b).any():
            rr, cc = np.where(body == b)
            k = len(rr) // 2
            extra.append([xs[cc[k]], ys[rr[k]], rr[k], cc[k]])
    if extra:
        seeds = np.vstack([seeds, np.array(extra)])
        seed_body = body[seeds[:, 2].astype(int), seeds[:, 3].astype(int)]
    # ---- nearest-seed assignment per body ----
    zone = np.full((H, W), -1, dtype=np.int32)
    next_id = 0
    for b in bodies:
        sb = np.where(seed_body == b)[0]
        rr, cc = np.where(body == b)
        tree = cKDTree(seeds[sb, :2])
        _, nn = tree.query(np.column_stack([xs[cc], ys[rr]]))
        zone[rr, cc] = next_id + nn
        next_id += len(sb)
    print(f"{next_id} initial zones", flush=True)

    # ---- repair: connected pieces, tiny zones ----
    zone = repair_zones(zone, seam_pairs)
    uniq = np.unique(zone[zone >= 0])
    remap = -np.ones(zone.max() + 2, dtype=np.int64); remap[uniq] = np.arange(len(uniq))
    zone = np.where(zone >= 0, remap[zone], -1).astype(np.int32)
    nz = len(uniq)

    # ---- zone stats ----
    zr, zc = np.where(zone >= 0)
    zl = zone[zr, zc]
    cnt = np.bincount(zl, minlength=nz)
    mx = np.bincount(zl, weights=xs[zc], minlength=nz) / cnt
    my = np.bincount(zl, weights=ys[zr], minlength=nz) / cnt
    # representative pixel: nearest member pixel to the mean (keeps the point in water)
    rep = np.zeros((nz, 2))
    order = np.argsort(zl, kind="stable"); starts = np.searchsorted(zl[order], np.arange(nz + 1))
    for z in range(nz):
        idx = order[starts[z]:starts[z + 1]]
        d = (xs[zc[idx]] - mx[z]) ** 2 + (ys[zr[idx]] - my[z]) ** 2
        k = idx[int(np.argmin(d))]
        rep[z] = (xs[zc[k]], ys[zr[k]])
    zone_body = np.array([body[zr[order[starts[z]]], zc[order[starts[z]]]] for z in range(nz)])

    # ---- zone-zone adjacency ----
    pairs = {}
    def add(a, b):
        if a >= 0 and b >= 0 and a != b:
            k = (min(a, b), max(a, b)); pairs[k] = pairs.get(k, 0) + 1
    for da, db in ((zone[:, :-1], zone[:, 1:]), (zone[:-1, :], zone[1:, :])):
        m = (da >= 0) & (db >= 0) & (da != db)
        for a, b in zip(da[m].tolist(), db[m].tolist()):
            add(a, b)
    for row, L, R in seam_pairs:
        add(int(zone[row, L]), int(zone[row, R]))

    # ---- coast links ----
    cx = np.load(inter / "cells" / "cell_x.npy").astype(np.float64); cy = np.load(inter / "cells" / "cell_y.npy").astype(np.float64)
    cprov = np.load(inter / "cells" / "cell_province.npy")
    wtree = cKDTree(np.column_stack([xs[zc], ys[zr]]))
    d, nn = wtree.query(np.column_stack([cx, cy]), distance_upper_bound=COAST_LINK_M)
    near = np.isfinite(d)
    cell_zone = np.full(len(cx), 0xFFFFFFFF, dtype=np.uint32)
    cell_zone[near] = zl[nn[near]]
    coast = {}
    for p, z in zip(cprov[near].tolist(), zl[nn[near]].tolist()):
        coast[(p, z)] = coast.get((p, z), 0) + 1

    # ---- province-province from cell edges ----
    ce = np.load(inter / "cells" / "cell_edges.npy")
    pa, pb = cprov[ce[:, 0]].astype(np.int64), cprov[ce[:, 1]].astype(np.int64)
    diff = pa != pb
    lo_, hi_ = np.minimum(pa[diff], pb[diff]), np.maximum(pa[diff], pb[diff])
    key = lo_ * 1_000_000 + hi_
    uk, inv = np.unique(key, return_inverse=True)
    plen = np.bincount(inv, weights=ce[diff, 2]).astype(np.int64)
    P = int(cprov.max()) + 1

    edges = [(int(k // 1_000_000), int(k % 1_000_000), LAND, int(w)) for k, w in zip(uk, plen)]
    edges += [(p, P + z, 1, n) for (p, z), n in sorted(coast.items())]
    edges += [(P + a, P + b, SEAK, n) for (a, b), n in sorted(pairs.items())]

    # ---- repair links: provinces or landmasses the thresholds left stranded ----
    repairs = []
    cstate = np.load(inter / "cells" / "cell_state.npy")
    deg = {}
    for e in edges:
        deg[e[0]] = deg.get(e[0], 0) + 1; deg[e[1]] = deg.get(e[1], 0) + 1
    ctree = cKDTree(np.column_stack([cx, cy]))
    for p in range(P):
        if deg.get(p, 0):
            continue
        mine = np.where(cprov == p)[0]
        best = None
        for i in mine:
            dd, ii = ctree.query([cx[i], cy[i]], k=12, distance_upper_bound=40_000.0)
            for d2, j in zip(np.atleast_1d(dd), np.atleast_1d(ii)):
                if np.isfinite(d2) and cprov[j] != p and cstate[j] == cstate[i] and (best is None or d2 < best[0]):
                    best = (d2, int(cprov[j]))
        if best:
            edges.append((min(p, best[1]), max(p, best[1]), LAND, 1)); repairs.append(f"province {p}: land link to {best[1]} ({best[0]/1000:.0f} km)")
    ev = np.array(edges, dtype=np.int64)
    from scipy.sparse import coo_matrix
    from scipy.sparse.csgraph import connected_components
    lm = ev[ev[:, 2] == LAND]
    nl, cl = connected_components(coo_matrix((np.ones(len(lm)), (lm[:, 0], lm[:, 1])), shape=(P, P)), directed=False)
    has_coast = np.zeros(nl, dtype=bool)
    for e in edges:
        if e[2] == 1:
            has_coast[cl[e[0]]] = True
    for c in np.where(~has_coast)[0]:
        members = np.where(cl == c)[0]
        mcells = np.where(np.isin(cprov, members))[0]
        if not len(mcells):
            continue        # provinces of states left out of a test subset have no cells
        dd, ii = wtree.query(np.column_stack([cx[mcells], cy[mcells]]), distance_upper_bound=200_000.0)
        k = int(np.argmin(dd))
        if np.isfinite(dd[k]):
            p, z = int(cprov[mcells[k]]), int(zl[ii[k]])
            edges.append((p, P + z, 1, 1)); repairs.append(f"landmass of province {p} ({len(members)} provinces): coast link to zone {z} ({dd[k]/1000:.0f} km)")
            cell_zone[mcells[k]] = z

    # ---- chokepoints ----
    cfg = json.loads((Path(__file__).resolve().parent / "config" / "chokepoints.json").read_text())
    report = [f"repair: {r}" for r in repairs]
    ex = {(e[0], e[1]): i for i, e in enumerate(edges)}
    for i, cp in enumerate(cfg["chokepoints"]):
        za = nearest_zone(cp["a"], wtree, zl); zb = nearest_zone(cp["b"], wtree, zl)
        if za is None or zb is None:
            report.append(f"{cp['name']}: NO ZONE near an endpoint"); continue
        k = (P + min(za, zb), P + max(za, zb))
        if za == zb:
            report.append(f"{cp['name']}: both ends in zone {za} (nothing to link)")
        elif k in ex:
            e = edges[ex[k]]; edges[ex[k]] = (e[0], e[1], STRAIT_K, i + 1)
            report.append(f"{cp['name']}: zones {za}-{zb} already adjacent, marked as strait")
        else:
            edges.append((k[0], k[1], STRAIT_K, i + 1)); ex[k] = len(edges) - 1
            report.append(f"{cp['name']}: zones {za}-{zb} linked (new edge)")

    edges = np.array(sorted(edges), dtype=np.int64)
    np.save(out / "strategic_edges.npy", edges)
    np.save(out / "cell_zone.npy", cell_zone)
    np.save(out / "zone_labels.npy", zone)
    inv_lonlat = [INV.transform(*rep[z]) for z in range(nz)]
    with open(out / "zones.csv", "w", encoding="utf-8", newline="\n") as f:
        f.write("id,node,x_m,y_m,lon,lat,area_km2,body\n")
        for z in range(nz):
            f.write(f"{z},{P+z},{rep[z][0]:.0f},{rep[z][1]:.0f},{inv_lonlat[z][0]:.3f},{inv_lonlat[z][1]:.3f},{cnt[z]*RES*RES/1e6:.0f},{zone_body[z]}\n")
    (out / "chokepoints.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
    kinds = np.bincount(edges[:, 2], minlength=4)
    print(f"{nz} sea zones in {len(bodies)} bodies (median {np.median(cnt)*RES*RES/1e6:,.0f} km2); provinces {P}")
    print(f"strategic edges: land {kinds[0]}, coast {kinds[1]}, strait {kinds[2]}, sea {kinds[3]}")
    print("\n".join(report))


def nearest_zone(lonlat, wtree, zl):
    x, y = FWD.transform(*lonlat)
    d, i = wtree.query([x, y], distance_upper_bound=200_000.0)
    return None if not np.isfinite(d) else int(zl[i])


def repair_zones(zone, seam_pairs):
    for _ in range(4):
        changed = False
        n = zone.max() + 1
        slices = ndimage.find_objects(zone + 1)
        counts = np.bincount(zone[zone >= 0], minlength=n)
        for z, sl in enumerate(slices):
            if sl is None or z >= n:
                continue
            sl = tuple(slice(max(s.start - 1, 0), min(s.stop + 1, m)) for s, m in zip(sl, zone.shape))
            view = zone[sl]
            comp, k = ndimage.label(view == z)
            if k == 0:
                continue
            sizes = np.bincount(comp.ravel())[1:]
            tiny = counts[z] < MIN_ZONE_PIXELS
            for c in range(1, k + 1):
                if c == int(np.argmax(sizes)) + 1 and not tiny:
                    continue
                piece = comp == c
                ring = ndimage.binary_dilation(piece) & ~piece
                nb = view[ring]; nb = nb[(nb >= 0) & (nb != z)]
                if len(nb):
                    view[piece] = int(np.bincount(nb).argmax()); changed = True
        if not changed:
            break
    return zone


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
