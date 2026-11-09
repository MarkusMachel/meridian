//! Which province is under a world point. Built from the finest render mesh, so what you click
//! is exactly what is drawn. A uniform grid holds the triangles that touch each grid square.

use crate::camera::Bounds;
use meridian_data::{RenderData, RenderLevel};

const COLS: usize = 512;
const ROWS: usize = 256;

pub struct Picker {
    bounds: Bounds,
    /// Each triangle's three corners as x, y pairs.
    tris: Vec<[i32; 6]>,
    owner: Vec<u32>,
    /// Grid square -> range into `items`.
    start: Vec<u32>,
    items: Vec<u32>,
}

fn square_of(b: &Bounds, x: f64, y: f64) -> Option<(usize, usize)> {
    let fx = (x - b.min_x) / b.width();
    let fy = (y - b.min_y) / b.height();
    if !(0.0..1.0).contains(&fx) || !(0.0..1.0).contains(&fy) {
        return None;
    }
    Some(((fx * COLS as f64) as usize, (fy * ROWS as f64) as usize))
}

impl Picker {
    pub fn new(level: &RenderLevel, bounds: Bounds) -> Picker {
        let n = level.tri.len() / 3;
        let mut tris = Vec::with_capacity(n);
        let mut owner = Vec::with_capacity(n);
        for t in level.tri.chunks_exact(3) {
            let v = |i: u32| (level.vx[i as usize], level.vy[i as usize]);
            let (a, b, c) = (v(t[0]), v(t[1]), v(t[2]));
            tris.push([a.0, a.1, b.0, b.1, c.0, c.1]);
            owner.push(level.vprov[t[0] as usize]);
        }
        // two passes: count per square, then fill
        let mut count = vec![0u32; COLS * ROWS + 1];
        let each_square = |f: &mut dyn FnMut(usize, usize)| {
            for (i, t) in tris.iter().enumerate() {
                let (x0, x1) = (t[0].min(t[2]).min(t[4]) as f64, t[0].max(t[2]).max(t[4]) as f64);
                let (y0, y1) = (t[1].min(t[3]).min(t[5]) as f64, t[1].max(t[3]).max(t[5]) as f64);
                let (Some((c0, r0)), Some((c1, r1))) = (square_of(&bounds, x0, y0), square_of(&bounds, x1, y1)) else { continue };
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        f(i, r * COLS + c);
                    }
                }
            }
        };
        each_square(&mut |_, sq| count[sq + 1] += 1);
        for i in 0..COLS * ROWS {
            count[i + 1] += count[i];
        }
        let start = count.clone();
        let mut fill = count;
        let mut items = vec![0u32; *start.last().unwrap() as usize];
        each_square(&mut |i, sq| {
            items[fill[sq] as usize] = i as u32;
            fill[sq] += 1;
        });
        Picker { bounds, tris, owner, start, items }
    }

    /// The province whose triangle contains the point, if any (sea and gaps give `None`).
    pub fn province_at(&self, x: f64, y: f64) -> Option<u32> {
        let (c, r) = square_of(&self.bounds, x, y)?;
        let sq = r * COLS + c;
        let (px, py) = (x, y);
        for &i in &self.items[self.start[sq] as usize..self.start[sq + 1] as usize] {
            let t = &self.tris[i as usize];
            let (ax, ay, bx, by, cx, cy) = (t[0] as f64, t[1] as f64, t[2] as f64, t[3] as f64, t[4] as f64, t[5] as f64);
            let d = |x1: f64, y1: f64, x2: f64, y2: f64| (px - x2) * (y1 - y2) - (x1 - x2) * (py - y2);
            let (d1, d2, d3) = (d(ax, ay, bx, by), d(bx, by, cx, cy), d(cx, cy, ax, ay));
            let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            if !(neg && pos) {
                return Some(self.owner[i as usize]);
            }
        }
        None
    }
}

/// Which cell is under a point. A grid over the cell centres finds the few cells near the point,
/// and the point is tested against each one's outline, so a click hits exactly the cell drawn there.
pub struct CellPicker {
    bounds: Bounds,
    cols: usize,
    rows: usize,
    square_m: f64,
    start: Vec<u32>,
    items: Vec<u32>,
}

impl CellPicker {
    pub fn new(data: &RenderData, bounds: Bounds) -> CellPicker {
        // Squares of about 50 km: cells are about 15 km across (a little more near the edges of the projection).
        let square_m = 50_000.0;
        let cols = (bounds.width() / square_m).ceil() as usize + 1;
        let rows = (bounds.height() / square_m).ceil() as usize + 1;
        let square = |x: i32, y: i32| (((x as f64 - bounds.min_x) / square_m) as usize, ((y as f64 - bounds.min_y) / square_m) as usize);
        let mut count = vec![0u32; cols * rows + 1];
        for i in 0..data.cell_mx.len() {
            let (c, r) = square(data.cell_mx[i], data.cell_my[i]);
            count[r.min(rows - 1) * cols + c.min(cols - 1) + 1] += 1;
        }
        for i in 0..cols * rows {
            count[i + 1] += count[i];
        }
        let start = count.clone();
        let mut fill = count;
        let mut items = vec![0u32; data.cell_mx.len()];
        for i in 0..data.cell_mx.len() {
            let (c, r) = square(data.cell_mx[i], data.cell_my[i]);
            let sq = r.min(rows - 1) * cols + c.min(cols - 1);
            items[fill[sq] as usize] = i as u32;
            fill[sq] += 1;
        }
        CellPicker { bounds, cols, rows, square_m, start, items }
    }

