//! Drawing the globe. See `globe.wgsl` for the two passes.

use super::camera::{GlobeCamera, EARTH_RADIUS_M, TAN_HALF_FOV};
use super::index::{HexIndex, Vertex, NO_CELL};
use super::{dir_of, sub};
use crate::modes::Rgba;
use bytemuck::{Pod, Zeroable};
use meridian_core::Map;
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Hexagons are drawn as triangles when a pixel is less than this many metres of ground; from
/// farther away the world picture is enough (a hexagon is then under about six pixels).
pub const TILES_BELOW_M_PER_PX: f64 = 1_000.0;
/// Size of the world picture (longitude by latitude): 4.9 km a pixel at the equator.
const PICTURE: (u32, u32) = (8192, 4096);
/// Width of the per-cell table texture.
const TABLE_WIDTH: u32 = 2048;
/// Tiles made per frame at most, so moving the camera never stalls a frame for long.
const TILES_PER_FRAME: usize = 24;
/// Tiles kept on the graphics card; the ones unused longest are dropped beyond this.
const TILES_KEPT: usize = 1200;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    row_x: [f32; 4],
    row_y: [f32; 4],
    toward: [f32; 4],
    east: [f32; 4],
    north: [f32; 4],
    misc: [f32; 4],
    ids: [u32; 4],
    more: [f32; 4],
    eye_whole: [i32; 4],
    eye_part: [f32; 4],
    screen: [f32; 4],
    pack_a: [f32; 4],
    pack_b: [u32; 4],
}

/// One piece of a real line as the graphics card gets it: 28 bytes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Piece {
    a: [i32; 3],
    b: [i32; 3],
    style: u32,
}

/// The real lines on the graphics card: all pieces in one buffer, in buckets.
struct Lines {
    pieces: wgpu::Buffer,
    /// (detail tier, direction to the bucket's middle, its pieces)
    buckets: Vec<(u8, [f64; 3], std::ops::Range<u32>)>,
    /// How far a bucket's pieces reach from its middle, as an angle.
    reach: f64,
}

/// The textures the shader reads, kept so the set can be made again when one is replaced.
struct Views {
    picture: wgpu::TextureView,
    table: wgpu::TextureView,
    palette: wgpu::TextureView,
    relief: wgpu::TextureView,
    cover_tiles: wgpu::TextureView,
    cover_table: wgpu::TextureView,
}

/// Detail tiers come in below these metres per pixel: 0 always, 1 from a continent, 2 from a
/// country, 3 from a region, 4 close up (regional packs only).
const TIER_BELOW_M_PER_PX: [f64; 5] = [f64::MAX, 5_000.0, 1_400.0, 600.0, 250.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coloring {
    PaletteShaded = 0,
    Palette = 1,
    Terrain = 2,
}

struct GpuTile {
    buffer: wgpu::Buffer,
    vertices: u32,
    /// The tile's anchor in metres from the Earth's centre (kept in 64 bits).
    anchor: [f64; 3],
    last_used: u64,
}

pub struct GlobeRenderer {
    globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    palette: wgpu::Texture,
    palette_rows: u32,
    far_pipeline: wgpu::RenderPipeline,
    tile_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    views: Views,
    /// The world's lines first, then each regional pack's.
    lines: Vec<Lines>,
    has_relief: bool,
    /// A pack's ground cover as the shader needs it: (west, north, pixel width, pixel height) in
    /// degrees and (width, height, tile size, tiles across) in pixels.
    pack: ([f32; 4], [u32; 4]),
    /// Draw the real lines (when a lines file was given).
    pub show_lines: bool,
    /// Shade the relief in, and draw a pack's woods, towns and water (when the files were given).
    pub show_relief: bool,
    pub show_cover: bool,
    offsets: wgpu::Buffer,
    tiles: HashMap<u32, GpuTile>,
    frame: u64,
    /// Tiles drawn and tiles still to make, in the last frame (for the status line).
    pub drawn: (usize, usize),
    /// How land is coloured: by the province palette with the terrain shaded in (political),
    /// by the palette alone, or by each hexagon's own terrain.
    pub coloring: Coloring,
    /// Draw roads, railways and bridges when zoomed in.
    pub show_roads: bool,
}

fn texture_u32(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, width: u32, height: u32, format: wgpu::TextureFormat, data: &[u8]) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            data,
        )
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// The ground cover of a pack as two textures: the stored tiles as layers of one array, and
/// a small table saying which layer each tile of the picture is.
#[allow(clippy::too_many_arguments)]
fn cover_textures(device: &wgpu::Device, queue: &wgpu::Queue, tile: u32, stored: u32, across: u32, down: u32, tiles: &[u8], table: &[u32]) -> (wgpu::TextureView, wgpu::TextureView) {
    let layers = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("ground cover"),
            size: wgpu::Extent3d { width: tile, height: tile, depth_or_array_layers: stored },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        tiles,
    );
    // a one-layer texture would otherwise be taken for a plain picture, not an array
    let layers_view = layers.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    (layers_view, texture_u32(device, queue, "ground cover table", across, down, wgpu::TextureFormat::R32Uint, bytemuck::cast_slice(table)))
}

