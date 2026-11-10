//! The window: winit for the window and input, wgpu for the surface, egui for panels.
//! The map itself is drawn by `Renderer`; this file only wires input, the session and the UI.

use client::camera::{Bounds, Camera};
use client::modes::{self, Mode};
use client::render::{Renderer, NO_PROVINCE};
use client::session::Session;
use client::{CellPicker, Picker, Speed};
use meridian_core::{Command, CountryId, Map, UnitId};
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
    pub render: Arc<RenderData>,
    pub session: Session,
    /// Starting camera (centre x, centre y, metres per pixel); `None` shows the whole world.
    pub view: Option<(f64, f64, f64)>,
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
    cell_picker: CellPicker,
    render: Arc<RenderData>,
    bounds: Bounds,
    terrain: Vec<u8>,
    gfx: Option<Gfx>,
    camera: Camera,
    mode: Mode,
    colors_dirty: bool,
    shown_tick: Option<u64>,
    selected: u32,
    selected_cell: Option<u32>,
    hover: u32,
    cursor: PhysicalPosition<f64>,
    press: Option<PhysicalPosition<f64>>,
    dragging: bool,
    frame_ms: f32,
    last_frame: Instant,
    /// When the window opened: drives the water animation.
    started: Instant,
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
    show_labels: bool,
    show_relief: bool,
    show_details: bool,
    /// The simulation's permanent bridge places (cell pairs), and which of them each drawn bridge shows.
    bridge_edges: Vec<(u32, u32)>,
    bridge_links: Option<Vec<Option<u32>>>,
    ui_scale: f32,
}

const SAVE_PATH: &str = "saves/quicksave.mrsv";

