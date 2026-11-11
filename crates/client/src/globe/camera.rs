//! The globe camera: it looks straight down at a point on the Earth from some height, with
//! north up. Dragging turns the Earth under the cursor; the wheel changes the height.

use super::{add, cross, dir_of, dot, lat_lon_of, scale, sub, unit, V3};

/// The radius the map is built on (`meridian_core::sphere::EARTH_RADIUS_M`).
pub const EARTH_RADIUS_M: f64 = meridian_core::sphere::EARTH_RADIUS_M as f64;
/// Tangent of half the vertical field of view (30 degrees top to bottom).
pub const TAN_HALF_FOV: f64 = 0.267_949_192_431_122_7;
/// Closest zoom: metres of ground per pixel at the centre of the screen.
pub const MIN_M_PER_PX: f64 = 20.0;
/// Farthest zoom: the camera three Earth radii above the ground (the whole globe with room).
pub const MAX_ALTITUDE_M: f64 = 3.0 * EARTH_RADIUS_M;
/// The camera never centres nearer a pole than this (north-up has no meaning at the pole).
const MAX_LAT: f64 = 88.0 * std::f64::consts::PI / 180.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobeCamera {
    /// The point looked at: latitude and longitude in radians.
    pub lat: f64,
    pub lon: f64,
    /// Height of the camera above the ground in metres.
    pub altitude_m: f64,
    pub width_px: f64,
    pub height_px: f64,
}

impl GlobeCamera {
    /// The whole globe, centred on a latitude and longitude in degrees.
    pub fn whole_globe(width_px: f64, height_px: f64, lat_deg: f64, lon_deg: f64) -> GlobeCamera {
        let mut c = GlobeCamera { lat: lat_deg.to_radians(), lon: lon_deg.to_radians(), altitude_m: MAX_ALTITUDE_M, width_px, height_px };
        c.clamp();
        c
    }

    /// Looking at a place so that the screen is `km_across` kilometres wide at its centre.
    pub fn looking_at(width_px: f64, height_px: f64, lat_deg: f64, lon_deg: f64, km_across: f64) -> GlobeCamera {
        let mut c = GlobeCamera::whole_globe(width_px, height_px, lat_deg, lon_deg);
        c.set_m_per_px(km_across * 1000.0 / width_px);
        c
    }

    pub fn resize(&mut self, width_px: f64, height_px: f64) {
        let m = self.m_per_px();
        self.width_px = width_px.max(1.0);
        self.height_px = height_px.max(1.0);
        self.set_m_per_px(m);
    }

    /// Metres of ground per pixel at the centre of the screen.
    pub fn m_per_px(&self) -> f64 {
        2.0 * self.altitude_m * TAN_HALF_FOV / self.height_px
    }
    pub fn set_m_per_px(&mut self, m: f64) {
        self.altitude_m = m * self.height_px / (2.0 * TAN_HALF_FOV);
        self.clamp();
    }

    pub fn clamp(&mut self) {
        let lowest = MIN_M_PER_PX * self.height_px / (2.0 * TAN_HALF_FOV);
        self.altitude_m = self.altitude_m.clamp(lowest.min(MAX_ALTITUDE_M), MAX_ALTITUDE_M);
        self.lat = self.lat.clamp(-MAX_LAT, MAX_LAT);
        self.lon = (self.lon + std::f64::consts::PI).rem_euclid(2.0 * std::f64::consts::PI) - std::f64::consts::PI;
    }

    /// The camera's axes: the direction to the point looked at (from the Earth's centre), and
    /// east and north there, which are right and up on the screen.
    pub fn axes(&self) -> (V3, V3, V3) {
        let f = dir_of(self.lat, self.lon);
        let east = unit(cross([0.0, 0.0, 1.0], f));
        (f, east, cross(f, east))
    }

    /// Where the camera is, in metres from the Earth's centre.
    pub fn eye(&self) -> V3 {
        scale(dir_of(self.lat, self.lon), EARTH_RADIUS_M + self.altitude_m)
    }

    fn aspect(&self) -> f64 {
        self.width_px / self.height_px
    }

    /// The point on the ground under a pixel, as a direction from the Earth's centre, or
    /// `None` where the pixel shows space.
    pub fn screen_to_dir(&self, px: f64, py: f64) -> Option<V3> {
        let (f, east, north) = self.axes();
        let x = (2.0 * px / self.width_px - 1.0) * TAN_HALF_FOV * self.aspect();
        let y = (1.0 - 2.0 * py / self.height_px) * TAN_HALF_FOV;
        // the ray leaves the eye along -f + east x + north y; s2 is the squared sine of its
        // angle to the straight-down line, which avoids subtracting two huge numbers
        let len2 = 1.0 + x * x + y * y;
        let s2 = (x * x + y * y) / len2;
        let far = EARTH_RADIUS_M + self.altitude_m;
        let under = EARTH_RADIUS_M * EARTH_RADIUS_M - far * far * s2;
        if under < 0.0 {
            return None;
        }
        let t = far / len2.sqrt() - under.sqrt(); // distance along the ray to the ground
        let ray = unit(add(add(scale(f, -1.0), scale(east, x)), scale(north, y)));
        Some(unit(add(scale(f, far), scale(ray, t))))
    }

