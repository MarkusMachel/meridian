//! Dev probe: draw one egui frame offscreen to a PNG, to see whether text renders.
//! Usage: cargo run --release -p client --example egui_probe -- out.png [ppp]
use client::offscreen::{headless_gpu, write_png, FORMAT};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(&args[1]);
    let ppp: f32 = args.get(2).map_or(2.0, |s| s.parse().unwrap());
    let (w, h) = (2880u32, 300u32);
    let gpu = headless_gpu().expect("gpu");
    let ctx = egui::Context::default();
    let mut renderer = egui_wgpu::Renderer::new(&gpu.device, FORMAT, egui_wgpu::RendererOptions::default());
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut last = None;
    for frame in 0..3 {
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32 / ppp, h as f32 / ppp))), ..Default::default() };
        raw.viewports.get_mut(&raw.viewport_id).unwrap().native_pixels_per_point = Some(ppp);
        let full = ctx.run_ui(raw, |ui| {
            egui::Panel::top("time").show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("Game", |ui| {
                        let _ = ui.button("Save");
                    });
                    ui.separator();
                    ui.strong("1990-01-01 00:00");
                    ui.label("tick 0");
                    ui.separator();
                    let _ = ui.selectable_label(true, "Pause");
                    let _ = ui.button("Step");
                    for n in 1..=5u8 {
                        let _ = ui.selectable_label(false, format!("{n}"));
                    }
                    ui.separator();
                    egui::ComboBox::from_id_salt("mode").selected_text("Political").show_ui(ui, |_| {});
                    ui.separator();
                    ui.label("16.0 ms/frame, level 0, 21 km/px");
                });
            });
        });
        for (id, d) in &full.textures_delta.set {
            for x in d {
                renderer.update_texture(&gpu.device, &gpu.queue, *id, x);
            }
        }
        let jobs = ctx.tessellate(full.shapes, full.pixels_per_point);
        eprintln!("frame {frame}: jobs {}, set {}, free {}, ppp {}", jobs.len(), full.textures_delta.set.len(), full.textures_delta.free.len(), full.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: full.pixels_per_point };
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        let extra = renderer.update_buffers(&gpu.device, &gpu.queue, &mut enc, &jobs, &screen);
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.8, g: 0.89, b: 0.95, a: 1.0 }), store: wgpu::StoreOp::Store } })],
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
        buf.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buf.slice(..).get_mapped_range().unwrap();
        let mut rgba = Vec::new();
        for row in 0..h as usize {
            rgba.extend_from_slice(&data[row * padded as usize..row * padded as usize + (w * 4) as usize]);
        }
        last = Some(rgba);
        drop(data);
        for id in &full.textures_delta.free {
            renderer.free_texture(id);
        }
    }
    write_png(&out, w, h, &last.unwrap()).unwrap();
}