pub fn run(inputs: Inputs) {
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let (x0, y0, x1, y1) = inputs.render.bounds();
    let bounds = Bounds::around(x0, y0, x1, y1);
    let level = inputs.render.levels.last().expect("render file has levels");
    let picker = Picker::new(level, bounds);
    let cell_picker = CellPicker::new(&inputs.render, bounds);
    let terrain = modes::dominant_terrain(&inputs.map);
    let mut app = App {
        map: inputs.map.clone(),
        session: Session::new(meridian_core::World::new(0, 1, 1), Box::new(proving_ground::rules(false)), CountryId(0)),
        picker,
        cell_picker,
        render: inputs.render.clone(),
        bounds,
        terrain,
        gfx: None,
        camera: Camera::fit(1600.0, 900.0, bounds),
        mode: Mode::Political,
        colors_dirty: true,
        shown_tick: None,
        selected: NO_PROVINCE,
        selected_cell: None,
        hover: NO_PROVINCE,
        cursor: PhysicalPosition::new(0.0, 0.0),
        press: None,
        dragging: false,
        frame_ms: 16.0,
        last_frame: Instant::now(),
        started: Instant::now(),
        snapshot: Default::default(),
        debug_frames: 0,
        debug_from_tick: 0,
        debug_at: Instant::now(),
        selected_unit: None,
        orders: Vec::new(),
        status: String::new(),
        show_settings: false,
        show_cells: true,
        show_labels: true,
        show_relief: true,
        show_details: true,
        bridge_edges: inputs.map.bridge_edges().iter().map(|&(a, b, _)| (a, b)).collect(),
        bridge_links: None,
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
        let mut map_renderer = Renderer::new(&device, &queue, format, &inputs.render, self.map.province_count() as usize);
        map_renderer.set_cities(&device, &client::labels::city_positions(&self.map, &inputs.render));
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, &*window, Some(window.scale_factor() as f32), window.theme(), Some(device.limits().max_texture_dimension_2d as usize));
        let egui_renderer = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        self.camera = Camera::fit(size.width.max(1) as f64, size.height.max(1) as f64, self.bounds);
        if let Some((cx, cy, m)) = inputs.view {
            self.camera.cx = cx;
            self.camera.cy = cy;
            self.camera.m_per_px = m;
            self.camera.clamp();
        }
        self.gfx = Some(Gfx { window, surface, device, queue, config, map_renderer, egui_ctx, egui_state, egui_renderer });
    }

    fn refresh_colors(&mut self) {
        let snap = self.session.refresh().clone();
        let changed = self.shown_tick != Some(snap.tick);
        if changed {
            // show each drawn bridge as the simulation has it: standing or destroyed
            if let Some(g) = &mut self.gfx {
                let links = self.bridge_links.get_or_insert_with(|| {
                    let sites: Vec<[i32; 2]> = g.map_renderer.bridge_sites().iter().map(|b| b.pos).collect();
                    client::bridges::link(&sites, &self.bridge_edges, &self.map.cell_x, &self.map.cell_y)
                });
                g.map_renderer.set_bridge_conditions(&client::bridges::conditions(links, &self.bridge_edges, &snap.bridges));
            }
        }
        if changed || self.colors_dirty {
            let colors = match self.mode {
                Mode::Political => modes::political(&snap.controllers),
                Mode::Value => modes::value_ramp(&snap.values),
                Mode::Terrain => modes::terrain(&self.terrain),
            };
            if let Some(g) = &mut self.gfx {
                g.map_renderer.land_mix = client::render::land_mix_for(self.mode);
                g.map_renderer.set_colors(&g.queue, &colors);
            }
            self.shown_tick = Some(snap.tick);
            self.colors_dirty = false;
        }
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
                self.session = Session::new(world, Box::new(proving_ground::rules(false)), CountryId(0));
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
        client::heights::to_screen(&self.camera, &self.render.relief, self.render.province_mx[p as usize] as f64, self.render.province_my[p as usize] as f64)
    }

    /// Screen position (physical pixels) of a cell's centre.
    fn cell_screen(&self, c: u32) -> (f64, f64) {
        client::heights::to_screen(&self.camera, &self.render.relief, self.render.cell_mx[c as usize] as f64, self.render.cell_my[c as usize] as f64)
    }

    /// Screen position of a unit's marker: the cell it stands in (or its province's centre when
    /// the cell is not known). Units sharing a place are fanned out sideways.
    fn unit_markers(&self) -> Vec<(u32, u16, (f64, f64))> {
        let mut seen: std::collections::BTreeMap<(bool, u32), u32> = Default::default();
        self.snapshot
            .units
            .iter()
            .enumerate()
            .map(|(i, &(id, owner, prov))| {
                let cell = self.snapshot.unit_cells.get(i).map_or(u32::MAX, |c| c.0);
                let in_cell = (cell as usize) < self.render.cell_mx.len();
                let k = seen.entry(if in_cell { (true, cell) } else { (false, prov) }).or_insert(0);
                let (x, y) = if in_cell { self.cell_screen(cell) } else { self.province_screen(prov) };
                // counters sharing a place stand in a row of four, then a new row below
                let ppp = self.gfx.as_ref().map_or(1.0, |g| g.window.scale_factor() * self.ui_scale as f64);
                let (dx, dy) = ((client::counters::WIDTH * COUNTER_SCALE + 3.0) as f64 * ppp, (client::counters::HEIGHT * COUNTER_SCALE + 10.0) as f64 * ppp);
                let pos = (x + (*k % 4) as f64 * dx, y + (*k / 4) as f64 * dy);
                *k += 1;
                (id, owner, pos)
            })
            .collect()
    }

    fn pick(&self, p: PhysicalPosition<f64>) -> u32 {
        let (x, y) = client::heights::to_world(&self.camera, &self.render.relief, p.x, p.y);
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
        g.map_renderer.show_relief = self.show_relief;
        g.map_renderer.show_details = self.show_details;
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
        let mut encoder = g.device.create_command_encoder(&Default::default());
        let time = self.started.elapsed().as_secs_f32();
        g.map_renderer.render(&g.device, &g.queue, &mut encoder, &view, &self.camera, self.selected, self.hover, time);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [g.config.width, g.config.height], pixels_per_point: ppp };
        let extra = g.egui_renderer.update_buffers(&g.device, &g.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
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
            ui.checkbox(&mut self.show_relief, "Shaded relief");
            ui.checkbox(&mut self.show_details, "Roads, railways and cities");
            ui.checkbox(&mut self.show_labels, "Place names");
            ui.add(egui::Slider::new(&mut self.ui_scale, 0.75..=2.0).text("Interface scale"));
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
            if let Some(c) = self.selected_cell {
                ui.separator();
                ui.heading("Cell");
                let c = c as usize;
                let terrain = ["plains", "hills", "mountains", "high mountains", "desert", "ice"];
                let flags = self.map.cell_flags[c];
                let mut f = Vec::new();
                for (bit, name) in [(1, "road"), (2, "rail"), (4, "river"), (8, "coast")] {
                    if flags & bit != 0 {
                        f.push(name);
                    }
                }
                egui::Grid::new("cell_fields").num_columns(2).show(ui, |ui| {
                    let mut row = |k: &str, v: String| {
                        ui.label(k);
                        ui.label(v);
                        ui.end_row();
                    };
                    row("id", c.to_string());
                    row("province", self.map.cell_province[c].to_string());
                    row("terrain", terrain.get(self.map.cell_terrain[c] as usize).copied().unwrap_or("?").to_string());
                    row("elevation", format!("{} m", self.map.cell_elev_m[c]));
                    row("relief", format!("{} m", self.map.cell_relief_m[c]));
                    row("area", format!("{} km2", self.map.cell_area_ha[c] / 100));
                    row("population", self.map.cell_pop[c].to_string());
                    row("built-up", format!("{} / 255", self.map.cell_urban[c]));
                    row("features", if f.is_empty() { "none".into() } else { f.join(", ") });
                });
            }
        });
        // the map area is what the panels left over: names and icons stay inside it
        self.draw_overlay(ui.ctx(), ui.available_rect_before_wrap());
        act
    }
}

