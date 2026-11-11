# Data formats

## Scenario file (RON, version 1)

Authored text, parsed once by the loader. See `data/scenarios/sample.ron`.

```
(
    version: 1,
    name: "...", seed: <u64>,
    countries: [ (id, name) ],
    provinces: [ (id, name, controller) ],
    units:     [ (id, owner, location) ],          // optional
    sites:     [ (id, name, kind, province, owner, condition, built_year, destroyed_year) ],  // optional
)
```

- `kind` is one of City, Port, Airfield, Crossing, Industry, Base.
- `condition` is 0 to 100. `destroyed_year` is `None` or `Some(year)` and must not be before `built_year`.
- IDs in each table are unique and run 0..n-1 with no gaps.
- Unknown fields are errors.

## Baked container (version 1)

All integers little-endian.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | magic `MRDN` |
| 4 | 4 | format version (u32) |
| 8 | 4 | kind (u32; 1 = scenario) |
| 12 | 8 | payload length (u64) |
| 20 | 8 | checksum (u64, FNV-1a 64 of the payload) |
| 28 | n | payload (postcard for scenarios; rkyv for the Phase 3 map) |

## Baked map container (version 2)

Written by `crates/map-builder/bake_map.py`, read by `meridian_data::load_map_file`. All integers little-endian.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | magic `MRMP` |
| 4 | 4 | format version (u32) |
| 8 | 4 | kind (u32; 2 = map) |
| 12 | 8 | payload length (u64) |
| 20 | 8 | checksum (u64: CRC-32 of the payload, zero-extended) |
| 28 | n | payload |

Payload: `u32` array count, then for each array `name_len u8 | name | dtype u8 | count u64 | data`.
dtype: 1 = u8, 2 = u16, 3 = u32, 4 = i16, 5 = i32, 6 = u64. Text tables are two arrays, `<name>_blob` (UTF-8 bytes) and `<name>_off` (u32 offsets, length + 1 entries).

