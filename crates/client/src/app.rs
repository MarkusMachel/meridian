//! The window: winit for the window and input, wgpu for the surface, egui for panels.
//! The map itself is drawn by `Renderer`; this file only wires input, the session and the UI.

use client::camera::Camera;
use client::modes::{self, Mode};
use client::render::{Renderer, NO_PROVINCE};
use client::session::Session;
use client::{Picker, Speed};
use meridian_core::{CountryId, Map};
use meridian_data::RenderData;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

pub struct Inputs {
    pub map: Arc<Map>,
    pub render: RenderData,
    pub session: Session,
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    map_renderer: Renderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

struct App {
    inputs: Option<Inputs>,
    map: Arc<Map>,
    session: Session,
    picker: Picker,
    terrain: Vec<u8>,
    gfx: Option<Gfx>,
    camera: Camera,
    mode: Mode,
    colors_dirty: bool,
    shown_tick: Option<u64>,
    selected: u32,
    hover: u32,
    cursor: PhysicalPosition<f64>,
    press: Option<PhysicalPosition<f64>>,
    dragging: bool,
    frame_ms: f32,
    last_frame: Instant,
    snapshot: client::Snapshot,
}

pub fn run(inputs: Inputs) {
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let level = inputs.render.levels.last().expect("render file has levels");
    let picker = Picker::new(level);
    let terrain = modes::dominant_terrain(&inputs.map);
    let mut app = App {
        map: inputs.map.clone(),
        session: Session::new(meridian_core::World::new(0, 1, 1), Box::new(proving_ground::schedule(false)), CountryId(0)),
        picker,
        terrain,
        gfx: None,
        camera: Camera::fit(1600.0, 900.0),
        mode: Mode::Political,
        colors_dirty: true,
        shown_tick: None,
        selected: NO_PROVINCE,
        hover: NO_PROVINCE,
        cursor: PhysicalPosition::new(0.0, 0.0),
        press: None,
        dragging: false,
        frame_ms: 16.0,
        last_frame: Instant::now(),
        snapshot: Default::default(),
        inputs: Some(inputs),
    };
    // The real session replaces the placeholder above (it cannot be moved out of `inputs` in the struct literal).
    if let Some(i) = app.inputs.as_mut() {
        std::mem::swap(&mut app.session, &mut i.session);
    }
    event_loop.run_app(&mut app).expect("run event loop");
}

impl App {
    fn init_gfx(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(event_loop.create_window(Window::default_attributes().with_title("Meridian").with_inner_size(winit::dpi::LogicalSize::new(1440.0, 900.0))).expect("create window"));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(event_loop.owned_display_handle())));
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() })).expect("no GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("create device");
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        // Plain (non-sRGB) format: colours are written as ordinary values, and egui expects this too.
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        config.format = format;
        surface.configure(&device, &config);

        let inputs = self.inputs.take().expect("inputs are used once");
        let map_renderer = Renderer::new(&device, format, &inputs.render, self.map.province_count() as usize);
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, &*window, Some(window.scale_factor() as f32), window.theme(), Some(device.limits().max_texture_dimension_2d as usize));
        let egui_renderer = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        self.camera = Camera::fit(size.width.max(1) as f64, size.height.max(1) as f64);
        self.gfx = Some(Gfx { window, surface, device, queue, config, map_renderer, egui_ctx, egui_state, egui_renderer });
    }

    fn refresh_colors(&mut self) {
        let snap = self.session.refresh().clone();
        let changed = self.shown_tick != Some(snap.tick);
        if changed || self.colors_dirty {
            let colors = match self.mode {
                Mode::Political => modes::political(&snap.controllers),
                Mode::Value => modes::value_ramp(&snap.values),
                Mode::Terrain => modes::terrain(&self.terrain),
            };
            if let Some(g) = &self.gfx {
                g.map_renderer.set_colors(&g.queue, &colors);
            }
            self.shown_tick = Some(snap.tick);
            self.colors_dirty = false;
        }
        self.snapshot = snap;
    }

    fn pick(&self, p: PhysicalPosition<f64>) -> u32 {
        let (x, y) = self.camera.screen_to_world(p.x, p.y);
        self.picker.province_at(x, y).unwrap_or(NO_PROVINCE)
    }

    fn key(&mut self, key: &Key) {
        match key {
            Key::Named(NamedKey::Space) => {
                let s = if self.session.speed() == Speed::Paused { Speed::Run(1) } else { Speed::Paused };
                self.session.set_speed(s);
            }
            Key::Character(c) => match c.as_str() {
                "1" | "2" | "3" | "4" | "5" => self.session.set_speed(Speed::Run(c.parse().unwrap())),
                "." => {
                    self.session.set_speed(Speed::Paused);
                    self.session.step();
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        self.frame_ms = self.frame_ms * 0.9 + now.duration_since(self.last_frame).as_secs_f32() * 1000.0 * 0.1;
        self.last_frame = now;
        self.session.frame();
        self.refresh_colors();
        let Some(mut g) = self.gfx.take() else { return };

        // --- egui ---
        let raw = g.egui_state.take_egui_input(&g.window);
        let mut ui_actions = UiActions::default();
        let full = g.egui_ctx.run_ui(raw, |ui| ui_actions = self.ui(ui));
        g.egui_state.handle_platform_output(&g.window, full.platform_output);
        if let Some(m) = ui_actions.mode {
            self.mode = m;
            self.colors_dirty = true;
        }
        if let Some(s) = ui_actions.speed {
            self.session.set_speed(s);
        }
        if ui_actions.step {
            self.session.set_speed(Speed::Paused);
            self.session.step();
        }
        let ppp = full.pixels_per_point;
        let jobs = g.egui_ctx.tessellate(full.shapes, ppp);

        // --- surface ---
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                g.surface.configure(&g.device, &g.config);
                self.gfx = Some(g);
                return;
            }
            _ => {
                self.gfx = Some(g);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        g.map_renderer.prepare(&g.queue, &self.camera, self.selected, self.hover);
        let mut encoder = g.device.create_command_encoder(&Default::default());
        for (id, delta) in &full.textures_delta.set {
            for d in delta {
                g.egui_renderer.update_texture(&g.device, &g.queue, *id, d);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [g.config.width, g.config.height], pixels_per_point: ppp };
        let extra = g.egui_renderer.update_buffers(&g.device, &g.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("frame"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(Renderer::sea_color()), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            g.map_renderer.draw(&mut pass);
            g.egui_renderer.render(&mut pass, &jobs, &screen);
        }
        g.queue.submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        g.window.pre_present_notify();
        g.queue.present(frame);
        for id in &full.textures_delta.free {
            g.egui_renderer.free_texture(id);
        }
        g.window.request_redraw();
        self.gfx = Some(g);
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> UiActions {
        let mut act = UiActions::default();
        let snap = &self.snapshot;
        egui::Panel::top("time").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(&snap.date);
                ui.label(format!("tick {}", snap.tick));
                ui.separator();
                let speed = self.session.speed();
                if ui.selectable_label(speed == Speed::Paused, "Pause").clicked() {
                    act.speed = Some(Speed::Paused);
                }
                if ui.button("Step").clicked() {
                    act.step = true;
                }
                for n in 1..=5u8 {
                    if ui.selectable_label(speed == Speed::Run(n), format!("{n}")).clicked() {
                        act.speed = Some(Speed::Run(n));
                    }
                }
                ui.separator();
                egui::ComboBox::from_id_salt("mode").selected_text(self.mode.name()).show_ui(ui, |ui| {
                    for m in Mode::ALL {
                        if ui.selectable_label(self.mode == m, m.name()).clicked() {
                            act.mode = Some(m);
                        }
                    }
                });
                ui.separator();
                ui.label(format!("{:.1} ms/frame, level {}, {:.0} km/px", self.frame_ms, self.gfx.as_ref().map_or(0, |g| g.map_renderer.level_in_use()), self.camera.m_per_px / 1000.0));
            });
        });
        egui::Panel::right("province").default_size(260.0).show(ui, |ui| {
            ui.heading("Province");
            let shown = if self.selected != NO_PROVINCE { self.selected } else { self.hover };
            if shown == NO_PROVINCE {
                ui.label("Click a province.");
                return;
            }
            let p = shown as usize;
            ui.label(if self.selected == NO_PROVINCE { "(hovering)" } else { "(selected)" });
            egui::Grid::new("fields").num_columns(2).show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(k);
                    ui.label(v);
                    ui.end_row();
                };
                let ctrl = snap.controllers.get(p).copied().unwrap_or(u16::MAX);
                row("id", shown.to_string());
                row("1990 state", self.map.state_names.get(self.map.province_state[p] as usize).cloned().unwrap_or_default());
                row("controller", if ctrl == u16::MAX { "unknown".into() } else { self.map.state_names.get(ctrl as usize).cloned().unwrap_or_default() });
                row("counter", snap.values.get(p).copied().unwrap_or(0).to_string());
                row("area", format!("{} km2", self.map.province_area_ha[p] / 100));
                row("population", self.map.province_pop[p].to_string());
                row("cells", self.map.cells_of(meridian_core::ProvinceId(shown)).len().to_string());
                let units = snap.units.iter().filter(|u| u.1 == shown).count();
                row("units here", units.to_string());
            });
        });
        act
    }
}

