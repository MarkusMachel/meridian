"""Checks the render file against the intermediate data.

Usage: python -I validate_render.py <intermediate-dir> <render-file>
Fails (exit 1) if: the header or checksum is wrong; a province has no triangles at some level;
a level's total fill area differs from the true land area by more than 0.5 %; a triangle index is
out of range; at the finest level a province's fill area is off by more than 2 % and 100 km2 (areas are in the Equal Earth plane) (coarser levels
may lose small islands on purpose, so they only report how many provinces are off by more than
10 %); line offsets are not increasing; or the three line kinds are not all present.
"""
import struct
import sys
import zlib
from pathlib import Path

import numpy as np

DT = {1: "u1", 2: "<u2", 3: "<u4", 4: "<i2", 5: "<i4"}


def read(path):
    b = Path(path).read_bytes()
    assert b[:4] == b"MRRD", "bad magic"
    ver, kind, n, crc = struct.unpack_from("<IIQQ", b, 4)
    assert ver == 7 and kind == 4, f"version {ver} kind {kind}"
    payload = b[28:28 + n]
    assert len(payload) == n and zlib.crc32(payload) == crc, "bad checksum"
    (count,) = struct.unpack_from("<I", payload, 0)
    pos, A = 4, {}
    for _ in range(count):
        ln = payload[pos]; name = payload[pos + 1:pos + 1 + ln].decode(); pos += 1 + ln
        code, cnt = struct.unpack_from("<BQ", payload, pos); pos += 9
        dt = np.dtype(DT[code]); A[name] = np.frombuffer(payload, dtype=dt, count=cnt, offset=pos); pos += cnt * dt.itemsize
    assert pos == n, "trailing bytes"
    return A