/// egui's list of textures it no longer needs; free them once the frame that could use them is done.
/// A small building icon for a site, in the style of a printed map: dark ink on a light disc.
/// Kinds: 0 city, 1 port, 2 airfield, 3 crossing, 5 base (industry, 4, has no symbol).
fn draw_site_icon(p: &egui::Painter, kind: u8, c: egui::Pos2, population: u32) {
    use egui::{pos2, vec2, Color32, Shape, Stroke};
    let ink = Color32::from_rgb(35, 22, 12);
    let paper = Color32::from_rgb(250, 244, 226);
    let line = Stroke::new(1.3, ink);
    match kind {
        0 => {
            if population >= 1_000_000 {
                // skyline: three buildings of different heights on a light base
                let w = if population >= 5_000_000 { 4.0 } else { 3.2 };
                p.rect_filled(egui::Rect::from_min_max(pos2(c.x - 2.0 * w - 1.5, c.y - 3.6 * w), pos2(c.x + 2.0 * w + 1.5, c.y + 1.5)), 1.5, paper);
                for (dx, h) in [(-1.5, 2.0), (-0.5, 3.2), (0.5, 2.5)] {
                    let x0 = c.x + dx * w * 1.25;
                    p.rect_filled(egui::Rect::from_min_max(pos2(x0, c.y - h * w), pos2(x0 + w, c.y)), 0.0, ink);
                }
            } else {
                let r = if population >= 300_000 { 3.4 } else { 2.6 };
                p.circle(c, r + 0.4, paper, Stroke::new(1.4, ink));
                p.circle_filled(c, r * 0.4, ink);
            }
        }
        1 => {
            // anchor
            p.circle_filled(c, 6.5, paper);
            p.line_segment([c + vec2(0.0, -4.5), c + vec2(0.0, 4.0)], line);
            p.line_segment([c + vec2(-2.5, -2.5), c + vec2(2.5, -2.5)], line);
            p.add(Shape::line(vec![c + vec2(-4.0, 1.0), c + vec2(-3.0, 3.5), c + vec2(0.0, 4.5), c + vec2(3.0, 3.5), c + vec2(4.0, 1.0)], Stroke::new(1.3, Color32::from_rgb(30, 60, 110))));
            p.circle_stroke(c + vec2(0.0, -5.0), 1.2, Stroke::new(1.0, ink));
        }
        2 => {
            // aeroplane seen from above
            p.circle_filled(c, 6.5, paper);
            p.line_segment([c + vec2(0.0, -5.0), c + vec2(0.0, 4.5)], Stroke::new(1.6, ink));
            p.add(Shape::convex_polygon(vec![c + vec2(0.0, -1.5), c + vec2(5.0, 1.0), c + vec2(-5.0, 1.0)], ink, Stroke::NONE));
            p.line_segment([c + vec2(-2.2, 4.2), c + vec2(2.2, 4.2)], Stroke::new(1.4, ink));
        }
        3 => {
            // bridge: deck and arch
            p.circle_filled(c, 6.0, paper);
            p.line_segment([c + vec2(-5.0, 1.5), c + vec2(5.0, 1.5)], Stroke::new(1.6, ink));
            p.add(Shape::line((0..=8).map(|i| { let t = i as f32 / 8.0 * std::f32::consts::PI; c + vec2(-4.0 * t.cos(), 1.5 - 4.0 * t.sin()) }).collect(), line));
        }
        _ => {
            // military base: a flag on a pole
            p.circle_filled(c, 6.5, paper);
            p.line_segment([c + vec2(-2.5, -5.0), c + vec2(-2.5, 5.0)], Stroke::new(1.4, ink));
            p.add(Shape::convex_polygon(vec![c + vec2(-2.5, -5.0), c + vec2(4.5, -3.0), c + vec2(-2.5, -1.0)], Color32::from_rgb(150, 40, 30), Stroke::NONE));
        }
    }
}

