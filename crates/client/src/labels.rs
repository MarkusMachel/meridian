//! Place-name labels: which names to show at the current zoom, how big, and where. Pure logic
//! (no graphics) so it can be tested; the window paints the result with egui.
//!
//! Method: a name is shown when the area it names is wide enough on screen to hold it. Text is
//! straight (no curved placement), upper case for countries. Bigger areas are placed first and a
//! label that would overlap one already placed is dropped, so crowded views stay readable.

use crate::camera::Camera;
#[cfg(test)]
use crate::camera::Bounds;
use meridian_data::{Labels, RenderData};

/// Average glyph width as a share of the font size.
const GLYPH: f64 = 0.62;
// Sizes in logical points (multiplied by the screen's scale factor, so they stay readable on
// high-density displays).
const COUNTRY_MAX_PX: f64 = 44.0;
const COUNTRY_MIN_PX: f64 = 13.0;
const REGION_MAX_PX: f64 = 17.0;
const REGION_MIN_PX: f64 = 12.0;
const MAX_LABELS: usize = 500;
/// Country names fade out between these zooms (metres per pixel) as you zoom in, and so do region
/// names further in: close up the map shows places, not the name of the whole country.
const COUNTRY_FADE: (f64, f64) = (1_200.0, 2_400.0);
const REGION_FADE: (f64, f64) = (150.0, 300.0);
/// A name is also hidden once its area is wider than this share of the screen (it fills the view).
const FILLS_SCREEN: f64 = 1.1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    Country,
    Region,
}

#[derive(Clone, Debug)]
pub struct PlacedLabel {
    pub text: String,
    /// Centre, in screen pixels.
    pub x: f64,
    pub y: f64,
    pub size_px: f64,
    pub kind: LabelKind,
    /// 0 (invisible) to 1: names fade out as you zoom in.
    pub alpha: f64,
}

struct Candidate {
    i: usize,
    kind: LabelKind,
    area: u32,
    size: f64,
    x: f64,
    y: f64,
    alpha: f64,
}

fn candidates(out: &mut Vec<Candidate>, labels: &Labels, relief: &meridian_data::Relief, kind: LabelKind, camera: &Camera, scale: f64) {
    let (lo, hi) = match kind {
        LabelKind::Country => COUNTRY_FADE,
        LabelKind::Region => REGION_FADE,
    };
    let alpha = ((camera.m_per_px - lo) / (hi - lo)).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let screen = camera.width_px.max(camera.height_px);
    let (min_px, max_px) = match kind {
        LabelKind::Country => (COUNTRY_MIN_PX * scale, COUNTRY_MAX_PX * scale),
        LabelKind::Region => (REGION_MIN_PX * scale, REGION_MAX_PX * scale),
    };
    for i in 0..labels.names.len() {
        let (x, y) = crate::heights::to_screen(camera, relief, labels.x[i] as f64, labels.y[i] as f64);
        if x < 0.0 || y < 0.0 || x > camera.width_px || y > camera.height_px {
            continue;
        }
        let n = labels.names[i].chars().count().max(1) as f64;
        let extent_px = (labels.area_km2[i] as f64).sqrt() * 1000.0 / camera.m_per_px;
        let size = (extent_px * 0.8 / (n * GLYPH)).min(max_px);
        if size < min_px || extent_px > FILLS_SCREEN * screen {
            continue;
        }
        out.push(Candidate { i, kind, area: labels.area_km2[i], size, x, y, alpha });
    }
}

/// The labels to draw for this camera, in drawing order.
pub fn place(data: &RenderData, camera: &Camera, scale: f64) -> Vec<PlacedLabel> {
    let mut cand = Vec::new();
    candidates(&mut cand, &data.countries, &data.relief, LabelKind::Country, camera, scale);
    candidates(&mut cand, &data.regions, &data.relief, LabelKind::Region, camera, scale);
    // countries first, then bigger areas first; ties by index so the result is stable
    cand.sort_by(|a, b| (a.kind as u8).cmp(&(b.kind as u8)).then(b.area.cmp(&a.area)).then(a.i.cmp(&b.i)));
    let mut taken: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut out = Vec::new();
    for c in cand {
        let names = match c.kind {
            LabelKind::Country => &data.countries.names,
            LabelKind::Region => &data.regions.names,
        };
        let text = match c.kind {
            LabelKind::Country => names[c.i].to_uppercase(),
            LabelKind::Region => names[c.i].clone(),
        };
        let w = text.chars().count() as f64 * c.size * GLYPH;
        let h = c.size * 1.15;
        let rect = (c.x - w / 2.0, c.y - h / 2.0, c.x + w / 2.0, c.y + h / 2.0);
        if taken.iter().any(|t| rect.0 < t.2 && rect.2 > t.0 && rect.1 < t.3 && rect.3 > t.1) {
            continue;
        }
        taken.push(rect);
        out.push(PlacedLabel { text, x: c.x, y: c.y, size_px: c.size, kind: c.kind, alpha: c.alpha });
        if out.len() >= MAX_LABELS {
            break;
        }
    }
    out
}

