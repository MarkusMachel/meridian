//! The wgpu map renderer: province fills coloured through a lookup buffer, then border lines
//! (and cell edges when zoomed in). It knows nothing about the simulation; it is given colours
//! and a camera. Drawing a frame is a handful of draw calls over buffers uploaded once.
//!
//! Zoomed out the map is flat, a printed "paper map" drawn straight to the screen. Zoomed in it
//! is 3D: the same flat map is drawn into an overlay texture seen from above, then laid over a
//! terrain mesh raised from the height raster, with water, trees and buildings (`terrain.wgsl`).

use crate::camera::Camera;
use crate::modes::Rgba;
use bytemuck::{Pod, Zeroable};
use meridian_data::{RenderData, RenderLevel};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    center: [i32; 2],
    viewport: [f32; 2],
    inv_scale: f32,
    selected: u32,
    hover: u32,
    relief_on: u32,
    relief_org: [i32; 2],
    relief_span: [f32; 2],
    land_mix: f32,
    detail: f32,
    px_scale: f32,
    relief_k: f32,
    paper: f32,
    /// Building plot size in metres and how visible the buildings are (0..1); both 0 on the paper map.
    plot_m: f32,
    plot_fade: f32,
    _p: f32,
}

/// Uniforms of the 3D pass (see `G3` in terrain.wgsl for what each field holds).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct G3 {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    ov: [f32; 4],
    ov_half: [f32; 4],
    grid: [f32; 4],
    hmap: [f32; 4],
    hpx: [f32; 4],
    misc: [f32; 4],
    center: [i32; 4],
    trees: [i32; 4],
    blds: [i32; 4],
    sizes: [f32; 4],
}

/// Side of the overlay texture the flat map is drawn into for the 3D view.
const OVERLAY_SIZE: u32 = 4096;
/// Terrain mesh: this many cells per side over the ground the screen shows.
const TERRAIN_GRID: u32 = 384;
/// Ground beyond the screen edges drawn in 3D, as a share of the visible ground (raised
/// mountains just past the top edge can rise into view).
const FOOTPRINT_MARGIN: f64 = 0.08;
/// Trees show below this zoom (metres per pixel), roughly this many screen pixels apart.
const TREES_BELOW: f64 = 1_500.0;
const TREE_SPACING_PX: f64 = 8.0;
/// Buildings show below this zoom, in built-up areas.
const BUILDINGS_BELOW: f64 = 1_200.0;
const BUILDING_SPACING_PX: f64 = 11.0;
/// Most model instances drawn per kind in a frame.
const MAX_INSTANCES: i64 = 400_000;
/// Political colours fade a little in the 3D view so the ground shows through.
const LAND_MIX_3D: f32 = 0.45;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Style {
    color: [f32; 4],
    width_px: f32,
    dash_px: f32,
    _pad: [f32; 2],
}

/// One line segment as the GPU sees it: two points and the distance along its line at the start.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LineSeg {
    a: [i32; 4],
    dist: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FillVertex {
    pos: [i32; 2],
    prov: u32,
}

/// One city as the GPU sees it, for its landmark: where the town is, how big the built-up area
/// around it is (radius of a circle of the same area), and a seed for variety.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct CityInst {
    pub pos: [i32; 2],
    pub radius_m: f32,
    pub seed: u32,
}
/// A landmark is six parts of 72 vertices each (see `vs_landmark` in terrain.wgsl).
const LANDMARK_VERTS: u32 = 432;
/// A city site is tied to the built-up area whose nearest triangle is within this distance.
const TOWN_REACH_M: f64 = 30_000.0;

/// Where each city's landmark stands. `cities` are site positions (cell centres, so often beside
/// the town, not in it): each is moved to the centre of the nearest built-up triangle within
/// `TOWN_REACH_M`, and gets the size of the built-up area around that point. Cities with no
/// built-up area nearby are left out. The order of `cities` is kept.
pub fn city_landmarks(urban: &[[i32; 2]], tri: &[u32], cities: &[[i32; 2]]) -> Vec<CityInst> {
    // centre and area of every built-up triangle
    let tris: Vec<(f64, f64, f64)> = tri
        .chunks_exact(3)
        .map(|t| {
            let [a, b, c] = [urban[t[0] as usize], urban[t[1] as usize], urban[t[2] as usize]].map(|p| (p[0] as f64, p[1] as f64));
            let area = ((b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)).abs() / 2.0;
            ((a.0 + b.0 + c.0) / 3.0, (a.1 + b.1 + c.1) / 3.0, area)
        })
        .collect();
    let mut out = Vec::new();
    for (i, city) in cities.iter().enumerate() {
        let (x, y) = (city[0] as f64, city[1] as f64);
        let d2 = |t: &(f64, f64, f64)| (t.0 - x).powi(2) + (t.1 - y).powi(2);
        // nearest triangle (ties to the first, so the result does not depend on float noise)
        let Some(near) = tris.iter().filter(|t| t.2 > 0.0 && d2(t) <= TOWN_REACH_M * TOWN_REACH_M).min_by(|a, b| d2(a).total_cmp(&d2(b))) else {
            continue;
        };
        // the middle of the town: area-weighted centre of the triangles around that one
        let around: Vec<&(f64, f64, f64)> = tris.iter().filter(|t| (t.0 - near.0).powi(2) + (t.1 - near.1).powi(2) <= TOWN_REACH_M * TOWN_REACH_M).collect();
        let area: f64 = around.iter().map(|t| t.2).sum();
        let cx = around.iter().map(|t| t.0 * t.2).sum::<f64>() / area;
        let cy = around.iter().map(|t| t.1 * t.2).sum::<f64>() / area;
        // if that centre fell outside every triangle (a ring-shaped town), stay on the nearest one
        let inside = around.iter().any(|t| (t.0 - cx).powi(2) + (t.1 - cy).powi(2) <= t.2);
        let (px, py) = if inside { (cx, cy) } else { (near.0, near.1) };
        out.push(CityInst { pos: [px.round() as i32, py.round() as i32], radius_m: (area / std::f64::consts::PI).sqrt() as f32, seed: i as u32 });
    }
    out
}

/// Province id meaning "none" for selection and hover.
pub const NO_PROVINCE: u32 = u32::MAX;

