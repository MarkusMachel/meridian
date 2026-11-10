//! Pan and zoom over the display plane (Equal Earth metres). Screen y grows downward, world y upward.

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
    /// World point the camera looks at (on the ground, sea level), shown in the middle of the viewport.
    pub cx: f64,
    pub cy: f64,
    /// Zoom: metres per screen pixel at the middle of the screen.
    pub m_per_px: f64,
    pub width_px: f64,
    pub height_px: f64,
    pub bounds: Bounds,
}

/// Vertical field of view of the 3D camera.
pub const FOV_Y_DEG: f64 = 30.0;
/// At this zoom and farther out the map is the flat "paper map" seen straight down ...
pub const PAPER_FROM_M_PER_PX: f64 = 6_000.0;
/// ... and at this zoom and closer it is fully 3D: tilted, with raised terrain.
pub const FULL_3D_BELOW_M_PER_PX: f64 = 700.0;
/// Tilt from straight down when fully 3D.
pub const MAX_PITCH_DEG: f64 = 48.0;
/// How much the terrain is raised when fully 3D (real heights are invisible at map scale).
pub const MAX_EXAGGERATION: f64 = 14.0;

type Mat4 = [[f64; 4]; 4]; // column-major: m[column][row], as the GPU expects

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            out[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm3(a: [f64; 3]) -> [f64; 3] {
    let l = dot3(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
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

    /// How 3D the view is: 0 for the flat paper map (zoomed out), 1 fully tilted (zoomed in),
    /// a smooth step between, measured on a log scale of zoom.
    pub fn depth_t(&self) -> f64 {
        let a = (PAPER_FROM_M_PER_PX.ln() - self.m_per_px.ln()) / (PAPER_FROM_M_PER_PX.ln() - FULL_3D_BELOW_M_PER_PX.ln());
        let t = a.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Tilt from straight down, radians.
    pub fn pitch(&self) -> f64 {
        MAX_PITCH_DEG.to_radians() * self.depth_t()
    }

    /// Factor applied to real heights (metres) for drawing.
    pub fn exaggeration(&self) -> f64 {
        MAX_EXAGGERATION * self.depth_t()
    }

    /// Distance from the eye to the target point, chosen so a pixel at the middle of the screen
    /// covers `m_per_px` metres.
    pub fn distance(&self) -> f64 {
        self.m_per_px * self.height_px / (2.0 * (FOV_Y_DEG.to_radians() / 2.0).tan())
    }

    /// The eye, relative to the target point (the camera looks north, from the south).
    pub fn eye(&self) -> [f64; 3] {
        let (d, p) = (self.distance(), self.pitch());
        [0.0, -d * p.sin(), d * p.cos()]
    }

    /// Projection times view, for points given relative to the target (x - cx, y - cy, z).
    /// Depth runs 0 (near) to 1 (far), as wgpu expects.
    pub fn view_proj(&self) -> [[f64; 4]; 4] {
        let eye = self.eye();
        let d = self.distance();
        let (near, far) = (d * 0.05, d * 20.0);
        let f = norm3(sub3([0.0, 0.0, 0.0], eye));
        let s = norm3(cross3(f, [0.0, 1.0, 0.0]));
        let u = cross3(s, f);
        let view: Mat4 = [[s[0], u[0], -f[0], 0.0], [s[1], u[1], -f[1], 0.0], [s[2], u[2], -f[2], 0.0], [-dot3(s, eye), -dot3(u, eye), dot3(f, eye), 1.0]];
        let g = 1.0 / (FOV_Y_DEG.to_radians() / 2.0).tan();
        let aspect = self.width_px / self.height_px;
        let proj: Mat4 = [[g / aspect, 0.0, 0.0, 0.0], [0.0, g, 0.0, 0.0], [0.0, 0.0, far / (near - far), -1.0], [0.0, 0.0, near * far / (near - far), 0.0]];
        mul(&proj, &view)
    }

    /// Screen position of a world point at height `z` (drawn metres, already exaggerated).
    /// Points behind the camera come back far off screen.
    pub fn world_to_screen_z(&self, x: f64, y: f64, z: f64) -> (f64, f64) {
        let m = self.view_proj();
        let p = [x - self.cx, y - self.cy, z, 1.0];
        let c: Vec<f64> = (0..4).map(|r| (0..4).map(|k| m[k][r] * p[k]).sum()).collect();
        if c[3] <= 1e-9 {
            return (-1e9, -1e9);
        }
        ((c[0] / c[3] + 1.0) / 2.0 * self.width_px, (1.0 - c[1] / c[3]) / 2.0 * self.height_px)
    }

    /// Screen position of a point on the ground at sea level.
    pub fn world_to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        self.world_to_screen_z(x, y, 0.0)
    }

    /// The ray from the eye through a screen pixel: (origin, direction), relative to the target.
    fn ray(&self, px: f64, py: f64) -> ([f64; 3], [f64; 3]) {
        let eye = self.eye();
        let f = norm3(sub3([0.0, 0.0, 0.0], eye));
        let s = norm3(cross3(f, [0.0, 1.0, 0.0]));
        let u = cross3(s, f);
        let th = (FOV_Y_DEG.to_radians() / 2.0).tan();
        let nx = (2.0 * px / self.width_px - 1.0) * th * self.width_px / self.height_px;
        let ny = (1.0 - 2.0 * py / self.height_px) * th;
        (eye, [f[0] + s[0] * nx + u[0] * ny, f[1] + s[1] * nx + u[1] * ny, f[2] + s[2] * nx + u[2] * ny])
    }

    /// The world point under a screen pixel where the ray meets height `z` (drawn metres).
    pub fn screen_to_world_z(&self, px: f64, py: f64, z: f64) -> (f64, f64) {
        let (o, d) = self.ray(px, py);
        // the camera never looks above the horizon (pitch plus half the field of view stays under 90 degrees)
        let t = if d[2] < -1e-9 { (z - o[2]) / d[2] } else { 0.0 };
        (self.cx + o[0] + t * d[0], self.cy + o[1] + t * d[1])
    }

    /// The point at sea level under a screen pixel.
    pub fn screen_to_world(&self, px: f64, py: f64) -> (f64, f64) {
        self.screen_to_world_z(px, py, 0.0)
    }

    /// The point on the terrain under a pixel, given the drawn ground height at any world point.
    pub fn screen_to_ground(&self, px: f64, py: f64, height: impl Fn(f64, f64) -> f64) -> (f64, f64) {
        let mut p = self.screen_to_world(px, py);
        for _ in 0..6 {
            p = self.screen_to_world_z(px, py, height(p.0, p.1));
        }
        p
    }

    /// The ground (sea level) area the screen shows, as a box (min x, min y, max x, max y).
    pub fn footprint(&self) -> (f64, f64, f64, f64) {
        let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (px, py) in [(0.0, 0.0), (self.width_px, 0.0), (0.0, self.height_px), (self.width_px, self.height_px)] {
            let (x, y) = self.screen_to_world(px, py);
            b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
        }
        b
    }

    /// Drag the map by a number of screen pixels: the ground under the cursor follows it.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        let (cx, cy) = (self.width_px / 2.0, self.height_px / 2.0);
        let a = self.screen_to_world(cx, cy);
        let b = self.screen_to_world(cx - dx, cy - dy);
        self.cx += b.0 - a.0;
        self.cy += b.1 - a.1;
        self.clamp();
    }

    /// Zoom by `factor` (above 1 zooms in) keeping the world point under the cursor fixed.
    pub fn zoom_at(&mut self, px: f64, py: f64, factor: f64) {
        let before = self.screen_to_world(px, py);
        self.m_per_px = (self.m_per_px / factor).clamp(MIN_M_PER_PX, self.max_m_per_px());
        let after = self.screen_to_world(px, py);
        self.cx += before.0 - after.0;
        self.cy += before.1 - after.1;
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
        // zoomed in the view is tilted, so test the point looked at (the corners see beyond it)
        assert!(c.cx >= b.min_x && c.cx <= b.max_x && c.cy >= b.min_y && c.cy <= b.max_y);
        c.zoom_at(800.0, 450.0, 1e-9);
        assert_eq!(c.m_per_px, c.max_m_per_px());
        assert_eq!((c.cx, c.cy), b.center());
    }

    #[test]
    fn tilted_view_round_trips_and_flattens_when_zoomed_out() {
        let mut c = Camera { cx: 1_000_000.0, cy: 5_000_000.0, m_per_px: 400.0, width_px: 1600.0, height_px: 900.0, bounds: world() };
        assert!(c.pitch() > 0.7 && c.exaggeration() > 10.0);
        for (px, py) in [(100.0, 100.0), (800.0, 450.0), (1500.0, 800.0)] {
            let (x, y) = c.screen_to_world(px, py);
            let (sx, sy) = c.world_to_screen(x, y);
            assert!((sx - px).abs() < 1e-6 && (sy - py).abs() < 1e-6, "{px},{py} -> {sx},{sy}");
        }
        // a raised point appears higher on screen than the same point at sea level
        assert!(c.world_to_screen_z(c.cx, c.cy, 20_000.0).1 < c.world_to_screen(c.cx, c.cy).1);
        // the top of a tilted screen sees farther than the bottom
        let fp = c.footprint();
        assert!(fp.3 - c.cy > c.cy - fp.1);
        c.m_per_px = 8_000.0;
        assert_eq!(c.pitch(), 0.0);
        assert_eq!(c.exaggeration(), 0.0);
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
