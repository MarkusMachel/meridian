//! Rendering without a window: used for screenshots and for checking the renderer in tests.

use crate::camera::Camera;
use crate::render::Renderer;

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_name: String,
}

/// A GPU with no surface, or `None` if the machine has no usable adapter.
pub fn headless_gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some(Gpu { device, queue, adapter_name: adapter.get_info().name })
}

/// Colours are written as ordinary (gamma) values, the same as in the window, so screenshots match the window.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Draw one frame into an RGBA8 image (rows top to bottom).
pub fn render_rgba(gpu: &Gpu, renderer: &mut Renderer, camera: &Camera, selected: u32, hover: u32) -> Vec<u8> {
    let (w, h) = (camera.width_px as u32, camera.height_px as u32);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.render(&gpu.device, &gpu.queue, &mut encoder, &view, camera, selected, hover, 0.0);
    let padded = (w * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (padded * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("gpu poll");
    let data = slice.get_mapped_range().expect("read mapped range");
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        out.extend_from_slice(&data[row * padded as usize..row * padded as usize + (w * 4) as usize]);
    }
    out
}

/// Draw one frame of anything into an RGBA8 image (rows top to bottom): `draw` is given the
/// encoder and the picture to draw into.
pub fn draw_rgba(gpu: &Gpu, w: u32, h: u32, draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView)) -> Vec<u8> {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    draw(&mut encoder, &view);
    let padded = (w * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (padded * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("gpu poll");
    let data = slice.get_mapped_range().expect("read mapped range");
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        out.extend_from_slice(&data[row * padded as usize..row * padded as usize + (w * 4) as usize]);
    }
    out
}

/// GPU throughput: draw `frames` frames back to back into one texture without reading them back,
/// wait for the GPU once at the end, and return the mean milliseconds per frame. Steadier than
/// timing single frames, which each wait for a full round trip to the GPU.
pub fn throughput_ms(gpu: &Gpu, renderer: &mut Renderer, camera: &Camera, frames: u32) -> f64 {
    let (w, h) = (camera.width_px as u32, camera.height_px as u32);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let start = std::time::Instant::now();
    for k in 0..frames {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.render(&gpu.device, &gpu.queue, &mut encoder, &view, camera, crate::render::NO_PROVINCE, crate::render::NO_PROVINCE, k as f32 / 60.0);
        gpu.queue.submit([encoder.finish()]);
    }
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("gpu poll");
    start.elapsed().as_secs_f64() * 1000.0 / frames as f64
}

pub fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Draw with egui onto a transparent picture of `w` by `h` pixels and return its RGBA bytes
/// (colour already multiplied by alpha). For probes that lay counters or labels over a rendered
/// map: `draw` gets a painter covering the whole picture, in points (`ppp` pixels to a point).
pub fn egui_rgba(gpu: &Gpu, w: u32, h: u32, ppp: f32, mut draw: impl FnMut(&egui::Painter)) -> Vec<u8> {
    let ctx = egui::Context::default();
    let mut renderer = egui_wgpu::Renderer::new(&gpu.device, FORMAT, egui_wgpu::RendererOptions::default());
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("egui offscreen"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut out = Vec::new();
    // egui needs a frame or two before fonts and sizes settle
    for _ in 0..2 {
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32 / ppp, h as f32 / ppp))), ..Default::default() };
        raw.viewports.get_mut(&raw.viewport_id).expect("the root viewport").native_pixels_per_point = Some(ppp);
        let full = ctx.run_ui(raw, |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            draw(&painter);
        });
        for (id, d) in &full.textures_delta.set {
            for x in d {
                renderer.update_texture(&gpu.device, &gpu.queue, *id, x);
            }
        }
        let jobs = ctx.tessellate(full.shapes, full.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: full.pixels_per_point };
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        let extra = renderer.update_buffers(&gpu.device, &gpu.queue, &mut enc, &jobs, &screen);
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui offscreen"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            renderer.render(&mut pass, &jobs, &screen);
        }
        let padded = (w * 4).div_ceil(256) * 256;
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (padded * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        gpu.queue.submit(extra.into_iter().chain(std::iter::once(enc.finish())));
        buf.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map the picture"));
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("wait for the GPU");
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        out.clear();
        for row in 0..h as usize {
            out.extend_from_slice(&data[row * padded as usize..row * padded as usize + (w * 4) as usize]);
        }
        drop(data);
        for id in &full.textures_delta.free {
            renderer.free_texture(id);
        }
    }
    out
}

/// Lay a picture from `egui_rgba` over another of the same size.
pub fn lay_over(base: &mut [u8], over: &[u8]) {
    for (b, o) in base.chunks_exact_mut(4).zip(over.chunks_exact(4)) {
        let keep = 255 - o[3] as u32;
        for k in 0..3 {
            b[k] = (o[k] as u32 + b[k] as u32 * keep / 255).min(255) as u8;
        }
    }
}
