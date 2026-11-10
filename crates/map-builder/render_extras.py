"""Extra layers of the render file (client only): shaded relief, rivers and place names.

All coordinates are Equal Earth metres (EPSG:8857), like the rest of the render file.
Deterministic: no random numbers, fixed sampling, sorted or file-order iteration.
"""
import json
from pathlib import Path

import numpy as np
import pyproj
import shapefile
import shapely
from scipy import ndimage
from shapely.geometry import shape

RELIEF_W = 8192                       # pixels across the map (about 4.2 km each)
_EQ_TO_LL = pyproj.Transformer.from_crs("EPSG:8857", "EPSG:4326", always_xy=True)
_LL_TO_EQ = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:8857", always_xy=True)


def _raster_grid(extent):
    """Pixel grid of the display rasters (relief and land colours): (px, x0, y0, y1, H, xs)."""
    x_min, x_max, y_min, y_max = extent
    px = (x_max - x_min + 600_000) / RELIEF_W
    x0 = np.floor((x_min - 300_000) / px) * px
    y0 = np.floor((y_min - 300_000) / px) * px
    y1 = np.ceil((y_max + 300_000) / px) * px
    H = int(round((y1 - y0) / px))
    xs = x0 + (np.arange(RELIEF_W) + 0.5) * px
    return px, x0, y0, y1, H, xs


def _sample_rows(xs, ys):
    """For one band of pixel rows: (ok, row, col) indexes into a global 1-arc-minute grid
    (10800 x 21600, row 0 at 90 N); `ok` is False outside the projection's outline."""
    X, Y = np.meshgrid(xs, ys)
    lon, lat = _EQ_TO_LL.transform(X.ravel(), Y.ravel())
    lon = np.asarray(lon).reshape(X.shape); lat = np.asarray(lat).reshape(X.shape)
    ok = np.isfinite(lon) & np.isfinite(lat) & (np.abs(lon) <= 180.0) & (np.abs(lat) <= 90.0)
    row = np.clip(np.floor((90.0 - np.where(ok, lat, 0.0)) * 60.0).astype(np.int64), 0, 10799)
    col = np.clip(np.floor((np.where(ok, lon, 0.0) + 180.0) * 60.0).astype(np.int64), 0, 21599)
    return ok, row, col


def relief(inter, extent):
    """Hillshade as a u8 raster in the Equal Earth plane. Byte 128 means "no change"; the client
    multiplies the province colour by byte/128. `extent` = (x_min, x_max, y_min, y_max) of the data.
    Returns (pixels, info) with info = width, height, x0, y0, x1, y1 (metres; row 0 is the top edge y1)."""
    elev = np.load(Path(inter) / "elev_60s_int16.npy", mmap_mode="r")
    px, x0, y0, y1, H, xs = _raster_grid(extent)
    Z = np.zeros((H, RELIEF_W), dtype=np.float32)
    step = 256
    for r0 in range(0, H, step):
        rows = np.arange(r0, min(H, r0 + step))
        ok, row, col = _sample_rows(xs, y1 - (rows + 0.5) * px)
        v = np.maximum(np.asarray(elev)[row, col].astype(np.float32), 0.0)
        Z[rows] = np.where(ok, v, 0.0)
    Z = ndimage.gaussian_filter(Z, 1.0)
    # the plane is equal-area, so a pixel is about px metres on the ground (shape distortion is small)
    dzdx = np.zeros_like(Z); dzdy = np.zeros_like(Z)
    dzdx[:, 1:-1] = (Z[:, 2:] - Z[:, :-2]) / (2 * px)
    dzdy[1:-1, :] = -(Z[2:, :] - Z[:-2, :]) / (2 * px)   # north is up, rows go down
    ez = 6.0
    gx, gy = ez * dzdx, ez * dzdy
    norm = np.sqrt(gx * gx + gy * gy + 1.0)
    az, alt = np.radians(315.0), np.radians(45.0)
    lx, ly, lz = np.sin(az) * np.cos(alt), np.cos(az) * np.cos(alt), np.sin(alt)
    shade = (-gx * lx - gy * ly + lz) / norm / lz        # 1.0 on flat ground
    mult = np.clip(shade, 0.35, 1.6) * (1.0 + 0.12 * np.minimum(Z / 3000.0, 1.0))
    out = np.clip(np.rint(mult * 128.0), 0, 255).astype(np.uint8)
    out[Z <= 0] = 128
    info = np.array([RELIEF_W, H, int(round(x0)), int(round(y0)), int(round(x0 + RELIEF_W * px)), int(round(y1))], dtype="<i4")
    return out.reshape(-1), info


