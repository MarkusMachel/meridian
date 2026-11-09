//! The window: winit for the window and input, wgpu for the surface, egui for panels.
//! The map itself is drawn by `Renderer`; this file only wires input, the session and the UI.

use client::camera::Camera;
use client::modes::{self, Mode};
use client::render::{Renderer, NO_PROVINCE};
use client::session::Session;
use client::{Picker, Speed};
use meridian_core::{Command, CountryId, Map, ProvinceId, UnitId};
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
    /// Diagnostics (MERIDIAN_DEBUG): frames and ticks since the last report, and when it was printed.
    debug_frames: u32,
    debug_from_tick: u64,
    debug_at: Instant,
    /// The unit that right-click orders go to: (unit id, owner).
    selected_unit: Option<(u32, u16)>,
    /// Move orders given and not yet finished: (unit id, destination province).
    orders: Vec<(u32, u32)>,
    status: String,
    show_settings: bool,
    show_cells: bool,
    ui_scale: f32,
}

const SAVE_PATH: &str = "saves/quicksave.mrsv";

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
        debug_frames: 0,
        debug_from_tick: 0,
        debug_at: Instant::now(),
        selected_unit: None,
        orders: Vec::new(),
        status: String::new(),
        show_settings: false,
        show_cells: true,
        ui_scale: 1.0,
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
        self.orders.retain(|&(u, to)| snap.units.iter().any(|x| x.0 == u && x.2 != to));
        self.snapshot = snap;
    }

    fn save(&mut self) {
        self.status = match self.session.save_bytes() {
            None => "save failed: the simulation thread has ended".into(),
            Some(bytes) => match std::fs::create_dir_all("saves").and_then(|_| std::fs::write(SAVE_PATH, &bytes)) {
                Ok(()) => format!("saved {} ({} bytes) at tick {}", SAVE_PATH, bytes.len(), self.snapshot.tick),
                Err(e) => format!("save failed: {e}"),
            },
        };
    }

    fn load(&mut self) {
        let bytes = match std::fs::read(SAVE_PATH) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("load failed: {SAVE_PATH}: {e}");
                return;
            }
        };
        match meridian_data::load_world(SAVE_PATH, &bytes, Some(self.map.clone())) {
            Ok(world) => {
                self.session = Session::new(world, Box::new(proving_ground::schedule(false)), CountryId(0));
                self.orders.clear();
                self.selected_unit = None;
                self.shown_tick = None;
                self.status = format!("loaded {SAVE_PATH}");
            }
            Err(e) => self.status = format!("load failed: {e}"),
        }
    }

    /// Screen position (physical pixels) of a province's centre.
    fn province_screen(&self, p: u32) -> (f64, f64) {
        self.camera.world_to_screen(self.map.province_x[p as usize] as f64, self.map.province_y[p as usize] as f64)
    }

    /// Screen position of a unit's marker. Units sharing a province are fanned out sideways.
    fn unit_markers(&self) -> Vec<(u32, u16, (f64, f64))> {
        let mut seen: std::collections::BTreeMap<u32, u32> = Default::default();
        self.snapshot
            .units
            .iter()
            .map(|&(id, owner, prov)| {
                let k = seen.entry(prov).or_insert(0);
                let (x, y) = self.province_screen(prov);
                let pos = (x + *k as f64 * 14.0, y);
                *k += 1;
                (id, owner, pos)
            })
            .collect()
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
        let dt = now.duration_since(self.last_frame).as_secs_f64();
        self.frame_ms = self.frame_ms * 0.9 + dt as f32 * 1000.0 * 0.1;
        self.last_frame = now;
        self.session.advance(dt.min(0.25));
        self.refresh_colors();
        if std::env::var_os("MERIDIAN_DEBUG").is_some() {
            self.debug_frames += 1;
            if self.debug_at.elapsed().as_secs_f64() >= 1.0 {
                eprintln!("{:?}: {} frames, {} ticks in the last second", self.session.speed(), self.debug_frames, self.snapshot.tick - self.debug_from_tick);
                self.debug_frames = 0;
                self.debug_from_tick = self.snapshot.tick;
                self.debug_at = Instant::now();
            }
        }
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
        if ui_actions.pick_unit.is_some() {
            self.selected_unit = ui_actions.pick_unit;
        }
        if ui_actions.save {
            self.save();
        }
        if ui_actions.load {
            self.load();
        }
        g.egui_ctx.set_pixels_per_point(g.window.scale_factor() as f32 * self.ui_scale);
        g.map_renderer.show_cells = self.show_cells;
        let ppp = full.pixels_per_point;
        let jobs = g.egui_ctx.tessellate(full.shapes, ppp);

        // egui sends each texture (the font atlas above all) exactly once, so upload it now, before
        // anything can make us skip the frame; skipping first would leave egui with no font for good.
        for (id, delta) in &full.textures_delta.set {
            for d in delta {
                g.egui_renderer.update_texture(&g.device, &g.queue, *id, d);
            }
        }

        // --- surface ---
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                g.surface.configure(&g.device, &g.config);
                g.window.request_redraw();
                free_textures(&mut g.egui_renderer, &full.textures_delta);
                self.gfx = Some(g);
                return;
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                // The window is hidden (another app in front, minimised). Stop drawing; winit tells us
                // when it is visible again (`WindowEvent::Occluded(false)`) and we ask for a frame then.
                free_textures(&mut g.egui_renderer, &full.textures_delta);
                self.gfx = Some(g);
                return;
            }
            _ => {
                // Timeout or a one-off error: skip this frame but keep the loop going.
                g.window.request_redraw();
                free_textures(&mut g.egui_renderer, &full.textures_delta);
                self.gfx = Some(g);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        g.map_renderer.prepare(&g.queue, &self.camera, self.selected, self.hover);
        let mut encoder = g.device.create_command_encoder(&Default::default());
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
        free_textures(&mut g.egui_renderer, &full.textures_delta);
        g.window.request_redraw();
        self.gfx = Some(g);
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> UiActions {
        let mut act = UiActions::default();
        let snap = &self.snapshot;
        egui::Panel::top("time").show(ui, |ui| {
            // The menu bar is itself a horizontal row that takes all the width it is given, so the
            // rest of the bar must live inside it (next to it, it would be pushed out of view).
            egui::MenuBar::new().ui(ui, |ui| {
                {
                    ui.menu_button("Game", |ui| {
                        if ui.button("Save").clicked() {
                            act.save = true;
                            ui.close();
                        }
                        if ui.button("Load").clicked() {
                            act.load = true;
                            ui.close();
                        }
                        if ui.button("Settings").clicked() {
                            self.show_settings = true;
                            ui.close();
                        }
                    });
                }
                ui.separator();
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
                    let tip = format!("{} game hours per second", Speed::Run(n).ticks_per_second());
                    if ui.selectable_label(speed == Speed::Run(n), format!("{n}")).on_hover_text(tip).clicked() {
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
        if !self.status.is_empty() {
            egui::Panel::bottom("status").show(ui, |ui| {
                ui.label(&self.status);
            });
        }
        egui::Window::new("Settings").open(&mut self.show_settings).show(ui.ctx(), |ui| {
            ui.checkbox(&mut self.show_cells, "Show cell edges when zoomed in");
            ui.add(egui::Slider::new(&mut self.ui_scale, 0.75..=2.0).text("Interface scale"));
        });
        self.draw_overlay(ui.ctx());
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
                row("units here", snap.units.iter().filter(|u| u.2 == shown).count().to_string());
            });
            for &(id, owner, _) in snap.units.iter().filter(|u| u.2 == shown) {
                let name = self.map.state_names.get(owner as usize).cloned().unwrap_or_default();
                if ui.selectable_label(self.selected_unit == Some((id, owner)), format!("unit {id} ({name})")).clicked() {
                    act.pick_unit = Some((id, owner));
                }
            }
            if self.selected_unit.is_some() {
                ui.label("Right-click a province to order the unit there.");
            }
        });
        act
    }
}

/// egui's list of textures it no longer needs; free them once the frame that could use them is done.
fn free_textures(renderer: &mut egui_wgpu::Renderer, delta: &egui::TexturesDelta) {
    for id in &delta.free {
        renderer.free_texture(id);
    }
}

impl App {
    /// Unit markers and order arrows, painted over the map and under the panels.
    fn draw_overlay(&self, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point() as f64;
        let to_pt = |p: (f64, f64)| egui::pos2((p.0 / ppp) as f32, (p.1 / ppp) as f32);
        let painter = ctx.layer_painter(egui::LayerId::background());
        let markers = self.unit_markers();
        // arrows for orders that have not finished
        for &(unit, to) in &self.orders {
            if let Some(&(_, _, from)) = markers.iter().find(|m| m.0 == unit) {
                let (a, b) = (to_pt(from), to_pt(self.province_screen(to)));
                let stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(20, 20, 20));
                painter.line_segment([a, b], stroke);
                let d = (b - a).normalized();
                let n = egui::vec2(-d.y, d.x);
                painter.add(egui::Shape::convex_polygon(vec![b, b - d * 12.0 + n * 5.0, b - d * 12.0 - n * 5.0], egui::Color32::from_rgb(20, 20, 20), egui::Stroke::NONE));
            }
        }
        for (id, owner, pos) in markers {
            let c = modes::country_color(owner);
            let r = egui::Rect::from_center_size(to_pt(pos), egui::vec2(12.0, 12.0));
            let selected = self.selected_unit == Some((id, owner));
            painter.rect_filled(r, 1.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
            let stroke = if selected { egui::Stroke::new(3.0, egui::Color32::WHITE) } else { egui::Stroke::new(1.5, egui::Color32::BLACK) };
            painter.rect_stroke(r, 1.0, stroke, egui::StrokeKind::Outside);
        }
    }
}

#[derive(Default)]
struct UiActions {
    mode: Option<Mode>,
    speed: Option<Speed>,
    step: bool,
    save: bool,
    load: bool,
    pick_unit: Option<(u32, u16)>,
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
            WindowEvent::Occluded(false) => {
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
            }
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
                        let k = self.gfx.as_ref().map_or(1.0, |g| g.window.scale_factor() * self.ui_scale as f64);
                        let hit = self.unit_markers().into_iter().find(|m| (m.2 .0 - self.cursor.x).abs() < 8.0 * k && (m.2 .1 - self.cursor.y).abs() < 8.0 * k);
                        match hit {
                            Some((id, owner, _)) => self.selected_unit = Some((id, owner)),
                            None => self.selected = self.pick(self.cursor),
                        }
                    }
                    self.press = None;
                    self.dragging = false;
                }
                _ => {}
            },
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } if !over_ui => {
                let to = self.pick(self.cursor);
                if let (Some((unit, owner)), true) = (self.selected_unit, to != NO_PROVINCE) {
                    self.session.issue(Command::MoveUnit { issuer: CountryId(owner), unit: UnitId(unit), to: ProvinceId(to) });
                    self.orders.retain(|o| o.0 != unit);
                    self.orders.push((unit, to));
                    self.status = format!("ordered unit {unit} to province {to}");
                }
            }
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
