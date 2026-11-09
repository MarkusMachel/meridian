# Meridian data sources: licenses and coverage

Checked 2026-11-09 for the Persian Gulf, 1990 development slice (see `docs/design-notes.md` section 24). This file is not legal advice. Licenses can change, so re-check each source before shipping anything.

## How to read this file

- **Verified** means the license wording was read on the source's own page during this check. The page is named so it can be re-read.
- **Not verified** means the page could not be read, said nothing about the license, or the claim comes from general knowledge. Treat it as unknown until checked.
- **Commercial?** answers only one question: can the data be used in a game that might be sold later. The project is a hobby now and commercial release is open, so a "no" here is a decision for later, not a blocker today.
- Facts themselves (for example the population of a country in 1990) are generally not copyrightable, but a compiled dataset can be. Using a dataset to inform values we author ourselves is different from copying or redistributing the dataset. When in doubt, author the values and cite the source.

## Source policy (decided 2026-11-09)

Two cases, because licensing applies differently to each.

- **Facts we extract and author ourselves** (an equipment specification, a unit's strength, a country's population share, an order of battle): copyright does not protect facts, so the source's license does not limit us, whatever the source is (a government report, a reference book, a website). We read, then write our own values and wording, and record the source and a confidence level in the provenance field. We do not copy wording or tables, and we do not bulk-scrape a whole compiled table: some jurisdictions (the EU and the UK) protect compiled databases, and some sites forbid bulk extraction in their terms.
- **Datasets we import and bake into the game** (map layers such as land cover, climate, settlements, rivers, borders): the data itself ends up in our files and in a shipped build, so the dataset's license applies.
  - **Use** public domain and CC0 datasets, and attribution-only datasets (such as CC BY) with the credit recorded.
  - **Leave out** non-commercial datasets, and share-alike datasets we cannot comply with, unless a later decision changes this (WorldClim, GeoEPR and CShapes are the known cases). They may still inform our own authored values privately.
- US federal government documents are public domain and are the preferred base for the 1991 war orders of battle. Publicly released does not mean public domain for other authors, but under the first case that rarely matters.
- This is not legal advice. Check before a commercial release.

## Summary

- **Free to use, including commercially:** Natural Earth (public domain), CHELSA climatologies (CC0), USGS-authored data (US public domain, with a third-party exception).
- **Attribution only:** ESA WorldCover (CC BY 4.0), GeoNames (CC BY 4.0), ERA5 (CC BY), HydroRIVERS (commercial use allowed, terms in the technical documentation).
- **Share-alike:** OpenStreetMap (ODbL). A baked map built from it would be a derived database and would have to be offered under the same license if distributed.
- **Blocks commercial release:** WorldClim (non-commercial), GeoEPR (CC BY-NC-SA 4.0), CShapes 2.0 (CC BY-NC 4.0). They are fine for private research and for informing hand-authored values, but must not be shipped or redistributed in a commercial build.
- **Caution:** FAOSTAT is CC BY 4.0 but forbids use "in conjunction with the promotion of a commercial enterprise". Ask before relying on it in a commercial product.
- **Verified 2026-11-09 (second pass):** World Bank datasets are CC BY 4.0 with third-party exceptions per indicator. Wikipedia text is CC BY-SA 4.0. The CIA World Factbook was discontinued on 4 Feb 2026; its public-domain status is secondhand.
- **Verified (third pass):** GHS-POP is CC BY 4.0 with a 1990 epoch. SIPRI data is commercial-licensed only (royalty), so it stays out.
- **Not verified:** the rest of "Military" below.

## Sources by need

### Terrain and base map