/// Where the cities of 1990 are (the centre of each city site's cell, in world metres), in site
/// order. The renderer puts a landmark in the town nearest to each (`Renderer::set_cities`).
pub fn city_positions(map: &meridian_core::Map, data: &RenderData) -> Vec<[i32; 2]> {
    (0..map.site_count() as usize)
        .filter(|&s| map.site_kind[s] == 0 && map.site_built[s] <= 1990 && map.site_destroyed[s] > 1990)
        .map(|s| {
            let c = map.site_cell[s] as usize;
            [data.cell_mx[c], data.cell_my[c]]
        })
        .collect()
}

/// A site (city, port, airfield, ...) to draw: a marker, and a name if there is room for it.
#[derive(Clone, Debug)]
pub struct SiteMark {
    pub kind: u8,
    /// Screen pixels.
    pub x: f64,
    pub y: f64,
    pub name: Option<String>,
    pub size_px: f64,
    /// City population (0 for other sites), for the icon size.
    pub population: u32,
}

/// Which sites show at this zoom: big cities from far away, everything when close. Returns the
/// smallest city population, or `None` for "no cities", and whether ports/airfields/other sites show.
fn site_tiers(m_per_px: f64) -> (Option<u32>, bool, bool) {
    let min_city = if m_per_px < 3_000.0 {
        Some(0)
    } else if m_per_px < 6_000.0 {
        Some(200_000)
    } else if m_per_px < 12_000.0 {
        Some(700_000)
    } else if m_per_px < 25_000.0 {
        Some(2_000_000)
    } else {
        None
    };
    (min_city, m_per_px < 4_000.0, m_per_px < 2_000.0)
}

