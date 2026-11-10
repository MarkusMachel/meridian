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

/// Half-way to white.
fn lighten(c: u8) -> u8 {
    ((c as u16 + 255) / 2) as u8
}
