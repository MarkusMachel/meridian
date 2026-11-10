//! Draws a small made-up landscape (hills, a mountain ridge, forests of three climates, farmland,
//! steppe, a city, a town, a coast) through the real renderer and writes PNG screenshots. It needs
//! no baked map, so the look of the 3D models can be checked on any machine with a GPU adapter.
//!
//! `cargo run --release -p client --example scene_probe -- OUT_DIR [LAT_DEG] [LON_DEG]`
//! LAT_DEG and LON_DEG (default 50, 5) move the scene around the world, which changes the kinds
//! of tree and the style of the towns and their landmarks.

use client::camera::{Bounds, Camera};
use client::offscreen;
use client::render::{Renderer, NO_PROVINCE};
use meridian_data::render_file::{Labels, LineLayer, Relief, RenderData, RenderLevel};

const W: i32 = 600_000; // metres
const H: i32 = 400_000;
const PX: i32 = 1_000; // one raster pixel in metres

fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374761393).wrapping_add((y as u32).wrapping_mul(668265263));
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 65535.0
}

fn noise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(ix, iy) + (hash(ix + 1, iy) - hash(ix, iy)) * ux;
    let b = hash(ix, iy + 1) + (hash(ix + 1, iy + 1) - hash(ix, iy + 1)) * ux;
    a + (b - a) * uy
}

/// Equal Earth y (metres) of a latitude, close enough for a test scene.
fn y_of_lat(lat_deg: f64) -> i32 {
    let theta = (3f64.sqrt() / 2.0 * lat_deg.to_radians().sin()).asin();
    let t2 = theta * theta;
    let y = theta * (1.340264 + t2 * (-0.081106 + t2 * t2 * (0.000893 + 0.003796 * t2)));
    (y * 6_371_008.8) as i32
}

fn scene(y_base: i32) -> RenderData {
    let (rw, rh) = ((W / PX) as u32, (H / PX) as u32);
    let mut height_m = Vec::with_capacity((rw * rh) as usize);
    let mut land_rgb = Vec::with_capacity((rw * rh * 3) as usize);
    for row in 0..rh as i32 {
        for col in 0..rw as i32 {
            let (x, y) = (col as f32, (rh as i32 - 1 - row) as f32); // km from the bottom-left corner
            let coast = 470.0 + 25.0 * (noise(y / 40.0, 3.0) - 0.5);
            let hills = 250.0 * noise(x / 35.0, y / 35.0) + 90.0 * noise(x / 9.0, y / 9.0);
            let ridge = 1_900.0 * (-((x - 130.0 - 0.15 * y) / 28.0).powi(2)).exp() * (0.6 + 0.6 * noise(x / 14.0, y / 14.0));
            let land = 30.0 + hills + ridge;
            let h = if x < coast { land * ((coast - x) / 25.0).min(1.0) + 2.0 } else { -40.0 - (x - coast) * 30.0 };
            height_m.push(h as i16);
            // land cover in Natural Earth II-like colours
            let patch = noise(x / 30.0 + 11.0, y / 30.0 + 5.0);
            let rgb: [f32; 3] = if y > 270.0 {
                if patch > 0.35 { [0.42, 0.55, 0.40] } else { [0.70, 0.74, 0.58] } // forest / farmland
            } else if y > 130.0 {
                if patch > 0.55 { [0.36, 0.52, 0.33] } else if patch > 0.3 { [0.72, 0.75, 0.58] } else { [0.80, 0.78, 0.62] }
            } else if x < 300.0 {
                [0.31, 0.50, 0.29] // dense forest
            } else {
                if patch > 0.45 { [0.72, 0.77, 0.56] } else { [0.86, 0.80, 0.64] } // dry, with patches of scrub
            };
            land_rgb.extend(rgb.map(|v| (v * 255.0) as u8));
        }
    }
    let quad = |x0: i32, y0: i32, x1: i32, y1: i32| (vec![x0, x1, x1, x0], vec![y0 + y_base, y0 + y_base, y1 + y_base, y1 + y_base]);
    let (vx, vy) = quad(0, 0, 470_000, H);
    let level = RenderLevel {
        tolerance_m: 500,
        vx,
        vy,
        vprov: vec![0; 4],
        tri: vec![0, 1, 2, 0, 2, 3],
        line_x: vec![470_000, 470_000],
        line_y: vec![y_base, y_base + H],
        line_off: vec![0, 2],
        line_kind: vec![2],
    };
    // a city (irregular blob of 16 triangles) and a small town
    let mut urban_vx = Vec::new();
    let mut urban_vy = Vec::new();
    let mut urban_tri = Vec::new();
    for (cx, cy, r) in [(300_000, 200_000, 26_000.0f32), (390_000, 300_000, 6_000.0), (220_000, 90_000, 4_000.0)] {
        let base = urban_vx.len() as u32;
        urban_vx.push(cx);
        urban_vy.push(cy + y_base);
        for k in 0..16 {
            let a = k as f32 / 16.0 * std::f32::consts::TAU;
            let rr = r * (0.65 + 0.6 * hash(k, cx));
            urban_vx.push(cx + (a.cos() * rr) as i32);
            urban_vy.push(cy + y_base + (a.sin() * rr) as i32);
        }
        for k in 0..16u32 {
            urban_tri.extend([base, base + 1 + k, base + 1 + (k + 1) % 16]);
        }
    }
    let line = |pts: &[(i32, i32)], class: u8| LineLayer { x: pts.iter().map(|p| p.0).collect(), y: pts.iter().map(|p| p.1 + y_base).collect(), off: vec![0, pts.len() as u32], class: vec![class] };
    RenderData {
        checksum: 0,
        levels: vec![level],
        cell_seg: vec![0, y_base, 470_000, y_base],
        cell_mx: vec![235_000],
        cell_my: vec![y_base + 200_000],
        cell_ring_off: vec![0, 4],
        cell_ring_x: vec![0, 470_000, 470_000, 0],
        cell_ring_y: vec![y_base, y_base, y_base + H, y_base + H],
        province_mx: vec![235_000],
        province_my: vec![y_base + 200_000],
        relief: Relief { width: rw, height: rh, x0: 0, y0: y_base, x1: W, y1: y_base + H, pixels: vec![128; (rw * rh) as usize], land_rgb, height_m },
        river_x: vec![200_000, 300_000, 470_000],
        river_y: vec![y_base + 380_000, y_base + 215_000, y_base + 150_000],
        river_off: vec![0, 3],
        river_rank: vec![1],
        roads: line(&[(20_000, 200_000), (300_000, 200_000), (390_000, 300_000)], 0),
        rails: line(&[(300_000, 20_000), (300_000, 200_000), (220_000, 90_000)], 0),
        urban_vx,
        urban_vy,
        urban_tri,
        countries: Labels::default(),
        regions: Labels::default(),
    }
}

