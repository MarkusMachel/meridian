//! The globe view (map rewrite M3): the hexagon world drawn on a sphere.
//!
//! - `camera`: a camera that orbits the Earth, with the arithmetic in 64-bit floats.
//! - `index`: finding hexagons (which one is under a point, which are in view) and building
//!   their triangles. This is the only place that uses the hexagon library (`h3o`); the engine
//!   itself has none.
//! - `render`: the drawing.
//!
//! Positions go to the graphics card relative to the camera. A graphics card works in 32-bit
//! floats, which near the Earth's surface (6,371 km from the centre) are only good to about
//! half a metre; distances from the camera are small numbers and stay exact.

pub mod camera;
pub mod index;
pub mod render;

pub use camera::GlobeCamera;
pub use index::HexIndex;
pub use render::GlobeRenderer;



/// A point or direction in three dimensions (x through 0°N 0°E, y through 0°N 90°E, z through
/// the north pole), the same axes as `meridian_core::sphere::Place`.
pub type V3 = [f64; 3];

pub(crate) fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub(crate) fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub(crate) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn unit(a: V3) -> V3 {
    scale(a, 1.0 / dot(a, a).sqrt())
}

/// The direction from the Earth's centre to a latitude and longitude (radians).
pub fn dir_of(lat: f64, lon: f64) -> V3 {
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}
/// Latitude and longitude (radians) of a direction.
pub fn lat_lon_of(d: V3) -> (f64, f64) {
    let d = unit(d);
    (d[2].clamp(-1.0, 1.0).asin(), d[1].atan2(d[0]))
}