#[derive(Default)]
struct UiActions {
    mode: Option<Mode>,
    speed: Option<Speed>,
    step: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_none() {
            self.init_gfx(event_loop);
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let mut over_ui = false;
        if let Some(g) = self.gfx.as_mut() {
            let r = g.egui_state.on_window_event(&g.window, &event);
            over_ui = r.consumed || g.egui_ctx.is_pointer_over_egui();
            // egui took a click or scroll (a panel, a menu): the map must not also react to it
            if r.consumed && matches!(event, WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. }) {
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gfx.as_mut() {
                    g.config.width = size.width.max(1);
                    g.config.height = size.height.max(1);
                    g.surface.configure(&g.device, &g.config);
                }
                self.camera.resize(size.width as f64, size.height as f64);
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(start) = self.press {
                    let d = ((position.x - start.x).powi(2) + (position.y - start.y).powi(2)).sqrt();
                    if d > 4.0 {
                        self.dragging = true;
                    }
                    if self.dragging {
                        self.camera.pan_pixels(position.x - self.cursor.x, position.y - self.cursor.y);
                    }
                }
                self.cursor = position;
                self.hover = if over_ui { NO_PROVINCE } else { self.pick(position) };
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed if !over_ui => {
                    self.press = Some(self.cursor);
                    self.dragging = false;
                }
                ElementState::Released => {
                    if self.press.is_some() && !self.dragging {
                        self.selected = self.pick(self.cursor);
                    }
                    self.press = None;
                    self.dragging = false;
                }
                _ => {}
            },
            WindowEvent::MouseWheel { delta, .. } if !over_ui => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                self.camera.zoom_at(self.cursor.x, self.cursor.y, 1.15f64.powf(lines));
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !self.gfx.as_ref().is_some_and(|g| g.egui_ctx.egui_wants_keyboard_input()) => {
                self.key(&event.logical_key);
            }
            _ => {}
        }
    }
}
