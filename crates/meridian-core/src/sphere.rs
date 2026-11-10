//! Distances on the globe in whole numbers (map rewrite, phase M1).
//!
//! A place is a point in three dimensions in whole metres from the Earth's centre, on a
//! sphere of radius `EARTH_RADIUS_M` (the radius the H3 hexagon grid uses). The distance over
//! the ground between two places is the length of the arc between them. It is worked out with
//! integer arithmetic only, so every machine gets the same answer: no floats, no library
//! trigonometry.

/// Mean radius of the Earth in metres (the authalic radius 6,371.0088 km, rounded).
pub const EARTH_RADIUS_M: i64 = 6_371_009;

/// A place on the globe: metres from the Earth's centre along three axes (x through 0°N 0°E,
/// y through 0°N 90°E, z through the north pole).
pub type Place = [i32; 3];

/// Straight-line distance through the Earth between two places, in metres (rounded down).
pub fn chord_m(a: Place, b: Place) -> u32 {
    crate::map::isqrt(chord_squared(a, b)) as u32
}

/// The square of the straight-line distance between two places (at most (2 R)², which fits).
pub fn chord_squared(a: Place, b: Place) -> u64 {
    let d = |k: usize| (a[k] as i64 - b[k] as i64).pow(2) as u64;
    d(0) + d(1) + d(2)
}

/// The arc over the ground for a chord given by its square: `ground_m` for callers that
/// already hold the squared chord.
pub fn ground_from_chord_squared(chord2: u64) -> u32 {
    arc_from_chord_squared(chord2)
}

/// Distance over the ground between two places, in metres: the great-circle arc. It is as
/// exact as the places allow. Places are whole metres, so two places almost on opposite sides
/// of the Earth can be a few metres out (there a metre of position is several metres of arc);
/// at ordinary ranges the answer is good to the metre.
pub fn ground_m(a: Place, b: Place) -> u32 {
    arc_from_chord_squared(chord_squared(a, b))
}

/// The arc over the ground that spans a straight chord of `chord` metres.
pub fn arc_from_chord_m(chord: u32) -> u32 {
    arc_from_chord_squared(chord as u64 * chord as u64)
}

/// The arc for a chord given by its square (so no rounding of the chord gets in).
///
/// arc = 2 R asin(chord / 2R). The angle is halved five times with square roots (which integers
/// can do exactly enough), which leaves an angle so small that asin(s) = s + s³/6 is exact to
/// far below a metre; the result is then scaled back up.
fn arc_from_chord_squared(chord2: u64) -> u32 {
    const ONE: u128 = 1 << 60; // fixed point: 60 fractional bits
    let two_r = 2 * EARTH_RADIUS_M as u128;
    // s = sin(angle / 2), where `angle` is the angle between the places seen from the centre
    let s2 = ((chord2 as u128) << 60) / (two_r * two_r); // s squared, same fixed point
    if s2 >= ONE {
        // opposite sides of the Earth (or rounding just past it): half the circumference
        return half_circumference_m();
    }
    let mut s = isqrt_u128(s2 << 60);
    let mut cos = isqrt_u128((ONE - s2) << 60); // cos x = sqrt(1 - sin² x)
    // sin(x / 2) = sin(x) / sqrt(2 (1 + cos x)), five times
    const HALVINGS: u32 = 5;
    for _ in 0..HALVINGS {
        let denom = isqrt_u128(2 * ONE * (ONE + cos)); // sqrt(2 (1 + cos x)), same fixed point
        s = s * ONE / denom;
        cos = isqrt_u128(ONE * ONE - s * s);
    }
    // now s = sin(angle / 2^6); asin(s) = s + s³/6 (the next term is below one part in 10^9)
    let cube = s * s / ONE * s / ONE;
    let small = s + cube / 6;
    // angle = 2^6 * small; arc = R * angle
    (((small << (HALVINGS + 1)) * EARTH_RADIUS_M as u128) >> 60) as u32
}

/// Half-way round the Earth in metres (pi R, with pi to nine decimal places).
fn half_circumference_m() -> u32 {
    (EARTH_RADIUS_M as u128 * 3_141_592_654 / 1_000_000_000) as u32
}