/// Line styles in draw order: cell edge, province border (dashed), state border, coast, river.
/// Each is (colour, width in pixels, dash period in pixels or 0 for solid).
const STYLES: [([f32; 4], f32, f32); 5] = [
    ([0.25, 0.18, 0.10, 0.14], 1.0, 0.0),
    ([0.25, 0.18, 0.10, 0.45], 1.0, 8.0),
    ([0.16, 0.11, 0.07, 0.95], 2.5, 0.0),
    ([0.20, 0.30, 0.38, 0.90], 1.2, 0.0),
    ([0.30, 0.48, 0.66, 0.85], 1.1, 0.0),
];
const RIVER_STYLE: usize = 4;
/// Map details drawn as extra styles after the five above: roads by class, the two passes of a
/// railway (dark bed, then light dashes on top for the classic black-and-white line), urban fill.
const DETAIL_STYLES: [([f32; 4], f32, f32); 10] = [
    ([0.58, 0.24, 0.14, 0.90], 1.9, 0.0),
    ([0.55, 0.34, 0.20, 0.80], 1.3, 0.0),
    ([0.42, 0.34, 0.25, 0.60], 0.9, 0.0),
    ([0.13, 0.10, 0.08, 0.80], 1.9, 0.0),
    ([0.90, 0.86, 0.76, 0.75], 0.8, 10.0),
    ([0.22, 0.19, 0.17, 0.55], 0.0, 0.0),
    // close up a road is a road: a dark edge under each class, and a broken centre line (set per frame)
    ([0.0, 0.0, 0.0, 0.0], 0.0, 0.0),
    ([0.0, 0.0, 0.0, 0.0], 0.0, 0.0),
    ([0.0, 0.0, 0.0, 0.0], 0.0, 0.0),
    ([0.0, 0.0, 0.0, 0.0], 0.0, 0.0),
];
/// Close up (the 3D view) roads turn from the red atlas line into asphalt: colour of the surface
/// by class (motorway, main road, lane), its width in pixels at the closest zoom, the edge colour
/// and the centre-line colour.
const ASPHALT: [([f32; 4], f32); 3] = [([0.33, 0.33, 0.35, 1.0], 9.0), ([0.37, 0.37, 0.38, 1.0], 6.0), ([0.45, 0.43, 0.40, 1.0], 3.6)];
const ROAD_EDGE: [f32; 4] = [0.17, 0.16, 0.15, 0.85];
const ROAD_CENTRE: [f32; 4] = [0.93, 0.90, 0.74, 0.95];
const ROAD_EDGE_STYLE: usize = 11;
const ROAD_CENTRE_STYLE: usize = 14;
const ROAD_STYLE: usize = 5;
const RAIL_BED_STYLE: usize = 8;
const RAIL_TIE_STYLE: usize = 9;
const URBAN_STYLE: usize = 10;
/// Zoom (metres per pixel) below which each class shows: major, secondary/regional, other.
const ROADS_BELOW: [f64; 3] = [6_000.0, 2_500.0, 1_200.0];
const RAILS_BELOW: [f64; 3] = [8_000.0, 3_000.0, 1_400.0];
const URBAN_BELOW: f64 = 9_000.0;
/// Rivers of rank 1..=MAJOR show whenever zoomed in below RIVERS_MAJOR_BELOW; to MEDIUM below
/// RIVERS_MEDIUM_BELOW; all of them below RIVERS_ALL_BELOW (metres per pixel).
const RIVER_MAJOR_RANK: u8 = 5;
const RIVER_MEDIUM_RANK: u8 = 7;
const RIVERS_MAJOR_BELOW: f64 = 30_000.0;
const RIVERS_MEDIUM_BELOW: f64 = 9_000.0;
const RIVERS_ALL_BELOW: f64 = 3_500.0;
/// Cell edges are drawn only when zoomed in closer than this many metres per pixel.
const CELL_LINES_BELOW_M_PER_PX: f64 = 900.0;
const SEA: wgpu::Color = wgpu::Color { r: 0.66, g: 0.76, b: 0.80, a: 1.0 };

struct GpuLevel {
    tolerance_m: u32,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Border lines by kind: province border, state border, coast.
    lines: GpuLines,
}

/// Tiles the world is cut into for culling lines: each frame draws only the tiles in view.
const TILES_X: usize = 64;
const TILES_Y: usize = 32;
const TILES: usize = TILES_X * TILES_Y;

#[derive(Clone, Copy)]
struct TileGrid {
    x0: f64,
    y0: f64,
    tile_w: f64,
    tile_h: f64,
}

impl TileGrid {
    fn new((x0, y0, x1, y1): (i32, i32, i32, i32)) -> TileGrid {
        TileGrid { x0: x0 as f64, y0: y0 as f64, tile_w: (x1 - x0).max(1) as f64 / TILES_X as f64, tile_h: (y1 - y0).max(1) as f64 / TILES_Y as f64 }
    }
    fn col(&self, x: f64) -> usize {
        (((x - self.x0) / self.tile_w).floor().max(0.0) as usize).min(TILES_X - 1)
    }
    fn row(&self, y: f64) -> usize {
        (((y - self.y0) / self.tile_h).floor().max(0.0) as usize).min(TILES_Y - 1)
    }
    fn tile_of(&self, s: &LineSeg) -> usize {
        let (mx, my) = ((s.a[0] as f64 + s.a[2] as f64) / 2.0, (s.a[1] as f64 + s.a[3] as f64) / 2.0);
        self.row(my) * TILES_X + self.col(mx)
    }
}

/// A world box (min x, min y, max x, max y) in metres: what a pass shows.
type ViewBox = (f64, f64, f64, f64);

/// Where each class and tile of a sorted line layer starts. Segments are sorted by class and then
/// by tile (row by row), so the segments of one class in one row of tiles are one contiguous run.
struct TileIndex {
    /// `starts[class * TILES + tile]` is the first segment of that class and tile; one extra at the end.
    starts: Vec<u32>,
    classes: usize,
    /// Segments are filed under the tile of their midpoint, so a view must reach out this far.
    reach_m: f64,
    grid: TileGrid,
}