def heights(inter, extent):
    """Elevation in metres (i16, sea floor negative) on the relief grid, lightly smoothed: the
    client lifts its 3D terrain from it and colours water by depth."""
    elev = np.load(Path(inter) / "elev_60s_int16.npy", mmap_mode="r")
    px, x0, y0, y1, H, xs = _raster_grid(extent)
    Z = np.zeros((H, RELIEF_W), dtype=np.float32)
    for r0 in range(0, H, 256):
        rows = np.arange(r0, min(H, r0 + 256))
        ok, row, col = _sample_rows(xs, y1 - (rows + 0.5) * px)
        v = np.asarray(elev)[row, col].astype(np.float32)
        Z[rows] = np.where(ok, v, -4000.0)
    Z = ndimage.gaussian_filter(Z, 0.7)
    return np.clip(np.rint(Z), -32768, 32767).astype("<i2").reshape(-1)


def land_colors(ne_dir, extent):
    """Natural Earth II land cover colours (forest, grassland, desert, ice ...) resampled onto the
    same pixel grid as the relief: RGB bytes, row by row. Each pixel averages a 2 x 2 block of the
    1-arc-minute source to soften aliasing."""
    import tifffile
    src = tifffile.imread(str(Path(ne_dir) / "NE2_HR_LC" / "NE2_HR_LC.tif"))   # (10800, 21600, 3) u8
    px, x0, y0, y1, H, xs = _raster_grid(extent)
    out = np.zeros((H, RELIEF_W, 3), dtype=np.uint8)
    step = 256
    for r0 in range(0, H, step):
        rows = np.arange(r0, min(H, r0 + step))
        ok, row, col = _sample_rows(xs, y1 - (rows + 0.5) * px)
        r1 = np.minimum(row + 1, 10799); c1 = (col + 1) % 21600
        acc = (src[row, col].astype(np.uint16) + src[r1, col] + src[row, c1] + src[r1, c1] + 2) // 4
        out[rows] = np.where(ok[..., None], acc, 0).astype(np.uint8)
    return out.reshape(-1)


def rivers(ne_dir):
    """NE river centrelines as polylines: (x, y, off, rank)."""
    path = Path(ne_dir) / "ne_10m_rivers_lake_centerlines" / "ne_10m_rivers_lake_centerlines"
    r = shapefile.Reader(str(path))
    names = [f[0] for f in r.fields[1:]]
    xs, ys, off, rank = [], [], [0], []
    for sr in r.shapeRecords():
        rec = dict(zip(names, sr.record))
        pts = np.array(sr.shape.points, dtype=np.float64)
        parts = list(sr.shape.parts) + [len(pts)]
        for a, b in zip(parts[:-1], parts[1:]):
            seg = pts[a:b]
            if len(seg) < 2:
                continue
            seg = seg[np.abs(seg[:, 1]) < 84.0] if (np.abs(seg[:, 1]) >= 84.0).any() else seg
            if len(seg) < 2:
                continue
            mx, my = _LL_TO_EQ.transform(seg[:, 0], seg[:, 1])
            xs.append(np.rint(mx).astype(np.int32)); ys.append(np.rint(my).astype(np.int32))
            off.append(off[-1] + len(seg)); rank.append(min(int(rec["scalerank"]), 255))
    return np.concatenate(xs), np.concatenate(ys), np.array(off, dtype=np.uint32), np.array(rank, dtype=np.uint8)


def _label_point(geom):
    """A point inside the largest part of a lon/lat geometry."""
    parts = shapely.get_parts(geom)
    best = max(parts, key=lambda p: (p.area, p.bounds))
    return best.representative_point()


def _pack(names, pts, areas, prefix, A):
    blobs = [n.encode("utf-8") for n in names]
    off = np.zeros(len(blobs) + 1, dtype=np.uint32)
    np.cumsum([len(b) for b in blobs], out=off[1:])
    A[f"{prefix}_name_off"] = off
    A[f"{prefix}_name_bytes"] = np.frombuffer(b"".join(blobs), dtype=np.uint8) if blobs else np.zeros(0, dtype=np.uint8)
    mx, my = _LL_TO_EQ.transform([p[0] for p in pts], [p[1] for p in pts])
    A[f"{prefix}_x"] = np.rint(mx).astype(np.int32); A[f"{prefix}_y"] = np.rint(my).astype(np.int32)
    A[f"{prefix}_area_km2"] = np.minimum(np.rint(areas), 4e9).astype("<u4")


