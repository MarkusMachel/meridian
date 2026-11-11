//! Meridian client. `client` opens the window (keys: space pause, 1-5 speed, . step); `client --screenshot out.png` renders one frame
//! without a window (for checking the map). Run from the repository root, or pass `--data DIR`.
//!
//! Options: --data DIR (default data/baked) | --size WxH | --view CX,CY,M_PER_PX (Equal Earth metres)
//!          --mode political|value|terrain | --ticks N | --select PROVINCE | --bench FRAMES
//!          --at LAT,LON,KM (hexagon map: look at a place, the screen KM kilometres wide)
//!          --network (hexagon map: draw the supply network, the links between hexagons by level)
//!
//! A hexagon map (map rewrite) is shown as a globe; an old cell map with its render file as the flat map.

mod app;
mod globe_app;

use client::camera::{Bounds, Camera};
use client::modes::{self, Mode};
use client::offscreen;
use client::render::{Renderer, NO_PROVINCE};
use client::session::Session;
use client::Speed;
use meridian_core::CountryId;
use std::path::PathBuf;
use std::sync::Arc;

struct Args {
    data: PathBuf,
    screenshot: Option<PathBuf>,
    size: (u32, u32),
    view: Option<(f64, f64, f64)>,
    at: Option<(f64, f64, f64)>,
    network: bool,
    mode: Mode,
    ticks: u64,
    select: u32,
    bench: u32,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { data: PathBuf::from("data/baked"), screenshot: None, size: (1600, 900), view: None, at: None, network: false, mode: Mode::Political, ticks: 0, select: NO_PROVINCE, bench: 0 };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--data" => a.data = PathBuf::from(value("--data")?),
            "--screenshot" => a.screenshot = Some(PathBuf::from(value("--screenshot")?)),
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size wants WIDTHxHEIGHT")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--view" => {
                let v: Vec<f64> = value("--view")?.split(',').map(|x| x.parse().map_err(|_| "bad --view number")).collect::<Result<_, _>>()?;
                if v.len() != 3 {
                    return Err("--view wants CX,CY,M_PER_PX".into());
                }
                a.view = Some((v[0], v[1], v[2]));
            }
            "--network" => a.network = true,
            "--at" => {
                let v: Vec<f64> = value("--at")?.split(',').map(|x| x.parse().map_err(|_| "bad --at number")).collect::<Result<_, _>>()?;
                if v.len() != 3 {
                    return Err("--at wants LAT,LON,KM".into());
                }
                a.at = Some((v[0], v[1], v[2]));
            }
            "--mode" => {
                a.mode = match value("--mode")?.as_str() {
                    "political" => Mode::Political,
                    "value" => Mode::Value,
                    "terrain" => Mode::Terrain,
                    other => return Err(format!("unknown mode {other}")),
                }
            }
            "--ticks" => a.ticks = value("--ticks")?.parse().map_err(|_| "bad --ticks")?,
            "--bench" => a.bench = value("--bench")?.parse().map_err(|_| "bad --bench")?,
            "--select" => a.select = value("--select")?.parse().map_err(|_| "bad --select")?,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("client: {e}");
            std::process::exit(2);
        }
    };
    let map = Arc::new(meridian_data::load_map_file(&args.data.join("meridian-map.bin")).unwrap_or_else(|e| panic!("{}", e[0])));
    if map.is_globe() {
        globe_main(&args, map);
        return;
    }
    let render = meridian_data::load_render_file(&args.data.join("meridian-render.bin")).unwrap_or_else(|e| panic!("{e}"));
    let provinces = map.province_count() as usize;
    let cities = client::labels::city_positions(&map, &render);
    let Some(out) = args.screenshot.clone() else {
        let world = proving_ground::scenario_on_map(1, map.clone());
        let session = Session::new(world, Box::new(proving_ground::rules(false)), CountryId(0));
        app::run(app::Inputs { map, render: Arc::new(render), session, view: args.view });
        return;
    };
    let terrain = modes::dominant_terrain(&map);

    let world = proving_ground::scenario_on_map(1, map);
    let mut session = Session::new(world, Box::new(proving_ground::rules(false)), CountryId(0));
    session.set_speed(Speed::Run(5));
    while session.refresh().tick < args.ticks {
        session.frame();
        std::thread::yield_now();
    }
    let snap = session.refresh().clone();

    let gpu = offscreen::headless_gpu().expect("no GPU adapter available");
    eprintln!("GPU: {}", gpu.adapter_name);
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, offscreen::FORMAT, &render, provinces);
    renderer.set_cities(&gpu.device, &cities);
    let colors = match args.mode {
        Mode::Political => modes::political(&snap.controllers),
        Mode::Value => modes::value_ramp(&snap.values),
        Mode::Terrain => modes::terrain(&terrain),
    };
    renderer.land_mix = client::render::land_mix_for(args.mode);
    renderer.set_colors(&gpu.queue, &colors);
    let (x0, y0, x1, y1) = render.bounds();
    let mut camera = Camera::fit(args.size.0 as f64, args.size.1 as f64, Bounds::around(x0, y0, x1, y1));
    if let Some((cx, cy, m)) = args.view {
        camera.cx = cx;
        camera.cy = cy;
        camera.m_per_px = m;
        camera.clamp();
    }
    if args.bench > 0 {
        let _ = offscreen::render_rgba(&gpu, &mut renderer, &camera, args.select, NO_PROVINCE); // warm up
        let mut times: Vec<f64> = (0..args.bench).map(|_| {
            let t = std::time::Instant::now();
            let _ = offscreen::render_rgba(&gpu, &mut renderer, &camera, args.select, NO_PROVINCE);
            t.elapsed().as_secs_f64() * 1000.0
        }).collect();
        times.sort_by(|a, b| a.total_cmp(b));
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        eprintln!("bench {}x{} level {}: mean {:.2} ms, p50 {:.2}, p99 {:.2}, max {:.2}", args.size.0, args.size.1, renderer.level_in_use(), mean, times[times.len() / 2], times[times.len() * 99 / 100], times[times.len() - 1]);
        let through: Vec<f64> = (0..5).map(|_| offscreen::throughput_ms(&gpu, &mut renderer, &camera, args.bench)).collect();
        let best = through.iter().copied().fold(f64::MAX, f64::min);
        eprintln!("throughput (no readback, {} frames x 5 runs): best {:.2} ms/frame, runs {}", args.bench, best, through.iter().map(|t| format!("{t:.2}")).collect::<Vec<_>>().join(" "));
        return;
    }
    let t = std::time::Instant::now();
    let rgba = offscreen::render_rgba(&gpu, &mut renderer, &camera, args.select, NO_PROVINCE);
    eprintln!("frame: {:.1} ms (level {}, tick {}, {})", t.elapsed().as_secs_f64() * 1000.0, renderer.level_in_use(), snap.tick, snap.date);
    offscreen::write_png(&out, args.size.0, args.size.1, &rgba).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("wrote {}", out.display());
}