impl TileIndex {
    /// Sort the segments (stable within a tile) and index them.
    fn build(grid: TileGrid, classes: usize, items: Vec<(u8, LineSeg)>) -> (TileIndex, Vec<LineSeg>) {
        let mut keyed: Vec<(usize, usize, LineSeg)> = items.into_iter().enumerate().map(|(i, (c, s))| ((c as usize).min(classes - 1) * TILES + grid.tile_of(&s), i, s)).collect();
        keyed.sort_by_key(|&(key, i, _)| (key, i));
        let mut starts = vec![0u32; classes * TILES + 1];
        for &(key, _, _) in &keyed {
            starts[key + 1] += 1;
        }
        for k in 1..starts.len() {
            starts[k] += starts[k - 1];
        }
        let reach_m = keyed.iter().map(|(_, _, s)| (((s.a[2] - s.a[0]) as f64).powi(2) + ((s.a[3] - s.a[1]) as f64).powi(2)).sqrt() / 2.0).fold(0.0, f64::max);
        (TileIndex { starts, classes, reach_m, grid }, keyed.into_iter().map(|(_, _, s)| s).collect())
    }

    /// Instance ranges of one class that cover a view: one run per row of tiles.
    fn ranges(&self, class: usize, view: ViewBox) -> Vec<std::ops::Range<u32>> {
        let g = &self.grid;
        let r = self.reach_m;
        let (c0, c1) = (g.col(view.0 - r), g.col(view.2 + r));
        let (r0, r1) = (g.row(view.1 - r), g.row(view.3 + r));
        let base = class.min(self.classes - 1) * TILES;
        (r0..=r1).map(|row| self.starts[base + row * TILES_X + c0]..self.starts[base + row * TILES_X + c1 + 1]).filter(|rg| !rg.is_empty()).collect()
    }
}

/// A line layer on the GPU with its tile index.
struct GpuLines {
    segments: wgpu::Buffer,
    index: TileIndex,
}

impl GpuLines {
    fn new(device: &wgpu::Device, label: &str, grid: TileGrid, classes: usize, items: Vec<(u8, LineSeg)>) -> GpuLines {
        let (index, mut segs) = TileIndex::build(grid, classes, items);
        // a zero-size vertex buffer is not allowed, so an empty layer gets one unused segment
        if segs.is_empty() {
            segs.push(LineSeg { a: [0; 4], dist: 0.0 });
        }
        GpuLines { segments: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytemuck::cast_slice(&segs), usage: wgpu::BufferUsages::VERTEX }), index }
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, class: usize, view: ViewBox) {
        pass.set_vertex_buffer(0, self.segments.slice(..));
        for rg in self.index.ranges(class, view) {
            pass.draw(0..6, rg);
        }
    }
}

/// Segments of polylines with their class.
fn classed_lines(x: &[i32], y: &[i32], off: &[u32], class: impl Fn(usize) -> u8) -> Vec<(u8, LineSeg)> {
    let mut out = Vec::new();
    let mut segs = Vec::new();
    for i in 0..off.len().saturating_sub(1) {
        let (a, b) = (off[i] as usize, off[i + 1] as usize);
        segs.clear();
        push_line(&mut segs, &x[a..b], &y[a..b]);
        out.extend(segs.iter().map(|&s| (class(i), s)));
    }
    out
}

/// How many classes of a layer show at this zoom.
fn tier(m_per_px: f64, below: [f64; 3]) -> usize {
    below.iter().filter(|&&b| m_per_px < b).count()
}

pub struct Renderer {
    globals: wgpu::Buffer,
    colors: wgpu::Buffer,
    province_count: usize,
    bind0: wgpu::BindGroup,
    styles: Vec<wgpu::BindGroup>,
    style_buffers: Vec<wgpu::Buffer>,
    fill_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    levels: Vec<GpuLevel>,
    cells: GpuLines,
    rivers: GpuLines,
    river_tier: usize,
    roads: GpuLines,
    rails: GpuLines,
    road_tier: usize,
    rail_tier: usize,
    /// Roads are drawn as asphalt with a dark edge (the 3D view); and with a centre line.
    road_3d: bool,
    road_centre: bool,
    urban_vertices: wgpu::Buffer,
    urban_indices: wgpu::Buffer,
    urban_count: u32,
    urban_pipeline: wgpu::RenderPipeline,
    /// Built-up triangles kept on the CPU, to place city landmarks (`set_cities`).
    urban_cpu: (Vec<[i32; 2]>, Vec<u32>),
    cities: Option<(wgpu::Buffer, u32)>,
    plaza_pipeline: wgpu::RenderPipeline,
    hole_pipeline: wgpu::RenderPipeline,
    landmark_pipeline: wgpu::RenderPipeline,
    draw_plazas: bool,
    draw_urban: bool,
    relief_org: [i32; 2],
    relief_span: [f32; 2],
    relief_px: [f32; 2],
    level_in_use: usize,
    sea_pipeline: wgpu::RenderPipeline,
    mask_pipeline: wgpu::RenderPipeline,
    terrain_pipeline: wgpu::RenderPipeline,
    tree_pipeline: wgpu::RenderPipeline,
    building_pipeline: wgpu::RenderPipeline,
    g3: wgpu::Buffer,
    bind3: wgpu::BindGroup,
    overlay_view: wgpu::TextureView,
    mask_view: wgpu::TextureView,
    overlay_size: u32,
    depth: Option<(u32, u32, wgpu::TextureView)>,
    draw_cells: bool,
    /// Turn cell edges off entirely (a setting).
    pub show_cells: bool,
    /// Shaded relief under the province colours (a setting).
    pub show_relief: bool,
    /// Roads, railways and built-up areas (a setting).
    pub show_details: bool,
    /// How much of the province colour covers the real land colours (0 = land only, 1 = flat colour).
    pub land_mix: f32,
}

/// Province colour share for map modes: political lets the land show through, terrain shows the land
/// itself, the debug value mode stays flat so its numbers read plainly.
pub const LAND_MIX_POLITICAL: f32 = 0.65;
pub const LAND_MIX_TERRAIN: f32 = 0.0;
pub const LAND_MIX_FLAT: f32 = 1.0; // 1 or more: plain province colours, no land texture

