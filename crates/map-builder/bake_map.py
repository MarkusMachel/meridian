"""Step 7 of the map builder: write the baked map file for the engine.

Usage: python -I bake_map.py <intermediate-dir> <out-file>
Container (little-endian), same header shape as scenario files:
  magic "MRMP" | format_version u32 | kind u32 (2 = map) | payload_len u64 | checksum u64
  The checksum is CRC-32 (IEEE) of the payload, zero-extended to 64 bits.
Payload: u32 array_count, then per array:
  name_len u8 | name | dtype u8 | count u64 | raw little-endian elements
  dtype: 1 u8, 2 u16, 3 u32, 4 i16, 5 i32
Names of arrays and their meaning are listed in docs/data-formats.md.
"""
import csv
import json
import struct
import sys
import zlib
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
FORMAT_VERSION = 1
KIND_MAP = 2
DTYPES = {np.dtype("u1"): 1, np.dtype("<u2"): 2, np.dtype("<u4"): 3, np.dtype("<i2"): 4, np.dtype("<i4"): 5}


def csr(n, src, dst, cols):
    """Both directions, sorted by (src, dst). Returns offsets and the permuted columns."""
    s = np.concatenate([src, dst]); d = np.concatenate([dst, src])
    cc = [np.concatenate([c, c]) for c in cols]
    order = np.lexsort((d, s))
    s, d = s[order], d[order]
    off = np.zeros(n + 1, dtype=np.uint32)
    np.cumsum(np.bincount(s, minlength=n), out=off[1:])
    return off, d.astype(np.uint32), [c[order] for c in cc]


def strings(items):
    enc = [x.encode("utf-8") for x in items]
    off = np.zeros(len(enc) + 1, dtype=np.uint32)
    np.cumsum([len(e) for e in enc], out=off[1:])
    return np.frombuffer(b"".join(enc), dtype=np.uint8), off


