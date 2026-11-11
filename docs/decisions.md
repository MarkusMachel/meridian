# Decision log

## Phase 1: Core loop (2026-11-08)

- **Fixed point:** Q32.32 in an i64, i128 intermediates for mul/div, saturating on overflow. Division by zero panics (caller bug).
- **RNG:** xoshiro256** seeded via splitmix64, verified against an independent Python implementation. Golden values pinned in tests.
- **Calendar:** tick 0 = 1990-01-01 00:00; date derived from tick count by integer civil-calendar math. Rollover signals are for the tick about to run (tick 0 counts as new day/month/year).
- **Hash:** FNV-1a 64 over explicit little-endian bytes, no dependencies. Not cryptographic; fine for divergence detection. A test mutates every field and checks the hash changes.
- **Tick order:** apply commands (in submission order) -> run ruleset -> advance clock.
- **Commands:** validated fully before mutating; a rejection changes nothing. Placeholder commands: MoveUnit, SetProvinceController. Issuer must own the unit.
- **Ruleset:** a trait in meridian-core (`Ruleset`); the proving-ground crate implements it, so the dependency points the right way. Rulesets keep no state of their own.
- **Rejected:** a float-free `f32` display helper in core (kept one `to_f64_for_display` only).
- **Linux verified (2026-11-08):** `scripts/check.sh` passes in Docker on linux/arm64 and linux/amd64 (rust:1.91). Seed 1 hash `2f1e797ab5e8d8f5` and seed 2 hash `490c68f0ed77447b` are identical to macOS.

## Phase 2: Data and scenarios (2026-11-08)

- **Crate versions checked on crates.io (2026-11-08):** ron 0.12.3 (updated 2026-10-28), postcard 1.1.3 (last release 2025-07-24, about 38M recent downloads, stable), rkyv 0.8.18 (updated 2026-08-05). All MIT or Apache-2.0, so fine for a possible commercial release. This supersedes the stale rkyv note in guide section 3.5.
- **Authored text format: RON**, as decided earlier. Files use anonymous structs `( ... )` with a top-level `version`. Schema types (`meridian-data/src/schema.rs`) are separate from engine types; the loader converts. `deny_unknown_fields` is on, so typos are errors.
- **Loader flow:** read version first (so an old file gets a version error, not a field error) -> full parse -> semantic checks collecting every error -> build world only if there were none. A bad file therefore cannot yield a partly built world; the API returns `Result<World, Vec<DataError>>`.
- **IDs:** unique and contiguous from 0 per table (so ID = array index). Gaps and duplicates are errors.
- **Errors** carry file, item (e.g. `units[2] (id 2)`), kind and problem. Classes: Io, Malformed (with line/column), MissingField, WrongVersion, DuplicateId, NonContiguousId, UnknownReference, BadValue, BadBinary.
- **Sites** added to the world (kind, province, owner, condition 0-100, built year, destroyed year) and to the state hash. Anchored to a province for now; the cell anchor arrives with cells in Phase 3.
- **Baker stub:** container header `MRDN | format_version u32 | kind u32 | payload_len u64 | checksum u64` (little-endian, FNV-1a checksum) around a postcard payload. Deterministic, round-trip tested, corruption and truncation detected. Scenarios are tiny, so the payload encoding does not matter yet.
- **Format benchmark** (`cargo run --release -p tools --bin format-bench`, Apple M4 Pro, median of 7 runs, synthetic map of 16,000 provinces, 600,000 cells, 3.6M adjacency entries):

  | Format | Size | Load |
  | --- | --- | --- |
  | RON text | 49.1 MB | 563 ms |
  | postcard to owned arrays | 21.6 MB | 12.6 ms |
  | custom flat arrays to owned | 28.3 MB | 0.6 ms |
  | rkyv validated zero-copy access | 28.3 MB | about 0 ms |
  | rkyv validate and convert to owned | 28.3 MB | 0.5 ms |

- **Decision: baked map data uses rkyv.** It is as fast as the hand-rolled flat format, validates on load, and spares us maintaining a custom container. Its layout is tied to type definitions, which is fine for data we regenerate. Revisit in Phase 3 if the real map (nested site tables) behaves differently. The custom flat format stays the fallback.
- **Decision: saves stay on postcard** (12.6 ms for 21.6 MB, small, and the format is under our version control). Needs a version number and migration step in Phase 5.
- **Rejected:** RON for the map (about 45x slower than postcard, 2x larger); bincode (unmaintained).
- **Caveat:** the benchmark map is synthetic random data. Real maps compress differently but the array sizes are the same, so load times should be comparable. rkyv zero-copy access needs a 16-byte-aligned buffer (use `rkyv::util::AlignedVec` when reading a file).
- **Deferred:** a stale-looking guide item "postcard actively maintained" is resolved as above (stable 1.x, heavy use, but the last release is over a year old; acceptable for a format we can also hand-roll).
- **Intentional hash change:** adding the site table to the state hash changed the Phase 1 golden hash (`9773012133288501445` -> `16117180310011935637`) and the runner hashes, even with zero sites, because the site count is hashed. Golden values re-pinned. The Phase 1 Linux hashes quoted above are historical.
- **Linux verified for Phase 2 (2026-11-08):** `scripts/check.sh` passes on linux/arm64 and linux/amd64. Runner hashes (seed 1 `9c4a31d740efcec5`, seed 2 `f99a6164964f1dab`) and `runner run data/scenarios/sample.ron` (`caded2dcbee2f0a5`) match macOS.

## Phase 3: Real map, step 1 (1990 states) (2026-11-08)

