//! Dev probe: how many ticks does a session advance in one second of 60 fps frames at each speed?
use client::{Session, Speed};
use meridian_core::CountryId;
use std::sync::Arc;

fn main() {
    let map = Arc::new(meridian_data::load_map_file(std::path::Path::new("data/baked/meridian-map.bin")).unwrap());
    // unthrottled: how many ticks per second can the sim thread do at all?
    {
        let w = proving_ground::scenario_on_map(1, map.clone());
        let mut s = Session::new(w, Box::new(proving_ground::schedule(false)), CountryId(0));
        s.set_speed(Speed::Run(5));
        let start = std::time::Instant::now();
        let mut frames = 0u64;
        while start.elapsed().as_secs_f64() < 1.0 {
            s.frame();
            frames += 1;
        }
        eprintln!("unthrottled speed 5: {frames} frame calls, tick {}", s.refresh().tick);
    }
    for n in 1..=5u8 {
        let w = proving_ground::scenario_on_map(1, map.clone());
        let mut s = Session::new(w, Box::new(proving_ground::schedule(false)), CountryId(0));
        s.set_speed(Speed::Run(n));
        let start = std::time::Instant::now();
        let mut frames = 0;
        while start.elapsed().as_secs_f64() < 1.0 {
            let t = std::time::Instant::now();
            s.frame();
            s.refresh();
            frames += 1;
            let spent = t.elapsed();
            if spent < std::time::Duration::from_millis(16) {
                std::thread::sleep(std::time::Duration::from_millis(16) - spent);
            }
        }
        eprintln!("speed {n}: {frames} frames, tick {}", s.refresh().tick);
    }
}

#[allow(dead_code)]
fn unused() {}
