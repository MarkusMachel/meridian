"""Step 5 of the map builder: terrain, urban density, roads, rails, rivers per cell.

Usage: python -I build_terrain.py <intermediate-dir> <natural-earth-dir> <etopo.tif>
Writes cells/: cell_elev_m.npy (i16)  cell_relief_m.npy (u16)  cell_terrain.npy (u8)
               cell_urban.npy (u8, 0-255 = share of the cell built up)
               cell_flags.npy (u8: 1 road, 2 rail, 4 river, 8 coast)
               cell_edge_flags.npy (u8 per cell edge, for what passes the shared border of the
               two cells: 1 a river, 2 a road, 4 a railway; river + road or railway = a bridge)
Terrain classes: 0 plains, 1 hills, 2 mountains, 3 high mountains, 4 desert, 5 ice.

Honest limits: elevation is ETOPO 2022 at 1.85 km (9 samples per cell); desert comes from
Natural Earth's named desert regions, ice from its glaciated areas; there is no forest or
jungle layer yet. Roads, rails and urban areas are Natural Earth's (about 2010s), not 1990.
"""
import sys
from pathlib import Path

import numpy as np
import pyproj
import shapefile
import shapely
import tifffile
from shapely.geometry import shape

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_provinces as bp  # noqa: E402
from validate_cells import read_wkb  # noqa: E402

INV = pyproj.Transformer.from_crs("EPSG:8857", "EPSG:4326", always_xy=True)
PLAIN, HILLS, MOUNTAINS, HIGH, DESERT, ICE = range(6)
ROAD, RAIL, RIVER, COAST = 1, 2, 4, 8
# bits on cell edges (cell_edge_flag in meridian-core): what passes the shared border of two cells
EDGE_RIVER, EDGE_ROAD, EDGE_RAIL = 1, 2, 4


def edges_passed(edges, polys, lines, line_idx, cell_idx):
    """Which cell edges does one of `lines` pass? An edge counts when the same line touches both
    of its cells and also the strip where the two cells meet (their shared border, 5 m wide).
    `line_idx`, `cell_idx` are the (line, cell) pairs that intersect. Returns a boolean array,
    one per edge."""
    out = np.zeros(len(edges), dtype=bool)
    by_cell = {}
    for l, cc in zip(np.asarray(line_idx).tolist(), np.asarray(cell_idx).tolist()):
        by_cell.setdefault(cc, set()).add(l)
    hit_a, hit_b, hit_l, hit_e = [], [], [], []
    for e in range(len(edges)):
        a, b = int(edges[e, 0]), int(edges[e, 1])
        la, lb = by_cell.get(a), by_cell.get(b)
        if not la or not lb:
            continue
        for l in sorted(la & lb):
            hit_a.append(a); hit_b.append(b); hit_l.append(l); hit_e.append(e)
    if hit_a:
        sliver = shapely.intersection(shapely.buffer(polys[hit_a], 5.0), shapely.buffer(polys[hit_b], 5.0))
        crosses = shapely.intersects(sliver, lines[hit_l])
        out[np.asarray(hit_e)[crosses]] = True
    return out


def load_layer(path, keep=None):
    r = shapefile.Reader(str(path))
    names = [f[0] for f in r.fields[1:]]
    geoms = []
    for sr in r.shapeRecords():
        rec = dict(zip(names, sr.record))
        if keep and not keep(rec):
            continue
        g = shape(sr.shape.__geo_interface__)
        if not g.is_valid:
            g = shapely.make_valid(g)
        geoms.append(g)
    return geoms


def project_all(geoms):
    return [bp.project(g) for g in geoms]


def overlap_fraction(polys, area, tree, layer):
    """Share (0-1) of each cell covered by the layer's polygons."""
    layer = np.array(layer, dtype=object)
    li, ci = tree.query(layer, predicate="intersects")
    inter = shapely.area(shapely.intersection(polys[ci], layer[li]))
    cov = np.bincount(ci, weights=inter, minlength=len(polys))
    return np.clip(cov / np.maximum(area, 1.0), 0.0, 1.0)


