"""Step 1 of the map builder: modern Natural Earth -> the states of 1990-01-01.

Usage: python -I build_states.py <natural-earth-dir> <out-dir>
Writes <out-dir>/states_1990.geojson and <out-dir>/states_1990_report.txt.
Deterministic: all iteration is over sorted keys.
"""
import json
import sys
from pathlib import Path

import pyproj
import shapefile
from shapely.geometry import mapping, shape
from shapely.geometry import shape as to_shape
from shapely.ops import transform, unary_union
from shapely.validation import make_valid

HERE = Path(__file__).resolve().parent


def read_layer(path):
    r = shapefile.Reader(str(path))
    names = [f[0] for f in r.fields[1:]]
    out = []
    for sr in r.shapeRecords():
        rec = dict(zip(names, sr.record))
        geom = shape(sr.shape.__geo_interface__)
        if not geom.is_valid:
            geom = make_valid(geom)
        out.append((rec, geom))
    return out


def main(ne_dir, out_dir):
    ne_dir, out_dir = Path(ne_dir), Path(out_dir)
    cfg = json.loads((HERE / "config" / "states_1990.json").read_text(encoding="utf-8"))
    admin0 = read_layer(ne_dir / "ne_10m_admin_0_countries" / "ne_10m_admin_0_countries")
    admin1 = read_layer(ne_dir / "ne_10m_admin_1_states_provinces" / "ne_10m_admin_1_states_provinces")

    by_a3 = {rec["ADM0_A3"]: (rec, g) for rec, g in admin0}
    by_admin = {rec["ADMIN"]: rec["ADM0_A3"] for rec, _ in admin0 if rec["ADM0_A3"] == rec["SOV_A3"] or rec["TYPE"] in ("Sovereign country", "Country")}

    # entity -> modern "base" code: its sovereign's entity, unless the entity is itself independent-ish.
    def base_code(rec):
        sov = rec["SOVEREIGNT"]
        return by_admin.get(sov, rec["ADM0_A3"])

    merge_of = {a3: state for state, spec in cfg["merge"].items() for a3 in spec["from"]}
    names = {s: spec["name"] for s, spec in cfg["merge"].items()}
    for ent, parts in cfg["split"].items():
        for s, spec in parts.items():
            names[s] = spec["name"]

    # Assign every admin-0 entity to a 1990 state tag.
    assigned = {}   # state tag -> list of (entity code, geometry)
    unassigned, excluded = [], []
    for code in sorted(by_a3):
        rec, geom = by_a3[code]
        if code in cfg["excluded"]:
            excluded.append(code)
            continue
        if code in cfg["split"]:
            continue  # handled below
        tag = cfg["entity_to_state"].get(code) or merge_of.get(base_code(rec)) or base_code(rec)
        tag = merge_of.get(tag, tag)
        if tag in cfg["split"]:  # a dependency of a split country (none expected)
            unassigned.append(code)
            continue
        names.setdefault(tag, by_a3[tag][0]["ADMIN"] if tag in by_a3 else tag)
        assigned.setdefault(tag, []).append((code, geom))

    # Splits: cut the admin-0 outline with the admin-1 union so outer borders stay shared.
    for ent in sorted(cfg["split"]):
        rec, whole = by_a3[ent]
        parts = cfg["split"][ent]
        taken = None
        for tag in sorted(parts):
            spec = parts[tag]
            if spec.get("rest"):
                continue
            wanted = set(spec["adm1_names"])
            polys = [g for r, g in admin1 if r["adm0_a3"] == ent and r["name"] in wanted]
            found = {r["name"] for r, _ in admin1 if r["adm0_a3"] == ent and r["name"] in wanted}
            missing = wanted - found
            if missing:
                sys.exit(f"{ent}: admin-1 names not found: {sorted(missing)}")
            part = whole.intersection(unary_union(polys).buffer(0))
            assigned.setdefault(tag, []).append((ent, part))
            taken = part if taken is None else taken.union(part)
        for tag in sorted(parts):
            if parts[tag].get("rest"):
                assigned.setdefault(tag, []).append((ent, whole.difference(taken)))

    # Dissolve each state.
    states = {}
    for tag in sorted(assigned):
        states[tag] = unary_union([g for _, g in assigned[tag]])

    # Report.
    eqearth = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:8857", always_xy=True).transform
    lines = [f"{len(states)} states; {len(excluded)} excluded: {' '.join(excluded)}; unassigned: {unassigned or 'none'}", ""]
    total = 0.0
    for tag in sorted(states):
        area = transform(eqearth, states[tag]).area / 1e6
        total += area
        lines.append(f"{tag:4} {names[tag]:34} {area:14,.0f} km2  parts: {' '.join(sorted(c for c, _ in assigned[tag]))}")
    lines += ["", f"total land area: {total:,.0f} km2"]

    out_dir.mkdir(parents=True, exist_ok=True)
    feats = [{"type": "Feature", "properties": {"tag": t, "name": names[t]}, "geometry": mapping(states[t])} for t in sorted(states)]
    (out_dir / "states_1990.geojson").write_text(json.dumps({"type": "FeatureCollection", "features": feats}, sort_keys=True), encoding="utf-8")
    (out_dir / "states_1990_report.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    main(*sys.argv[1:3])
