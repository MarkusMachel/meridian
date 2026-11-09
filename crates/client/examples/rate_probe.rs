//! Dev probe: real-time rate of each speed with a 60 fps frame loop.
use client::{Session, Speed};
use meridian_core::CountryId;

fn main() {
    for n in 1..=5u8 {
        let mut s = Session::new(proving_ground::scenario(1, 60), Box::new(proving_ground::schedule(false)), CountryId(0));
        s.set_speed(Speed::Run(n));
        let start = std::time::Instant::now();
        let mut last = start;
        while start.elapsed().as_secs_f64() < 2.0 {
            std::thread::sleep(std::time::Duration::from_millis(16));
            let now = std::time::Instant::now();
            s.advance(now.duration_since(last).as_secs_f64());
            last = now;
        }
        eprintln!("speed {n}: {:.1} ticks per second", s.refresh().tick as f64 / 2.0);
    }
}