/// Whole-number square root, rounded down.
fn isqrt_u128(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    // Newton's method from an estimate that is at or above the root
    let mut x = 1u128 << (128 - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A place from latitude and longitude in degrees (floats are fine in a test).
    fn place(lat: f64, lon: f64) -> Place {
        let (la, lo) = (lat.to_radians(), lon.to_radians());
        let r = EARTH_RADIUS_M as f64;
        [(r * la.cos() * lo.cos()).round() as i32, (r * la.cos() * lo.sin()).round() as i32, (r * la.sin()).round() as i32]
    }

    /// The true great-circle distance by the haversine formula, in metres.
    fn truth(a: (f64, f64), b: (f64, f64)) -> f64 {
        let (la1, lo1, la2, lo2) = (a.0.to_radians(), a.1.to_radians(), b.0.to_radians(), b.1.to_radians());
        let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_M as f64 * h.sqrt().asin()
    }

    #[test]
    fn ground_distance_matches_the_great_circle_at_every_range() {
        let cities = [
            ("Hannover", 52.37, 9.73),
            ("Braunschweig", 52.27, 10.52),
            ("Baghdad", 33.31, 44.37),
            ("Kuwait City", 29.38, 47.98),
            ("Moscow", 55.75, 37.62),
            ("Washington", 38.90, -77.04),
            ("Quito", -0.18, -78.47),
            ("Singapore", 1.35, 103.82),
            ("Tromso", 69.65, 18.96),
            ("Wellington", -41.29, 174.78),
            ("Suva", -18.14, 178.44),
            ("Apia", -13.83, -171.77),
        ];
        let mut worst = 0.0f64;
        for a in &cities {
            for b in &cities {
                let got = ground_m(place(a.1, a.2), place(b.1, b.2)) as f64;
                let want = truth((a.1, a.2), (b.1, b.2));
                // the places are rounded to a metre; that is a metre or two of arc at ordinary
                // ranges and more the nearer the two are to opposite sides of the Earth
                let half_angle = want / (2.0 * EARTH_RADIUS_M as f64);
                let allow = 2.0 + 3.0 / half_angle.cos().max(0.02);
                assert!((got - want).abs() < allow, "{} to {}: {got} against {want}", a.0, b.0);
                if want < 5_000_000.0 {
                    worst = worst.max((got - want).abs());
                }
            }
        }
        assert!(worst < 4.0, "up to 5,000 km the answer is within a few metres: {worst}");
        // across the 180th meridian it is the short way round (Suva to Apia is about 1,150 km)
        let d = ground_m(place(-18.14, 178.44), place(-13.83, -171.77));
        assert!((1_100_000..1_200_000).contains(&d), "{d}");
    }

    #[test]
    fn the_arc_is_never_shorter_than_the_chord_and_ends_at_half_the_circumference() {
        assert_eq!(arc_from_chord_m(0), 0);
        // neighbouring hexagons (a few kilometres): arc and chord agree to the metre
        for c in [1u32, 500, 3_700, 6_400, 17_000] {
            assert!(arc_from_chord_m(c).abs_diff(c) <= 1, "{c}");
        }
        let mut last = 0;
        for c in (0..=2 * EARTH_RADIUS_M as u32).step_by(37_337) {
            let arc = arc_from_chord_m(c);
            assert!(arc + 1 >= c && arc >= last, "chord {c}: arc {arc}");
            last = arc;
        }
        // the far side of the Earth: half the circumference, 20,015 km
        let half = arc_from_chord_m(2 * EARTH_RADIUS_M as u32);
        assert!(half.abs_diff(20_015_115) < 2_000, "{half}");
        assert_eq!(chord_m([EARTH_RADIUS_M as i32, 0, 0], [-(EARTH_RADIUS_M as i32), 0, 0]), 2 * EARTH_RADIUS_M as u32);
    }

    #[test]
    fn square_roots_round_down() {
        for n in [0u128, 1, 2, 3, 4, 15, 16, 17, 1 << 60, (1 << 120) - 1, u128::MAX] {
            let r = isqrt_u128(n);
            assert!(r * r <= n && (r + 1).checked_mul(r + 1).is_none_or(|sq| sq > n), "{n}");
        }
    }
}
