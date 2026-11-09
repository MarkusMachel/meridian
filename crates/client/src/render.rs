//! The wgpu map renderer: province fills coloured through a lookup buffer, then border lines
//! (and cell edges when zoomed in). It knows nothing about the simulation; it is given colours
//! and a camera. Drawing a frame is a handful of draw calls over buffers uploaded once.

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
    _pad: u32,
    relief_org: [i32; 2],
    relief_span: [f32; 2],
}

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
/// Rivers of rank 1..=MAJOR show whenever zoomed in below RIVERS_MAJOR_BELOW; to MEDIUM below
/// RIVERS_MEDIUM_BELOW; all of them below RIVERS_ALL_BELOW (metres per pixel).
const RIVER_MAJOR_RANK: u8 = 5;
const RIVER_MEDIUM_RANK: u8 = 7;
const RIVERS_MAJOR_BELOW: f64 = 30_000.0;
const RIVERS_MEDIUM_BELOW: f64 = 9_000.0;
const RIVERS_ALL_BELOW: f64 = 3_500.0;
/// Cell edges are drawn only when zoomed in closer than this many metres per pixel.
const CELL_LINES_BELOW_M_PER_PX: f64 = 2500.0;
const SEA: wgpu::Color = wgpu::Color { r: 0.66, g: 0.76, b: 0.80, a: 1.0 };

struct GpuLevel {
    tolerance_m: u32,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    segments: wgpu::Buffer,
    /// Instance ranges per line kind: province border, state border, coast.
    ranges: [std::ops::Range<u32>; 3],
}

/// Rivers sorted by rank; `ends` are the segment counts up to the major, medium and all tiers.
struct GpuRivers {
    segments: wgpu::Buffer,
    ends: [u32; 3],
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
    cell_segments: wgpu::Buffer,
    cell_segment_count: u32,
    rivers: GpuRivers,
    river_tier: usize,
    relief_org: [i32; 2],
    relief_span: [f32; 2],
    level_in_use: usize,
    draw_cells: bool,
    /// Turn cell edges off entirely (a setting).
    pub show_cells: bool,
    /// Shaded relief under the province colours (a setting).
    pub show_relief: bool,
}

/// Push the segments of one polyline, each with the distance along the line at its start.
fn push_line(segs: &mut Vec<LineSeg>, xs: &[i32], ys: &[i32]) {
    let mut dist = 0.0f64;
    for k in 0..xs.len().saturating_sub(1) {
        segs.push(LineSeg { a: [xs[k], ys[k], xs[k + 1], ys[k + 1]], dist: dist as f32 });
        dist += (((xs[k + 1] - xs[k]) as f64).powi(2) + ((ys[k + 1] - ys[k]) as f64).powi(2)).sqrt();
    }
}

fn rivers_of(data: &RenderData) -> (Vec<LineSeg>, [u32; 3]) {
    let mut order: Vec<usize> = (0..data.river_rank.len()).collect();
    order.sort_by_key(|&i| (data.river_rank[i], i));
    let mut segs = Vec::new();
    let mut ends = [0u32; 3];
    for &i in &order {
        let (a, b) = (data.river_off[i] as usize, data.river_off[i + 1] as usize);
        if data.river_rank[i] > RIVER_MAJOR_RANK && ends[0] == 0 {
            ends[0] = segs.len() as u32;
        }
        if data.river_rank[i] > RIVER_MEDIUM_RANK && ends[1] == 0 {
            ends[1] = segs.len() as u32;
        }
        push_line(&mut segs, &data.river_x[a..b], &data.river_y[a..b]);
    }
    if ends[0] == 0 {
        ends[0] = segs.len() as u32;
    }
    if ends[1] == 0 {
        ends[1] = segs.len() as u32;
    }
    ends[2] = segs.len() as u32;
    (segs, ends)
}