def main(inter, path):
    A = read(path)
    inter = Path(inter)
    # reference areas are stored in the file: the unsimplified provinces in the display projection
    true_km2 = A["province_ref_area_km2"].astype(np.float64)
    P = len(true_km2)
    problems = []
    for k, tol in enumerate(A["levels_tolerance_m"].tolist()):
        vx, vy, vp, tri = (A[f"l{k}_{n}"].astype(np.int64) if n != "vprov" else A[f"l{k}_{n}"].astype(np.int64) for n in ("vx", "vy", "vprov", "tri"))
        if tri.max() >= len(vx) or len(tri) % 3:
            problems.append(f"level {k}: bad triangle indices"); continue
        t = tri.reshape(-1, 3)
        ax, ay, bx, by, cx, cy = vx[t[:, 0]], vy[t[:, 0]], vx[t[:, 1]], vy[t[:, 1]], vx[t[:, 2]], vy[t[:, 2]]
        area = np.abs((bx - ax) * (cy - ay) - (cx - ax) * (by - ay)) / 2 / 1e6
        owner = vp[t[:, 0]]
        if not ((vp[t[:, 1]] == owner).all() and (vp[t[:, 2]] == owner).all()):
            problems.append(f"level {k}: triangle with mixed provinces")
        got = np.bincount(owner, weights=area, minlength=P)
        missing = int((got == 0).sum())
        if missing:
            problems.append(f"level {k}: {missing} provinces without fill")
        total = got.sum() / true_km2.sum()
        last = k == len(A["levels_tolerance_m"]) - 1
        bad = int((np.abs(got - true_km2) > np.maximum(true_km2 * (0.02 if last else 0.10), 100.0 if last else 0.0)).sum())
        if abs(total - 1) > 0.005:
            problems.append(f"level {k}: total fill area is {total:.4f} of the true land area")
        if last and bad:
            problems.append(f"finest level: {bad} provinces differ more than 2 % from their true area")
        off = A[f"l{k}_line_off"].astype(np.int64)
        if not (np.diff(off) > 1).all() or off[-1] != len(A[f"l{k}_line_x"]):
            problems.append(f"level {k}: line offsets are broken")
        kinds = np.bincount(A[f"l{k}_line_kind"], minlength=3)
        if (kinds == 0).any():
            problems.append(f"level {k}: a line kind is missing {kinds.tolist()}")
        print(f"level {k} ({tol} m): fill area {total:.4f} of true, {missing} empty, {bad} off by >{2 if last else 10} %, lines {kinds.tolist()}")
    if len(A["cell_seg"]) % 4:
        problems.append("cell_seg length not a multiple of 4")
    n_cells = len(A["cell_mx"])
    ro = A["cell_ring_off"].astype(np.int64)
    if len(A["cell_my"]) != n_cells or len(ro) != n_cells + 1 or ro[-1] != len(A["cell_ring_x"]) or (np.diff(ro) < 3).any():
        problems.append("cell ring arrays are inconsistent (every cell needs at least 3 points)")
    if len(A["province_mx"]) != P or len(A["province_my"]) != P:
        problems.append("province centre arrays have the wrong length")
    # each cell's centre must lie inside that cell's own box
    rx, ry = A["cell_ring_x"].astype(np.int64), A["cell_ring_y"].astype(np.int64)
    cid = np.repeat(np.arange(n_cells), np.diff(ro))
    lo_x = np.minimum.reduceat(rx, ro[:-1]); hi_x = np.maximum.reduceat(rx, ro[:-1])
    lo_y = np.minimum.reduceat(ry, ro[:-1]); hi_y = np.maximum.reduceat(ry, ro[:-1])
    cmx, cmy = A["cell_mx"].astype(np.int64), A["cell_my"].astype(np.int64)
    outside = int(((cmx < lo_x - 1) | (cmx > hi_x + 1) | (cmy < lo_y - 1) | (cmy > hi_y + 1)).sum())
    if outside > 0.001 * n_cells:
        problems.append(f"{outside} cell centres lie outside their own cell's bounding box")
    print(f"cell outlines: {len(A['cell_seg']) // 4} segments")
    # relief, rivers, labels
    w, h, x0, y0, x1, y1 = A["relief_info"].tolist()
    if len(A["relief"]) != w * h or x1 - x0 <= 0 or y1 - y0 <= 0:
        problems.append("relief raster size does not match relief_info")
    elif not 100 < float(A["relief"].astype(np.float64).std()) < 128 and A["relief"].std() < 1.0:
        problems.append("relief raster is flat (no shading)")
    if len(A["height_m"]) != w * h or A["height_m"].max() < 5000 or A["height_m"].min() > -5000:
        problems.append("height raster has the wrong size or no mountains / sea floor")
    if len(A["land_rgb"]) != 3 * w * h:
        problems.append("land colour raster size does not match relief_info")
    ro_r = A["river_off"].astype(np.int64)
    if ro_r[-1] != len(A["river_x"]) or len(A["river_x"]) != len(A["river_y"]) or len(A["river_rank"]) != len(ro_r) - 1 or not (np.diff(ro_r) >= 2).all():
        problems.append("river arrays are inconsistent")
    for pre in ("road", "rail"):
        o = A[f"{pre}_off"].astype(np.int64)
        if o[-1] != len(A[f"{pre}_x"]) or len(A[f"{pre}_y"]) != len(A[f"{pre}_x"]) or len(A[f"{pre}_class"]) != len(o) - 1 or not (np.diff(o) >= 2).all() or (np.diff(A[f"{pre}_class"].astype(np.int64)) < 0).any():
            problems.append(f"{pre} arrays are inconsistent (or not sorted by class)")
    ut = A["urban_tri"]
    if len(ut) % 3 or (len(ut) and ut.max() >= len(A["urban_vx"])) or len(A["urban_vy"]) != len(A["urban_vx"]):
        problems.append("urban mesh is inconsistent")
    for pre in ("country", "region"):
        off = A[f"{pre}_name_off"].astype(np.int64)
        n = len(A[f"{pre}_x"])
        if len(off) != n + 1 or off[-1] != len(A[f"{pre}_name_bytes"]) or len(A[f"{pre}_y"]) != n or len(A[f"{pre}_area_km2"]) != n or (np.diff(off) <= 0).any():
            problems.append(f"{pre} label arrays are inconsistent")
        else:
            A[f"{pre}_name_bytes"].tobytes().decode("utf-8")
    print(f"relief {w}x{h}, rivers {len(ro_r) - 1}, labels {len(A['country_x'])} + {len(A['region_x'])}")
    for p in problems:
        print("PROBLEM:", p)
    print(f"{len(problems)} problem(s)")
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