    /// The pixel a point shows at (metres from the Earth's centre), or `None` if it is behind
    /// the camera. Does not check whether the Earth is in the way; see `dir_to_screen`.
    pub fn point_to_screen(&self, p: V3) -> Option<(f64, f64)> {
        let (f, east, north) = self.axes();
        let v = sub(p, self.eye());
        let depth = -dot(v, f);
        if depth <= 0.0 {
            return None;
        }
        let x = dot(v, east) / (depth * TAN_HALF_FOV * self.aspect());
        let y = dot(v, north) / (depth * TAN_HALF_FOV);
        Some(((x + 1.0) * 0.5 * self.width_px, (1.0 - y) * 0.5 * self.height_px))
    }

    /// The pixel a point on the ground shows at, or `None` if it is over the horizon.
    pub fn dir_to_screen(&self, d: V3) -> Option<(f64, f64)> {
        let d = unit(d);
        // on the near side of the horizon: the angle from the point looked at is small enough
        if dot(d, dir_of(self.lat, self.lon)) < EARTH_RADIUS_M / (EARTH_RADIUS_M + self.altitude_m) {
            return None;
        }
        self.point_to_screen(scale(d, EARTH_RADIUS_M))
    }

    /// Turn the Earth so that the ground point `from` comes to where `to` is now.
    fn turn(&mut self, from: V3, to: V3) {
        let axis = cross(from, to);
        let s = dot(axis, axis).sqrt();
        if s < 1e-15 {
            return;
        }
        let (axis, angle) = (scale(axis, 1.0 / s), s.atan2(dot(from, to)));
        // the Earth turning one way is the camera going round the other way
        let f = dir_of(self.lat, self.lon);
        let (sin, cos) = (-angle).sin_cos();
        let turned = add(add(scale(f, cos), scale(cross(axis, f), sin)), scale(axis, dot(axis, f) * (1.0 - cos)));
        (self.lat, self.lon) = lat_lon_of(turned);
        self.clamp();
    }

    /// Turn the Earth until the ground point `ground` shows at a pixel. North stays up, so one
    /// turn does not land exactly; each further turn closes most of what is left.
    fn bring(&mut self, ground: V3, px: f64, py: f64) {
        for _ in 0..12 {
            let Some(there) = self.screen_to_dir(px, py) else { return };
            if dot(there, ground) > 1.0 - 1e-16 {
                return;
            }
            self.turn(ground, there);
        }
    }

    /// Drag: the ground that was under the cursor stays under it.
    pub fn pan_pixels(&mut self, from: (f64, f64), to: (f64, f64)) {
        match (self.screen_to_dir(from.0, from.1), self.screen_to_dir(to.0, to.1)) {
            (Some(held), Some(_)) => self.bring(held, to.0, to.1),
            _ => {
                // dragging on space: turn by the angle the pixels span at the centre
                let k = self.m_per_px() / EARTH_RADIUS_M;
                self.lon -= (to.0 - from.0) * k / self.lat.cos().max(0.05);
                self.lat += (to.1 - from.1) * k;
                self.clamp();
            }
        }
    }

    /// Zoom by `factor` (above 1 zooms in), keeping the ground under the cursor where it is.
    pub fn zoom_at(&mut self, px: f64, py: f64, factor: f64) {
        let before = self.screen_to_dir(px, py);
        self.altitude_m /= factor;
        self.clamp();
        if let Some(held) = before {
            self.bring(held, px, py);
        }
    }

