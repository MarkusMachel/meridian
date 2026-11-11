"""The ground of the globe view: heights for the lighting and a painted colour for the land.
Drawing only; the engine never reads this file.

Usage: python -I build_globe_ground.py <intermediate-dir> <natural-earth-dir> <out.bin>

Reads the elevation grid the terrain step keeps (<intermediate>/elev_60s_int16.npy, ETOPO 2022
at one minute of arc) and Natural Earth II's land colours (NE2_HR_LC, a hand-tuned picture of
the world's land cover, public domain), and writes both as pictures of the whole world, 8192 by
4096 (longitude by latitude, left edge 180 W, top edge 90 N).

Container kind 5, version 2. Arrays:
  size    u32: width, height
  height  u16 per pixel: metres above the sea (0 for the sea)
  color   u8, four per pixel: red, green, blue, 255
"""
import sys
from pathlib import Path

import numpy as np
from scipy import ndimage

sys.path.insert(0, str(Path(__file__).resolve().parent))
import container  # noqa: E402

OUT_W, OUT_H = 8192, 4096


def shrink(a):
    """A longitude-latitude picture (rows, columns[, channels]) brought to OUT_H by OUT_W, each
    new pixel blended from the four old ones around its middle. Done a band of rows at a time,
    so a big picture is never held in memory as floats."""
    h, w = a.shape[:2]
    x = (np.arange(OUT_W) + 0.5) * (w / OUT_W) - 0.5
    x0 = np.floor(x).astype(np.int64); fx = (x - x0).astype(np.float32)
    x1 = (x0 + 1) % w; x0 = x0 % w                       # the picture wraps round the world
    out = np.empty((OUT_H, OUT_W) + a.shape[2:], dtype=a.dtype)
    for r in range(0, OUT_H, 256):
        y = (np.arange(r, min(r + 256, OUT_H)) + 0.5) * (h / OUT_H) - 0.5
        y0 = np.clip(np.floor(y).astype(np.int64), 0, h - 1); fy = np.clip(y - y0, 0.0, 1.0).astype(np.float32)
        y1 = np.minimum(y0 + 1, h - 1)
        top = np.asarray(a[y0]).astype(np.float32); bottom = np.asarray(a[y1]).astype(np.float32)
        wx = fx.reshape((1, -1) + (1,) * (a.ndim - 2)); wy = fy.reshape((-1, 1) + (1,) * (a.ndim - 2))
        band = (top[:, x0] * (1 - wx) + top[:, x1] * wx) * (1 - wy) + (bottom[:, x0] * (1 - wx) + bottom[:, x1] * wx) * wy
        out[r:r + len(y)] = np.rint(band).astype(a.dtype)
    return out


def main(inter, ne_dir, out_file):
    elev = np.load(Path(inter) / "elev_60s_int16.npy", mmap_mode="r")
    height = np.maximum(shrink(elev), 0).astype(np.uint16)
    tif = Path(ne_dir) / "NE2_HR_LC" / "NE2_HR_LC.tif"
    if tif.exists():
        import tifffile
        src = tifffile.imread(str(tif))                 # (10800, 21600, 3) u8
        rgb = shrink(src[..., :3])
        painted = "Natural Earth II"
    else:
        # without the painted picture: a plain colour by height, green lowland to brown to white
        h = height.astype(np.float32)
        stops = np.array([0, 300, 1200, 2500, 4500], dtype=np.float32)
        ramp = np.array([[150, 178, 122], [176, 180, 126], [168, 150, 112], [150, 136, 124], [236, 238, 240]], dtype=np.float32)
        rgb = np.stack([np.interp(h, stops, ramp[:, k]) for k in range(3)], axis=-1).astype(np.uint8)
        painted = "a plain colour by height (NE2_HR_LC was not found)"
    color = np.concatenate([rgb, np.full(rgb.shape[:2] + (1,), 255, dtype=np.uint8)], axis=-1)
    assert height.shape == (OUT_H, OUT_W) and color.shape == (OUT_H, OUT_W, 4), (height.shape, color.shape)
    size = container.write(out_file, 5, 2, {"size": (np.array([OUT_W, OUT_H]), "<u4"), "height": (height.reshape(-1), "<u2"), "color": (color.reshape(-1), "u1")})
    print(f"{out_file}: {size:,} bytes; heights up to {int(height.max())} m; land colour from {painted}")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
