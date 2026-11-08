# Meridian: Engine Build Guide

As of 2026-11-08. Draft for review. This guide covers the **engine only**. Economy, military, politics, diplomacy, AI behavior and the 1990 content are out of scope until the engine phases pass their tests (see section 7).

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
| Baked runtime data (the data the game actually loads, including the map) | A binary, zero-copy-friendly format. Default candidate: rkyv. Fallback: a custom flat-array container | Loading approaches a straight read of bytes into the arrays the engine already uses. Produced by a baking tool, validated at bake time. |
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

- [ ] The current stable version of rkyv and its API. The one page that could be opened ([lib.rs](https://lib.rs/crates/rkyv)) listed 0.7.44 (February 2024) as the latest stable with a 0.8 alpha, but other search results mention later 0.7 and 0.8 release candidates, so that page looks stale. The page does confirm rkyv is a zero-copy framework with optional validation through the bytecheck crate.
- [ ] That postcard is still actively maintained and which version to use. Its page could not be loaded, so nothing about it is verified here beyond the RustSec advisory naming it as an alternative.
- [ ] That the RON crate is still actively maintained, and which version to use. Not checked when this guide was written.
- [ ] A small benchmark: load the sample map as text, as postcard, as rkyv and as a custom flat-array file. Pick by measurement, and record the result in `docs/decisions.md`.

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

Static terrain is baked: terrain type, elevation, baseline urban density, road and rail flags, water. Dynamic state is a dense array indexed by cell ID, a few megabytes in total (estimate):

| Field | Meaning | Size | Changes by |
| --- | --- | --- | --- |
| Controller | Which country holds the cell | 2 bytes | Fights, occupation |
| Damage | Destruction of buildings and terrain, 0 to 255 | 1 byte | Bombardment, fighting; slowly repaired |
| Fortification | Defensive works built | 1 byte | Unit engineering; destroyed by fire |
| Hazards | Flag bits for mines, craters, fire, fallout | 1 byte | Weapons; cleared over time |
| Last fought | Tick of the last fight, for recovery timers | 4 bytes | Fights |

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

### Phase 4: System scheduler and events

**Goal:** run game rules in a fixed order without the engine knowing what the rules are.

**Steps**

1. Define what a system is: a function over world state with a declared order and a rate (every tick, daily, monthly, yearly).
2. Build the registry. Resolve ordering and dependencies at startup, and fail loudly on a conflict or cycle.
3. Drive the rates from the clock's rollover signals.
4. Build the event queue, with handlers called at defined points in the tick. Effects such as bombardment damage use the same queue and are applied in a fixed order, by target ID and then source ID.
5. Build the trigger-and-effect hook so data files can express conditions and actions. Do not choose a scripting language yet.
6. Add deterministic parallel execution for systems that touch disjoint data. Verify it against the single-threaded run.
7. Add per-system timing.

**Done test**

- The same systems registered in a different source order but with the same declared order give the same hash.
- Single-threaded and multi-threaded runs give identical hashes over one simulated year.
- An event raised in one system is handled at the documented point in the tick.
- The timing report lists every system's cost per tick.

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

These are the gameplay phases, to be re-planned once the engine passes Phase 8:

- **G1 Economy:** production, resources, trade, budget, population.
- **G2 Military:** units, supply, combat, air power, occupation.
- **G3 Politics and diplomacy:** stability, breakups, alliances, war and peace, nuclear deterrence.
- **G4 AI:** strategic, operational, tactical and economic behavior.
- **G5 Content and events:** the 1990 scenario, flashpoints, tech trees, scripting.

---

## 8. Open decisions

- [x] Solo project; the user is a C# developer new to Rust, and will have AI write the Rust (decided 2026-11-08).
- [x] Engine language: Rust, for performance (decided 2026-11-08).
- [ ] Map-builder language: defaults to Python (best geographic libraries; output is a baked file, so determinism is unaffected). Veto if you prefer Rust or C#.
- [x] Hobby for now, commercial release left open, so prefer permissively licensed data (decided 2026-11-08).
- [x] Province count: about 15,000 to 17,000 land provinces over irregular cells of about 225 km² (agreed 2026-11-08; adjust in Phase 3 if needed).
- [x] Cell shape: irregular, generated by the map builder (decided 2026-11-08).
- [x] Cell projection: Equal Earth, equal-area; used only by the offline map builder, with true cell areas and centers baked into the file (decided 2026-11-08).
- [x] A regions layer above the provinces, for the AI, economy and UI (decided 2026-11-08).
- [x] Authored text format: RON (decided 2026-11-08).
- [ ] Baked data format: rkyv or a custom flat-array container (decide by benchmark, section 3.5).
- [ ] Save format: postcard or another serde binary format (verify maintenance first).
- [ ] Client technology: custom wgpu or Bevy.
- [ ] Historical-borders dataset for 1990 (deferred by the user until later).
- [ ] Scripting: Lua, Rhai, or data files only (needed by Phase 4 only if the trigger hook proves too limited).
- [ ] Platforms beyond macOS.

---

## Sources

- [RUSTSEC-2025-0141: Bincode is unmaintained](https://api.osv.dev/v1/vulns/RUSTSEC-2025-0141) (published December 16, 2025; modified January 16, 2026).
- [rkyv on lib.rs](https://lib.rs/crates/rkyv) (describes rkyv as a zero-copy framework with optional bytecheck validation; its version listing appears out of date).
