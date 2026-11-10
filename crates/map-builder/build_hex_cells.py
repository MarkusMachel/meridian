"""Map rewrite, phase M1: the cells of the map as H3 hexagons, joined to the 1990 provinces.

Takes the place of build_cells.py (irregular cells of about 225 km2). It writes the same files
into <intermediate>/cells/, so the later steps (sea, terrain, sites, population, bake) run on
hexagons as they ran on cells, plus three files only hexagons have.

Usage: python -I build_hex_cells.py <intermediate-dir> <natural-earth dir> [TAG,TAG,...]
Reads  states_1990.geojson, regions_1990.geojson, provinces.csv, province_points_*.npy
Writes cells/ : cell_state.npy cell_region.npy cell_province.npy cell_x.npy cell_y.npy
                cell_area_ha.npy (the hexagon's share of its region's true land area, not its own size)
                cell_landmass.npy cell_edges.npy (a, b, shared length m)
                cells.wkb states.txt
                cell_h3.npy (u64 H3 number)  cell_xyz.npy (i32, whole metres on the sphere)
                cell_edge_dist_m.npy (u32 ground distance between the two centres, per edge)
Also rewrites province areas (exact) into provinces_exact.csv.

Method:
  * the land is the 1990 regions; which hexagons are land is decided by build_hexes.py
    (centre on land, or 3 of 7 sample points, or a town on the coast);
  * a hexagon belongs to the region its centre is in (a coastal hexagon whose centre is in
    the sea: the region most of its sample points are in, else the nearest);
  * every region keeps at least one hexagon: a region too small to hold a hexagon centre
    (Monaco, the Vatican, a small island) takes the hexagon its own middle point is in;
  * hexagon -> province inside its region by the nearest labelled province sample point, then
    the same repair as build_cells.py (every province has a hexagon and is in one piece);
  * neighbours are H3's own, so nothing special is needed at the 180th meridian;
  * hexagons are numbered state by state, region by region, then by H3 number.

cell_x and cell_y stay Equal Earth metres (the later builder steps work on that plane); the
engine uses cell_xyz and the ground distances for hexagon maps.
"""
import csv
import json
import sys
import time
from pathlib import Path

import h3
import numpy as np
import shapely
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components
from scipy.spatial import cKDTree
from shapely.geometry import shape

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_cells as bc  # noqa: E402
import build_hexes as bh  # noqa: E402
import build_provinces as bp  # noqa: E402

LEVEL = bh.LEVEL


def region_parts(regions):
    """Every polygon of every region (longitude, latitude), with the place of its region in `regions`."""
    polys, owner = [], []
    for k, feat in enumerate(regions):
        g = shape(feat["geometry"])
        for p in (g.geoms if g.geom_type == "MultiPolygon" else [g]):
            p = p if p.is_valid else p.buffer(0)
            for q in (p.geoms if hasattr(p, "geoms") else [p]):
                if q.geom_type == "Polygon" and not q.is_empty:
                    polys.append(q)
                    owner.append(k)
    return polys, np.array(owner, dtype=np.int64)


