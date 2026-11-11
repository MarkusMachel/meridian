//! Renders the globe from a baked hexagon map into pictures, without a window, to check the
//! drawing by eye: `cargo run --release -p client --example globe_probe -- <map.bin> <out-dir>`.

use client::globe::{GlobeCamera, GlobeRenderer, HexIndex};
use client::{modes, offscreen};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [map_path, out_dir] = args.as_slice() else {
        eprintln!("usage: globe_probe <map.bin> <out-dir>");
        std::process::exit(2);
    };
    let t = Instant::now();
    let map = meridian_data::load_map_file(std::path::Path::new(map_path)).unwrap_or_else(|e| panic!("{}", e[0]));
    eprintln!("map loaded in {:.1} s", t.elapsed().as_secs_f64());
    let t = Instant::now();
    let index = HexIndex::new(&map);
    eprintln!("hexagon index ({} tiles) in {:.2} s", index.tiles.len(), t.elapsed().as_secs_f64());
    let gpu = offscreen::headless_gpu().expect("no GPU adapter available");
    eprintln!("GPU: {}", gpu.adapter_name);
    let t = Instant::now();
    let mut renderer = GlobeRenderer::new(&gpu.device, &gpu.queue, offscreen::FORMAT, &map, &index);
    eprintln!("renderer (world picture and tables) in {:.2} s", t.elapsed().as_secs_f64());
    let controllers: Vec<u16> = map.province_state.clone();
    renderer.set_colors(&gpu.queue, &modes::political(&controllers));

    let (w, h) = (1600u32, 900u32);
    let views = [
        ("1_globe_europe", 45.0, 15.0, 40_000.0),
        ("2_globe_pacific", -10.0, 179.0, 40_000.0),
        ("3_europe", 50.0, 12.0, 4_000.0),
        ("4_germany", 51.5, 10.0, 1_200.0),
        ("5_hannover", 52.37, 9.73, 300.0),
        ("6_rhine_cologne", 50.94, 6.96, 120.0),
        ("7_arctic_norway", 70.0, 25.0, 900.0),
        ("8_fiji_dateline", -17.5, 179.5, 700.0),
        ("9_gulf", 30.0, 48.0, 1_500.0),
    ];
    for (name, lat, lon, km) in views {
        let camera = GlobeCamera::looking_at(w as f64, h as f64, lat, lon, km);
        let selected = index.cell_at(client::globe::dir_of(camera.lat, camera.lon)).map(|c| c.0);
        let t = Instant::now();
        let rgba = offscreen::draw_rgba(&gpu, w, h, |enc, view| renderer.render(&gpu.device, &gpu.queue, enc, view, &camera, &map, &index, selected, true));
        let first = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let _ = offscreen::draw_rgba(&gpu, w, h, |enc, view| renderer.render(&gpu.device, &gpu.queue, enc, view, &camera, &map, &index, selected, true));
        eprintln!("{name}: {:.0} m/px, {} tiles; first frame {first:.0} ms (makes the tiles), next {:.0} ms", camera.m_per_px(), renderer.drawn.0, t.elapsed().as_secs_f64() * 1000.0);
        let path = std::path::Path::new(out_dir).join(format!("{name}.png"));
        offscreen::write_png(&path, w, h, &rgba).unwrap_or_else(|e| panic!("{e}"));
    }
}
