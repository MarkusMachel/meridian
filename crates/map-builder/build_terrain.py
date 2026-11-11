"""Step 5 of the map builder: terrain, urban density, roads, rails, rivers per cell.

Usage: python -I build_terrain.py <intermediate-dir> <natural-earth-dir> <etopo.tif>
Writes cells/: cell_elev_m.npy (i16)  cell_relief_m.npy (u16)  cell_terrain.npy (u8)
               cell_urban.npy (u8, 0-255 = share of the cell built up)
               cell_flags.npy (u8: 1 road, 2 rail, 4 river, 8 coast)
               cell_edge_flags.npy (u8 per cell edge: 1 a river runs along the shared side of the
               two cells, 2 a road passes it, 4 a railway passes it; river + road or railway = a bridge)
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
import build_network  # noqa: E402
import build_rivers  # noqa: E402

INV = pyproj.Transformer.from_crs("EPSG:8857", "EPSG:4326", always_xy=True)
PLAIN, HILLS, MOUNTAINS, HIGH, DESERT, ICE = range(6)
ROAD, RAIL, RIVER, COAST = 1, 2, 4, 8
# bits on cell edges (cell_edge_flag in meridian-core): what passes the shared border of two cells
EDGE_RIVER, EDGE_ROAD, EDGE_RAIL = 1, 2, 4


CHUNK = 200_000   # cells handled at a time; keeps the memory flat however many cells there are


def read_blobs(path):
    """The cell outlines as raw WKB, one entry per cell; decoded a part at a time with outlines()."""
    data = Path(path).read_bytes()
    out, pos = [], 0
    while pos < len(data):
        n = int.from_bytes(data[pos:pos + 4], "little")
        out.append(data[pos + 4:pos + 4 + n]); pos += 4 + n
    return np.array(out, dtype=object)


def outlines(blobs, idx):
    return shapely.from_wkb(blobs[idx])


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


def overlap_area(polys, tree, layer):
    """Area of each cell (of this part) covered by the layer's polygons. A cell wholly inside
    one polygon counts in full without cutting it out, which is most cells of a big desert."""
    cov = np.zeros(len(polys))
    whole_l, whole_c = tree.query(layer, predicate="contains_properly")
    cov[whole_c] = shapely.area(polys[whole_c])
    li, ci = tree.query(layer, predicate="intersects")
    part = ~np.isin(ci, whole_c)
    li, ci = li[part], ci[part]
    np.add.at(cov, ci, shapely.area(shapely.intersection(polys[ci], layer[li])))
    return cov


def main(inter, ne_dir, tif):
    inter, ne_dir = Path(inter), Path(ne_dir)
    c = inter / "cells"
    blobs = read_blobs(c / "cells.wkb")
    n = len(blobs)
    cx = np.load(c / "cell_x.npy").astype(np.float64); cy = np.load(c / "cell_y.npy").astype(np.float64)
    edges = np.load(c / "cell_edges.npy")
    cz = np.load(inter / "sea" / "cell_zone.npy")

    cache = inter / "elev_60s_int16.npy"
    if not cache.exists():  # decode the compressed GeoTIFF once per intermediate directory
        np.save(cache, np.rint(tifffile.imread(tif)).astype(np.int16))
    elev = np.load(cache, mmap_mode="r")

    def layer(name, keep=None):
        return np.array(project_all(load_layer(ne_dir / name / name, keep=keep)), dtype=object)
    ice_l = layer("ne_10m_glaciated_areas")
    desert_l = layer("ne_10m_geography_regions_polys", keep=lambda rec: rec["FEATURECLA"] == "Desert")
    urban_l = layer("ne_10m_urban_areas")
    # Rivers run along hexagon sides (build_rivers.py). Lake centre lines are kept so a river
    # is not broken where it passes through a lake.
    rivers_ll = load_layer(ne_dir / "ne_10m_rivers_lake_centerlines" / "ne_10m_rivers_lake_centerlines", keep=lambda rec: rec["featurecla"] in ("River", "Canal", "Lake Centerline"))
    rivers = np.array(project_all(rivers_ll), dtype=object)

    area = np.zeros(n); mean = np.zeros(n); relief = np.zeros(n)
    ice = np.zeros(n); desert = np.zeros(n); urban = np.zeros(n)
    ang = np.arange(8) * (np.pi / 4)
    for lo in range(0, n, CHUNK):
        k = np.arange(lo, min(lo + CHUNK, n))
        polys = outlines(blobs, k)
        a = shapely.area(polys); area[k] = a
        # ---- elevation: 9 samples per cell ----
        r = np.minimum(0.5 * np.sqrt(a / np.pi), 8000.0)
        sx = np.concatenate([cx[k, None], cx[k, None] + r[:, None] * np.cos(ang)], axis=1)
        sy = np.concatenate([cy[k, None], cy[k, None] + r[:, None] * np.sin(ang)], axis=1)
        inside = shapely.contains_xy(polys[:, None], sx, sy)
        inside[:, 0] |= ~inside.any(axis=1)       # always keep the centroid as a fallback
        lon, lat = INV.transform(sx.ravel(), sy.ravel())
        lon = np.asarray(lon).reshape(sx.shape); lat = np.asarray(lat).reshape(sx.shape)
        row = np.clip(np.floor((90.0 - lat) * 60.0).astype(np.int64), 0, 10799)
        col = np.clip(np.floor((lon + 180.0) * 60.0).astype(np.int64), 0, 21599)
        v = np.maximum(elev[row, col].astype(np.float64), 0.0)   # land cells: no negative heights
        mean[k] = (v * inside).sum(axis=1) / inside.sum(axis=1)
        relief[k] = np.where(inside, v, -1).max(axis=1) - np.where(inside, v, 1e9).min(axis=1)
        # ---- ice, desert, built-up area ----
        tree = shapely.STRtree(polys)
        for cover, lay in [(ice, ice_l), (desert, desert_l), (urban, urban_l)]:
            cover[k] = np.clip(overlap_area(polys, tree, lay) / np.maximum(a, 1.0), 0.0, 1.0)
    cell_elev = np.rint(mean).astype(np.int16); cell_relief = np.minimum(np.rint(relief), 65535).astype(np.uint16)

    terrain = np.full(n, PLAIN, dtype=np.uint8)
    terrain[(relief >= 100) | (mean >= 1000)] = HILLS
    terrain[(relief >= 300) | (mean >= 2000)] = MOUNTAINS
    terrain[(relief >= 700) | (mean >= 3500)] = HIGH
    plainish = terrain <= HILLS
    terrain[(desert >= 0.5) & plainish] = DESERT
    terrain[ice >= 0.5] = ICE
    cell_urban = np.rint(urban * 255).astype(np.uint8)

    # ---- roads, rails, rivers ----
    flags = np.zeros(n, dtype=np.uint8)
    edge_flags = np.zeros(len(edges), dtype=np.uint8)
    # Roads and railways are links between neighbouring hexagons, each with a level (its
    # capacity class): build_network.py. A cell or an edge "has a road" when a link is there.
    cell_h3 = np.load(c / "cell_h3.npy")
    road_level, rail_level = build_network.world_levels(ne_dir, cell_h3, edges)
    np.save(c / "cell_edge_road.npy", road_level); np.save(c / "cell_edge_rail.npy", rail_level)
    for level, bit, edge_bit in [(road_level, ROAD, EDGE_ROAD), (rail_level, RAIL, EDGE_RAIL)]:
        edge_flags[level > 0] |= edge_bit
        flags[np.unique(edges[level > 0, :2])] |= bit
    river_edge = build_rivers.snap_rivers(rivers_ll, rivers, cell_h3, edges, bp.PROJ)
    edge_flags[river_edge] |= EDGE_RIVER
    flags[np.unique(edges[river_edge, :2])] |= RIVER     # a river cell has the river on one of its sides
    flags[cz != 0xFFFFFFFF] |= COAST
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
