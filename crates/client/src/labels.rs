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

const EARTH_RADIUS_M: f64 = 6_378_137.0;
/// Average glyph width as a share of the font size.
const GLYPH: f64 = 0.62;
// Sizes in logical points (multiplied by the screen's scale factor, so they stay readable on
// high-density displays).
const COUNTRY_MAX_PX: f64 = 44.0;
const COUNTRY_MIN_PX: f64 = 13.0;
const REGION_MAX_PX: f64 = 17.0;
const REGION_MIN_PX: f64 = 12.0;
const MAX_LABELS: usize = 500;

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
}

/// How much Mercator stretches lengths at this northing (1 at the equator).
fn stretch(y_m: f64) -> f64 {
    (y_m / EARTH_RADIUS_M).cosh()
}

struct Candidate {
    i: usize,
    kind: LabelKind,
    area: u32,
    size: f64,
    x: f64,
    y: f64,
}

fn candidates(out: &mut Vec<Candidate>, labels: &Labels, kind: LabelKind, camera: &Camera, scale: f64) {
    let (min_px, max_px) = match kind {
        LabelKind::Country => (COUNTRY_MIN_PX * scale, COUNTRY_MAX_PX * scale),
        LabelKind::Region => (REGION_MIN_PX * scale, REGION_MAX_PX * scale),
    };
    for i in 0..labels.names.len() {
        let (x, y) = camera.world_to_screen(labels.x[i] as f64, labels.y[i] as f64);
        if x < 0.0 || y < 0.0 || x > camera.width_px || y > camera.height_px {
            continue;
        }
        let n = labels.names[i].chars().count().max(1) as f64;
        let extent_px = (labels.area_km2[i] as f64).sqrt() * 1000.0 * stretch(labels.y[i] as f64) / camera.m_per_px;
        let size = (extent_px * 0.8 / (n * GLYPH)).min(max_px);
        if size < min_px {
            continue;
        }
        out.push(Candidate { i, kind, area: labels.area_km2[i], size, x, y });
    }
}

/// The labels to draw for this camera, in drawing order.
pub fn place(data: &RenderData, camera: &Camera, scale: f64) -> Vec<PlacedLabel> {
    let mut cand = Vec::new();
    candidates(&mut cand, &data.countries, LabelKind::Country, camera, scale);
    candidates(&mut cand, &data.regions, LabelKind::Region, camera, scale);
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
        out.push(PlacedLabel { text, x: c.x, y: c.y, size_px: c.size, kind: c.kind });
        if out.len() >= MAX_LABELS {
            break;
        }
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
            relief: Relief { width: 1, height: 1, x0: 0, y0: 0, x1: 1, y1: 1, pixels: vec![128] },
            river_x: vec![],
            river_y: vec![],
            river_off: vec![0],
            river_rank: vec![],
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

    #[test]
    fn mercator_stretch_grows_toward_the_poles() {
        assert!((stretch(0.0) - 1.0).abs() < 1e-9);
        // 60 degrees north is about 8.4 million metres in Web Mercator; the stretch there is 1 / cos 60 = 2
        assert!((stretch(8_399_737.0) - 2.0).abs() < 0.01);
    }
}