pub fn land_mix_for(mode: crate::modes::Mode) -> f32 {
    match mode {
        crate::modes::Mode::Political => LAND_MIX_POLITICAL,
        crate::modes::Mode::Terrain => LAND_MIX_TERRAIN,
        crate::modes::Mode::Value => LAND_MIX_FLAT,
    }
}

/// Push the segments of one polyline, each with the distance along the line at its start.
fn push_line(segs: &mut Vec<LineSeg>, xs: &[i32], ys: &[i32]) {
    let mut dist = 0.0f64;
    for k in 0..xs.len().saturating_sub(1) {
        segs.push(LineSeg { a: [xs[k], ys[k], xs[k + 1], ys[k + 1]], dist: dist as f32 });
        dist += (((xs[k + 1] - xs[k]) as f64).powi(2) + ((ys[k + 1] - ys[k]) as f64).powi(2)).sqrt();
    }
}

fn segments_of(level: &RenderLevel) -> Vec<(u8, LineSeg)> {
    classed_lines(&level.line_x, &level.line_y, &level.line_off, |i| level.line_kind[i])
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, data: &RenderData, province_count: usize) -> Renderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("map"), source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()) });
        let globals = device.create_buffer(&wgpu::BufferDescriptor { label: Some("globals"), size: std::mem::size_of::<Globals>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let colors = device.create_buffer(&wgpu::BufferDescriptor { label: Some("colors"), size: (province_count.max(1) * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bgl0"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            ],
        });
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bgl1"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
        });
        let rf = &data.relief;
        let relief_tex = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("relief"),
                size: wgpu::Extent3d { width: rf.width, height: rf.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &rf.pixels,
        );
        let relief_view = relief_tex.create_view(&wgpu::TextureViewDescriptor::default());
        // land colours: RGB in the file, RGBA on the GPU (wgpu has no 3-channel 8-bit format)
        let land_rgba: Vec<u8> = rf.land_rgb.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect();
        let land_tex = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("land colours"),
                size: wgpu::Extent3d { width: rf.width, height: rf.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &land_rgba,
        );
        let land_view = land_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let relief_sampler = device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        // heights as 16-bit floats: every GPU can filter that format, so the mesh and the lighting are smooth
        let height_bits: Vec<u16> = rf.height_m.iter().map(|&h| crate::heights::f16_bits(h as f32)).collect();
        let height_tex = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("heights"),
                size: wgpu::Extent3d { width: rf.width, height: rf.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&height_bits),
        );
        let height_view = height_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind0"),
            layout: &bgl0,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: colors.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&relief_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&relief_sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&land_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&height_view) },
            ],
        });
        let style_buffers: Vec<wgpu::Buffer> = STYLES
            .iter()
            .chain(DETAIL_STYLES.iter())
            .map(|(color, width, dash)| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("style"), contents: bytemuck::bytes_of(&Style { color: *color, width_px: *width, dash_px: *dash, _pad: [0.0; 2] }), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST }))
            .collect();
        let styles = style_buffers
            .iter()
            .map(|buf| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("style"), layout: &bgl1, entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }] }))
            .collect();

        let layout_fill = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fill"), bind_group_layouts: &[Some(&bgl0)], immediate_size: 0 });
        let layout_line = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("line"), bind_group_layouts: &[Some(&bgl0), Some(&bgl1)], immediate_size: 0 });
        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
        let target = [Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })];
        let fill_attrs = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint32x2, offset: 0, shader_location: 0 }, wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 8, shader_location: 1 }];
        let line_attrs = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint32x2, offset: 0, shader_location: 0 }, wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint32x2, offset: 8, shader_location: 1 }, wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 16, shader_location: 2 }];
        let fill_buffers = [Some(wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &fill_attrs })];
        let line_buffers = [Some(wgpu::VertexBufferLayout { array_stride: 20, step_mode: wgpu::VertexStepMode::Instance, attributes: &line_attrs })];
        let fill_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fill"),
            layout: Some(&layout_fill),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_fill"), compilation_options: Default::default(), buffers: &fill_buffers },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_fill"), compilation_options: Default::default(), targets: &target }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let urban_attrs = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint32x2, offset: 0, shader_location: 0 }];
        let urban_buffers = [Some(wgpu::VertexBufferLayout { array_stride: 8, step_mode: wgpu::VertexStepMode::Vertex, attributes: &urban_attrs })];
        let urban_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("urban"),
            layout: Some(&layout_line),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_urban"), compilation_options: Default::default(), buffers: &urban_buffers },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_urban"), compilation_options: Default::default(), targets: &target }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("line"),
            layout: Some(&layout_line),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_line"), compilation_options: Default::default(), buffers: &line_buffers },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_line"), compilation_options: Default::default(), targets: &target }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sea_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sea"),
            layout: Some(&layout_fill),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_sea"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_sea"), compilation_options: Default::default(), targets: &target }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let mask_target = [Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let mask_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("urban mask"),
            layout: Some(&layout_fill),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_solid"), compilation_options: Default::default(), buffers: &urban_buffers },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_mask"), compilation_options: Default::default(), targets: &mask_target }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // city landmarks: one instance per city (position, town radius, seed)
        let city_attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint32x2, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 8, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 12, shader_location: 2 },
        ];
        let city_buffers = [Some(wgpu::VertexBufferLayout { array_stride: 16, step_mode: wgpu::VertexStepMode::Instance, attributes: &city_attrs })];
        let plaza_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("plaza"),
            layout: Some(&layout_fill),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_plaza"), compilation_options: Default::default(), buffers: &city_buffers },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_plaza"), compilation_options: Default::default(), targets: &target }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let hole_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("plaza hole"),
            layout: Some(&layout_fill),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_plaza"), compilation_options: Default::default(), buffers: &city_buffers },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_hole"), compilation_options: Default::default(), targets: &mask_target }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- 3D: overlay and mask targets, the height texture, the terrain and model pipelines ---
        let overlay_size = OVERLAY_SIZE.min(device.limits().max_texture_dimension_2d);
        let target_tex = |label: &str, format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: overlay_size, height: overlay_size, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let overlay_view = target_tex("overlay", format);
        let mask_view = target_tex("urban mask", wgpu::TextureFormat::R8Unorm);
        let g3 = device.create_buffer(&wgpu::BufferDescriptor { label: Some("3d globals"), size: std::mem::size_of::<G3>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry { binding, visibility: both, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
        let bgl3 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bgl3"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: both, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: both, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let bind3 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind3"),
            layout: &bgl3,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: g3.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&overlay_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&mask_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&height_view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&land_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&relief_sampler) },
            ],
        });
        let terrain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("terrain"), source: wgpu::ShaderSource::Wgsl(include_str!("terrain.wgsl").into()) });
        let layout3 = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("3d"), bind_group_layouts: &[Some(&bgl3)], immediate_size: 0 });
        let opaque = [Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let pipeline3 = |label: &str, vs: &str, fs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout3),
                vertex: wgpu::VertexState { module: &terrain_shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState { module: &terrain_shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &opaque }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let terrain_pipeline = pipeline3("terrain", "vs_terrain", "fs_terrain");
        let tree_pipeline = pipeline3("trees", "vs_tree", "fs_model");
        let building_pipeline = pipeline3("buildings", "vs_building", "fs_model");
        let landmark_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("landmarks"),
            layout: Some(&layout3),
            vertex: wgpu::VertexState { module: &terrain_shader, entry_point: Some("vs_landmark"), compilation_options: Default::default(), buffers: &city_buffers },
            fragment: Some(wgpu::FragmentState { module: &terrain_shader, entry_point: Some("fs_model"), compilation_options: Default::default(), targets: &opaque }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let grid = TileGrid::new(data.bounds());
        let levels = data
            .levels
            .iter()
            .map(|l| {
                let verts: Vec<FillVertex> = (0..l.vx.len()).map(|i| FillVertex { pos: [l.vx[i], l.vy[i]], prov: l.vprov[i] }).collect();
                GpuLevel {
                    tolerance_m: l.tolerance_m,
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("verts"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("indices"), contents: bytemuck::cast_slice(&l.tri), usage: wgpu::BufferUsages::INDEX }),
                    index_count: l.tri.len() as u32,
                    lines: GpuLines::new(device, "borders", grid, 3, segments_of(l)),
                }
            })
            .collect();
        let cell_segs: Vec<(u8, LineSeg)> = data.cell_seg.chunks_exact(4).map(|c| (0, LineSeg { a: [c[0], c[1], c[2], c[3]], dist: 0.0 })).collect();
        let cells = GpuLines::new(device, "cell edges", grid, 1, cell_segs);
        let river_class = |i: usize| {
            let r = data.river_rank[i];
            if r <= RIVER_MAJOR_RANK { 0 } else if r <= RIVER_MEDIUM_RANK { 1 } else { 2 }
        };
        let rivers = GpuLines::new(device, "rivers", grid, 3, classed_lines(&data.river_x, &data.river_y, &data.river_off, river_class));
        let roads = GpuLines::new(device, "roads", grid, 3, classed_lines(&data.roads.x, &data.roads.y, &data.roads.off, |i| data.roads.class[i]));
        let rails = GpuLines::new(device, "rails", grid, 3, classed_lines(&data.rails.x, &data.rails.y, &data.rails.off, |i| data.rails.class[i]));
        let urban_verts: Vec<[i32; 2]> = if data.urban_vx.is_empty() { vec![[0, 0]] } else { (0..data.urban_vx.len()).map(|i| [data.urban_vx[i], data.urban_vy[i]]).collect() };
        let urban_idx: Vec<u32> = if data.urban_tri.is_empty() { vec![0, 0, 0] } else { data.urban_tri.clone() };
        let urban_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("urban verts"), contents: bytemuck::cast_slice(&urban_verts), usage: wgpu::BufferUsages::VERTEX });
        let urban_indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("urban indices"), contents: bytemuck::cast_slice(&urban_idx), usage: wgpu::BufferUsages::INDEX });

        Renderer {
            globals,
            colors,
            province_count,
            bind0,
            styles,
            style_buffers,
            fill_pipeline,
            line_pipeline,
            levels,
            cells,
            rivers,
            river_tier: 0,
            roads,
            rails,
            road_tier: 0,
            rail_tier: 0,
            road_3d: false,
            road_centre: false,
            urban_vertices,
            urban_indices,
            urban_count: data.urban_tri.len() as u32,
            urban_pipeline,
            urban_cpu: (urban_verts.clone(), urban_idx.clone()),
            cities: None,
            plaza_pipeline,
            hole_pipeline,
            landmark_pipeline,
            draw_plazas: false,
            draw_urban: false,
            relief_org: [rf.x0, rf.y1],
            relief_span: [(rf.x1 - rf.x0) as f32, (rf.y1 - rf.y0) as f32],
            relief_px: [(rf.x1 - rf.x0) as f32 / rf.width as f32, (rf.y1 - rf.y0) as f32 / rf.height as f32],
            level_in_use: 0,
            sea_pipeline,
            mask_pipeline,
            terrain_pipeline,
            tree_pipeline,
            building_pipeline,
            g3,
            bind3,
            overlay_view,
            mask_view,
            overlay_size,
            depth: None,
            draw_cells: false,
            show_cells: true,
            show_relief: true,
            show_details: true,
            land_mix: LAND_MIX_POLITICAL,
        }
    }

    /// One colour per province (RGBA8).
    /// Tell the renderer where the cities are (site positions in world metres), so each town that
    /// is big enough on screen gets a landmark in the style of its part of the world.
    pub fn set_cities(&mut self, device: &wgpu::Device, cities: &[[i32; 2]]) {
        let inst = city_landmarks(&self.urban_cpu.0, &self.urban_cpu.1, cities);
        self.cities = if inst.is_empty() {
            None
        } else {
            Some((device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("cities"), contents: bytemuck::cast_slice(&inst), usage: wgpu::BufferUsages::VERTEX }), inst.len() as u32))
        };
    }

    pub fn set_colors(&self, queue: &wgpu::Queue, colors: &[Rgba]) {
        assert_eq!(colors.len(), self.province_count, "one colour per province");
        queue.write_buffer(&self.colors, 0, bytemuck::cast_slice(colors));
    }

    /// Pick the detail level and line layers for this zoom, and set the zoom-dependent line styles.
    fn prepare_layers(&mut self, queue: &wgpu::Queue, camera: &Camera) {
        let tolerances: Vec<u32> = self.levels.iter().map(|l| l.tolerance_m).collect();
        self.level_in_use = camera.level_for(&tolerances);
        self.draw_cells = self.show_cells && camera.m_per_px < CELL_LINES_BELOW_M_PER_PX;
        self.river_tier = tier(camera.m_per_px, [RIVERS_MAJOR_BELOW, RIVERS_MEDIUM_BELOW, RIVERS_ALL_BELOW]);
        let details = if self.show_details { 1 } else { 0 };
        self.road_tier = tier(camera.m_per_px, ROADS_BELOW) * details;
        self.rail_tier = tier(camera.m_per_px, RAILS_BELOW) * details;
        self.draw_urban = self.show_details && camera.m_per_px < URBAN_BELOW;
        self.draw_plazas = false;   // set by the 3D path when buildings show
        // Roads and railways get wider as you zoom in (up to 4x their base width below 300 m/px),
        // so close up they are features you can see and click, not hairlines.
        let widen = (1_200.0 / camera.m_per_px).clamp(1.0, 4.0) as f32;
        for (k, (color, width, dash)) in DETAIL_STYLES.iter().enumerate().take(5) {
            let st = Style { color: *color, width_px: width * widen, dash_px: dash * widen, _pad: [0.0; 2] };
            queue.write_buffer(&self.style_buffers[ROAD_STYLE + k], 0, bytemuck::bytes_of(&st));
        }
        // Roads: the red line of the paper map becomes asphalt as the view turns 3D, with a dark
        // edge under it and, on the two bigger classes, a broken centre line.
        let t = camera.depth_t() as f32;
        self.road_3d = t > 0.02;
        let mix = |a: f32, b: f32| a + (b - a) * t;
        for class in 0..3 {
            let (red, base_w, _) = DETAIL_STYLES[class];
            let (grey, close_w) = ASPHALT[class];
            // the asphalt width is reached at the closest zoom; farther out it follows the old width
            let width = mix(base_w * widen, (close_w * widen / 4.0).max(base_w * widen));
            let color = [mix(red[0], grey[0]), mix(red[1], grey[1]), mix(red[2], grey[2]), mix(red[3], grey[3])];
            queue.write_buffer(&self.style_buffers[ROAD_STYLE + class], 0, bytemuck::bytes_of(&Style { color, width_px: width, dash_px: 0.0, _pad: [0.0; 2] }));
            let mut edge = ROAD_EDGE;
            edge[3] *= t;
            queue.write_buffer(&self.style_buffers[ROAD_EDGE_STYLE + class], 0, bytemuck::bytes_of(&Style { color: edge, width_px: width + 1.0 + 1.6 * widen / 4.0, dash_px: 0.0, _pad: [0.0; 2] }));
        }
        let mut centre = ROAD_CENTRE;
        centre[3] *= ((t - 0.5) * 2.0).clamp(0.0, 1.0) * ((widen - 2.0) / 1.0).clamp(0.0, 1.0);
        self.road_centre = centre[3] > 0.02;
        queue.write_buffer(&self.style_buffers[ROAD_CENTRE_STYLE], 0, bytemuck::bytes_of(&Style { color: centre, width_px: 0.28 * widen, dash_px: 4.5 * widen, _pad: [0.0; 2] }));
        // Province borders fade out when the whole world is in view, where 15,000 of them are only noise.
        let fade = (1.0 - camera.m_per_px / 20_000.0).clamp(0.0, 1.0) as f32;
        let mut s = STYLES[1];
        s.0[3] = 0.12 + (s.0[3] - 0.12) * fade;
        queue.write_buffer(&self.style_buffers[1], 0, bytemuck::bytes_of(&Style { color: s.0, width_px: s.1, dash_px: s.2, _pad: [0.0; 2] }));
    }

    /// Uniforms of the flat map drawn around (cx, cy) at `m_per_px` into a `viewport` of pixels.
    #[allow(clippy::too_many_arguments)]
    fn globals_2d(&self, camera: &Camera, center: [i32; 2], viewport: [f32; 2], m_per_px: f64, selected: u32, hover: u32, plot: (f32, f32)) -> Globals {
        let t = camera.depth_t() as f32;
        let land_mix = if self.land_mix >= 1.0 { self.land_mix } else { self.land_mix + (self.land_mix.min(LAND_MIX_3D) - self.land_mix) * t };
        Globals {
            center,
            viewport,
            inv_scale: (1.0 / m_per_px) as f32,
            selected,
            hover,
            relief_on: u32::from(self.show_relief),
            relief_org: self.relief_org,
            relief_span: self.relief_span,
            land_mix,
            detail: ((3_000.0 - camera.m_per_px) / 2_000.0).clamp(0.0, 1.0) as f32,
            px_scale: (camera.m_per_px / m_per_px) as f32,
            relief_k: 1.0 - t,
            paper: 1.0 - t,
            plot_m: plot.0,
            plot_fade: plot.1,
            _p: 0.0,
        }
    }

    /// Draw one frame of the map into `target` (sized like the camera's viewport). `time` (seconds)
    /// moves the waves. Anything drawn after (the UI) should load, not clear, the target.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, camera: &Camera, selected: u32, hover: u32, time: f32) {
        self.prepare_layers(queue, camera);
        let t = camera.depth_t();
        let center = [camera.cx.round() as i32, camera.cy.round() as i32];
        let color_ops = wgpu::Operations { load: wgpu::LoadOp::Clear(SEA), store: wgpu::StoreOp::Store };
        if t <= 0.0 {
            // the paper map: flat, straight to the screen
            let g = self.globals_2d(camera, center, [camera.width_px as f32, camera.height_px as f32], camera.m_per_px, selected, hover, (0.0, 0.0));
            queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("paper map"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: color_ops })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind0, &[]);
            pass.set_pipeline(&self.sea_pipeline);
            pass.draw(0..3, 0..1);
            self.draw_2d(&mut pass, camera.footprint());
            return;
        }

        // 1. the flat map, seen from above, over the ground the screen shows
        let (fx0, fy0, fx1, fy1) = camera.footprint();
        let (mx, my) = ((fx1 - fx0) * FOOTPRINT_MARGIN, (fy1 - fy0) * FOOTPRINT_MARGIN);
        let (gx0, gy0, gx1, gy1) = (fx0 - mx, fy0 - my, fx1 + mx, fy1 + my);
        let (gw, gh) = (gx1 - gx0, gy1 - gy0);
        let size = self.overlay_size as f64;
        let om = (gw.max(gh) / size).max(camera.m_per_px * 0.5);
        let (ow, oh) = ((gw / om).ceil().min(size).max(1.0), (gh / om).ceil().min(size).max(1.0));
        let ocenter = [((gx0 + gx1) / 2.0).round() as i32, ((gy0 + gy1) / 2.0).round() as i32];
        // the building plots: the flat map draws the streets between them, the 3D pass the houses
        let buildings = self.show_details && self.draw_urban && camera.m_per_px < BUILDINGS_BELOW && self.urban_count > 0;
        let bld = instances(gx0, gy0, gx1, gy1, camera.m_per_px * BUILDING_SPACING_PX, 100);
        let bld_fade = if buildings { ((BUILDINGS_BELOW - camera.m_per_px) / (BUILDINGS_BELOW * 0.3)).clamp(0.0, 1.0) } else { 0.0 };
        let g = self.globals_2d(camera, ocenter, [ow as f32, oh as f32], om, selected, hover, (bld.spacing as f32, bld_fade as f32));
        self.draw_plazas = bld_fade > 0.0 && self.cities.is_some();
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.overlay_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(0.0, 0.0, ow as f32, oh as f32, 0.0, 1.0);
            self.draw_2d(&mut pass, (gx0, gy0, gx1, gy1));
        }
        // 2. where the towns are, for the buildings
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("urban mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.mask_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if self.draw_urban && self.urban_count > 0 {
                pass.set_viewport(0.0, 0.0, ow as f32, oh as f32, 0.0, 1.0);
                pass.set_bind_group(0, &self.bind0, &[]);
                pass.set_pipeline(&self.mask_pipeline);
                pass.set_vertex_buffer(0, self.urban_vertices.slice(..));
                pass.set_index_buffer(self.urban_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.urban_count, 0, 0..1);
                // no houses on the landmark squares
                if let (true, Some((buf, n))) = (self.draw_plazas, &self.cities) {
                    pass.set_pipeline(&self.hole_pipeline);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..6, 0..*n);
                }
            }
        }

        // 3. terrain, water and models with a depth buffer
        let (w, h) = (camera.width_px as u32, camera.height_px as u32);
        if self.depth.as_ref().is_none_or(|d| d.0 != w || d.1 != h) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.depth = Some((w, h, tex.create_view(&Default::default())));
        }
        let rel = |x: f64, y: f64| [(x - center[0] as f64) as f32, (y - center[1] as f64) as f32];
        let vp = camera.view_proj();
        let eye = camera.eye();
        let tree = instances(gx0, gy0, gx1, gy1, camera.m_per_px * TREE_SPACING_PX, 250);
        let tree_fade = ((TREES_BELOW - camera.m_per_px) / (TREES_BELOW * 0.3)).clamp(0.0, 1.0);
        let show_trees = self.show_details && tree_fade > 0.0;
        let show_blds = buildings && bld_fade > 0.0;
        let ov = rel(ocenter[0] as f64, ocenter[1] as f64);
        let hm = rel(self.relief_org[0] as f64, self.relief_org[1] as f64);
        let g3 = G3 {
            view_proj: vp.map(|c| c.map(|v| v as f32)),
            eye: [eye[0] as f32, eye[1] as f32, eye[2] as f32, time],
            ov: [ov[0], ov[1], (1.0 / om) as f32, 0.0],
            ov_half: [(ow / 2.0) as f32, (oh / 2.0) as f32, 1.0 / size as f32, t as f32],
            grid: { let c = rel(gx0, gy0); [c[0], c[1], (gw / TERRAIN_GRID as f64) as f32, (gh / TERRAIN_GRID as f64) as f32] },
            hmap: [hm[0], hm[1], self.relief_span[0], self.relief_span[1]],
            hpx: [self.relief_px[0], self.relief_px[1], 0.0, 0.0],
            misc: [camera.exaggeration() as f32, camera.m_per_px as f32, TERRAIN_GRID as f32, t as f32],
            center: [center[0], center[1], 0, 0],
            trees: tree.cells,
            blds: bld.cells,
            sizes: [(tree.spacing * 1.25 * tree_fade) as f32, (bld.spacing * bld_fade) as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.g3, 0, bytemuck::bytes_of(&g3));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("terrain"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: color_ops })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth.as_ref().expect("made above").2,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.bind3, &[]);
        pass.set_pipeline(&self.terrain_pipeline);
        pass.draw(0..TERRAIN_GRID * TERRAIN_GRID * 6, 0..1);
        if show_trees {
            pass.set_pipeline(&self.tree_pipeline);
            pass.draw(0..36, 0..tree.count);
        }
        if show_blds {
            pass.set_pipeline(&self.building_pipeline);
            pass.draw(0..42, 0..bld.count);
            if let Some((buf, n)) = &self.cities {
                pass.set_pipeline(&self.landmark_pipeline);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..LANDMARK_VERTS, 0..*n);
            }
        }
    }

    pub fn level_in_use(&self) -> usize {
        self.level_in_use
    }

    pub const fn sea_color() -> wgpu::Color {
        SEA
    }

    /// The flat map: fills, built-up areas, rivers, roads, railways, cell edges, borders. Lines
    /// are drawn only in the tiles that cover `view`.
    fn draw_2d(&self, pass: &mut wgpu::RenderPass<'_>, view: ViewBox) {
        let level = &self.levels[self.level_in_use];
        pass.set_bind_group(0, &self.bind0, &[]);
        pass.set_pipeline(&self.fill_pipeline);
        pass.set_vertex_buffer(0, level.vertices.slice(..));
        pass.set_index_buffer(level.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..level.index_count, 0, 0..1);

        if self.draw_urban && self.urban_count > 0 {
            pass.set_pipeline(&self.urban_pipeline);
            pass.set_bind_group(1, &self.styles[URBAN_STYLE], &[]);
            pass.set_vertex_buffer(0, self.urban_vertices.slice(..));
            pass.set_index_buffer(self.urban_indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.urban_count, 0, 0..1);
            if let (true, Some((buf, n))) = (self.draw_plazas, &self.cities) {
                pass.set_pipeline(&self.plaza_pipeline);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..*n);
            }
        }
        pass.set_pipeline(&self.line_pipeline);
        pass.set_bind_group(1, &self.styles[RIVER_STYLE], &[]);
        for class in 0..self.river_tier {
            self.rivers.draw(pass, class, view);
        }
        // roads: the less important classes first, so major highways sit on top. Close up, all the
        // dark edges go down first, so where roads meet their surfaces join without a line between.
        if self.road_3d {
            for class in (0..self.road_tier).rev() {
                pass.set_bind_group(1, &self.styles[ROAD_EDGE_STYLE + class], &[]);
                self.roads.draw(pass, class, view);
            }
        }
        for class in (0..self.road_tier).rev() {
            pass.set_bind_group(1, &self.styles[ROAD_STYLE + class], &[]);
            self.roads.draw(pass, class, view);
        }
        if self.road_centre {
            pass.set_bind_group(1, &self.styles[ROAD_CENTRE_STYLE], &[]);
            for class in 0..self.road_tier.min(2) {
                self.roads.draw(pass, class, view);
            }
        }
        for style in [RAIL_BED_STYLE, RAIL_TIE_STYLE] {
            pass.set_bind_group(1, &self.styles[style], &[]);
            for class in 0..self.rail_tier {
                self.rails.draw(pass, class, view);
            }
        }
        if self.draw_cells {
            pass.set_bind_group(1, &self.styles[0], &[]);
            self.cells.draw(pass, 0, view);
        }
        for kind in 0..3 {
            pass.set_bind_group(1, &self.styles[kind + 1], &[]);
            level.lines.draw(pass, kind, view);
        }
    }
}

