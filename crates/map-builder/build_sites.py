"""Step 6a of the map builder: the site table (cities, ports, airfields, strategic sites).

Usage: python -I build_sites.py <intermediate-dir> <natural-earth-dir>
Writes sites/ : sites.csv  site_*.npy (kind u8, cell u32, province u32, size u32,
                built i16, destroyed i16 (32767 = none), verified u8)  report.txt

Sources and limits:
  * cities: Natural Earth populated places with POP_MAX >= 300,000 or any capital;
    ports: scalerank <= 6; airports: scalerank <= 5. Natural Earth is modern, so
    built = 0 with verified = 0 means "assumed to exist in 1990, not checked".
    City size is Natural Earth's POP_MAX (about 2010), NOT a 1990 figure.
  * strategic: config/strategic_sites.json, hand-written, verified = 1 only for built years
    the author is confident of; coordinates need checking.
Kinds: 0 city, 1 port, 2 airfield, 3 crossing, 4 industry, 5 base (same order as the engine).
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
KIND = {"City": 0, "Port": 1, "Airfield": 2, "Crossing": 3, "Industry": 4, "Base": 5}
MAX_ANCHOR_M = 25_000.0
NONE_YEAR = 32767


def read(ne, name, keep):
    r = shapefile.Reader(str(ne / name / name))
    f = [x[0] for x in r.fields[1:]]
    out = []
    for sr in r.shapeRecords():
        rec = dict(zip(f, sr.record))
        if keep(rec):
            out.append((rec, sr.shape.points[0]))
    return out


def main(inter, ne):
    inter, ne = Path(inter), Path(ne)
    c = inter / "cells"; out = inter / "sites"; out.mkdir(exist_ok=True)
    polys = read_wkb(c / "cells.wkb")
    prov = np.load(c / "cell_province.npy"); state = np.load(c / "cell_state.npy"); flags = np.load(c / "cell_flags.npy")
    tags = (c / "states.txt").read_text().split()
    tree = shapely.STRtree(polys)
    coast_idx = np.where((flags & 8) != 0)[0]
    coast_tree = shapely.STRtree(polys[coast_idx])

    items = []  # (kind, name, lon, lat, size, built, destroyed, verified, source, prefer_coast)
    for rec, (lon, lat) in read(ne, "ne_10m_populated_places",
                                lambda r: r["FEATURECLA"] in ("Populated place", "Admin-1 capital", "Admin-0 capital", "Admin-0 capital alt", "Admin-0 region capital", "Admin-1 region capital")
                                and (r["POP_MAX"] >= 300_000 or r["FEATURECLA"].startswith("Admin-0"))):
        items.append((0, rec["NAME"], lon, lat, int(rec["POP_MAX"]), 0, NONE_YEAR, 0, "ne_populated_places", False))
    for rec, (lon, lat) in read(ne, "ne_10m_ports", lambda r: r["scalerank"] <= 6):
        items.append((1, rec["name"], lon, lat, 0, 0, NONE_YEAR, 0, "ne_ports", False))
    for rec, (lon, lat) in read(ne, "ne_10m_airports", lambda r: r["scalerank"] <= 5):
        items.append((2, rec["name"], lon, lat, 0, 0, NONE_YEAR, 0, "ne_airports", False))
    cfg = json.loads((Path(__file__).resolve().parent / "config" / "strategic_sites.json").read_text(encoding="utf-8"))
    for s in cfg["sites"]:
        items.append((KIND[s["kind"]], s["name"], s["lon"], s["lat"], 0, s["built"], s.get("destroyed", NONE_YEAR), 1, "strategic", False))

    dropped = []
    rows = []
    for it in items:
        kind, name, lon, lat, size, built, destroyed, verified, src, prefer_coast = it
        x, y = FWD.transform(lon, lat)
        pt = shapely.Point(x, y)
        if prefer_coast and len(coast_idx):
            j = int(coast_idx[coast_tree.nearest(pt)])
        else:
            j = int(tree.nearest(pt))
        d = shapely.distance(polys[j], pt)
        if d > MAX_ANCHOR_M:
            dropped.append(f"{src}: {name} ({lon:.2f}, {lat:.2f}) is {d/1000:.0f} km from any land cell")
            continue
        rows.append((kind, tags[state[j]], -size, name, j, int(prov[j]), size, built, destroyed, verified, src, lon, lat))
    rows.sort(key=lambda r: (r[0], r[1], r[2], r[3], r[4]))
    n = len(rows)
    np.save(out / "site_kind.npy", np.array([r[0] for r in rows], dtype=np.uint8))
    np.save(out / "site_cell.npy", np.array([r[4] for r in rows], dtype=np.uint32))
    np.save(out / "site_province.npy", np.array([r[5] for r in rows], dtype=np.uint32))
    np.save(out / "site_size.npy", np.array([r[6] for r in rows], dtype=np.uint32))
    np.save(out / "site_built.npy", np.array([r[7] for r in rows], dtype=np.int16))
    np.save(out / "site_destroyed.npy", np.array([r[8] for r in rows], dtype=np.int16))
    np.save(out / "site_verified.npy", np.array([r[9] for r in rows], dtype=np.uint8))
    with open(out / "sites.csv", "w", encoding="utf-8", newline="\n") as f:
        f.write("id,kind,state,name,cell,province,size,built,destroyed,verified,source,lon,lat\n")
        for i, r in enumerate(rows):
            nm = r[3].replace('"', "'")
            f.write(f'{i},{r[0]},{r[1]},"{nm}",{r[4]},{r[5]},{r[6]},{r[7]},{r[8]},{r[9]},{r[10]},{r[11]:.4f},{r[12]:.4f}\n')
    counts = np.bincount([r[0] for r in rows], minlength=6)
    report = [f"{n} sites: city {counts[0]}, port {counts[1]}, airfield {counts[2]}, crossing {counts[3]}, industry {counts[4]}, base {counts[5]}",
              f"dropped (not within {MAX_ANCHOR_M/1000:.0f} km of a land cell): {len(dropped)}"] + dropped
    (out / "report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
    print("\n".join(report[:2]))
    for d in dropped[:12]:
        print("  ", d)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