fn free_textures(renderer: &mut egui_wgpu::Renderer, delta: &egui::TexturesDelta) {
    for id in &delta.free {
        renderer.free_texture(id);
    }
}

impl App {
    /// Unit markers and order arrows, painted over the map and under the panels.
    fn draw_overlay(&self, ctx: &egui::Context, map_rect: egui::Rect) {
        let ppp = ctx.pixels_per_point() as f64;
        let to_pt = |p: (f64, f64)| egui::pos2((p.0 / ppp) as f32, (p.1 / ppp) as f32);
        let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(map_rect);
        if let Some(c) = self.selected_cell {
            let pts: Vec<egui::Pos2> = self.render.cell_ring(c as usize).map(|(x, y)| to_pt(client::heights::to_screen(&self.camera, &self.render.relief, x as f64, y as f64))).collect();
            painter.add(egui::Shape::closed_line(pts.clone(), egui::Stroke::new(4.0, egui::Color32::WHITE)));
            painter.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.5, egui::Color32::BLACK)));
        }
        if self.show_labels {
            let placed = client::labels::place(&self.render, &self.camera, ppp);
            for m in client::labels::place_sites(&self.map, &self.render, &self.camera, ppp, &placed) {
                let pos = to_pt((m.x, m.y));
                draw_site_icon(&painter, m.kind, pos, m.population);
                if let Some(name) = &m.name {
                    let font = egui::FontId::proportional((m.size_px / ppp) as f32);
                    let at = pos + egui::vec2(8.0, 0.0);
                    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                        painter.text(at + egui::vec2(dx, dy), egui::Align2::LEFT_CENTER, name, font.clone(), egui::Color32::from_rgba_unmultiplied(255, 248, 225, 230));
                    }
                    painter.text(at, egui::Align2::LEFT_CENTER, name, font, egui::Color32::from_rgb(35, 22, 12));
                }
            }
            for l in placed {
                let pos = to_pt((l.x, l.y));
                let font = egui::FontId::proportional((l.size_px / ppp) as f32);
                let (ink, halo) = match l.kind {
                    client::labels::LabelKind::Country => (egui::Color32::from_rgb(35, 22, 12), egui::Color32::from_rgba_unmultiplied(255, 248, 225, 235)),
                    client::labels::LabelKind::Region => (egui::Color32::from_rgb(40, 28, 18), egui::Color32::from_rgba_unmultiplied(255, 248, 225, 225)),
                };
                let (ink, halo) = (ink.gamma_multiply(l.alpha as f32), halo.gamma_multiply(l.alpha as f32));
                for (dx, dy) in [(-1.3, 0.0), (1.3, 0.0), (0.0, -1.3), (0.0, 1.3), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    painter.text(pos + egui::vec2(dx, dy), egui::Align2::CENTER_CENTER, &l.text, font.clone(), halo);
                }
                painter.text(pos, egui::Align2::CENTER_CENTER, &l.text, font, ink);
            }
        }
        let markers = self.unit_markers();
        // arrows for orders that have not finished
        // (marches as the simulation has them: from each marching unit to the cell it is heading for)
        let marching: Vec<(u32, u32)> = self.snapshot.units.iter().zip(&self.snapshot.unit_cells).filter(|(_, c)| (c.1 as usize) < self.render.cell_mx.len()).map(|(u, c)| (u.0, c.1)).collect();
        for &(unit, to) in &marching {
            if let Some(&(_, _, from)) = markers.iter().find(|m| m.0 == unit) {
                let (a, b) = (to_pt(from), to_pt(self.cell_screen(to)));
                let stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(20, 20, 20));
                painter.line_segment([a, b], stroke);
                let d = (b - a).normalized();
                let n = egui::vec2(-d.y, d.x);
                painter.add(egui::Shape::convex_polygon(vec![b, b - d * 12.0 + n * 5.0, b - d * 12.0 - n * 5.0], egui::Color32::from_rgb(20, 20, 20), egui::Stroke::NONE));
            }
        }
        // unit counters in the NATO style: the type inside the frame, the size above it
        for (id, owner, pos) in markers {
            let c = modes::country_color(owner);
            let selected = self.selected_unit == Some((id, owner));
            let (kind, size) = self.snapshot.units.iter().position(|u| u.0 == id).and_then(|i| self.snapshot.unit_kinds.get(i).copied()).unwrap_or((0, 0));
            client::counters::draw(&painter, to_pt(pos), COUNTER_SCALE, kind, size, [c[0], c[1], c[2]], selected);
        }
    }
}

