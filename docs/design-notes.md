# Meridian design notes

Written 2026-11-08 and 2026-11-09 (economy, politics, diplomacy, intelligence, tech, events, AI and the player interface added 2026-11-09) from long design discussions. Nothing here is built yet. Each section says what was decided, what the engine must provide, and which phase picks it up.

## How to read this file

**Map redesign (2026-11-10):** section 25 replaces the earlier map design (irregular cells on a flat Equal Earth map) with hexagon levels on a globe and a ladder of military map scales. Where sections 2, 11, 13 and the build guide's section 3A speak of cells of about 225 km², read them with section 25 in mind until they are rewritten.

- **Engine** means code in `meridian-core` or `meridian-data`: state, primitives, validation. It must be deterministic and know nothing about specific weapons, countries or numbers.
- **Data** means content in RON or CSV files (catalogs, templates, tables, rules). Changing it never requires engine changes.
- **Rules** means gameplay formulas and numbers (the G1 to G5 backlog in the guide). Rules are decided later. The engine only has to make them expressible.
- Words like "estimate" and "verify" mark claims that were not checked against a source. Check them before relying on them.
- `CLAUDE.md` and `ENGINE_BUILD_GUIDE.md` still win on any conflict. If something here changes the plan, update the guide too.

## 1. Cross-cutting principles

1. **Derive, don't store.** Strength, wing veterancy, task-force veterancy, province values, ZOC strength, visibility and front lines are derived from source state. Stored state is the minimum needed to rebuild them. Derived caches are never hashed, and a test checks that rebuilding gives identical results.
2. **Generic in the engine, specific in data.** Supply classes, altitude bands, traits, equipment types, readiness levels and stances are all numbered entries defined in data. The engine never hardcodes "fuel" or "high band". Counts such as "three bands" are data settings.
3. **Rules decide, the engine keeps score.** The engine stores fields, validates commands, runs systems in a fixed order and exposes queries. Formulas and numbers live in the rules data.
4. **Scarcity is real.** Expensive equipment has finite production lines, long lead times and real costs. Pilots, officers and hulls are individual records that cannot be conjured.
5. **Active sets.** Most of the world is quiet at any moment. Systems that touch many cells or units (hunger, ground condition, detection, ZOC) process only an active set kept as a sorted list.
6. **Conservation tests.** Anything that moves people or goods between places must conserve totals except through explicit births, deaths, production or consumption. Each such system gets a conservation test in the proving ground.
7. **Double-buffered flows.** Flows (refugees, supply, shortage splitting) compute from the start-of-step state and then apply, so processing order cannot change the result. Rounding remainders go out in a fixed order, by ID.
8. **AI commands are logged.** Commands from the AI and from delegated headquarters go into the replay command log like player commands. Replays then do not depend on the AI code version.
9. **Facts in data, formulas in rules.** Catalog entries store real-world specifications. Game values are derived by formulas, so balancing means editing formulas, not hundreds of entries.
10. **Honest sourcing.** Every catalog entry and every hand-made judgment (starting veterancy, traits, readiness) records a source or a reason, plus a confidence level.

## 2. Presentation

### Map style

- **Clean paper-map style**, printed look, no creases or stains: off-white sheet, a few muted ink colors, contour lines from elevation, light hill shading, thin dark rivers and coasts, dashed borders, faint grid ticks, typewriter or stencil labels, NATO-style counters.
- Drawn almost entirely from **vector lines** by code. Only the paper texture is an image (generated in code or a CC0 texture; check the license).
- A **plain political map mode** is always available as a switch, because the paper look can hide gameplay information. Fronts, selected provinces and ownership need strong contrast.
- **Map-builder gets a render-only output:** simplified contours, rivers, coasts and borders for each zoom level, plus cell outlines (the baked simulation map stores only centers and areas). The engine never reads this file.
- **Names are needed for labels:** provinces, sea zones and regions. Cities already have names. Label placement (no overlaps, curved labels along rivers and ranges) is real work.
- **Fonts:** open-licensed typewriter-like fonts (for example Courier Prime or IBM Plex Mono, both OFL). Verify each file's license.
- **Client choice:** a clean 2D line-and-shader renderer favors a lighter custom wgpu renderer over Bevy. Still to be decided in Phase 7.

### Effects

- Explosions, smoke, flying planes and similar effects are **client-only** and **event-driven**. They never touch the simulation.
- The engine's read interface includes a **per-tick event list** (hits, strikes, missions flown, movements, battles), in addition to changed cells (Phase 6).
- The client uses its own random numbers for sparks and smoke. During replays it regenerates effects from the events.
- Detail scales with zoom: pulsing markers and flashes when zoomed out, full animations only on screen and zoomed in.
- Aircraft in the client are cosmetic flights drawn between a base and a target. The simulation does not track flights (see section 7).
- Effects match the paper look (ink-stamp bursts, dotted arcs).
- Free asset packs (Kenney, OpenGameArt) are candidates, but check each pack's license. Keep `docs/attributions.md` as the ledger for every asset.
- Leader portraits are not used. Commanders are generic (section 6).

## 3. Weather and ground condition

- **Two layers.** Weather is regional and changes daily. Ground condition is local, lags behind weather and recovers slowly.
- **Weather zones:** a few hundred regional zones (large province groups, plus sea zones for sea state). Each holds a small state: temperature band, precipitation, wind, cloud cover.
- **Ground condition:** a dense one-byte array per cell (mud, snow depth, frozen ground), updated daily from the zone's weather and the cell's terrain, using the active set. Frozen ground can help (marshes passable in a hard winter).
- **Generation:** drawn from climate statistics, not scripted history. A climate table by zone and month gives the odds. A persistence rule (Markov chain) makes today depend on yesterday. A yearly severity roll per zone makes a harsh winter or wet spring. Draws use the world's seeded generator in a fixed zone order.
- **Modifiers:** a generic engine mechanism. Rules data lists conditions and multipliers (fixed-point), stacking in a fixed order. They apply to movement cost, supply capacity, combat, air operations (cloud grounds aircraft), naval operations (storms) and detection. Pathfinding already takes per-query cost tables, so weather plugs in without changing the search.
- **Data needs (all to be license-checked):** a climate table by zone and month (candidates: NOAA data, generally public domain; Copernicus ERA5, free with attribution; I believe WorldClim is non-commercial, so avoid it), and a wetland layer for mud (the ESA WorldCover land-cover dataset is a candidate; I believe it is CC BY 4.0, verify).
- **Phase:** zone map in Phase 3 follow-up, state and scheduling in Phase 4.

## 4. Supply, food and population

### Supply classes

- **N generic supply classes**, defined in data. The first set is **fuel, ammunition and food**. Others (spare parts, medical) can be added without engine changes.
- **Stockpiles:** a small buffer on each unit (a few days), large stocks at depots and sites, and daily flow along links.
- **Network:** a multi-source propagation primitive spreads supply from sources across a graph using integer edge costs and capacities. Ties break by ID. Roads and rails raise link capacity and rough terrain lowers it. Start with one shared capacity per link and a priority order between classes.
- **Two layers:** abstract strategic supply on the province graph daily, detailed supply on cells only in opened provinces.
- **Recompute only when control or infrastructure changes.** Damage to roads, bridges and ports lowers capacity through the condition values.
- **Consumption comes from activity:** moving burns fuel, fighting burns ammunition, existing eats food.
- **Production** (refineries, factories, harvests) is the economy (section 14). Supply only moves and consumes.
- **Shortage splitting primitive:** when demand exceeds supply, split proportionally with integer math, giving remainders out by claimant ID. Gameplay sets priority.
- Land and sea supply at first. Airlift is a separate simple mechanism (section 7). Allied sharing of supply is not designed yet; it would be a treaty term (section 16).
- High-value missiles (SAMs, cruise and ballistic missiles) are counted equipment, not generic ammunition.

### Food and population

- **Food comes from both sources.** A unit takes local food first (foraging from the cells it stands on), then shipped supply covers the shortfall.
- **Local yield** depends on farmland, population, season, who controls the cell, and depletion. Foraging lowers the cell's stock and it regrows slowly. Damage or scorched earth lowers the yield.
- **Foraging hurts the population.** Civilians and armies claim the same local food, and the shortage splitting primitive decides who gets it. Gameplay sets the priority.
- **Population is live state.** Population is stored as group records (section 14), not a plain per-cell number; per-cell and province totals are derived from the records. Beside it sit a per-cell hunger level (1 byte) and local food stock or depletion (1 byte). All in the hash and the save diffs. The group table size is unmeasured (see section 23).
- **Consequences go all the way (option 2):** hunger lowers output and raises unrest, and people can **die or flee**. Refugees move cell to cell along the cell graph, double-buffered, in fixed order, with integer transfers. Births and deaths are explicit steps with integer rates.
- **Conservation test:** total population changes only through explicit births and deaths, never through movement.
- Refugees cascade (they raise demand in neighboring cells) and cross borders (diplomatic effects, G3).
- **Identity is decided in section 14.** Population is stored as group records (culture, religion, head count, political support) instead of one number per cell. The sizes above are for the plain array and will grow; measure the group table in Phase 4.
- **Data gaps:** farmland is missing (the land-cover dataset would give it; a cruder fallback is estimating from population density, terrain and climate). Cell populations are modeled, not census data.

## 5. Units and equipment

### Principles

- **Equipment-based.** A unit holds counts of equipment types and personnel. Strength is derived: actual against the authorized table of organization and equipment (TO&E).
- **Fixed historical structures.** Each country has its real 1990 organization. The player reinforces, modernizes and reassigns, but does not design divisions. Attaching or detaching a brigade is a change of parent pointer.
- **Command both ways.** The player commands brigades individually, or gives objectives to higher headquarters, whose AI turns them into orders through the same command interface. Headquarters hold orders and objectives as state.
- **Brigade is the smallest maneuver unit**, about one cell. Everything smaller is internal detail.
- **Unit table:** parallel arrays indexed by unit ID: owner, type, parent, cell, orders, stocks, attributes. Destroyed units leave empty slots and IDs are never reused during a game.
- **Hierarchy** by parent pointers (brigade, division, corps, army, theater). The engine stores and moves it and gives it no meaning.

### Unit attributes

| Attribute | Meaning | Stored or derived | Changes by |
| --- | --- | --- | --- |
| Strength | Equipment and personnel against the authorized TO&E | Derived | Losses, replacements |
| Veterancy | Experience, levels from thresholds in data | Stored (small integer) | Combat, training; diluted by recruits |
| Morale | Willingness to fight | Stored (fixed-point 0 to 100) | Results, supply, victories and defeats |
| Cohesion | Organizational integrity | Stored (thousandths, 0 to 1000) | Falls with movement, fighting, lost command or supply; recovers with rest |
| Readiness | How prepared the unit is to deploy and fight | Stored (thousandths) | Mobilize and Demobilize commands, time |

- **Replacement dilution:** when losses are replaced by recruits, experience becomes the headcount-weighted average of old experience and the recruits' experience (integer math).
- **Readiness is a ladder** with transition times from data (for example cadre or reserve, mobilizing, ready, deployed). It sets how much of the authorized equipment and manpower is combat-ready and how fast a unit can move out. Reserve units can start understrength and fill on mobilization. Starting levels vary by country and unit.
- **Unit traits** (section 6) make two paper-identical units differ.
- Starting veterancy, morale and traits for real units are **judgments**. Document the reasoning per country or formation.
- **Updates** run in a daily system with the active-set approach.

### Equipment catalog

- **Specific models everywhere**, for every branch (for example F-16A and F-16C, MiG-29). A rough guess is 300 to 600 air models and a few thousand entries overall for 1990. That is an estimate, not researched.
- **No pure trainers.** Trainers that flew combat roles (Hawk, Alpha Jet, L-39, MB-339) are included as light attack aircraft. Pilot training is an abstract pool with a training time.
- **Store real specs, derive game stats** by formula (speed, range, payload, sensors, crew, entry year, production figures).
- **Variants inherit from a base** and list only differences.
- **Provenance:** every entry has a source note and a confidence level.
- **Availability by year:** new generations do not exist before their entry years, and development takes years.
- **Production lines are limited entities** (data and rules): a site (a factory, so it can be bombed), a maximum monthly rate, ramp-up time, lead time from order to delivery, and a unit cost. Expanding a line or starting a new one takes years and money. Real production rates anchor the numbers (verify figures from manufacturer and other sources before use).
- **Pilots are the other bottleneck.** An aircraft without a pilot is useless.
- **Maintenance:** only part of a fleet is mission-capable and airframes wear out, which fits readiness.
- The same scarcity applies to tanks, ships and missiles.
- **Operating costs** (fuel, spare parts) draw on the supply system.

### Data formats

- **CSV (edited in a spreadsheet) for bulk tables:** the equipment catalog and country inventories. **RON for nested data:** scenarios, rules, the unit hierarchy and traits. The baker reads both, validates everything with full error collection, and writes the binary files.
- **CSV rules:** UTF-8, comma separators, dot decimals. The baker rejects anything else with a clear error. Spreadsheets in Brazilian settings often export semicolons and decimal commas, so export from Google Sheets or LibreOffice with the right settings. Store most values as integers in fixed units.
- **Rows have stable text keys** (for example `f16c_block50`). The baker assigns numeric IDs at bake time in a deterministic order. Saves are tied to a content version.

### Content workload (G5)

- Orders of battle for about 170 countries are the biggest job, bigger than the code.
- **Develop on a regional slice first** (one theater, for example the Persian Gulf or Central Europe, with a handful of countries in detail), then expand. The theater is not chosen yet.
- **Tiers:** major powers detailed to brigade level, regional powers moderate, the rest from generic templates scaled to national equipment totals.
- **Sources need license checks.** The 1990 CFE Treaty data exchange covers Europe in detail. Wikipedia is CC BY-SA (record facts, write our own text). Commercial references such as Jane's and the IISS Military Balance are copyrighted compilations, so use them for checking, not copying. US government fact sheets are generally public domain.
- Real unit designations are facts and fine to use. Do not use unit insignia (rights vary). Use NATO-style counters.

## 6. Commanders, traits and careers

- **Commanders are generic officers**, not real people. Names come from per-country name pools that we author as simple lists.
- **Officer table** with stable IDs: country, name, birth date (days before 1990-01-01, so age is derived), rank, experience, traits, status (active, wounded, retired, dead) and assignment. Dead and retired officers keep their slots as history.
- **One commander per formation with a headquarters**, including task forces. A few thousand officers worldwide.
- **Initial officers** are generated offline into the scenario file. New officers during play come from the seeded generator in a fixed order.
- **Two trait kinds, one mechanism.** Trait definitions in data are bundles of modifiers with a scope. Each entity holds a small sorted list or bitset of trait IDs.
  - **Commander traits** describe the officer. Scope can extend down the chain (an army commander's trait may affect subordinates' cohesion).
  - **Unit traits** describe the formation (Guards, airborne tradition, mountain-trained, poorly maintained, politically unreliable). They are assigned in scenario data and can be earned or lost (battle honors).
  - Start with a small catalog, roughly 10 commander traits and 15 unit traits.
- **Careers:**
  - Promotion by experience and rank thresholds, limited by posts in the hierarchy.
  - Wounding and death in combat, at rates from data.
  - Retirement by age.
  - **Vacancies cascade.** A post emptied by retirement is filled by the best candidate in the same country by a fixed rule (rank, then experience, then ID). A subordinate's promotion empties his post, and so on down to a new officer entering at the bottom.
  - **Appoint command:** the player can fill a post. If the player does not act within a set number of days, the default fills it, and the empty post carries a penalty meanwhile.
  - Career processing runs monthly; combat injuries resolve with combat.
- **Hooks into politics:** loyalty, purges, favoritism and coups are designed in section 15 (per-unit loyalty, three coup types).

## 7. Air power

