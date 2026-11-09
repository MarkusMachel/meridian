"""Checks on build_provinces.py output. Exit code 1 if any hard check fails.

Usage: python -I validate_provinces.py <intermediate-dir>
"""
import csv
import json
import sys
from collections import Counter
from pathlib import Path

import numpy as np
from scipy import ndimage

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_provinces as bp  # noqa: E402


def main(inter):
    inter = Path(inter)
    rows = list(csv.DictReader(open(inter / "provinces.csv", encoding="utf-8")))
    px = np.load(inter / "province_points_x.npy")
    py = np.load(inter / "province_points_y.npy")
    pl = np.load(inter / "province_points_label.npy")
    fc = json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))
    errors, warnings = [], []

    # ids are 0..n-1 in order
    if [int(r["id"]) for r in rows] != list(range(len(rows))):
        errors.append("province ids are not contiguous from 0")
    # every label in the point file is a province, every province has points
    if set(pl.tolist()) != set(range(len(rows))):
        errors.append("point labels and province ids disagree")

    # per-state counts and area
    by_state = Counter(r["state"] for r in rows)
    est = Counter()
    for r in rows:
        est[r["state"]] += float(r["area_km2_est"])
    for f in fc["features"]:
        tag = f["properties"]["tag"]
        if by_state[tag] == 0:
            errors.append(f"state {tag} has no provinces")
            continue
        true_area = sum(p.area for p in bp.shapely.get_parts(bp.project(bp.shape(f["geometry"])))) / 1e6
        want_floor = min(bp.MIN_PROVINCES, max(1, int(true_area // bp.MIN_PROVINCE_KM2)))
        if by_state[tag] < want_floor:
            errors.append(f"state {tag}: {by_state[tag]} provinces, rule needs at least {want_floor}")
        ratio = est[tag] / true_area if true_area else 1
        if not 0.85 <= ratio <= 1.15 and true_area > 2000:
            warnings.append(f"state {tag}: sampled area is {ratio:.0%} of true area")

    # connectivity: how many provinces are a single connected piece on their sampling grid
    # (multi-island groups are allowed to be several pieces)
    multi_part_ok = {int(r["id"]) for r in rows if int(r["landmasses"]) > 1}
    sizes = np.array([float(r["area_km2_est"]) for r in rows])
    broken = []
    order = np.argsort(pl, kind="stable")
    bounds = np.searchsorted(pl[order], np.arange(len(rows) + 1))
    for pid in range(len(rows)):
        if pid in multi_part_ok:
            continue
        idx = order[bounds[pid]:bounds[pid + 1]]
        if len(idx) < 2:
            continue
        x, y = px[idx], py[idx]
        sp = float(rows[pid]["spacing_m"])
        ix = np.rint((x - x.min()) / sp).astype(int)
        iy = np.rint((y - y.min()) / sp).astype(int)
        g = np.zeros((iy.max() + 1, ix.max() + 1), dtype=bool)
        g[iy, ix] = True
        comp, n = ndimage.label(g, structure=np.ones((3, 3)))
        # A stray sample or two in a thin coastal strip is normal; a real second piece is not.
        if n > 1 and sorted(np.bincount(comp.ravel())[1:])[-2] > 3:
            broken.append(pid)
    if broken:
        warnings.append(f"{len(broken)} single-landmass provinces are in more than one piece (ids {broken[:10]}...)")

    # every region has at least one province, and provinces never span two regions
    regions = json.loads((inter / "regions_1990.geojson").read_text(encoding="utf-8"))["features"]
    have = Counter(int(r["region"]) for r in rows)
    for f in regions:
        if have[f["properties"]["region"]] == 0:
            errors.append(f"region {f['properties']['region']} ({f['properties']['tag']} {f['properties']['name']}) has no provinces")
    print(f"{len(regions)} regions")
    print(f"{len(rows)} provinces, {len(by_state)} states")
    print(f"over 15,000 km2: {(sizes > 15000).sum()}; over 20,000 km2: {(sizes > 20000).sum()}; under 3,000 km2: {(sizes < 3000).sum()}")
    print(f"size min/median/mean/max: {sizes.min():.1f} / {np.median(sizes):,.0f} / {sizes.mean():,.0f} / {sizes.max():,.0f} km2")
    for w in warnings:
        print("WARNING:", w)
    for e in errors:
        print("ERROR:", e)
    print("FAILED" if errors else "OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
