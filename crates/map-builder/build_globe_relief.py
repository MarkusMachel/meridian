"""Shaded relief for the globe view: hills and mountains lit from the north-west, as on a
printed map. Drawing only; the engine never reads this file.

Usage: python -I build_globe_relief.py <intermediate-dir> <out.bin>

Reads the elevation grid the terrain step keeps (<intermediate>/elev_60s_int16.npy, ETOPO 2022
at one minute of arc) and writes a picture of the whole world, 8192 by 4096 (longitude by
latitude, left edge 180 W, top edge 90 N), one byte per pixel: 128 is flat ground, lighter
faces the light, darker faces away. The sea is flat.

Container kind 5, version 1. Arrays: size (u32: width, height), shade (u8, width * height).
"""
import sys
from pathlib import Path

import numpy as np
from scipy import ndimage

sys.path.insert(0, str(Path(__file__).resolve().parent))
import container  # noqa: E402

OUT_W, OUT_H = 8192, 4096
STRENGTH = 9.0          # how steep a slope has to be to go fully light or dark; tuned by eye


def hillshade(elev, first, last):
    """Shade for rows first..last of a longitude-latitude height grid (rows north to south),
    as bytes 0..255. Worked a band of rows at a time so the whole grid never sits in memory
    several times over."""
    h, w = elev.shape
    lo, hi = max(first - 1, 0), min(last + 1, h)
    z = np.maximum(np.asarray(elev[lo:hi]).astype(np.float32), 0.0)     # the sea floor is not drawn
    rows = np.arange(lo, hi)
    lat = np.radians(90.0 - (rows + 0.5) * 180.0 / h)
    metres_ns = 6_371_009.0 * np.pi / h                                  # one row
    metres_ew = np.maximum(metres_ns * np.cos(lat), 200.0)[:, None]      # one column, narrowing to the poles
    dz_east = (np.roll(z, -1, axis=1) - np.roll(z, 1, axis=1)) / (2.0 * metres_ew)
    dz_north = np.zeros_like(z)
    dz_north[1:-1] = (z[:-2] - z[2:]) / (2.0 * metres_ns)
    # light from the north-west: a slope rising to the south-east faces it
    toward_light = (dz_north - dz_east) * np.float32(0.7071)
    shade = np.clip(0.5 - toward_light * STRENGTH, 0.0, 1.0)
    return np.rint(shade[first - lo:first - lo + (last - first)] * 255.0).astype(np.uint8)


def main(inter, out_file):
    elev = np.load(Path(inter) / "elev_60s_int16.npy", mmap_mode="r")
    h = elev.shape[0]
    shade = np.concatenate([hillshade(elev, r, min(r + 720, h)) for r in range(0, h, 720)])
    small = ndimage.zoom(shade, (OUT_H / shade.shape[0], OUT_W / shade.shape[1]), order=1)[:OUT_H, :OUT_W]
    assert small.shape == (OUT_H, OUT_W), small.shape
    size = container.write(out_file, 5, 1, {"size": (np.array([OUT_W, OUT_H]), "<u4"), "shade": (small.reshape(-1), "u1")})
    print(f"{out_file}: {size:,} bytes; shade from {int(small.min())} to {int(small.max())}, {float((np.abs(small.astype(int) - 128) > 20).mean()):.1%} of pixels visibly shaded")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(*sys.argv[1:])