    /// How far from the point looked at the screen reaches, as an angle in radians: to the
    /// corner of the screen, or to the horizon where the corner shows space.
    pub fn view_radius(&self) -> f64 {
        let f = dir_of(self.lat, self.lon);
        match self.screen_to_dir(0.0, 0.0) {
            Some(corner) => dot(corner, f).clamp(-1.0, 1.0).acos(),
            None => (EARTH_RADIUS_M / (EARTH_RADIUS_M + self.altitude_m)).acos(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cameras the done test names: the equator, 70 degrees north and across the 180th
    /// meridian, from the whole globe down to the closest zoom.
    fn cameras() -> Vec<GlobeCamera> {
        let mut out = Vec::new();
        for (lat, lon) in [(0.0, 20.0), (70.0, 25.0), (-17.0, 179.9), (52.4, 9.7), (-41.0, -179.8)] {
            for km in [40_000.0, 9_000.0, 2_000.0, 300.0, 51.2] {
                out.push(GlobeCamera::looking_at(2560.0, 1440.0, lat, lon, km));
            }
        }
        out
    }

    #[test]
    fn a_pixel_goes_to_the_ground_and_back_to_the_same_pixel() {
        for c in cameras() {
            let mut on_earth = 0;
            for (px, py) in [(1280.0, 720.0), (0.5, 0.5), (2559.5, 1439.5), (400.0, 1000.0), (2000.0, 100.0), (1281.0, 720.0)] {
                let Some(d) = c.screen_to_dir(px, py) else { continue };
                on_earth += 1;
                let (bx, by) = c.dir_to_screen(d).expect("a point seen is not over the horizon");
                assert!((bx - px).abs() < 1e-4 && (by - py).abs() < 1e-4, "{c:?}: ({px},{py}) came back as ({bx},{by})");
            }
            assert!(on_earth >= 2, "the centre of the screen is always on the Earth");
            // the centre pixel is the point looked at
            let d = c.screen_to_dir(1280.0, 720.0).unwrap();
            assert!(dot(d, dir_of(c.lat, c.lon)) > 1.0 - 1e-12);
        }
    }

    #[test]
    fn zoom_limits_and_scale() {
        let mut c = GlobeCamera::looking_at(2560.0, 1440.0, 52.0, 9.0, 0.001);
        assert!((c.m_per_px() - MIN_M_PER_PX).abs() < 1e-9, "closest zoom is 20 m a pixel");
        // one pixel to the right of centre really is 20 m of ground away
        let a = c.screen_to_dir(1280.0, 720.0).unwrap();
        let b = c.screen_to_dir(1281.0, 720.0).unwrap();
        let apart = dot(a, b).clamp(-1.0, 1.0).acos() * EARTH_RADIUS_M;
        assert!((apart - 20.0).abs() < 0.05, "{apart}");
        c.zoom_at(1280.0, 720.0, 1e-9);
        assert_eq!(c.altitude_m, MAX_ALTITUDE_M);
        assert!(c.screen_to_dir(1.0, 1.0).is_none(), "from far away the corners show space");
        assert!(c.dir_to_screen(scale(dir_of(c.lat, c.lon), -1.0)).is_none(), "the far side is hidden");
    }

    #[test]
    fn the_ground_under_the_cursor_stays_there_when_zooming_and_dragging() {
        for mut c in cameras() {
            let cursor = (1900.0, 420.0);
            let Some(held) = c.screen_to_dir(cursor.0, cursor.1) else { continue };
            c.zoom_at(cursor.0, cursor.1, 1.6);
            let (x, y) = c.dir_to_screen(held).unwrap();
            assert!((x - cursor.0).abs() < 0.5 && (y - cursor.1).abs() < 0.5, "zoom moved the ground to ({x},{y})");

            let to = (1700.0, 600.0);
            let Some(held) = c.screen_to_dir(cursor.0, cursor.1) else { continue };
            c.pan_pixels(cursor, to);
            let (x, y) = c.dir_to_screen(held).unwrap();
            assert!((x - to.0).abs() < 1.0 && (y - to.1).abs() < 1.0, "{c:?}: drag left the ground at ({x},{y})");
        }
    }

    /// No jitter: what the graphics card computes in 32-bit floats from camera-relative
    /// positions lands within a small fraction of a pixel of the exact answer, at the closest
    /// zoom, where an absolute position in 32-bit floats would be off by tens of pixels.
    #[test]
    fn camera_relative_positions_are_exact_enough_in_32_bit_floats() {
        for (lat, lon) in [(0.0, 20.0), (70.0, 25.0), (-17.0, 179.9)] {
            let c = GlobeCamera::looking_at(2560.0, 1440.0, lat, lon, 0.001);
            let (f, east, north) = c.axes();
            let anchor = scale(dir_of(c.lat + 0.006, c.lon - 0.004), EARTH_RADIUS_M); // a tile's anchor, about 45 km off
            let mut worst = 0.0f64;
            for (px, py) in [(10.0, 10.0), (1280.0, 720.0), (2550.0, 1430.0), (333.3, 977.7)] {
                let p = scale(c.screen_to_dir(px, py).unwrap(), EARTH_RADIUS_M);
                // as sent to the card: the vertex relative to its anchor, the anchor relative to the eye
                let local = sub(p, anchor).map(|v| v as f32);
                let offset = sub(anchor, c.eye()).map(|v| v as f32);
                let v = [local[0] + offset[0], local[1] + offset[1], local[2] + offset[2]];
                let d32 = |a: V3| a[0] as f32 * v[0] + a[1] as f32 * v[1] + a[2] as f32 * v[2];
                let depth = -d32(f);
                let x = d32(east) / (depth * (TAN_HALF_FOV * c.width_px / c.height_px) as f32);
                let y = d32(north) / (depth * TAN_HALF_FOV as f32);
                let (sx, sy) = ((x as f64 + 1.0) * 0.5 * c.width_px, (1.0 - y as f64) * 0.5 * c.height_px);
                worst = worst.max((sx - px).abs()).max((sy - py).abs());
            }
            assert!(worst < 0.05, "camera-relative error {worst} px at {lat},{lon}");
        }
    }
}