| Need | Candidate | License (as read) | Commercial? | Status | Notes |
|---|---|---|---|---|---|
| Elevation and bathymetry | ETOPO (already in the pipeline) | NOAA product; not re-read in this check | Probably yes | Not verified | Re-check the license text on the NOAA page |
| Coastlines, borders, populated places, roads and rails (coarse) | Natural Earth | "in the public domain"; "personal, educational, and commercial purposes"; no permission needed; credit optional | Yes | Verified at naturalearthdata.com/about/terms-of-use | Some third-party data was licensed to Natural Earth "for the sole purpose of creating a world base map". Check the metadata of each layer before relying on it for anything else |
| Rivers (small and major classes) | Natural Earth rivers and lake centerlines | Public domain, as above | Yes | Verified (same terms page) | Low detail; check whether its attributes can separate small from major rivers |
| Rivers (detailed) | HydroRIVERS (HydroSHEDS) | "freely available for scientific, educational and commercial use" | Yes | Verified at hydrosheds.org/products/hydrorivers | Redistribution and attribution terms are in the HydroSHEDS technical documentation, which was not read. Read it before baking the data into a shipped map |

### Land cover and climate

| Need | Candidate | License (as read) | Commercial? | Status | Notes |
|---|---|---|---|---|---|
| Land cover (forest, marsh, farmland) | ESA WorldCover | CC BY 4.0 | Yes, with attribution | Verified (AWS open data registry) | Covers 2020 and 2021 only, so it is a modern proxy for 1990. For a desert slice this matters little |
| Climate normals | CHELSA climatologies and bioclim (V2.1) | CC0 1.0 | Yes | Verified at chelsa-climate.org | Normals start at 1981 to 2010, a reasonable stand-in for 1990. Other CHELSA products may carry other licenses, so check each |
| Climate normals | WorldClim 2.1 | "Redistribution or commercial use is not allowed without prior permission" | **No** | Verified at worldclim.org/about.html | Avoid for anything that ships. Permission could be requested |
| Weather history (for weather zones) | ERA5 (Copernicus Climate Data Store) | "CC-BY licence"; data from 1940 onward | Likely yes, with attribution | Verified licence label; commercial wording not read | Covers 1990. Cite DOI 10.24381/cds.adbb2d47 |

### Population, settlements and identity

| Need | Candidate | License (as read) | Commercial? | Status | Notes |
|---|---|---|---|---|---|
| Population distribution in 1990 | GHS-POP R2023A (GHSL, EU Joint Research Centre) | The AWS Open Data registry says "GHSL data are provided under the Creative Commons Attribution 4.0 International (CC BY 4.0) license"; the Commission's own pages say "Reuse is authorised, provided the source is acknowledged" (Commission Reuse and Copyright Notice). Epochs 1975 to 2020 in 5-year steps, so **1990 exists**; 2025 and 2030 are projections. 100 m and 1 km grids | Yes | **Verified** (registry and Earth Engine catalog; the Commission notice itself not read) | Cite as "GHSL gridded population estimates (GHS-POP) ... accessed on DATE" with the DOI 10.2905/2FF68A52-5B5B-4A22-8F40-C41DA8332CFE. The 1975 to 2020 values are modeled from CIESIN GPWv4.11 census data, so check GPW's own terms if we redistribute it. 1 km is plenty for 15 km cells |
| Towns and cities for garrisons | GeoNames | CC BY 4.0 | Yes, with attribution | Verified at geonames.org/about.html | The page did not address commercial use directly, but CC BY allows it. Some entries carry source-specific notices (for example Ordnance Survey) |
| Towns and cities (coarse) | Natural Earth populated places | Public domain | Yes | Verified | Fewer places than GeoNames |
| Ethnic groups and where they live | GeoEPR (ETH Zurich) | CC BY-NC-SA 4.0; latest release 2021 | **No** (non-commercial, share-alike) | Verified at icr.ethz.ch/data/epr/geoepr | Useful for research and as a check on our own values. Do not ship or redistribute it |
| Country-level ethnic and religious shares | CIA World Factbook | **Discontinued by the CIA on 4 Feb 2026**; the site and its archives were taken down. The CIA's own pages only say its photographs were copyright-free. Public-domain status of the text is reported by secondary sources (Data Rescue Project) but the formal statement was not retrievable | Probably yes (US government work; third-party material inside may differ) | **Partly verified (secondhand)** | Frozen modern editions only, via archives (a 2020 full archive exists; Wayback Machine snapshots since 2017). No 1990 edition confirmed. Use only as a cross-check; prefer censuses and academic compilations, cited. Under our fact-extraction policy we may author values from it with provenance |
| Economic and demographic statistics | World Bank open data | Datasets are provided under **CC BY 4.0** (Terms of Use for Datasets). Attribution format: "The World Bank: Dataset name: Data source (if known)." Some indicators come from third parties and "may not be redistributed or reused without the consent of the original data provider"; those conditions are in each indicator's metadata. No implied endorsement; no use of the World Bank name or logo | Yes for CC BY indicators; check each indicator's metadata for third-party ones | **Verified** (terms page); per-indicator metadata still to check | The page does not address commercial use explicitly, but CC BY 4.0 permits it. Note the dispute clause (mediation, then arbitration). Coverage of 1990 is partial (many countries start later). Record the indicator code and source in provenance |
| Agriculture and food statistics | FAOSTAT | CC BY 4.0, but "Datasets shall not be used for or in conjunction with the promotion of a commercial enterprise" | Doubtful | Verified at fao.org/contact-us/terms/db-terms-of-use | Ask FAO before using in a commercial product |

