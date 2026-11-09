//! Pan and zoom over the Equal Earth plane (metres). Screen y grows downward, world y upward.

/// Half the width and height of the Equal Earth world, in metres, with a little margin.
pub const WORLD_HALF_W: f64 = 17_400_000.0;
pub const WORLD_HALF_H: f64 = 8_700_000.0;
/// Closest zoom: metres per screen pixel.
pub const MIN_M_PER_PX: f64 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World point at the middle of the viewport.
    pub cx: f64,
    pub cy: f64,
    pub m_per_px: f64,
    pub width_px: f64,
    pub height_px: f64,
}

impl Camera {
    /// A camera that shows the whole world.
    pub fn fit(width_px: f64, height_px: f64) -> Camera {
        let mut c = Camera { cx: 0.0, cy: 0.0, m_per_px: 1.0, width_px: width_px.max(1.0), height_px: height_px.max(1.0) };
        c.m_per_px = c.max_m_per_px();
        c
    }

    /// Farthest zoom: the whole world fits in the viewport.
    pub fn max_m_per_px(&self) -> f64 {
        (2.0 * WORLD_HALF_W / self.width_px).max(2.0 * WORLD_HALF_H / self.height_px)
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

    /// Keep the view over the world: the centre stays inside the world frame, and a world
    /// narrower than the viewport is centred.
    pub fn clamp(&mut self) {
        self.m_per_px = self.m_per_px.clamp(MIN_M_PER_PX, self.max_m_per_px());
        let half_w = self.width_px * self.m_per_px / 2.0;
        let half_h = self.height_px * self.m_per_px / 2.0;
        self.cx = if half_w >= WORLD_HALF_W { 0.0 } else { self.cx.clamp(-(WORLD_HALF_W - half_w), WORLD_HALF_W - half_w) };
        self.cy = if half_h >= WORLD_HALF_H { 0.0 } else { self.cy.clamp(-(WORLD_HALF_H - half_h), WORLD_HALF_H - half_h) };
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

    #[test]
    fn screen_and_world_round_trip() {
        let c = Camera { cx: 1_000_000.0, cy: -500_000.0, m_per_px: 2_000.0, width_px: 1600.0, height_px: 900.0 };
        let (x, y) = c.screen_to_world(321.0, 654.0);
        let (sx, sy) = c.world_to_screen(x, y);
        assert!((sx - 321.0).abs() < 1e-9 && (sy - 654.0).abs() < 1e-9);
        // the middle of the screen is the centre; up on screen is north
        assert_eq!(c.screen_to_world(800.0, 450.0), (1_000_000.0, -500_000.0));
        assert!(c.screen_to_world(800.0, 0.0).1 > c.cy);
    }

    #[test]
    fn fit_shows_the_whole_world_and_zoom_keeps_the_cursor_point() {
        let mut c = Camera::fit(1600.0, 900.0);
        let (l, _) = c.screen_to_world(0.0, 450.0);
        let (r, _) = c.screen_to_world(1600.0, 450.0);
        assert!(l <= -WORLD_HALF_W + 1.0 && r >= WORLD_HALF_W - 1.0);
        // zoom in at a point near the right edge: the world point under it stays put
        c.zoom_at(1200.0, 300.0, 4.0);
        let before = c.screen_to_world(1200.0, 300.0);
        c.zoom_at(1200.0, 300.0, 2.0);
        let after = c.screen_to_world(1200.0, 300.0);
        assert!((before.0 - after.0).abs() < 1.0 && (before.1 - after.1).abs() < 1.0, "{before:?} {after:?}");
    }

    #[test]
    fn zoom_and_pan_are_limited() {
        let mut c = Camera::fit(1600.0, 900.0);
        c.zoom_at(800.0, 450.0, 1e9);
        assert_eq!(c.m_per_px, MIN_M_PER_PX);
        c.pan_pixels(-1e9, 1e9);
        assert!(c.cx.abs() <= WORLD_HALF_W && c.cy.abs() <= WORLD_HALF_H);
        c.zoom_at(800.0, 450.0, 1e-9);
        assert_eq!(c.m_per_px, c.max_m_per_px());
        assert_eq!((c.cx, c.cy), (0.0, c.cy.clamp(-0.0, 0.0)));
    }

    #[test]
    fn detail_level_follows_zoom() {
        let tol = [8000, 2500, 800];
        let mut c = Camera::fit(1600.0, 900.0);
        assert_eq!(c.level_for(&tol), 0);
        c.m_per_px = 2_000.0;
        assert_eq!(c.level_for(&tol), 1);
        c.m_per_px = 300.0;
        assert_eq!(c.level_for(&tol), 2);
        c.m_per_px = 100.0;
        assert_eq!(c.level_for(&tol), 2);
    }
}
