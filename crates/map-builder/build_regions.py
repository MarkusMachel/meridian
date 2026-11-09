"""Step 1b of the map builder: cut every 1990 state into regions along real admin-1 lines.

Usage: python -I build_regions.py <natural-earth-dir> <intermediate-dir>
Reads  states_1990.geojson, entity_state.json, config/states_1990.json, Natural Earth admin-1
Writes regions_1990.geojson (lon/lat, one feature per region) and regions_report.txt

A *region* is a piece of a state that follows real internal borders (Natural Earth admin-1:
provinces, states, oblasts, counties). Provinces are later cut inside a region and never across
its edge, so province and cell borders sit on real lines.

Method (deterministic, sorted iteration, no random numbers):
  1. Each admin-1 unit goes to the 1990 state of its admin-0 entity (Germany and Yemen by name).
  2. A unit is cut to its state outline, so regions tile the state exactly. Overlaps between units
     go to the earlier unit (sorted by adm1_code). Land no unit covers (slivers, coast mismatch)
     joins the touching region with the longest shared border.
  3. Regions smaller than MIN_REGION_KM2 merge into the neighbour (same admin-0 preferred) with the
     longest shared border; an island with no neighbour merges into the nearest region of the state.
  4. A state with no admin-1 data is one region.
"""
import json
import sys
from pathlib import Path

import numpy as np
import pyproj
import shapely
import shapefile
from shapely.geometry import mapping, shape
from shapely.validation import make_valid

HERE = Path(__file__).resolve().parent
MIN_REGION_KM2 = 1500.0
EPS = 1e-7  # degrees (about 1 cm): "touching" tolerance for geometry cut from the same source

_EQ = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:8857", always_xy=True)


def area_km2(geom):
    return shapely.transform(geom, lambda c: np.column_stack(_EQ.transform(c[:, 0], c[:, 1]))).area / 1e6


def clean(g):
    if not g.is_valid:
        g = make_valid(g)
    polys = [p for p in shapely.get_parts(g) if p.geom_type in ("Polygon", "MultiPolygon")]
    out = []
    for p in polys:
        out.extend(shapely.get_parts(p))
    return shapely.MultiPolygon(out) if out else shapely.Polygon()


def polygons_of(g):
    return [p for p in shapely.get_parts(g) if p.geom_type == "Polygon" and not p.is_empty]


def shared_length(a, b):
    return shapely.length(shapely.intersection(shapely.boundary(a), shapely.buffer(b, EPS)))


def read_admin1(ne_dir):
    path = ne_dir / "ne_10m_admin_1_states_provinces" / "ne_10m_admin_1_states_provinces"
    r = shapefile.Reader(str(path))
    names = [f[0] for f in r.fields[1:]]
    out = []
    for sr in r.shapeRecords():
        rec = dict(zip(names, sr.record))
        out.append((rec, clean(shape(sr.shape.__geo_interface__))))
    out.sort(key=lambda t: (t[0]["adm0_a3"], t[0]["adm1_code"], t[0]["name"]))
    return out


def units_by_state(admin1, cfg, entity_state):
    """state tag -> list of (record, geometry), in sorted order."""
    split_names = {}
    for ent, parts in cfg["split"].items():
        for tag, spec in parts.items():
            if not spec.get("rest"):
                split_names[(ent, tag)] = set(spec["adm1_names"])
    by_state = {}
    for rec, g in admin1:
        ent = rec["adm0_a3"]
        if ent in cfg["split"]:
            tag = None
            for t in sorted(cfg["split"][ent]):
                if (ent, t) in split_names and rec["name"] in split_names[(ent, t)]:
                    tag = t
            if tag is None:  # the "rest" state
                tag = next(t for t in sorted(cfg["split"][ent]) if cfg["split"][ent][t].get("rest"))
        else:
            tags = entity_state.get(ent)
            if not tags:
                continue  # excluded entity (Antarctica, reefs)
            tag = tags[0]
        by_state.setdefault(tag, []).append((rec, g))
    return by_state