### Borders in 1990

| Need | Candidate | License (as read) | Commercial? | Status | Notes |
|---|---|---|---|---|---|
| Historical country borders | CShapes 2.0 | CC BY-NC 4.0 (the R package is GPL, the data is not) | **No** | Verified at research-collection.ethz.ch | The data license differs from the package license |
| Modern borders as a base | Natural Earth | Public domain | Yes | Verified | **For the Gulf slice, 1990 borders are nearly the same as today's:** the main changes are Yemen (unified in May 1990) and disputed border lines. Editing these by hand from public sources is practical and avoids CShapes |

### Infrastructure and landing sites

| Need | Candidate | License (as read) | Commercial? | Status | Notes |
|---|---|---|---|---|---|
| Roads, rails, ports, beaches, airfields | OpenStreetMap | ODbL: credit OSM; "If you alter or build upon our data, you may distribute the result only under the same license" | Yes, but share-alike | Verified at openstreetmap.org/copyright | A baked map derived from OSM is a derived database. Decide early whether to avoid OSM or accept ODbL for the map. It also shows today's network, not 1990's |
| Landing suitability | Derived from coast type, bathymetry slope and beach tags | Depends on inputs | Depends | Not verified | The slice's coastline is short enough to grade by hand, with notes |

### Resources and industry

| Need | Candidate | License (as read) | Commercial? | Status | Notes |
|---|---|---|---|---|---|
| Oil, gas and mineral deposits and statistics | USGS publications and data | "USGS-authored or produced data and information are considered to be in the U.S. Public Domain" | Yes | Verified at usgs.gov copyright page | Third-party content on the site may be copyrighted. Credit "U.S. Geological Survey" |
| Oil field and refinery locations | Not chosen | n/a | n/a | Not verified | Needs a source. Candidates must be found and license-checked |
| Power plants, industry | Not chosen | n/a | n/a | Not verified | Same |

### Military

| Need | Candidate | License (as read) | Commercial? | Status | Notes |
|---|---|---|---|---|---|
| Arms transfers and military spending | SIPRI databases | Commercial use "must be authorized" and typically involves a royalty. Reproducing more than 10% of a data set needs permission. Fair use (citing in papers, articles, blogs, charts) needs only credit: "Information from the Stockholm International Peace Research Institute (SIPRI), URL". Swedish law | **No** (without a license) | **Verified** at sipri.org/databases/terms | Do not import or ship SIPRI data in a commercial build. Using it to inform our own hand-authored equipment numbers, with citation, is within fair use for a hobby, but a commercial game may count as commercial use of SIPRI material, so ask SIPRI or avoid it. Prefer other sources for orders of battle and equipment |
| Orders of battle, equipment specifications | Official US reports on the 1991 war (Final Report to Congress, Army Center of Military History, Gulf War Air Power Survey) | Works of the US government are generally public domain, but individual documents can include third-party material | Probably yes | **Not verified** | Check each document. These cover the coalition and the Iraqi forces in outline |
| Orders of battle, equipment specifications | IISS Military Balance | Commercial publication | No | Not verified | Do not copy. Do not depend on it |
| Orders of battle, equipment specifications, general reference | Wikipedia | Text is **CC BY-SA 4.0** for new edits (older edits BY-SA 3.0), per Creative Commons' announcement of the move. Media files carry their own licenses. Wikidata is CC0 (from memory, not re-verified here) | Text: share-alike, so avoid importing | Partly verified | Do not copy text or tables into our data. Use it to find primary sources, then cite those. Facts we extract and write in our own words are covered by the fact-extraction policy, with provenance. Wikidata structured facts (CC0) are safe to import if needed |