Units: coordinates are Equal Earth (EPSG:8857) metres as i32; areas are hectares; distances are metres. Version 2 (map rewrite M1, 2026-11-10): the cells are H3 level-6 hexagons on the globe. A place on the globe is three i32 (whole metres from the Earth's centre, on a sphere of radius 6,371,009 m; `meridian_core::sphere`). `c_dist_m` and `g_cost_m` are distances over the ground between those places plus 2 m, so they are never shorter than the straight line through the Earth, which is what the route search is guided by. `cell_area_ha` is the hexagon's share of its region's true land area (the hexagons of a region add up to the region), not the hexagon's own size. The Equal Earth x and y stay in the file for the flat views. A version 1 file is refused; rebuild the map.

| Group | Arrays |
| --- | --- |
| States | `state_tags`, `state_names` (text tables; state index = country ID in a map world) |
| Provinces | `province_state`, `province_x`, `province_y`, `province_area_ha`, `province_pop`, `province_cell_off`, `province_cells` (cells grouped by province, ascending) |
| Sea zones | `zone_x`, `zone_y`, `zone_area_ha`, `zone_body` |
| Strategic graph (provinces 0..P then zones P..P+Z, CSR, both directions, sorted) | `g_off`, `g_dst`, `g_kind` (0 land, 1 coast, 2 strait or canal, 3 sea), `g_weight` (shared border m for land, coastal cell count for coast), `g_strait` (chokepoint index + 1, or 0), `g_cost_m`, `node_xyz` (3 i32 per node: a province's is the area-weighted middle of its hexagons, put on the surface) |
| Chokepoints | `choke_names`, `choke_built` (year, 0 if ancient) |
| Cells | `cell_h3` (u64, the hexagon's H3 number: its permanent name, never interpreted by the engine), `cell_xyz` (3 i32 per cell), `cell_state`, `cell_province`, `cell_x`, `cell_y`, `cell_area_ha`, `cell_landmass`, `cell_elev_m`, `cell_relief_m`, `cell_terrain` (0 plains, 1 hills, 2 mountains, 3 high mountains, 4 desert, 5 ice), `cell_urban` (0 to 255 built-up share), `cell_flags` (1 road, 2 rail, 4 river, 8 coast), `cell_zone` (nearest sea zone or 0xFFFFFFFF), `cell_pop` |
| Cell graph (CSR, both directions, sorted) | `c_off`, `c_dst`, `c_len_m` (shared border), `c_flags` (1 a river runs along the shared side, so going from one cell to the other crosses it; 2 a road passes the side; 4 a railway passes it; river with road or railway = a permanent bridge of 1990), `c_dist_m` (centre to centre over the ground, plus 2 m) |
| Sites | `site_kind` (0 city, 1 port, 2 airfield, 3 crossing, 4 industry, 5 base), `site_cell`, `site_province`, `site_size` (city population estimate), `site_built`, `site_destroyed` (32767 = never), `site_verified` (0 = assumed present in 1990, from modern data), `site_names` |

The loader checks the checksum, every array's length, value ranges, that cells form an exact partition into provinces, that both graphs are sorted and symmetric (and edge kinds agree in both directions), and for the globe that every place is on the sphere, no hexagon number is used twice and no edge is shorter than its straight line. `Map::validate_world()` then checks the world rules.

## Save file (version 3)

Written by `meridian_data::save_world`, read by `load_world`. Header is little-endian, same shape as the other containers.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | magic `MRSV` |
| 4 | 4 | format version (u32) |
| 8 | 4 | kind (u32; 3 = save) |
| 12 | 8 | payload length (u64) |
| 20 | 8 | checksum (u64: CRC-32 of the payload, zero-extended) |
| 28 | n | payload |

Payload: postcard encoding (serde) of `SaveBody { map_checksum: Option<u32>, state_hash: u64, changes: Option<CellChanges>, world: World }`.

- `world` is every field of `World` except the attached map and the per-cell arrays. Field order is the declaration order in `meridian-core` (postcard has no field names), so reordering or adding fields to a serialized type is a format change: bump `SAVE_VERSION` and add a step to `migrate`. Version 2 (2026-11-10) added `march` to each unit's extra attributes; version 1 saves cannot be opened (there is no migration step, as no version 1 saves were ever released). Version 3 (the same day) added the unit type, size and name; version 2 saves cannot be opened either.
- `changes` holds only the cells whose state differs from a fresh world on the same map (ascending by cell) plus the list of opened provinces.
- The map is never inside a save. `map_checksum` must match the map given to `load_world`.
- After decoding, `load_world` recomputes the state hash and refuses the file if it differs from `state_hash`.
- Derived caches (zones of control, front segments, units by cell) are not saved; callers rebuild them after loading.
- Versions: a newer version than the build knows fails with "written by a newer build"; an older one than `OLDEST_MIGRATABLE` fails with "older than the oldest this build can migrate". Version 1 is the first, so there are no migration steps yet.

## Replay file (version 1)

Written by `meridian_data::replay::write_replay`. Same 28-byte header with magic `MRRP`, kind 4, CRC-32 checksum.

Payload: postcard of `Replay { scenario: String, seed: u64, map_checksum: Option<u32>, ticks: u64, commands: Vec<Entry>, checkpoints: Vec<(tick, hash)>, final_hash: u64 }`.

- `Entry` is `{ tick, source (Player, Ai, Headquarters), command }`, in the order the commands were applied.
- AI and headquarters commands are logged like player commands, so playing a replay never runs the AI.
- `checkpoints` hold the state hash before the stated tick runs; the player stops at the first mismatch.

## Render file (version 7)

Written by `crates/map-builder/build_render.py` (checked by `validate_render.py`), read by `meridian_data::load_render_file`. **Client only; the engine never reads it.** Same header and array directory as the baked map, with magic `MRRD` and kind 4 (CRC-32 checksum). **Coordinates are Equal Earth metres (EPSG:8857) as i32, the same equal-area plane the simulation uses, so the display has no stretch toward the poles.** Versions 1 to 3 (1: Equal Earth; 2 and 3: Web Mercator) are no longer read.

| Group | Arrays |
| --- | --- |
| Levels | `levels_tolerance_m` (u32, coarsest first; default 8000, 2500, 800 m, measured in Equal Earth metres before projection) |
| Per level `k` | `l{k}_vx`, `l{k}_vy` (i32), `l{k}_vprov` (u32 province per vertex), `l{k}_tri` (u32, 3 vertex indexes per triangle; a triangle never spans two provinces) |
| Border lines per level | `l{k}_line_x`, `l{k}_line_y` (i32 points), `l{k}_line_off` (u32, lines + 1 entries), `l{k}_line_kind` (u8: 0 province border, 1 state border, 2 coast) |
| Cells | `cell_seg` (i32, four values per distinct cell edge: x0, y0, x1, y1), `cell_mx`, `cell_my` (i32 centres), `cell_ring_off` (u32, cells + 1 entries), `cell_ring_x`, `cell_ring_y` (i32, each cell's outer ring without the repeated closing point; used for picking and highlighting) |
| Provinces | `province_mx`, `province_my` (i32, area-weighted centres), `province_ref_area_km2` (u32, unsimplified Mercator area, for the validator) |

- Provinces are the union of their cells, simplified as one coverage per level (neighbours keep fitting) in Equal Earth, rounded to whole metres vertex by vertex and repaired where rounding made a polygon invalid. Small islands may vanish at the coarse levels; the finest level keeps every province within 2 % (or 100 km2) of its true projected area.
- Mercator inflates areas away from the equator (about 4x at 60 degrees), so the file's areas are not real areas.
- Lines come from noding all province boundaries; a piece touching one province is coast, two provinces of one state a province border (province borders now include real admin-1 lines), two states a state border.
- Not yet in the file: cell fill meshes, rivers, contours, names and label positions, sea zones.
- Since version 7: `height_m` (i16, metres on the relief grid, sea floor negative; from ETOPO 2022 (60 arc-second surface elevation, land and sea floor; the source the terrain step already uses), a light Gaussian blur (sigma 0.7 px), -4,000 m outside the world outline). The client raises the 3D terrain from it and shades the sea by depth.
- Since version 6: `land_rgb` (u8, 3 bytes per pixel, Natural Earth II land cover colours on the relief grid).
- Since version 5: `road_x`, `road_y`, `road_off`, `road_class` (0 major highway, 1 secondary, 2 other) and `rail_x`, `rail_y`, `rail_off`, `rail_class` (0 main, 1 regional, 2 other), each sorted by class; `urban_vx`, `urban_vy`, `urban_tri` (one triangle mesh of built-up areas).
- Since version 3: `relief` (u8 hillshade raster, 8192 wide, Equal Earth plane, row 0 = top; byte 128 = no change, client multiplies colour by byte/128) with `relief_info` (i32: width, height, x0, y0, x1, y1 metres); rivers `river_x`, `river_y` (i32), `river_off` (u32), `river_rank` (u8, Natural Earth scalerank, lower = bigger); place names `country_*` (one per country in map order) and `region_*` (one per region): `_name_off` (u32), `_name_bytes` (u8 UTF-8), `_x`, `_y` (i32 Equal Earth label point inside the largest part), `_area_km2` (u32).
- Size: about 353 MB (heights 61 MB, land colours 92 MB, relief 30 MB, roads and railways about 6 MB, urban areas 3 MB). Earlier: about 191 MB for the whole world (cell outlines 34 MB and cell rings 32 MB of it).

## Globe lines file (version 1)

`data/baked/meridian-globe-lines.bin`, written by `crates/map-builder/build_globe_lines.py`, read by `meridian_data::load_globe_lines_file`. Drawing only: the engine never reads it, and the client runs without it. The same container as the baked map (magic `MRMP`, version, kind, length, CRC-32), with kind 4.

| Array | Type | Meaning |
| --- | --- | --- |
| `seg_a`, `seg_b` | i32, three per piece | the two ends of each short straight piece, whole metres from the Earth's centre |
| `seg_style` | u8 per piece | 0 coast, 1 border of 1990, 2 great river, 3 river, 4 major highway, 5 highway or road, 6 minor road, 7 railway |
| `bucket_off` | u32, buckets + 1 | bucket `k` is pieces `bucket_off[k]..bucket_off[k+1]` |
| `bucket_tier` | u8 per bucket | detail tier: 0 shown from the whole globe, 1 from a continent, 2 from a country, 3 from a region |
| `bucket_xyz` | i32, three per bucket | the middle of the bucket's level-2 hexagon |

Pieces are sorted by tier, then bucket hexagon, then style. No piece is longer than a quarter of a degree. Size on the world: 3.07 million pieces, 77 MB.

A regional pack's lines file (`data/baked/packs/<name>.lines.bin`) is the same format with three more arrays: `info` (u32: how far a bucket reaches from its middle, in millionths of a radian), and the coverage outline `cover` (i32 longitude, latitude pairs in millionths of a degree) with `cover_off` (u32, ring starts, rings + 1 entries). Style 8 is a local road; tier 4 shows only close up. Inside the outline the client hides every world line except the 1990 borders.

## Globe ground file (version 2)

`data/baked/meridian-globe-ground.bin`, written by `build_globe_ground.py`. Container kind 5. Two pictures of the world, 8192 by 4096, rows north to south, left edge 180 W: `size` (u32: width, height), `height` (u16 per pixel, metres above the sea, 0 for the sea, from ETOPO 2022) and `color` (u8, four per pixel: red, green, blue, 255, from Natural Earth II's land cover picture, public domain). 201 MB. Version 1 was a ready-made hillshade (`meridian-globe-relief.bin`), replaced because lighting in the shader can follow the zoom.

## Pack ground cover file (version 2)

`data/baked/packs/<name>.cover.bin`, written by `build_pack.py`. Container kind 6.

| Array | Type | Meaning |
| --- | --- | --- |
| `grid` | i32, 9 values | west edge and north edge (millionths of a degree), pixel width and height (billionths of a degree), picture width and height (pixels), tile size (pixels), tiles across, tiles down |
| `tile_layer` | u32 per tile, row by row from the north-west | which stored tile it is, or 0xFFFFFFFF when the tile is all sea |
| `tiles` | u8 | the stored tiles one after another, tile * tile bytes each, rows north to south: 0 sea, 1 inland water, 2 woods, 3 built-up, 4 industry, 6 other land, 7 land outside the pack's coverage |

A pixel is 0.001 by 0.00075 degrees (about 79 by 83 m at 45 N). Drawing only.

Version 2 (2026-11-11): land and sea come from OpenStreetMap's coastline (the land polygons of osmdata.openstreetmap.de), so 0 now means sea and the client cuts the hexagons' colour off at the real coast.
