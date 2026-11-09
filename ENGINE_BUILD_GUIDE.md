# Meridian: Engine Build Guide

As of 2026-11-09. Draft for review. Gameplay-facing design decisions from the 2026-11-08 and 2026-11-09 discussions are in `docs/design-notes.md`; this guide lists the engine work they add in section 3B and in each phase. This guide covers the **engine only**. Economy, military, politics, diplomacy, AI behavior and the 1990 content are out of scope until the engine phases pass their tests (see section 7).

Companion document: "1990 Grand Strategy Engine: Build Plan by Phase" (the phase plan). This guide turns that plan into working instructions.

## How to use this guide

- Work one phase at a time, in order. Do not start a phase until the previous phase's **done test** passes.
- Write the done test first, then build until it passes.
- If a task is about gameplay (how an economy works, how combat resolves, what a country wants), stop. It goes in the gameplay backlog, not the engine.
- This guide is written to be followed by a person or handed to an AI coding assistant. Either way, the ground rules in section 1 apply to every line of code.

---

## 1. Ground rules

### 1.1 What the engine is

A purpose-built simulation core for this one game. It must:

1. Hold the world state.
2. Advance time (one tick = one in-game hour).
3. Take orders as commands.
4. Run rules in a fixed, declared order.
5. Load its content from data files.
6. Answer map questions (adjacency, paths).
7. Stay reproducible: same seed + same commands = same state.
8. Expose its state to a UI and an AI.

It is not a general game engine. It does not do audio, physics, networking or asset pipelines.

### 1.2 Determinism rules (apply everywhere in simulation code)

- No floating point in any value that feeds back into the simulation. Use integers or the fixed-point type. Floats are allowed only for display.
- All randomness comes from the one seeded generator owned by the world. Its state is part of the saved state and the state hash.
- No system clock, no thread IDs, no memory addresses, no environment lookups inside the simulation.
- Never iterate a hash-based collection in simulation code. Use arrays indexed by ID, or sorted vectors.
- Sorting must be stable and fully specified (ties broken by ID).
- Systems run in a declared order. Parallel systems may only touch disjoint data, and must give results identical to the single-threaded run.
- Every file the simulation reads has a fixed field order, fixed byte order (little-endian) and a version number.

### 1.3 Separation rules

- The simulation core knows nothing about graphics, windows, files or the network. It receives bytes or already-parsed data.
- The only way to change the world is a command applied at the start of a tick.
- The UI and the AI read state through a read interface and give orders through the command interface. They never touch engine internals.
- The engine never depends on the proving-ground ruleset (section 5).

### 1.4 Working habits

- Every phase ends with a tagged commit and an entry in the decision log (what was decided, why, what was rejected).
- Every performance claim comes with a measurement. Budgets are written down in the phase where they are set.
- Prefer boring and obvious over clever. This project will take a long time; future you has to read it.

---

## 2. Workspace layout

```
meridian/
  crates/
    meridian-core/        the engine: state, clock, commands, scheduler, map queries
    meridian-data/        data schemas, loader, validator, baker, save/replay formats
    map-builder/     offline tool (Python): real boundary data -> baked province and cell map
    proving-ground/  throwaway test ruleset (never a dependency of meridian-core)
    runner/          headless program: run a scenario, print hashes, benchmarks
    tools/           replay player, state-hash diff, benchmark harness
    client/          2D renderer and UI shell (Phase 7)
  data/
    scenarios/       authored text scenario files
    rules/           authored text rule/config files
    baked/           generated binary files (not hand-edited, not committed)
  docs/
    decisions.md     decision log
    budgets.md       time and memory budgets
    data-formats.md  file format specs
```

Dependency direction: `client`, `runner`, `tools` and `proving-ground` depend on `meridian-core` and `meridian-data`. `meridian-core` depends on nothing in this workspace except what it must (keep it small). `map-builder` depends on `meridian-data` only.

Build settings for the M4 Pro: build natively for Apple Silicon (the `aarch64-apple-darwin` target), and use a release profile tuned for speed (link-time optimization, a single codegen unit). Keep debug builds at light optimization so tests run fast.

---

## 3. Data formats

### 3.1 The short answer

JSON is not the format to build around, but text parsing speed is not the real issue. A text file is parsed once, at load time. The simulation never reads files while ticking. Where text formats hurt is with **large data** (the province map, saves, replays), where size and parse time add up, and with typing (no comments, no integers vs floats guarantee, no schema).

So use **three formats for three jobs**, and never put a text format on a hot path:

| Job | Format | Why |
| --- | --- | --- |
| Authored content (scenarios, rules, unit and country definitions) | RON (decided 2026-11-08; TOML only for plain engine settings, if at all) | Humans edit it. Comments allowed. Parsed once, by the loader or baker, not by the running simulation. |
| Baked runtime data (the data the game actually loads, including the map) | A binary, zero-copy-friendly format. Decided in Phase 3: a custom flat-array container with a CRC-32 (rkyv benchmarked equal, but the Python map builder cannot write it) | Loading approaches a straight read of bytes into the arrays the engine already uses. Produced by a baking tool, validated at bake time. |
| Saves and replays | A compact binary serde format with an explicit version and migration step. Default candidate: postcard, optionally compressed with zstd or lz4 | Small, simple, and stable across versions if you manage the version number yourself. |
| Source geometry (GeoJSON, Shapefile, GeoPackage) | Whatever the dataset ships in | Input to `map-builder` only. Never read at runtime. |

### 3.2 The pipeline

Authored text files and map-builder output go through a **baker** that validates everything and writes binary files into `data/baked/`. The game loads only baked files. A developer or modder mode may bake on the fly (with a cache) so editing text files stays quick.

