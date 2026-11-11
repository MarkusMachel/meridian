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
    offsets: wgpu::Buffer,
    tiles: HashMap<u32, GpuTile>,
    frame: u64,
    /// Tiles drawn and tiles still to make, in the last frame (for the status line).
    pub drawn: (usize, usize),
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

impl GlobeRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, map: &Map, index: &HexIndex) -> GlobeRenderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("globe"), source: wgpu::ShaderSource::Wgsl(include_str!("globe.wgsl").into()) });
        let globals = device.create_buffer(&wgpu::BufferDescriptor { label: Some("globe globals"), size: std::mem::size_of::<Globals>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        let picture = index.world_picture(PICTURE.0, PICTURE.1);
        let picture_view = texture_u32(device, queue, "world picture", PICTURE.0, PICTURE.1, wgpu::TextureFormat::R32Uint, bytemuck::cast_slice(&picture));
        drop(picture);
        // per cell: province and country
        let cells = map.cell_count();
        let table_rows = cells.div_ceil(TABLE_WIDTH).max(1);
        let mut table = vec![[0u32; 2]; (TABLE_WIDTH * table_rows) as usize];
        for (c, entry) in table.iter_mut().enumerate().take(cells as usize) {
            *entry = [map.cell_province[c], map.cell_state[c] as u32];
        }
        let table_view = texture_u32(device, queue, "cell table", TABLE_WIDTH, table_rows, wgpu::TextureFormat::Rg32Uint, bytemuck::cast_slice(&table));
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
            ],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globe"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&picture_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&table_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&palette_view) },
            ],
        });
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
        let offsets = device.create_buffer(&wgpu::BufferDescriptor { label: Some("tile offsets"), size: (TILES_KEPT * 12) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        GlobeRenderer { globals, bind, palette, palette_rows, far_pipeline, tile_pipeline, offsets, tiles: HashMap::new(), frame: 0, drawn: (0, 0) }
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
        let eye = camera.eye();
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
    }
}