def main(inter, out_file):
    inter = Path(inter)
    c = inter / "cells"
    A = {}   # name -> array (ordered)

    def put(name, arr, dt):
        A[name] = np.ascontiguousarray(np.asarray(arr).astype(dt))

    tags = (c / "states.txt").read_text().split()
    fc = json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))
    names = {f["properties"]["tag"]: f["properties"]["name"] for f in fc["features"]}
    nb, no = strings([names[t] for t in tags]); tb, to = strings(tags)
    A["state_tags_blob"], A["state_tags_off"] = tb, to
    A["state_names_blob"], A["state_names_off"] = nb, no

    prov_rows = list(csv.DictReader(open(inter / "provinces.csv", encoding="utf-8")))
    P = len(prov_rows)
    tag_index = {t: i for i, t in enumerate(tags)}
    prov_state = np.array([tag_index[r["state"]] for r in prov_rows])

    cell_state = np.load(c / "cell_state.npy"); cell_prov = np.load(c / "cell_province.npy")
    cx = np.load(c / "cell_x.npy"); cy = np.load(c / "cell_y.npy"); carea = np.load(c / "cell_area_ha.npy")
    N = len(cell_state)
    put("cell_state", cell_state, "<u2"); put("cell_province", cell_prov, "<u4")
    put("cell_x", cx, "<i4"); put("cell_y", cy, "<i4"); put("cell_area_ha", carea, "<u4")
    put("cell_landmass", np.load(c / "cell_landmass.npy"), "<u4")
    put("cell_elev_m", np.load(c / "cell_elev_m.npy"), "<i2"); put("cell_relief_m", np.load(c / "cell_relief_m.npy"), "<u2")
    put("cell_terrain", np.load(c / "cell_terrain.npy"), "u1"); put("cell_urban", np.load(c / "cell_urban.npy"), "u1")
    put("cell_flags", np.load(c / "cell_flags.npy"), "u1")
    put("cell_zone", np.load(inter / "sea" / "cell_zone.npy"), "<u4")
    put("cell_pop", np.load(inter / "population" / "cell_pop.npy"), "<u4")

    # provinces
    w = carea.astype(np.float64)
    px = np.rint(np.bincount(cell_prov, weights=cx * w, minlength=P) / np.maximum(np.bincount(cell_prov, weights=w, minlength=P), 1)).astype(np.int64)
    py = np.rint(np.bincount(cell_prov, weights=cy * w, minlength=P) / np.maximum(np.bincount(cell_prov, weights=w, minlength=P), 1)).astype(np.int64)
    put("province_state", prov_state, "<u2"); put("province_x", px, "<i4"); put("province_y", py, "<i4")
    put("province_area_ha", np.bincount(cell_prov, weights=w, minlength=P), "<u4")
    put("province_pop", np.load(inter / "population" / "province_pop.npy"), "<u4")
    order = np.argsort(cell_prov, kind="stable")
    off = np.zeros(P + 1, dtype=np.uint32); np.cumsum(np.bincount(cell_prov, minlength=P), out=off[1:])
    put("province_cell_off", off, "<u4"); put("province_cells", order, "<u4")

    # zones
    zr = list(csv.DictReader(open(inter / "sea" / "zones.csv")))
    Z = len(zr)
    zx = np.array([float(r["x_m"]) for r in zr]); zy = np.array([float(r["y_m"]) for r in zr])
    put("zone_x", np.rint(zx), "<i4"); put("zone_y", np.rint(zy), "<i4")
    put("zone_area_ha", [float(r["area_km2"]) * 100 for r in zr], "<u4")
    put("zone_body", [int(r["body"]) for r in zr], "<u2")

    # strategic graph (nodes: provinces then zones)
    e = np.load(inter / "sea" / "strategic_edges.npy")
    nx = np.concatenate([px, np.rint(zx)]); ny = np.concatenate([py, np.rint(zy)])
    a, b, kind, wt = e[:, 0], e[:, 1], e[:, 2], e[:, 3]
    strait = np.where(kind == 2, wt, 0); weight = np.where(kind == 2, 0, wt)
    cost = np.ceil(np.hypot(nx[a].astype(np.float64) - nx[b], ny[a].astype(np.float64) - ny[b])).astype(np.int64)  # rounded UP: keeps the A* heuristic admissible
    goff, gdst, (gk, gw, gs, gc) = csr(P + Z, a, b, [kind, weight, strait, cost])
    put("g_off", goff, "<u4"); put("g_dst", gdst, "<u4"); put("g_kind", gk, "u1"); put("g_weight", gw, "<u4")
    put("g_strait", gs, "<u2"); put("g_cost_m", gc, "<u4")
    chokes = json.loads((HERE / "config" / "chokepoints.json").read_text())["chokepoints"]
    cb, co = strings([x["name"] for x in chokes])
    A["choke_names_blob"], A["choke_names_off"] = cb, co
    put("choke_built", [x["built"] if x["built"] is not None else 0 for x in chokes], "<i2")

    # cell graph
    ce = np.load(c / "cell_edges.npy"); cf = np.load(c / "cell_edge_flags.npy")
    cdist = np.ceil(np.hypot(cx[ce[:, 0]].astype(np.float64) - cx[ce[:, 1]], cy[ce[:, 0]].astype(np.float64) - cy[ce[:, 1]])).astype(np.int64)
    coff, cdst, (clen, cfl, cdi) = csr(N, ce[:, 0], ce[:, 1], [ce[:, 2], cf, cdist])
    put("c_off", coff, "<u4"); put("c_dst", cdst, "<u4"); put("c_len_m", clen, "<u4"); put("c_flags", cfl, "u1")
    put("c_dist_m", cdi, "<u4")  # distance between cell centres, rounded up

    # sites
    s = inter / "sites"
    for nm, dt in [("kind", "u1"), ("cell", "<u4"), ("province", "<u4"), ("size", "<u4"), ("built", "<i2"), ("destroyed", "<i2"), ("verified", "u1")]:
        put("site_" + nm, np.load(s / f"site_{nm}.npy"), dt)
    srows = list(csv.DictReader(open(s / "sites.csv", encoding="utf-8")))
    sb, so = strings([r["name"] for r in srows])
    A["site_names_blob"], A["site_names_off"] = sb, so

    payload = bytearray(struct.pack("<I", len(A)))
    for name, arr in A.items():
        arr = np.ascontiguousarray(arr)
        dt = arr.dtype.newbyteorder("<") if arr.dtype.itemsize > 1 else arr.dtype
        code = DTYPES[np.dtype(dt)]
        nb_ = name.encode("ascii")
        payload += struct.pack("<B", len(nb_)) + nb_ + struct.pack("<BQ", code, arr.size) + arr.astype(dt, copy=False).tobytes()
    header = b"MRMP" + struct.pack("<IIQQ", FORMAT_VERSION, KIND_MAP, len(payload), zlib.crc32(bytes(payload)))
    Path(out_file).parent.mkdir(parents=True, exist_ok=True)
    Path(out_file).write_bytes(header + bytes(payload))
    print(f"{out_file}: {len(header)+len(payload)/1:,.0f} bytes, {len(A)} arrays; provinces {P}, zones {Z}, cells {N}, sites {len(srows)}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
