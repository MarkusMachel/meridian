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