## Findings that change the design

1. **Keep provenance and license per data layer.** The design principle already says every catalog entry records a source and confidence. Add a license field and a "commercial build" flag, so the bake step can refuse to include a non-commercial or share-alike source in a commercial build and can generate the attribution file automatically.
2. **Decide early about OpenStreetMap.** Using it makes the baked map a share-alike database. Natural Earth and hand-made data avoid that. For the slice, hand-grading a few infrastructure features may be cheaper than carrying an ODbL obligation.
3. **Treat identity data as authored content.** Culture, religion and political support for a handful of countries can be authored from cited published sources. Non-commercial datasets (GeoEPR) can help check the values but must not be copied.
4. **Historical borders are a small job for the slice.** Start from Natural Earth and edit by hand.
5. **Population in 1990 has a real source** (GHS-POP, CC BY 4.0, 1990 epoch confirmed).

## Still to check

- The Commission's Reuse and Copyright Notice itself, and GPWv4.11's terms if we redistribute GHS-POP. (GHSL other products such as GHS-BUILT and GHS-SMOD are not checked.)
- For the World Bank, the per-indicator metadata of each indicator we actually use. For Wikidata, confirm the CC0 statement.
- The HydroSHEDS technical documentation (redistribution and attribution).
- ETOPO's current license wording.
- A source for oil fields, refineries and power plants.
- Whether GADM, geoBoundaries and WorldPop fit (they were not checked).
- Fonts and icon sets for the client.
- Each individual US government document used for orders of battle.

## Sources read in this check

- [Natural Earth terms of use](https://www.naturalearthdata.com/about/terms-of-use/)
- [WorldClim about page](https://worldclim.org/about.html)
- [CHELSA climatologies](https://www.chelsa-climate.org/datasets/chelsa_climatologies)
- [ERA5 on the Copernicus Climate Data Store](https://cds.climate.copernicus.eu/datasets/reanalysis-era5-single-levels)
- [ESA WorldCover on the AWS registry](https://registry.opendata.aws/esa-worldcover-vito/index.html)
- [GeoNames](http://www.geonames.org/about.html)
- [SIPRI database terms](https://www.sipri.org/databases/terms)
- [GHS-POP on the AWS Open Data registry](https://registry.opendata.aws/ghs-pop/) and [Earth Engine catalog](https://developers.google.com/earth-engine/datasets/catalog/JRC_GHSL_P2023A_GHS_POP)
- [GHSL download page](https://human-settlement.emergency.copernicus.eu/download.php) and [GHS-POP page](https://human-settlement.emergency.copernicus.eu/ghs_pop2023.php)
- [GeoEPR](https://icr.ethz.ch/data/epr/geoepr/)
- [CShapes 2.0 record](https://www.research-collection.ethz.ch/handle/20.500.11850/487987?show=full)
- [OpenStreetMap copyright](https://www.openstreetmap.org/copyright)
- [HydroRIVERS](https://www.hydrosheds.org/products/hydrorivers)
- [USGS copyrights and credits](https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits)
- [FAO database terms of use](https://www.fao.org/contact-us/terms/db-terms-of-use/en/)
- [World Bank terms of use for datasets](https://www.worldbank.org/ext/en/legal/terms-conditions/datasets)
- [Creative Commons: Wikipedia moves to CC 4.0](https://creativecommons.org/2023/06/29/wikipedia-moves-to-cc-4-0-licenses/)
- [Data Rescue Project: The World Factbook is Gone](https://www.datarescueproject.org/guest-post-the-world-factbook-is-gone/)
- [WTOP/AP: CIA ends publication of the World Factbook](https://wtop.com/government/2026/02/cia-ends-publication-of-its-popular-world-factbook-reference-tool/)