/// The hexagon world: the globe window, or one frame of it into a picture.
fn globe_main(args: &Args, map: Arc<meridian_core::Map>) {
    use client::globe::{GlobeCamera, GlobeRenderer, HexIndex};
    let world = proving_ground::scenario_on_map(1, map.clone());
    let mut session = Session::new(world, Box::new(proving_ground::rules(false)), CountryId(0));
    // real lines, relief and regional packs are optional: without them the globe shows hexagons only
    let assets = client::globe::assets::load(&args.data);
    for note in &assets.notes {
        eprintln!("{note}");
    }
    let Some(out) = args.screenshot.clone() else {
        globe_app::run(globe_app::Inputs { map, session, assets, view: args.at });
        return;
    };
    session.set_speed(Speed::Run(5));
    while session.refresh().tick < args.ticks {
        session.frame();
        std::thread::yield_now();
    }
    let snap = session.refresh().clone();
    let gpu = offscreen::headless_gpu().expect("no GPU adapter available");
    eprintln!("GPU: {}", gpu.adapter_name);
    let index = HexIndex::new(&map);
    let mut renderer = GlobeRenderer::new(&gpu.device, &gpu.queue, offscreen::FORMAT, &map, &index);
    let colors = match args.mode {
        Mode::Political => modes::political(&snap.controllers),
        Mode::Value => modes::value_ramp(&snap.values),
        Mode::Terrain => modes::terrain(&modes::dominant_terrain(&map)),
    };
    renderer.set_colors(&gpu.queue, &colors);
    renderer.coloring = globe_app::coloring_for(args.mode);
    renderer.show_roads = args.network || (assets.lines.is_none() && assets.packs.is_empty());
    renderer.set_assets(&gpu.device, &gpu.queue, &assets);
    let (w, h) = args.size;
    let camera = match args.at {
        Some((lat, lon, km)) => GlobeCamera::looking_at(w as f64, h as f64, lat, lon, km),
        None => GlobeCamera::whole_globe(w as f64, h as f64, 48.0, 12.0),
    };
    let mut frame = |wait: bool| offscreen::draw_rgba(&gpu, w, h, |enc, view| renderer.render(&gpu.device, &gpu.queue, enc, view, &camera, &map, &index, None, wait));
    let rgba = frame(true);
    if args.bench > 0 {
        let mut times: Vec<f64> = (0..args.bench)
            .map(|_| {
                let t = std::time::Instant::now();
                let _ = frame(false);
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(|a, b| a.total_cmp(b));
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        eprintln!("bench {w}x{h} at {:.0} m/px: mean {mean:.2} ms, p50 {:.2}, max {:.2} (each frame is drawn and read back)", camera.m_per_px(), times[times.len() / 2], times[times.len() - 1]);
        return;
    }
    offscreen::write_png(&out, w, h, &rgba).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("wrote {} (tick {}, {})", out.display(), snap.tick, snap.date);
}
