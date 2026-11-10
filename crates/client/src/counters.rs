//! Unit counters in the NATO style (the map symbols of APP-6): a framed box in the owner's colour
//! with the unit type drawn inside and the unit size marked above it.
//!
//! The type and size codes are the proving ground's (`proving_ground::unit_kind`, `echelon`);
//! a unit with no type gets an empty frame. Drawn with lines only, so no font or image is needed.

use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2};
use proving_ground::{echelon, unit_kind};

/// Size of a counter's frame in points at scale 1.
pub const WIDTH: f32 = 30.0;
pub const HEIGHT: f32 = 20.0;

/// Draw one counter centred on `center`. `scale` multiplies its size; `color` is the owner's.
pub fn draw(p: &Painter, center: Pos2, scale: f32, kind: u16, size: u8, color: [u8; 3], selected: bool) {
    let (hw, hh) = (WIDTH * 0.5 * scale, HEIGHT * 0.5 * scale);
    let at = |x: f32, y: f32| center + Vec2::new(x * hw, y * hh);      // -1..1 across the frame, y down
    let frame = Rect::from_center_size(center, Vec2::new(hw * 2.0, hh * 2.0));
    // the owner's colour, lightened so the black symbol reads on it
    let fill = Color32::from_rgb(lighten(color[0]), lighten(color[1]), lighten(color[2]));
    let ink = Color32::from_rgb(15, 15, 15);
    let line = Stroke::new((1.5 * scale).max(1.0), ink);
    if selected {
        p.rect_filled(frame.expand(3.0 * scale), 1.0, Color32::WHITE);
    }
    p.rect_filled(frame, 0.0, fill);
    let diagonal_up = |p: &Painter| {
        p.line_segment([at(-1.0, 1.0), at(1.0, -1.0)], line);
    };
    let diagonal_down = |p: &Painter| {
        p.line_segment([at(-1.0, -1.0), at(1.0, 1.0)], line);
    };
    let cross = |p: &Painter| {
        diagonal_up(p);
        diagonal_down(p);
    };
    let track = |p: &Painter| {
        // the armour symbol: a long oval, like a tank track seen from the side
        let pts: Vec<Pos2> = (0..32).map(|k| (k as f32 / 32.0 * std::f32::consts::TAU).sin_cos()).map(|(s, c)| at(c * 0.62, s * 0.50)).collect();
        p.add(egui::Shape::closed_line(pts, line));
    };
    let dot = |p: &Painter| {
        p.circle_filled(center, 3.2 * scale, ink);
    };
    match kind {
        unit_kind::INFANTRY => cross(p),
        unit_kind::ARMOUR => track(p),
        unit_kind::MECHANISED => {
            cross(p);
            track(p);
        }
        unit_kind::MOTORISED => {
            cross(p);
            p.line_segment([at(0.0, -1.0), at(0.0, 1.0)], line);
        }
        unit_kind::ARTILLERY => dot(p),
        unit_kind::SP_ARTILLERY => {
            track(p);
            dot(p);
        }
        unit_kind::RECON => diagonal_up(p),
        unit_kind::ARMOURED_RECON => {
            track(p);
            diagonal_up(p);
        }
        unit_kind::ENGINEER => {
            // a bridge: a deck on three piers
            p.line_segment([at(-0.5, -0.25), at(0.5, -0.25)], line);
            for x in [-0.5, 0.0, 0.5] {
                p.line_segment([at(x, -0.25), at(x, 0.4)], line);
            }
        }
        unit_kind::AIR_DEFENCE => {
            // a dome over the bottom of the frame
            let pts: Vec<Pos2> = (0..=24).map(|k| (k as f32 / 24.0 * std::f32::consts::PI).sin_cos()).map(|(s, c)| at(-c, 1.0 - s * 1.1)).collect();
            p.add(egui::Shape::line(pts, line));
        }
        unit_kind::ANTI_TANK => {
            p.add(egui::Shape::line(vec![at(-1.0, 1.0), at(0.0, -1.0), at(1.0, 1.0)], line));
        }
        unit_kind::AIRBORNE => {
            cross(p);
            // a gull wing (a parachute canopy) at the bottom
            p.add(egui::Shape::line(vec![at(-0.42, 0.78), at(-0.21, 0.5), at(0.0, 0.78), at(0.21, 0.5), at(0.42, 0.78)], line));
        }
        unit_kind::MOUNTAIN => {
            cross(p);
            p.add(egui::Shape::convex_polygon(vec![at(-0.28, 1.0), at(0.0, 0.42), at(0.28, 1.0)], ink, Stroke::NONE));
        }
        unit_kind::SUPPLY => {
            p.line_segment([at(-1.0, 0.5), at(1.0, 0.5)], line);
        }
        unit_kind::HEADQUARTERS => {
            // a headquarters stands on a staff from its lower left corner, and has a bar across the top
            p.line_segment([at(-1.0, 1.0), at(-1.0, 2.0)], line);
            p.line_segment([at(-1.0, -0.45), at(1.0, -0.45)], line);
        }
        _ => {}
    }
    p.rect_stroke(frame, 0.0, Stroke::new((1.8 * scale).max(1.0), ink), egui::StrokeKind::Middle);

    // the size mark above the frame: bars for battalion (II) and regiment (III), crosses for
    // brigade (X), division (XX) and corps (XXX)
    let (marks, crosses) = match size {
        echelon::BATTALION => (2, false),
        echelon::REGIMENT => (3, false),
        echelon::BRIGADE => (1, true),
        echelon::DIVISION => (2, true),
        echelon::CORPS => (3, true),
        _ => (0, false),
    };
    let (mark_h, step) = (5.0 * scale, if crosses { 6.5 * scale } else { 3.6 * scale });
    let top = frame.top() - 2.5 * scale;
    let thin = Stroke::new((1.3 * scale).max(1.0), ink);
    for k in 0..marks {
        let x = center.x + (k as f32 - (marks - 1) as f32 * 0.5) * step;
        if crosses {
            let r = mark_h * 0.5;
            p.line_segment([Pos2::new(x - r, top - mark_h), Pos2::new(x + r, top)], thin);
            p.line_segment([Pos2::new(x - r, top), Pos2::new(x + r, top - mark_h)], thin);
        } else {
            p.line_segment([Pos2::new(x, top - mark_h), Pos2::new(x, top)], thin);
        }
    }
}