/// A grid of model sites (one per cell, jittered inside it in the shader) over a box of ground.
struct Instances {
    /// First cell (x, y), cells per row, cell size in metres, as the shader reads them.
    cells: [i32; 4],
    count: u32,
    spacing: f64,
}

/// Cells of at least `want_m` metres (a power-of-two multiple of `base_m`, so the cells stay put
/// while zooming between two steps) over the box, doubled until there are not too many.
fn instances(x0: f64, y0: f64, x1: f64, y1: f64, want_m: f64, base_m: i64) -> Instances {
    let mut s = base_m;
    while (s as f64) < want_m {
        s *= 2;
    }
    loop {
        let (i0, j0) = ((x0 / s as f64).floor() as i64, (y0 / s as f64).floor() as i64);
        let (i1, j1) = ((x1 / s as f64).ceil() as i64, (y1 / s as f64).ceil() as i64);
        let (nx, ny) = (i1 - i0 + 1, j1 - j0 + 1);
        if nx * ny <= MAX_INSTANCES {
            return Instances { cells: [i0 as i32, j0 as i32, nx as i32, s as i32], count: (nx * ny) as u32, spacing: s as f64 };
        }
        s *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(x0: i32, y0: i32, x1: i32, y1: i32) -> LineSeg {
        LineSeg { a: [x0, y0, x1, y1], dist: 0.0 }
    }

    #[test]
    fn line_tiles_cover_the_view_and_skip_the_rest() {
        let grid = TileGrid::new((0, 0, 6400, 3200)); // 100 x 100 m tiles
        let items = vec![(0, seg(10, 10, 20, 20)), (1, seg(5000, 3000, 5010, 3010)), (0, seg(150, 10, 160, 20)), (0, seg(6000, 3100, 6010, 3110))];
        let (index, segs) = TileIndex::build(grid, 2, items);
        let count = |c: usize, v: ViewBox| index.ranges(c, v).iter().map(|r| r.len()).sum::<usize>();
        assert_eq!(count(0, (0.0, 0.0, 300.0, 300.0)), 2);
        assert_eq!(count(1, (0.0, 0.0, 300.0, 300.0)), 0);
        assert_eq!(count(0, (0.0, 0.0, 6400.0, 3200.0)), 3);
        assert_eq!(count(1, (4900.0, 2900.0, 5100.0, 3100.0)), 1);
        // sorted by class, then tile: class 1 comes last
        assert_eq!(segs[3].a[0], 5000);
        // a view off the grid still finds the edge tiles, never panics
        assert_eq!(count(0, (-1e9, -1e9, -1e8, -1e8)), 1);
    }

    #[test]
    fn city_landmarks_move_into_their_town_and_skip_cities_without_one() {
        // one square town of 10 km by 10 km around (100 km, 100 km), as two triangles
        let urban = [[95_000, 95_000], [105_000, 95_000], [105_000, 105_000], [95_000, 105_000]];
        let tri = [0, 1, 2, 0, 2, 3];
        // a site 12 km beside the town, one far from any town, one inside it
        let got = city_landmarks(&urban, &tri, &[[112_000, 100_000], [400_000, 100_000], [99_000, 101_000]]);
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].seed, got[1].seed), (0, 2));
        for g in &got {
            assert_eq!(g.pos, [100_000, 100_000]);
            // a circle with the area of the square: r = sqrt(100 km2 / pi) = 5,642 m
            assert!((g.radius_m - 5_642.0).abs() < 2.0, "{}", g.radius_m);
        }
        assert!(city_landmarks(&[[0, 0]], &[0, 0, 0], &[[0, 0]]).is_empty());
    }

    #[test]
    fn instance_cells_snap_and_stay_bounded() {
        let a = instances(0.0, 0.0, 100_000.0, 50_000.0, 3_000.0, 250);
        assert_eq!(a.cells[3], 4_000); // 250 * 16, the first step of at least 3,000 m
        assert_eq!(a.count as i64, (25 + 1) * (13 + 1));
        let b = instances(-1.0e7, -1.0e7, 1.0e7, 1.0e7, 10.0, 250);
        assert!(b.count as i64 <= MAX_INSTANCES && b.cells[0] < 0);
    }
}
