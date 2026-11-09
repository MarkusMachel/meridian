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
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Style {
    color: [f32; 4],
    width_px: f32,
    _pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FillVertex {
    pos: [i32; 2],
    prov: u32,
}

/// Province id meaning "none" for selection and hover.
pub const NO_PROVINCE: u32 = u32::MAX;

/// Line styles in draw order: cell edge, province border, state border, coast.
const STYLES: [([f32; 4], f32); 4] = [
    ([0.0, 0.0, 0.0, 0.18], 1.0),
    ([0.10, 0.10, 0.12, 0.55], 1.0),
    ([0.05, 0.05, 0.08, 0.95], 2.0),
    ([0.02, 0.05, 0.12, 0.95], 1.5),
];
/// Cell edges are drawn only when zoomed in closer than this many metres per pixel.
const CELL_LINES_BELOW_M_PER_PX: f64 = 2500.0;
const SEA: wgpu::Color = wgpu::Color { r: 0.80, g: 0.89, b: 0.95, a: 1.0 };

struct GpuLevel {
    tolerance_m: u32,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    segments: wgpu::Buffer,
    /// Instance ranges per line kind: province border, state border, coast.
    ranges: [std::ops::Range<u32>; 3],
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
    level_in_use: usize,
    draw_cells: bool,
}

fn segments_of(level: &RenderLevel) -> (Vec<[i32; 4]>, [std::ops::Range<u32>; 3]) {
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
            for k in a..b - 1 {
                segs.push([level.line_x[k], level.line_y[k], level.line_x[k + 1], level.line_y[k + 1]]);
            }
        }
        ranges[kind as usize] = start..segs.len() as u32;
    }
    (segs, ranges)
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, data: &RenderData, province_count: usize) -> Renderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("map"), source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()) });
        let globals = device.create_buffer(&wgpu::BufferDescriptor { label: Some("globals"), size: std::mem::size_of::<Globals>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let colors = device.create_buffer(&wgpu::BufferDescriptor { label: Some("colors"), size: (province_count.max(1) * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bgl0"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bgl1"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
        });
        let bind0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind0"),
            layout: &bgl0,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: colors.as_entire_binding() }],
        });
        let style_buffers: Vec<wgpu::Buffer> = STYLES
            .iter()
            .map(|(color, width)| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("style"), contents: bytemuck::bytes_of(&Style { color: *color, width_px: *width, _pad: [0.0; 3] }), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST }))
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
        let line_attrs = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint32x2, offset: 0, shader_location: 0 }, wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint32x2, offset: 8, shader_location: 1 }];
        let fill_buffers = [Some(wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &fill_attrs })];
        let line_buffers = [Some(wgpu::VertexBufferLayout { array_stride: 16, step_mode: wgpu::VertexStepMode::Instance, attributes: &line_attrs })];
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
        let cell_segments = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("cell segments"), contents: bytemuck::cast_slice(&data.cell_seg), usage: wgpu::BufferUsages::VERTEX });

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
            level_in_use: 0,
            draw_cells: false,
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
        self.draw_cells = camera.m_per_px < CELL_LINES_BELOW_M_PER_PX;
        let g = Globals {
            center: [camera.cx.round() as i32, camera.cy.round() as i32],
            viewport: [camera.width_px as f32, camera.height_px as f32],
            inv_scale: (1.0 / camera.m_per_px) as f32,
            selected,
            hover,
            _pad: 0,
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        // Province borders fade out when the whole world is in view, where 15,000 of them are only noise.
        let fade = (1.0 - camera.m_per_px / 20_000.0).clamp(0.0, 1.0) as f32;
        let mut s = STYLES[1];
        s.0[3] = 0.12 + (s.0[3] - 0.12) * fade;
        queue.write_buffer(&self.style_buffers[1], 0, bytemuck::bytes_of(&Style { color: s.0, width_px: s.1, _pad: [0.0; 3] }));
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
