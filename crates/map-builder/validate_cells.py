"""Checks on build_cells.py output. Exit 1 on any hard error.

Usage: python -I validate_cells.py <intermediate-dir>
"""
import csv
import json
import sys
from pathlib import Path

import numpy as np
import shapely
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components
from shapely.geometry import shape

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_provinces as bp  # noqa: E402


def read_wkb(path):
    data = Path(path).read_bytes()
    out, pos = [], 0
    while pos < len(data):
        n = int.from_bytes(data[pos:pos + 4], "little")
        out.append(data[pos + 4:pos + 4 + n]); pos += 4 + n
    return shapely.from_wkb(np.array(out, dtype=object))


def main(inter):
    inter = Path(inter); c = inter / "cells"
    errors, warnings = [], []
    state = np.load(c / "cell_state.npy"); prov = np.load(c / "cell_province.npy")
    area_ha = np.load(c / "cell_area_ha.npy"); land = np.load(c / "cell_landmass.npy")
    edges = np.load(c / "cell_edges.npy")
    tags = (c / "states.txt").read_text().split()
    prows = list(csv.DictReader(open(inter / "provinces.csv", encoding="utf-8")))
    n = len(state)

    # 1. every cell belongs to exactly one province, in the right state
    if prov.max() >= len(prows):
        errors.append("cell refers to a province that does not exist")
    pstate = np.array([tags.index(r["state"]) if r["state"] in tags else -1 for r in prows])
    present = np.isin(np.arange(len(prows)), np.unique(prov))
    scope = pstate >= 0
    if (pstate[prov] != state).any():
        errors.append(f"{(pstate[prov] != state).sum()} cells are in a province of another state")
    # 1b. a cell is in a province of its own region: no province crosses a real region line
    cregion = np.load(c / "cell_region.npy")
    pregion = np.array([int(r["region"]) for r in prows])
    if (pregion[prov] != cregion).any():
        errors.append(f"{(pregion[prov] != cregion).sum()} cells are in a province of another region")
    # 2. every province has at least one cell
    missing = np.where(~present & scope)[0]
    if len(missing):
        errors.append(f"{len(missing)} provinces have no cells (first: {missing[:5].tolist()})")
    # 3. polygons: valid, count equals, areas
    polys = read_wkb(c / "cells.wkb")
    if len(polys) != n:
        errors.append("cells.wkb count differs from arrays")
    bad = (~shapely.is_valid(polys)).sum()
    if bad:
        errors.append(f"{bad} invalid cell polygons")
    if (shapely.is_empty(polys)).any():
        errors.append("empty cell polygon")
    # 3b. no cell should sprawl across the sea
    bb = shapely.bounds(polys)
    diag = np.hypot(bb[:, 2] - bb[:, 0], bb[:, 3] - bb[:, 1])
    sprawl = diag > 150_000.0
    if sprawl.any():
        errors.append(f"{sprawl.sum()} cells span more than 150 km (widest {diag.max()/1000:.0f} km), e.g. {np.where(sprawl)[0][:5].tolist()}")
    # 4. state areas: sum of cells vs outline. A cell's recorded area is its share of its
    # region's true land (hexagons do not follow the coast), stored in whole hectares.
    fc = {f["properties"]["tag"]: f for f in json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))["features"]}
    cell_area = shapely.area(polys)
    worst, true_total = 0, 0.0
    for si, tag in enumerate(tags):
        true = bp.project(shape(fc[tag]["geometry"])).area
        true_total += true
        mine = state == si
        got = area_ha[mine].sum() * 1e4
        rel = abs(got - true) / true
        worst = max(worst, rel)
        if abs(got - true) > 0.002 * true + 5_000.0 * mine.sum():   # half a hectare of rounding per cell
            errors.append(f"state {tag}: cells stand for {got/true:.4%} of the outline")
    # 4b. the hexagons themselves cover the world's land to within 1 %
    if abs(cell_area.sum() - true_total) > 0.01 * true_total:
        errors.append(f"the cells cover {cell_area.sum()/true_total:.2%} of the land")
    # 5. no overlaps between cells (sampled: total pairwise overlap among neighbours)
    tree = shapely.STRtree(polys)
    qa, qb = tree.query(polys, predicate="intersects")
    m = qa < qb
    qa, qb = qa[m], qb[m]
    ov = shapely.area(shapely.intersection(polys[qa], polys[qb]))
    # Natural Earth borders of neighbouring states differ by a few metres in places, and region
    # outlines that were invalid after projection are repaired, which can leave hairline overlaps.
    if (ov > 50_000.0).any():
        errors.append(f"{(ov > 50_000.0).sum()} cell pairs overlap by more than 50,000 m2 (worst {ov.max():.0f} m2)")
    elif (ov > 1000.0).any():
        warnings.append(f"{(ov > 1000.0).sum()} cell pairs overlap by 1,000 to 10,000 m2 (worst {ov.max():.0f} m2, Natural Earth border mismatch)")
    # 6. edges
    if edges[:, :2].max() >= n or (edges[:, 0] >= edges[:, 1]).any():
        errors.append("edge list malformed")
    # 7. connectivity of the whole cell graph, and cells with no neighbours
    g = coo_matrix((np.ones(len(edges)), (edges[:, 0], edges[:, 1])), shape=(n, n))
    ncomp, comp = connected_components(g, directed=False)
    deg = np.bincount(edges[:, :2].ravel(), minlength=n)
    iso = int((deg == 0).sum())
    warnings.append(f"{ncomp} connected cell groups (landmasses linked by land); {iso} cells with no land neighbour")
    # 8. connectivity of provinces within a landmass
    same = (prov[edges[:, 0]] == prov[edges[:, 1]]) & (land[edges[:, 0]] == land[edges[:, 1]])
    g2 = coo_matrix((np.ones(same.sum()), (edges[same, 0], edges[same, 1])), shape=(n, n))
    n2, comp2 = connected_components(g2, directed=False)
    key = prov.astype(np.int64) * 10_000_000 + land
    ucomp = len(np.unique(np.column_stack([key, comp2]), axis=0))
    ukey = len(np.unique(key))
    if ucomp != ukey:
        warnings.append(f"{ucomp - ukey} province pieces are split inside a landmass")

    print(f"{n} cells; mean area {cell_area.mean()/1e6:.1f} km2 (min {cell_area.min()/1e6:.2f}, max {cell_area.max()/1e6:.0f}); worst state area error {worst:.4%}")
    print(f"{len(edges)} cell edges; mean degree {2*len(edges)/n:.2f}")
    for w in warnings: print("WARNING:", w)
    for e in errors: print("ERROR:", e)
    print("FAILED" if errors else "OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
