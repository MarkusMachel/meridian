//! Pan and zoom over the display plane (Web Mercator metres). Screen y grows downward, world y upward.

/// Closest zoom: metres per screen pixel.
pub const MIN_M_PER_PX: f64 = 100.0;

/// The part of the plane that has map on it. The view never leaves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bounds {
    /// Bounds of the drawn map with a small margin all round.
    pub fn around(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> Bounds {
        let (w, h) = ((max_x - min_x) as f64, (max_y - min_y) as f64);
        let m = 0.02 * w.max(h);
        Bounds { min_x: min_x as f64 - m, min_y: min_y as f64 - m, max_x: max_x as f64 + m, max_y: max_y as f64 + m }
    }
    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }
    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }
    pub fn center(&self) -> (f64, f64) {
        ((self.min_x + self.max_x) / 2.0, (self.min_y + self.max_y) / 2.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World point at the middle of the viewport.
    pub cx: f64,
    pub cy: f64,
    pub m_per_px: f64,
    pub width_px: f64,
    pub height_px: f64,
    pub bounds: Bounds,
}

impl Camera {
    /// A camera that shows the whole world.
    pub fn fit(width_px: f64, height_px: f64, bounds: Bounds) -> Camera {
        let (cx, cy) = bounds.center();
        let mut c = Camera { cx, cy, m_per_px: 1.0, width_px: width_px.max(1.0), height_px: height_px.max(1.0), bounds };
        c.m_per_px = c.max_m_per_px();
        c
    }

    /// Farthest zoom: the whole world fits in the viewport.
    pub fn max_m_per_px(&self) -> f64 {
        (self.bounds.width() / self.width_px).max(self.bounds.height() / self.height_px)
    }

    pub fn resize(&mut self, width_px: f64, height_px: f64) {
        self.width_px = width_px.max(1.0);
        self.height_px = height_px.max(1.0);
        self.clamp();
    }

    pub fn world_to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.cx) / self.m_per_px + self.width_px / 2.0, (self.cy - y) / self.m_per_px + self.height_px / 2.0)
    }

    pub fn screen_to_world(&self, px: f64, py: f64) -> (f64, f64) {
        (self.cx + (px - self.width_px / 2.0) * self.m_per_px, self.cy - (py - self.height_px / 2.0) * self.m_per_px)
    }

    /// Drag the map by a number of screen pixels.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        self.cx -= dx * self.m_per_px;
        self.cy += dy * self.m_per_px;
        self.clamp();
    }

    /// Zoom by `factor` (above 1 zooms in) keeping the world point under the cursor fixed.
    pub fn zoom_at(&mut self, px: f64, py: f64, factor: f64) {
        let (wx, wy) = self.screen_to_world(px, py);
        self.m_per_px = (self.m_per_px / factor).clamp(MIN_M_PER_PX, self.max_m_per_px());
        self.cx = wx - (px - self.width_px / 2.0) * self.m_per_px;
        self.cy = wy + (py - self.height_px / 2.0) * self.m_per_px;
        self.clamp();
    }

    /// Keep the view over the map: the viewport stays inside the bounds, and a map smaller than
    /// the viewport (in one direction) is centred in it.
    pub fn clamp(&mut self) {
        self.m_per_px = self.m_per_px.clamp(MIN_M_PER_PX, self.max_m_per_px());
        let b = self.bounds;
        let half_w = self.width_px * self.m_per_px / 2.0;
        let half_h = self.height_px * self.m_per_px / 2.0;
        let (mid_x, mid_y) = b.center();
        self.cx = if half_w * 2.0 >= b.width() { mid_x } else { self.cx.clamp(b.min_x + half_w, b.max_x - half_w) };
        self.cy = if half_h * 2.0 >= b.height() { mid_y } else { self.cy.clamp(b.min_y + half_h, b.max_y - half_h) };
    }

    /// Which detail level to draw. `tolerances_m` runs coarsest first. Pick the coarsest level
    /// whose simplification error is under two pixels at this zoom, else the finest.
    pub fn level_for(&self, tolerances_m: &[u32]) -> usize {
        tolerances_m.iter().position(|&t| (t as f64) <= 2.0 * self.m_per_px).unwrap_or(tolerances_m.len().saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> Bounds {
        Bounds { min_x: -20_000_000.0, min_y: -8_000_000.0, max_x: 20_000_000.0, max_y: 18_000_000.0 }
    }

    #[test]
    fn screen_and_world_round_trip() {
        let c = Camera { cx: 1_000_000.0, cy: -500_000.0, m_per_px: 2_000.0, width_px: 1600.0, height_px: 900.0, bounds: world() };
        let (x, y) = c.screen_to_world(321.0, 654.0);
        let (sx, sy) = c.world_to_screen(x, y);
        assert!((sx - 321.0).abs() < 1e-9 && (sy - 654.0).abs() < 1e-9);
        // the middle of the screen is the centre; up on screen is north
        assert_eq!(c.screen_to_world(800.0, 450.0), (1_000_000.0, -500_000.0));
        assert!(c.screen_to_world(800.0, 0.0).1 > c.cy);
    }

    #[test]
    fn fit_shows_the_whole_map_and_zoom_keeps_the_cursor_point() {
        let mut c = Camera::fit(1600.0, 900.0, world());
        let b = world();
        let (l, t) = c.screen_to_world(0.0, 0.0);
        let (r, bt) = c.screen_to_world(1600.0, 900.0);
        assert!(l <= b.min_x + 1.0 && r >= b.max_x - 1.0 && t >= b.max_y - 1.0 && bt <= b.min_y + 1.0);
        c.zoom_at(1200.0, 300.0, 4.0);
        let before = c.screen_to_world(1200.0, 300.0);
        c.zoom_at(1200.0, 300.0, 2.0);
        let after = c.screen_to_world(1200.0, 300.0);
        assert!((before.0 - after.0).abs() < 1.0 && (before.1 - after.1).abs() < 1.0, "{before:?} {after:?}");
    }

    #[test]
    fn zoom_and_pan_are_limited() {
        let b = world();
        let mut c = Camera::fit(1600.0, 900.0, b);
        c.zoom_at(800.0, 450.0, 1e9);
        assert_eq!(c.m_per_px, MIN_M_PER_PX);
        c.pan_pixels(-1e12, 1e12);
        let (l, t) = c.screen_to_world(0.0, 0.0);
        let (r, bt) = c.screen_to_world(1600.0, 900.0);
        assert!(l >= b.min_x - 1.0 && r <= b.max_x + 1.0 && t <= b.max_y + 1.0 && bt >= b.min_y - 1.0);
        c.zoom_at(800.0, 450.0, 1e-9);
        assert_eq!(c.m_per_px, c.max_m_per_px());
        assert_eq!((c.cx, c.cy), b.center());
    }

    #[test]
    fn detail_level_follows_zoom() {
        let tol = [8000, 2500, 800];
        let mut c = Camera::fit(1600.0, 900.0, world());
        assert_eq!(c.level_for(&tol), 0);
        c.m_per_px = 2_000.0;
        assert_eq!(c.level_for(&tol), 1);
        c.m_per_px = 300.0;
        assert_eq!(c.level_for(&tol), 2);
        c.m_per_px = 100.0;
        assert_eq!(c.level_for(&tol), 2);
    }
}