fn segments_of(level: &RenderLevel) -> (Vec<LineSeg>, [std::ops::Range<u32>; 3]) {
    // Lines are stored sorted by kind, so each kind is one contiguous run of segments.
    let mut segs = Vec::new();
    let mut ranges = [0..0, 0..0, 0..0];
    for kind in 0..3u8 {
        let start = segs.len() as u32;
        for i in 0..level.line_count() {
            if level.line_kind[i] != kind {
                continue;
            }
            let (a, b) = (level.line_off[i] as usize, level.line_off[i + 1] as usize);
            push_line(&mut segs, &level.line_x[a..b], &level.line_y[a..b]);
        }
        ranges[kind as usize] = start..segs.len() as u32;
    }
    (segs, ranges)
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
        let relief_sampler = device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        let bind0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind0"),
            layout: &bgl0,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: colors.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&relief_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&relief_sampler) },
            ],
        });
        let style_buffers: Vec<wgpu::Buffer> = STYLES
            .iter()
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

        let levels = data
            .levels
            .iter()
            .map(|l| {
                let verts: Vec<FillVertex> = (0..l.vx.len()).map(|i| FillVertex { pos: [l.vx[i], l.vy[i]], prov: l.vprov[i] }).collect();
                let (segs, ranges) = segments_of(l);
                GpuLevel {
                    tolerance_m: l.tolerance_m,
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("verts"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("indices"), contents: bytemuck::cast_slice(&l.tri), usage: wgpu::BufferUsages::INDEX }),
                    index_count: l.tri.len() as u32,
                    segments: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("segments"), contents: bytemuck::cast_slice(&segs), usage: wgpu::BufferUsages::VERTEX }),
                    ranges,
                }
            })
            .collect();
        let cell_segs: Vec<LineSeg> = data.cell_seg.chunks_exact(4).map(|c| LineSeg { a: [c[0], c[1], c[2], c[3]], dist: 0.0 }).collect();
        let cell_segments = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("cell segments"), contents: bytemuck::cast_slice(&cell_segs), usage: wgpu::BufferUsages::VERTEX });
        let (river_segs, river_ends) = rivers_of(data);
        let rivers = GpuRivers { segments: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rivers"), contents: bytemuck::cast_slice(&river_segs), usage: wgpu::BufferUsages::VERTEX }), ends: river_ends };

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
            cell_segments,
            cell_segment_count: (data.cell_seg.len() / 4) as u32,
            rivers,
            river_tier: 0,
            relief_org: [rf.x0, rf.y1],
            relief_span: [(rf.x1 - rf.x0) as f32, (rf.y1 - rf.y0) as f32],
            level_in_use: 0,
            draw_cells: false,
            show_cells: true,
            show_relief: true,
        }
    }

    /// One colour per province (RGBA8).
    pub fn set_colors(&self, queue: &wgpu::Queue, colors: &[Rgba]) {
        assert_eq!(colors.len(), self.province_count, "one colour per province");
        queue.write_buffer(&self.colors, 0, bytemuck::cast_slice(colors));
    }

    /// Upload the camera and pick the detail level for this frame.
    pub fn prepare(&mut self, queue: &wgpu::Queue, camera: &Camera, selected: u32, hover: u32) {
        let tolerances: Vec<u32> = self.levels.iter().map(|l| l.tolerance_m).collect();
        self.level_in_use = camera.level_for(&tolerances);
        self.draw_cells = self.show_cells && camera.m_per_px < CELL_LINES_BELOW_M_PER_PX;
        let g = Globals {
            center: [camera.cx.round() as i32, camera.cy.round() as i32],
            viewport: [camera.width_px as f32, camera.height_px as f32],
            inv_scale: (1.0 / camera.m_per_px) as f32,
            selected,
            hover,
            _pad: 0,
            relief_org: self.relief_org,
            relief_span: if self.show_relief { self.relief_span } else { [0.0, 0.0] },
        };
        self.river_tier = if camera.m_per_px < RIVERS_ALL_BELOW { 3 } else if camera.m_per_px < RIVERS_MEDIUM_BELOW { 2 } else if camera.m_per_px < RIVERS_MAJOR_BELOW { 1 } else { 0 };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        // Province borders fade out when the whole world is in view, where 15,000 of them are only noise.
        let fade = (1.0 - camera.m_per_px / 20_000.0).clamp(0.0, 1.0) as f32;
        let mut s = STYLES[1];
        s.0[3] = 0.12 + (s.0[3] - 0.12) * fade;
        queue.write_buffer(&self.style_buffers[1], 0, bytemuck::bytes_of(&Style { color: s.0, width_px: s.1, dash_px: s.2, _pad: [0.0; 2] }));
    }

    pub fn level_in_use(&self) -> usize {
        self.level_in_use
    }

    pub const fn sea_color() -> wgpu::Color {
        SEA
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let level = &self.levels[self.level_in_use];
        pass.set_bind_group(0, &self.bind0, &[]);
        pass.set_pipeline(&self.fill_pipeline);
        pass.set_vertex_buffer(0, level.vertices.slice(..));
        pass.set_index_buffer(level.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..level.index_count, 0, 0..1);

        pass.set_pipeline(&self.line_pipeline);
        if self.river_tier > 0 {
            pass.set_bind_group(1, &self.styles[RIVER_STYLE], &[]);
            pass.set_vertex_buffer(0, self.rivers.segments.slice(..));
            pass.draw(0..6, 0..self.rivers.ends[self.river_tier - 1]);
        }
        if self.draw_cells {
            pass.set_bind_group(1, &self.styles[0], &[]);
            pass.set_vertex_buffer(0, self.cell_segments.slice(..));
            pass.draw(0..6, 0..self.cell_segment_count);
        }
        pass.set_vertex_buffer(0, level.segments.slice(..));
        for kind in 0..3 {
            pass.set_bind_group(1, &self.styles[kind + 1], &[]);
            pass.draw(0..6, level.ranges[kind].clone());
        }
    }
}