def labels(inter, A):
    """Country names (state order = the map's country ids) and region names, each with a point."""
    inter = Path(inter)
    states = json.loads((inter / "states_1990.geojson").read_text(encoding="utf-8"))["features"]
    regions = json.loads((inter / "regions_1990.geojson").read_text(encoding="utf-8"))["features"]
    state_area = {}
    for f in regions:
        state_area[f["properties"]["tag"]] = state_area.get(f["properties"]["tag"], 0.0) + f["properties"]["area_km2"]
    names, pts, areas = [], [], []
    for f in sorted(states, key=lambda f: f["properties"]["tag"]):
        p = _label_point(shape(f["geometry"]))
        names.append(f["properties"]["name"]); pts.append((p.x, p.y)); areas.append(state_area.get(f["properties"]["tag"], 0.0))
    _pack(names, pts, areas, "country", A)
    names, pts, areas = [], [], []
    for f in sorted(regions, key=lambda f: f["properties"]["region"]):
        p = _label_point(shape(f["geometry"]))
        names.append(f["properties"]["name"]); pts.append((p.x, p.y)); areas.append(f["properties"]["area_km2"])
    _pack(names, pts, areas, "region", A)


def _read(ne_dir, name):
    path = Path(ne_dir) / name / name
    r = shapefile.Reader(str(path))
    fields = [f[0] for f in r.fields[1:]]
    return [(dict(zip(fields, sr.record)), sr.shape) for sr in r.shapeRecords()]


def _lines(items, tolerance_m):
    """items: (class, shape) in file order. Returns (x, y, off, cls) sorted by class (stable),
    each line projected to Equal Earth and simplified."""
    out = []
    for k, (cls, shp) in enumerate(items):
        pts = np.array(shp.points, dtype=np.float64)
        if len(pts) < 2:
            continue
        parts = list(shp.parts) + [len(pts)]
        for a, b in zip(parts[:-1], parts[1:]):
            seg = pts[a:b]
            seg = seg[np.abs(seg[:, 1]) < 89.0]
            if len(seg) < 2:
                continue
            ex, ey = _LL_TO_EQ.transform(seg[:, 0], seg[:, 1])
            line = shapely.simplify(shapely.LineString(np.column_stack([ex, ey])), tolerance_m)
            c = np.rint(shapely.get_coordinates(line)).astype(np.int32)
            # drop repeated points after rounding
            keep = np.r_[True, (np.diff(c, axis=0) != 0).any(axis=1)]
            c = c[keep]
            if len(c) >= 2:
                out.append((cls, k, c))
    out.sort(key=lambda t: (t[0], t[1]))
    xs = [c[:, 0] for _, _, c in out]; ys = [c[:, 1] for _, _, c in out]
    off = np.zeros(len(out) + 1, dtype=np.uint32)
    np.cumsum([len(c) for _, _, c in out], out=off[1:])
    return np.concatenate(xs), np.concatenate(ys), off, np.array([t[0] for t in out], dtype=np.uint8)


ROAD_CLASS = {"Major Highway": 0, "Beltway": 0, "Bypass": 0, "Secondary Highway": 1, "Road": 2, "Unknown": 2, "Track": 2}


def roads(ne_dir):
    """Natural Earth roads: class 0 major highway, 1 secondary, 2 other. Ferries are left out."""
    items = [(ROAD_CLASS.get(rec["type"], 2), shp) for rec, shp in _read(ne_dir, "ne_10m_roads") if rec["featurecla"] == "Road"]
    return _lines(items, 300.0)


def rails(ne_dir):
    """Natural Earth railways: class 0 main lines (scalerank <= 5), 1 regional (<= 8), 2 other."""
    items = []
    for rec, shp in _read(ne_dir, "ne_10m_railroads"):
        if rec["featurecla"] != "Railroad":
            continue
        sr = int(rec["scalerank"] or 10)
        items.append((0 if sr <= 5 else 1 if sr <= 8 else 2, shp))
    return _lines(items, 300.0)


def urban(ne_dir):
    """Natural Earth urban areas as one triangle mesh: (vx, vy, tri)."""
    polys = []
    for rec, shp in _read(ne_dir, "ne_10m_urban_areas"):
        g = shape(shp.__geo_interface__)
        if not g.is_valid:
            g = shapely.make_valid(g)
        g = shapely.transform(g, lambda c: np.column_stack(_LL_TO_EQ.transform(c[:, 0], c[:, 1])))
        g = shapely.simplify(g, 150.0)
        for p in shapely.get_parts(g):
            if p.geom_type == "Polygon" and not p.is_empty and p.area > 0:
                polys.append(p)
    tris = shapely.constrained_delaunay_triangles(np.array(polys, dtype=object))
    parts = shapely.get_parts(tris)
    c = np.rint(shapely.get_coordinates(parts).reshape(-1, 4, 2)[:, :3, :]).astype(np.int64).reshape(-1, 2)
    uniq, inverse = np.unique(c, axis=0, return_inverse=True)
    return uniq[:, 0].astype(np.int32), uniq[:, 1].astype(np.int32), inverse.astype(np.uint32).reshape(-1)
