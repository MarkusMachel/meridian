//! Which province is under a world point. Built from the finest render mesh, so what you click
//! is exactly what is drawn. A uniform grid holds the triangles that touch each grid square.

use crate::camera::{WORLD_HALF_H, WORLD_HALF_W};
use meridian_data::RenderLevel;

const COLS: usize = 512;
const ROWS: usize = 256;

pub struct Picker {
    /// Each triangle's three corners as x, y pairs.
    tris: Vec<[i32; 6]>,
    owner: Vec<u32>,
    /// Grid square -> range into `items`.
    start: Vec<u32>,
    items: Vec<u32>,
}

fn square_of(x: f64, y: f64) -> Option<(usize, usize)> {
    let fx = (x + WORLD_HALF_W) / (2.0 * WORLD_HALF_W);
    let fy = (y + WORLD_HALF_H) / (2.0 * WORLD_HALF_H);
    if !(0.0..1.0).contains(&fx) || !(0.0..1.0).contains(&fy) {
        return None;
    }
    Some(((fx * COLS as f64) as usize, (fy * ROWS as f64) as usize))
}

impl Picker {
    pub fn new(level: &RenderLevel) -> Picker {
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
                let (Some((c0, r0)), Some((c1, r1))) = (square_of(x0, y0), square_of(x1, y1)) else { continue };
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
        Picker { tris, owner, start, items }
    }

    /// The province whose triangle contains the point, if any (sea and gaps give `None`).
    pub fn province_at(&self, x: f64, y: f64) -> Option<u32> {
        let (c, r) = square_of(x, y)?;
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
        let p = Picker::new(&square_level());
        assert_eq!(p.province_at(100.0, 100.0), Some(0));
        assert_eq!(p.province_at(900.0, 950.0), Some(0));
        assert_eq!(p.province_at(1500.0, 500.0), Some(1));
        assert_eq!(p.province_at(5000.0, 500.0), None);
        assert_eq!(p.province_at(-50.0, 500.0), None);
        // far outside the world frame
        assert_eq!(p.province_at(1e9, 0.0), None);
    }
}
