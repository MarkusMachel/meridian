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

## Baked map container (version 1)

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
dtype: 1 = u8, 2 = u16, 3 = u32, 4 = i16, 5 = i32. Text tables are two arrays, `<name>_blob` (UTF-8 bytes) and `<name>_off` (u32 offsets, length + 1 entries).

Units: coordinates are Equal Earth (EPSG:8857) metres as i32; areas are hectares; distances are metres. Rounded-up distances (`c_dist_m`, `g_cost_m`) keep the A* heuristic admissible.

| Group | Arrays |
| --- | --- |
| States | `state_tags`, `state_names` (text tables; state index = country ID in a map world) |
| Provinces | `province_state`, `province_x`, `province_y`, `province_area_ha`, `province_pop`, `province_cell_off`, `province_cells` (cells grouped by province, ascending) |
| Sea zones | `zone_x`, `zone_y`, `zone_area_ha`, `zone_body` |
| Strategic graph (provinces 0..P then zones P..P+Z, CSR, both directions, sorted) | `g_off`, `g_dst`, `g_kind` (0 land, 1 coast, 2 strait or canal, 3 sea), `g_weight` (shared border m for land, coastal cell count for coast), `g_strait` (chokepoint index + 1, or 0), `g_cost_m` |
| Chokepoints | `choke_names`, `choke_built` (year, 0 if ancient) |
| Cells | `cell_state`, `cell_province`, `cell_x`, `cell_y`, `cell_area_ha`, `cell_landmass`, `cell_elev_m`, `cell_relief_m`, `cell_terrain` (0 plains, 1 hills, 2 mountains, 3 high mountains, 4 desert, 5 ice), `cell_urban` (0 to 255 built-up share), `cell_flags` (1 road, 2 rail, 4 river, 8 coast), `cell_zone` (nearest sea zone or 0xFFFFFFFF), `cell_pop` |
| Cell graph (CSR, both directions, sorted) | `c_off`, `c_dst`, `c_len_m` (shared border), `c_flags` (1 = river crosses), `c_dist_m` (centre to centre, rounded up) |
| Sites | `site_kind` (0 city, 1 port, 2 airfield, 3 crossing, 4 industry, 5 base), `site_cell`, `site_province`, `site_size` (city population estimate), `site_built`, `site_destroyed` (32767 = never), `site_verified` (0 = assumed present in 1990, from modern data), `site_names` |

The loader checks the checksum, every array's length, value ranges, that cells form an exact partition into provinces, and that both graphs are sorted and symmetric (and edge kinds agree in both directions). `Map::validate_world()` then checks the world rules.

## Save file (version 1)

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

- `world` is every field of `World` except the attached map and the per-cell arrays. Field order is the declaration order in `meridian-core` (postcard has no field names), so reordering or adding fields to a serialized type is a format change: bump `SAVE_VERSION` and add a step to `migrate`.
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

## Render file (version 2)

Written by `crates/map-builder/build_render.py` (checked by `validate_render.py`), read by `meridian_data::load_render_file`. **Client only; the engine never reads it.** Same header and array directory as the baked map, with magic `MRRD` and kind 4 (CRC-32 checksum). **Coordinates are Web Mercator metres (EPSG:3857) as i32, the display projection**; the simulation keeps its equal-area Equal Earth coordinates, so the file carries cell and province centres in Mercator and the client never converts. Version 1 (Equal Earth) is no longer read.

| Group | Arrays |
| --- | --- |
| Levels | `levels_tolerance_m` (u32, coarsest first; default 8000, 2500, 800 m, measured in Equal Earth metres before projection) |
| Per level `k` | `l{k}_vx`, `l{k}_vy` (i32), `l{k}_vprov` (u32 province per vertex), `l{k}_tri` (u32, 3 vertex indexes per triangle; a triangle never spans two provinces) |
| Border lines per level | `l{k}_line_x`, `l{k}_line_y` (i32 points), `l{k}_line_off` (u32, lines + 1 entries), `l{k}_line_kind` (u8: 0 province border, 1 state border, 2 coast) |
| Cells | `cell_seg` (i32, four values per distinct cell edge: x0, y0, x1, y1), `cell_mx`, `cell_my` (i32 centres), `cell_ring_off` (u32, cells + 1 entries), `cell_ring_x`, `cell_ring_y` (i32, each cell's outer ring without the repeated closing point; used for picking and highlighting) |
| Provinces | `province_mx`, `province_my` (i32, area-weighted centres), `province_ref_area_km2` (u32, unsimplified Mercator area, for the validator) |

- Provinces are the union of their cells, simplified as one coverage per level (neighbours keep fitting) in Equal Earth, then projected vertex by vertex and repaired where the projection made a polygon invalid. Small islands may vanish at the coarse levels; the finest level keeps every province within 2 % (or 100 km2) of its true projected area.
- Mercator inflates areas away from the equator (about 4x at 60 degrees), so the file's areas are not real areas.
- Lines come from noding all province boundaries; a piece touching one province is coast, two provinces of one state a province border (province borders now include real admin-1 lines), two states a state border.
- Not yet in the file: cell fill meshes, rivers, contours, names and label positions, sea zones.
- Size: about 191 MB for the whole world (cell outlines 34 MB and cell rings 32 MB of it).
