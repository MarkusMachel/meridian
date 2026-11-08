"""Step 6b of the map builder: 1990 population per province (an ESTIMATE).

Usage: python -I build_population.py <intermediate-dir> <natural-earth-dir> <worldbank-dir>
Writes population/ : province_pop.npy (u32) cell_pop.npy (u32) report.txt

Country totals are World Bank SP.POP.TOTL for 1990 (CC BY 4.0), attributed to the 1990
states through entity_state.json; two splits use hand estimates (see config). Inside a
state the total is spread over cells by a density proxy (area x terrain x built-up
share), with Natural Earth cities (POP_MAX scaled by the country's 1990/2019 ratio) as
point masses. This is a plausible shape, NOT census data.
"""
import json
import sys
from pathlib import Path

import numpy as np
import pyproj
import shapefile
import shapely

sys.path.insert(0, str(Path(__file__).resolve().parent))
from validate_cells import read_wkb  # noqa: E402

FWD = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:8857", always_xy=True)
SPLIT_SHARE = {  # modern entity -> {1990 state: share of its population}. Estimates.
    "DEU": {"DDR": 0.2022, "BRD": 0.7978},   # GDR about 16.0M of 79.3M in 1990
    "YEM": {"YAR": 0.79, "YMD": 0.21},       # North about 9.4M, South about 2.5M
}
EXTRA_1990 = {"TWN": 20_400_000}             # Taiwan is not in the World Bank table
WB_ALIAS = {"XKX": "KOS", "PSE": "PSX", "SSD": "SDS"}
TERRAIN_DENSITY = np.array([1.0, 0.8, 0.35, 0.08, 0.06, 0.0])  # plains hills mtn high desert ice
CITY_SHARE_CAP = 0.85


def main(inter, ne, wb):
    inter, ne, wb = Path(inter), Path(ne), Path(wb)
    c = inter / "cells"; out = inter / "population"; out.mkdir(exist_ok=True)
    ent = json.loads((inter / "entity_state.json").read_text())
    tags = (c / "states.txt").read_text().split()
    p90 = {x["countryiso3code"]: x["value"] for x in json.loads((wb / "pop1990.json").read_text())[1] if x["value"]}
    p19 = {x["countryiso3code"]: x["value"] for x in json.loads((wb / "pop2019.json").read_text())[1] if x["value"]}
    real = {x["id"] for x in json.loads((wb / "countries.json").read_text())[1] if x["region"]["value"] != "Aggregates"}

    # NE entity code for each World Bank code
    r = shapefile.Reader(str(ne / "ne_10m_admin_0_countries" / "ne_10m_admin_0_countries"))
    f = [x[0] for x in r.fields[1:]]
    by_wb, ne_recs = {}, {}
    for rec in (dict(zip(f, x)) for x in r.records()):
        ne_recs[rec["ADM0_A3"]] = rec
        for k in ("WB_A3", "ISO_A3", "ISO_A3_EH", "ADM0_A3"):
            v = rec[k]
            if v and v != "-99":
                by_wb.setdefault(v, rec["ADM0_A3"])
    state_pop = {t: 0.0 for t in tags}
    ratio = {}   # per state: pop1990 / pop2019, for scaling city sizes
    s19 = {t: 0.0 for t in tags}
    unmatched = []
    def add(code, p, q):
        if code not in ent:
            unmatched.append(code); return
        shares = SPLIT_SHARE.get(code)
        for tag in ent[code]:
            sh = shares[tag] if shares else 1.0
            state_pop[tag] += p * sh; s19[tag] += q * sh
    for wbc in sorted(p90):
        if wbc not in real:
            continue
        code = by_wb.get(WB_ALIAS.get(wbc, wbc))
        if code is None:
            unmatched.append(wbc); continue
        add(code, p90[wbc], p19.get(wbc, p90[wbc]))
    for code, p in EXTRA_1990.items():
        add(code, p, p * 1.15)
    for t in tags:
        ratio[t] = state_pop[t] / s19[t] if s19[t] else 1.0

    # density proxy per cell
    area = np.load(c / "cell_area_ha.npy").astype(np.float64)
    terrain = np.load(c / "cell_terrain.npy"); urban = np.load(c / "cell_urban.npy").astype(np.float64) / 255.0
    state = np.load(c / "cell_state.npy"); prov = np.load(c / "cell_province.npy")
    w = area * (TERRAIN_DENSITY[terrain] * 0.3 + urban * 3.0)
    n = len(area)

    # cities as point masses
    polys = read_wkb(c / "cells.wkb"); tree = shapely.STRtree(polys)
    city = np.zeros(n)
    rr = shapefile.Reader(str(ne / "ne_10m_populated_places" / "ne_10m_populated_places"))
    ff = [x[0] for x in rr.fields[1:]]
    for sr in rr.shapeRecords():
        rec = dict(zip(ff, sr.record))
        if rec["FEATURECLA"] in ("Scientific station", "Historic place") or rec["POP_MAX"] < 100_000:
            continue
        lon, lat = sr.shape.points[0]
        pt = shapely.Point(*FWD.transform(lon, lat))
        j = int(tree.nearest(pt))
        if shapely.distance(polys[j], pt) > 25_000:
            continue
        city[j] += rec["POP_MAX"] * ratio[tags[state[j]]]
    cell_pop = np.zeros(n)
    report = []
    for si, t in enumerate(tags):
        idx = np.where(state == si)[0]
        total = state_pop[t]
        if not len(idx) or total <= 0:
            continue
        cities = city[idx]
        cap = CITY_SHARE_CAP * total
        if cities.sum() > cap:
            cities = cities * (cap / cities.sum())
        rest = total - cities.sum()
        ww = w[idx]
        ww = ww / ww.sum() if ww.sum() > 0 else np.full(len(idx), 1.0 / len(idx))
        cell_pop[idx] = cities + rest * ww
    pp = np.bincount(prov, weights=cell_pop, minlength=int(prov.max()) + 1)
    np.save(out / "cell_pop.npy", np.rint(cell_pop).astype(np.uint32))
    np.save(out / "province_pop.npy", np.rint(pp).astype(np.uint32))
    world = sum(state_pop.values())
    report.append(f"world total attributed to states: {world:,.0f} (World Bank 1990 world: about 5.33 billion)")
    report.append(f"unmatched World Bank codes / NE entities: {sorted(set(unmatched))}")
    report.append(f"province population: median {np.median(pp):,.0f}, max {pp.max():,.0f}, zero {int((pp < 1).sum())}")
    big = sorted(((state_pop[t], t) for t in tags), reverse=True)[:10]
    report.append("largest states: " + ", ".join(f"{t} {v/1e6:.0f}M" for v, t in big))
    for t in ("SUN", "YUG", "DDR", "BRD", "YAR", "YMD", "CSK", "TWN"):
        if t in state_pop:
            report.append(f"{t}: {state_pop[t]/1e6:.1f}M")
    (out / "report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
    print("\n".join(report))


if __name__ == "__main__":
    main(*sys.argv[1:4])