/// Equal Earth x (metres) of a longitude at a latitude.
fn x_of(lat_deg: f64, lon_deg: f64) -> i32 {
    let theta = (3f64.sqrt() / 2.0 * lat_deg.to_radians().sin()).asin();
    let t2 = theta * theta;
    let d = 1.340264 + 3.0 * -0.081106 * t2 + t2 * t2 * t2 * (7.0 * 0.000893 + 9.0 * 0.003796 * t2);
    (2.0 * 3f64.sqrt() * lon_deg.to_radians() * theta.cos() / (3.0 * d) * 6_371_008.8) as i32
}

/// Move the whole scene east by `dx` metres.
fn shift_x(d: &mut RenderData, dx: i32) {
    let add = |v: &mut Vec<i32>| v.iter_mut().for_each(|x| *x += dx);
    for l in &mut d.levels {
        add(&mut l.vx);
        add(&mut l.line_x);
    }
    d.cell_seg.iter_mut().step_by(2).for_each(|x| *x += dx);
    for v in [&mut d.cell_mx, &mut d.cell_ring_x, &mut d.province_mx, &mut d.river_x, &mut d.roads.x, &mut d.rails.x, &mut d.urban_vx] {
        add(v);
    }
    d.relief.x0 += dx;
    d.relief.x1 += dx;
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| "scene".into()));
    let lat: f64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(50.0);
    std::fs::create_dir_all(&out).expect("create output directory");
    let lon: f64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(5.0);
    let y_base = y_of_lat(lat) - H / 2;
    let x_base = x_of(lat, lon) - W / 2;
    let mut data = scene(y_base);
    shift_x(&mut data, x_base);
    let gpu = offscreen::headless_gpu().expect("no GPU adapter available");
    eprintln!("GPU: {}", gpu.adapter_name);
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, offscreen::FORMAT, &data, 1);
    renderer.set_colors(&gpu.queue, &[[150, 160, 120, 255]]);
    // the city sites: a little off the middle of each town, as cell centres are on the real map
    renderer.set_cities(&gpu.device, &[[x_base + 304_000, y_base + 197_000], [x_base + 392_000, y_base + 301_000], [x_base + 221_000, y_base + 91_000]]);
    let (x0, y0, x1, y1) = data.bounds();
    let (w, h) = (1600u32, 1000u32);
    // (name, centre x, centre y above the scene's bottom edge, metres per pixel)
    let shots = [
        ("city_close", 300_000.0, 200_000.0, 110.0),
        ("city_mid", 300_000.0, 200_000.0, 300.0),
        ("forest_close", 250_000.0, 320_000.0, 110.0),
        ("forest_mid", 250_000.0, 320_000.0, 350.0),
        ("south_close", 250_000.0, 70_000.0, 130.0),
        ("dry_close", 400_000.0, 70_000.0, 130.0),
        ("mountains", 150_000.0, 200_000.0, 400.0),
        ("town", 390_000.0, 300_000.0, 110.0),
        ("far", 300_000.0, 200_000.0, 1_000.0),
    ];
    for (name, cx, cy, m) in shots {
        let mut camera = Camera::fit(w as f64, h as f64, Bounds::around(x0, y0, x1, y1));
        camera.cx = x_base as f64 + cx;
        camera.cy = y_base as f64 + cy;
        camera.m_per_px = m;
        camera.clamp();
        let t = std::time::Instant::now();
        let rgba = offscreen::render_rgba(&gpu, &mut renderer, &camera, NO_PROVINCE, NO_PROVINCE);
        let path = out.join(format!("{name}.png"));
        offscreen::write_png(&path, w, h, &rgba).unwrap_or_else(|e| panic!("{e}"));
        eprintln!("{name}: {:.0} ms -> {}", t.elapsed().as_secs_f64() * 1000.0, path.display());
    }
}