/// Size of the unit counters on screen (1 = 30 by 20 points).
const COUNTER_SCALE: f32 = 0.9;

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
                        let hit = self.unit_markers().into_iter().find(|m| (m.2 .0 - self.cursor.x).abs() < (client::counters::WIDTH * COUNTER_SCALE * 0.5) as f64 * k && (m.2 .1 - self.cursor.y).abs() < (client::counters::HEIGHT * COUNTER_SCALE * 0.5) as f64 * k);
                        match hit {
                            Some((id, owner, _)) => self.selected_unit = Some((id, owner)),
                            None => {
                                self.selected = self.pick(self.cursor);
                                let (x, y) = client::heights::to_world(&self.camera, &self.render.relief, self.cursor.x, self.cursor.y);
                                self.selected_cell = self.cell_picker.cell_at(&self.render, x, y);
                            }
                        }
                    }
                    self.press = None;
                    self.dragging = false;
                }
                _ => {}
            },
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } if !over_ui => {
                // units move from cell to cell: the order names the cell under the cursor, and the
                // simulation plans the route (roads, rivers, bridges) and marches the unit along it
                let (x, y) = client::heights::to_world(&self.camera, &self.render.relief, self.cursor.x, self.cursor.y);
                if let (Some((unit, owner)), Some(to)) = (self.selected_unit, self.cell_picker.cell_at(&self.render, x, y)) {
                    self.session.issue(Command::MoveToCell { issuer: CountryId(owner), unit: UnitId(unit), to: meridian_core::CellId(to) });
                    self.status = format!("ordered unit {unit} to cell {to}");
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
