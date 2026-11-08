"""Checks on the strategic graph (provinces + sea zones). Exit 1 on hard errors.

Usage: python -I validate_graph.py <intermediate-dir>
"""
import csv
import sys
from pathlib import Path

import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components


def main(inter):
    inter = Path(inter)
    e = np.load(inter / "sea" / "strategic_edges.npy")
    P = sum(1 for _ in open(inter / "provinces.csv")) - 1
    Z = sum(1 for _ in open(inter / "sea" / "zones.csv")) - 1
    n = P + Z
    errors, warnings = [], []
    if e[:, :2].max() >= n or e[:, :2].min() < 0:
        errors.append("edge refers to a node that does not exist")
    deg = np.bincount(e[:, :2].ravel(), minlength=n)
    if (deg[:P] == 0).any():
        errors.append(f"{(deg[:P]==0).sum()} provinces have no neighbours at all (first {np.where(deg[:P]==0)[0][:5].tolist()})")
    if (deg[P:] == 0).any():
        errors.append(f"{(deg[P:]==0).sum()} sea zones have no neighbours")
    g = coo_matrix((np.ones(len(e)), (e[:, 0], e[:, 1])), shape=(n, n))
    nc, comp = connected_components(g, directed=False)
    sizes = np.bincount(comp)
    main_c = int(np.argmax(sizes))
    stray = np.where(comp != main_c)[0]
    if len(stray):
        prov_stray = stray[stray < P]
        errors.append(f"graph has {nc} connected pieces; {len(stray)} nodes ({len(prov_stray)} provinces) are not connected to the main world, e.g. {stray[:8].tolist()}")
    # a land group with no coast link at all cannot be reached by sea
    land = e[e[:, 2] == 0]
    gl = coo_matrix((np.ones(len(land)), (land[:, 0], land[:, 1])), shape=(P, P))
    nl, cl = connected_components(gl, directed=False)
    coast = e[e[:, 2] == 1]
    has_coast = np.zeros(nl, dtype=bool)
    has_coast[cl[coast[:, 0]]] = True
    land_only = np.where(~has_coast)[0]
    big_land_only = [(int(c), int((cl == c).sum())) for c in land_only]
    if big_land_only:
        errors.append(f"{len(big_land_only)} landmasses have no sea link: {big_land_only[:6]}")
    # provinces must have an owner: every province row has a state
    rows = list(csv.DictReader(open(inter / "provinces.csv", encoding="utf-8")))
    if any(not r["state"] for r in rows):
        errors.append("a province has no owner")
    kinds = np.bincount(e[:, 2], minlength=4)
    print(f"{P} provinces + {Z} sea zones = {n} nodes; edges: land {kinds[0]}, coast {kinds[1]}, strait {kinds[2]}, sea {kinds[3]}")
    print(f"{nl} land groups, mean land degree {2*kinds[0]/P:.2f}")
    for w in warnings: print("WARNING:", w)
    for x in errors: print("ERROR:", x)
    print("FAILED" if errors else "OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