def main(inter, ne_dir, tif):
    inter, ne_dir = Path(inter), Path(ne_dir)
    c = inter / "cells"
    polys = read_wkb(c / "cells.wkb")
    n = len(polys)
    area = shapely.area(polys)
    cx = np.load(c / "cell_x.npy").astype(np.float64); cy = np.load(c / "cell_y.npy").astype(np.float64)
    edges = np.load(c / "cell_edges.npy")
    cz = np.load(inter / "sea" / "cell_zone.npy")

    # ---- elevation: 9 samples per cell ----
    cache = inter / "elev_60s_int16.npy"
    if not cache.exists():  # decode the compressed GeoTIFF once per intermediate directory
        np.save(cache, np.rint(tifffile.imread(tif)).astype(np.int16))
    elev = np.load(cache, mmap_mode="r")
    r = np.minimum(0.5 * np.sqrt(area / np.pi), 8000.0)
    ang = np.arange(8) * (np.pi / 4)
    sx = np.concatenate([cx[:, None], cx[:, None] + r[:, None] * np.cos(ang)], axis=1)
    sy = np.concatenate([cy[:, None], cy[:, None] + r[:, None] * np.sin(ang)], axis=1)
    inside = shapely.contains_xy(polys[:, None], sx, sy)
    inside[:, 0] |= ~inside.any(axis=1)       # always keep the centroid as a fallback
    lon, lat = INV.transform(sx.ravel(), sy.ravel())
    lon = np.asarray(lon).reshape(sx.shape); lat = np.asarray(lat).reshape(sx.shape)
    row = np.clip(np.floor((90.0 - lat) * 60.0).astype(np.int64), 0, 10799)
    col = np.clip(np.floor((lon + 180.0) * 60.0).astype(np.int64), 0, 21599)
    v = np.maximum(np.asarray(elev)[row, col].astype(np.float64), 0.0)   # land cells: no negative heights
    cnt = inside.sum(axis=1)
    mean = (v * inside).sum(axis=1) / cnt
    vmax = np.where(inside, v, -1).max(axis=1); vmin = np.where(inside, v, 1e9).min(axis=1)
    relief = vmax - vmin
    cell_elev = np.rint(mean).astype(np.int16); cell_relief = np.minimum(np.rint(relief), 65535).astype(np.uint16)

    terrain = np.full(n, PLAIN, dtype=np.uint8)
    terrain[(relief >= 100) | (mean >= 1000)] = HILLS
    terrain[(relief >= 300) | (mean >= 2000)] = MOUNTAINS
    terrain[(relief >= 700) | (mean >= 3500)] = HIGH

    tree = shapely.STRtree(polys)
    # ---- ice, desert ----
    ice = overlap_fraction(polys, area, tree, project_all(load_layer(ne_dir / "ne_10m_glaciated_areas" / "ne_10m_glaciated_areas")))
    desert = overlap_fraction(polys, area, tree, project_all(load_layer(
        ne_dir / "ne_10m_geography_regions_polys" / "ne_10m_geography_regions_polys", keep=lambda rec: rec["FEATURECLA"] == "Desert")))
    plainish = terrain <= HILLS
    terrain[(desert >= 0.5) & plainish] = DESERT
    terrain[ice >= 0.5] = ICE

    # ---- urban density ----
    urban = overlap_fraction(polys, area, tree, project_all(load_layer(ne_dir / "ne_10m_urban_areas" / "ne_10m_urban_areas")))
    cell_urban = np.rint(urban * 255).astype(np.uint8)

    # ---- roads, rails, rivers ----
    flags = np.zeros(n, dtype=np.uint8)
    edge_flags = np.zeros(len(edges), dtype=np.uint8)
    for name, bit, edge_bit, keep in [("ne_10m_roads", ROAD, EDGE_ROAD, lambda rec: str(rec.get("type", "")) != "Ferry"),
                                      ("ne_10m_railroads", RAIL, EDGE_RAIL, None)]:
        lines = np.array(project_all(load_layer(ne_dir / name / name, keep=keep)), dtype=object)
        li, ci = tree.query(lines, predicate="intersects")
        flags[np.unique(ci)] |= bit
        edge_flags[edges_passed(edges, polys, lines, li, ci)] |= edge_bit
    rivers = np.array(project_all(load_layer(
        ne_dir / "ne_10m_rivers_lake_centerlines" / "ne_10m_rivers_lake_centerlines",
        keep=lambda rec: rec["featurecla"] in ("River", "Canal"))), dtype=object)
    rl, rc = tree.query(rivers, predicate="intersects")
    flags[np.unique(rc)] |= RIVER
    flags[cz != 0xFFFFFFFF] |= COAST
    edge_flags[edges_passed(edges, polys, rivers, rl, rc)] |= EDGE_RIVER
    bridges = (edge_flags & EDGE_RIVER != 0) & (edge_flags & (EDGE_ROAD | EDGE_RAIL) != 0)

    np.save(c / "cell_elev_m.npy", cell_elev); np.save(c / "cell_relief_m.npy", cell_relief)
    np.save(c / "cell_terrain.npy", terrain); np.save(c / "cell_urban.npy", cell_urban)
    np.save(c / "cell_flags.npy", flags); np.save(c / "cell_edge_flags.npy", edge_flags)
    names = ["plains", "hills", "mountains", "high mountains", "desert", "ice"]
    share = np.bincount(terrain, weights=area, minlength=6) / area.sum()
    print("terrain by area: " + ", ".join(f"{nm} {s:.1%}" for nm, s in zip(names, share)))
    print(f"elevation mean {cell_elev.mean():.0f} m, max {cell_elev.max()} m; urban cells (>10%): {(urban > 0.1).sum()}; "
          f"road {(flags&1>0).mean():.1%}, rail {(flags&2>0).mean():.1%}, river {(flags&4>0).mean():.1%}, coast {(flags&8>0).mean():.1%}; "
          f"edges: river {(edge_flags & EDGE_RIVER > 0).sum()}, road {(edge_flags & EDGE_ROAD > 0).sum()}, "
          f"rail {(edge_flags & EDGE_RAIL > 0).sum()}, bridges {bridges.sum()}")


if __name__ == "__main__":
    main(*sys.argv[1:4])