fn bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, globals: &wgpu::Buffer, v: &Views) -> wgpu::BindGroup {
    let view = |binding, view| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("globe"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }, view(1, &v.picture), view(2, &v.table), view(3, &v.palette), view(4, &v.relief), view(5, &v.cover_tiles), view(6, &v.cover_table)],
    })
}

/// Is a longitude and latitude (degrees) inside one of the rings? Rings are longitude and
/// latitude pairs in millionths of a degree; a point is inside when a line from it to the
/// east crosses a ring's edge an odd number of times.
pub fn inside_rings(points: &[i32], ring_off: &[u32], lon: f64, lat: f64) -> bool {
    let (x, y) = (lon * 1e6, lat * 1e6);
    ring_off.windows(2).any(|w| {
        let ring = &points[2 * w[0] as usize..2 * w[1] as usize];
        let n = ring.len() / 2;
        let mut inside = false;
        for i in 0..n {
            let (ax, ay) = (ring[2 * i] as f64, ring[2 * i + 1] as f64);
            let (bx, by) = (ring[2 * ((i + 1) % n)] as f64, ring[2 * ((i + 1) % n) + 1] as f64);
            if (ay > y) != (by > y) && x < ax + (y - ay) / (by - ay) * (bx - ax) {
                inside = !inside;
            }
        }
        inside
    })
}

