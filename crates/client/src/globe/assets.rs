//! The optional drawing files of the globe: real lines, shaded relief and regional packs.
//! All are looked for next to the baked map; the globe runs without any of them.

use meridian_data::{GlobeLines, GlobeRelief, PackCover};
use std::path::Path;

#[derive(Default)]
pub struct GlobeAssets {
    /// `meridian-globe-lines.bin`: the world's coasts, borders, rivers, roads and railways.
    pub lines: Option<GlobeLines>,
    /// `meridian-globe-relief.bin`: shaded relief of the world.
    pub relief: Option<GlobeRelief>,
    /// `packs/<name>.lines.bin`: the fine lines of each regional pack, in name order.
    pub packs: Vec<GlobeLines>,
    /// `packs/<name>.cover.bin`: the ground cover of the first pack that has one.
    pub cover: Option<PackCover>,
    /// What was found and what could not be read, for the status line and the terminal.
    pub notes: Vec<String>,
}

/// Load whatever is there. A file that cannot be read is reported in `notes` and left out.
pub fn load(dir: &Path) -> GlobeAssets {
    let mut a = GlobeAssets::default();
    fn get<T>(path: &Path, notes: &mut Vec<String>, read: impl Fn(&Path) -> Result<T, Vec<meridian_data::DataError>>) -> Option<T> {
        if !path.exists() {
            return None;
        }
        match read(path) {
            Ok(v) => {
                notes.push(format!("loaded {}", path.display()));
                Some(v)
            }
            Err(e) => {
                notes.push(format!("could not read {}: {}", path.display(), e[0]));
                None
            }
        }
    }
    a.lines = get(&dir.join("meridian-globe-lines.bin"), &mut a.notes, meridian_data::load_globe_lines_file);
    a.relief = get(&dir.join("meridian-globe-relief.bin"), &mut a.notes, meridian_data::load_globe_relief_file);
    let mut pack_files: Vec<std::path::PathBuf> = std::fs::read_dir(dir.join("packs")).into_iter().flatten().flatten().map(|e| e.path()).collect();
    pack_files.sort();
    for path in pack_files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if name.ends_with(".lines.bin") {
            a.packs.extend(get(&path, &mut a.notes, meridian_data::load_globe_lines_file));
        } else if name.ends_with(".cover.bin") {
            if a.cover.is_none() {
                a.cover = get(&path, &mut a.notes, meridian_data::load_pack_cover_file);
            } else {
                a.notes.push(format!("{}: left out, only one pack's ground cover is drawn for now", path.display()));
            }
        }
    }
    a
}