- **Source data: Natural Earth 5.1.1, 10m admin-0 countries and admin-1 states/provinces**, downloaded from naciscdn.org and pinned by SHA-256 in `crates/map-builder/fetch_sources.sh` (files themselves are not committed). License: the official Terms of Use say all vector data is public domain, free for personal, educational and commercial use, no credit required. (Natural Earth's forum mentions a GADM-derived-columns concern on admin-1; we use only geometry and names, never those columns.)
- **Chosen over** CShapes 2.0 (CC BY-NC, and two catalogs disagree on SA), historical-basemaps (unclear license, coarse), GADM (non-commercial only). CShapes may be used as a private cross-check only; nothing from it ships.
- **Approach:** 1990 states are built from modern entities with a hand-written table (`crates/map-builder/config/states_1990.json`): merges (USSR 15 republics, Yugoslavia 7 entities including Kosovo, Czechoslovakia) and splits (Germany into East and West, Yemen into North and South) done by cutting the admin-0 outline with admin-1 unions so outer borders stay shared. Dependencies follow their 1990 sovereign; 8 tiny or unclaimed areas and Antarctica are excluded, each with a reason in the config.
- **Known simplifications** (also in the config `notes`): Berlin whole to East Germany; Namibia independent (South African-administered until March 1990); Western Sahara to Morocco; Palestine to Israel; Hong Kong to the UK; Macau to Portugal; Palau to the US; Al Bayda' to North Yemen.
- **Result:** 173 states, total land 134.46 million km2 (guide estimate was about 135 million, includes dependencies). Spot checks against reference areas: West Germany 248.6k (real about 248.6k), Yugoslavia 254.8k (real 255.8k), Soviet Union 22.13M (real 22.4M including inland water). Both Yemens together are 453k km2 because Natural Earth's outline stops at the desert border; real figures are larger.
- **Tooling:** Python 3.15 in `crates/map-builder/.venv` (not committed), versions pinned in `requirements.txt` (shapely 2.2.0, pyproj 3.8.0, pyshp 3.1.7, numpy 2.5.4, scipy 1.18.1). Running `build_states.py` twice gives byte-identical GeoJSON (sha256 `16fa95d6...3508a`) on this machine; cross-machine reproducibility depends on GEOS version and is not yet tested.
- **Not yet done in Phase 3:** provinces, cells, adjacency graph, sea zones, sites, 1990 population (Natural Earth's population is 2019, so 1990 values need another source or scaling), baked map file, pathfinding, budgets.

## Phase 3: Real map, step 2 (provinces) (2026-11-08)

- **Result:** 15,240 provinces in 173 states (target was 15,000 to 17,000). Estimated area: median 9,001 km2, mean 8,823 km2, max 28,705 km2; 17 provinces over 15,000 km2, 1 over 20,000; 360 under 3,000 (mostly small states). Run time about 25 seconds.
- **Method** (`crates/map-builder/build_provinces.py`, no random numbers): project each state to Equal Earth (EPSG:8857), split it into landmasses, sample a regular grid (spacing chosen per state so a province gets about 100 points, capped at 10 km), cluster with Lloyd's k-means started from evenly spaced points along a Morton curve, then repair clusters that came out in pieces. Province IDs are ordered by state tag, then north to south and west to east.
- **Province count per state:** `max(round(area / 9000), min(10, max(1, floor(area / 450))))`. This is a change from the guide's flat "minimum 10 per country": provinces are never cut below about 450 km2 (two tactical cells), so Andorra (452 km2) gets 1, Luxembourg gets 5, and states from 4,500 km2 up get the full 10. The allocation across a state's landmasses uses the largest-remainder method so the total is exact.
- **Islands:** landmasses smaller than 0.6 of a province are pooled with neighbours within 1,500 km (single linkage) into island-group provinces, which may be several pieces joined by sea links. Example: Alaska's 191-island chain forms the largest province (28,705 km2). Every landmass above that size gets its own province.
- **Province area is an estimate** (points x spacing squared). Exact areas come from the cells in the next step. States' sampled areas are checked against true area (within 15% for states over 2,000 km2; no warnings).
- **Bug found and fixed during the step:** the fragment repair first ignored neighbours on the edge of a province's bounding box (141 split provinces); fixed by widening the box (14 remain, all thin coastal strips, accepted).
- **Known issues, to handle later:**
  - Natural Earth cuts polygons at the antimeridian, so Chukotka, Fiji and parts of Alaska's Aleutians are separate landmasses with an artificial seam; the adjacency step must wrap across it.
  - Greenland (about 240 provinces) and the Canadian Arctic mostly ice and tundra: a gameplay question, not an engine one.
  - French Guiana is part of the France polygon, so France's 83 provinces include it; same pattern for other overseas parts (logged for the regions layer).
  - Special territories (Berlin, Palestine and similar) get their own pass later, as agreed.
- **Outputs** (all in `data/intermediate/`, not committed): `provinces.csv` (id, state, lon, lat, estimated area, points, landmass count, spacing) and the labelled sample points as `.npy` files. Output is byte-identical across runs.
- **Pipeline:** `crates/map-builder/build_map.sh` runs fetch, states, provinces and the validator.

## Phase 3: Real map, steps 3 and 4 (cells, sea zones, graph) (2026-11-08)

- **Cells** (`build_cells.py`): 599,261 irregular cells, mean 224.4 km2 (target 225), 39.3 per province. About 75 seconds. Seeds come from a global jittered hex lattice (16.1 km spacing, jitter from an integer hash, no random numbers). A Voronoi diagram is made **per landmass** and clipped to the real outline, so cells follow coasts and never cross borders or sea. Landmasses under 20 km2 join a cell within 40 km, otherwise they are binned into 100 km squares (one multi-piece cell each); the largest landmass of a state is always treated as big. Cell area is exact: cells cover every state's outline to 0.0000% error.
- **Cell attributes in the data:** state, province, centroid (Equal Earth metres, i32), area in hectares (u32), landmass. Province assignment is by the nearest labelled sample point, then repaired so every province has at least one cell and is connected within each landmass. Province areas are now exact (`provinces_exact.csv`).
- **Cell graph:** 1.75 million edges (mean 5.86 neighbours) with shared border length in metres: Voronoi ridges clipped to land inside a landmass, a border pass between states (500 m tolerance, at least 1 km shared) and pairing across the 180-degree seam. Natural Earth's neighbouring borders overlap by up to 2.8 m2 in one place; accepted (error threshold is 10,000 m2).
- **Bug found by the validator and fixed:** tiny islands (for example Pitcairn) first got no seed and attached to the nearest cell, which could be thousands of kilometres away, leaving a province with a stolen far-away cell and no neighbours. Cells are now per landmass, and the validator fails any cell spanning more than 150 km.
- **Sea zones** (`build_sea.py`): a 20 km raster of the water in Equal Earth coordinates (after fixing a validity mask that counted 68M km2 outside the map outline as water; water is now 363.2M km2). Five water bodies above 100,000 km2 get zones (open ocean, Mediterranean, Black Sea, Baltic, Caspian); 454 smaller ones are lakes without zones. 862 zones on a jittered lattice that is finer near coasts: 450 km spacing within 250 km of land, 800 km out to 1,500 km, 1,400 km beyond. The raster seam at 180 degrees is joined so the Pacific is one body. Antarctica is blocked. Zones are repaired so each is one connected piece of at least 25 pixels (10,000 km2).
- **Strategic graph:** nodes are 15,240 provinces then 862 zones. Edges: 41,998 land borders (shared length), 3,701 coast links (number of coastal cells within 30 km of water), 2,159 sea-to-sea, 8 strait/canal. Chokepoints are in `config/chokepoints.json`; Gibraltar, Turkish Straits, Suez, Hormuz, Malacca, Panama and others are explicit edges. Eight of 16 chokepoints (for example Dover, Taiwan, Korea) have both ends in one large zone at this zone size, so they exist as names but not as separate edges; revisit if the gameplay needs them.
- **Repairs:** one DR Congo coastal province (5158) is connected by a repair link to its nearest province 18 km away because its corridor is under 300 m wide. The graph validator now passes: every province and zone has a neighbour, the whole world is one connected piece, and every landmass has a sea link.
- **Not done yet in Phase 3:** terrain (elevation download in progress), urban/road/rail/river data, sites, 1990 population, the baked file, Rust loading and map validation, pathfinding, budgets.

## Phase 3: Real map, steps 5 to 10 (terrain, sites, population, baked file, engine) (2026-11-08)

- **Terrain** (`build_terrain.py`): ETOPO 2022 60 arc-second surface elevation (NOAA, 466 MB, decoded once to an int16 cache) sampled at 9 points per cell: mean elevation and relief. Classes: plains, hills, mountains, high mountains from relief and height; desert from Natural Earth's named desert regions; ice from its glaciated areas. By area: plains 45%, hills 20%, mountains 12%, high mountains 7%, desert 13%, ice 2%. Spot checks (Everest, Sahara, Gobi, Greenland, Netherlands, Moscow) all land in the right class. Per cell also: built-up share, road, rail, river and coast flags; 48,163 cell edges are marked as crossed by a river. **Limits:** no forest or jungle layer (needs a land-cover dataset), and roads, rails and urban areas are modern Natural Earth data, not 1990.
- **Sites** (`build_sites.py`): 2,492 sites anchored to cells: 1,581 cities (population 300,000 or more, or any capital), 522 ports, 333 airfields, plus 56 hand-picked strategic sites (crossings, dams, nuclear and oil facilities, bases, launch sites; `config/strategic_sites.json`). Hand-picked coordinates are approximate and come from memory; they must be verified. `verified = 0` marks sites taken from modern data and assumed to exist in 1990 (for example a port opened in 2000 would wrongly be present). Sites built after 1990 (Channel Tunnel, Oresund Bridge, Three Gorges Dam, Chek Lap Kok, Kansai) are kept with their years so a 1990 start excludes them. City size is Natural Earth's modern population, not a 1990 figure.
- **Population** (`build_population.py`): World Bank SP.POP.TOTL for 1990 (CC BY 4.0, snapshots committed), attributed to the 1990 states; 5.30 billion of about 5.33 billion. Hand estimates for the two splits (East Germany 20.2% of Germany, South Yemen 21% of Yemen) and Taiwan (20.4 million, not in the World Bank table). Inside a state: cities as point masses (scaled by the country's 1990/2019 ratio, capped at 85% of the total), the rest over cells by area x terrain x built-up share. It is a plausible shape, not census data. 144 provinces (ice and the driest desert) have zero people.
- **Decision change: the baked map is a flat-array container, not rkyv.** The Phase 2 benchmark showed the custom flat format loading as fast as rkyv (0.6 ms against 0.5 ms), and the Python builder can write it directly while it cannot write rkyv. This is the fallback named in the Phase 2 decision. Header like the scenario container but with a CRC-32 checksum (fast in Python and Rust via the `crc32fast` crate, MIT or Apache). Postcard stays the choice for saves. Format in `docs/data-formats.md`.
- **Engine** (`meridian-core/src/map/`): `RawMap` (the arrays) validated by `Map::from_raw` (lengths, ranges, cells partition into provinces, both graphs sorted and symmetric with matching edge kinds), then indexed (a 64 km spatial grid and sites-by-cell, built by counting sort so it is deterministic). `Map::validate_world()` checks: no orphan nodes, one connected world, every landmass has a sea link, every province has cells whose areas add up. A* on both graphs with integer costs (rounded-up edge distances and a floor-rounded straight-line heuristic keep it admissible), reusable search memory, ties broken by node ID. Range queries `cells_within` and `sites_within` return IDs in ascending order. `World::from_map` makes a world with one country per 1990 state and each province controlled by its owner; the map's checksum is part of the state hash (map-less hashes unchanged).
- **Tests:** 25 core tests including A* against plain Dijkstra on 500 random queries (both graphs), range queries against brute force, path determinism with reused search memory, and 8 kinds of file corruption refused. 6 world-map tests on the real file (zero validation problems, 15,000 to 17,000 provinces, land area within 1%, named chokepoints, cross-world paths, corrupt files refused, world determinism).
- **Done test results:**
  - Builder run twice from scratch gives byte-identical output (`crates/map-builder/check_determinism.sh`, sha256 `8a80b557...8fa9`).
  - The Rust validator reports zero problems on the full world map, and every cell belongs to exactly one province.
  - Load 36 ms against a 120 ms budget; 10,000 strategic and 10,000 local cell queries inside their budgets (`docs/budgets.md`).
  - macOS, Linux arm64 and Linux x86_64 agree on all hashes and on the path fingerprint `bc7ecb8ba28ec26c`.
- **Deviations from the guide:** minimum provinces per country relaxed for tiny states (decided earlier); baked format as above; the tactical open/close of a province into cells is not built yet (see below); the Phase 3 text lists sites as an offline table but the loader exposes them directly.
- **Not built, deliberately deferred:**
  - Opening and closing a province into cells and the dynamic per-cell state (controller, damage, fortification, hazards, last fought; guide section 3A). The static data it needs is in place (`cells_of`, cell graph). It touches the world state, hash and saves, so it fits better with Phase 4 (systems) and Phase 5 (saves).
  - Units in `World` still sit on provinces; "units within N cells" needs cell positions for units.
  - Special territories (Berlin, Palestine, Western Sahara, Northern Cyprus, Kosovo and similar), which you asked to do in a separate pass.
  - The antimeridian: cells on the two sides of 180 degrees are linked, but the 441 or so province pieces on each side are separate provinces (Chukotka, Fiji, Aleutians).
  - Chokepoints at Dover, Taiwan, Korea, Bering, Lombok, Sicily and Bab-el-Mandeb are named but have both ends in one sea zone; they become real edges only if zones get finer there.
  - Sea zone names (Natural Earth marine polygons) and province names.

## Phase 4A: scheduler and primitives, steps 1 to 10 (2026-11-09)

- **Scheduler** (`schedule.rs`): a system is a plain function with a name, a rate (hourly, daily, monthly, yearly) and a list of systems it runs after. `ScheduleBuilder::build` sorts by dependencies with ties broken by name, so registration order never matters; cycles, unknown dependencies and duplicate names are errors at startup. Per-system timing is kept in atomic counters outside the world and is never hashed. A `Schedule` implements `Ruleset`, so `World::tick` is unchanged.
- **Events** (`world.rs`): `World::raise_event`; after the systems run, the queue is applied sorted by (target, source, kind) with raise order breaking ties. Events raised by handlers wait for the next tick. Pending events are hashed only when present, so the existing golden hashes did not change.
- **Hooks** (`hooks.rs`): data rules of rate + condition + effects over a small generic vocabulary (no scripting language). Conditions are checked against start-of-pass state, then effects of the fired rules are applied in rule-ID order. Loading from RON is not written yet.
- **Parallelism** (`parallel.rs`): chunked `thread::scope` helpers. Allowed only when each element's new value depends on its own index and old value; chunk results merge in chunk order. Tested identical for 1 to 16 threads over a simulated year.
- **Active sets** (`active.rs`): sorted `Vec<u32>`, checked against a `BTreeSet` model. Not wired into the world until per-cell state exists.
- **Modifiers and traits** (`modifiers.rs`): same-category modifiers add and are clamped per category; categories multiply in ascending order. Traits are sorted ID lists with scope `Holder` or `HolderAndBelow`.
- **Flow primitives** (`flows.rs`): shortage splitting (proportional, remainders by index), double-buffered flows, multi-source supply propagation. Conservation tests on random inputs. Version 1 has no per-link capacity.
- **Performance probe** (`runner probe`, numbers in `docs/budgets.md`): a full sweep of 600,000 cells and groups costs under a millisecond; parallelism gives no gain at that cost; hashing 18 MB takes 19.5 ms, so do not hash every tick.
- **Changed from the guide:** none. Step 5's data loader and the RON schema for hooks are left for when the first real rules exist.

## Phase 4B: world state, units and combat plumbing, steps 11 to 14 (2026-11-09)

- **Per-cell state** (`cells.rs`): dense arrays for controller, damage, hazard bits, last-fought tick, hunger, local food and ground condition, plus the sorted list of opened provinces. Present only on a world with a map. `open_province` copies the province controller to every cell; `close_province` gives the province to whoever holds most cells (ties to the lower country ID) and resets the cells. Population groups are not here: they belong to Phase 4C (step 15), and directional fortification is in the combat tables.
- **Military tables** (`forces.rs`): extra unit attributes in an array parallel to the unit table (parent, cell, status, stance, morale, cohesion, readiness in thousandths, veterancy, commander, encircled days, traits, transit), plus append-only officer, pilot and hull tables with stable IDs. Transit is a destination and an arrival tick; arrivals are applied at the start of the tick, right after commands, in unit ID order.
- **Combat tables** (`combat.rs`): engagements with a start-of-fight snapshot, signed pressure per cell, prisoner pools, garrison records, doctrine per country and track, country traits, directional fortification per (cell, facing cell), bridges per cell pair. Every table is a sorted vector; insertion order cannot change the hash. `garrison_size` is a pure integer function (no random numbers).
- **Derived caches** (`derived.rs`): zones of control (held, contested, empty), front segments between two countries, and units-by-cell with demand aggregation and the shortage split. All live outside `World`, are never hashed, and are tested to rebuild identically and independent of unit insertion order.
- **Hash compatibility:** the military, combat and event sections are hashed only when non-empty, so worlds that do not use them keep their old hashes (the golden hash is unchanged). The cell section is hashed on every map world.
- **Not built yet:** the visibility and air-defense caches (they need sensor and weapon data), the tech unlocked-model set (step 19), and any saving of caches (they are rebuilt, per the guide). The "rebuild after a save and load" part of the done test waits for Phase 5 saves; for now the tests rebuild from clones and from reordered inputs.
- **Deviation from the guide:** none in substance. Units still carry both a province and an optional cell; moving the whole unit model to cells is left for the rules phase.

## Phase 4C to 4F: economy, politics, diplomacy, events, intelligence, technology, views, AI boundary (2026-11-09)

All of this is plumbing: tables, conserving primitives, votes and validation. No goods, laws, treaties, techs or numbers are named in `meridian-core`. Every table is a sorted vector, hashed only when non-empty (so earlier hashes are unchanged), and tested for insertion-order independence where it matters.

- **Economy** (`econ.rs`, 4C): population group records (cell, culture, religion, heads, support shares) with explicit births, deaths and identity-keeping transfers, plus running `born`/`died` totals so conservation is checkable at any time; levelled buildings; regional stocks and prices; recipes (`apply_recipe`), staffing and banded prices as pure functions; construction projects that draw goods and labor with the shortage split and respect a minimum build time; production orders; equipment stockpiles; trade routes; treasury, debt and inflation per country, with `pay` moving money without creating it. New commands: `StartConstruction` (needs the cell under the issuer's control and a cost from the ruleset's `project_template`) and `SetProductionOrder`.
- **Politics** (`polity.rs`, 4D): regime, legitimacy and stability per country; factions with support and cohesion; a weighted faction vote (support x cohesion) for laws and treaties; laws; unrest per population group; movements; per-unit and per-officer loyalty; war levels per side. Commands `SetWarLevel` (levels only rise; lowering is a treaty's job), `HostileAct` (refused unless the issuer's war level reaches the requirement), `SignTreaty`. `World::create_country`, `set_unit_owner` and `split_country` (the civil war split: new country gets chosen provinces, units, buildings, sites and a share of treasury, debt and equipment; the original government keeps its ID).
- **Diplomacy** (`polity.rs`, 4D): treaties (major ones need every party's factions to approve), alliances with a call where each member decides once, a Security Council vote with vetoes and a majority-only Assembly vote.
- **Events** (`events.rs`, 4D): data-defined events with a trigger (the same condition vocabulary as hooks, now with flags), options, cooldown, chain gate and severity code; pending events, flags, chain stages and cooldowns are state. Polled by the schedule at a set rate; `ChooseEvent` command. Separate linked instances per party are separate definitions sharing a chain.
- **Intelligence** (`intel.rs`, 4E): agencies with capability per discipline, networks, operations (true owner and attributed party stored separately), per-country contacts with knowledge levels, and `estimate_range`, a deterministic range that always contains the truth and narrows with knowledge (no random numbers used).
- **Technology** (`tech.rs`, 4E): field levels, development programs that unlock specific models, a generic prerequisite check, learning capped by a rules-supplied diffusion limit, and transfers that give the model but not the fields.
- **Views and AI boundary** (`view.rs`, `crates/meridian-ai`, 4F): `World::view(country)` shows own forces exactly and other countries' units only through contacts, with owner and strength range filled in from "identified" up; `check` and `World::dry_run` validate a command without applying it (exact, implemented by running it on a copy of the world; fine for previews, not for every tick). The new `meridian-ai` crate has an `Ai` trait (view in, commands out), a private save section outside the world hash, and a tiered schedule with per-AI timing. The `omniscient` Cargo feature exists on `meridian-core`; the AI crate does not enable it (checked with `cargo tree`).
- **Done tests** (proving-ground): economy conserves goods and money and drafting lowers output; war-level refusal; law change by faction support; one defection plus a civil war split that conserves people, goods, money, debt, equipment and units; a major treaty that stalls then passes; an alliance call and a vetoed vote; a polled event chosen through the command interface.
- **Not done, honestly:**
  - Raw `World` accessors (`unit`, `controller` and so on) are still public. Making them crate-private so only views remain is a Phase 6 change; today the discipline is that the AI crate only receives a `CountryView`.
  - Visibility and air-defense caches, goods transport over the network beyond the flow primitive, trade-route execution, price updates as systems, and the civil-war trigger rules are rules/systems work and not built.
  - The "caches rebuild identically after save and load" test waits for Phase 5.
  - Only the map-less paths of economy, politics and events were exercised on the full map; none of this has run on the real 600,000-cell world yet.

## Phase 5: save, load and replay (2026-11-09)

- **Encoding: serde + postcard on the core types, not a hand-written writer.** `meridian-core` now depends on `serde` (derive only, no I/O); state types derive `Serialize`/`Deserialize`, and the attached map and the per-cell arrays are `#[serde(skip)]`. Data definitions that are not state (hooks, event definitions, recipes, schedules, rejections) deliberately do not derive it. Postcard has no field names, so field order is the format: adding or reordering a serialized field means bumping `SAVE_VERSION` and adding a `migrate` step. This is written in `docs/data-formats.md`.
- **Save** (`meridian-data/src/save.rs`): container `MRSV` v1 with CRC-32, body = map checksum, state hash, cell differences, world. Per-cell state is saved as differences from a fresh world on the same map (35.9 KB for a whole map world). The loader recomputes the state hash after decoding and refuses a mismatch; a wrong or missing map is refused with the checksums in the message. Derived caches are never saved.
- **Versions:** newer than known fails with "written by a newer build"; older than `OLDEST_MIGRATABLE` fails with "older than the oldest this build can migrate". Version 1 is the first, so `migrate` is a stub with one arm.
- **Replay** (`replay.rs`): container `MRRP` v1. Commands are logged with tick, source (player, AI, headquarters) and order of application; checkpoints every N ticks. Playing stops at the first mismatching checkpoint and reports its tick. `hash_trace` plus `first_divergence` compare two runs tick by tick. Replays never run the AI.
- **Autosave** (`autosave.rs`): policy only (when to save, file names, which old ones to delete); no menus, no file writing in the engine.
- **Tools:** `runner record | replay | diff | save | load`; `scripts/check.sh` now round-trips a replay and a save and compares against an uninterrupted run.
- **Fixed on the way:** `treaty_threshold` was a world field missing from the state hash; it is now hashed when it differs from the default (so older hashes are unchanged), and the hash-coverage test covers it.
- **Done test results:**
  - Save at 10 random ticks of a year, load, continue: hash equals the uninterrupted run every time.
  - A recorded replay (with AI-tagged commands) reproduces its final hash after a file round trip; tampering is caught at the next checkpoint (tick 400 for a change on tick 300 with checkpoints every 100); a wrong seed is caught at tick 0.
  - The divergence finder names tick 301 for a command removed on tick 300.
  - Version 0, version 99, a flipped byte, a truncated file and a non-save file each fail with a specific message.
  - On the real map, a world with an opened province and changed cells loads to an identical hash, and `ZocCache` rebuilt from the loaded world equals the original (this also closes the Phase 4 "caches rebuild identically after save and load" item).
  - Save and replay bytes are identical on macOS and Linux arm64 (sha256); tests pass in Docker `rust:1.91`.
- **Not done:** Linux x86_64 was not re-run for this phase; the map-world save was not timed (sizes only); a migration has never been exercised because no old version exists.

## Phase 6: client boundary (2026-11-09)

- **Raw accessors are gated by the `omniscient` feature.** Without it, `World::unit`, `controller`, `econ`, `polity` and the other raw readers/writers are `pub(crate)` (a `gated!` macro in `world.rs`); `World` fields are crate-private; `CellState` fields are private with getters and dirty-tracking setters. `meridian-data`, `proving-ground` and `runner` enable the feature; `meridian-ai` and `test-client` do not. **Caveat:** a workspace-wide build unifies features, so the check only means something per crate: `cargo check -p meridian-ai` and `-p test-client` (both are in `scripts/check.sh`). Proven by adding `w.unit_count()` to the AI crate: "method `unit_count` is private".
- **Commands:** `SetStance`, `Mobilize`, `Demobilize`, `Appoint`, `Launch`, `AirMission`, `Transit`, `SetDoctrine`, `Withdraw`, `Hold`, `CancelOrder`, `Fortify`, `AmphibiousAssault`, `JoinAssault`, `Bombard`, `BuildBridge`, `DismantleBridge`, `DemolishBridge`. Validated in `orders.rs`; a rejected command never changes state (tested). New `Ruleset` hooks (`bombard_range_m`, `landing_suitability`, `validate_launch`, `validate_mission`, `transit_ticks`, `doctrine_transition_ticks`, `can_build_bridge`) default to refusing, so the engine invents no gameplay. Bombard visibility: a target cell must be own, a contact, or adjacent to an own unit or cell.
- **Outputs:** `TickReport` carries changed cells, `ClientEvent`s and `Message`s (a transient outbox, not saved or hashed). `trace_modifiers` explains a modified value step by step.
- **Thread boundary** (`sim.rs`): `SimHandle` runs the world on its own thread. Commands go in over a channel; the UI reads through `query(closure)`, which runs on the sim thread between ticks, so the sim never waits on the UI. Queries are timed; `MAX_QUERY_NS` is 2 ms. `dry_run` copies the world, so it is for previews, not per-frame use.
- **Done tests:** `test-client` (no internals) runs a stub player against a stub AI deterministically by hash, including 400 refusals; the same match over `SimHandle` gives the same hash; a refusal returns a reason and changes nothing. On the real map (`runner/tests/sim_thread.rs`) the slowest of 300 `own_provinces` queries during a running sim was 17 µs against the 2 ms limit.
- **Not done:** Linux x86_64 not re-run for Phases 4 to 6; the 2 ms limit is measured only for ordinary reads, not for `dry_run`.

## Phase 7: client technology (2026-11-09)

- **Decision: custom renderer on wgpu, not Bevy.** Chosen by the owner, who accepts the longer build time. Reasons: the engine keeps sole ownership of state, scheduling and determinism; the client only reads through `SimHandle::query` and sends commands; Bevy's API churn is a bad fit for AI-written code maintained by a Rust newcomer.
- **Stack (versions checked 2026-11-09, all released within the last 3 months):** `wgpu` 30.0.1, `winit` 0.30.13 (stable; not the 0.31 beta), `egui` / `egui-wgpu` / `egui-winit` 0.37.0 for panels. `egui-wgpu` 0.37 needs `wgpu ^30` and `egui-winit` 0.37 needs `winit ^0.30.13`, so these versions fit together. Licenses (MIT/Apache-2.0) to be confirmed in `docs/attributions.md` when the crates are added.
- **Blocker found: the baked map has no geometry.** It stores cell centres and areas only, so province polygons, borders and coasts cannot be drawn from it. The render-only output file already planned in `design-notes.md` (contours, borders and cell outlines by zoom level) must be built in `map-builder` first. The engine never reads it.

## Phase 7, step 0: the render-only file (2026-11-09)

- **Built by `build_render.py`, read by `meridian_data::load_render_file`, never by the engine.** Province fill meshes and noded border lines at three simplification levels (8 km, 2.5 km, 0.8 km), plus every cell edge at full detail. Details in `docs/data-formats.md`.
- **Simplification is done on the whole province set at once (`coverage_simplify`)**, so neighbouring provinces keep sharing borders instead of opening gaps. The cost is that the coarse level drops small islands (190 of 15,240 provinces are off by more than 10 % in area there); the finest level keeps every province within 1 % or 50 km2. The validator enforces this and the file is byte-identical across runs.
- **Left out on purpose for now:** cell fill meshes (needed for contested-cell hatching), rivers, contours, names and label positions, sea zones. Each is an addition to the same file, not a format change, as long as new arrays are only added.

## Phase 7, first slice: the client window (2026-11-09)

- **Rust toolchain raised from 1.91.1 to 1.99.1.** `egui` 0.34 to 0.37 need Rust 1.92 or newer (0.37 needs 1.95), so a UI crate on 1.91 would have meant an old egui. Everything still builds and the golden hashes did not change. The 1.99 clippy added `chunks_exact_to_as_chunks`; it is allowed in the three places that use `chunks_exact`, because `chunks_exact` reads more plainly. The Docker Linux check needs a `rust:1.99` image from now on.
- **Structure.** `crates/client` is a library (camera, picker, map modes, session, renderer, offscreen drawing) with tests, plus a binary (`client`) that holds only the winit/egui window. Floats are used freely here; nothing in this crate feeds the simulation.
- **Raw values need the `inspect` feature.** The debug value mode and the province panel read raw per-province values, which are gated behind `meridian-core/omniscient`. The client has a default-on `inspect` feature that turns it on; without it the snapshot falls back to the viewer's own `CountryView`. A build for players should turn it off. Until country-filtered inspection exists, this is the one place the client sees the true world.
- **Session and command log.** `Session` runs the sim thread and sends commands plus a run request only while the sim is idle, so every command lands on a known tick and the log `(tick, command)` is exact. Done test (`crates/client/tests/session.rs`, real map): a session through all five speeds and single steps, with orders along the way, ends with the same hash as `replay_headless` of its log, and a different hash from a run without the orders.
- **Rendering.** Province fills are triangle meshes coloured through a per-province colour buffer (a mode only produces colours, so a new mode never touches the renderer); border lines are instanced quads sized in pixels; cell edges appear below 2.5 km per pixel. World positions are subtracted as integers before they become floats, so there is no jitter at any zoom. The detail level follows zoom (coarsest whose error is under 2 px). Picking tests the finest mesh through a grid, so a click hits exactly what is drawn.
- **Checking without seeing a screen.** `client --screenshot out.png [--view CX,CY,M_PER_PX] [--mode ...] [--ticks N] [--select P]` renders a frame without a window; `scripts/check.sh` runs it as a smoke test. The window itself was started and ran for 8 seconds without a panic, but its interactive behaviour (drag, wheel, click, panels, keys) has not been exercised by a person yet.
- **Measured, not tuned.** Offscreen frames with readback at 2560x1440 over the whole world: mean 8.3 ms world view, 7.4 ms regional, 12.0 ms and 11.4 ms close up; the slowest single frame was 25 ms close up. Cell edges are drawn without viewport culling (2.1 million segments), which is most of the close-up cost; culling by tile is the first optimisation to make if the target is missed.
- **Not yet in this slice:** unit icons and movement arrows, save/load menu, settings screen, frame-rate target written into the budgets, labels (no province names exist yet).

## Phase 7, second slice: units, orders, save and load (2026-11-09)

- **Units and arrows.** Unit markers (a square in the owner's colour, white outline when selected) and arrows for unfinished move orders are painted by egui over the map and under the panels. Select a unit by clicking it or through the province panel, then right-click a province: the client issues `MoveUnit` through the session (so it is in the command log and applied at the next tick). The arrow stays until the unit is at its destination.
- **Save and load.** Game menu: Save writes `saves/quicksave.mrsv` (git-ignored); Load replaces the session with a new one built from the file, after the loader has checked the state hash and map checksum. `Session::save_bytes` first waits until the sim has finished every tick it was asked for, so the saved tick is exactly the requested one. Settings window: cell-edge toggle and interface scale.
- **Exact stops.** `Session::set_stop_at(tick)` asks the sim for no ticks past a chosen tick, and `finish` waits for the sim to go idle. An earlier version of the new save/load test was flaky (605 ticks against 600) because a run request issued just before the check was still in flight; the stop and the wait fixed it, and the test passed 8 of 8 runs afterwards. The session done test had the same weakness in principle and now reads its final tick from the finished world.
- **Done test additions:** a session that saves at tick 250, loads into a new session and runs to 600 ends with the same hash as an uninterrupted run to 600 (real map).
- **Frame-rate target: 60 fps (16.6 ms) at 2560x1440 with the whole world on screen.** The offscreen numbers in `docs/budgets.md` meet it on average at every zoom (mean 7 to 12 ms including readback); the slowest single close-up frames (16.7 and 25 ms) do not, and cell edges are not culled. The windowed frame time (with panels and vsync) is shown live in the top bar and has not been recorded by a person yet.
- **Honest gaps:** the window, the overlay (units, arrows), menus and right-click orders compile and the window runs, but nobody has driven them yet; they are untested beyond starting up. No province labels (no names yet), no autosave in the client, one save slot.

## Phase 7: time speeds in real time (2026-11-09)

- **Speeds are game hours per real second, not ticks per frame.** The owner asked for speed 1 to be one game hour per second, with each speed a bit faster, as in a paradox-style game. This replaces the guide's "speeds of 1 to 5 ticks per frame", which ran at 60 to 300 hours a second and looked the same at every setting. Speeds 1 to 5 are 1, 3, 8, 24 and 72 hours per second (`HOURS_PER_SECOND` in `session.rs`; change the numbers there). Measured with a 60 fps loop over 2 seconds: 0.5 (one tick in the window), 2.5, 7.5, 23.5 and 71 ticks per second.
- **Determinism is unchanged.** `Session::advance(dt)` only decides how soon the next ticks are asked for (fractions carry over, and at most a quarter second is owed, so a stall does not become a burst); which ticks run and which commands land on which tick are as before, so a client run still matches a headless run of its log. `Session::frame()` is `advance(1/60)` for tests, which have no real clock.
- Single step and keys `1` to `5` work as before. The speed buttons show their rate when hovered.

## Phase 7 follow-up: Mercator display and cell selection (2026-11-09)

- **The display projection is Web Mercator; the simulation stays equal-area.** The owner prefers Mercator. Distances, areas and the cell grid are still computed in Equal Earth (a cell is about 225 km2 wherever it is), so nothing in the engine changed. Mercator only changes how the render file draws things: `build_render.py` simplifies in Equal Earth, then projects vertex by vertex. Render file version 2 carries the cell and province centres in Mercator so the client needs no projection code. Consequence the owner should know: Mercator stretches areas away from the equator (about 4x at 60 degrees), so cells and provinces *look* much bigger in the north (Greenland, Canada, Russia) although they are the same size in the simulation.
- **Cell selection.** Click selects the province and the cell under the cursor. The cell is found through a grid over cell centres, then tested against each cell's real outline (the render file now stores every cell's outer ring), and highlighted with a white-and-black outline. The panel shows the cell's province, terrain, elevation, relief, area, population, built-up share and road/rail/river/coast flags. Test on the real map: 98 % or more of cell centres pick their own cell, and a mid-Pacific point is no cell and no province.
- **Camera limits follow the map**, not fixed world constants: the view stays inside the drawn map plus a 2 % margin.
- **Validator:** the finest-level area gate is now 2 % or 100 km2 (it was 1 % or 50 km2), because Mercator inflates high-latitude areas and the 800 m simplification loses proportionally more there.

## Province rebuild: provinces follow real admin-1 lines (2026-11-09)

- **Why.** Provinces were equal-area k-means blobs per country, so their edges meant nothing. The owner wanted provinces closer to real ones (Victoria 2 style), and chose to rebuild now.
- **Regions.** New map-builder step `build_regions.py` cuts each 1990 state into *regions* from Natural Earth admin-1 (public domain, already a source): each unit is assigned to its state through its admin-0 entity (Germany and Yemen by the name lists in `states_1990.json`), cut to the state outline, made disjoint, and any land no unit covers joins the touching region with the longest shared border. Regions under 1,500 km2 merge into the neighbour with the longest shared border (same admin-0 preferred; an island with no neighbour joins the nearest region). A state with no admin-1 data is one region. Result: 3,019 regions in 173 states, median 7,920 km2.
- **Provinces and cells never cross a region.** `build_provinces.py` runs k-means per region with about one province per 9,000 km2 (at least one per region; a small state keeps its old floor, so tiny countries still get several). `build_cells.py` builds the Voronoi cells per region, assigns each cell only to provinces of its own region, and repairs only inside the region. The cell graph treats region borders like state borders. `provinces.csv` gains a `region` column and the cells get `cell_region.npy` (intermediate only; the baked map format is unchanged).
- **Result:** 16,182 provinces (was 15,240); estimated area min 0 (Vatican), median 8,880, mean 8,310, max 28,588 km2; 1,179 provinces under 3,000 km2 (small regions kept whole). 599,300 cells. Render file 191 MB (was 149 MB) because province borders now follow detailed admin-1 lines.
- **Validators:** every region has a province; no cell is in a province of another region; the hairline-overlap limit for cell pairs is now 50,000 m2 (region outlines made valid after projection can overlap by up to about 40,000 m2, a fifth of a hundredth of a cell).
- **Consequences:** province ids changed, so older saves do not load (the loader refuses them on the map checksum). Provinces are not equal-size any more: a region just over the threshold gets one province, so sizes range widely. Province names and 1990 admin changes (for example Soviet republic borders, which are admin-0 lines in the source) are not modelled yet.
- **Path fingerprint changed with the map:** `096013a9723c06d4` on macOS (the Phase 4 value `bc7ecb8ba28ec26c` belonged to the old provinces). Linux has not been re-run on the new map.

## Phase 7: first Victoria-2-style look pass (2026-11-09)

- Muted sea colour, brown-ink province lines (soft), heavier dark-brown state borders, blue-grey coast line, and country colours in a narrower earthy band. Colours and widths only (`render.rs` `STYLES`, `SEA`, `modes.rs` `country_color`); no new geometry. Still to do for the full look: terrain/shaded-relief base under the owner tint, rivers, dashed borders, labels.

## Phase 7: relief, rivers, dashed borders, place names (2026-11-09)

- **Render file version 3** (see data-formats.md): shaded relief raster, river lines and place names are baked by `render_extras.py`. Relief is a hillshade of the ETOPO 2022 elevation (already used for terrain; public domain), sun from the north-west, vertical exaggeration 6, one pixel about 4.9 km. It is coarse: mountain ranges read well, hills do not. A sharper raster costs file size (16384 wide would add about 130 MB), so it is not done.
- **Relief is multiplied into the province colour in the fill shader** (strength 0.85), so every map mode gets it; Settings can turn it off.
- **Rivers** are Natural Earth 10 m centrelines (public domain), drawn with the line pipeline in three tiers by `scalerank` and zoom (major below 30 km/px, medium below 9 km/px, all below 3.5 km/px). They are drawn over fills and under borders; they are not clipped to land.
- **Dashed province borders:** each line instance now carries the distance along its line, and the fragment shader discards the gaps of a 8 px period. State borders and coasts stay solid. Cell edges are padded with a zero distance.
- **Place names:** countries (1990 names from `states_1990.geojson`) and regions (Natural Earth admin-1 names, merged regions take the larger part's name). No province names exist; regions stand in for them. Placement is straight text, no curves: a name is shown only if its area is wide enough on screen, bigger areas are placed first and overlapping labels are dropped (`client/src/labels.rs`, tested). Painted with egui over the map; Settings can turn them off. Names are Natural Earth `name` fields in Latin script.
- **Not done:** curved labels, bathymetry in the sea, hand-tuned label positions, province names, name translations.

## Provinces are the real divisions; readable labels (2026-11-09)

- **One province per region.** The owner wants provinces to match each country's own states or divisions, not equal-size blobs, and does not need them to be the same size. `build_provinces.py` now makes one province per region (a Natural Earth admin-1 unit, tiny ones merged into a neighbour as before). A region over 120,000 km2 is cut into provinces of about 70,000 km2 so that a Siberian oblast or a Canadian territory is not one 3 million km2 province. A region made of several islands is still one province. The old rule of at least 10 provinces per state is gone: a small state may be one province (Monaco, the Vatican).
- **Result:** 4,008 provinces (was 16,182); area median 15,650 km2, mean 33,546, max 174,400; 579 under 3,000 km2. Cells are unchanged (599,300, mean 150 per province). Map file 73.3 MB, render file 204 MB, strategic graph 181 land groups.
- **Consequences:** province ids changed again (old saves do not load); the strategic graph is four times smaller, so path queries and per-province systems are cheaper; a province is now a much coarser unit for the game (a tactical fight spans many cells inside one province). The map test now expects 2,500 to 4,500 provinces. Path fingerprint on macOS: `afb28d83a9b0e6ef`.
- **Poles:** the display is Mercator, which inflates area toward the poles whatever the provinces are; what changed is that high-latitude provinces now follow real borders instead of k-means blobs computed in Equal Earth. A different display projection would be needed to remove the inflation itself.
- **Labels:** the unreadable names came from sizing in physical pixels, so on a high-density screen they were half as big as intended. Sizes are now logical points times the screen scale (country 13 to 44, region 12 to 17), with a thicker light halo and darker ink.

## Display projection back to Equal Earth (2026-11-09)

- **Supersedes the Mercator choice.** The owner wants no stretch toward the poles. The render file (version 4) is in the simulation's own Equal Earth plane: areas on screen are true, cells all look the same size, and nothing converts between display and simulation coordinates. Shapes are slightly bent toward the edges of the map (the cost of an equal-area projection); the world is no longer a rectangle with straight meridians.
- **What changed:** `build_render.py` no longer projects (simplification happens in the display plane itself, which is better for neighbours fitting); the relief raster is resampled in the Equal Earth plane (4.2 km per pixel, 30 MB) by inverting the projection per pixel; labels no longer need a stretch factor; camera limits and the `--view` option are in Equal Earth metres. The render file is 189 MB (was 204 MB). Provinces, cells and the baked map are untouched.
- **Cost:** `--view` coordinates from earlier notes (Mercator) no longer apply.

## Phase 7: information by zoom, darker colours (2026-11-09)

- **Sites appear as you zoom in** (`labels::place_sites`, painted by egui): cities with a name and a dot, by population: over 2 M below 25 km/px, over 700 k below 12 km/px, over 200 k below 6 km/px, all below 3 km/px. Ports (blue square) and airfields (grey triangle) below 4 km/px; other sites (industry, bases, crossings) below 2 km/px. Only sites that exist in 1990. A name is dropped when it would overlap a place name or another site name. Country names show from far away, region names (province stand-ins) from mid zoom, cell edges below 2.5 km/px, rivers by tier, dashed province borders throughout. Site markers and names follow the "Place names" setting.
- **Darker, calmer colours:** country colours have lower lightness and saturation (about 0.36 to 0.57 lightness) and relief darkens slightly (strength 0.7, times 0.92), because the earlier palette read as bright.
- **Still missing** (needs more data or work): roads and railways on the map (cell flags exist but no lines are drawn), terrain texture at close zoom (cells are not filled separately), building icons, province names, unit icons by type.

## Phase 7: roads, railways, built-up areas and building icons (2026-11-09)

- **What the owner asked for:** a map that shows the country like HOI or Victoria do when zoomed in (roads, rail network, cities, buildings), not only colours and names.
- **Data:** Natural Earth 10 m roads (56,355 lines; class 0 major highway, 1 secondary, 2 other; ferries left out), railroads (25,368 lines; class by scalerank: 0 main, 1 regional, 2 other) and urban areas (11,878 outlines, one triangle mesh). All public domain, all already downloaded. Lines are simplified at 300 m, urban outlines at 150 m. Render file version 5, about 200 MB.
- **They are modern data, not 1990.** Like the sites, roads and railways are today's network: motorways built after 1990 (much of Eastern Europe, China) are shown, some closed lines are missing. A 1990 network would need a historical source; none permissive was found yet.
- **Drawing (by zoom):** built-up areas as a dark translucent fill below 9 km/px; roads below 6 / 2.5 / 1.2 km/px by class, drawn other-first so highways sit on top (red-brown, widths 1.9 / 1.3 / 0.9 px); railways below 8 / 3 / 1.4 km/px by class, drawn twice (dark bed, then light dashes on top) for the classic railway look. Cell edges now only below 900 m/px (they cluttered mid zoom). Settings: "Roads, railways and cities".
- **Building icons** (egui shapes, ink on a light disc): skyline for cities over 1 million, ringed dot for others (bigger over 300,000), anchor for ports, aeroplane for airfields, bridge for crossings, factory for industry, flag for military bases. A port or airfield in a city's cell sits beside the city, and a name is written once.
- **Names and icons are clipped to the map area** (they drew over the top bar and side panel).
- `client --view` now also sets the window's starting camera.
- **Not done:** 3D models, city sprites that grow with population, terrain textures (forest, desert) at close zoom, a 1990 road/rail network.

## Phase 7: names fade when zoomed in, wider roads and railways close up (2026-11-09)

- **Country names fade out** between 2.4 and 1.2 km/px and are gone closer in; region names fade between 300 and 150 m/px. A name is also hidden once its area is wider than the screen (it fills the view). Close up the map shows cities and infrastructure, not the name of the country you are standing in (as in HOI). Tested in `labels.rs`.
- **Roads and railways widen as you zoom in:** width (and the railway dash length) times 1,200 / metres-per-pixel, between 1x and 4x. At 300 m/px a major highway is about 7.6 px and a main railway about 7.6 px, so they are visible targets. Cutting a road or railway (interdiction) is gameplay and belongs in the gameplay backlog; the map already marks each cell with road/rail flags, so a cut would be a command on those cells.

## Phase 7: land texture, no factory symbol (2026-11-09)

- **The land is no longer flat colour.** The owner wanted texture like HOI and Victoria. Source: Natural Earth II `NE2_HR_LC` 2.0.0 (public domain raster of land cover colours: forest, grassland, desert, tundra, ice; 1 arc minute, 125 MB download, SHA-256 in `fetch_sources.sh`). `render_extras.land_colors` resamples it onto the relief grid in the Equal Earth plane (2 x 2 average per pixel). Render file version 6, now about 292 MB (the colours are 92 MB; a 4,096-wide colour raster would save about 70 MB if size matters later).
- **Blend:** political mode multiplies the country colour by the land colour (x 1.2) and mixes it 65 % with the plain land colour (x 0.9), so a country keeps its hue but is darker over forest and warmer over desert, and relief shades it as before. Terrain mode now shows the land colours themselves; the debug value mode stays flat. Tried and rejected: a plain 60 % mix (washed out to pastels) and a 1.45 gain (neon over deserts).
- **Fine ground texture close up:** below 3 km/px the shader adds a three-scale value noise (1.6 km, 450 m, 140 m; plus or minus 8 %), so zoomed in the ground is mottled rather than one flat tone. It is a hash of the world position, so it does not shimmer when panning.
- **Industry sites have no symbol any more** (the owner asked). They are still in the map data.

## Phase 7: 3D map close up, paper map far out (2026-11-09)

The owner wanted the map to look closer to Hearts of Iron IV's map: terrain, models, water, and a paper map when zoomed out. It stays on wgpu, with no engine change. The renderer has two modes, chosen by zoom.

- **Camera.** It is a perspective camera that keeps the old API. How 3D the view is (`depth_t`) is a smooth step on a log scale of zoom, from 0 at 6,000 m/px and farther out to 1 at 700 m/px and closer. Tilt is 48 degrees times that, and terrain exaggeration is 14 times that. The field of view is 30 degrees, and the camera distance is set so a pixel at the screen centre still covers `m_per_px`. With no tilt the view equals the old flat view. Panning and zooming keep the ground point under the cursor fixed. Tested in `camera.rs`, including a round trip through the tilted view.
- **Zoomed out: paper map.** The flat map is drawn straight to the screen with a faint paper grain fixed to the world. The sea is an atlas sea: lighter on the shelf, a shade darker past 200, 1,000, 3,000 and 5,000 m, with a thin contour at each. Depth comes from the new `height_m` raster (render file version 7).
- **Zoomed in: 3D.** Each frame:
  1. The flat map is drawn from straight above into a 4,096-pixel overlay texture covering the ground on screen plus 8 %. Line widths are scaled so they stay the same on screen.
  2. A 384 x 384 terrain mesh is raised from the height texture (16-bit float, filterable on every GPU), takes its colour from the overlay, and is lit from the north-west by slope.
  3. Water has depth colour, slow animated swell, a weak sun highlight and a lighter sheen at low angles. The swell is five directional waves whose crests are bent by noise and whose strength varies by place; it calms with distance so far water does not shimmer. Value-noise waves and straight sine waves were tried first and rejected: the first looked blotchy and boxy, the second like corduroy.
  4. Distant ground fades into a light haze.
  5. The political colour fades from 65 % to 45 % as the view tilts, so the ground shows through.
- **Models.** These are procedural, with no model files.
  - **Trees** are six-sided cones on a world-fixed jittered grid, about 12 screen pixels apart, below 1,500 m/px. A grid cell has a tree only where the Natural Earth II colour is forest. The test is green minus red over about 0.1 and red under about 0.66; it was checked on samples of taiga, Amazon, Congo, Black Forest, farmland, steppe and desert, and farmland and steppe get none.
  - **Buildings** are boxes inside built-up areas below 1,200 m/px, using a mask of the urban triangles drawn each frame.
  - Grid spacing snaps to powers of two so models stay put while zooming. Each kind is capped at 250,000 instances.
  - Not done yet: ports, airfields and bridges as models; they are still the 2D icons.
- **Labels, icons and picking** use the drawn terrain height (`heights.rs`, the same bilinear sampling and the same sea-at-zero rule as the shader), so they sit on the mountains, and a click picks the ground under the cursor.
- **Line culling.** All line layers (borders, cell edges, rivers, roads, railways) are sorted by class and then by world tile (64 x 32). Each frame draws only the tile rows in view. This took the close 3D frame from 20.2 to 7.8 ms at 2560x1440 (budgets.md).
- **Display only.** Floats are used here, which the determinism rules allow for display. The engine and the map file are unchanged; the path fingerprint is still `afb28d83a9b0e6ef`.


## Phase 7: trees and buildings closer to the Hearts of Iron IV map (2026-11-09)

The owner found the cone trees and box buildings poor. Free model packs were looked at (Quaternius, Kenney, Poly Haven) and rejected: too cartoonish or too detailed for this scale. The models stay procedural, with no model files, and are reshaped toward the Hearts of Iron IV look.

- **Forests are dense carpets.** Trees are about 8 screen pixels apart (was 12) and larger, so crowns touch. The ground under them is darkened and mottled (`forest_at` reads the same land-cover colours as before), so a forest reads as one dark mass from far away. Slow noise opens clearings and ragged edges.
- **Tree kinds by climate.** A tree is two six-sided cones (36 vertices, was 18). Conifers (two stacked tiers) toward the poles and up mountains, lumpy broadleaf crowns elsewhere, darker and taller near the equator. Latitude is estimated from Equal Earth y in the shader, within about two degrees. Greens are muted, and crowns are darker toward the bottom.
- **Towns.** A building is walls, a pitched roof and gable ends (42 vertices, was 30). Houses are low, in plaster and concrete tones with slate, brown and dull red roofs; houses of one neighbourhood (8 x 8 plots) face the same way. Deep inside a big town some plots become taller flat-roofed blocks. Some plots stay empty, more of them near the edge. The ground under a town is grey and mottled.
- **`scene_probe` example** (`cargo run --release -p client --example scene_probe -- OUT_DIR [LAT_DEG]`): draws a made-up landscape through the real renderer and writes screenshots, with no baked map needed. It was used to check this change on a machine without the map.
- **Not measured yet:** this was checked only on the made-up scene with a software renderer, not on the real map or on the M4 Pro. The instance cap went from 250,000 to 400,000 per kind and trees have twice the triangles, so the close 3D frame time in `docs/budgets.md` (7.8 ms) must be measured again with `client --bench` before this counts as within budget.
- **Still not done:** ports, airfields and bridges are still 2D icons.

## Phase 7: town plans and roads that look like roads (2026-11-09)

The owner wanted prettier cities and roads that read as roads. Still procedural, no model files.

- **Roads.** As the view turns 3D, the red atlas line becomes asphalt: a dark edge is drawn under every road first, then the grey surface (motorway, main road, lane: about 9, 6 and 3.6 pixels at the closest zoom), then a broken centre line on the two bigger classes. All edges go down before any surface, so junctions join cleanly. These are extra passes of the existing line pipeline with new styles (`ROAD_EDGE_STYLE`, `ROAD_CENTRE_STYLE`); the paper map is unchanged.
- **Town plan.** A town is cut into square plots (the grid the buildings already used). Streets run along plot edges: one between every two rows, so each house fronts a street and backs onto a yard; a cross street every four plots or so; a wider avenue every 16. Each 16 x 16 district runs its rows east-west or north-south. The plan is four small functions repeated in `shader.wgsl` (which paints asphalt, pavements, yards, gardens and park grass into the overlay, `fs_urban`) and in `terrain.wgsl` (which stands the buildings on it). Real roads and railways are drawn after the town ground, so they stay visible through a town.
- **Buildings.** Three kinds, all axis-aligned to their street: detached houses; terraces (the houses of a block joined into one row with one roof); and, deep inside a big town, fewer and lower flat-roofed blocks than before. Roofs lean to terracotta or slate by district. Park blocks get grass and broadleaf trees instead of houses (drawn by the building pipeline with the shared `crown` function).
- **Known limits.** Buildings still stand on top of a real road or railway that crosses their plot. The plan is a symbol, not real streets: a plot is 11 screen pixels, so at the closest zoom a "house" is about a kilometre wide, and the plan redraws when the plot size doubles while zooming. Checked only with `scene_probe` on a software renderer; frame time on the M4 Pro is not measured (the town ground costs a few hash and noise lookups per town pixel in the overlay pass, and roads are drawn up to three times).

## Phase 7: city landmarks, regional town styles, forests by climate (2026-11-09)

The owner asked for a landmark in each city, buildings that fit where they are, and forests that fit their climate. Still procedural, no model files. Display only: none of this touches the simulation.

- **Where in the world.** `lat_lon` in `terrain.wgsl` runs the Equal Earth projection backwards (Newton's method, four steps), replacing the rough latitude estimate. `culture` sorts a point into nine broad parts of the world by boxes of latitude and longitude: Europe, Russia and Eastern Europe, Middle East and North Africa (with Turkey, Iran, Central Asia), Africa south of the Sahara, South Asia, East Asia, South-East Asia, North America and Australia, Latin America. The boxes are coarse and are a display choice only; borders are blurred with slow noise for houses. They are not game data and should come from the culture data once that exists.
- **Houses by region** (`house_style`): roof pitch, share of flat roofs, three wall and three roof colours, and how many and how tall the tall blocks are. For example flat sand and white houses in the Middle East, steep red-brown roofs in South-East Asia, dark and blue tile roofs in East Asia.
- **Landmarks.** One per city site, drawn by a new instanced pipeline (`vs_landmark`): up to six parts, each a block with rectangle or octagon outlines that can taper (box, drum, roof wedge, spire, dome). Church with a spire (Europe; smaller and whitewashed in Africa), onion-dome church, mosque with dome and minaret, temple tower, pagoda, stupa, office towers, colonial church with two towers. They are drawn 1.7 times their plot size so they stand out.
  - `Renderer::set_cities` takes the city positions (`labels::city_positions`: sites of kind 0 that exist in 1990). `city_landmarks` moves each to the middle of the built-up area within 30 km (site positions are cell centres, often beside the town), records the town's radius, and drops cities with no built-up area nearby. Tested.
  - A landmark stands on a paved square of four plots by four on plot corners. The square is cut out of the town mask (so no houses or trees stand on it) and painted into the overlay. A town shows its landmark only when its radius is at least four plots, so small towns get theirs only close up.
- **Forests by climate** (`vs_tree`): from latitude, height and the land colour. Narrow dark spruce in the far north and on high mountains, thinning toward the tree line; rounded broadleaf with some conifers in temperate country; scattered stone pines, cypresses and olive-grey trees in dry warm country below 46 degrees; scattered acacias and palms in savanna; a tall deep green canopy with a few giants above it in rainforest. Trees now have four shapes (`crown`: broadleaf, conifer, umbrella, palm), still 36 vertices. Scattered trees stand where the ground colour is dry but a little green ("scrub"), which was treeless before. Park trees in towns are palms in hot countries.
- **`scene_probe`** takes a longitude too, and passes three city sites.
- **Not verified:** all of this was checked only on the made-up scene with a software renderer. On the real map the scrub rule may put scattered trees on farmland or leave real savanna bare, depending on the Natural Earth II colours there; the region boxes will be wrong along some borders (the Balkans, the Caucasus, the Sahel, northern India); and the frame time on the M4 Pro is not measured.

## Phase 7: rivers you can see, traffic, nothing built on roads (2026-11-09)

The owner asked where the rivers were, and for little cars, trucks and trains moving on the map (1990). Display only.

- **Rivers were there but hidden.** They were drawn 1.1 pixels wide at every zoom, under the dark forest floor, the trees and the houses. Now each class (big, medium, small) has its own style and widens as you zoom in, to about 8, 5 and 3 pixels at the closest zoom, and turns from map ink to a water colour. In the 3D view the river band is drawn as water with the same moving surface as the sea.
- **The mask has three channels** (was one; now `Rgba8Unorm`): red = town, green = keep clear, blue = river. Rivers, roads and railways are drawn into it with the line pipeline and per-channel write masks. Trees and buildings do not stand where green is set (a building checks the middle and four sides of its plot), and the forest floor is not darkened there. This removes the earlier limit that buildings stood on roads.
- **River data is unchanged:** Natural Earth 10 m river centrelines, the main file only. Its regional supplements (Europe, North America, Australia), which add many smaller rivers, are not fetched; adding them means new pinned downloads and a map rebuild.
- **Traffic.** Below 450 m/px, cars and trucks move on the roads and trains on the railways. Each road carries two conveyor belts of slots, one each way, keeping right; a railway carries one belt of car-length slots with a train of 4 to 9 cars (locomotive in front, coaches of one livery or mixed goods wagons) in some of every 56 slots. A slot's vehicle, kind and colour come from a fixed hash of its number, so a vehicle stays the same as it crosses segments. The vehicles in view are placed on the CPU each frame from the tile-culled segments (`road_traffic`, `rail_traffic`, tested), at most 30,000, and drawn as two boxes each (`vs_vehicle`). Roads and railways keep a CPU copy of their segments for this.
- **Symbols, not simulation.** Speeds are in pixels per second (16 for road, 22 for rail), spacing is in pixels at a zoom step, traffic density is by road class only, everybody drives on the right, and it moves with wall-clock time, also when the game is paused. None of it reads or changes the world. It should later follow the game clock and real traffic or supply flows.
- **Not measured:** only checked with `scene_probe` on a software renderer. The per-frame CPU pass over the road and railway segments in view, and the frame time on the M4 Pro, are unmeasured. The CPU copy of the segments costs 20 bytes each.

## Phase 7: farmland (2026-11-09)

The owner asked for farmland and the like. Display only.

- **Where.** There is no farmland layer in the map yet, so `arable_at` in `terrain.wgsl` guesses it from the land-cover colour and the relief: some green in the colour (not desert, not ice), not forest, below about 2,000 m, not steep, and (by the callers) not north of about 60 degrees. When a real farmland layer is baked (planned in the design notes), this function should read it instead.
- **Fields** (`field_tint`, in the terrain fragment shader, below about 1,600 m/px and only in the 3D view): a patchwork that multiplies the ground colour, so borders, roads and the owner's colour stay visible. By part of the world: blocks at all angles cut into strips with hedges (Europe and the default); very large straight fields (Russia); square sections, some with pivot-sprinkler circles (North America, Australia); small green or flooded paddies (South, East and South-East Asia); small plots in earth colours (Middle East, Africa). Fields keep off towns, roads, railways and rivers through the mask.
- **Farms.** About one plot in twenty in farmland gets a farmhouse (`farmstead`, drawn by the building pipeline, sharing `house_vertex` with town houses) in the regional colours.
- **Lone trees** stand here and there in farmland, by the same climate rules as the forests.
- **Not verified:** the guess of where farmland is was checked only on the made-up scene. On the real Natural Earth II colours it may farm steppe, savanna or tundra edges, or miss real farmland. The field pattern adds a projection inverse and several hashes per ground pixel; frame time on the M4 Pro is not measured.

## More rivers, and marshes (2026-11-09)

The owner asked whether rivers had been added, and for marshes.

- **More rivers (needs a map rebuild).** The render file now also takes Natural Earth's regional river supplements for Europe, North America and Australia (ranks 10 to 13): 9,456 river lines in all, against 1,473 features in the main file alone. Natural Earth has supplements only for those three regions, so the rest of the world keeps the main file's rivers. `fetch_sources.sh` downloads the three shapefiles from the Natural Earth repository on GitHub at the v5.1.1 tag (commit `9380cca`), each file with its own SHA-256, because the usual download site could not be reached from the machine this was written on. `render_extras.rivers` was run on the downloaded files there; the full `build_map.sh` was not, so run it and `validate_render.py` on the Mac. The client draws these streams (rank 10 and up) as a fourth river class below 1,800 m/px. The engine's map (river flags per cell) is unchanged: it still reads the main file only.
- **Marshes (display only).** The map has no wetland layer, so `crates/client/src/wetlands.rs` holds a hand-written list of 36 of the world's big wetlands as ellipses (centre, size, direction), placed from general knowledge. They are approximate, good to a few tens of kilometres at best, and far from complete. They are drawn into a small latitude-longitude picture at start-up and read by the shader (`wet_at`), with frayed edges. A marsh is dark wet ground with pools and winding channels of open water, reed clumps with a stunted tree now and then, and low swamp forest in the tropics; no fields and no farms. The Mesopotamian Marshes of southern Iraq are in the list for the Gulf slice.
- **To do properly:** marsh is a terrain class in the design notes (it blocks heavy vehicles). It needs a real wetland layer baked into the map from a licensed land-cover dataset; then the client should draw that and this list should go.

## Phase 7: rivers as ribbons, and bridges (2026-11-10)

The owner asked for rivers that look like those on the Hearts of Iron IV map, and for bridges. Display only.

- **Rivers are ribbons.** A river segment is now drawn as a capsule (`vs_river`, `fs_river` in `shader.wgsl`): round joins, a soft edge, lighter water along the middle and deeper toward the banks, over a soft darker bank drawn in a first pass under all segments. The water colour is a lighter blue than before. In the 3D view the ribbon keeps its painted look and only shimmers as it flows (it no longer borrows the sea's wave shading; marsh pools still do).
- **Rivers taper.** `river_segments` gives each segment the river's width at its two ends, packed into the segment's `dist` (rivers have no dashes, so it was unused). A river line starts at 30 % of its class width at its source and reaches full width 300 km downstream. The source is the higher end of the line (from the height raster), and only if no other river line ends there, because Natural Earth cuts one river into several lines. Tested.
- **Bridges.** `find_bridges` crosses every road and railway segment with the river segments (through a 20 km grid) when the renderer is built, and keeps one site per crossing and kind within about 400 m. Each frame the sites in view whose river and way are both shown become instances (at most 6,000), drawn by `vs_bridge` below 900 m/px: a road bridge is a deck with a parapet on each side, a railway bridge a deck between two steel girders, both on an abutment at each bank, level at the higher bank, long enough for the river's width there and the angle of the crossing. Roads and railways also take the water out of the mask where they cross, so the ribbon does not run over the road. Traffic rides a little higher so the deck does not swallow it. Tested.
- **Display only.** The design notes plan a real bridge layer (a dynamic bridge table that can be destroyed). These bridges are worked out from the drawing data and nothing in the simulation knows them; when the map bakes bridges, the client should draw those.
- **Not measured, not seen on the real map.** `find_bridges` runs once at start-up over all road, railway and river segments; its time on the real data (now 9,456 river lines) is unmeasured. Where the data's roads and rivers run side by side and cross repeatedly there may be clusters of bridges. The test scene's river has only three points, so the joins on real, winding rivers have not been looked at.

## Rivers and bridges in the simulation; rivers drawn like real rivers (2026-11-10)

The owner asked that the simulation know the rivers and bridges (crossing a river costs, bridges can be destroyed), and that rivers look like real rivers. This entry has engine changes (`meridian-core`, `map-builder`), not only display.

**What the engine already had:** river flags on cell edges, `CellCosts::river_extra_m` in the cell pathfinder, a bridge table (`combat.rs`: cell pair, condition 0 to 1000, laid or permanent, owner) and the commands `BuildBridge`, `DismantleBridge`, `DemolishBridge`. What was missing was any permanent bridge, and the pathfinder did not look at bridges.

- **Roads and railways on cell edges (needs a map rebuild).** `cell_edge_flag` gains `ROAD` (2) and `RAIL` (4) beside `RIVER` (1): the line passes the shared border of the two cells. `build_terrain.py` computes all three with one function (`edges_passed`, checked on a small hand-made case; the full build was not run here). No change to the file layout: `c_flags` was already one byte per edge. A river edge that a road or railway also passes is where a permanent bridge stood in 1990 (`Map::bridge_edges`). This is an approximation at cell scale: it says a river and a road both pass between the two cells, not that the road crosses the river at that exact spot.
- **Permanent bridges.** `World::seed_bridges` puts one bridge at condition 1000 on every such edge (owner: whoever holds the lower-numbered cell). `proving_ground::scenario_on_map` calls it. With a map baked before the new flags it adds nothing, so existing hashes do not change until the map is rebuilt; after the rebuild, state hashes of map worlds change, as they must (bridges are state).
- **Crossing costs.** `Map::find_cell_path_bridged` takes a "is this river edge bridged now" test; `World::find_cell_path` supplies it from the bridge table with a minimum condition. A river edge costs `river_extra_m`, or `bridge_extra_m` (new, default 0) over a usable bridge. `Map::find_cell_path` is unchanged in result (no bridges). The numbers and the condition a unit needs are rules.
- **Destroying.** `CombatState::damage_bridge` and `repair_bridge` are the primitives (for bombardment and engineers; the amounts are rules). `DemolishBridge` now works on permanent bridges: the unit must stand at the crossing on a bank its country holds (before, only the recorded owner could, which made no sense for a bridge that changes hands with the ground). A permanent bridge cannot be dismantled. Tested in `orders.rs`.
- **Not done:** units still move by `MoveUnit` between provinces with no travel time, so nothing in play pays the river cost yet; that is the rules phase's movement system, which should call `World::find_cell_path`. No bombardment rule damages bridges yet. The client has no button for demolition.
- **The client shows the simulation's bridges.** `CountryView::bridges` (bridges are landmarks; every country sees them) feeds the client snapshot. `client::bridges::link` ties each drawn bridge to the nearest simulation bridge edge within 25 km, and a bridge at condition 0 is drawn destroyed: no span, the river showing through the gap, a scorched stub at each bank. Drawn bridges with no simulation bridge near them stay as decoration. Tested.
- **Rivers drawn like real rivers.** The course wanders and the width swells and pinches with noise fixed to the ground, so banks are irregular; the water is a muted green-blue, deeper in the channel and paler in the shallows, with a sandbar here and there in a wide river, over a damp bank. Wider than before close up (about 11, 7, 4.5 and 3 pixels by class) and starting at 40 % of the width at the source. Trees are kept back from the banks so a river shows in a forest. The courses themselves are still Natural Earth's generalised lines.
- **Test noise:** `sim::tests::the_threaded_run_matches_a_plain_run_and_queries_never_change_state` failed about half the time on the build machine, with and without these changes (its `queries >= 500` check depends on thread timing). Not caused by this work; worth making robust.

## Units march from cell to cell (2026-11-10)

The owner pointed out that units move between cells, not provinces. Until now the only ground move was `MoveUnit`, which put a unit in another province at once, and the client ordered moves by province.

- **`Command::MoveToCell { unit, to }`**: the engine plans the cheapest route over the cell graph with `World::find_cell_path` (terrain, roads, rivers, and the bridges as they stand) and stores it on the unit as a `March` (the cells still to enter and the tick of the next step). Each tick, `World::advance_marches` moves every unit whose step is due into its next cell, in unit order, and times the next step: the step's cost in metres over the unit's pace, rounded up, at least one tick. The unit's province follows its cell. A new order replaces the route; an order to the cell the unit stands in halts it.
- **Rivers and bridges count.** A step across a river edge costs the river extra unless a bridge sound enough for the unit carries it. A unit that comes to a river with no usable bridge ahead (for instance because the bridge was blown after the route was planned) plans its route again from that bank: it goes round or fords. A unit already on its step across when the bridge goes still completes that step (the engine does not track where in a step a unit is).
- **The rules supply the numbers** through `Ruleset::march_profile(unit) -> MarchProfile { costs, min_bridge_condition, m_per_tick }`; the default is `None` (the unit cannot march). The proving ground gives every unit one placeholder profile (12 km an hour on open ground, a ford costs as much as 30 km, a bridge needs half condition) through a new `proving_ground::Rules`, which wraps the schedule; the client now runs that. Real profiles per unit type are rules data.
- **Hash and saves.** A march is hashed only when present, so worlds where nobody marches keep their hashes. Units' extra attributes gained a field, so `SAVE_VERSION` is now 2 and version 1 saves cannot be opened.
- **Client.** Right-click orders the selected unit to the cell under the cursor. Unit markers stand on the unit's cell, and an arrow runs from each marching unit to its destination as the simulation has it (through `CountryView::own_unit_place`). Proving-ground units now start in a cell.
- **Not done:** `MoveUnit` (the instant province move) still exists and tests use it; it should go once nothing needs it. Marching ignores other units, zones of control, supply and combat; those are the rules phase. The pathfinder's search memory is allocated per order (about 7 MB on the full map), which an AI giving many orders will want to reuse. None of this has run on the real map: the march test uses a 16 x 16 test map.

## Test units with NATO counters (2026-11-10)

The owner asked for some units with NATO counters to try the map out.

- **Unit type and size codes.** `UnitExt` gained `kind: u16` and `echelon: u8`, rules-defined codes the engine gives no meaning (0 = not set; hashed only when set, so older hashes hold). The proving ground defines 15 placeholder types (`unit_kind`: infantry, armour, mechanised, motorised, artillery, self-propelled artillery, recon, armoured recon, engineer, air defence, anti-tank, airborne, mountain, supply, headquarters) and five sizes (`echelon`: battalion to corps). In the real game a unit's type should come from its equipment and template; these codes are for testing.
- **Test formation.** `scenario_on_map` now gives every country that owns land `TEST_FORMATION`: 15 units, one of each type in a mix of sizes, spread over the cells of its first province (was one unit per country). That is about 15 units for each of the roughly 190 countries.
- **Counters** (`client/src/counters.rs`): a frame in the owner's colour, lightened, with the type symbol drawn in lines (infantry cross, armour oval, artillery dot, recon slash, engineer bridge, air-defence dome, anti-tank chevron, airborne wing, mountain peak, supply bar, headquarters staff, and their combinations) and the size above it (II, III, X, XX, XXX). They replace the plain squares; counters sharing a cell stand in rows of four. `counter_probe` (an example) draws the whole set to a PNG. The symbols follow the common NATO map-symbol conventions from memory and were not checked against the standard; the frame is always the friendly rectangle, coloured by owner, not by affiliation.
- **Views.** `CountryView::own_unit_type` gives the codes for a country's own units; with the `inspect` feature the client shows every unit's.

## Stacked counters, closer zoom, gentler tilt, a first real formation (2026-11-10)

The owner asked for counters that stack when zoomed out (as in Hearts of Iron IV), a closer zoom, less tilt close up, and a first German division with its brigades and chain of command (as in Hearts of Iron III).

- **Stacks** (`counters::stacks`, tested): each frame the client groups the counters of one country that would overlap on screen. The most senior unit (size, then headquarters) is on top; the stack shows the edges of the counters behind it and a plate with the number of units. Zooming in spreads a stack out again, down to units in the same cell, which always stack. A click picks the top unit; clicking again goes down the stack. This replaces the rows of four.
- **Camera.** The closest zoom is 20 m/px (was 100). The tilt when fully 3D is 34 degrees (was 48). The terrain exaggeration eases from 14 times at 300 m/px to about a third of that at the closest zoom, so hills do not tower over towns. Buildings, trees, traffic and fields were built for 100 m/px and coarser; closer than that they are simply larger, and the height and land rasters (about 1 km) are blurred.
- **Chain of command.** `UnitExt::parent` already existed; units now also have a `name`. `proving_ground::add_formation` places a named formation (each unit in the cell nearest its coordinates, given as whole Equal Earth metres so every machine picks the same cell; `Map::nearest_cell`) and sets who reports to whom. `PANZERDIVISION_1` is the first: I. Korps (Münster), 1. Panzerdivision (Hannover), Panzergrenadierbrigade 1 (Hildesheim), Panzerbrigade 2 (Braunschweig), Panzerbrigade 3 (Nienburg) and Artillerieregiment 1. It is written from general knowledge of the Bundeswehr's Army Structure 4, not checked against a source, and stops at brigades. Its owner is whoever holds the cell at Münster on the 1990 map.
- **Client.** Selecting a unit draws a line up to the unit it reports to (and on up the chain, yellow) and down to each unit under it (green), and the side panel lists both as links. Units are listed by name.
- **Brigades and divisions.** As in the design notes, the brigade is the unit on the map; a division or corps is a headquarters unit that brigades report to. Nothing yet follows from the chain (no orders passed down, no command range, no bonuses): that is rules work.
- **Saves.** Units gained three fields in all today (type, size, name), so `SAVE_VERSION` is 3.
- **Not checked on the real map:** `scenario_on_map` could not be run here (no baked map), so the placement of the division at the German towns is untested; `add_formation` skips the formation if the map has no cell within 60 km of Münster.

## Map redesign decided: hexagon levels on a globe, five map scales (2026-11-10)

A design decision, not built. The owner is starting a second big rewrite of the map and the graphics.

- The map is shown as military maps at 1:1,000,000, 1:250,000, 1:100,000, 1:50,000 and 1:25,000, with transitions while zooming; the graphics will probably mix 2D and 3D.
- The irregular cells are replaced by hexagons on the globe: H3 level 5 (about 10 km) always on, level 7 (about 1.4 km) where units fight, level 8 (about 530 m) for local tactical play. Provinces stay as the political layer.
- The simulation uses whole-metre positions on the sphere, and the client draws a globe.
- The design, the reasons, the data found for each scale and the long list of open points are in `docs/design-notes.md` section 25 and `docs/data-sources.md`. The build guide's section 3A is marked as superseded in outline.
- Today's map (cells, flat 3D view) remains the working one until its replacement exists.

## OpenStreetMap for the fine map scales (2026-11-10)

Decided by the owner: OpenStreetMap is the source for the map scales from 1:100,000 to 1:25,000. Data derived from it stays in files of its own, offered under its licence (ODbL), with "© OpenStreetMap contributors" on screen; the game's code and other data are not affected. It shows the world of the 2020s, not 1990. Recorded in `docs/design-notes.md` section 25 and `docs/data-sources.md`. This settles the question left open on 2026-11-09.

## Map coverage: world base to 1:250,000, fine scales as regional packs (2026-11-10)

Decided by the owner after seeing the sizes: the base is the whole world at 1:1,000,000 and 1:250,000 with level-5 and level-7 hexagon terrain (5 to 20 GB, most likely about 10 GB; estimates). The scales from 1:100,000 to 1:25,000 and level-8 terrain come later as regional packs, the Persian Gulf first; everything is built in regional pieces from the start. The whole world at every scale was estimated at 50 to 180 GB with a build that needs about 46 GB of memory and up to 1 TB of disk, and was set aside. Sizes and reasoning are in `docs/design-notes.md` section 25.

## A plan for the map rewrite (2026-11-10)

`ENGINE_BUILD_GUIDE.md` section 9 lays the map redesign out as phases M0 to M7, each with a done test: check the tools and sources; the level-5 hexagon world in the engine, headless; rivers, roads and units on hexagon edges; the globe client; the base map at 1:1,000,000 and 1:250,000; level-7 hexagons where units fight; the first regional pack (Persian Gulf) with the three fine scales; level-8 tactical play. The order proves the riskiest idea first (hexagons with whole-number positions on the sphere) with no graphics, and today's map stays the working one until the globe client passes. Not started.

## Brigade is the smallest unit; one hexagon level, H3 level 6 (2026-11-10)

Decided by the owner: nothing below the brigade is on the map. With that, one hexagon level is simulated, everywhere and always on: H3 level 6 (about 6.4 km across, 36 km², about 4.1 million land hexagons by estimate), where a brigade holds about one hexagon in attack and two in defence. This replaces the three levels (5, 7, 8) chosen earlier the same day; the open-and-close of finer hexagons, and plan phases M5 and M7, are dropped. Economy and population stay coarser than the hexagon. Whether the scenery-only scales below 1:250,000 are still wanted is open. Recorded in `docs/design-notes.md` section 25 and `ENGINE_BUILD_GUIDE.md` section 9. Work on the rewrite is on the branch `map-rewrite`; `main` keeps today's map.

## Map rewrite M0: tools and sources checked (2026-11-10)

- **Hexagon grid in the builder:** the official `h3` Python package, version 4.5.0 (C library 4.5.0), Apache License 2.0. Installed and used; added to the builder's requirements.
- **Hexagon grid in Rust:** `h3o` 0.11.1, BSD-3-Clause, a from-scratch Rust implementation ("100% Rust (no C deps)" is its stated design goal). Seen on crates.io through `cargo info` and on its GitHub page; its last commit date and which H3 version it tracks could not be read. Not yet a dependency.
- **The engine does not need a hexagon library.** With one level always on, the builder bakes everything the simulation uses: each hexagon's H3 number, its centre in whole metres, its neighbours and the distances. The client may use `h3o` for outlines and for finding the hexagon under the cursor (display only).
- **Same results on every machine.** The engine's distances are integer arithmetic on the baked whole-metre centres (`meridian-core/src/sphere.rs`, tested against the haversine formula; within a few metres up to 5,000 km). The builder uses floating point to place centres and decide the coast, so, as with today's map, the baked file may differ between machines in rare last-digit cases: build once, pin the checksum.
- **Licences.** Copernicus elevation: the collection page says GLO-30 and GLO-90 are "available worldwide with a free license" and gives the notice to show when distributing ("© DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved", and "produced using Copernicus WorldDEM-30" for adapted data); the licence document itself, and so an explicit word on commercial use, was not read. Still unread: GHSL built-up, VMAP0 and VMAP1 from the NGA itself, a 100 m land-cover product. These matter for M4, not M1.
- **Build machine:** the owner's free disk and memory are not known yet; needed before M4.

## Map rewrite M1, first step: the land as level-6 hexagons (2026-11-10)

On the branch `map-rewrite`. Not the whole of M1: the hexagons are not yet joined to provinces, terrain or the map file, and the engine does not load them yet.

- **`crates/map-builder/build_hexes.py`** decides which H3 level-6 hexagons are land and writes each one's H3 number, its centre in whole metres on the sphere, its land neighbours and the ground distance to each. Rules: a hexagon is land when its centre is on land, or 3 of 7 sample points are, or it touches land and holds a town (230 such hexagons, so a port on a bay stays on land: Kuwait City and Tromsø were in the sea without this); every piece of land of 3 km² or more keeps one hexagon (1,585 small islands). Antarctica (south of 60° S) is left out for now.
- **`validate_hexes.py`** has a self-test on a made-up island and checks a real run: the graph is the same from both sides, the area is within 1 % of the source, known cities are on land and the distances between them are right. A run on Natural Earth's land passed (numbers in `docs/budgets.md`).
- **`meridian-core/src/sphere.rs`**: ground distance between two whole-metre places on the sphere, by integer arithmetic only (halve the angle five times with integer square roots, then a short series). Tested against the haversine formula: within a few metres up to 5,000 km, and correct across the 180th meridian. The builder computes its distances the same way, so file and engine agree.
- **`runner probe`** takes a cell count; the stub world at 3,686,556 cells is measured in `docs/budgets.md`.
- **Next in M1:** provinces and countries by hexagon centre (with the rule that every state keeps a hexagon), terrain and population per hexagon, sea zones, the new map file and loader, and the engine's map tests on hexagons.

## Map rewrite M1: the world as hexagons, end to end (2026-11-10)

**Done.** The builder now makes the whole world as H3 level-6 hexagons and the engine loads and uses it: `build_hex_cells.py` replaces `build_cells.py` in `build_map.sh`, the map file is version 2 (data-formats.md), and `meridian-core` measures distance over the globe.

- **Whose hexagon.** A hexagon belongs to the region its centre is in; a coastal hexagon whose centre is in the sea goes by the land under its sample points, then by the nearest region. Every region keeps at least one hexagon. Provinces inside a region are assigned as before. Hexagons are numbered by state, region, then H3 number.
- **Area.** A hexagon does not follow the coast or a border, so its recorded area is its share of its region's true area. Countries and provinces keep their real areas (the Vatican is not 36 km2).
- **Distance.** On a globe map all cell and strategic-graph distances are great-circle distances in integers (`sphere.rs`). The route search is guided by the straight line through the Earth, which can never overestimate; baked distances carry a 2 m margin so rounding cannot break that. The small flat test maps still work the old way (a map is a globe map when it has places).
- **Range queries** on the globe use boxes in three dimensions, kept in a sorted list (no hash map). There is no seam at the 180 degree line or the poles.
- **The engine has no hexagon library.** It sees cells, neighbours and places; the H3 number is only a name.
- **Terrain step** rewritten to work 200,000 cells at a time (same results on a test region as before the rewrite).

**Done test.** Same file from two builds: yes. Hexagons cover the land within 1 %: +0.37 %. Eleven cities land in the right country and their distances are within 1 % (test `cities_are_where_they_should_be_on_the_globe`, including across the 180 degree line and north of the Arctic circle). Path queries within the (provisional) budgets: yes. **Not done here: the same hash on macOS and Linux.** The path fingerprint on Linux with stand-in elevation was `8a14cae6b1244950`; it has to be compared on the Mac with a real build on both.

**Known gaps.**
- Elevation here was a stand-in (zero everywhere) because the ETOPO file cannot be downloaded on the machine this was written on. The real terrain step has only been run on the Mac before the rewrite. First real build: on the Mac.
- Budgets for the hexagon world are provisional (budgets.md).
- The flat client cannot show this map: `build_map.sh` no longer writes the render file (3.7 million outlines are too much for it). The globe client is M3. Until then the map is headless on this branch; `main` still has the old map and client.
- Population and economy per hexagon are not there (planned per province).
- `sim::tests::the_threaded_run_matches...` still fails now and then on a busy machine (timing in the test, not the map).

## Map rewrite M2: rivers, bridges and units on hexagons (2026-11-10)

Started on the owner's word before M1's last check (the same hash on macOS and Linux) could be run; that check is still owed and needs the Mac.

- **Rivers run along hexagon sides** (`build_rivers.py`). A river line becomes the cheapest chain of sides, on the web of hexagon corners, from the corner nearest its start to the corner nearest its end, where a side costs more the farther it lies from the line. A chain cannot have a gap. The river flag on a cell edge now means "going from one of these cells to the other crosses the river" (before, it marked edges the river passed through, which let a unit cross inside a cell for free). Lake centre lines are included so a river is not broken at a lake.
- **Bridges** are river sides that a road or a railway passes, as before: 17,663 on the world.
- **Setting up by latitude and longitude.** `sphere::place_from_micro_degrees` turns millionths of a degree into a place with whole-number sine and cosine (power series), so a unit lands in the same hexagon on every machine; a machine's own sine can differ in the last digit. The test division is now given by its towns' coordinates.
- **Search memory is kept in the world** between route searches (`SearchMemory`). It is not state: not saved, not hashed, never changes a result (the year's hash is the same with and without it).
- **Land connection table.** On load the map numbers its connected pieces of land; a route between two pieces is refused at once.

**Done test.** A unit at the Rhine bridge nearest Cologne crosses in 2 ticks; with the bridge demolished it goes round by the next crossing and takes 4 (`hex_world.rs`). Ten great rivers (Rhine twice, Danube, Vistula, Volga, Tigris, Euphrates, Nile, Mississippi, Yangtze): with fording forbidden and no bridges, the route between the banks is 6 to 50 times the open one, so none has a gap on that stretch. A year of the proving ground with marching units replays to the same hashes (budgets.md); there is no tick budget to hold it against yet. Two builds of the map give the same file.

**Known limits.**
- Where a road follows a river bank, the hexagon sides make it cross the river several times, and each crossing counts as a bridge. So there are more bridges along such stretches than in reality. A real fix needs bridge points (where road and river lines truly cross) kept apart from the road flag.
- Where a tributary joins, or a river reaches the sea, the chain ends at the nearest corner; a one-side gap there is possible and is not tested.
- Lakes are still land hexagons (as they were land cells).
- Elevation was still the stand-in. Real terrain will change routes and the fingerprint.

## Fine map scales: working assumption (2026-11-10)

Asked whether 1:50,000 and 1:25,000 should be scenery only or dropped, the owner answered "do all" to a list that included the question, which does not settle it. **Assumed until told otherwise:** nothing finer than 1:250,000 is built now, and M6 (the OpenStreetMap packs) stays on hold. Nothing in M2 to M4 depends on the answer.

## Map rewrite M3, first part: the globe client (2026-11-10)

Started on the owner's word ("do all the steps") with M1's and M2's Mac checks still owed.

**What is there.**
- **Camera** (`globe/camera.rs`): looks straight down at a point from a height, north up, 30 degree field of view. Dragging keeps the ground under the cursor; the wheel zooms about the cursor. From the whole globe down to 20 m a pixel. All camera arithmetic is in 64-bit floats (display only; nothing here feeds the simulation).
- **Camera-relative positions.** Hexagon triangles are stored relative to their tile's anchor, and each frame the anchor's position relative to the camera is worked out in 64 bits and sent as one small number per tile. A test does the graphics card's 32-bit arithmetic by hand at the closest zoom and lands within 0.05 pixel.
- **Two ways of drawing** (`globe.wgsl`). Far: one triangle over the screen; each pixel finds where its line of sight meets the Earth and looks the hexagon up in a picture of the world (8192 x 4096, one cell number per pixel, made at start-up). Near (under 1 km a pixel): hexagons as triangles, in tiles of one level-3 hexagon (343 cells), made as they come into view and kept on the card. Colour comes from a per-cell table and a per-province palette, so a map mode is still only a list of province colours.
- **Lines without line geometry.** Each hexagon is a fan of six triangles; each triangle knows what runs along its side (province border, country border, river, coast), and the pixel shader draws the line from how close the pixel is to the side. Rivers therefore show exactly where the simulation has them.
- **Picking**: pixel to ground point to H3 number to cell. `h3o` 0.11.1 (BSD-3-Clause) is used for this and for hexagon corners, in the client only. The engine still has no hexagon library.
- **The game shell** (`globe_app.rs`): the flat client's panels, time controls, save and load, counters, stacks, command lines and march orders, on the globe. City names come from the map's sites.

**Done test.** Pixel to ground and back: exact to 0.0001 pixel in the camera tests; on the real map a hexagon put on screen is picked again at the equator, 70 degrees north and both sides of the 180th meridian, from the whole globe to 30 km across. Jitter: the 32-bit test above; to be looked at on the Mac. Frame time on the build machine: **not measured** (no graphics card here; budgets.md has the commands). Selecting, ordering and watching a unit march: built, and the window was opened and drawn under software rendering with the division at Hannover, but nobody has clicked in it yet.

**Not done yet (the rest of M3).**
- Borders and coasts follow hexagon sides; the real border lines and relief are not drawn.
- The camera cannot tilt.
- March arrows are straight lines on screen, not the route.
- Terrain, roads, railways and bridges are not drawn on the globe.
- The flat client and the old builder steps are still in the tree. They are not retired until the owner says so: the trees, buildings, traffic and landmarks made for the flat map live there, and the globe has none of them.

## Map rewrite M3, second part: roads, terrain and routes on the globe (2026-11-10)

The owner built the hexagon map on the Mac and reported that everything works (the `map-bench`, `map-run` and frame-time numbers have not been sent yet, so the budgets stay provisional and the macOS-against-Linux comparison is still open).

- **Roads and railways** are drawn from a hexagon's centre to the middle of each side they pass, so they join centre to centre, the way units move. They are a diagram of the movement network, not the real road lines (those come with the map sheets in M4). They appear below about 450 m a pixel.
- **Bridges** show as a pale deck across the river side; a destroyed bridge gets a red cross (from the simulation's bridge table).
- **Terrain per hexagon.** The per-cell table now also carries terrain and how built-up a cell is. The political colouring lets the ground show through a little and greys built-up hexagons; the terrain mode colours each hexagon by its own terrain (before, a whole province took its commonest terrain).
- **March routes** are drawn hexagon by hexagon. The read interface gained `CountryView::own_unit_route`.

**Still not done in M3:** real border lines and shaded relief, a tilting camera, and retiring the flat client (the owner's call).

## Real lines on the globe (2026-11-10)

The owner asked why roads and railways were not curved and expected something closer to a real map. The hexagon diagram (roads from centre to centre) shows how units move, not what the ground looks like.

- **A second layer of real lines** is drawn over the hexagons: coastline, the 1990 borders, rivers, roads and railways as they run on the ground (`build_globe_lines.py`, `meridian-globe-lines.bin`, data-formats.md). With it, the diagram of roads is off by default, hexagon-side coasts are hidden, hexagon-side borders go faint, and the river sides (where a unit really crosses) show only close up.
- **Exact at any zoom.** A piece's ends are whole metres from the Earth's centre; the shader takes the camera's whole-metre place off in integers before turning anything into 32-bit floats.
- **Source and limits.** Natural Earth at 1:10 million: the main roads and railways of about 2010, no streets, no buildings, no relief. Built-up ground is shown per hexagon from Natural Earth's urban areas.
- **What "like a real map" still needs** (not built): shaded relief from elevation; land drawn from real outlines instead of whole hexagons; and OpenStreetMap for streets, buildings and land use at close zoom. OpenStreetMap cannot be downloaded on the machine this is written on, so that builder has to be run and checked on the owner's Mac. The earlier assumption that the OpenStreetMap packs were on hold is withdrawn: the owner expects them.

## Shaded relief and the first OpenStreetMap pack, written blind (2026-11-11)

The owner chose to have the detail builder written without my being able to run it on real data (OpenStreetMap and the elevation file cannot be downloaded where this is written), starting with Italy.

- **Shaded relief** (`build_globe_relief.py`): a hillshade of the whole world from the ETOPO grid the terrain step already keeps, 8192 by 4096, lit from the north-west, blended into the land colour in both the far and the near drawing. Worked in bands of rows so it needs little memory (in one piece it ran out of 7 GB).
- **Regional pack** (`fetch_pack.sh`, `build_pack.py`): downloads a Geofabrik extract, keeps and exports the wanted features with the `osmium` command-line tool (installed with Homebrew; it does the heavy reading in C++ and needs no Python package that might not exist for Python 3.15), and writes a lines file (motorways to unclassified roads, railways without sidings, rivers and canals; tunnels left out; simplified to about 8 m) and a ground-cover picture (water, woods, built-up, industry) at about 80 m a pixel in 2048-pixel tiles.
- **Ground cover is a picture, not polygons.** Its size depends on the region's area, not on how detailed the mapping is, and the shader looks it up from the pixel's longitude and latitude.
- **Inside a pack's outline the world's coarse roads, railways and small rivers are hidden**, so they are not drawn twice.
- **Buildings and streets are not in this first cut.** Residential streets and single buildings would multiply the size several times and are below what a brigade-level map shows; built-up areas are shown as areas.
- **Not 1990.** The pack is the present-day map. A 1990 look would need hand correction or an older source.
- **Licence.** ODbL: packs stay in their own files and the credit is shown in Settings and in attributions.md.

**Tested here only on made-up data** (an invented extract around Rome in the format `osmium export` writes, and invented elevation): the builders run, the client loads and draws everything. **Not tested:** the `osmium` commands themselves, real Italy's size and build time (the numbers in the script's comments are estimates), memory on the real extract, and how the result looks. The first real run is the owner's.

## First real Italy pack: what the owner saw and what changed (2026-11-11)

The owner built the map and the Italy pack on the Mac. Real numbers: Italy extract 2.1 GB; pack lines 117 MB (4.68 million pieces), ground cover 143 MB (34 of 56 tiles); 16.6 ms a frame at the closest zoom (the display's refresh rate, so the real cost is lower).

Faults found and fixed:
- **`build_globe_lines.py` failed on the Mac**: it reads `ne_10m_land`, which `fetch_sources.sh` never downloaded (the file was only on the machine this was written on). Now fetched and checksummed. Because the build stopped there, the Mac had no world lines and no relief, and the client fell back to the hexagon diagram of roads.
- **Land colour ran into the sea** (whole hexagons past the coast, the Venetian lagoon). The pack's ground cover now knows land from sea, from OpenStreetMap's coastline (land polygons from osmdata.openstreetmap.de, 0.9 GB, downloaded once), and the client paints sea wherever the real map has sea. Real land with no land hexagon under it is drawn as bare ground.
- **The country colour covered the map close up.** Where a pack has detail, the ground now turns into a printed map as you zoom in: paper-coloured ground, green woods, pale towns, shaded relief, and only a faint tint of the country (full colour from 400 m a pixel outward, 12 % at 60 m a pixel and closer).
- **Rivers and roads still followed hexagon sides.** The river sides and the diagram are now shown only with "Movement network" switched on; it is off whenever real lines or a pack are loaded.
- Inside a pack the world's coarse coastline is hidden too; only the 1990 borders are kept from the world file.

Not tested on real data: the land-polygon download and its reading (the link and the file layout are from memory).

## The globe's look: a stylised landscape, as Hearts of Iron IV does it (2026-11-11)

The owner found the first Italy pack "still not good" and asked how Hearts of Iron IV does its map. It uses no survey data: a small hand-painted world, a lit surface from a height picture, tiled ground textures blended softly, a wash of country colour strongest at the borders, very few stylised lines, and towns as objects. Ours was the opposite: flat colour with every real road and land-use outline on top. The owner asked for those five things, somewhat stylised. This replaces the "printed military map" direction of section 25 for the globe.

What changed, all in the pixel shader's `ground` function (one place for the far and the near drawing):
1. **Lit terrain.** A height picture of the world; the shader works out the slope and lights it from the north-west, with heights exaggerated more from far away. Fine roughness is made up from noise, stronger where the real slope is steep, because the height picture (4.9 km a pixel) is too coarse close up.
2. **Painted, textured ground.** The base colour is Natural Earth II's painted land picture. Where a pack knows the ground cover it is read smoothly between its pixels (curved edges instead of 80 m squares) and drawn as textures: mottled dark woods, a patchwork of fields on open ground, towns as blocks of roofs with streets, lighter water toward the shore. Rock on steep slopes and snow on the heights come from the heights.
3. **Few lines.** World file: great rivers and coasts from the whole globe, highways and rivers from a country down, roads from a region down. Packs: motorways from far off, trunk and main roads and railways from a region down, secondary roads close up; tertiary and unclassified roads and canals only with a setting, at the closest zoom. Roads have a dark rim and get bolder close up.
4. **Country colour as a wash.** Each cell knows how many hexagons it is from the nearest country border (counted once at start-up, smoothed between neighbours in the triangles). The wash is 55 % at a border and falls to 15 % inland; from 2.8 km a pixel outward it is a plain political map.
5. **Towns as objects**, as far as a camera that looks straight down allows: roof blocks, not land-use outlines. Models that stand up need the tilting camera, which is not built.

Also: hexagon outlines are off by default (a setting), province borders show faintly and only close up.

**Honest limits.** All of this was tuned on made-up heights and a made-up pack; the real Alps, the real colour picture and real Italian towns have not been seen with it. The patterns are generated, not drawn by an artist. Outside a pack there is no ground cover, only the painted colour and the lighting.

## Roads and railways as links with levels (2026-11-11)

The owner: roads and railways must be objects, because they will be supply routes, and they must differ in capacity, as in Hearts of Iron IV. Work is on Italy only for now.

- **The object is the link**: the road, and the railway, between two neighbouring hexagons. Each has a level from 1 to 5, its capacity class, baked per cell edge (`c_road`, `c_rail`; map version 3). A link is addressed by its two cells. This is the engine's primitive; what a level carries, what supply is and how it flows are rules and belong to the gameplay backlog. `proving-ground` has a placeholder table of tonnes a day per level so the client can show something.
- **How a line becomes links** (`build_network.py`): it is walked in steps under a kilometre and every change of hexagon is a link; a link keeps the best level of the lines on it. This replaced the polygon test that set the old road and rail flags; the flags are now set exactly where a level is above 0.
- **Levels.** Road: local 1, secondary 2, main 3, trunk 4, motorway 5. Rail: 1, plus one each for main line, two or more tracks, electrified, 200 km/h or more. The world comes from Natural Earth (road 1 to 4 by its road type; rail 2 to 4 from two undocumented codes read as 1 = yes). Italy comes from OpenStreetMap through the pack's new `network.bin`, so the links there match the roads drawn.
- **A pack now feeds the simulation.** `load_map_file` lays `packs/*.network.bin` over the map and mixes the pack into the map's checksum. The lookup uses whole numbers only (the pack lists its region's hexagons), so every machine gets the same map. The file stays a separate ODbL database.
- **Supply route query**: `Map::find_link_route(from, to, road or rail, level_now, ...)` returns the route whose weakest link is strongest, and the shortest among those, by trying level 5 links only, then 4 and up, and so on. `level_now` is the hook through which the world will apply damage and upgrades (not built yet).
- **Client**: the "Supply network" overlay (key N) draws each link wider and stronger in colour by level; the panel lists the selected hexagon's links, and with a unit selected shows the road and rail supply routes from the unit to the hexagon, drawn on the map, with the weakest level.

**Not built:** damage, repair and upgrading of links; named routes (a motorway or a line as one object made of links); the supply system itself; movement cost by link level (movement still uses "both cells have a road"). **Known:** a route's level is often set by the links at a city's own hexagon (Hamburg to Munich comes out level 2 on Natural Earth data); whether the first and last stretch should count is a supply-rules question. Italy's levels are today's network, not 1990's (the high-speed lines are mostly later). The real Italy network file has not been built yet: it was tested on made-up data.

## Close zoom as a drawn sheet (2026-11-11)

The owner showed a map from Flashpoint Campaigns (Aiterhofen) as the look wanted: one olive ground, hills as stepped shapes, trees and houses as little symbols, bright outlined rivers, three kinds of road, no country colour. Decided: that look close up, the lit landscape with the country wash from countries and continents outward; houses are generated symbols, not real footprints.

- **The sheet** takes over below 220 m a pixel and is complete under 90 (`sheet` in `globe.wgsl`). One olive ground with a faint patchwork; no country wash, no province lines; the border line stays.
- **Symbols larger than life.** A tree is about 9 pixels and a house about 7 whatever the zoom; their size on the ground doubles each time the zoom would make them smaller. Trees stand where a pack has woods, houses (red roofs, some pale) where it has built-up ground, larger grey ones on industry. Each has an inked rim.
- **Water** is one bright blue with a dark line at the shore.
- **Height as steps** of 40 m with an inked edge, a shadow below and a light rim above, left out where steps would be under 7 pixels apart or the ground is flat. With the present height picture (4.9 km a pixel) these are broad shapes, not the small hills of the model map: that needs finer heights for Italy (Copernicus 30 m), which the owner has not yet agreed to download.
- **Roads** (the owner: "way too cluttered"): orange motorways, yellow main roads, pale minor roads, each with a dark rim; railways black and white by turns. Main roads and railways now appear only below 300 m a pixel and secondary roads below 70; before it was 600 and 150.
- **Town names** are white with a dark edge, large towns in capitals.

Seen only on made-up data. Not done: the kilometre grid and road numbers of the model map, streams as outlined lines, real small hills.

## The sheet's small hills, grid, numbers and streams (2026-11-11)

The owner asked for all of what the drawn sheet still lacked. Written blind again: seen only on made-up data here, to be run on the Mac.

- **Fine heights for a pack.** `fetch_pack.sh` downloads the Copernicus DEM GLO-90 squares over the pack (one file per degree, about 5 MB each, kept in `data/source/copernicus-dem-90`), and `build_pack.py` writes `<name>.heights.bin`: metres on a grid of 0.002 by 0.0015 degrees (about 160 m), each source pixel first averaged with its eight neighbours. GLO-90 and not GLO-30: a ninth of the download, and the sheet's steps are no finer than 20 m anyway. Where a pack has heights they replace the world's in the shader, for the lighting and the steps.
- **Steps set by the zoom**, 20 m up to 30 m a pixel and doubling with each doubling of the view. Where the ground is too steep for the edges to stay 5 pixels apart, every second edge is dropped, and again. A first version chose the step from each pixel's own slope; on rough ground neighbouring pixels chose differently and the hills came out as speckle.
- **Kilometre grid**: faint lines every kilometre (every 2 km above 45 m a pixel), a setting, on by default on the sheet. The lines follow true meridians and parallels, with the spacing between meridians set per band of latitude; it is a reading aid, not a military grid.
- **Labels from the pack** (`<name>.labels.bin`): cities, towns and villages from OpenStreetMap's place points, road numbers from `ref` on motorways and main roads, river names. One label per name and level-5 hexagon. The client shows each kind below its own zoom, at most 140 at once, none over another. The world's own city names are drawn only where no pack label was placed.
- **Streams** (`waterway=stream`) are drawn as thin outlined lines below 70 m a pixel. They are drawing only: no stream becomes a river side on the hexagons.
- **A one-layer texture array read as empty** in the shader under the software renderer here (wgpu 30, Vulkan, llvmpipe): the made-up pack's heights fit one tile and every height came back zero. The renderer now always stores at least two layers. Not known whether Metal has the same fault; the padding costs one blank tile and is harmless.
- **`fetch_pack.sh` re-exports** when its `EXPORT` number is newer than the one remembered beside the export, since the filter gained streams and place points.

Not checked on real data: the elevation file names and address, the place-point export with osmium, how real Italian relief reads as steps, label crowding in the Po plain. Only one pack's heights are drawn at a time, as with cover.

## The supply route overlay was mistaken for a road (2026-11-11)

The owner's screenshot showed a dead-straight orange line running for a hundred kilometres across real, curved roads, and asked that roads, railways and bridges follow the real lines on the ground. They were real roads (the OSM pack lines already follow the ground); the straight line was the debug overlay for the selected unit's supply route, drawn as one straight segment per hexagon on its path. It is really a statement about the hexagon-edge road and rail network, not a road itself, so:

- It is now drawn only with "Supply network" (the hexagon-edge view, `N`) turned on, and as a thin dashed line rather than a thick solid one, so it reads as the debug line it is.
- A standing bridge is now drawn as a small pale deck crossing the river at right angles, at every river crossing that still has one, not only (as a red cross) where the bridge is down. Seen on made-up data only; not yet checked on the real Italy pack.

Roads and railways themselves were already the real OpenStreetMap lines, curved as drawn; nothing changed there.

## Fewer rivers, and city, region and country outlines (2026-11-11)

The owner's screenshot showed the ground choked with blue lines, and asked that rivers, roads, railways and bridges follow the real lines on the map (they already did; see the entry above), and for city, region and country areas to be outlined, the way Victoria 3 and Hearts of Iron IV do.

- **Rivers.** OpenStreetMap tags countless small, unnamed watercourses as `waterway=river` in hilly country, and `build_pack.py` was drawing every one of them as a major river, visible at any reasonably close zoom. An unnamed river or canal is now drawn as a stream instead (close zoom only, under 70 m a pixel): a real, named river keeps its place, the typical nameless hillside gully does not. Seen only on made-up data, where it cut forty watercourses to one; real Italian terrain, and how many named rivers survive, is unchecked.
- **A city's own outline** (`globe.wgsl`, `ground()`): where a pack's built-up ground crosses the half-way mark, a dark ink line now follows it, the width always about a pixel (from `fwidth` of the blended fraction, not from the zoom). It fades out on the drawn sheet, where the houses already say the same thing, and shows nowhere without a pack.
- **A region's border** (the hexagon-edge province line in `fs_tile`) is stronger and in its own colour (a dark red-brown) rather than a faint darkening, and now stays visible out to a region's own scale (to about 3,200 m a pixel) instead of fading by 600. It still follows hexagon edges, a coarser shape than the real admin-1 line engineering M4 may draw later, not the province's true outline.
- **Country borders already draw from the real 1990 line** (`build_globe_lines.py`, style 1); nothing changed there.

