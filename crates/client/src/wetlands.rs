//! The big marshes, swamps and bogs of the world, for the look of the map only.
//!
//! The baked map has no wetland layer yet (the design notes plan one from a land-cover dataset).
//! Until then the client draws marsh where this hand-written list says: each wetland as an ellipse
//! placed from general knowledge. Positions and sizes are approximate, good to a few tens of
//! kilometres at best, and the list is far from complete. Nothing in the simulation reads it.

/// The raster covers the whole world in latitude and longitude, eight pixels to a degree.
pub const WIDTH: u32 = 2880;
pub const HEIGHT: u32 = 1440;

/// A wetland: name, centre (latitude, longitude in degrees), half length and half width in
/// kilometres, and the direction of its length in degrees anticlockwise from east.
type Wetland = (&'static str, f64, f64, f64, f64, f64);

pub const WETLANDS: [Wetland; 36] = [
    ("Pripet Marshes", 52.0, 27.5, 240.0, 90.0, 0.0),
    ("Mesopotamian Marshes", 31.0, 47.0, 110.0, 65.0, -30.0),
    ("Shatt al-Arab and Faw", 30.2, 48.4, 50.0, 25.0, -45.0),
    ("Everglades", 25.8, -80.9, 80.0, 55.0, 90.0),
    ("Okefenokee", 30.7, -82.3, 30.0, 28.0, 0.0),
    ("Mississippi Delta and Atchafalaya", 29.6, -90.8, 150.0, 45.0, 0.0),
    ("Hudson Bay Lowlands", 53.5, -85.0, 450.0, 140.0, -25.0),
    ("Yukon-Kuskokwim Delta", 61.3, -163.5, 170.0, 120.0, 0.0),
    ("Orinoco Delta", 9.0, -61.5, 100.0, 70.0, 0.0),
    ("Pantanal", -18.0, -57.0, 250.0, 150.0, 90.0),
    ("Ibera", -28.3, -57.3, 110.0, 50.0, 60.0),
    ("Camargue", 43.5, 4.5, 30.0, 20.0, 0.0),
    ("Danube Delta", 45.2, 29.3, 45.0, 40.0, 0.0),
    ("Volga Delta", 46.2, 48.5, 70.0, 50.0, 0.0),
    ("Vasyugan Swamp", 58.0, 78.0, 400.0, 170.0, 0.0),
    ("Ob lowlands", 61.5, 70.0, 350.0, 190.0, 0.0),
    ("Pechora bogs", 65.0, 55.0, 250.0, 140.0, 0.0),
    ("Sanjiang Plain", 47.3, 133.5, 150.0, 90.0, 30.0),
    ("Sudd", 7.5, 30.5, 220.0, 130.0, 80.0),
    ("Okavango Delta", -19.4, 22.8, 90.0, 70.0, -45.0),
    ("Niger Inland Delta", 15.0, -4.3, 180.0, 70.0, 45.0),
    ("Lake Chad marshes", 13.2, 14.3, 80.0, 70.0, 0.0),
    ("Bangweulu Swamps", -11.6, 30.0, 80.0, 60.0, 0.0),
    ("Kafue Flats", -15.7, 27.3, 110.0, 30.0, 0.0),
    ("Barotse Floodplain", -15.5, 23.0, 110.0, 30.0, 90.0),
    ("Congo swamp forest", 0.5, 18.0, 250.0, 150.0, 60.0),
    ("Sundarbans", 22.0, 89.3, 110.0, 45.0, 0.0),
    ("Rann of Kutch", 23.9, 70.0, 150.0, 50.0, 0.0),
    ("Indus Delta", 24.2, 67.6, 80.0, 50.0, 0.0),
    ("Irrawaddy Delta", 16.2, 95.2, 90.0, 70.0, 90.0),
    ("Mekong Delta", 9.8, 105.6, 110.0, 80.0, 0.0),
    ("Tonle Sap floodplain", 12.9, 104.1, 110.0, 40.0, -40.0),
    ("Sumatra peat swamps", 0.5, 102.5, 250.0, 70.0, -45.0),
    ("Borneo peat swamps", -2.3, 113.8, 200.0, 80.0, 0.0),
    ("Southern New Guinea swamps", -7.5, 139.5, 300.0, 110.0, 0.0),
    ("Kakadu and Arnhem wetlands", -12.4, 132.4, 90.0, 50.0, 0.0),
];

const KM_PER_DEGREE: f64 = 111.2;

/// How deep inside one wetland a point is: 0 outside, rising to 1 well inside.
fn inside(w: &Wetland, lat: f64, lon: f64) -> f64 {
    let &(_, clat, clon, a, b, turn) = w;
    // kilometres east and north of the centre (a degree of longitude shrinks with latitude)
    let east = (lon - clon) * KM_PER_DEGREE * clat.to_radians().cos();
    let north = (lat - clat) * KM_PER_DEGREE;
    let (s, c) = turn.to_radians().sin_cos();
    let (along, across) = (east * c + north * s, north * c - east * s);
    let r = ((along / a).powi(2) + (across / b).powi(2)).sqrt();
    // soft edge: full inside 0.8 of the ellipse, gone at 1.1
    ((1.1 - r) / 0.3).clamp(0.0, 1.0)
}

/// How deep inside any wetland a point is (0..1).
pub fn wetness(lat: f64, lon: f64) -> f64 {
    WETLANDS.iter().map(|w| inside(w, lat, lon)).fold(0.0, f64::max)
}

/// The wetlands as a picture: one byte per pixel, rows from 90 north to 90 south, columns from
/// 180 west to 180 east. Only the box around each wetland is visited, so this takes milliseconds.
/// (No wetland in the list is near the 180th meridian, so boxes do not wrap.)
pub fn raster() -> Vec<u8> {
    let mut out = vec![0u8; (WIDTH * HEIGHT) as usize];
    let (per_lat, per_lon) = (HEIGHT as f64 / 180.0, WIDTH as f64 / 360.0);
    for w in &WETLANDS {
        let &(_, clat, clon, a, b, _) = w;
        let reach = a.max(b) * 1.15 / KM_PER_DEGREE;
        let reach_lon = reach / clat.to_radians().cos().max(0.2);
        let row0 = (((90.0 - (clat + reach)) * per_lat).floor().max(0.0)) as u32;
        let row1 = (((90.0 - (clat - reach)) * per_lat).ceil().min(HEIGHT as f64)) as u32;
        let col0 = (((clon - reach_lon + 180.0) * per_lon).floor().max(0.0)) as u32;
        let col1 = (((clon + reach_lon + 180.0) * per_lon).ceil().min(WIDTH as f64)) as u32;
        for row in row0..row1 {
            let lat = 90.0 - (row as f64 + 0.5) / per_lat;
            for col in col0..col1 {
                let lon = -180.0 + (col as f64 + 0.5) / per_lon;
                let v = (inside(w, lat, lon) * 255.0).round() as u8;
                let px = &mut out[(row * WIDTH + col) as usize];
                *px = (*px).max(v);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marshes_are_where_the_list_says_and_nowhere_else() {
        // the middle of each wetland is fully wet
        for &(name, lat, lon, ..) in &WETLANDS {
            assert_eq!(wetness(lat, lon), 1.0, "{name}");
        }
        // the marshes of southern Iraq (the Gulf slice), but not Baghdad or Kuwait City
        assert!(wetness(31.0, 47.0) > 0.9);
        assert_eq!(wetness(33.3, 44.4), 0.0);
        assert_eq!(wetness(29.4, 48.0), 0.0);
        // dry places
        for (lat, lon) in [(48.9, 2.3), (23.0, 10.0), (40.0, -100.0), (-25.0, 135.0)] {
            assert_eq!(wetness(lat, lon), 0.0);
        }
        // a small share of the world, and the picture has the right size
        let r = raster();
        assert_eq!(r.len(), (WIDTH * HEIGHT) as usize);
        let wet = r.iter().filter(|&&v| v > 127).count() as f64 / r.len() as f64;
        assert!(wet > 0.001 && wet < 0.02, "{wet}");
        // the picture agrees with the function it is drawn from (checked on every 37th pixel)
        for i in (0..r.len()).step_by(37) {
            let (row, col) = (i as u32 / WIDTH, i as u32 % WIDTH);
            let (lat, lon) = (90.0 - (row as f64 + 0.5) * 180.0 / HEIGHT as f64, -180.0 + (col as f64 + 0.5) * 360.0 / WIDTH as f64);
            assert_eq!(r[i], (wetness(lat, lon) * 255.0).round() as u8, "{lat} {lon}");
        }
    }
}