def regions_of_state(tag, state_geom, units):
    """Returns a list of dicts {geom, name, adm0, area}."""
    if not units:
        return [dict(geom=state_geom, name=tag, adm0="", area=area_km2(state_geom))]
    tree = shapely.STRtree([g for _, g in units])
    regions = []
    for i, (rec, g) in enumerate(units):
        piece = shapely.intersection(g, state_geom)
        for j in sorted(tree.query(g)):
            if j < i and regions[j] is not None and not regions[j]["geom"].is_empty:
                piece = shapely.difference(piece, regions[j]["geom"])
        piece = clean(piece)
        regions.append(dict(geom=piece, name=rec["name"] or rec["adm1_code"], adm0=rec["adm0_a3"], area=0.0) if not piece.is_empty else None)
    regions = [r for r in regions if r is not None]
    # what no unit covers
    covered = shapely.union_all([r["geom"] for r in regions])
    left = [p for p in polygons_of(shapely.difference(state_geom, covered)) if p.area > 0]
    for p in left:
        best, best_len, best_dist = None, 0.0, None
        for k, r in enumerate(regions):
            if shapely.dwithin(p, r["geom"], EPS):
                ln = shared_length(p, r["geom"])
                if ln > best_len or best is None:
                    best, best_len = k, ln
        if best is None:
            d = [shapely.distance(p, r["geom"]) for r in regions]
            best = int(np.argmin(d))
        regions[best]["geom"] = clean(shapely.union(regions[best]["geom"], p))
    for r in regions:
        r["area"] = area_km2(r["geom"])
    # merge small regions, smallest first
    while len(regions) > 1:
        k = min(range(len(regions)), key=lambda i: (regions[i]["area"], regions[i]["name"]))
        small = regions[k]
        if small["area"] >= MIN_REGION_KM2:
            break
        best, best_score = None, 0.0
        for j, r in enumerate(regions):
            if j == k or not shapely.dwithin(small["geom"], r["geom"], EPS):
                continue
            score = shared_length(small["geom"], r["geom"]) * (3.0 if r["adm0"] == small["adm0"] else 1.0)
            if score > best_score or best is None:
                best, best_score = j, score
        if best is None:
            d = [(shapely.distance(small["geom"], r["geom"]), j) for j, r in enumerate(regions) if j != k]
            best = min(d)[1]
        target = regions[best]
        keep_big = target if target["area"] >= small["area"] else small
        target["geom"] = clean(shapely.union(target["geom"], small["geom"]))
        target["area"] += small["area"]
        target["name"], target["adm0"] = keep_big["name"], keep_big["adm0"]
        del regions[k]
    # the overlay can leave hairline overlaps between neighbours: make them disjoint, in order
    tree = shapely.STRtree([r["geom"] for r in regions])
    for i, r in enumerate(regions):
        for j in sorted(tree.query(r["geom"])):
            if j < i:
                inter = shapely.intersection(r["geom"], regions[j]["geom"])
                if not inter.is_empty and inter.area > 0:
                    r["geom"] = clean(shapely.difference(r["geom"], regions[j]["geom"]))
    for r in regions:
        r["area"] = area_km2(r["geom"])
    return regions


def main(ne_dir, inter):
    ne_dir, inter = Path(ne_dir), Path(inter)
    cfg = json.loads((HERE / "config" / "states_1990.json").read_text(encoding="utf-8"))
    entity_state = json.loads((inter / "entity_state.json").read_text(encoding="utf-8"))
    fc = json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))
    admin1 = read_admin1(ne_dir)
    by_state = units_by_state(admin1, cfg, entity_state)

    feats, lines = [], []
    rid = 0
    for f in sorted(fc["features"], key=lambda f: f["properties"]["tag"]):
        tag = f["properties"]["tag"]
        geom = clean(shape(f["geometry"]))
        regs = regions_of_state(tag, geom, by_state.get(tag, []))
        # deterministic order inside a state: north to south, then west to east
        for r in regs:
            c = shapely.centroid(r["geom"])
            r["key"] = (-round(c.y, 2), round(c.x, 2), r["name"])
        regs.sort(key=lambda r: r["key"])
        total = sum(r["area"] for r in regs)
        for r in regs:
            feats.append({"type": "Feature", "properties": {"region": rid, "tag": tag, "name": r["name"], "adm0": r["adm0"], "area_km2": round(r["area"], 1)},
                          "geometry": mapping(r["geom"])})
            rid += 1
        lines.append(f"{tag:4} units {len(by_state.get(tag, [])):4}  regions {len(regs):4}  area {total:12,.0f}")
    (inter / "regions_1990.geojson").write_text(json.dumps({"type": "FeatureCollection", "features": feats}, sort_keys=True), encoding="utf-8")
    areas = np.array([f["properties"]["area_km2"] for f in feats])
    summary = [f"{len(feats)} regions in {len(fc['features'])} states",
               f"area km2: min {areas.min():,.0f}  median {np.median(areas):,.0f}  mean {areas.mean():,.0f}  max {areas.max():,.0f}",
               f"provinces at 9,000 km2 each: about {int(np.maximum(1, np.rint(areas / 9000)).sum())}", ""]
    (inter / "regions_report.txt").write_text("\n".join(summary + lines) + "\n", encoding="utf-8")
    print("\n".join(summary))


if __name__ == "__main__":
    main(*sys.argv[1:3])