impl GlobeRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, map: &Map, index: &HexIndex) -> GlobeRenderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("globe"), source: wgpu::ShaderSource::Wgsl(include_str!("globe.wgsl").into()) });
        let globals = device.create_buffer(&wgpu::BufferDescriptor { label: Some("globe globals"), size: std::mem::size_of::<Globals>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        let picture = index.world_picture(PICTURE.0, PICTURE.1);
        let picture_view = texture_u32(device, queue, "world picture", PICTURE.0, PICTURE.1, wgpu::TextureFormat::R32Uint, bytemuck::cast_slice(&picture));
        drop(picture);
        // per cell: province, country, terrain and how built-up it is
        let cells = map.cell_count();
        let table_rows = cells.div_ceil(TABLE_WIDTH).max(1);
        let mut table = vec![[0u32; 4]; (TABLE_WIDTH * table_rows) as usize];
        for (c, entry) in table.iter_mut().enumerate().take(cells as usize) {
            *entry = [map.cell_province[c], map.cell_state[c] as u32, map.cell_terrain[c] as u32, map.cell_urban[c] as u32];
        }
        let table_view = texture_u32(device, queue, "cell table", TABLE_WIDTH, table_rows, wgpu::TextureFormat::Rgba32Uint, bytemuck::cast_slice(&table));
        let palette_rows = map.province_count().div_ceil(4096).max(1);
        let palette = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("palette"),
            size: wgpu::Extent3d { width: 4096, height: palette_rows, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let palette_view = palette.create_view(&wgpu::TextureViewDescriptor::default());

        let uint = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                uint(1),
                uint(2),
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
                uint(6),
            ],
        });
        // until the real ones are given: flat relief and an empty ground cover
        let relief_view = texture_u32(device, queue, "relief", 1, 1, wgpu::TextureFormat::R8Unorm, &[128]);
        let (cover_tiles, cover_table) = cover_textures(device, queue, 1, 1, 1, 1, &[0], &[NO_CELL]);
        let views = Views { picture: picture_view, table: table_view, palette: palette_view, relief: relief_view, cover_tiles, cover_table };
        let bind = bind_group(device, &layout, &globals, &views);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("globe"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let target = [Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let far_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("far"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_far"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_far"), compilation_options: Default::default(), targets: &target }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertex_attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 16, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 20, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 24, shader_location: 5 },
        ];
        let offset_attrs = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 4 }];
        let buffers = [
            Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &vertex_attrs }),
            Some(wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Instance, attributes: &offset_attrs }),
        ];
        let tile_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tiles"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_tile"), compilation_options: Default::default(), buffers: &buffers },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_tile"), compilation_options: Default::default(), targets: &target }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let piece_attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint32x3, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 24, shader_location: 2 },
        ];
        let piece_buffers = [Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Piece>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &piece_attrs })];
        let blended = [Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })];
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lines"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_line"), compilation_options: Default::default(), buffers: &piece_buffers },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_line"), compilation_options: Default::default(), targets: &blended }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let offsets = device.create_buffer(&wgpu::BufferDescriptor { label: Some("tile offsets"), size: (TILES_KEPT * 12) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        GlobeRenderer { globals, bind, palette, palette_rows, far_pipeline, tile_pipeline, line_pipeline, layout, views, lines: Vec::new(), has_relief: false, pack: ([0.0; 4], [0; 4]), show_lines: true, show_relief: true, show_cover: true, offsets, tiles: HashMap::new(), frame: 0, drawn: (0, 0), coloring: Coloring::PaletteShaded, show_roads: true }
    }

    /// Give the renderer the real lines to draw over the hexagons: the world's, then each
    /// regional pack's. Inside a pack's coverage the world's roads, railways and small rivers
    /// are hidden, because the pack has the same ones in finer detail.
    pub fn set_lines(&mut self, device: &wgpu::Device, world: Option<&meridian_data::GlobeLines>, packs: &[meridian_data::GlobeLines]) {
        self.lines.clear();
        let covered = |lon: f64, lat: f64| packs.iter().any(|p| inside_rings(&p.cover, &p.cover_off, lon, lat));
        for (data, is_world) in world.into_iter().map(|w| (w, true)).chain(packs.iter().map(|p| (p, false))) {
            let pieces: Vec<Piece> = (0..data.seg_style.len())
                .map(|i| {
                    let (a, b) = ([data.seg_a[3 * i], data.seg_a[3 * i + 1], data.seg_a[3 * i + 2]], [data.seg_b[3 * i], data.seg_b[3 * i + 1], data.seg_b[3 * i + 2]]);
                    let mut style = data.seg_style[i] as u32;
                    if is_world && style >= 3 && !packs.is_empty() {
                        let (lat, lon) = super::lat_lon_of([(a[0] + b[0]) as f64, (a[1] + b[1]) as f64, (a[2] + b[2]) as f64]);
                        if covered(lon.to_degrees(), lat.to_degrees()) {
                            style = 255;
                        }
                    }
                    Piece { a, b, style }
                })
                .collect();
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("real lines"), contents: bytemuck::cast_slice(&pieces), usage: wgpu::BufferUsages::VERTEX });
            let buckets = (0..data.bucket_tier.len())
                .map(|k| {
                    let middle = super::unit([data.bucket_xyz[3 * k] as f64, data.bucket_xyz[3 * k + 1] as f64, data.bucket_xyz[3 * k + 2] as f64]);
                    (data.bucket_tier[k], middle, data.bucket_off[k]..data.bucket_off[k + 1])
                })
                .collect();
            self.lines.push(Lines { pieces: buffer, buckets, reach: data.bucket_reach_microrad as f64 * 1e-6 });
        }
    }

    /// Hand over everything that was found next to the map.
    pub fn set_assets(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, a: &super::assets::GlobeAssets) {
        self.set_lines(device, a.lines.as_ref(), &a.packs);
        if let Some(relief) = &a.relief {
            self.set_relief(device, queue, relief);
        }
        if let Some(cover) = &a.cover {
            self.set_cover(device, queue, cover);
        }
    }

    /// Give the renderer the shaded relief of the world.
    pub fn set_relief(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, relief: &meridian_data::GlobeRelief) {
        self.views.relief = texture_u32(device, queue, "relief", relief.width, relief.height, wgpu::TextureFormat::R8Unorm, &relief.shade);
        self.has_relief = true;
        self.bind = bind_group(device, &self.layout, &self.globals, &self.views);
    }

    /// Give the renderer a regional pack's ground cover (one pack at a time for now).
    pub fn set_cover(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, c: &meridian_data::PackCover) {
        let stored = c.tiles.len() as u32 / (c.tile * c.tile);
        (self.views.cover_tiles, self.views.cover_table) = cover_textures(device, queue, c.tile, stored, c.across, c.down, &c.tiles, &c.tile_layer);
        self.pack = ([c.west_e6 as f32 * 1e-6, c.north_e6 as f32 * 1e-6, c.pixel_lon_e9 as f32 * 1e-9, c.pixel_lat_e9 as f32 * 1e-9], [c.width, c.height, c.tile, c.across]);
        self.bind = bind_group(device, &self.layout, &self.globals, &self.views);
    }

    /// One colour per province (from a map mode).
    pub fn set_colors(&self, queue: &wgpu::Queue, colors: &[Rgba]) {
        let mut data = vec![[0u8; 4]; (4096 * self.palette_rows) as usize];
        data[..colors.len()].copy_from_slice(colors);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.palette, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4096 * 4), rows_per_image: Some(self.palette_rows) },
            wgpu::Extent3d { width: 4096, height: self.palette_rows, depth_or_array_layers: 1 },
        );
    }

    /// Draw a frame. `selected` is the highlighted cell (`NO_CELL` for none). With
    /// `wait_for_tiles` every tile in view is made before drawing (for screenshots); otherwise
    /// only a few are made per frame and the far picture shows through until they are ready.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, camera: &GlobeCamera, map: &Map, index: &HexIndex, selected: Option<u32>, wait_for_tiles: bool) {
        self.frame += 1;
        let (f, east, north) = camera.axes();
        let aspect = camera.width_px / camera.height_px;
        let m_per_px = camera.m_per_px();
        let near = m_per_px < TILES_BELOW_M_PER_PX;
        // the far land fades out as the hexagons take over, and the grid fades in once a hexagon is some pixels wide
        let far_land = ((m_per_px - 0.4 * TILES_BELOW_M_PER_PX) / (0.6 * TILES_BELOW_M_PER_PX)).clamp(0.0, 1.0);
        let grid = ((250.0 - m_per_px) / 150.0).clamp(0.0, 1.0);
        // roads and railways come in once a hexagon is about 15 pixels wide
        let roads = if self.show_roads { ((450.0 - m_per_px) / 150.0).clamp(0.0, 1.0) } else { 0.0 };
        let real_lines = self.show_lines && !self.lines.is_empty();
        let eye = camera.eye();
        let eye_whole = eye.map(f64::floor);
        // built-up ground shows from a country down
        let built_up = ((1_500.0 - m_per_px) / 700.0).clamp(0.0, 1.0);
        let v4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];
        let k = |v: [f64; 3], by: f64| v.map(|x| x / by);
        let globals = Globals {
            row_x: v4(k(east, TAN_HALF_FOV * aspect), 0.0),
            row_y: v4(k(north, TAN_HALF_FOV), 0.0),
            toward: v4(f, EARTH_RADIUS_M + camera.altitude_m),
            east: v4(east, TAN_HALF_FOV * aspect),
            north: v4(north, TAN_HALF_FOV),
            misc: [EARTH_RADIUS_M as f32, far_land as f32, grid as f32, 0.0],
            ids: [selected.unwrap_or(NO_CELL), PICTURE.0, PICTURE.1, TABLE_WIDTH],
            more: [roads as f32, self.coloring as u32 as f32, if real_lines { 1.0 } else { 0.0 }, built_up as f32],
            eye_whole: [eye_whole[0] as i32, eye_whole[1] as i32, eye_whole[2] as i32, 0],
            eye_part: [(eye[0] - eye_whole[0]) as f32, (eye[1] - eye_whole[1]) as f32, (eye[2] - eye_whole[2]) as f32, m_per_px as f32],
            screen: [camera.width_px as f32, camera.height_px as f32, if self.has_relief && self.show_relief { 1.0 } else { 0.0 }, 0.0],
            pack_a: self.pack.0,
            pack_b: if self.show_cover { self.pack.1 } else { [0; 4] },
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        // which tiles to draw, making the missing ones nearest the centre first
        let mut show: Vec<u32> = Vec::new();
        let mut missing = 0;
        if near {
            let mut wanted = index.tiles_near(dir_of(camera.lat, camera.lon), camera.view_radius());
            wanted.sort_by(|&a, &b| super::dot(index.tiles[b as usize].center, f).total_cmp(&super::dot(index.tiles[a as usize].center, f)));
            wanted.truncate(TILES_KEPT);
            let mut made = 0;
            for t in wanted {
                let tile = match self.tiles.entry(t) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        if made >= TILES_PER_FRAME && !wait_for_tiles {
                            missing += 1;
                            continue;
                        }
                        made += 1;
                        let (anchor, vertices) = index.tile_mesh(map, t);
                        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("tile"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
                        e.insert(GpuTile { buffer, vertices: vertices.len() as u32, anchor, last_used: 0 })
                    }
                };
                tile.last_used = self.frame;
                show.push(t);
            }
            if self.tiles.len() > TILES_KEPT {
                let mut by_age: Vec<(u64, u32)> = self.tiles.iter().map(|(&t, g)| (g.last_used, t)).collect();
                by_age.sort_unstable();
                for &(_, t) in by_age.iter().take(self.tiles.len() - TILES_KEPT) {
                    self.tiles.remove(&t);
                }
            }
        }
        self.drawn = (show.len(), missing);
        let offsets: Vec<[f32; 3]> = show.iter().map(|t| sub(self.tiles[t].anchor, eye).map(|v| v as f32)).collect();
        if !offsets.is_empty() {
            queue.write_buffer(&self.offsets, 0, bytemuck::cast_slice(&offsets));
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("globe"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_pipeline(&self.far_pipeline);
        pass.draw(0..3, 0..1);
        if !show.is_empty() {
            pass.set_pipeline(&self.tile_pipeline);
            pass.set_vertex_buffer(1, self.offsets.slice(..));
            for (i, t) in show.iter().enumerate() {
                let tile = &self.tiles[t];
                pass.set_vertex_buffer(0, tile.buffer.slice(..));
                pass.draw(0..tile.vertices, i as u32..i as u32 + 1);
            }
        }
        for lines in self.lines.iter().filter(|_| real_lines) {
            pass.set_pipeline(&self.line_pipeline);
            pass.set_vertex_buffer(0, lines.pieces.slice(..));
            let limit = (camera.view_radius() + lines.reach).min(std::f64::consts::PI).cos();
            for (tier, middle, range) in &lines.buckets {
                if m_per_px < TIER_BELOW_M_PER_PX[(*tier as usize).min(4)] && super::dot(*middle, f) >= limit {
                    pass.draw(0..6, range.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inside_rings;

    #[test]
    fn a_point_is_inside_a_ring_or_not() {
        // a square from 10 to 12 E, 40 to 42 N, and a triangle far away
        let points = [10_000_000, 40_000_000, 12_000_000, 40_000_000, 12_000_000, 42_000_000, 10_000_000, 42_000_000, 30_000_000, 0, 31_000_000, 0, 30_500_000, 1_000_000];
        let rings = [0, 4, 7];
        assert!(inside_rings(&points, &rings, 11.0, 41.0));
        assert!(inside_rings(&points, &rings, 30.5, 0.3));
        assert!(!inside_rings(&points, &rings, 9.99, 41.0));
        assert!(!inside_rings(&points, &rings, 11.0, 42.01));
        assert!(!inside_rings(&points, &rings, 30.1, 0.9));
        assert!(!inside_rings(&[], &[], 11.0, 41.0));
    }
}