    /// The cell whose outline contains the point, if any (sea gives `None`).
    pub fn cell_at(&self, data: &RenderData, x: f64, y: f64) -> Option<u32> {
        let (fx, fy) = ((x - self.bounds.min_x) / self.square_m, (y - self.bounds.min_y) / self.square_m);
        if fx < 0.0 || fy < 0.0 {
            return None;
        }
        let (c, r) = (fx as usize, fy as usize);
        if c >= self.cols || r >= self.rows {
            return None;
        }
        // nearest centres first, so the usual answer is found on the first test
        let mut near: Vec<(f64, u32)> = Vec::new();
        for rr in r.saturating_sub(1)..=(r + 1).min(self.rows - 1) {
            for cc in c.saturating_sub(1)..=(c + 1).min(self.cols - 1) {
                let sq = rr * self.cols + cc;
                for &i in &self.items[self.start[sq] as usize..self.start[sq + 1] as usize] {
                    let (dx, dy) = (data.cell_mx[i as usize] as f64 - x, data.cell_my[i as usize] as f64 - y);
                    near.push((dx * dx + dy * dy, i));
                }
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        near.into_iter().map(|(_, i)| i).find(|&i| inside(data, i as usize, x, y))
    }
}

/// Is the point inside the cell's outline (even-odd rule)?
fn inside(data: &RenderData, cell: usize, x: f64, y: f64) -> bool {
    let (a, b) = (data.cell_ring_off[cell] as usize, data.cell_ring_off[cell + 1] as usize);
    let n = b - a;
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (data.cell_ring_x[a + i] as f64, data.cell_ring_y[a + i] as f64);
        let (xj, yj) = (data.cell_ring_x[a + j] as f64, data.cell_ring_y[a + j] as f64);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_level() -> RenderLevel {
        // province 0: square (0,0)-(1000,1000); province 1: square (1000,0)-(2000,1000)
        RenderLevel {
            tolerance_m: 1,
            vx: vec![0, 1000, 1000, 0, 1000, 2000, 2000, 1000],
            vy: vec![0, 0, 1000, 1000, 0, 0, 1000, 1000],
            vprov: vec![0, 0, 0, 0, 1, 1, 1, 1],
            tri: vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
            line_x: vec![],
            line_y: vec![],
            line_off: vec![0],
            line_kind: vec![],
        }
    }

    #[test]
    fn finds_the_triangle_under_a_point() {
        let p = Picker::new(&square_level(), Bounds { min_x: -100.0, min_y: -100.0, max_x: 2100.0, max_y: 1100.0 });
        assert_eq!(p.province_at(100.0, 100.0), Some(0));
        assert_eq!(p.province_at(900.0, 950.0), Some(0));
        assert_eq!(p.province_at(1500.0, 500.0), Some(1));
        assert_eq!(p.province_at(5000.0, 500.0), None);
        assert_eq!(p.province_at(-50.0, 500.0), None);
        // far outside the map frame
        assert_eq!(p.province_at(1e9, 0.0), None);
    }

    fn two_cells() -> RenderData {
        // cell 0: square (0,0)-(1000,1000); cell 1: triangle (1000,0), (3000,0), (1000,2000)
        RenderData {
            checksum: 0,
            levels: vec![],
            cell_seg: vec![],
            cell_mx: vec![500, 1500],
            cell_my: vec![500, 700],
            cell_ring_off: vec![0, 4, 7],
            cell_ring_x: vec![0, 1000, 1000, 0, 1000, 3000, 1000],
            cell_ring_y: vec![0, 0, 1000, 1000, 0, 0, 2000],
            province_mx: vec![],
            province_my: vec![],
            relief: meridian_data::Relief { width: 1, height: 1, x0: 0, y0: 0, x1: 1, y1: 1, pixels: vec![128] },
            river_x: vec![],
            river_y: vec![],
            river_off: vec![0],
            river_rank: vec![],
            countries: Default::default(),
            regions: Default::default(),
        }
    }

    #[test]
    fn cell_picker_tests_the_outline_not_just_the_nearest_centre() {
        let d = two_cells();
        let b = Bounds { min_x: -100.0, min_y: -100.0, max_x: 3100.0, max_y: 2100.0 };
        let p = CellPicker::new(&d, b);
        assert_eq!(p.cell_at(&d, 100.0, 100.0), Some(0));
        assert_eq!(p.cell_at(&d, 1500.0, 500.0), Some(1));
        // nearer to cell 1's centre (1500, 700) than cell 0's (500, 500), but inside cell 0's outline
        assert_eq!(p.cell_at(&d, 990.0, 900.0), Some(0));
        // in the bounds but in no cell (above the triangle's slanted edge)
        assert_eq!(p.cell_at(&d, 2800.0, 1500.0), None);
        assert_eq!(p.cell_at(&d, -50.0, -50.0), None);
        assert_eq!(p.cell_at(&d, 1e9, 0.0), None);
    }
}
