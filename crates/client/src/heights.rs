//! Ground heights for the 3D view, on the CPU side: labels, icons and the mouse need to know how
//! high the drawn terrain is at a point so they line up with what the GPU draws.

use crate::camera::Camera;
use meridian_data::Relief;

/// Height of the ground in metres at a world point (bilinear between raster pixels), with the sea
/// flat at zero, the same rule the terrain shader uses.
pub fn ground_m(relief: &Relief, x: f64, y: f64) -> f64 {
    if relief.height_m.is_empty() {
        return 0.0;
    }
    let (w, h) = (relief.width as usize, relief.height as usize);
    // pixel centres sit half a pixel in from the raster edges, as the GPU samples them
    let fx = (x - relief.x0 as f64) / (relief.x1 - relief.x0) as f64 * w as f64 - 0.5;
    let fy = (relief.y1 as f64 - y) / (relief.y1 - relief.y0) as f64 * h as f64 - 0.5;
    let (ix, iy) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - ix, fy - iy);
    let at = |cx: f64, cy: f64| {
        let c = (cx.max(0.0) as usize).min(w - 1);
        let r = (cy.max(0.0) as usize).min(h - 1);
        relief.height_m[r * w + c] as f64
    };
    let top = at(ix, iy) * (1.0 - tx) + at(ix + 1.0, iy) * tx;
    let bottom = at(ix, iy + 1.0) * (1.0 - tx) + at(ix + 1.0, iy + 1.0) * tx;
    (top * (1.0 - ty) + bottom * ty).max(0.0)
}

/// Screen position of a point on the drawn ground.
pub fn to_screen(camera: &Camera, relief: &Relief, x: f64, y: f64) -> (f64, f64) {
    let e = camera.exaggeration();
    if e == 0.0 {
        return camera.world_to_screen(x, y);
    }
    camera.world_to_screen_z(x, y, ground_m(relief, x, y) * e)
}

/// The world point on the drawn ground under a screen pixel.
pub fn to_world(camera: &Camera, relief: &Relief, px: f64, py: f64) -> (f64, f64) {
    let e = camera.exaggeration();
    if e == 0.0 {
        return camera.screen_to_world(px, py);
    }
    camera.screen_to_ground(px, py, |x, y| ground_m(relief, x, y) * e)
}

/// Half-precision float bits for a value (the GPU height texture is 16-bit float, which every
/// GPU can filter). Heights are whole metres well inside the format's range.
pub fn f16_bits(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let mant = b & 0x7f_ffff;
    if v == 0.0 || exp <= 0 {
        return sign; // zero (values this small do not occur)
    }
    if exp >= 31 {
        return sign | 0x7bff; // clamp to the largest finite value
    }
    // round to nearest on the 13 dropped bits
    let mut out = ((exp as u32) << 10) | (mant >> 13);
    if mant & 0x1fff > 0x1000 || (mant & 0x1fff == 0x1000 && out & 1 == 1) {
        out += 1;
    }
    sign | out as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_of_whole_metres() {
        assert_eq!(f16_bits(0.0), 0);
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(-2.0), 0xc000);
        assert_eq!(f16_bits(1000.0), 0x63d0);
        assert_eq!(f16_bits(-4000.0), 0xebd0);
        assert_eq!(f16_bits(8848.0), 0x7052);
    }

    #[test]
    fn ground_is_bilinear_and_flat_at_sea() {
        let r = Relief { width: 2, height: 1, x0: 0, y0: 0, x1: 20, y1: 10, pixels: vec![128; 2], land_rgb: vec![0; 6], height_m: vec![100, 300] };
        assert_eq!(ground_m(&r, 5.0, 5.0), 100.0); // centre of the left pixel
        assert_eq!(ground_m(&r, 10.0, 5.0), 200.0);
        assert_eq!(ground_m(&r, 15.0, 5.0), 300.0);
        let sea = Relief { height_m: vec![-500, -500], ..r };
        assert_eq!(ground_m(&sea, 10.0, 5.0), 0.0);
    }
}