/// Group counters that would overlap on screen into stacks, as the map does when zoomed out.
/// `marks` are (x, y, owner, rank) per unit, in screen points; `reach` is how close two counters
/// of the same owner must be (in x and in y) to share a stack. Units are taken highest rank
/// first (ties in the order given), and each joins the first stack of its owner whose top
/// counter is within reach, or starts a new one. Returns the stacks as lists of indices into
/// `marks`, the top counter first; the stack stands where its top counter stands.
pub fn stacks(marks: &[(f32, f32, u16, u8)], reach: (f32, f32)) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..marks.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(marks[i].3), i));
    let mut out: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let (x, y, owner, _) = marks[i];
        let home = out.iter_mut().find(|s| {
            let (tx, ty, towner, _) = marks[s[0]];
            towner == owner && (tx - x).abs() < reach.0 && (ty - y).abs() < reach.1
        });
        match home {
            Some(s) => s.push(i),
            None => out.push(vec![i]),
        }
    }
    out
}

/// Draw a stack of `count` counters: the top one in full, with the edges of two more showing
/// behind it and the number of units in a small plate at its corner.
#[allow(clippy::too_many_arguments)]
pub fn draw_stack(p: &Painter, center: Pos2, scale: f32, kind: u16, size: u8, color: [u8; 3], selected: bool, count: usize) {
    if count > 1 {
        let ink = Color32::from_rgb(15, 15, 15);
        let fill = Color32::from_rgb(lighten(color[0]), lighten(color[1]), lighten(color[2]));
        for k in (1..=count.min(3) - 1).rev() {
            let r = Rect::from_center_size(center + Vec2::new(3.0, -3.0) * k as f32 * scale, Vec2::new(WIDTH, HEIGHT) * scale);
            p.rect_filled(r, 0.0, fill);
            p.rect_stroke(r, 0.0, Stroke::new((1.4 * scale).max(1.0), ink), egui::StrokeKind::Middle);
        }
    }
    draw(p, center, scale, kind, size, color, selected);
    if count > 1 {
        let at = center + Vec2::new(WIDTH * 0.5, HEIGHT * 0.5) * scale;
        let plate = Rect::from_center_size(at, Vec2::new(if count > 9 { 16.0 } else { 11.0 }, 11.0) * scale);
        p.rect_filled(plate, 2.0, Color32::from_rgb(25, 25, 25));
        p.text(at, egui::Align2::CENTER_CENTER, count.to_string(), egui::FontId::proportional(9.0 * scale), Color32::WHITE);
    }
}

/// Half-way to white.
fn lighten(c: u8) -> u8 {
    ((c as u16 + 255) / 2) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_that_overlap_stack_by_owner_with_the_senior_unit_on_top() {
        // two units of country 1 close together (a brigade, then a division), a third of the
        // same country far away, and one of country 2 right on top of the first two
        let marks = [(100.0, 100.0, 1, 3), (110.0, 104.0, 1, 4), (400.0, 100.0, 1, 3), (102.0, 101.0, 2, 3)];
        let s = stacks(&marks, (30.0, 20.0));
        assert_eq!(s, vec![vec![1, 0], vec![2], vec![3]]);
        // zoomed in (the same units farther apart on screen) they stand alone
        let apart = [(100.0, 100.0, 1, 3), (190.0, 136.0, 1, 4), (2_800.0, 100.0, 1, 3), (118.0, 109.0, 2, 3)];
        assert_eq!(stacks(&apart, (30.0, 20.0)).len(), 4);
        // every unit is in exactly one stack
        let mut all: Vec<usize> = s.into_iter().flatten().collect();
        all.sort_unstable();
        assert_eq!(all, vec![0, 1, 2, 3]);
        assert!(stacks(&[], (30.0, 20.0)).is_empty());
    }
}
