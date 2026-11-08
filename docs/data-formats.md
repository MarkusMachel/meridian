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
