"""Checks the render file against the intermediate data.

Usage: python -I validate_render.py <intermediate-dir> <render-file>
Fails (exit 1) if: the header or checksum is wrong; a province has no triangles at some level;
a level's total fill area differs from the true land area by more than 0.5 %; a triangle index is
out of range; at the finest level a province's fill area is off by more than 1 % and 50 km2 (coarser levels
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
    assert ver == 1 and kind == 4, f"version {ver} kind {kind}"
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
    exact = np.loadtxt(inter / "provinces_exact.csv", delimiter=",", skiprows=1)
    true_km2 = exact[:, 2]
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
        bad = int((np.abs(got - true_km2) > np.maximum(true_km2 * (0.01 if last else 0.10), 50.0 if last else 0.0)).sum())
        if abs(total - 1) > 0.005:
            problems.append(f"level {k}: total fill area is {total:.4f} of the true land area")
        if last and bad:
            problems.append(f"finest level: {bad} provinces differ more than 1 % from their true area")
        off = A[f"l{k}_line_off"].astype(np.int64)
        if not (np.diff(off) > 1).all() or off[-1] != len(A[f"l{k}_line_x"]):
            problems.append(f"level {k}: line offsets are broken")
        kinds = np.bincount(A[f"l{k}_line_kind"], minlength=3)
        if (kinds == 0).any():
            problems.append(f"level {k}: a line kind is missing {kinds.tolist()}")
        print(f"level {k} ({tol} m): fill area {total:.4f} of true, {missing} empty, {bad} off by >{1 if last else 10} %, lines {kinds.tolist()}")
    if len(A["cell_seg"]) % 4:
        problems.append("cell_seg length not a multiple of 4")
    print(f"cell outlines: {len(A['cell_seg']) // 4} segments")
    for p in problems:
        print("PROBLEM:", p)
    print(f"{len(problems)} problem(s)")
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