### Aircraft and wings

- Wings and squadrons are equipment-based units with aircraft models as counts, **based at airfield sites**. A squadron is roughly a dozen to two dozen aircraft, a wing a few squadrons.
- **Pilots are individual records** (fixed-wing only). Each has a wing, experience, kills, age, rank and status. A wing's veterancy is **derived** from its pilots. Aircraft loss does not equal pilot loss: ejection and recovery (over friendly territory is better) means the pilot may be rescued, captured or killed.
- **Kill attribution:** with abstract combat, the engine credits kills by drawing from the wing's active pilots, weighted by experience, from the seeded generator in a fixed order. Ace status (five kills) is derived. Pilot traits and promotion into command from the pilot pool are possible later.
- **Helicopter pilots are not tracked.** Helicopter units store their own veterancy.

### Missions (abstract)

- **Missions are orders** assigning a wing to a mission type and target (a province, site, or a cell in an opened fight): air superiority and patrol, close air support, interdiction, strategic bombing, reconnaissance, SEAD, naval strike and anti-submarine patrol, airlift.
- **Resolved once per day** in a fixed order by wing ID. **Flights are not tracked.** There is no flight entity. Wing state is its mission, target, sorties used today, readiness.
- **Sorties** per day depend on mission-capable aircraft, pilots, readiness, runway condition, weather and fuel. Consumption and losses come from supply and combat resolution.
- **Interception** is a regional contest. Fighters on patrol defend a region, and any enemy mission into it must beat them first. Radar coverage sets reaction. SAM and AAA units add to the defense.
- **Bases are targets.** Damaged runways or fuel stocks ground the wing, through condition values on sites.
- **Weather** can ground aircraft through modifiers.

### Distances (engine fix needed)

- Equal Earth preserves areas, not distances. Distances are distorted, a little at short range and noticeably over hundreds or thousands of kilometers, especially at high latitudes.
- Fine for ground combat within tens of kilometers. **Not fine for aircraft and missile ranges.**
- **Fix:** bake each cell's latitude and longitude (integer microdegrees) beside its projected position, and use an **integer or fixed-point geodesic distance** (lookup tables or CORDIC, no floating-point trig). Also check how `c_dist_m` in the cell graph was computed (true ground distance or projected). I have not verified this in the code.

### Air defense

- **Layers:** strategic SAMs (S-300, Patriot), medium-range SAMs (Hawk, SA-6, Buk), short-range SAMs (Roland, Rapier, SA-8), MANPADS (Stinger, SA-7 and later), and AAA guns.
- **Three altitude bands: low, medium, high**, kept as a data setting. Each air mission has an altitude profile. Each system has an effectiveness table by band and by target class (fast jet, helicopter, bomber, cruise missile, ballistic missile). High altitude avoids short-range systems but meets long-range SAMs. Low altitude evades radar but meets AAA and MANPADS. Helicopters are low-band only. Cruise missiles fly low, ballistic missiles high.
- **MANPADS are organic equipment** in ordinary TO&Es, so they exist across the battlefield.
- **Coverage:** systems sit in cells with a range, and the existing range queries give the area. Per province and per band, a **derived defense strength** is cached and rebuilt when units move or change status.
- **Radar and suppression:** radar-guided SAMs depend on radar. Anti-radiation missiles and jamming **suppress** a battery for a time (a unit status). Stealth is a spec (the F-117 was operational by 1990) that lowers detection.
- **Cost:** SAM batteries and missiles go through the same limited production lines. High-value missiles are counted equipment; cheap shells use the generic ammunition class.

### Helicopters

- **Helicopters belong to ground formations** (aviation battalions or brigades inside a division, corps or army), with the same rule for every country. A few countries really keep them in the air force; we assign them to ground formations anyway and log it as a simplification.
- Types: attack, utility and transport, heavy lift, scout. Naval helicopters belong to their ships.
- **Basing:** forward sites at any cell with a friendly unit, limited by fuel. No airfield needed.
- **Resolution:** hourly in the ground combat step when supporting an opened province, daily otherwise.
- Vulnerable in the low band only, hit hard by weather, high maintenance and fuel use, expensive to replace. Terrain masking is their main protection. Night and adverse-weather capability is a spec per model.

### Airlift and paratroopers (kept simple)

- **Supply airlift:** a transport wing is assigned to one destination airfield and delivers a fixed amount per day (mission-capable aircraft times a payload number, one sortie a day). In range it delivers in full, out of range nothing. It burns fuel, needs a working airfield, and can be hit by medium-band air defense. No airdrop resupply for now.
- **Unit moves by air:** whole units only, no splitting. Each unit definition has an **airlift cost** number, each transport wing a **lift capacity**. A move is allowed if the capacity covers the cost and the destination is in range. Heavy units cost more than any wing can carry, so they cannot fly.
- **Paradrops:** only units flagged airborne. Same capacity check. The unit is in a **transit state** with an arrival tick, lands with a fixed cohesion penalty and no partial equipment loss, and a drop in enemy territory opens a normal fight. The drop zone must have suitable terrain (not water or high mountains). Medium-band air defense, enemy fighters and weather apply.
- **Assignments are explicit**, one task per wing, so transports never compete over one pool.
- **Air assault** by helicopter uses the same transit mechanism (low band).
- **Transit state:** a unit mid-air-move or mid-sea-move holds a destination and an arrival tick. It is a movement with a risk, not a tracked flight or ship.

## 8. Missiles and nuclear weapons

- **Classes:** ballistic (short to intercontinental, including submarine-launched; high band), cruise (ground, air, ship and submarine launched; low band), rocket artillery and tactical missiles (weapons inside ordinary ground units), anti-ship missiles (naval) and anti-radiation missiles (SEAD).
- **Launchers are equipment in units** (launcher vehicles, silos, submarines, aircraft). A unit holds counts of launchers and of each missile type as counted equipment.
- **A `Launch` command** names the unit, missile type, target (site, province or cell) and count. It is validated for range, stock and readiness.
- **Resolution:** a salvo resolves on its arrival tick from flight time (distance and speed). Defenses apply in the target area (high band for ballistic, low band for cruise). No in-flight entity is tracked.
- **Effects:** damage to sites through condition, to cells through the damage and hazard arrays, and to units in the target cells. Reload and replacement use the usual production and supply systems.
- **Nuclear weapons are fully usable, with strong gating.**
  - A **warhead type** on the same delivery systems, with limited counted stocks.
  - **Ring-damage tables** in data: damage to cells, sites and units by distance from the target. A large-radius `cells_within` covers hundreds of cells; measure it in the budgets.
  - **Hazards:** fallout and fire flags decay over time. Wind drift can come later using the weather system.
  - **Mass casualties** go through the dynamic population state, and refugees and hunger follow.
  - **Release is a gated command,** validated against alert level and the country's release rules. The rules and the AI's use of them are G3 and G4.
  - **Second-strike survivability** ties to hardened silos and submarines hidden in sea zones.
  - **Balance:** limited stocks, slow production and heavy political consequences keep it a last resort. In 1990 only a handful of states had arsenals, some undeclared, and the data needs careful handling.
  - **Tests:** a nuclear strike in the proving ground must account for deaths explicitly and give identical hashes on every platform.

## 9. Naval

- **Movement runs on the sea-zone graph.** The 862 zones are 450 to 1,400 km apart. At 15 to 30 knots a ship covers roughly 28 to 55 km per hour, so crossing a zone takes hours to a day. That fits the hourly tick, using pathfinding with sea costs.
- **Individual hulls for major ships** (roughly corvette and up; the cutoff is rules data), counted equipment in flotillas for small craft (patrol boats, landing craft, small mine warfare). There are probably a few thousand vessels worldwide in 1990 (estimate).
- **Hull record:** catalog model, real name and hull number, owner, location (zone or port), parent task force, condition (hull integrity and damaged systems: propulsion, weapons, sensors; sinking at zero), fuel and missile and ammunition stocks, crew veterancy and readiness, status (active, in repair, sunk, decommissioned). Sunk ships stay as history.
- **Task forces** are formations: commander from the officer table, traits, morale and cohesion. Task-force veterancy is derived from its hulls. No per-ship captains unless added later.
- **Repair** happens at ports with drydocks, with times set by damage and the port's condition.
- **Carriers** have a capacity for air wings. An embarked wing flies its missions from the carrier, with deck damage limiting sorties. Carrier pilots are fixed-wing and are in the pilot table. Marines and helicopters are units loaded on amphibious ships.
- **Operations:** surface and air-naval combat (detection first, then the exchange with anti-ship missile salvos and point defense), anti-submarine warfare (sonar, ASW aircraft and ship-based helicopters), mines and blockades, amphibious assault and sealift (units in transit landing at a coast cell or port; heavy equipment moves this way), and supply at sea (fuel and stores, replenishment ships).
- **Weather:** sea state and storms per zone through modifiers.
- **Map problem:** eight of 16 chokepoints, including Dover, Taiwan and Korea, have both ends in one large sea zone, so they are not real edges. Naval gameplay needs finer zones near coasts and chokepoints. This is a map-builder change to plan before relying on naval operations.
- **Amphibious ZOC and coastal operations** use the coast links between sea zones and coast cells. Shore bombardment and coastal missiles use the existing range and effect systems.

## 10. Information and fog of war