This gives you: fast loads, errors found at bake time instead of mid-game, and human-friendly content.

### 3.3 Format rules

- Little-endian everywhere. Fixed field order. A magic number, a format version and a checksum in every binary file header.
- No hash maps in file formats. Use arrays, or sorted key-value vectors.
- No floats in anything the simulation reads. Coordinates, areas and costs are integers or fixed-point raw values.
- A schema version in every text file too, with a migration path for older files.

### 3.4 Why not some other formats

- **JSON at runtime:** larger and slower than binary for big data, no comments, loose typing. Fine for tiny tools and debugging output, not for content.
- **bincode:** do not use. The RustSec database marks it unmaintained, published December 16, 2025: the maintainers stopped development, and the advisory points to wincode, postcard, bitcode and rkyv as alternatives. Source: [RUSTSEC-2025-0141](https://api.osv.dev/v1/vulns/RUSTSEC-2025-0141).
- **rkyv for saves:** its archived layout is tied to your type definitions, which makes loading old saves painful. It suits baked data, which you regenerate, much better than saves, which players keep. (This is a judgment call; revisit if a benchmark says otherwise.)
- **FlatBuffers or Cap'n Proto:** worth considering only if non-Rust tools need to read the data. Otherwise they add a schema language and code generation for little gain.

### 3.5 To verify at the start of Phase 2

These were not fully confirmed when this guide was written. Check them on crates.io and each crate's repository before committing:

- [x] (Resolved in Phase 2; see `docs/decisions.md`: ron 0.12.3, postcard 1.1.3, rkyv 0.8.18 checked on crates.io on 2026-11-08, and the format benchmark run.) The current stable version of rkyv and its API. The one page that could be opened ([lib.rs](https://lib.rs/crates/rkyv)) listed 0.7.44 (February 2024) as the latest stable with a 0.8 alpha, but other search results mention later 0.7 and 0.8 release candidates, so that page looks stale. The page does confirm rkyv is a zero-copy framework with optional validation through the bytecheck crate.
- [x] (Resolved in Phase 2; see `docs/decisions.md`: ron 0.12.3, postcard 1.1.3, rkyv 0.8.18 checked on crates.io on 2026-11-08, and the format benchmark run.) That postcard is still actively maintained and which version to use. Its page could not be loaded, so nothing about it is verified here beyond the RustSec advisory naming it as an alternative.
- [x] (Resolved in Phase 2; see `docs/decisions.md`: ron 0.12.3, postcard 1.1.3, rkyv 0.8.18 checked on crates.io on 2026-11-08, and the format benchmark run.) That the RON crate is still actively maintained, and which version to use. Not checked when this guide was written.
- [x] (Resolved in Phase 2; see `docs/decisions.md`: ron 0.12.3, postcard 1.1.3, rkyv 0.8.18 checked on crates.io on 2026-11-08, and the format benchmark run.) A small benchmark: load the sample map as text, as postcard, as rkyv and as a custom flat-array file. Pick by measurement, and record the result in `docs/decisions.md`.

---

## 3A. Map scale decisions

Agreed 2026-11-08 as working defaults; Phase 3 may adjust them.

| Item | Decision |
| --- | --- |
| Strategic provinces | About 15,000 to 17,000 land provinces worldwide, about 9,000 km² on average |
| Small countries | A minimum of about 10 provinces per country, so small countries are cut finer |
| Tactical cells | Irregular cells averaging about 225 km² (generated by the map builder), about 600,000 land cells worldwide, about 40 per province |
| Sea | Sea zones in the same graph, probably hundreds, finer near coasts and chokepoints |
| Sites | A separate table of cities, ports, airfields, bridges and facilities |

The counts are estimates from about 135 million km² of land, excluding Antarctica.

### Two layers, one grid

- **Strategic layer:** the province graph. Ownership, supply, long-range movement, pathfinding and the AI work here.
- **Tactical layer:** a world-wide set of irregular cells averaging about 225 km². Each cell is a graph node with a center, an area and a neighbor list, found by a spatial index; range is measured in km from cell centers. Each province is a group of cells, so a fight near a border crosses it without special handling.
- When forces meet in a province, the engine opens it into cells, brigades fight cell by cell, and when the fight ends the province closes and its control is updated. The rules for winning and for deciding control are gameplay (G2). The engine provides open and close, and both must be exact and deterministic.

### Cell state

Static terrain is baked: terrain type, elevation, baseline urban density, road and rail flags, water. Dynamic state is a dense array indexed by cell ID, a few MB in total for the fixed fields below, plus the population group table, which is variable and unmeasured (estimate; see `docs/design-notes.md` sections 4 and 14):

| Field | Meaning | Size | Changes by |
| --- | --- | --- | --- |
| Controller | Which country holds the cell | 2 bytes | Fights, occupation |
| Damage | Destruction of buildings and terrain, 0 to 255 | 1 byte | Bombardment, fighting; slowly repaired |
| Fortification | Defensive works built | 1 byte | Unit engineering; destroyed by fire |
| Hazards | Flag bits for mines, craters, fire, fallout | 1 byte | Weapons; cleared over time |
| Last fought | Tick of the last fight, for recovery timers | 4 bytes | Fights |
| Population | Not a per-cell number: stored as group records (culture, religion, head count, support shares), see `docs/design-notes.md` section 14; cell and province totals are derived | variable | Births, deaths, hunger, refugees |
| Hunger | How short of food the cell is | 1 byte | Food balance between civilians and armies |
| Local food | Remaining local food stock or depletion | 1 byte | Foraging, regrowth by season, damage |
| Ground condition | Mud, snow depth or frozen ground | 1 byte | Weather and terrain, lagging behind weather |

Damage here is to the place, not to units. Casualties live on the units, and sites keep their own condition. Province-level values, such as average damage, are derived from the cells on demand or cached, and never stored as a separate truth. Transient fight state (which brigades are in which cells) is sparse and kept only for open provinces. Saves store the dynamic arrays as differences from the starting state.

### Sites

- **Types:** city, port, airfield, bridge or tunnel or canal, power plant or dam or refinery or factory, base or fortification.
- **Anchoring:** a city covers cells (those cells are tagged urban with a density level); a port sits on a coastal cell and links to a sea zone; a bridge, tunnel or canal sits on an edge; the rest sit on one cell.
- **Fields:** type, cell, province, owner, controller, condition, built year, destroyed year (so a 1990 start excludes later construction), plus type-specific properties.
- **Roads, railways and rivers** are edge attributes (type and quality), not sites.
- **Tiering:** include sites above a size threshold, plus hand-picked strategic ones (for example Suez, the Gulf oil terminals and nuclear facilities). Natural Earth populated places, GeoNames and OpenStreetMap are candidate sources only; check licenses and 1990 coverage in Phase 3.

### Bombardment

- A standing Bombard order names the firing unit, the target and a mode.
- The engine provides a range query on the cell grid; a command that validates range and visibility; condition values on units, cells, sites and edges; and an effects pass applied in a fixed order (target ID, then source ID), with chance rolls from the world's seeded generator.
- Short-range fire resolves on cells. Air and missile strikes hit sites or provinces directly, without opening cells.
- Interception, counter-battery fire, spotting, ammunition use and all the numbers are gameplay.

### Sea (working assumptions)

- Sea zones sit in the same graph as land, with a kind flag (land, sea or lake) and edge types (land border, coast-to-sea link, strait).
- Chokepoints such as Hormuz, Suez, the Bosporus, Gibraltar, Malacca and Panama are explicit edges.
- Large inland seas such as the Caspian are inland sea zones; smaller lakes are terrain.
- Air reuses the land and sea regions at first.

### 3B. Engine requirements from the design notes

Full detail is in `docs/design-notes.md`. The engine provides generic primitives; rules and numbers stay in data and the gameplay backlog.

- **Generic, data-defined entries:** supply classes (first set: fuel, ammunition, food), altitude bands (three), traits, equipment types, readiness levels, stances (attack, defend). The engine never hardcodes their names.
- **More dynamic state:** population group records (culture, religion, head count, political support), per-cell hunger, local food stock and ground condition; unit, officer, pilot and hull tables with stable IDs and status; per-country contact lists; orders, stances and transit states. All in the hash and the save diffs.
- **Primitives:** active-set scheduling, a modifier mechanism, a trait mechanism, multi-source supply propagation, shortage splitting, double-buffered flows with conservation tests.
- **Derived caches (not hashed, rebuilt and tested):** zones of control, visibility, air-defense coverage by band, front segments.
- **Read interface:** a per-country view limited by what that country knows, with a debug `omniscient` feature; commands that target enemies refer to contacts or places.
- **Combat and special cells (design notes sections 12 and 13):** engagement state and contested-cell pressure; two fight types (ZOC fighting, direct engagement); unit status and prisoner pools; garrisons for cities, sites and towns; doctrine, country traits and tech state; directional fortifications; river classes and a bridge layer; amphibious landing suitability and queues. No hard stack limit, enemy units never share a cell, and supply plus attrition are the brake.
- **Economy (design notes section 14):** extensible state tables declared by the ruleset but owned and hashed by the world (levelled building records, regional stocks, prices, construction projects, production orders, equipment stockpiles, treasury and debt); regional goods transport on the shared network; banded dynamic prices; labor staffing; construction and production as commands; money and debt. Economy systems run at daily, weekly and monthly rates.
- **Politics (design notes section 15):** data-defined factions, regimes, laws, war levels (per side) and an internal-conflict ladder (unrest, insurrection, revolution, civil war); per-group unrest, per-unit and per-officer loyalty, movement entities; law changes by faction support; runtime country creation and a deterministic civil war split; command validation against war levels.
- **Diplomacy (design notes section 16):** data-defined treaties (minor and major, major ones approved by faction support), alliances in two tiers (historic and regional) with a call-and-decide defense obligation, and a purpose-built UN with a binding Council (veto) and a non-binding Assembly.
- **Intelligence (design notes section 17):** data-defined agencies with stored capability per discipline, derived soft reach, abstract networks per target plus named operations, and deterministic estimate views with a margin that shrinks as knowledge rises.
- **Technology (design notes section 18):** stored field levels per country; specific development programs produce specific models (fields are prerequisites); research buildings plus learning by doing, capped by diffusion; transfers give the model but not the field.
- **Events (design notes section 19):** data-defined events with scopes, triggers, options and effects; event chains with stored state; separate linked instances per party; data-only conditions and effects (no scripting language for now); pending events, chain state, flags and cooldowns in the hash.
- **AI (design notes section 20):** a separate AI crate with the same rules and information as the player; data-driven utility AI with personalities, commitment and dedicated Rust planners; AI memory in its own save section outside the hash; difficulty by competence, with optional openly shown bonuses off by default.
- **Player interface (design notes section 21):** map first with contextual panels; nine map modes; direct orders plus drawn operation plans for headquarters, executed through the military planners with logged orders; order validation without applying; plan state outside the hash.
- **Map follow-ups:** cell latitude and longitude with an integer geodesic distance, finer sea zones near chokepoints, names, a render-only output file, river classes, a bridge layer, landing suitability, a settlement layer for towns, integer neighbor bearings, and new data layers (farmland, wetland, forest, climate) after license checks.

---

## 4. Phase-by-phase build steps

Each phase lists: goal, ordered steps, the done test, and common traps.

### Phase 1: Core loop

**Goal:** a headless simulation core that advances time deterministically and accepts commands.

**Steps**

1. Create the workspace with `meridian-core` and `runner`. Set the build profiles.
2. Build the fixed-point type. Use wider intermediate math so multiplication and division cannot overflow. Test arithmetic and edge cases.
3. Build the seeded random generator. Use a portable algorithm so every platform gives the same sequence. Expose its state for hashing and saving.
4. Build the clock: hourly ticks, a real calendar with leap years, and rollover signals for new day, new month and new year.
5. Define ID types as distinct wrapper types over integers (province, country, unit), so IDs cannot be mixed up.
6. Define world state as parallel arrays indexed by ID.
7. Define the command list and the tick function: apply commands first, then (later) run systems, then advance the clock.
8. Build state hashing that covers everything, including the random generator state.
9. Add the minimal proving-ground ruleset (one counter per province) so the hash changes in a meaningful way.
10. Write a one-command build-and-test script.

**Done test**

- Same seed and same command list give the same hash after 8,760 ticks, across repeated runs and on both macOS and Linux.
- Different seeds give different hashes.
- A command that breaks a rule (for example an order for a unit that does not exist) is rejected and leaves state unchanged.

**Common traps:** a float sneaking into state; iterating a hash map; a hash that forgets a field, which makes the determinism test pass when it should fail.

### Phase 2: Data and scenarios

**Goal:** load the world from data files and refuse bad data with clear errors.

**Steps**

1. Authored text format is RON, chosen over TOML because the content is deeply nested and enum-heavy, which RON mirrors directly. Record this in `docs/decisions.md`. Keep the schema types separate from the file format so switching later only touches the parsing code.
2. Define the schema types, each with a version field. Include site records (type, location, condition, built and destroyed years).
3. Write the loader that turns a scenario into world state. Every error names the file, the item and the problem.
4. Write the validator as a standalone command that reports all errors in one pass.
5. Write a baker stub that writes a binary file from a loaded scenario (the real format comes from the benchmark in section 3.5).
6. Create a small sample scenario.
7. Write one test per error class: unknown reference, duplicate ID, missing field, wrong version, malformed file.
8. Run the section 3.5 format benchmark and record the decision.

**Done test**

- The sample scenario loads and gives the same hash on every run.
- Each broken-file case produces its own readable error and no partly built world.
- The validator runs standalone and reports every error in one pass.

**Common traps:** stopping at the first error; letting content leak into code; loading files inside the simulation instead of before it.

### Phase 3: Real map

**Goal:** turn real geographic data into a baked province map the engine loads quickly.

**Steps**

1. Pick source datasets and check each license. Candidates: Natural Earth and GADM for regions, and a historical-borders dataset (such as CShapes) for 1990 country borders. Confirm each before relying on it.
2. Apply the map scale decisions (section 3A): about 9,000 km² per province, a minimum of about 10 per country, irregular cells averaging about 225 km². Respect 1990 borders.
3. Write `map-builder` to generate provinces, with fully deterministic output.
4. Build the adjacency graph: land borders, sea zones and strait links, coastal flags. Sea zones sit in the same graph, finer near coasts and chokepoints.
5. Build the world-wide irregular cell set (cut along rivers, ridges and coasts, in an equal-area projection) with baked terrain, and define each province as a group of cells. Terrain and climate come from the cells, with rivers and mountains as edge modifiers.
6. Add per-province centroid, area and 1990 population estimate, and bake the site table (cities, ports, airfields, bridges, facilities) with built and destroyed years.
7. Write the baked map file in the format chosen in Phase 2.
8. Write the map validator: no orphan provinces, no unreachable landmass without a sea link, every province has an owner.
9. In the engine, load the baked map and implement pathfinding with terrain costs. Start with Dijkstra, move to A*. Add the range query: all cells, sites or units within N cells of a cell.
10. Set the load-time and path-query budgets and record them in `docs/budgets.md`.

**Done test**

- Running the builder twice on the same inputs gives byte-identical output.
- The validator passes with zero errors on the full world map, including that every cell belongs to exactly one province.
- The engine loads the full map within the load-time budget.
- 10,000 random path queries stay within the per-query budget.

**Common traps:** non-deterministic geometry processing (floating point and unordered iteration both bite here); provinces that are valid shapes but useless for play; unverified dataset licenses.

**Follow-ups after the design discussion (see `docs/design-notes.md` section 22):** cell latitude and longitude plus integer geodesic distance (and a check of how cell-graph distances were computed); finer sea zones near chokepoints; antimeridian province links; province, sea and region names; the render-only output file; farmland, wetland and climate layers after license checks; the expected map checksum recorded in the repo; river classes (small and major) and a bridge layer at road and rail crossings; graded landing suitability per coast cell; a settlement layer for towns; integer neighbor bearings.

### Phase 4: System scheduler and events

**Goal:** run game rules in a fixed order without the engine knowing what the rules are.

**Sub-phases.** Phase 4 now holds 22 steps because the design notes grew. Build it as six sub-phases, each with its own done test, and run an early performance probe (a stub world with 600,000 cells, a stub group table and about 190 stub AI countries) at the end of 4A:

- **4A Core scheduler and primitives:** steps 1 to 10 (systems, order, rates, events, hooks, parallelism, timing, active sets, modifiers, traits, flow primitives).
- **4B World state, units and combat:** steps 11 to 14.
- **4C Economy and population:** step 15.
- **4D Politics, diplomacy and events:** steps 16, 17 and 20.
- **4E Intelligence and technology:** steps 18 and 19.
- **4F AI interface and view queries:** steps 21 and 22.

The step numbers below stay as they are; the sub-phase labels group them.

**Steps**

1. Define what a system is: a function over world state with a declared order and a rate (every tick, daily, monthly, yearly).
2. Build the registry. Resolve ordering and dependencies at startup, and fail loudly on a conflict or cycle.
3. Drive the rates from the clock's rollover signals.
4. Build the event queue, with handlers called at defined points in the tick. Effects such as bombardment damage use the same queue and are applied in a fixed order, by target ID and then source ID.
5. Build the trigger-and-effect hook so data files can express conditions and actions. Do not choose a scripting language yet.
6. Add deterministic parallel execution for systems that touch disjoint data. Verify it against the single-threaded run.
7. Add per-system timing.
8. Add active-set scheduling: sorted lists of cells or units that systems process, so quiet parts of the world cost almost nothing.
9. Add the generic modifier mechanism (fixed-point multipliers, fixed stacking order) and the generic trait mechanism (sorted trait lists with modifier bundles and scopes).
10. Add the shared primitives: multi-source supply propagation, shortage splitting (integer, remainders by ID) and double-buffered flows. Each gets a conservation test.
11. Add the dynamic per-cell state (controller, damage, fortification, hazards, last fought, population, hunger, food stock, ground condition) and the open and close of a province into cells, deferred from Phase 3.
12. Add the unit, officer, pilot and hull tables with stable IDs, plus transit states with arrival ticks.
13. Add the derived caches (zones of control, visibility, air-defense coverage, front segments) and a test that rebuilding them gives identical results.
14. Add the combat plumbing from `docs/design-notes.md` section 12: engagement state with a start-of-fight snapshot, contested-cell pressure, unit status and the days-encircled counter, prisoner pools, garrison records and the on-contact garrison function, doctrine, country-trait and tech state, directional fortification and bridge tables, per-cell unit lists with supply demand aggregation. Rules and numbers stay out of `meridian-core`.
15. Add the extensible state tables and the economy plumbing from `docs/design-notes.md` section 14: the population group table, levelled building records, regional stocks, labor staffing, goods transport over the shared network, banded prices, trade routes, construction projects, production orders and equipment stockpiles, treasury and debt. Rules and numbers stay in data.
16. Add the politics plumbing from `docs/design-notes.md` section 15: faction, regime, law and transition tables, legitimacy and stability, per-group unrest, movement entities, loyalty values, war levels per side with command validation, runtime country creation and the civil war split with conservation checks. Rules and numbers stay in data.
17. Add the diplomacy plumbing from `docs/design-notes.md` section 16: treaty, alliance and UN tables, the deterministic UN voting engine, and command validation for treaties and resolutions. Rules and numbers stay in data.
18. Add the intelligence plumbing from `docs/design-notes.md` section 17: agency, network and operation tables, and the deterministic estimate function for the view API. Rules and numbers stay in data.
19. Add the technology plumbing from `docs/design-notes.md` section 18: field levels, development programs, research buildings, the unlocked-model set and a generic prerequisite check. Rules and numbers stay in data.
20. Add the events plumbing from `docs/design-notes.md` section 19: the event queue with polled and raised triggers, pending event and chain state tables, and the generic condition and effect evaluator over a fixed primitive vocabulary. Rules and numbers stay in data.
21. Add the AI interface from `docs/design-notes.md` section 20 in Phase 6 or later: the AI crate boundary, the AI save section, scheduling by importance tier and per-AI timing. The AI itself is built after the systems it decides on exist.
22. Add the view queries for the map modes and the order dry-run validation from `docs/design-notes.md` section 21 to the Phase 6 interface. Layout and panel design belong to Phase 7.

**Done test**

- The same systems registered in a different source order but with the same declared order give the same hash.
- Single-threaded and multi-threaded runs give identical hashes over one simulated year.
- An event raised in one system is handled at the documented point in the tick.
- The timing report lists every system's cost per tick.
- The tiny proving-ground economy conserves goods and money, and drafting lowers output.
- A tiny diplomacy loop works: a major treaty passes or stalls by faction approval, an alliance call lets each member decide, and a UN vote respects a veto.
- A tiny political loop works: a law change passes or stalls by faction support, a command outside a side's war level is refused, one unit defects, and a civil war split conserves people, goods, money and units.
- Each flow primitive passes its conservation test, and each derived cache rebuilds to identical values after a save and load.

**Common traps:** parallel systems that look disjoint but share a counter; ordering that depends on registration order.

### Phase 5: Save, load and replay

**Goal:** any state can be saved, and any run can be replayed exactly.

**Steps**

1. Define the save format: header (magic, version, checksum), full world state, random generator state. Save per-cell state as differences from the starting state.
2. Write load, and verify the loaded state by hash.
3. Define the replay format: scenario reference, seed, and a command log stamped with tick numbers.
4. Build the replay player: re-run a log headless and compare hashes at checkpoints.
5. Build the divergence finder: given two runs, report the first tick where their hashes differ.
6. Add a migration stub so older save versions either load or fail with a clear message.
7. Add autosave hooks (no menus yet).
8. Include the new dynamic state (population, hunger, contacts, orders, stances, transit states, careers) in the save differences, record AI and delegated-headquarters commands in the replay log, and rebuild derived caches on load.

**Done test**

- Saving at any tick and loading gives the same hash as an uninterrupted run, checked at ten random ticks.
- A recorded replay reproduces the original final hash.
- The divergence finder names the first differing tick when two runs diverge on purpose.
- An older-version save either loads or fails with a clear message.

**Common traps:** state that exists at runtime but was forgotten in the save; saving derived data that should be rebuilt on load.

### Phase 6: Interfaces for UI and AI

**Goal:** define how outside code reads the world and gives orders.

**Steps**

1. Build the read interface: state access plus a per-tick report of what changed (including which cells changed), so a UI never copies everything.
2. Build the command interface shared by the player, the AI and scripts. Validate every command and return a specific reason on rejection. Include a Bombard order, validated for range and visibility.
3. Add a visibility hook so a player or AI can later be restricted to what its country could see.
4. Add a message stream for things the player should be told about.
5. Add the thread boundary so the simulation can run on its own thread while the UI renders.
6. Write a stub AI that gives orders through the command interface, to prove the path works.
7. Add the per-tick event list for the client (hits, strikes, missions, movements, battles) next to the changed cells.
8. Replace the visibility hook with the per-country view: contacts with knowledge levels, `World` internals `pub(crate)`, and a debug `omniscient` Cargo feature that the AI crate is built without. Commands that target an enemy refer to a contact or a place.
9. Add the new commands: Launch, Mobilize, Demobilize, Appoint, SetStance and the air mission and transit orders, each validated with a specific rejection reason.
10. Add the front query: boundary edges and contested cells between two countries, grouped into front segments.
11. Add the combat commands: SetDoctrine, Withdraw, Hold, Fortify, Amphibious Assault, join-assault, and bridge build, dismantle and demolish orders, plus an optional per-engagement modifier trace for UI tooltips.

**Done test**

- A test client reads state and issues commands without touching engine internals.
- A stub AI and a stub player run against each other headless, and the run is deterministic by hash.
- A rejected command returns a reason and leaves state unchanged.
- Reading state while the sim runs on another thread never stalls the sim beyond a limit set at the start of the phase.

**Common traps:** the UI quietly mutating state; handing out references that live across ticks.

### Phase 7: Client shell

**Goal:** a 2D map on screen with time controls. No gameplay screens.

**Steps**

1. Decide the renderer (custom wgpu or Bevy) and record the decision. The simulation stays separate either way.
2. Open a window and draw the baked map: province polygons, borders, terrain colors, labels.
3. Add camera control (zoom and pan).
4. Add province picking and a basic panel showing raw engine values.
5. Add map modes as a mechanism: political, terrain, and a debug mode that colors by any province value.
6. Add time controls: pause, speeds of 1 to 5 ticks per frame, single-step.
7. Draw unit icons and movement arrows for the proving-ground ruleset.
8. Add save and load menu entries and a settings screen.
   - Style: the clean paper-map look drawn from the render-only vector file, with a political mode switch (see `docs/design-notes.md` section 2).
   - Add an effects layer driven by the event list, contact markers with uncertainty, and hatching for contested cells.
9. Set the frame-rate target at the full province count and record it.

**Done test**

- A person can load the proving-ground scenario, pan and zoom the full map, click a province, step time at every speed, and save and load.
- The client holds the frame-rate target on the M4 Pro at the full province count.
- A client session and a headless run of the same command log end with the same hash.

**Common traps:** letting render needs reshape simulation data; redrawing everything every frame.

### Phase 8: Performance and tooling

**Goal:** prove the engine fits its time budget at full scale and keep it that way.

**Steps**

1. Build the benchmark harness: run the proving-ground ruleset at a realistic number of provinces and units.
2. Set up a profiling workflow on macOS (for example Instruments).
3. Write the per-tick and per-system time budgets and the memory budget, and make the build-and-test script enforce them.
4. Review the memory layout of the hottest arrays.
5. Tune pathfinding and spatial queries: A*, caching, a search-cost budget.
6. Run a scale test: a simulated decade, headless, faster than the speed target set at the start of this phase.
7. Add a developer log or dashboard showing tick time, system time and entity counts.

**Done test**

- The benchmark meets the tick-time budget at full province and unit counts on the M4 Pro.
- A budget regression fails the build-and-test script.
- A simulated decade runs headless within the speed target.

**Common traps:** optimizing before measuring; benchmarks that do not resemble real workloads.

---

## 5. The proving-ground ruleset

A deliberately tiny, throwaway ruleset that exercises the engine from Phase 1 on.

**Contents**

- Provinces with one counter that grows each day (stands in for an economy).
- Units that walk the province graph and change who controls a province (stands in for movement and occupation).
- One random event each month (stands in for the event system).
- Two or three countries and a war flag.

**Rules**

- It lives in its own crate. The engine never depends on it.
- It grows only when a phase needs something to test, never to become the game.
- It is deleted or replaced when real gameplay work begins.

---

## 6. Definition of done for any phase

- [ ] The done test passes, on macOS and (where relevant) Linux.
- [ ] Budgets set in this phase are written in `docs/budgets.md`.
- [ ] File formats touched are described in `docs/data-formats.md`.
- [ ] Decisions made are logged in `docs/decisions.md`.
- [ ] Nothing gameplay-specific has crept into `meridian-core`.
- [ ] The build-and-test script passes from a clean checkout.

---

## 7. Out of scope until the engine is done

These are the gameplay phases, to be re-planned once the engine passes Phase 8. The systems themselves are designed in `docs/design-notes.md` sections 12 to 21; what is left here is rules, numbers and content:

- **G1 Economy:** production, resources, trade, budget, population.
- **G2 Military:** equipment and units, supply rules, combat, air power and air defense, missiles and nuclear release, naval, occupation, ZOC and control timers.
- **G3 Politics and diplomacy:** regime and law content, factions, stability numbers, treaty and alliance content, UN resolutions, nuclear deterrence, civil war rules.
- **G4 AI:** personalities, planners (strategic, operational, tactical, economic, diplomatic) and difficulty settings.
- **G5 Content and events:** the 1990 scenario, equipment catalog, orders of battle, officers and traits, event chains, tech programs, intelligence agencies. Events are data-only (no scripting language). Start with the Persian Gulf slice.

---

## 8. Open decisions

- [x] Solo project; the user is a C# developer new to Rust, and will have AI write the Rust (decided 2026-11-08).
- [x] Engine language: Rust, for performance (decided 2026-11-08).
- [x] Map-builder language: Python (output is a baked file, so determinism is unaffected).
- [x] Hobby for now, commercial release left open, so prefer permissively licensed data (decided 2026-11-08).
- [x] Province count: about 15,000 to 17,000 land provinces over irregular cells of about 225 km² (agreed 2026-11-08; adjust in Phase 3 if needed).
- [x] Cell shape: irregular, generated by the map builder (decided 2026-11-08).
- [x] Cell projection: Equal Earth, equal-area; used only by the offline map builder, with true cell areas and centers baked into the file (decided 2026-11-08).
- [x] A regions layer above the provinces, for the AI, economy and UI (decided 2026-11-08).
- [x] Authored text format: RON (decided 2026-11-08).
- [x] Baked data format: custom flat-array container with CRC-32 (decided in Phase 3; see `docs/decisions.md`).
- [x] Save format: postcard (benchmarked in Phase 2; needs a version and migration step in Phase 5).
- [ ] Client technology: custom wgpu or Bevy.
- [ ] Historical-borders dataset for 1990 (deferred by the user until later).
- [x] Scripting: data files only for conditions and effects, no scripting language for now; revisit only if the primitive vocabulary proves too limited (decided 2026-11-09).
- [ ] Platforms beyond macOS.
- [x] The development slice is the Persian Gulf, 1990; Yugoslavia is the likely second slice (decided 2026-11-09).
- [x] Climate and land-cover candidates checked 2026-11-09: CHELSA climatologies (CC0) and ESA WorldCover (CC BY 4.0) are fine; WorldClim is non-commercial and avoided (see `docs/data-sources.md`).
- [ ] Rules still open (G-backlog): ZOC radius and control timers, ambush bonus, nuclear release rules, priorities in food shortages.
- [x] Map style: clean paper-map look, vector-drawn, with a political mode (decided 2026-11-08).
- [x] Supply: separate fuel, ammunition and food, with local foraging that hurts the population (decided 2026-11-08).
- [x] Units: equipment-based, fixed historical structures, specific models everywhere, helicopters in ground formations (decided 2026-11-08).
- [x] Commanders: generic officers with careers, plus commander and unit traits (decided 2026-11-08).
- [x] Air: abstract daily missions, individual fixed-wing pilots, three altitude bands, simple airlift (decided 2026-11-08).
- [x] Nuclear weapons: fully usable with strong gating (decided 2026-11-08).
- [x] Naval: individual hulls for major ships (decided 2026-11-08).
- [x] Information: full model with contacts, same information for the AI (decided 2026-11-08).
- [x] Ground control: zones of control with contested cells; stances attack and defend only (decided 2026-11-08).
- [x] Stacking: no hard limit, supply and attrition are the brake; enemy units never share a cell (decided 2026-11-09).
- [x] Combat: two fight types (ZOC fighting, direct engagement), automatic support, joining an assault is an order, bounded random variance, modifier categories with caps (decided 2026-11-09).
- [x] Retreat, rout, withdraw, hold, fight until death and surrender as in the design notes; no pursuit system (decided 2026-11-09).
- [x] Doctrine can change; country traits; tech covers non-equipment (decided 2026-11-09).
- [x] Cities are special: supply hubs with stocks, economy, population, strong defense; fixed garrisons by tier; difficulty comes from what is stationed there (decided 2026-11-09).
- [x] Rivers: small and major classes, bridges destroyable, laid bridges attach to roads and can be dismantled (decided 2026-11-09).
- [x] Fortifications are directional, per neighbor edge (decided 2026-11-09).
- [x] Amphibious landings build up over time, only on suitable coast cells (graded), and not everywhere (decided 2026-11-09).
- [x] Economy: light Victoria 3; about 14 goods; levelled building records with fixed capacity; regions as the economic unit; banded dynamic prices with a world price; goods move over the shared network; labor staffing from working-age population; construction and production from goods and labor; treasury with debt, printing and inflation, one currency; equipment from bills of materials (decided 2026-11-09).
- [x] Population identity: group records with culture, religion and political support (option 2, decided 2026-11-09). Strata stay open.
- [x] Politics: factions, regimes, laws, legitimacy, stability, national and faction cohesion; war levels (insurgency, limited, total, nuclear) set per side; laws change by faction support with no political-capital currency; internal conflict ladder unrest, insurrection, revolution, civil war; unrest per population group; rebels are movement entities promoted to countries; defections per unit and officer; the capital-holding government continues as the original country after a split (decided 2026-11-09).
- [x] Legitimacy is an integer 0 to 100 with a per-regime illegitimacy and collapse threshold; coups are played out with real units in three data-defined types (military, cabinet, foreign); movement goals are fixed at formation and coalitions fracture by faction cohesion (decided 2026-11-09).
- [x] Diplomacy: treaties minor and major (major need domestic faction approval); alliances in two tiers (historic and regional) with a call and each member decides; the UN is purpose-built with a binding Security Council (veto, rotating seats on a fixed data schedule) and a non-binding Assembly; resolutions are clause packages voted as a whole with no amendments; members choose compliance with detection; peacekeepers are voluntary contributions with host consent and reimbursement; a UN budget funded by dues; incidents open sessions automatically (decided 2026-11-09).
- [x] Country tiers: two axes, a derived power tier (hegemon, great power, regional power, minor) and bilateral recognition with a derived status; a tier is an automatic country trait that grants modifiers and unlocks actions (decided 2026-11-09).
- [x] Relations and standing: bilateral opinion with a derived baseline plus sparse decaying modifiers; standing is a derived 0 to 200 number (100 neutral), the tier-weighted average of others' opinions; actions affect only observers; seven standing bands (pariah, distrusted, wary, neutral, regarded, respected, esteemed) as automatic traits with hysteresis; opinion modifiers are discrete events with decay, observers and category caps, and some leave a residual scar (decided 2026-11-09).
- [x] Intelligence (in progress): agencies with stored capability per discipline (six: human sources, signals, imagery and space, open sources, counterintelligence, covert action); reach derived and soft; abstract networks plus named operations; estimate views with a shrinking, deterministic margin (decided 2026-11-09).
- [x] Covert action exposure and attribution are separate steps, and attribution can be wrong (decided 2026-11-09).
- [x] Covert actions: support a movement, foreign coup, sabotage, propaganda, election interference, sanctions busting and arms smuggling, special operations, and targeted removal with heavy consequences (decided 2026-11-09).
- [x] Deception and turned networks are deferred to a later expansion (decided 2026-11-09).
- [x] Agencies are separate and flagged foreign, domestic or both (decided 2026-11-09).
- [x] Internal operations: infiltrate movements, suppress and crack down, censorship and domestic propaganda, purge the officer corps, election control, counter-subversion; a country always knows its own factions and groups exactly (decided 2026-11-09).
- [x] Technology: fields are prerequisites, specific development programs produce specific models; fields grow by research buildings plus learning by doing, capped by diffusion; transfers give the model but not the field (decided 2026-11-09).
- [x] Tech starts with 13 fields: ground systems, aircraft, naval systems, missiles and guidance, electronics and sensors, nuclear, space, computing and information, materials and manufacturing, energy and heavy industry, agriculture and food, logistics and transport, medicine and public health (decided 2026-11-09).
- [x] Tech level is a 0 to 100 progress number per country and field, with eras derived for display (decided 2026-11-09).
- [x] Research buildings are specialized institutes, one type per field (decided 2026-11-09).
- [x] Development programs have delays and cost overruns, not outright failure (decided 2026-11-09).
- [x] Doctrine: one doctrine per branch (land, air, naval, strategic), unlocked by prerequisites, changed with a long transition, spread by alliances and training (decided 2026-11-09).
- [x] Events: data-defined events with scopes, options, chains and effects; separate linked instances per party; data-only conditions and effects, no scripting language for now (decided 2026-11-09).
- [x] Event tooltips and "why did this fire" text are generated from the primitives; every event also has authored flavor text for the event and each option (decided 2026-11-09).
- [x] Events have a data-defined severity; the client decides pause, ping, inbox or ticker (decided 2026-11-09).
- [x] AI: data-driven utility AI with personalities, commitment and historical tendencies, plus dedicated Rust planners; same rules and information as the player; memory outside the hash in its own save section; difficulty by competence with optional openly shown bonuses off by default (decided 2026-11-09).
- [x] Player interface: map first with contextual panels; nine map modes; direct orders plus drawn operation plans for headquarters (decided 2026-11-09).
- [ ] Interface open: layout and panel design, time controls, onboarding, accessibility, render budget.
- [ ] AI open: considerations and weights, personalities, planners, importance tiers.
- [ ] Events open: the final primitive list, the severity list, content workload.
- [ ] Tech open: program parameters, research buildings, diffusion cap, doctrine links.
- [ ] Intelligence open: effects and numbers of each covert action, deception, starting capability values for 1990.
- [x] Casus belli is a stored justification record with an expiry (events, claims, treaties, UN, fabrication); wars end by a negotiated peace as a major treaty accepted by the derived war position, no war score points (decided 2026-11-09).
- [ ] Diplomacy open: casus belli types and war goals, opinion modifier lists and exact standing band effects, treaty, alliance and resolution lists, UN membership details.
- [x] Civil war trigger: when a revolution meets its win conditions, a raised critical event offers the ruling entity yield, negotiate a transition or resist; the default is derived from loyal forces and army backing (decided 2026-11-09).
- [ ] Politics open: the dynamics of the special revolutionary country during a civil war (a data-defined country kind, rules later), the events list, lists of regimes, laws, factions and ideologies, treasury and debt split rule.
- [ ] Economy data sources: partly checked in `docs/data-sources.md` (USGS public domain; FAOSTAT promotion clause; GeoEPR non-commercial; World Bank CC BY 4.0, Factbook discontinued Feb 2026 and secondhand, Wikipedia text CC BY-SA 4.0; no oil field or industry source yet). Also open: region size for the economy and strata.
- [x] Settlements (GeoNames CC BY 4.0, Natural Earth public domain), rivers (Natural Earth, HydroRIVERS) and land cover (ESA WorldCover) are verified in `docs/data-sources.md`; landing suitability is hand-graded for the Gulf slice.
- [x] Data source policy: facts we extract and author ourselves are not limited by the source's license (record provenance, write our own wording, no bulk copying); datasets we import and bake must be public domain, CC0 or attribution-only, and non-commercial ones stay out (decided 2026-11-09; see `docs/data-sources.md`).
- [ ] Data-source follow-ups: GHS-BUILT and GHS-SMOD terms (GHS-POP verified; SIPRI is commercial-only), HydroSHEDS documentation, an oil field and industry source, whether to use OpenStreetMap (share-alike). Add a license field per data layer and a commercial-build check in the bake step.
- [x] Bulk tables as CSV, structured data as RON (decided 2026-11-08).

---

## Sources

- [RUSTSEC-2025-0141: Bincode is unmaintained](https://api.osv.dev/v1/vulns/RUSTSEC-2025-0141) (published December 16, 2025; modified January 16, 2026).
- [rkyv on lib.rs](https://lib.rs/crates/rkyv) (describes rkyv as a zero-copy framework with optional bytecheck validation; its version listing appears out of date).
