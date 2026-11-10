# Data attributions and licenses

| Data | Used for | License | Notes |
| --- | --- | --- | --- |
| [Natural Earth](https://www.naturalearthdata.com/about/terms-of-use/) 5.1.1 (admin-0, admin-1, populated places, ports, airports, roads, railroads, urban areas, rivers and the regional river supplements for Europe, North America and Australia, glaciated areas, geography regions) and the Natural Earth II raster `NE2_HR_LC` 2.0.0 (land cover colours) | borders, cities, ports, airports, terrain overlays | Public domain; commercial use allowed, credit not required | "Made with Natural Earth." (optional credit) |
| [ETOPO 2022](https://www.ncei.noaa.gov/products/etopo-global-relief-model) 60 arc-second surface elevation, NOAA NCEI | elevation and ruggedness per cell | US government work, no restrictions as published by NOAA (verify the exact wording on the product page before a commercial release) | DOI 10.25921/fd45-gt74 for the 15 arc-second parent |
| [World Bank](https://data.worldbank.org/indicator/SP.POP.TOTL) SP.POP.TOTL, 1990 and 2019 | country population totals | CC BY 4.0 | Attribution required: "The World Bank: Population, total (SP.POP.TOTL)". Snapshots committed in `data/source/worldbank/`. |

Hand-written data (state merge table, chokepoints, strategic sites) is original work in this repository; strategic site coordinates are approximate and unverified.

## Software used by the client (checked 2026-11-09)

- `wgpu`, `winit`, `egui`, `egui-wgpu`, `egui-winit`, `pollster`, `bytemuck`, `png`: MIT and/or Apache-2.0 (licences as declared on crates.io; re-check with `cargo license` before a release).
- `epaint_default_fonts` (enabled by egui's `default_fonts` feature, used for the interface text): the crate is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`. It bundles Ubuntu-Light (Ubuntu Font License), Hack (MIT-style), Noto Emoji (SIL OFL) and an emoji icon font (MIT). All allow commercial use; the licence texts ship in that crate's `fonts/` folder and must be included with a release. Replace with the typewriter-style OFL fonts from `docs/design-notes.md` when the paper-map look is built.