def assign_regions(cells, polys, owner, n_regions, log=print):
    """The region (place in the region list) of each hexagon; -1 where it could not be told."""
    tree = shapely.STRtree(polys)
    latlng = np.array([h3.cell_to_latlng(c) for c in cells], dtype=np.float64)
    region = np.full(len(cells), -1, dtype=np.int64)
    hit_pt, hit_poly = tree.query(shapely.points(latlng[:, 1], latlng[:, 0]), predicate="within")
    region[hit_pt] = owner[hit_poly]
    # centre in the sea: the region most of the sample points are in
    loose = np.where(region < 0)[0]
    if len(loose):
        pts = np.array([q for i in loose for q in bh.sample_points(cells[i])], dtype=np.float64)
        pts[:, 0] = (pts[:, 0] + 180.0) % 360.0 - 180.0
        hp, hq = tree.query(shapely.points(pts), predicate="within")
        votes = {}
        for p, q in zip(hp.tolist(), hq.tolist()):
            votes.setdefault(p // 7, []).append(int(owner[q]))
        for k, got in votes.items():
            region[loose[k]] = max(sorted(set(got)), key=got.count)
        still = loose[region[loose] < 0]
        if len(still):
            near = tree.nearest(shapely.points(latlng[still, 1], latlng[still, 0]))
            region[still] = owner[near]
        log(f"  hexagons with their centre in the sea: {len(loose)} (by sample points {len(votes)}, by nearest region {len(still)})")
    return region


def keep_every_region(cells, region, regions, log=print):
    """Give each region with no hexagon the hexagon its middle point is in. Returns the (maybe
    longer) hexagon list and region array."""
    cells = list(cells)
    number = {c: i for i, c in enumerate(cells)}
    region = region.tolist()
    count = np.bincount(np.array(region, dtype=np.int64), minlength=len(regions)).tolist()
    given, added = 0, 0
    for k, feat in enumerate(regions):
        if count[k]:
            continue
        g = shape(feat["geometry"])
        biggest = max((g.geoms if g.geom_type == "MultiPolygon" else [g]), key=lambda p: p.area)
        pt = biggest.representative_point()
        home = h3.latlng_to_cell(pt.y, pt.x, LEVEL)
        # the hexagon it is in, or failing that the nearest one around it that can be spared
        for c in [home] + [x for ring in (1, 2, 3) for x in sorted(h3.grid_ring(home, ring))]:
            i = number.get(c)
            if i is None:
                number[c] = len(cells)
                cells.append(c)
                region.append(k)
                added += 1
                break
            if count[region[i]] > 1:
                count[region[i]] -= 1
                region[i] = k
                break
        else:
            raise SystemExit(f"region {feat['properties']} could not be given a hexagon")
        count[k] = 1
        given += 1
    log(f"  regions too small for a hexagon centre, given one: {given} (new hexagons {added})")
    return cells, np.array(region, dtype=np.int64)


def hex_polygon(cell, clat, clng):
    """The outline of a hexagon projected to the Equal Earth plane. A hexagon on the 180th
    meridian is cut off at the meridian, on its centre's side."""
    ring = []
    for lat, lng in h3.cell_to_boundary(cell):
        lng = clng + ((lng - clng + 180.0) % 360.0 - 180.0)
        ring.append((min(max(lng, -180.0), 180.0), lat))
    xs, ys = bp.PROJ.transform([p[0] for p in ring], [p[1] for p in ring])
    return shapely.Polygon(np.column_stack([xs, ys]))


def main(inter, ne_dir, only=None, log=print):
    inter = Path(inter)
    out = inter / "cells"
    out.mkdir(exist_ok=True)
    t0 = time.time()
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
    if only:
        tags = [t for t in tags if t in only]
    tag_index = {t: i for i, t in enumerate(tags)}
    regions = sorted((f for f in rfc["features"] if f["properties"]["tag"] in tag_index), key=lambda f: (f["properties"]["tag"], f["properties"]["region"]))

    # ---- which hexagons are land, and whose ----
    polys, owner = region_parts(regions)
    log(f"regions {len(regions)}, land polygons {len(polys)}")
    cells = bh.land_hexagons(polys, LEVEL, -90.0, log=log, towns=bh.load_towns(ne_dir))
    region_k = assign_regions(cells, polys, owner, len(regions), log=log)
    cells, region_k = keep_every_region(cells, region_k, regions, log=log)

    # ---- number them: state, region, H3 number ----
    ids = np.array([h3.str_to_int(c) for c in cells], dtype=np.uint64)
    rid_of = np.array([f["properties"]["region"] for f in regions], dtype=np.int64)
    state_of = np.array([tag_index[f["properties"]["tag"]] for f in regions], dtype=np.int64)
    order = np.lexsort((ids, region_k))          # the region list is already in (state, region) order
    cells = [cells[i] for i in order]
    ids, region_k = ids[order], region_k[order]
    n = len(cells)
    number = {c: i for i, c in enumerate(cells)}
    latlng = np.array([h3.cell_to_latlng(c) for c in cells], dtype=np.float64)
    xyz = np.array([bh.place(la, lo) for la, lo in latlng], dtype=np.int32)
    ex, ey = bp.PROJ.transform(latlng[:, 1], latlng[:, 0])
    ex, ey = np.asarray(ex), np.asarray(ey)
    hex_area = np.array([h3.cell_area(c, unit="m^2") for c in cells], dtype=np.float64)
    # A hexagon on the coast or on a border is not all this region's land, and a small country
    # is far smaller than its one hexagon. So the area a hexagon carries is its share of its
    # region's true area: the hexagons of a region add up to exactly the region.
    true_area = np.array([bp.project(shape(f["geometry"])).area for f in regions])
    area = hex_area * (true_area / np.bincount(region_k, weights=hex_area, minlength=len(regions)))[region_k]
    starts = np.searchsorted(region_k, np.arange(len(regions) + 1))
    offsets = {int(rid_of[k]): (int(starts[k]), int(starts[k + 1])) for k in range(len(regions))}
    log(f"  {n} hexagons numbered ({time.time() - t0:.0f} s)")

    # ---- neighbours: shared border length and ground distance ----
    ea, eb, elen, edist = [], [], [], []
    for i, c in enumerate(cells):
        a = xyz[i].tolist()
        for nb in h3.grid_ring(c, 1):
            j = number.get(nb)
            if j is None or j <= i:
                continue
            ea.append(i); eb.append(j)
            elen.append(round(h3.edge_length(h3.cells_to_directed_edge(c, nb), unit="m")))
            edist.append(bh.ground_m(a, xyz[j].tolist()))
    edges = np.column_stack([ea, eb, elen]).astype(np.int64)
    edist = np.array(edist, dtype=np.int64)
    eo = np.lexsort((edges[:, 1], edges[:, 0]))
    edges, edist = edges[eo], edist[eo]
    log(f"  {len(edges)} neighbour pairs ({time.time() - t0:.0f} s)")

    # ---- landmass: a connected piece of one region ----
    same = region_k[edges[:, 0]] == region_k[edges[:, 1]]
    g = coo_matrix((np.ones(int(same.sum())), (edges[same, 0], edges[same, 1])), shape=(n, n))
    _, land = connected_components(g, directed=False)

    # ---- provinces ----
    prov = np.zeros(n, dtype=np.int64)
    for k in range(len(regions)):
        lo, hi = starts[k], starts[k + 1]
        pids = prov_by_region[int(rid_of[k])]
        ppts = np.vstack([np.column_stack(prov_pts[p]) for p in pids])
        plab = np.concatenate([np.full(len(prov_pts[p][0]), p) for p in pids])
        _, nn = cKDTree(ppts).query(np.column_stack([ex[lo:hi], ey[lo:hi]]))
        prov[lo:hi] = plab[nn]
    cat = dict(state=state_of[region_k], region=rid_of[region_k], x=ex, y=ey, area=area, land=land.astype(np.int64))
    prov = bc.repair_provinces(prov, cat, edges, prov_by_region, offsets, prow)
    missing = sorted(set(int(r["id"]) for r in prow if r["state"] in tag_index) - set(prov.tolist()))
    if missing:
        log(f"  WARNING: {len(missing)} provinces have no hexagon (a region with fewer hexagons than provinces): {missing[:10]}")
    log(f"  provinces assigned ({time.time() - t0:.0f} s)")

    # ---- write ----
    np.save(out / "cell_state.npy", cat["state"].astype(np.uint16))
    np.save(out / "cell_region.npy", cat["region"].astype(np.uint32))
    np.save(out / "cell_province.npy", prov.astype(np.uint32))
    np.save(out / "cell_x.npy", np.rint(ex).astype(np.int32))
    np.save(out / "cell_y.npy", np.rint(ey).astype(np.int32))
    np.save(out / "cell_area_ha.npy", np.maximum(np.rint(area / 1e4), 1).astype(np.uint32))
    np.save(out / "cell_landmass.npy", land.astype(np.uint32))
    np.save(out / "cell_edges.npy", edges)
    np.save(out / "cell_h3.npy", ids)
    np.save(out / "cell_xyz.npy", xyz)
    np.save(out / "cell_edge_dist_m.npy", edist.astype(np.uint32))
    with open(out / "cells.wkb", "wb") as f:
        for lo in range(0, n, 200_000):
            chunk = [hex_polygon(cells[i], latlng[i, 0], latlng[i, 1]) for i in range(lo, min(n, lo + 200_000))]
            for b in shapely.to_wkb(np.array(chunk, dtype=object)):
                f.write(len(b).to_bytes(4, "little")); f.write(b)
    (out / "states.txt").write_text("\n".join(tags) + "\n", encoding="utf-8")
    exact = np.bincount(prov, weights=area / 1e6, minlength=len(prow))
    cnt = np.bincount(prov, minlength=len(prow))
    with open(inter / "provinces_exact.csv", "w", encoding="utf-8", newline="\n") as f:
        f.write("id,cells,area_km2\n")
        for i in range(len(prow)):
            f.write(f"{i},{cnt[i]},{exact[i]:.2f}\n")
    degree = np.bincount(np.concatenate([edges[:, 0], edges[:, 1]]), minlength=n)
    log(f"{n} hexagons, {len(edges)} edges; mean hexagon {hex_area.mean() / 1e6:.1f} km2; hexagons cover {hex_area.sum() / 1e12:.3f} million km2, "
        f"the land they stand for {area.sum() / 1e12:.3f} million km2; "
        f"hexagons per province mean {n / max(1, len(set(prov.tolist()))):.0f}, fewest {cnt[cnt > 0].min()}; "
        f"neighbours per hexagon {np.bincount(degree).tolist()}; {time.time() - t0:.0f} s")


if __name__ == "__main__":
    only = set(sys.argv[3].split(",")) if len(sys.argv) > 3 else None
    main(sys.argv[1], sys.argv[2], only)