- **Principle:** every country sees the world only through what its sensors and intelligence detect. The UI and the AI read only a per-country view, never the true state. The AI uses the same information as the player (cheating AI could come later as a difficulty option).
- **Full model:** four knowledge levels (unknown, detected, identified, detailed) and **contacts** with memory that fades.
- **Sensors** (eyes, radar, sonar, signals intelligence, recon aircraft, scouts) have a range, a type and a detection strength, defined in the catalog. Targets have a signature (size, stealth, submarine depth, camouflage). Weather, night, terrain masking and altitude band adjust the comparison. Rolls use the seeded generator in a fixed order. Satellites in 1990 are modeled as scheduled sweeps with a limited number of assets.
- **Contacts are state.** A record has observer country, target kind and ID, last-seen tick, knowledge level, last-known cell or zone, and a few bits of estimated strength. Stored in one sorted list ordered by observer, target kind, target ID (binary search, deterministic iteration). About 24 bytes a record. Stale position uncertainty is computed on read from age and the target's top speed. A monthly pass removes contacts older than a rules limit. Allied sharing is merged on read.
- **Visibility is a derived cache.** Keep a spatial index of entities by cell (the 64 km grid idea). Each sensor queries the entities in range, so cost is sensors times entities in range, not cells. Terrain, relief, weather, night and band adjust strength; there is no ray-traced line of sight. Hourly near fronts and in opened provinces, daily elsewhere. Not hashed; a test checks that rebuilding after a load gives identical contacts.
- **Read interface:** `World` internals are `pub(crate)` (comparable to `internal` in C#). The public API is `world.view(country)`, returning types like `KnownUnit` with optional fields filled only up to the knowledge level. A debug `omniscient` Cargo feature (comparable to a conditional compilation symbol) exposes everything and is enabled only for client debug builds and tests. The AI crate is compiled without it.
- **Commands that target an enemy** refer to the contact or a place (the contact's last-known cell), not a real unit ID. Rejection messages must not reveal hidden facts. A stale contact can send you to an empty cell.
- **Combat uses true state.** Fog changes decisions and surprise, not arithmetic. Who was aware of whom when a fight began is passed to the rules, so they can apply an ambush bonus (the size is gameplay). Fighting raises both sides' knowledge level.
- **Cost:** add a worst-case scenario (for example 20,000 entities and a few thousand sensors) to the benchmark harness and set budgets from measurements. Observers are independent, so the pass could run in parallel with results merged in fixed order. Add that only after measuring.

## 11. Ground control, fronts and stances

- **Zones of control (ZOC).** Each ground unit projects ZOC into cells around it, to a radius by unit type from data. Strength comes from current strength, cohesion and type, reduced by terrain. Per cell, the engine sums each side's strength:
  - **Held:** only one side projects into it.
  - **Contested:** both sides project into it.
  - **Empty:** neither does, and the cell keeps its previous controller.
- **Control changes through pressure** (section 12). ZOC fighting builds a signed pressure value on each contested cell, and the cell changes hands when it passes a threshold. A contested cell keeps its previous controller and is marked contested. A unit walking through empty cells does not flip them instantly. A cell holding an enemy unit cannot be taken by pressure.
- **Effects of contested cells (rules, later):** supply cannot pass or passes at reduced capacity; movement through enemy ZOC is slowed or must stop. Encirclement emerges when a unit is surrounded by enemy ZOC and its supply and retreat paths are cut.
- **Engine side:** ZOC strengths are a derived cache rebuilt from unit positions with a reused scratch buffer and a touched-cell list. Hourly near fronts, daily elsewhere. The controller array is stored. The contested flag is derived. ZOC is ground-only for now; naval and air projection may come later.
- **Fronts emerge from cell control.** A front is the set of boundary edges where cells controlled by one country touch cells of an enemy, plus contested cells. The engine provides a **derived front query**: boundary edges between two countries, grouped into contiguous **front segments**. The UI draws them as a band with hatched contested cells. Delegated commands use them ("hold this front segment", "attack along this segment"). The player sees what their country knows, with last-known control and "last seen" fade elsewhere.
- **Automatic engagement.** Opposing units in contact (adjacent cells or overlapping ZOC) fight automatically. Contact is low-intensity ZOC fighting; an attack order on an occupied cell is a direct engagement. See section 12. The province opens into cells, and brigades fight cell by cell on the tactical ticks.
- **Enemy units never share a cell, and there is no hard stack limit.** Supply is the brake (section 12).
- **Stances are attack and defend only.** Attack: seek contact, press toward the objective, use more ammunition, take higher losses, no fortification. Defend: hold the cell and dig in, with fortification growing over time. The default is defend. A headquarters sets a default stance for its subordinates and a unit's own stance overrides it. A `SetStance` command sets it (one small byte per unit, in hash and saves). Naval task forces get a similar field with values from data. Retreat is not a stance; it comes from morale and cohesion rules (a unit breaks and routs) or an explicit move order. The set can grow later as a data change.

## 12. Combat resolution

Combat is a gameplay system (G2). The formulas and numbers are rules. The engine provides the plumbing: engagement detection, fixed processing order, applying results, the unit status field, forced movement, prisoner counts, events and the seeded generator. Suggestion: when gameplay work starts, the rules live in a separate crate that implements the `Ruleset` trait, replacing the throwaway `proving-ground`, so `meridian-core` stays free of gameplay.

### Stacking and supply

- **No hard limit on friendly units per cell.** Supply is the brake. A cell's capacity is the throughput of the roads and rails into it, plus local food yield, unit buffers and any city stocks. Every unit adds demand for fuel, ammunition and food.
- When demand exceeds capacity, the shortage-splitting primitive divides what is available among the units in a fixed order. A unit that gets less than it needs suffers **attrition** (lost equipment and men, falling morale and cohesion). A big enough shortfall puts attrition at its maximum. Foraging also depletes local food and hurts civilians.
- **Enemy units never share a cell.** Every fight crosses a cell border.
- **Backups against doomstacks:** area strikes (artillery, air, missiles, nuclear) hit every unit in the cell; large stacks have a bigger signature and are easier to detect; unit buffers let a stack survive a few days for a short concentration; an optional combat-width rule (only some units fight at once, the rest are reserves) can come later as a gameplay rule.
- **Engine side:** a per-cell list of units (the spatial index supports it), a demand-aggregation pass for cells with units on the active set, the shortage splitting with a conservation test.

### Two kinds of fighting

| | ZOC fighting | Direct engagement (assault) |
| --- | --- | --- |
| Trigger | Overlapping zones of control: adjacent units that are not assaulting, and empty cells that both sides project into | An attack order on a cell an enemy unit is standing on |
| Who takes part | Every unit projecting into the area, including artillery, aircraft and helicopters in range | The defender, the attackers from adjacent cells, plus support fire |
| Intensity | Low and continuous (patrols, harassing fire, shelling) | High, per tick |
| Losses | Small real losses plus a steady cohesion and supply drain; contacts improve | The full equipment exchange |
| Result | **Pressure** on each contested cell: the side with stronger ZOC pushes it its way each tick, it decays when balanced, and the cell changes hands when it passes a threshold | Decisive: the defender is destroyed, driven out or holds; if the cell empties, an attacker in the attack stance can advance |
| State | Signed pressure per contested cell (sparse) | Engagement state per assaulted cell: principals on each side and the start tick |

- ZOC is never fought over directly. Units fight, and ZOC follows because ZOC strength comes from current strength and cohesion.
- **Frontage:** the shared border length between cells (`c_len_m`) and terrain can limit how many attackers fit across an edge. The river flag on an edge adds a crossing penalty. Attacks from several adjacent cells give flanking and encirclement without special rules.
- A cell holding an enemy unit cannot be taken by pressure. It stays that unit's until the unit leaves or is broken.

### Supporters and principals

- **Principals:** the units directly in the assault, in the full high-intensity exchange.
- **Supporters:** units not in the assault whose ZOC or range reaches the cell. Support is **automatic**: fire support (artillery, helicopters, air defense in range, scaled by distance and supply), reserve weight (a fraction of power that grows as the fight drags on or the principal loses; defenders in the defend stance contribute less), and ZOC blocking (a breakthrough does not run free, because the vacated cell is still inside the neighbors' zones, so an attacker advancing into it finds a contested cell, not a captured one).
- Supporters also take and return low-intensity ZOC fire, so a unit can be inactive in the assault and still wear down.
- **Joining the assault is an order** by the player or a delegated headquarters, including in the middle of a fight. The unit becomes a principal on the next tick. Mid-fight orders are logged like any command.
- **Bystanders:** units out of range have no effect.
- Nobody is counted twice: principals are in the high-intensity exchange, the others in the ZOC exchange.

### What a tick computes

1. Find engagements. All of them are computed from the **start-of-tick state** and applied afterward, so processing order cannot bias results.
2. Gather support fire into each engagement.
3. Compute each side's power from its equipment: attack values by target class (soft, light armor, heavy armor, aircraft by band, structures) against defense values from the specs, looping only over equipment types present. Scale by the modifiers below.
4. Exchange fire. Damage becomes losses split into destroyed, damaged (repairable) and personnel casualties, in integer math. **Bounded random variance** from the seeded generator, drawn in a fixed order: a clearly stronger force almost always wins.
5. Update unit state: morale, cohesion, veterancy, ammunition and fuel use.
6. Resolve the cell (retreat, advance, control and ZOC follow).
7. Emit events for the client and upgrade contacts for both sides.

Damaged equipment goes to a repair pool and returns slowly with maintenance and supply. Wounded recover slowly. Replacements arrive through supply and production with the veterancy dilution rule.

### Factors (modifier categories)

Modifiers inside a category add up. Categories multiply in a fixed order, each with a cap.

- Training (veterancy, morale, cohesion, readiness).
- Terrain, weather and **daylight**. Daylight comes from the clock and each cell's latitude and longitude via integer lookup tables, including polar day and night. The effect depends on **equipment** (night-vision and thermal-sight specs per model, and the night capability of aircraft and helicopters).
- Fortification (directional, section 13).
- **Doctrine** (below).
- **Technology** covers what is not equipment: command and control, electronic warfare, logistics, training methods. Technology is designed in section 18: fields with 0 to 100 levels, specific development programs and a per-country unlocked-model set (a bitset in the state) that gives modifiers or unlocks equipment models.
- Command (commander traits) and unit traits.
- **Country traits:** traits attached to the whole country using the same trait mechanism, for example professional army, mass conscription, strong logistics, corruption. Some are permanent national character, others can be gained or lost through events and reforms (sections 15 and 19).
- Stance, support and surprise (who was aware of whom when the fight began; the size of the bonus is a rule).

**Doctrine can change.** The state per country and per track (land, air, naval) is the current doctrine, a maturity value and optionally a target doctrine with a transition timer. It develops through combat experience, training and exercises, allied or enemy influence, and commander traits. A `SetDoctrine` command sets a target. The switch takes a transition period with penalties (lower cohesion and readiness) and maturity drops. A monthly update on a few numbers per country.

**Modifier trace.** The engine can optionally record, per engagement, what each modifier contributed, for UI tooltips. It is diagnostic only and never hashed. Every factor must have a visible effect.

### Retreat, rout, withdraw, hold, fight until death and surrender

- **Lost battle means retreat, and a retreat is orderly.** The engagement result decides the loss: the defender is driven out, or the attacker is repelled. There is no separate mid-fight break check. The loser retreats in order and **leaves equipment behind**, more of the heavy and fuel-starved equipment.
- **Rout is decided at the start of the fight.** The engagement stores each principal's cohesion, morale and strength at the start. A unit that started below the rout thresholds and then loses is **routed**: it retreats uncontrolled, leaves much more equipment, and cannot take orders until it recovers.
- **Retreat destination:** chosen deterministically from neighbor cells that are friendly-controlled or inside friendly ZOC and free of enemies, scored by distance from the enemy and closeness to supply and headquarters, ties by cell ID. A forced move at reduced speed. Conflicts resolve by unit ID. Retreat decisions are state-derived system actions, not logged commands.
- **No separate pursuit system.** Exploitation is the player ordering units to advance. The cost of retreating under pressure is covered by equipment left behind and by the disengage check.
- **Withdraw** is an order for a **defending** unit. Success depends on whether it can disengage: enemy mobility against its own, whether retreat cells are inside enemy ZOC, terrain, weather, fuel, cohesion, and commander traits such as a good rearguard. Success is a clean break with a small cohesion cost. Failure keeps the unit in the fight and exposes it to pursuit fire. Deterministic score plus a small random factor. Attackers stop by changing stance or cancelling the attack order.
- **Hold at all costs** is an order for any unit that the player can cancel. A unit under it **cannot retreat in order**: if it loses a battle it is routed. It does not prevent surrender. A hold bonus is a rules decision.
- **Fight until death** is a unit trait or a commander trait (with scope). Such a unit never retreats and never surrenders. It stands its ground until destroyed, and the attacker pays a last-stand cost in losses. No prisoners.
- **Surrender** applies to **encircled** units (no valid retreat cell, derived each tick from control and ZOC; a small days-encircled counter per unit).
  - Unengaged and starving: morale falls with supply shortage and attrition. As days pass and conditions worsen, the unit reaches its surrender threshold. A high-morale unit holds out longer.
  - Engaged: the threshold shifts toward surrender, by an amount that grows as supply gets worse, so a starving unit under attack is much closer to giving up.
  - Also surrenders if it loses a battle while encircled.
  - Survivors go to the captor's **prisoner pool** (a count per captor and origin country, in the hash). Equipment is captured or destroyed by the rules.

| | Lost battle, retreat available | Lost battle, no retreat | Encircled and starving |
| --- | --- | --- | --- |
| Normal unit | Orderly retreat; routed if it started under the thresholds | Surrenders | Surrenders by morale and conditions |
| Hold at all costs (order) | Cannot retreat in order: routed, heavy equipment losses | Surrenders | Surrenders by morale and conditions |
| Fight until death (trait) | Stands its ground until destroyed | Destroyed | Never surrenders; fights until destroyed |

## 13. Terrain and special cells

### Terrain classes

The map has six classes (plains, hills, mountains, high mountains, desert, ice) plus road, rail, river and coast flags. Forest, jungle, marsh and farmland need a land-cover dataset (candidate: ESA WorldCover, license to verify). Effects are qualitative here; the numbers are rules, and everything works through the modifier mechanism.

| Terrain | Movement | Combat and detection | Supply and food |
| --- | --- | --- | --- |
| Plains | Fast, vehicles at full speed | Open ground favors armor, little cover, easy to detect | Best capacity, supports large forces |
| Hills | Moderate, vehicles slower | Defender bonus, artillery observation advantage | Moderate |
| Mountains | Slow, mostly along passes | Strong defense, limits heavy equipment, severe weather, mountain-trained trait matters | Few roads, small sustainable forces |
| High mountains | Near-impassable except passes | Extreme; helicopters limited by altitude | Tiny capacity |
| Desert | Fast for vehicles | Wide visibility, sandstorms, heat | No local food; high fuel and water demand |
| Ice and tundra | Slow, snow ground condition | Equipment and cold problems, little cover | No local food, almost no population |
| Forest (needs data) | Slow for vehicles | Strong cover, hard to detect, favors infantry, less effective air | Moderate |
| Jungle (needs data) | Very slow | Strongest cover, disease attrition | Hard to supply |
| Marsh (needs data) | Impassable to heavy vehicles unless dry or frozen | Cover, ground condition decides | Poor |
| Farmland (needs data) | Fast, mud-prone | Open | Rich food, high foraging yield |

### Cities and garrisons

A city is a bundle of distinct properties, not a tile with a defense number. The map already has a built-up level per cell, 1,581 city sites and modeled population.

- **Supply hub:** city sites hold real stockpiles of fuel, ammunition and food, larger for bigger cities, plus high link capacity from road and rail convergence. Units inside draw from the stocks. Capturing a city captures what is left.
- **Economy:** factories, refineries and ports are sites with condition values. Holding the city yields production (G1) and bombing cuts output.
- **Population:** large civilian populations consume food. Fighting there kills civilians, creates refugees and starves the city faster. A besieged city starves both garrison and people.
- **Defense:** the defender's bonus scales with the built-up level. Vehicles and heavy equipment are penalized, so infantry and engineers matter more.
- **Cover:** units in dense urban cells are harder to detect.
- **Bombardment resistance:** fire hurts dug-in defenders less but raises damage, and rubble helps defending infantry and hinders vehicles.
- **Assaults are slow and costly:** limited approach axes, lower intensity per tick but sustained, higher attacker losses. Cities start with a higher fortification base.
- **Political weight:** capitals and major cities affect morale and stability (G3).
- **Scale:** cells are about 15 km across. A mid-size city sits in one cell and a metropolis spans several. Thresholds for town, city and metropolis come from the data. City sizes and built-up levels come from modern data, so some cities were much smaller in 1990 and need checking.
- **Difficulty comes from what is actually there.** A big city sustains whole divisions or army corps thanks to its stocks and capacity. A small town with only its garrison falls easily to a brigade.
- **Garrisons.** Every settlement has a fixed garrison attached to it. It cannot leave, fights as a defender with the urban modifiers, and is equipment-based (small arms, light anti-tank, MANPADS or AAA, police and territorial defense forces). Its size scales with population and the country's template, and with doctrine and traits (a territorial-defense country has stronger garrisons). Readiness can raise it in wartime. If it is destroyed and the attacker takes the cell, control flips and survivors become prisoners or disperse. No individual commander by default. Resistance or insurgency from the population is a later hook (G3).
- **Tiers to keep state small:** cities and sites have explicit garrison records in the unit table; towns above about 10,000 to 20,000 people get a garrison created on first contact by a deterministic function of the cell's population, built-up level and the country template (no random numbers); villages and rural cells have none, only a small friction in the cell's ZOC.
- **Data needed:** a settlement layer for towns (candidate: GeoNames, which I believe is CC BY 4.0; verify the license). The tens of thousands of entries are not stored as units.

### Rivers and bridges

- **Two classes on cell edges: small and major.** Today the cell graph has a single river flag on 48,163 edges, so the map builder needs a classification from a source attribute (not checked what Natural Earth offers; it may include a scale rank; fallback: classify by size).
- **Small river:** crossing allowed with major movement and attack penalties. Specialized units (amphibious vehicles, assault engineers, marines) cut or remove the penalty.
- **Major river:** only over a **bridge** or with **bridging equipment**. Air movement (helicopter lift, airlift, paradrops) bypasses edges.
- **Permanent bridges** are baked in wherever a road or railway meets a river edge.
- **Laid bridges** are built by units with bridging equipment on an edge next to them. Building takes hours, needs engineers on site, and is exposed to fire. Bridging equipment is a scarce, expensive catalog item with a span, a capacity and a build time. A laid bridge can be **dismantled** by its owner to recover the equipment for the next crossing.
- **Network attachment:** both kinds attach to the road or rail network, and link capacity across the river edge equals the bridge's capacity. A destroyed bridge has capacity zero, which cuts supply and rail movement there.
- **Destruction:** bombardment, air and missile strikes, and demolition reduce a bridge's condition. Engineers can prepare a bridge for demolition and blow it during a retreat, and repair it over time. A bridge captured intact is yours.
- **On the map:** bridges, permanent and laid, are drawn from the state.
- **Engine and data:** river class on edges; a baked bridge layer (road and rail crossings) plus a dynamic table of bridge condition and laid bridges, in the state and hash; movement profiles so pathfinding costs take a unit's crossing ability into account.

### Fortifications

- **Field fortifications:** a value (0 to 255) grows while a unit holds the cell in the defend stance, at a rate set by its engineers and time spent. It stops when the unit attacks or leaves. Enemy fire reduces it.
- **Directional.** Values are stored **per neighbor edge**, in a sparse table kept only for fortified cells (active set). Units build by default toward the neighbor with the most enemy ZOC, or toward an edge chosen with a Fortify order. An attack gets the full value on the edge it comes through, a fraction on flank edges and little from the rear. Direction comes from the integer bearing between cell centers (deterministic, lookup tables).
- **Captured works face the wrong way:** after a cell changes hands, the works still point at the old enemy, which is now the new owner's own side. They help only after being reoriented, which takes time.
- **Permanent works** (bunker lines, fortress sites such as the Korean DMZ and the Golan) are baked with their edges, are sites with condition values, and start with a high value.
- **Minefields** use the mine hazard flag. They slow movement, cause attrition, and are cleared by engineers over time.
- **Breaching:** assault engineers and breaching equipment cut the effective value for the attacker; artillery and air strikes reduce the real value over time.
- **Ownership and upkeep:** on a change of control the value decays and is partly captured. Unmanned works slowly decay.

### Amphibious landings

- **Suitability class per coast cell:** none, poor, fair or good (graded), baked by the map builder from terrain, relief and coast flags (cliffs, high mountains and ice are none). Ports are a separate best case once captured. Beaches are not in our data: OpenStreetMap has beach tags but its license (ODbL) has share-alike terms, so check before use. Alternatives are deriving suitability from terrain and hand-picking well-known landing areas as strategic sites.
- **Embark:** units load at ports onto amphibious ships and sail the sea-zone graph in transit. Heavy units need ships with the capacity (a lift cost, like airlift).
- **Landing builds up over time.** From the adjacent sea zone each tick, landing capacity sets how many units come ashore. Capacity depends on landing craft count, the suitability class, sea state and weather. Whole units land in a fixed order and the rest wait aboard.
- **The first units fight disorganized** with a cohesion penalty. Coastal defenses on the sea-facing edge (directional fortification, mines, coastal artillery) hurt them, while naval gunfire and air cover help.
- **Embarked units are vulnerable** to anti-ship missiles, submarines and aircraft while they wait, so sea control matters.
- **Supply:** landed units live on shipborne supply for a few days until they capture a port (which opens sealift) or set up low-capacity logistics over the beach.
- **Engine:** a suitability class per coast cell, landing capacity and lift cost as catalog numbers, a landing queue with a transit state, and an Amphibious Assault order naming the target cell and force.

## 14. Economy and population identity

Decided 2026-11-09. The approach is a light Victoria 3: enough economy to make war cost something and to give countries real strengths and weaknesses, without a full market simulation. War is a major core of the game, but only one part of it.

### Goods

- **About 14 major goods**, defined in data: food, crude oil, natural gas, coal, iron ore, non-ferrous ores, steel, fuel, chemicals, machinery and electronics, consumer goods, ammunition, electricity, military equipment. The engine never hardcodes a good by name, and the first list can change.
- **Extraction and refining are separate.** Oil is extracted in one place and refined into fuel in another, so a country can have oil and no fuel. There is one fuel good.
- **Supply classes link in.** Fuel, ammunition and food are also military supply classes (section 4). The economy produces them and the supply system moves them to units.

### Buildings (sites on the map)

- Buildings are **levelled records**: one per building type per cell, with a level, a condition, an owner and a capacity. A cell can hold several buildings of different types.
- **Capacity is fixed.** Resource extraction does not deplete. Deposits and supporting conditions limit where a building can stand (a port needs coast, a mine needs a deposit).
- **Buildings are valid targets** for bombing, shelling and sabotage. Damage lowers condition and output, repair costs goods and labor, and a destroyed building loses levels.
- Recipes (inputs, outputs, labor, base capacity per level) are data. Real deposits and industrial sites come from data sources that still need license checks.

### Regions and the market

- **The region is the economic unit**: a stock of each good, a labor pool and local demand. Regions already exist as a layer above provinces.
- **Prices are banded and dynamic.** Each good has a base price in data. A country's price moves with its supply-to-demand ratio, kept inside a fixed band (for example 50% to 300% of base), and a **world price** comes from global supply and demand. Updates are daily or weekly, never hourly, in fixed-point and in stable order.
- **Regional prices** exist inside the national band, so a cut-off region sees prices spike while the rest of the country does not.
- Prices drive **trade** (exports and imports where the price gap beats route cost and tariffs), the **treasury**, and **AI building choices and player signals** (a building is profitable when output value beats input and labor cost).

### Moving goods

- **Inside a country**: goods move between regions along the shared road, rail and port network, reusing the supply propagation primitive. Links have capacity, and bombing, bridge destruction and port damage cut it. Surplus ships to deficit by price gap.
- **The military and the civilian economy share the network.** Heavy troop supply competes with freight, and a cut line hurts both.
- **Between countries**: explicit **trade routes** over ports, sea zones, borders and pipelines. Blockades, embargoes and tariffs act on routes directly. Merchant shipping is an aggregate **tonnage** per country and route, not individual ships.
- Flows are double-buffered with a conservation test (nothing created or lost except production, consumption, construction and explicit destruction).

### Labor and population

- **Buildings need workers** from the working-age population of their region. Output equals capacity times staffing, so a half-staffed refinery makes half the fuel.
- **Drafting, casualties and refugees** draw on the same pool. Mobilization hurts industry and food, which is the main link from war back to the economy.
- Leftover labor is **unemployment**, which feeds standard of living and political support. There are **no wages** in this version. Strata are an **open question** and labor is not split by profession.
- **Standard of living** comes from needs satisfaction (goods available against goods needed). The **recruitment pool** comes from the working-age population.

### Population identity

- **Option 2 was chosen**: population is stored as **group records**, replacing the plain per-cell number in section 4. A record holds a cell, a culture, a religion, a head count and **political support shares** across a small data-defined set of ideologies.
- Hunger, births, deaths and refugee flows run **per group**. Refugees keep their identity when they move. Cell and region totals are derived.
- Political support changes slowly (monthly) with events, policy, living standards, war and occupation. The rules belong to the politics layer (G3).
- **Size estimate (not measured):** roughly one record per inhabited cell plus extra records in mixed areas, perhaps 600,000 records at 16 to 24 bytes, about 10 to 15 MB. Measure it in Phase 4.
- A **stratum dimension** can be added to the record later without redoing the design.
- **Data is the hard part.** Country-level shares are available, but sub-national culture, religion and political data for 1990 is scarce. The first version is modeled from country shares and regional assumptions. Candidate sources (CIA World Factbook, GeoEPR, Atlas Narodov Mira, academic religion datasets) are all unverified for license and coverage.

### Construction

- A construction project is a **command** (cell, building type, target level), so it is logged in replays and the AI uses the same path.
- Each type has a **cost in goods**, a **labor requirement** and a **minimum build time**, all data. Progress draws goods and labor from the region and slows or stalls when they are short.
- Projects sit in a **region queue** with player priority, limited by the construction labor the region can spare. Only regions with active projects are processed.
- Upgrade, downgrade, demolish and repair use the same path (repair is cheaper than new).

### Government finance

- **Money model:** a treasury, debt with an interest rate set by a rating, printing money (which raises an inflation stat), GDP as a derived figure, and loans and aid between countries. **One world currency and no exchange rates.**
- **Income:** one national tax rate on the value of regional output (derived GDP times rate), tariffs per good, and sales from state-owned buildings. Higher taxes lower living standards and support.
- **Spending:** military upkeep driven by fielded equipment and personnel (mobilized units cost more than reserves), state construction, subsidies on chosen goods, and debt interest.
- A deficit is covered by borrowing. When borrowing fails, the country prints money, which raises inflation and then prices.
- **Interest groups belong to the politics layer (G3).** The economy exposes only policy levers (tax rate, tariffs, subsidies, budget priorities).

### Military production

- Every equipment model has a **bill of materials** in goods, a **labor requirement** and a **build time**, so expensive models are scarce because the economy makes them scarce, not because of an abstract production number.
- Specific **building types** produce specific categories (arms plants, vehicle plants, aircraft plants, shipyards, missile plants). Capturing or bombing them matters.
- A **production order** is a command: model, quantity or continuous, priority. The engine assigns it to eligible buildings by priority within capacity and staffing.
- Output goes to a **national equipment stockpile** keyed by model. Units draw from it to reach their authorized TO&E (section 5). Ammunition is an ordinary good in the same flow.
- Equipment can be imported, exported or given as aid, subject to routes and embargoes. Production competes with civilian use and construction, so hard mobilization lowers consumer goods and living standards.
- Which models are available comes from the tech and doctrine systems.

### Engine implications

- **Extensible state tables.** The ruleset declares tables (buildings, stocks, prices, projects, orders, population groups) and the world owns, stores and hashes them. The engine still knows no specific goods.
- Deterministic and in stable order throughout: fixed-point, no floats, no hash-map iteration, double-buffered flows, active sets for regions and projects, conservation tests for goods and money.
- The market, price and treasury systems run at daily, weekly or monthly rates through the Phase 4 scheduler, never hourly.

### Still open

- Population strata (professions or classes).
- Data sources and licenses for resources, industry, trade and population identity: checked 2026-11-09 for the Gulf slice in `docs/data-sources.md` (USGS verified public domain; FAOSTAT has a promotion clause; GeoEPR is non-commercial; World Bank and Factbook terms still unverified; no oil field or industry source chosen yet).
- Region size and count (the regions layer was sized for the AI and UI, not yet checked for the economy).
- Good list, price bands, tax and tariff numbers, upkeep costs, recipes, unemployment effects, inflation rules.
- How tech and doctrine unlock models and buildings is decided in section 18.

## 15. Politics, war levels and internal conflict

Decided 2026-11-09. Politics is meant to be fairly complex. Like the economy, the engine stores generic data-defined tables and runs systems in order; formulas, thresholds and content are rules and data (the G3 backlog). Diplomacy is in section 16, including casus belli.

### Core pieces (all data-defined entries)

- **Population groups are the raw material** (section 14): culture, religion, head count, political support shares, and an unrest level per group.
- **Factions** (military, industrialists, labor, religious establishment, regional or ethnic blocs, parties). Faction strength is derived from the groups behind it, weighted by the regime. Each faction has an approval of the government, stakes in particular laws, and its own **cohesion**.
- **Regimes** are bundles: who holds power, how power changes hands, how much each faction counts, which laws are available. Monarchy, junta, one-party state and democracy are data, not code.
- **Laws** are categories (conscription, economic model, trade, press, civil rights, mobilization) with options. Options feed the levers the economy and military already expose (tax rate, tariffs, subsidies, draft).
- **Legitimacy** is how widely the regime's right to rule is accepted. It is a **stored integer from 0 to 100 per country**. Each regime entry defines in data what raises and lowers it and its own **illegitimacy threshold**, which is also the **collapse threshold** (a junta may survive at 15 where a democracy loses its right to rule at 40). War losses, hunger and broken promises erode it. Below the threshold the government is illegitimate: that unlocks the revolution rung, speeds up defections and makes coups far more likely, but the regime stays in power while it holds the army and the capital. Collapse happens when legitimacy stays below the threshold and the movement wins, or the army stops backing the government.
- **Stability** is the short-term order, derived from living standards, unrest, legitimacy, faction approval and occupation.
- **Cohesion comes in two kinds:** national cohesion per region (how well a region is bound to the nation, derived from its culture and religion mix, tension between groups and treatment by the government) and faction cohesion (how united a faction is; low cohesion means it splits or loses power).
- Everything possible is derived, not stored. Stored: group unrest, faction approval and cohesion, legitimacy, law choices and transitions, loyalty values.

### Changing laws

- A law change is a **command**, logged in replays; the AI uses the same path. It starts a **transition** of some time, during which old and new rules partly apply.
- It **passes or stalls by faction support**: the derived power balance (supporting factions against opposing ones) against a threshold set by the law and the regime. Constitutional changes need more than tax tweaks.
- Factions that lose become less approving, and the unhappiest push back. A regime that forces a change through pays in legitimacy and in the cohesion of the groups it crushes.
- **There is no separate political-capital currency.** The balance of power is the resource.

### War levels (politics type of war)

- A **ladder of data-defined levels:** insurgency and covert action, limited war, total war, nuclear war. Each level defines the allowed order types, target classes, force shares and areas.
  - *Insurgency and covert:* tiny force cap, no declared war, little political cost.
  - *Limited war:* a designated area (set of regions or cells) and a cap on the share of the army. Orders outside it are refused.
  - *Total war:* no area or force caps. Strategic bombing and attacks on the economy are allowed. Pushes mobilization and the economy hard.
  - *Nuclear war:* unlocks nuclear release on top of the existing gating (section 8).
- **The level is set per side.** Each country's orders are validated against its own level, so a conventional army can fight an insurgent in the same war, and one side can escalate while the other stays limited. The AI follows the same limits.
- **Gating by regime, laws and country traits.** Entry, speed and cost depend on them (a junta escalates fast; a democracy needs legitimacy and a mobilization law). Traits can restrict target classes.
- **Escalation is a command** with a political price (stability, legitimacy, faction approval). Events can also force it (homeland attacked, heavy losses, a rival escalating). De-escalation is possible with its own costs.
- **Civilian harm:** a strike has a chance of hitting civilians, by target and weapon. A hit fires an **event** that changes legitimacy, faction approval, cohesion of the affected groups and international standing. It uses the Phase 4 event queue and trigger and effect data.

### Internal conflict ladder: unrest, insurrection, revolution, civil war

Each rung is a data entry with entry conditions (stability, legitimacy, regional and faction cohesion, approval), effects and ways back down. Each has a **regional scope**, so one region can be in insurrection while the rest is only in unrest.

1. **Unrest.** No armed groups. **Unrest is stored per population group record** (about 1 byte each); region and country figures are derived. It moves gradually toward a target set by hunger and unmet needs, unemployment, faction approval, legitimacy and cohesion, war weariness, occupation and discrimination, and recent events or laws. Garrisons and concessions reduce it. Effects: lower output, tax and recruitment, more faction demands, and the entry check for insurrection.
2. **Insurrection.** High unrest plus capability (arms, helpful terrain, enough people, outside support) forms small armed bands that use the normal unit tables at small scale. They hide among the population (the information model applies), do guerrilla attacks, sabotage of buildings, rails and bridges, supply raids, terror and recruiting, and hold no territory. The government answers with garrisons, search-and-clear operations and concessions. Heavy repression helps in the short term and raises unrest in the long term.
3. **Revolution.** Aims to replace the regime. Several groups or regions join, led by a faction. It holds territory, runs its own supply, takes over buildings (their output and tax go to the movement), and raises larger units. A coup is the fast version (see below). **Defections are per unit and per officer:** each has a loyalty derived from faction, group support, pay, supply and recent events, and individual units can switch sides with their equipment as legitimacy falls. Revolutions can also be **started by events** (defined later). The movement wins by taking the capital or by regime collapse (legitimacy below the regime's threshold with the army not backing the government); the regime then changes along a data-defined path. The government wins by crushing it or by conceding to peel off groups.
   - **Coup types** are data-defined entries; each defines its initiators (which factions can start it), resources, resolution rule and outcomes (leader change, ruling faction change, regime change path, purges). A coup is **played out with real units**, not a roll. Three types:
     - *Military coup:* a military faction with low approval acts when the government is near or below its legitimacy threshold and enough units are loyal to the faction. Every unit and officer whose loyalty to the faction exceeds loyalty to the government defects at once, in one tick, with its equipment. It is then fought as ordinary combat around the capital and key buildings. It succeeds if the plotters hold the capital for some days or the loyal units stand down. A failure costs the faction approval, cohesion and officers (purges).
     - *Cabinet coup:* a faction already inside the government takes over the seat of power with few or no units. It needs a majority of the internal power balance **and** the guard units at the seat of power not staying loyal to the incumbent. If the guard defects or stands down it succeeds quietly (a leader or ruling-faction change, usually without a change of regime type). If the guard holds it fails or becomes a military coup.
     - *Foreign coup:* sponsored by another country through the covert war level. The sponsor funds and arms a faction or movement with commands (money, equipment, advisers, special forces), and the coup itself runs as one of the two types above with the sponsor's help added. Exposure costs the sponsor standing, and failure can push the target into a war level.
   - **Movement goals** are data-defined (autonomy, independence, regime change to a named regime, policy change, a place in government). A movement takes its goal from the faction leading it and the groups behind it; in a coalition the strongest faction sets the goal and the others' demands become concessions. **The goal is fixed at formation.** If the movement's faction cohesion falls, the coalition **fractures** into separate movements, each with its own goal and a share of the units and territory.
4. **Civil war.** If neither side can win, or the ruling entity refuses to yield, the country splits.
   - **The moment of decision.** When a revolution meets its win conditions (the movement holds the capital, or the regime has collapsed: legitimacy below the regime's threshold and the army no longer backing the government), a **raised critical event** goes to the ruling entity with options: *yield* (power transfers along the regime's data-defined path; the old leadership's fate, such as exile, trial or a retained position, comes from the regime and the movement's goal), *negotiate a transition* (a partial path such as a coalition government, reforms or elections, offered only when the movement's goal and the factions allow it; costs legitimacy but avoids war), or *resist* (the country splits as described below).
   - The AI picks by data-defined weights (loyal units, foreign backing, regime type, faction positions, the leader's traits). A player decides themselves. If there is no answer by the deadline, the **default is derived:** resist if loyal forces still hold the capital and the army's backing is above a threshold, otherwise yield. Foreign support to either side (covert action, aid, UN stance) enters the derived numbers and the AI's weights.
   - This reuses events, severity, the AI's scoring and the civil war split, so it needs almost no new machinery.

### Rebel actors

- **Movement entity**, promoted to a country at civil war. A lightweight record: goal (autonomy, independence, regime change, policy change), groups and factions behind it, strength, morale, operating regions, and its bands, drawn from the same unit tables. AI-controlled by default. It uses normal supply and combat. Promotion keeps its units and state.

### The civil war split

- The movement becomes a **special revolutionary country** until the civil war is resolved. It is a data-defined country kind with its own dynamics, which are modeled later (they are not designed yet). It is still a full country in the tables: it has an ID, stocks, a treasury, units and a war level. The **side holding the recognized government and capital continues as the original country** and keeps the ID, name, treaties, alliances and debt record. The breakaway becomes a new country. If the capital changes hands, the continuing identity follows it.
- Assets are divided deterministically: regions and cells by control; units, officers and pilots by loyalty; buildings and stockpiles by region; population groups by cell; treasury and debt by share of population or output of the regions held (the rule is in the backlog). Each side starts at its own war level.
- Conservation tests cover people, goods, money and units: they are moved, never created or lost.
- Endings: one side takes the other's capital or collapses, a settlement (diplomacy, later), or a lasting split.

### Engine implications

- A data-defined country kind (normal, revolutionary) that rules can read, so the revolutionary rules can be added later without engine changes.
- Runtime country creation (new IDs and tables) and a deterministic split of regions, units, stockpiles, buildings, treasury and debt.
- Tables: factions, regimes, laws and law transitions, legitimacy and stability, per-group unrest, movement entities, per-unit and per-officer loyalty, war levels per side, event definitions. All owned and hashed by the world and declared by the ruleset.
- Systems at monthly or event rates: unrest update, faction approval and cohesion, law transitions, defections, movement strength, regime change. Command validation against the war level.
- Information model reused for insurgent contacts; the event queue reused for civilian harm and revolution triggers.

### Still open

- The events list (including what starts a revolution) and their effects. Coup conditions, the guard-unit definition and the purge rules are numbers and data.
- The dynamics of the special revolutionary country during a civil war (how it differs from a normal country, and what happens to it when the war is resolved).
- Lists of regimes, laws, factions and ideologies, and all thresholds and numbers.
- The treasury and debt split rule, regime change paths, and how a player or scenario can control a movement.
- How tech and doctrine tie to laws and regimes.

## 16. Diplomacy: treaties, alliances and the UN

Decided 2026-11-09. Casus belli and the end of wars are decided below, with the relations model. The three parts below are designed separately, on purpose: treaties as a generic data-defined mechanism, alliances in two tiers, and the UN as a purpose-built system.

### Treaties (minor and major)

- Every treaty is a **data-defined type** with parties, terms, duration, obligations and what happens when a party breaks it. Terms feed levers that already exist: tariffs, trade routes, basing and transit rights, war levels, supply and intelligence sharing.
- **Minor treaties** are cheap and reversible: trade deals, transit rights, non-aggression pacts, cooperation or research sharing, tariff cuts. A government signs one by command and it takes effect on the next tick. Ending one costs a small amount of standing.
- **Major treaties** reshape a country's position and last: alliance membership, mutual defense, foreign bases, arms limits (relevant in 1990), border settlements, unions, peace treaties. They have long terms and large costs for breaking them, and they affect legitimacy and factions.
- **Major treaties need domestic faction approval.** Signing is a command that starts a transition, like a law change. It passes or stalls by the domestic power balance against a threshold set by the treaty type and the regime. Each party passes its own approval, so a deal can die in one capital. A regime that forces a signature through pays in legitimacy.

### Alliances

- An alliance is an **object that members join** (joining is a major treaty). It has members with roles (leader, full member, associate), obligations that trigger in set situations, shared mechanics by tier (intelligence sharing, basing and transit rights, shared supply, joint command in tight alliances) and an **alliance cohesion** number. Cohesion falls when members ignore obligations, quarrel or go through political change, and can lead to a split or dissolution.
- **Two tiers.** *Major historic alliances* (NATO, the Warsaw Pact) get their own rules and events written in data: their obligations, command structures, internal tensions and scripted histories. In 1990 the Warsaw Pact is already coming apart, and that can play out through cohesion and events, not code. *Regional alliances* use a simpler data template with shared defense, trade and access terms.
- **Defense obligations open a call, and each member decides.** An attack on a member opens a window in which each member chooses whether to join the war. The AI uses the same rules as the player. Joining raises cohesion and standing. Refusing costs standing and cohesion, and repeated refusal can break the alliance. Domestic factions and war-level limits still apply, so a member may be unable to join.

### The UN (purpose-built)

A global forum in the spirit of Stellaris's Galactic Forum, with a 1990 flavor. Everything is data and commands, plus one small deterministic voting engine.

- **Members:** every country. Membership can be suspended or removed, and a new country (a civil war breakaway, a successor state) applies through a resolution.
- **Security Council:** permanent members with a veto (data, so a successor state can inherit a seat through an event) plus rotating seats. **Rotating seats follow a fixed schedule in data**, by region and in a set order, for fixed terms. Elections by the Assembly are a possible later expansion. Council resolutions are binding.
- **Assembly:** one vote per country, majority or two-thirds depending on the type. Resolutions are non-binding but move standing and legitimacy.
- **Agenda:** **incidents open sessions automatically** through data-defined triggers (an invasion, civilian harm above a threshold, nuclear use, a coup in a member state, an attack on peacekeepers). Members can also request a session or an item, using limited **agenda slots** per period so the AI cannot flood the forum.

**What is voted on.** A resolution is a **type** (which sets the body, voting rule and allowed clauses), a **target** (country, conflict or region), a **sponsor** and a list of **clauses**. A clause is a data-defined effect with parameters: demand an action, set a deadline, impose sanctions on a good or route, authorize force after a deadline, establish a peacekeeping mission (mandate area and level), require a report, set a review date. Members vote yes, no or abstain on the **whole package**. **There are no amendments** (a possible later expansion). Negotiation happens through lobbying before the vote, and a sponsor can withdraw and re-propose.

**Life of a resolution.** Proposal (command) → debate window of some days, during which members lobby (spend standing, aid or trade concessions, as commands) → vote at a fixed tick (players decide; the AI uses a derived score from relations, alliances, trade, domestic factions and its own interests, with bounded random variance; votes are public) → effects through existing levers, with an expiry or review date → compliance tracking.

**Enforcement.** Binding clauses are obligations on every member, and **each member chooses its compliance** by command, possibly partial. A member that ignores an embargo keeps trading with the target through hidden routes (less volume, higher cost) with a chance of exposure each period, based on the information model. Exposure costs standing and can lead to follow-up resolutions and sanctions. The target of a demand can comply, stall or defy. Sanctions are therefore leaky, as in real life.

**Peacekeeping (as in real life).**
- Members contribute units **voluntarily**, as commands. A mission can fail for lack of volunteers.
- A mission needs **host-country consent** unless it is an enforcement mandate. The mandate level is *monitoring* (self-defense only) or *robust* (force to protect civilians and the mission). The mandate has an area of cells.
- A contingent remains the contributor's unit in its own chain of command, with UN operational control for the mission. Contributors can attach **caveats**. An attack on peacekeepers fires an event with standing and legitimacy effects.
- **Reimbursement** is paid per soldier from the UN budget. Contributors gain standing.

**Budget.** A yearly **assessment** sets each member's dues as a share of its economy (derived from GDP). Dues go into a UN pool, which pays reimbursements, humanitarian aid and running costs. Non-payment costs standing and, after some time, the Assembly vote. A shortfall cuts reimbursements. The pool is covered by the money conservation test.

**Resolution types (data), examples.** Condemnation, ceasefire demand, sanctions, arms limits, humanitarian aid, peacekeeping mission, authorization of force (which gives a legitimate basis for war as a casus belli record, see the casus belli part of this section), admission or suspension of members.

**Example (Gulf crisis, 1990 style).** Type *authorization of force*, target the aggressor; clauses: demand withdrawal by a date, keep sanctions in place, authorize members to use all necessary means after the deadline. One vote, and a deadline that triggers effects on its own.

**Possible expansions later:** amendments during debate, elected rotating seats.

### Relations and standing

- **Relations (bilateral opinion).** For each pair of countries, how one feels about the other, as an integer on the same 0 to 200 scale as standing (100 is neutral). It has two parts: a **baseline derived from existing state** (regime and ideology similarity, shared borders and disputes, trade volume, alliances and treaties, cultural and religious ties, the tier gap), which is never stored, and **opinion modifiers** stored sparsely. A modifier records an event that happened (a treaty broken, aid given, a coup sponsored, civilian harm, a refused call to arms) with a size, a source and a decay time, so grudges fade. Only modifiers that exist are stored.
- **Standing (global reputation) is a number from 0 to 200, 100 neutral, and is derived.** It is the tier-weighted average of other countries' opinions of a country, calculated when needed and never stored. Data-defined bands over the scale (names and effects to be set) drive consequences: for example, a low band makes treaties hard to sign and a high band adds influence at the UN.
- **Standing bands (starting set, data-defined; names, cut points and effects can change without engine work).** Each band is an automatic country trait with a small margin (hysteresis) so a country at an edge does not flip each month. Standing and power tier are independent: a hegemon can be a pariah and a regional power can be esteemed.

  | Band | Range | Examples of effects |
  |---|---|---|
  | Pariah | 0 to 29 | Major treaties blocked except with allies, can be suspended at the UN, sanctions and coups against it cost others less, trade routes cost more |
  | Distrusted | 30 to 59 | Higher approval threshold for treaties, alliance offers refused more often, weaker lobbying |
  | Wary | 60 to 89 | Small penalties on treaty terms and trade |
  | Neutral | 90 to 110 | No effect |
  | Regarded | 111 to 140 | Small bonus on treaty terms and lobbying |
  | Respected | 141 to 170 | Cheaper treaties, stronger lobbying, easier alliance offers |
  | Esteemed | 171 to 200 | Extra UN agenda slot, can mediate between others, strong influence on votes |

  The effects are examples and the numbers are untested.
- **Actions change opinion only among the countries that learn about them,** through the information model. A hidden sanctions violation costs nothing until it is exposed, and an exposed one costs a lot among those who care. Each action's data names who reacts (observers) and by how much.
- **Opinion modifiers.** Ongoing conditions live in the derived baseline and are never stored; a modifier records a discrete event. A modifier type (data) defines:
  - its **size and sign**, scaled by the scale of the act;
  - **duration and decay** (linear over time, fixed-point), and a **residual**: most modifiers decay to zero, but data-flagged ones (nuclear use, a massacre, a treaty betrayal) decay only to a small permanent residual, so history is never fully forgiven;
  - **stacking** (refresh, add up to a cap, or replace);
  - **observers** (the victim in full; the victim's allies, neighbors and great powers in part; anyone who learned of it through the information model, a little), and **visibility** (public, or hidden until exposed);
  - a **category cap**, so no single issue can dominate a pair's opinion.
  - Starting list (examples, not final): *diplomatic* (treaty signed, treaty broken, recognition granted or withdrawn, call to arms answered or refused, alliance joined or left); *UN* (voted with or against, defied a binding resolution, contributed peacekeepers, sanction violation exposed); *military* (attacked or invaded, civilian harm, nuclear use, border incident, occupation, sponsoring rebels or a coup once exposed); *economic* (aid given, embargo imposed, debt default, trade deal signed); *humanitarian* (refugees accepted, a massacre).
- The AI's attitude toward a country follows from its own opinion of it.
- Opinion and standing feed treaties, alliances, UN voting scores, recognition, tier effects and casus belli.
- Cost: sparse modifier records per pair (the pair space is about 190 by 190), with decay processed at a slow rate.

### Country tiers and recognition

Two separate axes, because a strong state can be unrecognized and a recognized state can be weak.

- **Power tier (derived).** A data-defined ladder: hegemon, great power, regional power, and **minor** for everyone else. A country's tier comes from a score built from state the simulation already has: economy (GDP, industry), military (equipment and units), nuclear capability, reach (navy, air lift, bases) and alliances. Thresholds are data, and a margin (hysteresis) stops countries flickering between tiers. The number of countries per tier is not fixed, so 1990 can have two hegemons (the leaders of the two blocs).
- **Recognition (bilateral, with a derived status).** Each country decides whether to recognize each other country, by command, stored as sparse pair records (only recognitions that differ from the default are stored). Treaties and diplomacy between two countries need mutual recognition. A country's overall status is derived: recognized if enough countries recognize it (weighted for great powers and UN membership), unrecognized if too few, and partially recognized in between. Recognition can be withdrawn, and UN resolutions can push it. A breakaway or revolutionary country starts unrecognized. Unrecognized states cannot sign major treaties or join the UN.
- **What a tier gives you.** A tier is an **automatic country trait** (the existing trait and modifier mechanism) that grants modifiers (standing gains and costs, upkeep, escalation cost) **and unlocks actions**, all in data. Examples: *hegemon* can establish bases and security guarantees anywhere, lead a bloc, impose sanctions outside the UN at a cost, and run covert action more cheaply; *great power* can sponsor movements and coups and join the highest-level treaties; *regional power* can lead a regional alliance and exert influence in its region. Losing a tier loses its actions.
- Other hooks: permanent Council seats are data and may be tied to a tier; some treaty types need a minimum tier; alliance leadership goes to the highest tier; war-level and escalation costs scale with tier; the AI sets goals by tier.

### Casus belli and ending wars

- **A casus belli is a stored justification record** with a type, the holder, the target and an expiry. Types are data-defined, for example a border or territorial claim, defense of an ally or treaty obligation, liberation of an ethnic or religious group, preemption against an imminent threat, retaliation for an incident, UN authorization, and a humanitarian cause. Events create them (an incident, a border clash, an exposed covert action), and so do claims, treaty triggers and UN resolutions. A holder can also spend time and standing to **fabricate** one. Declaring a war consumes the record, and unused ones expire. Provocation is therefore a real strategy, and covert action and diplomacy have a reason to exist.
- **Effects.** A war declared with a valid casus belli costs little standing. Without one, observers apply opinion modifiers, domestic factions object (a legitimacy cost, harder for democracies), and the UN can respond. The casus belli also defines the **war goals**: what may be demanded in the peace (territory, regime change, reparations, disarmament, security guarantees).
- **A war ends by a negotiated peace, which is a major treaty** (so it needs faction approval on each side) with terms limited by the winner's war goals. Whether the other side accepts is **derived from the war position,** not from a stored score: occupied territory and capitals, casualties, economic damage, war weariness, legitimacy, stability and outside support. A government near collapse accepts harsher terms, a stalemate leads to a ceasefire at the line, and domestic politics can block a peace the leaders want. **Ceasefires and armistices** are lighter treaty types that stop fighting without settling terms. The UN can demand a ceasefire, and mediators can help.
- **No war score points.** There is no abstract currency beside the simulation.

### Engine implications

- Tables owned and hashed by the world: treaty instances and terms, alliance records with members and cohesion, UN bodies, proposals and votes. Treaty types, alliance entries and resolution types are data.
- A small, purpose-built voting engine for the UN (two voting rules, veto, rotating seats from a data schedule), kept deterministic: stable member order, votes resolved at a fixed point in the tick. A UN pool for dues and reimbursements, covered by the money conservation test. Incident triggers that open sessions, using the event queue.
- Command validation: treaties and resolutions are commands, checked against domestic approval, war level and membership.
- Treaty terms act on the economy and military through existing levers, never through special cases in the engine.

### Still open

- The relations model is decided; its modifier lists, baseline weights and decay times are data and rules. The casus belli type list, the war goal list, the fabrication cost, and the war position formula and acceptance thresholds are data and rules.
- The exact effects and numbers of the standing bands (the seven bands are decided), and the final list of opinion modifier types with their sizes, durations, residuals, observers and caps.
- Tier score weights and thresholds, the recognition thresholds, and the list of tier actions.
- Lists of treaty types, alliance entries, resolution types, clause types and UN membership details (seats schedule, admission), plus the dues formula, reimbursement rates, agenda-slot counts, detection chances and all thresholds.
- How AI countries choose diplomacy (AI design, G4).
- The events list for diplomatic incidents.

## 17. Intelligence

Decided 2026-11-09 (in progress). A strategic layer on top of the military information model (section 10). Section 10 covers contacts and fog of war on the battlefield. This section covers what countries know about each other's economies, politics, nuclear programs and plans, and the covert actions that go with it. It will be built out further.

### Where it connects

- **Information model:** the knowledge levels extend to strategic data, so what a country's view shows about a foreign economy or program depends on its intelligence.
- **Covert war level (section 15):** covert actions (sponsoring movements, foreign coups, sabotage) run through it.
- **Exposure:** detection of hidden sanctions violations and of covert action uses the same observer and exposure logic as opinion modifiers (section 16).
- **Politics:** factions, movements and defections are things intelligence can see, influence or miss.

### Agencies, capability and reach

- **Agencies** are data-defined, **one or more per country, flagged foreign, domestic or both** (like the KGB's directorates, the Stasi, MI5 and MI6, the FBI and the CIA), each with its own capability and budget. Separate agencies allow rivalry, a regime that relies on its security service, and capture of an agency by a faction, which feeds back into loyalty and coups. with capability by **discipline** (six, data-defined, so the list can change):
  - *Human sources:* spies, informants, defectors. Political and leadership intentions, faction plans, plots. Slow, risky, and the main route to influencing movements and coups.
  - *Signals:* intercepting communications. Military plans and movements, some diplomatic traffic. Quality depends on the target's encryption.
  - *Imagery and space:* satellites, reconnaissance aircraft. Military buildup, bases, missile sites, industry, nuclear facilities. Accurate on structures, not intentions. Technical monitoring such as nuclear and missile signatures sits here for now.
  - *Open sources:* public information. A cheap baseline view of any country, especially its economy.
  - *Counterintelligence:* catching enemy spies and protecting secrets. Lowers what others learn about you and raises their exposure risk.
  - *Covert action:* sabotage, propaganda, supporting movements and coups, special operations. Decides how well an agency carries out operations at the covert war level and how likely they are to succeed or be exposed. Collection and action are separate skills.
- **Capability is stored**, a level from 0 to 100 per discipline. The 1990 scenario data gives each agency its starting values, set by history with a note on the reasoning. Capability grows with investment (from the economy), experience from successful operations and tech, and falls with neglect, purges, defectors or a major exposure. It does **not** follow the country's power tier: a regional power such as Israel can have a worldwide agency.
- **Reach is derived:** where an agency can build networks and how easily. It comes from capability plus access: diplomatic presence, diaspora and ethnic ties through the population groups, alliances and intelligence-sharing treaties, bases, and regional position.
- **Reach is soft.** An agency can try to build a network anywhere. Where it has little access, growth is slow, costs are high and exposure risk is higher, so reach shows up as cost and risk, not as a wall.

### Networks and operations

- **Abstract networks, not individual agents.** Each agency has a **network strength per target country and discipline**, a stored number that grows with investment and shrinks with losses, capped by capability. There are no individual agent records.
- **Operations are named records** with a goal, resources, a duration and an outcome. Commands: *gather* (on a target and category), *covert action* (support a movement or coup, sabotage, propaganda), and *counterintelligence sweep*. Each has costs, risks and an exposure chance.
- **Counterintelligence:** the target's counterintelligence lowers your knowledge and raises your exposure risk.
- **Covert action types (data-defined; each has a skill requirement, costs, risk and effects through existing levers).** Support a movement (funds, arms, advisers); foreign coup (back a military or cabinet coup); sabotage (buildings, rails, bridges, stockpiles); propaganda (unrest, faction approval, legitimacy, opinion); election and political interference (faction support, law outcomes); sanctions busting and arms smuggling (the hidden routes from the UN design); special operations (raids, hostage rescue, seizing a target, small units at the covert war level); and **targeted removal** (eliminate a leader, officer or faction head). Targeted removal is rare and costly by design: the person is permanently removed and succession rules apply, exposure brings large opinion modifiers and possible UN sessions, and it can escalate to a war level.
- **Exposure and attribution are separate steps.** First the act is **noticed**, with a chance from the target's counterintelligence, the scale of the act and the sponsor's covert action skill. Then **attribution** gives the target a suspected sponsor with a confidence level, derived from its knowledge of that sponsor and the evidence left behind. **Attribution can be wrong**, so a false flag can send the target after the wrong country. Opinion modifiers (section 16) apply to whoever the observers believe did it, and incidents can open UN sessions.

### Internal operations

Intelligence also works at home. Internal operations connect to the politics layer (section 15).

- **A country always knows its own factions and groups exactly.** Faction approval, cohesion and group state are not estimates, so there is no "monitor" operation.
- **Internal operation types (data-defined):**
  - *Infiltrate movements:* reveals their members and plans, lowers their strength, discourages recruitment. Heavy infiltration can also provoke or radicalize a movement.
  - *Suppress and crack down:* cuts unrest and insurrection in the short term, but raises unrest in the targeted group in the long term and costs legitimacy and cohesion (as in section 15).
  - *Censorship and domestic propaganda:* shifts unrest, support and legitimacy.
  - *Purge the officer corps:* raises unit loyalty by removing disloyal officers, but loses experience and can push a faction to act first.
  - *Election control and political management:* in regimes that allow it.
  - *Counter-subversion:* catches foreign agents and foreign-backed movements inside the country (counterintelligence applied at home).
- Regime and laws decide what is allowed and what it costs: a security state has more tools and fewer limits than a democracy. Abuse is exposed through the same observer logic as other incidents.

### Intelligence views

- A view of a foreign country returns **estimates with a margin that shrinks as knowledge rises** (for example, steel output "about 40 Mt, give or take 25%"). The error is computed **deterministically** from the pair, the knowledge level and a coarse time bucket, so asking twice gives the same answer and repeated queries cannot be mined. The error is derived and never part of the hash.
- Low knowledge gives a wide range and high knowledge gives nearly the true value. A deception operation (the target feeding bad data) can bias the estimate. **The AI sees the same estimates.**

### Engine implications

- Tables owned and hashed by the world: agency capability per discipline, network strength per agency, target and discipline, and named operations.
- A deterministic estimate function for the view API (noise derived from a hash of pair, level and time bucket, never from the simulation random stream).
- Intelligence systems at slow rates (weekly or monthly); operations resolve at fixed points in the tick. Commands are logged in replays like all others.
- Reuses the information model, the event queue, opinion modifiers and the covert war level.

### Still open

- The numbers and effects of each covert action type. **Deception and turned networks (double agents) are deferred to a later expansion:** deception would bias the target's estimates through the deterministic estimate function, and a turned network would be a flag on a network record. Also open: defectors, counterintelligence details, and how intelligence feeds the AI.
- Starting capability values for 1990 agencies, and the numbers (growth, decay, exposure chances).
- Data sources for historical agency strength (judgment-based; verify and record confidence).

## 18. Technology

Decided 2026-11-09 (in progress). Tech gates what exists for each country, adds non-equipment effects, and connects to the economy, diplomacy and intelligence. It builds on the combat design (tech is one modifier category and covers non-equipment, section 12) and the equipment catalog (section 5).

### Fields and programs

- **Fields** are data-defined (for example aerospace, electronics, materials, nuclear, naval, ground systems, space). Each country has a **stored level per field**.
- **Starting set of 13 fields** (data-defined; a finer split can come later, for example separate radar, engines and armor):
  - *Military:* ground systems; aircraft; naval systems; missiles and guidance; electronics and sensors; nuclear.
  - *Dual-use:* space; computing and information; materials and manufacturing.
  - *Civil and support:* energy and heavy industry; agriculture and food; logistics and transport; medicine and public health.
  - Civil and dual-use fields raise economy numbers through the modifier mechanism, so peacetime investment pays off in war (for example agriculture feeds the food and hunger system). Chemical and biological weapons are not covered and would be a separate topic.
- **The level is a 0 to 100 progress number per country and field,** matching the other scales. Programs and unlocks name a required value (for example "aircraft 62 for a fourth-generation fighter"). The scenario data gives each country a starting value per field for 1990. The world frontier is the highest value anywhere, which the diffusion cap uses. Named eras (like "1970s level") are derived bands over the number, for display.
- A field level is a **prerequisite, not the unlock.** To get a specific aircraft, tank or ship, a country starts a **development program** for that model: a project with a cost, a duration of years and a team, drawing on the economy (goods, labor, money) like construction does. When it finishes, the model becomes available for production. **Programs suffer delays and cost overruns, not outright failure:** a bounded random draw from the seeded generator, at fixed points in the program, can stretch the schedule or raise the cost, by an amount that depends on how far the model is beyond the country's field level, its experience and its funding. A program is lost only if the player cancels it or funding collapses. The draws are logged and replayable. This keeps the principle of specific, expensive, scarce models.
- **Models in service in 1990 start unlocked** according to the catalog. Programs can be bought, licensed or stolen, so a country can field a model without having the field level to build one itself.
- Fields and programs also gate some **building types, laws and doctrine options**, through the same prerequisite mechanism. Doctrine remains a separate changeable state (section 12).

### How fields grow

- **Research buildings are specialized institutes, one building type per field** (an aerospace institute, a nuclear research center, an agricultural station and so on), levelled records on the map like any other. A building's output goes to its own field and depends on staffing (labor). The government sets how the research budget splits across fields (a policy lever), which decides funding and staffing priority. Because each institute is a real site, it can be bombed, sabotaged or targeted by intelligence, capturing one can yield progress, and a country with no institute in a field makes no home progress there. This is the base growth.
- **Learning by doing** adds to it: finishing programs, producing and operating equipment, and fighting wars raise the relevant field. A country at war, or with a large industry, learns faster in the right areas.
- Growth is **capped by diffusion:** a country cannot easily get far ahead of what exists in the world, and catching up is cheaper than inventing.

### Technology transfer

Transfers move through systems that already exist.

- **Licensed production** (a treaty), **exports and aid** (finished equipment, which can later be captured or copied), **espionage** (a covert operation steals a program or its progress), **reverse engineering** (captured equipment gives progress toward the matching program), and **blocks** (sanctions and UN embargo clauses can cut transfers).
- **A transfer gives the model (or progress on its program), not the field.** The recipient's field level does not rise; it only grows through its own research and experience. A country can field equipment it could not develop, but cannot build the next model, modify the licensed one, or keep it supplied without its own industry. A licensed jet is not an aerospace industry.

### Doctrine

- Doctrine is how a country fights, as distinct from what it fights with. It is a separate, changeable state per country (section 12) and feeds the doctrine modifier category in combat.
- **One doctrine per branch.** Tracks for land, air, naval and strategic (nuclear), each with a data-defined list. Examples for land in 1990: Soviet deep operations, AirLand Battle, mobile defense, people's war, territorial defense. A country has one active doctrine in each track.
- Each doctrine gives modifiers, favors certain unit types and orders, and may require field levels or experience (a doctrine is unlocked through the same prerequisite mechanism as models).
- **Changing doctrine** takes a long transition during which effectiveness drops, and it costs money and leader attention.
- Doctrines spread by alliance membership and training with other countries, which gives NATO and the Warsaw Pact their flavor.
- A tree of many small doctrine choices is a possible later expansion.

### Non-equipment effects

- Tech fields and finished programs can grant effects through the existing modifier mechanism (communications, logistics methods, medical care and similar), scoped like traits and counted in the tech category of combat.

### Engine implications

- Tables owned and hashed by the world: field level per country and field, development programs, research building records, and a per-country unlocked-model set (the per-country tech bitset already planned).
- Prerequisite checks as a generic mechanism over fields, programs, buildings, laws and doctrines.
- Monthly or yearly systems for research, learning by doing and diffusion caps; programs advance on the economy tick. All in fixed-point and stable order.
- Commands: StartProgram, CancelProgram, SetResearchBudget, license and transfer commands through the treaty system.

### Still open

- The list of fields, the level scale, the program parameters (cost, duration, failure and delay chances), research building types, the diffusion cap formula, the learning-by-doing rates, and the list of non-equipment effects.
- The doctrine lists per track, which field levels or experience each doctrine requires, the transition time and cost, and how doctrines spread through alliances and training.
- How the AI chooses programs and research allocation (AI design).
- Data: which programs and years to model for 1990 onward (judgment-based; record the source and confidence).

## 19. Events

Decided 2026-11-09 (in progress). One generic system for the incidents, crises and choices that other systems depend on: revolution triggers, UN sessions, civilian harm, diplomatic incidents, covert action fallout, coups and, later, casus belli. The content is data; the engine only runs the mechanism.

### Shape

- **An event is a data-defined entry** with: a **scope** (country, region, population group, unit or officer, a pair of countries, or the whole world), **trigger conditions** over state, a **chance or mean time to happen**, **options** (choices) with effects, and flags (once only, repeatable with a cooldown, hidden, news only).
- **Varied types come from scope and flavor, not from different code:** political and economic country events, regional and local events, unit and officer events, diplomatic incidents, crises, world events and plain news items.
- **Choices.** A player-facing event offers several options. A country controlled by the AI picks by **data-defined weights**. An event with no answer by its deadline takes a **default option**, so the simulation never waits for a person. Choices are commands, so they are logged in replays like all others.
- **Effects** come from a shared vocabulary that the rest of the engine uses: modifiers, opinion modifiers, flags, law and treaty changes, unit or building changes, stability, legitimacy and unrest changes, spawning a movement, and scheduling another event.
- **Event chains** are sequences with memory: a small stored state per scope (which step, flags, timers). An option can schedule the next event after a delay, so branches and consequences can run for years. A chain is just events that set state and schedule others.

### Multi-party events

- **Separate event instances per party, linked by a chain.** An incident creates an instance for each affected party, with its own text and options (for example the victim: protest, retaliate, take it to the UN; the accused: deny, apologize, escalate). One party's choice can schedule a follow-up for the other. Each party answers on its own deadline, an AI-controlled party answers without a special case, and replays stay simple.
- Shared events with multi-party resolution (a negotiation where combined choices produce the outcome) are a possible later expansion.

### Triggers

- **Polled triggers** are checked at fixed rates (daily or monthly) over active sets, not every tick, with a chance or mean time to happen.
- **Raised triggers** are fired by other systems the moment something happens (civilian harm, a coup, a sanction violation exposed, an attack on peacekeepers, a nuclear detonation) and open the matching events.

### Determinism

- Random draws use the seeded generator in a stable order. Events fire in a fixed order (priority, then event ID, then scope ID). Pending events, chain state, flags and cooldowns are stored state in the hash and the save. Event text and display are client-side and never affect the simulation.

### Conditions and effects are data only

- Conditions and effects are built from a **fixed vocabulary of primitives** written in RON, combined with and, or, not, comparisons and scopes. New primitives are added in Rust when needed. This is deterministic by construction and validated at load time (unknown names fail loudly). **No scripting language for now;** revisit only if the vocabulary proves too limited. This also settles the Phase 4 trigger-and-effect hook.

### The primitive vocabulary (starting shape)

- **Scopes and navigation.** Every condition and effect runs in a scope (country, region, population group, unit, officer, a pair of countries, the world). Navigation primitives move between scopes: THIS, the other party (FROM), the owner or capital of something, regions of a country, neighbors, alliance members, groups in a region. Iteration (any or every neighbor, every group in a region) always runs in a stable sorted order with a bounded size, and there are no open-ended loops.
- **Conditions (examples).** Combinators: and, or, not, at least N of. Comparisons on named values: stability, legitimacy, standing, opinion of another country, unrest, GDP, treasury, tech field level, unit strength. State checks: a flag is set (and for how long), has a trait or modifier, the law or regime is X, the war level, at war with, a treaty or alliance exists, tier and standing band, a building exists at a level, recognition, an event chain is at step N. Time and chance: after a date, a month, a seeded random chance.
- **Effects (examples).** Values: add to or set a stat (clamped), give or take money or goods. Marks: add or remove a flag or timer, add or remove a modifier or trait (with duration), add an opinion modifier. Politics: change a law or regime, sign or break a treaty, join or leave an alliance, set a war level, create a movement, start a coup, propose a UN resolution, set a country kind. World: spawn, destroy or transfer a unit, change or damage a building, shift population or unrest in a group. Flow: schedule an event after a delay, fire an event for another scope, a weighted random choice from a list, if and else.
- **Values** can combine constants, named stats and fixed-point arithmetic (add, multiply, min, max, scale). Everything is deterministic and loaded and validated from RON.

### Severity

- Each event definition carries a **severity** from a data-defined list (for example news, minor, major, critical). The engine only exposes pending events and their severity. **The client decides how to present them:** critical events can request a pause, major ones can ping, minor ones go to an inbox, and news is a ticker. Players set their own thresholds. Deadlines run on the simulation clock, so presentation never changes the simulation.

### Text and tooltips

- **Tooltips are generated from the primitives.** Every condition and effect carries a text template, so an option's tooltip ("-10 legitimacy, +5 opinion with X for 2 years") and the "why did this fire" explanation are built automatically from the data. This keeps text and logic from drifting apart, lets the AI explain its choices, and allows a preview of effects before choosing. Each new primitive needs a template.
- **Every event also has authored flavor text:** a title and description, plus a short line for each option. Flavor text is stored with the event definition and never affects the simulation.

### Engine implications

- Tables owned and hashed by the world: pending events with deadlines, chain state per scope, flags and cooldowns. Event definitions are data.
- An event queue and a fixed processing point in the tick, with polled and raised triggers.
- A generic condition and effect evaluator over the primitive vocabulary, shared with traits, modifiers, laws and treaties.
- Commands: ChooseEventOption.
- The client gets the list of pending events per country (with severity, so it can decide to pause or file them in an inbox).

### Still open

- The final primitive list (the starting shape is in this section), the text-template format and localization, event volume, and the content workload (this is very large and judgment-based; begin with the events other systems need).
- The severity list and the client's presentation settings.
- AI weights per option, text and localization format, and how events are balanced and tested.
- Casus belli as event content: the events that create justification records.

## 20. AI

Decided 2026-11-09 (in progress). The AI plays by the same rules as the player and has to decide across every system designed so far. This section covers the approach; the content (weights, personalities, planners) is large and comes later.

### Principles already fixed

- **Same rules, same information.** The AI uses the same commands, validated the same way, and the same per-country view (knowledge levels, intelligence estimates, contacts). It does not read the omniscient state (section 10).
- **Commands are logged.** AI commands, including those from delegated headquarters, go into the replay log, so replays do not depend on AI code.
- **Deterministic.** Any randomness comes from the seeded generator in a stable order.

### What the AI decides

- Military operations (fronts, offensives, defense, air, naval, logistics), economy (construction, production orders, budget, trade), politics (laws, unrest, coups, internal operations), diplomacy (treaties, alliances, UN votes and proposals, casus belli, peace), intelligence (budgets, operations), research (programs, allocation) and event choices, for about 190 countries.

### Layers and rates

1. **Strategic layer (per country):** goals, threat assessment, resource priorities, and a posture (peace, preparation, war level). Slow rates (weekly or monthly).
2. **Operational layer:** theater plans, front assignments, offensive and defensive plans, logistics. Medium rates (daily) for countries at war.
3. **Execution layer:** orders to units and headquarters, which the engine executes each tick.

- **Level of detail by importance.** Countries that matter (great powers, those at war, those near the player) get frequent and detailed decisions. Distant minor countries get coarse, low-rate decisions. This controls the cost of 190 countries.

### Technique

- **Data-driven utility AI with dedicated planners.** Each possible action gets a score from weighted considerations. The **weights come from data:** a country's personality (traits, regime, tier, doctrine, history) sets them, so countries behave differently and designers tune by editing data. The military layers use **dedicated Rust planners** (front assignment, offensive planning, supply-aware routing) because those need real search. The same scoring model drives the event option weights (section 19). It is deterministic, cheap and can explain itself (the score breakdown can be shown).
- **Behavior trees** may serve narrow, routine jobs but are not the main brain. Heavy search or machine learning is avoided in the first version for cost and determinism.
- **What makes it feel alive:**
  - personality from data;
  - constraints from domestic politics (the AI needs faction approval, has unrest, runs out of money);
  - **commitment** (plans and goals persist, with hysteresis, so it does not flip-flop every week);
  - imperfect information (it works from estimates and contacts, so it misjudges, is surprised and can be deceived);
  - visible reasons (the score breakdown can feed events and diplomatic messages);
  - a thin layer of historical tendencies (per-country long-term goals, rivalries and red lines) so history's shape shows up without scripting outcomes.

### Where the AI's memory lives

- **Outside the simulation.** The AI is a separate component that reads the per-country view and emits commands. Its memory (goals, plans, assigned units, tracked grudges) is **not part of the world hash.** It is saved in a **separate section of the save file** and loaded with it. Replays use the command log and do not need the AI at all, so improving the AI never breaks old replays, AI versions can be swapped for testing, and the simulation can run headless without an AI. In tests, the same save plus the same AI version must give the same commands (checked in the proving ground).

### Difficulty

- **Competence settings first:** how often the AI re-plans, how accurately it estimates, how aggressive or cautious its personality weights are, and how many countries get the detailed layers.
- **Optional explicit bonuses** for the AI (a modifier on its production, supply or research, shown openly in the game) can be turned on by the player. They are **off at the default setting,** so by default the AI wins or loses by decisions alone. They use the existing modifier mechanism.

### Engine implications

- The per-country view API and the command interface are the AI's only connections to the engine (sections 10 and Phase 6). A separate AI crate depends on them and not on `meridian-core` internals.
- A save-file section for AI memory, versioned and separate from the hashed world.
- Planner components in Rust (fronts, offensives, routing), reusing the supply and ZOC queries.
- Scheduling hooks so the AI runs at its own rates over importance tiers, and a per-AI timing report.

### Still open

- The considerations and weights for each decision type, the personality definitions, and the historical tendencies per country (judgment-based, large).
- The planner designs for fronts, offensives, air and naval operations, and logistics.
- The importance tier definitions and rates, and the competence settings.
- How the AI uses intelligence, how it explains moves, and testing and balancing methods (self-play, scenario tests).

## 21. Player interface

Decided 2026-11-09 (in progress). What the player sees and touches. The engine provides the per-country view and the command interface (Phase 6); the client is Phase 7. This section decides what the view API must offer and how the game plays.

### Philosophy

- **Map first, with contextual panels.** The map fills the screen. Panels open when you select something or ask for them. This suits the clean paper-map style (section 2) and a game this deep, and keeps the screen calm. The risk is that important systems get buried, so alerts, the event inbox and a few always-visible indicators must carry the load.
- A dense multi-panel dashboard was rejected: it clashes with the map style and overwhelms newcomers.

### Map modes (starting set)

Each mode is a view over state the engine already has, drawn in the same style. Overlays (rivers, labels) can be toggled on any mode.

| Mode | What it shows |
|---|---|
| Political | Country borders and control, contested cells, recognized or unrecognized status |
| Terrain and weather | Terrain classes, rivers, bridges, current weather and ground condition |
| Supply and logistics | Supply by class, stockpiles, rail and road capacity, hunger, attrition risk |
| Military | Units, fronts, zones of control, contacts with their uncertainty, air defense bands |
| Economy | Goods and buildings by region, production, trade routes, shortages, prices |
| Population | Culture, religion, political support, unrest, refugees |
| Diplomacy | Opinion toward others, alliances, treaties, UN membership and votes |
| Intelligence | Knowledge level per country and category, networks, known operations |
| Internal conflict | Unrest, insurrection, movements and loyalty |

Technology and research have no map mode; they live in panels, except that research institutes appear as sites.

### Giving orders

- **Direct orders plus drawn operation plans for headquarters.**
  - *Direct orders:* select a unit, then move, attack, set a stance, or hold. Always available.
  - *Operation plans:* select a headquarters and draw an operation on the map: an objective (cells or a city), an axis of advance, phases and a stance. The headquarters carries it out through the military planners (the same planners the AI uses, section 20), issuing **logged orders** to its subordinates. The player can edit, pause or cancel a plan at any time and see what each part is doing. This makes the delegation decided in section 5 practical in big wars.
- **Plan state** (objective, axis, phases) is kept in the same separate save section as the AI's memory, outside the hash. Execution produces logged commands, so replays do not need the planner (section 20).
- **Order previews:** the client can ask the engine to validate an order without applying it (legality, expected supply, route), so the player sees problems before committing.

### Information and presentation

- **Tooltips and "why" explanations are generated from the primitives** (section 19), with authored flavor text for events.
- **Events** are presented by severity (news ticker, inbox, notification, pause request), with player-set thresholds (section 19).
- **Fog of war and uncertainty** are shown honestly: contacts appear with their uncertainty, intelligence estimates show their margin (sections 10 and 17).

### Screens and panels (to design)

- Country overview, unit and officer details, production and construction, economy and budget, politics (laws, factions, stability, legitimacy), diplomacy and the UN, intelligence operations, technology and research.

### Engine implications

- The view API must offer a query per map mode (aggregated by cell, region or country, at the player's knowledge level) and the data tooltips need.
- Order validation without applying (dry run), returning a result and reasons.
- Event lists with severity, and generated text templates.
- A client-side layer for presentation settings (thresholds, speed). The client never changes simulation state except by commands.

### Still open

- Layout details, screen design for each panel, time controls (pause, speeds, auto-slow), onboarding and tutorials, accessibility, input devices, and the client technology (wgpu or Bevy).
- The final list of map modes and overlays, and the performance budget for map rendering at 600,000 cells.
- Notification design (alerts, always-visible indicators).

## 22. Engine work this adds, by phase

### Phase 3 follow-ups (map-builder and map)

- Bake cell latitude and longitude (integer microdegrees). Check how `c_dist_m` was computed.
- Finer sea zones near coasts and chokepoints (Dover, Taiwan, Korea, Bering, Lombok, Sicily, Bab-el-Mandeb).
- Confirm the province graph links the two sides of the antimeridian (Chukotka, Fiji, Aleutians).
- Names for provinces, sea zones and regions.
- Render-only output file (contours, rivers, coasts, borders by zoom level, cell outlines).
- New data layers (all license-checked): farmland and wetland (land cover), a climate table, weather zones.
- Record the expected map checksum in the repo, and plan to publish the baked file as a release artifact (the Python output depends on the GEOS version, so rebuilding on another machine could give a different map).
- Update `ENGINE_BUILD_GUIDE.md` to match what was built (flat-array container, relaxed province minimum, deferred cell open and close).
- River classes (small and major) on cell-graph edges from a source attribute, and a baked bridge layer at road and rail crossings.
- A graded landing-suitability class (none, poor, fair, good) per coast cell.
- A settlement layer for towns above about 10,000 to 20,000 people (candidate GeoNames; verify the license).
- Neighbor bearings (integer) for directional fortifications and flank checks.
- Land cover (forest, jungle, marsh, farmland) after a license check.

### Phase 4 (scheduler and events)

- Active-set scheduling (sorted lists of cells or units to process).
- Generic modifier mechanism (fixed-point multipliers, fixed stacking order).
- Generic trait mechanism (sorted lists or bitsets of trait IDs; modifier bundles with scope).
- Shortage-splitting primitive, multi-source supply propagation primitive, double-buffered flow primitive for people.
- Dynamic per-cell state: hunger, local food stock, ground condition, and the population group table (section 14).
- Extensible state tables declared by the ruleset but owned and hashed by the world (economy: buildings, regional stocks, prices, construction projects, production orders, equipment stockpiles, population groups, treasury and debt; politics, section 15: factions, regimes, laws and transitions, legitimacy and stability, per-group unrest, movement entities, per-unit and per-officer loyalty, war levels per side).
- Diplomacy tables (section 16): treaty instances and terms, alliance records with members and cohesion, UN bodies, proposals and votes; a purpose-built deterministic UN voting engine; command validation for treaties and resolutions.
- Player interface (section 21): a view query per map mode at the player's knowledge level; order validation without applying; plan state in the separate AI and delegation save section, outside the hash.
- AI (section 20): a separate AI crate that reads the per-country view and emits commands; a versioned AI memory section in the save file, outside the hash; planner components for fronts, offensives and routing; scheduling by importance tier.
- Event tables (section 19): pending events with deadlines, chain state per scope, flags and cooldowns; an event queue with polled and raised triggers; a generic condition and effect evaluator shared with traits, modifiers, laws and treaties.
- Technology tables (section 18): field level per country and field, development programs, research building records, a per-country unlocked-model set; a generic prerequisite check; research, learning and diffusion systems at slow rates.
- Intelligence tables (section 17): agency capability per discipline, network strength per agency, target and discipline, named operations; a deterministic estimate function for the view API; slow-rate intelligence systems.
- Runtime country creation and the civil war split (deterministic division of regions, units, stockpiles, buildings, treasury and debt), with conservation tests.
- Politics systems at monthly or event rates: unrest, faction approval and cohesion, law transitions, defections, movement strength, regime change; command validation against each side's war level.
- Economy systems at daily, weekly and monthly rates: production and labor staffing, regional goods transport on the shared network, prices, trade routes, construction, treasury, production orders. Active sets for regions and projects, and conservation tests for goods and money.
- Weather system and zone state.
- Unit, officer, pilot and hull tables with stable IDs and status.
- Cell open and close (deferred from Phase 3) and per-cell controller, damage, fortification, hazards, last fought.
- ZOC and front derived caches; visibility pass and contacts.
- Daily systems for attribute updates (cohesion, readiness, morale), officer careers monthly.
- Transit state (air move, paradrop, sea move, amphibious landing queue) with arrival ticks.
- Engagement state (principals and start-of-fight snapshot), contested-cell pressure, unit status (steady, retreating, routed, surrendered or destroyed), days-encircled counter, Hold flag, prisoner pools.
- Garrison records for cities and sites, and the deterministic on-contact garrison function for towns.
- Doctrine state per country and track, country traits, per-country tech bitset.
- Sparse directional fortification table, dynamic bridge table, per-cell unit lists and the supply demand aggregation with attrition hooks.

### Phase 5 (save, load, replay)

- Save diffs include all new dynamic arrays and tables, contacts, orders, stances, transit states, careers, and the economy tables (buildings, stocks, prices, projects, production orders, stockpiles, population groups, treasury).
- Replay logs include AI and delegated-headquarters commands.
- Rebuild derived caches on load and test that results match.

### Phase 6 (interfaces)

- Per-tick event list for the client (hits, strikes, missions, movements, battles) plus changed cells.
- Per-country view API with knowledge levels, `omniscient` feature, commands that target contacts and places.
- Event commands: ChooseEventOption.
- Technology commands: StartProgram, CancelProgram, SetResearchBudget.
- Intelligence commands: Gather, CovertAction, CounterIntelSweep, SetIntelBudget.
- Diplomacy commands: Propose and Sign treaties (minor, major), Join and Leave alliance, Answer a call to arms, UN Propose and Vote.
- Politics commands: ChangeLaw, SetWarLevel (escalate or de-escalate), Suppress, Concede, search-and-clear orders.
- Economy commands: Construct, Upgrade, Demolish, Repair, ProductionOrder, SetTaxRate, SetTariff, SetSubsidy, Borrow, Aid, Embargo.
- Commands: Launch, Mobilize, Demobilize, Appoint, SetStance, SetDoctrine, Withdraw, Hold, Fortify, Amphibious Assault, join-assault orders, bridge build, dismantle and demolish orders, air mission and transit orders, Bombard.
- Optional modifier trace per engagement for UI tooltips (diagnostic, never hashed).
- Front query and front segments.

### Phase 7 (client)

- Paper-map renderer from the render-only file, political mode switch, labels, effects layer driven by events, contact markers with uncertainty, contested-cell hatching.

### Phase 8 (performance)

- Budgets for the economy passes (labor, transport, prices, construction), the population group table size and flows, the detection pass, ZOC pass, engagement resolution, supply propagation and demand aggregation, population flow, daily attribute updates and the nuclear large-radius query. Measure before optimizing.

### Proving ground additions

- Food, hunger, refugees and the population conservation check (per group).
- A tiny AI loop: one AI country that reads its view, commits to a goal, emits logged commands, survives a save and load with the same commands afterwards, and plays a replay without the AI.
- A tiny event loop: a polled event and a raised event, a two-party incident with linked instances, a chain that schedules a follow-up, an unanswered event taking its default, and an AI choice by weights, all deterministic.
- A tiny tech loop: one field, one development program that draws on the economy and unlocks a model, and a licence that gives the model without raising the field.
- A tiny diplomacy loop: one minor treaty, one major treaty needing faction approval, an alliance call with two members deciding, and a UN vote with a veto.
- A tiny political loop: two factions, one law, a stability and unrest model, a war-level validation test, defection of one unit, and a civil war split with conservation checks.
- A tiny economy: three goods, one extraction and one refining building, a labor pool, regional transport, banded prices, one construction project and one production order, with conservation checks for goods and money and a mobilization test (drafting lowers output).
- Supply propagation with one class, then three.
- A small modifier and trait example.
- Hash-equality tests for each derived cache.
- A two-brigade engagement with a supporter, a retreat, a rout and an encirclement surrender, checked for determinism and prisoner accounting.

## 23. Risks and things to verify

- **Licenses:** climate data, land cover, fonts, icon packs, texture sources, equipment and order-of-battle sources. Verify each; hobby now, commercial open.
- **Subjective starting values:** veterancy, morale, readiness, traits for real units. Document the reasoning.
- **Content workload:** orders of battle and catalogs dwarf the code. Use the regional slice and the tier approach.
- **Scope:** this list is long for a solo project. Build the primitives first, with one supply class, a few traits and a small catalog, and widen later.
- **Balance:** nuclear weapons, hunger cascades and air power can dominate. Rules need gating and testing.
- **Interface scope:** nine map modes over 600,000 cells need a render budget and aggregation queries. Design the view API with the client in mind, and measure early.
- **AI scope:** the AI is the largest content and balance task after the map. Start with one strategic layer and one planner for a small theater. Per-AI timing must be measured early (190 countries).
- **Events scope:** the content workload is very large. Start with the events that other systems need, and widen later. Keep the primitive vocabulary small.
- **Tech scope:** the content (fields, programs, years) is large and judgment-based. Build the tables and a few programs first.
- **Intelligence scope:** the layer is still in design; keep it abstract (networks, not agents) and widen later. The estimate function must stay deterministic and out of the hash.
- **Diplomacy scope:** AI diplomacy is still undesigned, and the UN, historic alliances and casus belli content are content-heavy. Build the tables and a few entries first.
- **Politics scope:** this is a large, content-heavy layer (regimes, laws, factions, events). Build the tables and a few entries first, widen later. The civil war split is the riskiest engine work (runtime country creation, conservation).
- **Economy tuning:** prices, trade and money systems can spiral or stall. Use fixed bands, slow adjustment, and long proving-ground runs that check for runaway inflation or empty markets.
- **Population group size:** the group table could be larger than 10 to 15 MB if mixed areas multiply records. Measure it before committing.
- **Estimates not yet checked:** model counts, officer counts, vessel counts, pilot counts, byte sizes of new arrays (about 3.5 MB for population, hunger and food stock at 600,000 cells), contact counts.
- **Distances:** verify how cell-graph distances were computed before using ranges.
- **Data sources:** the source policy (decided 2026-11-09) is in `docs/data-sources.md`: facts we extract and author ourselves are not limited by the source's license (record provenance, write our own wording, no bulk copying); datasets we import and bake into the game must be public domain, CC0 or attribution-only, and non-commercial ones stay out. See that file for what was verified on 2026-11-09. Non-commercial sources (WorldClim, GeoEPR, CShapes) must stay out of a commercial build, and the bake step should enforce that. What follows is the earlier list of items, some now checked: river classification (what Natural Earth's river attributes offer), the settlement layer for garrisons, beach and landing data (OpenStreetMap's ODbL has share-alike terms), and land cover. None has been checked.
- **Cell scale:** cells are about 15 km across, so city, bridge and frontage rules depend on how well built-up levels and border lengths describe reality. Spot-check a few famous cases (a river city, a mountain pass) in Phase 3 follow-ups.
- **Map reproducibility:** the baked map may differ across machines; pin and record the checksum.

## 24. Still open

- **Decided: the development slice is the Persian Gulf, 1990** (Iraq's invasion of Kuwait; the UN sanctions and authorization of force, the coalition alliance, the oil economy, air power, casus belli, desert supply). It exercises most systems with a manageable amount of content. Yugoslavia (1990 to 1995) is the likely second slice because it tests politics and internal conflict harder. The engine and map still cover the whole world; only the full content (orders of battle, catalogs, groups, factions, events, AI personalities) is built for the slice first.
- Content for the slice still to be gathered: the orders of battle, equipment catalog entries, population groups and factions, the UN resolution events, and the data sources, all judgment-based and unverified.
- Rules for ZOC radius, pressure thresholds, ambush bonus, nuclear release, priorities in food shortages, attrition rates, rout thresholds, garrison sizes, the optional combat-width rule.
- Whether to add ship captains, pilot traits or per-branch individual records beyond pilots and hulls.
- Climate and land-cover dataset choices: the leading candidates are CHELSA climatologies (CC0) and ESA WorldCover (CC BY 4.0), both verified in `docs/data-sources.md`. WorldClim is non-commercial and is avoided. Confirm the choice.
- Client technology (wgpu or Bevy), scripting, platforms beyond macOS, 1990 borders cross-checks.
- The player interface is decided in outline (section 21) and the AI approach is decided in outline (section 20); the events system is designed in outline (section 19); politics (section 15), diplomacy structure (section 16), intelligence (section 17) and tech (section 18) are designed in outline. Combat resolution (section 12) and the economy (section 14) are designed in outline; their numbers are rules.
- Economy items still open are listed at the end of section 14, politics items at the end of section 15, diplomacy items at the end of section 16, intelligence items at the end of section 17, tech items at the end of section 18, event items at the end of section 19, AI items at the end of section 20, interface items at the end of section 21.
- Data sources: see `docs/data-sources.md` (checked 2026-11-09). Still to check: GHS-BUILT and GHS-SMOD terms (GHS-POP is verified; SIPRI is commercial-only), HydroSHEDS documentation, an oil field and industry source, and the individual US government documents for orders of battle. Decide whether to use OpenStreetMap (share-alike) for the map.

## 25. Map redesign: a globe, hexagon levels, military map scales

Decided 2026-11-10, as the start of a second big rewrite. Nothing in this section is built. The map as built today (provinces from real admin-1 regions, about 600,000 irregular cells of about 225 km² on an Equal Earth plane, a 3D view over it) stays in the repo until its replacement works.

### Map scales on screen

The map is shown as military maps at five scales, and zooming moves from one to the next with a transition between them:

| Map scale | Ground distance per 1 cm | Purpose |
| --- | --- | --- |
| 1:1,000,000 | 10 km | Theatre strategy |
| 1:250,000 | 2.5 km | Corps operations |
| 1:100,000 | 1 km | Operational planning |
| 1:50,000 | 500 m | Tactical planning |
| 1:25,000 | 250 m | Local tactical planning |

- The graphics will probably mix 2D and 3D.
- **Assumed, not yet confirmed by the owner:** the maps are drawn by the client from data, in the style of military topographic maps, not scanned paper maps. Scanned US military charts (ONC, TPC, JOG, Army Map Service sheets) are public-domain works and good style references, but a picture cannot show what the game changes (borders, blown bridges), and nothing public-domain covers the world at 1:100,000 or finer.

### Hexagons instead of irregular cells

- **Why.** Every hexagon has six neighbours at the same distance, so movement, range, frontage and zones of control behave the same everywhere. A river lies on hexagon edges and a bridge is a hexagon edge, which replaces the weak rule of today ("a river passes between these two cells"). A hexagon's place and outline follow from its number, so nothing has to be stored to draw or pick it. The levels of a hexagon hierarchy match the map scales.
- **The grid is on the globe.** A flat hexagon grid on a projected map was rejected: the projection stretches shapes toward the poles, so the hexagons would not be the same size on the ground. The candidate is H3, an open hexagon grid of the whole globe with fixed levels. Each level has 12 pentagons, and finer hexagons fit inside coarser ones only approximately.
- **One level is simulated: H3 level 6, everywhere, always on.** Decided 2026-11-10, replacing the three levels (5, 7 and 8) chosen earlier the same day, because the owner then set the brigade as the smallest unit on the map. A hexagon should be about as wide as a brigade's front; finer hexagons would make one brigade spread over several, and coarser ones would stack brigades inside a hexagon.

| H3 level | Across | Edge | Area (average; smallest to largest) | Land hexagons (estimate, 29 % of the globe) | Use |
| --- | --- | --- | --- | --- | --- |
| 6 | about 6.4 km | about 3.7 km | 36 km² (21.9 to 43.6) | 3,686,556 north of 60° S (measured; about 4.1 million with Antarctica, estimate) | The one simulated level. A brigade holds about one hexagon in attack and two in defence |
| 5 | about 17 km | about 9.9 km | 253 km² (154 to 305) | about 590,000 | Not simulated. About the size of the old cells; may serve to group hexagons for drawing or statistics |
| 7, 8 | 2.4 km, 0.9 km | 1.4 km, 0.53 km | 5.2 km², 0.74 km² | about 29 million, about 200 million | Dropped: battalion and company scale |

  - **No opening and closing.** With one level always on, a hexagon never opens into finer ones. The open-and-close machinery of the old design (a province opening into its cells) is not needed on the new map.
  - **Cost.** About seven times the old cell count. Per-hexagon state is estimated at 60 to 100 MB; baked terrain for the world at this level is tens of MB. Systems already skip quiet areas (active sets). None of this is measured yet.
  - **Economy and population stay coarser** than the hexagon (per province or region), so those tables do not grow sevenfold.
- **Provinces stay** as the political and administrative layer above the hexagons (real admin-1 regions). A hexagon belongs to the province and country that holds its centre. The real border lines are drawn on the map; in the simulation a border follows hexagon edges. Very small states need a rule so they keep at least one hexagon.
- **What survives in the engine.** The engine treats cells as a graph of neighbours with stored distances, so pathfinding, flows, marches, bridges and the per-cell state keep working when the cells are hexagons. The map builder, the map file and the client's drawing are what get rewritten.

### A globe

- **The simulation uses positions on the sphere.** Each hexagon centre is a point in three dimensions in whole metres from the Earth's centre (it fits the 32-bit integers used now). Distance is integer arithmetic on those points: no projection formulas, no floats, true distances at every latitude. There is no map edge: the Pacific wraps and the poles are not a special case, so the planned antimeridian links are not needed.
- **The client draws a globe** (the owner chose a drawn globe over a flat projected map on screen). Zoomed out it is a globe; zoomed in toward the map scales the curvature disappears by itself, so the military maps look the same as on a flat map. The flat paper-map look when zoomed out is given up.
- **Cost.** The client's map drawing is rebuilt: a camera that orbits a sphere, the map drawn as tiles on its surface, positions sent to the graphics card relative to the camera (it cannot hold metres at Earth scale precisely), picking and labels on a sphere.

### Data for the map scales

Sources checked on 2026-11-10 are in `docs/data-sources.md`. In short:

- **1:1,000,000:** VMAP0 (the Digital Chart of the World, public domain, compiled 1991 to 1993 from the ONC charts, so close to 1990) and Natural Earth.
- **1:250,000:** VMAP1 where it was released (only partly public; parts of Iraq are), GRIP4 roads (CC0, modern), built-up areas from GHSL (a 1990 layer exists), contours from open elevation data.
- **1:100,000 to 1:25,000:** OpenStreetMap (or Overture Maps, whose roads and buildings come from it), the only worldwide data with that detail. **Decided 2026-11-10: use it.** It is modern, not 1990, and its licence is share-alike on the data: map data we ship that is derived from it is kept in files of its own, offered under the same open licence (ODbL), with credit on screen. The game's code and other data are not affected, and map images need only the credit.

### How much of the world, and how big

Decided 2026-11-10: **the base covers the whole world down to 1:250,000, with level-6 hexagon terrain; the three fine scales come later as regional packs**, the Persian Gulf first. The owner first chose the whole world at every scale, then this, after seeing the sizes. Everything is built and stored in regional pieces from the start, so a pack adds to the base without redoing it.

Rough sizes (estimates, except where a source is named):

| What | Size |
| --- | --- |
| Base: the world at 1:1,000,000 and 1:250,000, level-6 terrain | 5 to 20 GB, most likely about 10 GB: 2 to 6 GB of roads, railways, rivers, town shapes and names; elevation about 10 GB at 90 m or 1 to 1.5 GB at 250 m; land cover at about 100 m about 1 GB; hexagon terrain about 0.2 GB |
| The whole world at every scale, for comparison | 100 to 180 GB at full detail, 50 to 70 GB trimmed. Whole-planet street tiles alone are about 81 GB from a 92 GB OpenStreetMap file (planetiler's published figures), and building them wants about 46 GB of memory and 0.5 to 1 TB of disk |

- **What the base shows.** At 1:250,000 one centimetre is 2.5 km (about 66 m to a pixel at 96 pixels to the inch): towns as shapes, main and secondary roads, railways, rivers, forests, airfields, contours every 50 to 100 m; no single streets or buildings. A level-6 hexagon is about 6.4 km across, about 2.6 cm at that scale and 6 mm at 1:1,000,000.
- **Until a region has its pack,** zooming there stops at 1:250,000 (or shows the fine scales generated plainly from the base; not decided). The simulation is the same everywhere.
- **The base need not use OpenStreetMap:** its roads can come from GRIP4 (CC0) and VMAP0 and VMAP1, which avoids the 92 GB planet file. OpenStreetMap is then needed only for the packs, from regional extracts.
- **Building on the owner's machine** (Apple M4 Pro, 500 GB disk): the base should be buildable there if the builder works through the world piece by piece (download a tile of elevation or land cover, reduce it, delete it) and never holds the full sources at once. The full-resolution sources are large (land cover at 10 m is on the order of 100 GB, from memory). Expect on the order of 100 GB free disk needed at the peak and some hours of downloading and processing; none of this is measured.

### Engine and builder work this implies

- A new map builder: H3 level-6 hexagons for the land, their neighbours, distances, terrain, province and country by centre, rivers and roads snapped to hexagon edges, population per hexagon; sea zones again.
- A new map file (hexagon numbers, three-dimensional integer centres, edge flags) and loader; the map checksum and every golden hash change.
- Terrain baked for every land hexagon of the world (tens of MB).
- The client: globe camera, map tiles per scale, the transition between scales, counters and command lines on the globe.
- What was built on 2026-11-09 and 2026-11-10 for the flat map (trees, towns, landmarks, farmland, traffic, rivers, bridges drawn in 3D) is display code for the old map. How much of it carries over depends on the new look.

### Still open

- Confirm drawn maps rather than scanned ones.
- Whether the scales finer than 1:250,000 (and so the regional packs and OpenStreetMap) are still wanted now that nothing is simulated below the brigade: they would be scenery only.
- How the brigade and division model, combat width and frontage (section 12) read on hexagons of about 6.4 km.
- Which things are kept per hexagon and which per province (population, economy, weather).
- The H3 library: `h3o` is a pure-Rust version; its licence and upkeep could not be checked on 2026-11-10 (its page did not load). The engine may need only numbers baked by the builder, plus the integer parent and child steps.
- How map tiles and regional pieces are cut on the globe, and what the regions of the packs are.
- 1990 accuracy at the fine scales: the worldwide data is modern, so roads and suburbs built after 1990 show unless they are corrected by hand where it matters.
- The order of work, and which parts of the current engine phases (3 and 7 above all) are redone.

