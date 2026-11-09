//! Map modes: a mode turns per-province numbers into one colour per province. A mode is only
//! a function, so adding one never touches the renderer.

pub type Rgba = [u8; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Colour by controlling country.
    Political,
    /// Colour by the raw per-province counter (debug).
    Value,
    /// Colour by dominant terrain of the province.
    Terrain,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Political, Mode::Value, Mode::Terrain];
    pub fn name(self) -> &'static str {
        match self {
            Mode::Political => "Political",
            Mode::Value => "Debug value",
            Mode::Terrain => "Terrain",
        }
    }
}

/// A distinct, muted colour for each country: golden-ratio steps around the hue circle, with
/// lightness and saturation cycling so neighbours with close ids still differ.
pub fn country_color(country: u16) -> Rgba {
    let hue = (country as f64 * 0.618_033_988_749_895).fract();
    let s = [0.45, 0.60, 0.35][country as usize % 3];
    let l = [0.62, 0.52, 0.72, 0.45][country as usize % 4];
    hsl(hue, s, l)
}

pub fn political(controllers: &[u16]) -> Vec<Rgba> {
    controllers.iter().map(|&c| country_color(c)).collect()
}

/// Dark blue (lowest) to yellow (highest), scaled over the range present. Equal values give one colour.
pub fn value_ramp(values: &[i64]) -> Vec<Rgba> {
    let lo = values.iter().copied().min().unwrap_or(0);
    let hi = values.iter().copied().max().unwrap_or(0);
    let span = (hi - lo).max(1) as f64;
    values
        .iter()
        .map(|&v| {
            let t = (v - lo) as f64 / span;
            let c = |a: f64, b: f64| (a + (b - a) * t).round() as u8;
            [c(30.0, 250.0), c(40.0, 220.0), c(110.0, 60.0), 255]
        })
        .collect()
}

/// Terrain classes as in the baked map: 0 plains, 1 hills, 2 mountains, 3 high mountains, 4 desert, 5 ice.
pub fn terrain(dominant: &[u8]) -> Vec<Rgba> {
    dominant
        .iter()
        .map(|&t| match t {
            0 => [150, 190, 110, 255],
            1 => [180, 170, 100, 255],
            2 => [150, 120, 90, 255],
            3 => [210, 200, 195, 255],
            4 => [230, 205, 140, 255],
            _ => [235, 245, 250, 255],
        })
        .collect()
}

fn hsl(h: f64, s: f64, l: f64) -> Rgba {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h * 6.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let q = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [q(r), q(g), q(b), 255]
}

/// The most common terrain class among each province's cells (ties go to the lower class).
pub fn dominant_terrain(map: &meridian_core::Map) -> Vec<u8> {
    (0..map.province_count())
        .map(|p| {
            let mut count = [0u32; 6];
            for &c in map.cells_of(meridian_core::ProvinceId(p)) {
                count[(map.cell_terrain[c as usize] as usize).min(5)] += 1;
            }
            (0..6).max_by_key(|&t| (count[t], std::cmp::Reverse(t))).unwrap() as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countries_get_distinct_colours() {
        let mut seen = std::collections::BTreeSet::new();
        for c in 0..173u16 {
            seen.insert(country_color(c));
        }
        assert_eq!(seen.len(), 173);
        assert_eq!(country_color(5), country_color(5));
    }

    #[test]
    fn value_ramp_spans_its_range() {
        let r = value_ramp(&[10, 20, 30]);
        assert_eq!(r[0], [30, 40, 110, 255]);
        assert_eq!(r[2], [250, 220, 60, 255]);
        assert_eq!(value_ramp(&[7, 7]).len(), 2);
        assert!(value_ramp(&[]).is_empty());
    }
}
