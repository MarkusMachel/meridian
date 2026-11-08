//! Phase 2 benchmark (guide section 3.5): load a synthetic world-scale map as
//! RON text, postcard, rkyv and a custom flat-array file. Run in release mode:
//!   cargo run --release -p tools --bin format-bench
//!
//! The map is synthetic (the real one is Phase 3) but sized like the plan:
//! 16,000 provinces, 600,000 cells, about 6 neighbors per cell.

use meridian_core::Rng;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct MapData {
    province_area: Vec<u32>,
    cell_province: Vec<u32>,
    cell_terrain: Vec<u8>,
    cell_elevation: Vec<i16>,
    cell_area: Vec<u32>,
    cell_x: Vec<i32>,
    cell_y: Vec<i32>,
    adj_offsets: Vec<u32>,
    adj_targets: Vec<u32>,
}

fn synth(provinces: u32, cells: u32) -> MapData {
    let mut r = Rng::from_seed(12345);
    let mut m = MapData {
        province_area: (0..provinces).map(|_| 5000 + r.below(8000) as u32).collect(),
        cell_province: Vec::new(),
        cell_terrain: Vec::new(),
        cell_elevation: Vec::new(),
        cell_area: Vec::new(),
        cell_x: Vec::new(),
        cell_y: Vec::new(),
        adj_offsets: vec![0],
        adj_targets: Vec::new(),
    };
    for c in 0..cells {
        m.cell_province.push(c * provinces / cells);
        m.cell_terrain.push(r.below(12) as u8);
        m.cell_elevation.push(r.below(5000) as i16 - 200);
        m.cell_area.push(150 + r.below(150) as u32);
        m.cell_x.push(r.below(40_000_000) as i32 - 20_000_000);
        m.cell_y.push(r.below(18_000_000) as i32 - 9_000_000);
        for _ in 0..(4 + r.below(5)) {
            m.adj_targets.push(r.below(cells as u64) as u32);
        }
        m.adj_offsets.push(m.adj_targets.len() as u32);
    }
    m
}

// ---- custom flat-array container: [len u64][raw little-endian elements] per array ----

macro_rules! flat_io {
    ($write:ident, $read:ident, $t:ty) => {
        fn $write(out: &mut Vec<u8>, v: &[$t]) {
            out.extend_from_slice(&(v.len() as u64).to_le_bytes());
            for x in v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        fn $read(b: &[u8], pos: &mut usize) -> Vec<$t> {
            const W: usize = std::mem::size_of::<$t>();
            let n = u64::from_le_bytes(b[*pos..*pos + 8].try_into().unwrap()) as usize;
            *pos += 8;
            let v = b[*pos..*pos + n * W]
                .chunks_exact(W)
                .map(|c| <$t>::from_le_bytes(c.try_into().unwrap()))
                .collect();
            *pos += n * W;
            v
        }
    };
}
flat_io!(w_u32, r_u32, u32);
flat_io!(w_u8, r_u8, u8);
flat_io!(w_i16, r_i16, i16);
flat_io!(w_i32, r_i32, i32);

fn flat_write(m: &MapData) -> Vec<u8> {
    let mut o = Vec::new();
    w_u32(&mut o, &m.province_area);
    w_u32(&mut o, &m.cell_province);
    w_u8(&mut o, &m.cell_terrain);
    w_i16(&mut o, &m.cell_elevation);
    w_u32(&mut o, &m.cell_area);
    w_i32(&mut o, &m.cell_x);
    w_i32(&mut o, &m.cell_y);
    w_u32(&mut o, &m.adj_offsets);
    w_u32(&mut o, &m.adj_targets);
    o
}

fn flat_read(b: &[u8]) -> MapData {
    let mut p = 0;
    MapData {
        province_area: r_u32(b, &mut p),
        cell_province: r_u32(b, &mut p),
        cell_terrain: r_u8(b, &mut p),
        cell_elevation: r_i16(b, &mut p),
        cell_area: r_u32(b, &mut p),
        cell_x: r_i32(b, &mut p),
        cell_y: r_i32(b, &mut p),
        adj_offsets: r_u32(b, &mut p),
        adj_targets: r_u32(b, &mut p),
    }
}

/// Median of `runs` timings in milliseconds.
fn time_ms<T>(runs: usize, mut f: impl FnMut() -> T) -> f64 {
    let mut t: Vec<f64> = (0..runs)
        .map(|_| {
            let s = Instant::now();
            std::hint::black_box(f());
            s.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    t[runs / 2]
}

fn main() {
    let map = synth(16_000, 600_000);
    let runs = 7;
    println!("synthetic map: {} provinces, {} cells, {} adjacency entries\n", map.province_area.len(), map.cell_province.len(), map.adj_targets.len());
    println!("{:<28} {:>10} {:>14}", "format", "size (MB)", "load (ms)");

    // RON text
    let ron_text = ron::to_string(&map).unwrap();
    let ms = time_ms(3, || ron::from_str::<MapData>(&ron_text).unwrap());
    println!("{:<28} {:>10.1} {:>14.1}", "RON text (3 runs)", ron_text.len() as f64 / 1e6, ms);
    assert_eq!(ron::from_str::<MapData>(&ron_text).unwrap(), map);

    // postcard
    let pc = postcard::to_stdvec(&map).unwrap();
    let ms = time_ms(runs, || postcard::from_bytes::<MapData>(&pc).unwrap());
    println!("{:<28} {:>10.1} {:>14.1}", "postcard -> owned", pc.len() as f64 / 1e6, ms);
    assert_eq!(postcard::from_bytes::<MapData>(&pc).unwrap(), map);

    // custom flat arrays
    let flat = flat_write(&map);
    let ms = time_ms(runs, || flat_read(&flat));
    println!("{:<28} {:>10.1} {:>14.1}", "custom flat -> owned", flat.len() as f64 / 1e6, ms);
    assert_eq!(flat_read(&flat), map);

    // rkyv: serialized bytes must be aligned for access.
    let rk = rkyv::to_bytes::<rkyv::rancor::Error>(&map).unwrap();
    let size = rk.len() as f64 / 1e6;
    let ms = time_ms(runs, || rkyv::access::<ArchivedMapData, rkyv::rancor::Error>(&rk).unwrap().cell_province.len());
    println!("{:<28} {:>10.1} {:>14.1}", "rkyv validated access (0-copy)", size, ms);
    let ms = time_ms(runs, || rkyv::from_bytes::<MapData, rkyv::rancor::Error>(&rk).unwrap());
    println!("{:<28} {:>10.1} {:>14.1}", "rkyv validate + -> owned", size, ms);
    assert_eq!(rkyv::from_bytes::<MapData, rkyv::rancor::Error>(&rk).unwrap(), map);
    println!("\nAll formats round-trip to identical data (asserted).");
}