/// Sites for the current zoom, with names where they fit (names never overlap each other or the
/// place names in `avoid`). Only sites that exist in 1990.
pub fn place_sites(map: &meridian_core::Map, data: &RenderData, camera: &Camera, scale: f64, avoid: &[PlacedLabel]) -> Vec<SiteMark> {
    let (min_city, show_ports, show_other) = site_tiers(camera.m_per_px);
    let size = 12.0 * scale;
    let mut taken: Vec<(f64, f64, f64, f64)> = avoid
        .iter()
        .map(|l| {
            let w = l.text.chars().count() as f64 * l.size_px * GLYPH;
            (l.x - w / 2.0, l.y - l.size_px * 0.6, l.x + w / 2.0, l.y + l.size_px * 0.6)
        })
        .collect();
    let mut shown: Vec<(u32, usize)> = Vec::new();
    for s in 0..map.site_count() as usize {
        if map.site_built[s] > 1990 || map.site_destroyed[s] <= 1990 {
            continue;
        }
        let ok = match map.site_kind[s] {
            4 => false, // industry: no map symbol (the owner asked for it to go)
            0 => min_city.is_some_and(|m| map.site_size[s] >= m),
            1 | 2 => show_ports,
            _ => show_other,
        };
        if !ok {
            continue;
        }
        let c = map.site_cell[s] as usize;
        let (x, y) = crate::heights::to_screen(camera, &data.relief, data.cell_mx[c] as f64, data.cell_my[c] as f64);
        if x < 0.0 || y < 0.0 || x > camera.width_px || y > camera.height_px {
            continue;
        }
        shown.push((map.site_size[s], s));
    }
    // big first, ties by index
    shown.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    shown.truncate(MAX_LABELS);
    let mut out = Vec::new();
    // A port or airfield in the same cell as a city sits beside it, and repeats the name only once.
    let city_cells: std::collections::BTreeSet<u32> = shown.iter().filter(|&&(_, s)| map.site_kind[s] == 0).map(|&(_, s)| map.site_cell[s]).collect();
    let mut named: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut beside: std::collections::BTreeMap<u32, f64> = std::collections::BTreeMap::new();
    for (_, s) in shown {
        let c = map.site_cell[s] as usize;
        let (mut x, mut y) = crate::heights::to_screen(camera, &data.relief, data.cell_mx[c] as f64, data.cell_my[c] as f64);
        if map.site_kind[s] != 0 && city_cells.contains(&map.site_cell[s]) {
            let k = beside.entry(map.site_cell[s]).or_insert(0.0);
            *k += 1.0;
            x -= 12.0 * scale * *k;
            y -= 10.0 * scale;
        }
        let name = map.site_names.get(s).filter(|n| !n.is_empty()).cloned();
        let name = name.filter(|n| named.insert(n.clone()));
        let name = name.filter(|n| {
            let w = n.chars().count() as f64 * size * GLYPH;
            let rect = (x + 5.0 * scale, y - size * 0.6, x + 5.0 * scale + w, y + size * 0.6);
            if taken.iter().any(|t| rect.0 < t.2 && rect.2 > t.0 && rect.1 < t.3 && rect.3 > t.1) {
                return false;
            }
            taken.push(rect);
            true
        });
        let population = if map.site_kind[s] == 0 { map.site_size[s] } else { 0 };
        out.push(SiteMark { kind: map.site_kind[s], x, y, name, size_px: size, population });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use meridian_data::Relief;

    fn data(countries: Labels, regions: Labels) -> RenderData {
        RenderData {
            checksum: 0,
            levels: vec![],
            cell_seg: vec![],
            cell_mx: vec![],
            cell_my: vec![],
            cell_ring_off: vec![0],
            cell_ring_x: vec![],
            cell_ring_y: vec![],
            province_mx: vec![],
            province_my: vec![],
            relief: Relief { width: 1, height: 1, x0: 0, y0: 0, x1: 1, y1: 1, pixels: vec![128], land_rgb: vec![0, 0, 0], height_m: vec![0] },
            river_x: vec![],
            river_y: vec![],
            river_off: vec![0],
            river_rank: vec![],
            roads: Default::default(),
            rails: Default::default(),
            urban_vx: vec![],
            urban_vy: vec![],
            urban_tri: vec![],
            countries,
            regions,
        }
    }

    fn label(name: &str, x: i32, y: i32, area: u32) -> Labels {
        Labels { names: vec![name.to_string()], x: vec![x], y: vec![y], area_km2: vec![area] }
    }

    #[test]
    fn a_name_shows_only_when_its_area_is_wide_enough() {
        // 500,000 km2 at the equator is about 707 km across
        let d = data(label("Examplia", 0, 0, 500_000), Labels::default());
        let mut cam = Camera { cx: 0.0, cy: 0.0, m_per_px: 2_000.0, width_px: 1000.0, height_px: 800.0, bounds: Bounds { min_x: -1e9, min_y: -1e9, max_x: 1e9, max_y: 1e9 } };
        let near = place(&d, &cam, 1.0);
        assert_eq!(near.len(), 1);
        assert_eq!(near[0].text, "EXAMPLIA");
        cam.m_per_px = 200_000.0; // 3.5 px wide: far too small for a name
        assert!(place(&d, &cam, 1.0).is_empty());
        cam.m_per_px = 2_000.0;
        cam.cx = 50_000_000.0; // label off screen
        assert!(place(&d, &cam, 1.0).is_empty());
    }

    #[test]
    fn country_names_disappear_when_zoomed_in() {
        let d = data(label("Examplia", 0, 0, 500_000), Labels::default());
        let b = Bounds { min_x: -1e9, min_y: -1e9, max_x: 1e9, max_y: 1e9 };
        let far = Camera { cx: 0.0, cy: 0.0, m_per_px: 3_000.0, width_px: 1000.0, height_px: 800.0, bounds: b };
        assert_eq!(place(&d, &far, 1.0)[0].alpha, 1.0);
        let fading = Camera { m_per_px: 1_800.0, ..far };
        let a = place(&d, &fading, 1.0)[0].alpha;
        assert!(a > 0.0 && a < 1.0);
        let close = Camera { m_per_px: 1_000.0, ..far };
        assert!(place(&d, &close, 1.0).is_empty());
    }

    #[test]
    fn overlapping_labels_keep_the_bigger_area() {
        let big = label("Bigland", 0, 0, 800_000);
        let small = label("Smallville", 1000, 0, 10_000);
        let mut regions = big.clone();
        regions.names.push(small.names[0].clone()); regions.x.push(small.x[0]); regions.y.push(small.y[0]); regions.area_km2.push(small.area_km2[0]);
        let d = data(Labels::default(), regions);
        let cam = Camera { cx: 0.0, cy: 0.0, m_per_px: 1_000.0, width_px: 1000.0, height_px: 800.0, bounds: Bounds { min_x: -1e9, min_y: -1e9, max_x: 1e9, max_y: 1e9 } };
        let placed = place(&d, &cam, 1.0);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].text, "Bigland");
    }

}
