//! The globe window (map rewrite M3): the same game shell as `app.rs`, on the hexagon world.
//! winit for the window and input, wgpu for the surface, egui for panels; the globe itself is
//! drawn by `client::globe`. This file only wires input, the session and the UI.

use client::globe::{GlobeCamera, GlobeRenderer, HexIndex, V3};
use client::modes::{self, Mode};
use client::session::Session;
use client::Speed;
use meridian_core::{Command, CountryId, Map, UnitId};
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
    pub session: Session,
    /// Starting view (latitude, longitude, kilometres across); `None` shows the whole globe.
    pub view: Option<(f64, f64, f64)>,
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    map_renderer: GlobeRenderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

struct App {
    view: Option<(f64, f64, f64)>,
    map: Arc<Map>,
    index: HexIndex,
    session: Session,
    terrain: Vec<u8>,
    gfx: Option<Gfx>,
    camera: GlobeCamera,
    mode: Mode,
    colors_dirty: bool,
    shown_tick: Option<u64>,
    selected_cell: Option<u32>,
    hover_cell: Option<u32>,
    cursor: PhysicalPosition<f64>,
    press: Option<PhysicalPosition<f64>>,
    dragging: bool,
    frame_ms: f32,
    last_frame: Instant,
    snapshot: client::Snapshot,
    /// The unit that right-click orders go to: (unit id, owner).
    selected_unit: Option<(u32, u16)>,
    status: String,
    show_settings: bool,
    show_labels: bool,
    show_roads: bool,
    ui_scale: f32,
    /// Tiles drawn and tiles still loading in the last frame.
    tiles_drawn: (usize, usize),
    /// City sites, largest first: (site, population).
    cities: Vec<(u32, u32)>,
}

/// Countries are coloured with the ground shaded in, the debug values plainly, and the
/// terrain mode by each hexagon's own terrain.
pub fn coloring_for(mode: Mode) -> client::globe::render::Coloring {
    use client::globe::render::Coloring;
    match mode {
        Mode::Political => Coloring::PaletteShaded,
        Mode::Value => Coloring::Palette,
        Mode::Terrain => Coloring::Terrain,
    }
}

const SAVE_PATH: &str = "saves/quicksave.mrsv";
/// Looking at Europe when the window opens.
const START: (f64, f64) = (48.0, 12.0);

pub fn run(inputs: Inputs) {
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let map = inputs.map.clone();
    let mut cities: Vec<(u32, u32)> = (0..map.site_kind.len() as u32).filter(|&s| map.site_kind[s as usize] == 0).map(|s| (s, map.site_size[s as usize])).collect();
    cities.sort_by_key(|c| (std::cmp::Reverse(c.1), c.0));
    let mut app = App {
        view: inputs.view,
        index: HexIndex::new(&map),
        terrain: modes::dominant_terrain(&map),
        map,
        session: inputs.session,
        gfx: None,
        camera: GlobeCamera::whole_globe(1600.0, 900.0, START.0, START.1),
        mode: Mode::Political,
        colors_dirty: true,
        shown_tick: None,
        selected_cell: None,
        hover_cell: None,
        cursor: PhysicalPosition::new(0.0, 0.0),
        press: None,
        dragging: false,
        frame_ms: 16.0,
        last_frame: Instant::now(),
        snapshot: Default::default(),
        selected_unit: None,
        status: String::new(),
        show_settings: false,
        show_labels: true,
        show_roads: true,
        ui_scale: 1.0,
        tiles_drawn: (0, 0),
        cities,
    };
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

        let map_renderer = GlobeRenderer::new(&device, &queue, format, &self.map, &self.index);
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, &*window, Some(window.scale_factor() as f32), window.theme(), Some(device.limits().max_texture_dimension_2d as usize));
        let egui_renderer = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let (w, h) = (size.width.max(1) as f64, size.height.max(1) as f64);
        self.camera = match self.view {
            Some((lat, lon, km)) => GlobeCamera::looking_at(w, h, lat, lon, km),
            None => GlobeCamera::whole_globe(w, h, START.0, START.1),
        };
        self.gfx = Some(Gfx { window, surface, device, queue, config, map_renderer, egui_ctx, egui_state, egui_renderer });
    }

    fn refresh_colors(&mut self) {
        let snap = self.session.refresh().clone();
        if self.shown_tick != Some(snap.tick) || self.colors_dirty {
            let colors = match self.mode {
                Mode::Political => modes::political(&snap.controllers),
                Mode::Value => modes::value_ramp(&snap.values),
                Mode::Terrain => modes::terrain(&self.terrain),
            };
            if let Some(g) = &mut self.gfx {
                g.map_renderer.set_colors(&g.queue, &colors);
                g.map_renderer.coloring = coloring_for(self.mode);
                g.map_renderer.show_roads = self.show_roads;
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
                self.selected_unit = None;
                self.shown_tick = None;
                self.status = format!("loaded {SAVE_PATH}");
            }
            Err(e) => self.status = format!("load failed: {e}"),
        }
    }

    /// Where a point on the ground shows on screen (physical pixels), or `None` if it is over
    /// the horizon.
    fn ground_screen(&self, place: [i32; 3]) -> Option<(f64, f64)> {
        let d: V3 = place.map(|v| v as f64);
        self.camera.dir_to_screen(d)
    }

    /// Screen position of a cell's centre.
    fn cell_screen(&self, c: u32) -> Option<(f64, f64)> {
        self.ground_screen(self.map.cell_place(meridian_core::CellId(c))?)
    }

    /// Screen position of a unit: the cell it stands in, or its province's centre when the
    /// cell is not known. `i` is its place in the snapshot's unit list.
    fn unit_screen(&self, i: usize) -> Option<(f64, f64)> {
        let cell = self.snapshot.unit_cells.get(i).map_or(u32::MAX, |c| c.0);
        if cell < self.map.cell_count() {
            return self.cell_screen(cell);
        }
        let p = self.snapshot.units[i].2 as usize;
        self.ground_screen([self.map.node_xyz[3 * p], self.map.node_xyz[3 * p + 1], self.map.node_xyz[3 * p + 2]])
    }

    /// What to call a unit in lists: its name if it has one, else its type and size, with its country.
    fn unit_title(&self, i: usize) -> String {
        let (id, owner, _) = self.snapshot.units[i];
        let country = self.map.state_names.get(owner as usize).cloned().unwrap_or_default();
        let name = self.snapshot.unit_command.get(i).map_or("", |c| c.0.as_str());
        if !name.is_empty() {
            return format!("{name} ({country})");
        }
        let (kind, size) = self.snapshot.unit_kinds.get(i).copied().unwrap_or((0, 0));
        let kinds = ["unit", "infantry", "armour", "mechanised", "motorised", "artillery", "SP artillery", "recon", "armoured recon", "engineer", "air defence", "anti-tank", "airborne", "mountain", "supply", "headquarters"];
        let sizes = ["", "battalion", "regiment", "brigade", "division", "corps"];
        format!("{} {} {id} ({country})", kinds.get(kind as usize).unwrap_or(&"unit"), sizes.get(size as usize).unwrap_or(&"")).replace("  ", " ")
    }

    /// The unit counters to draw: units of one country that would overlap on screen share a
    /// stack (so zooming out gathers them and zooming in spreads them), the most senior on top.
    /// Each stack is the places of its units in the snapshot's unit list (top first) and where
    /// it stands in physical pixels. Units off the screen or over the horizon are left out.
    fn unit_stacks(&self) -> Vec<(Vec<usize>, (f64, f64))> {
        let ppp = self.gfx.as_ref().map_or(1.0, |g| g.window.scale_factor() * self.ui_scale as f64);
        let (w, h) = (client::counters::WIDTH as f64 * COUNTER_SCALE as f64 * ppp, client::counters::HEIGHT as f64 * COUNTER_SCALE as f64 * ppp);
        let mut shown: Vec<usize> = Vec::new();
        let mut marks: Vec<(f32, f32, u16, u8)> = Vec::new();
        for (i, u) in self.snapshot.units.iter().enumerate() {
            let Some((x, y)) = self.unit_screen(i) else { continue };
            if x < -w || y < -h || x > self.camera.width_px + w || y > self.camera.height_px + h {
                continue;
            }
            let (kind, size) = self.snapshot.unit_kinds.get(i).copied().unwrap_or((0, 0));
            // seniority: size first, and a headquarters above other units of its size
            let rank = size * 2 + u8::from(kind == proving_ground::unit_kind::HEADQUARTERS);
            shown.push(i);
            marks.push((x as f32, y as f32, u.1, rank));
        }
        client::counters::stacks(&marks, (w as f32 * 0.9, h as f32 * 0.9))
            .into_iter()
            .map(|s| {
                let top = s[0];
                (s.into_iter().map(|k| shown[k]).collect(), (marks[top].0 as f64, marks[top].1 as f64))
            })
            .collect()
    }

    /// The land hexagon under a pixel.
    fn pick(&self, p: PhysicalPosition<f64>) -> Option<u32> {
        self.index.cell_at(self.camera.screen_to_dir(p.x, p.y)?).map(|c| c.0)
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
        g.map_renderer.render(&g.device, &g.queue, &mut encoder, &view, &self.camera, &self.map, &self.index, self.selected_cell, false);
        self.tiles_drawn = g.map_renderer.drawn;
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
                let (drawn, waiting) = self.tiles_drawn;
                ui.label(format!("{:.1} ms/frame, {:.2} km/px, {drawn} tiles{}", self.frame_ms, self.camera.m_per_px() / 1000.0, if waiting > 0 { format!(" ({waiting} loading)") } else { String::new() }));
            });
        });
        if !self.status.is_empty() {
            egui::Panel::bottom("status").show(ui, |ui| {
                ui.label(&self.status);
            });
        }
        egui::Window::new("Settings").open(&mut self.show_settings).show(ui.ctx(), |ui| {
            ui.checkbox(&mut self.show_labels, "City names");
            if ui.checkbox(&mut self.show_roads, "Roads, railways and bridges").changed() {
                self.colors_dirty = true;
            }
            ui.add(egui::Slider::new(&mut self.ui_scale, 0.75..=2.0).text("Interface scale"));
        });
        egui::Panel::right("province").default_size(260.0).show(ui, |ui| {
            ui.heading("Province");
            let Some(cell) = self.selected_cell.or(self.hover_cell) else {
                ui.label("Click a hexagon.");
                return;
            };
            let shown = self.map.cell_province[cell as usize];
            let p = shown as usize;
            ui.label(if self.selected_cell.is_none() { "(hovering)" } else { "(selected)" });
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
                row("hexagons", self.map.cells_of(meridian_core::ProvinceId(shown)).len().to_string());
                row("units here", snap.units.iter().filter(|u| u.2 == shown).count().to_string());
            });
            for (i, &(id, owner, _)) in snap.units.iter().enumerate().filter(|(_, u)| u.2 == shown) {
                if ui.selectable_label(self.selected_unit == Some((id, owner)), self.unit_title(i)).clicked() {
                    act.pick_unit = Some((id, owner));
                }
            }
            if let Some(sel) = self.selected_unit.and_then(|(id, _)| snap.units.iter().position(|u| u.0 == id)) {
                ui.separator();
                ui.heading("Unit");
                ui.strong(self.unit_title(sel));
                // the chain of command: who it reports to, then who reports to it
                let index_of = |id: u32| snap.units.iter().position(|u| u.0 == id);
                let mut up = snap.unit_command.get(sel).and_then(|c| index_of(c.1));
                let mut depth = 0;
                while let Some(i) = up.filter(|_| depth < 8) {
                    if ui.link(format!("reports to: {}", self.unit_title(i))).clicked() {
                        act.pick_unit = Some((snap.units[i].0, snap.units[i].1));
                    }
                    up = snap.unit_command.get(i).and_then(|c| index_of(c.1));
                    depth += 1;
                }
                let under: Vec<usize> = snap.unit_command.iter().enumerate().filter(|(_, c)| c.1 == snap.units[sel].0).map(|(i, _)| i).collect();
                if !under.is_empty() {
                    ui.label("under its command:");
                    for i in under {
                        if ui.link(format!("  {}", self.unit_title(i))).clicked() {
                            act.pick_unit = Some((snap.units[i].0, snap.units[i].1));
                        }
                    }
                }
                ui.label("Right-click a hexagon to march the unit there.");
            }
            if let Some(c) = self.selected_cell {
                ui.separator();
                ui.heading("Hexagon");
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
    /// City names, unit counters and order arrows, painted over the map and under the panels.
    fn draw_overlay(&self, ctx: &egui::Context, map_rect: egui::Rect) {
        let ppp = ctx.pixels_per_point() as f64;
        let to_pt = |p: (f64, f64)| egui::pos2((p.0 / ppp) as f32, (p.1 / ppp) as f32);
        let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(map_rect);
        if self.show_labels {
            // the closer the view, the smaller the towns that get a name; never more than 60 at once
            let m = self.camera.m_per_px();
            let smallest = if m > 6_000.0 { 3_000_000 } else if m > 2_500.0 { 1_000_000 } else if m > 900.0 { 300_000 } else if m > 300.0 { 100_000 } else { 0 };
            let mut taken: Vec<egui::Rect> = Vec::new();
            for &(site, population) in self.cities.iter().take_while(|c| c.1 >= smallest) {
                if taken.len() >= 60 {
                    break;
                }
                let Some(at) = self.cell_screen(self.map.site_cell[site as usize]) else { continue };
                let pos = to_pt(at);
                if !map_rect.contains(pos) {
                    continue;
                }
                let name = &self.map.site_names[site as usize];
                let size = if population >= 3_000_000 { 15.0 } else if population >= 500_000 { 13.0 } else { 11.5 };
                let font = egui::FontId::proportional(size);
                let anchor = pos + egui::vec2(8.0, 0.0);
                let space = egui::Rect::from_min_size(anchor - egui::vec2(14.0, size * 0.6), egui::vec2(name.len() as f32 * size * 0.6 + 14.0, size * 1.2));
                if taken.iter().any(|r| r.intersects(space)) {
                    continue; // a bigger town already has this spot
                }
                taken.push(space);
                draw_site_icon(&painter, 0, pos, population);
                for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                    painter.text(anchor + egui::vec2(dx, dy), egui::Align2::LEFT_CENTER, name, font.clone(), egui::Color32::from_rgba_unmultiplied(255, 248, 225, 230));
                }
                painter.text(anchor, egui::Align2::LEFT_CENTER, name, font, egui::Color32::from_rgb(35, 22, 12));
            }
        }
        let stacks = self.unit_stacks();
        // where a unit's counter is on screen: its stack if it is shown, else its own place
        let place_of = |i: usize| stacks.iter().find(|s| s.0.contains(&i)).map(|s| s.1).or_else(|| self.unit_screen(i));
        let index_of = |id: u32| self.snapshot.units.iter().position(|u| u.0 == id);
        let ink = egui::Color32::from_rgb(20, 20, 20);
        // marches as the simulation has them: the route each marching unit has still to go,
        // hexagon by hexagon, with an arrowhead at its destination
        for (i, route) in self.snapshot.unit_routes.iter().enumerate() {
            if route.is_empty() {
                continue;
            }
            let Some(start) = place_of(i) else { continue };
            let mut pts = vec![to_pt(start)];
            pts.extend(route.iter().filter_map(|&c| self.cell_screen(c)).map(to_pt));
            let (Some(&b), Some(&a)) = (pts.last(), pts.iter().rev().find(|p| (**p - *pts.last().unwrap()).length() > 6.0)) else { continue };
            if !pts.iter().any(|p| map_rect.contains(*p)) {
                continue;
            }
            painter.add(egui::Shape::line(pts, egui::Stroke::new(2.0, ink)));
            let d = (b - a).normalized();
            let n = egui::vec2(-d.y, d.x);
            painter.add(egui::Shape::convex_polygon(vec![b, b - d * 12.0 + n * 5.0, b - d * 12.0 - n * 5.0], ink, egui::Stroke::NONE));
        }
        // destroyed bridges: a red cross on the river where the bridge was (zoomed in only)
        if self.camera.m_per_px() < 450.0 {
            let red = egui::Stroke::new(2.5, egui::Color32::from_rgb(200, 30, 30));
            for &(a, b, condition) in &self.snapshot.bridges {
                if condition > 0 {
                    continue;
                }
                let (Some(pa), Some(pb)) = (self.cell_screen(a), self.cell_screen(b)) else { continue };
                let mid = to_pt(((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0));
                if map_rect.contains(mid) {
                    painter.line_segment([mid + egui::vec2(-6.0, -6.0), mid + egui::vec2(6.0, 6.0)], red);
                    painter.line_segment([mid + egui::vec2(-6.0, 6.0), mid + egui::vec2(6.0, -6.0)], red);
                }
            }
        }
        // the chain of command of the selected unit: a line up to the unit it reports to (and on
        // up the chain), and a line down to each unit that reports to it
        if let Some(sel) = self.selected_unit.and_then(|(id, _)| index_of(id)) {
            let link = |a: usize, b: usize, color: egui::Color32| {
                let (Some(pa), Some(pb)) = (place_of(a), place_of(b)) else { return };
                let (pa, pb) = (to_pt(pa), to_pt(pb));
                painter.line_segment([pa, pb], egui::Stroke::new(4.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 110)));
                painter.line_segment([pa, pb], egui::Stroke::new(2.0, color));
            };
            let parent_of = |i: usize| self.snapshot.unit_command.get(i).and_then(|c| index_of(c.1));
            let (mut at, mut steps) = (sel, 0);
            while let Some(up) = parent_of(at).filter(|_| steps < 8) {
                link(at, up, egui::Color32::from_rgb(235, 200, 70));
                (at, steps) = (up, steps + 1);
            }
            let sel_id = self.snapshot.units[sel].0;
            for (i, c) in self.snapshot.unit_command.iter().enumerate() {
                if c.1 == sel_id {
                    link(sel, i, egui::Color32::from_rgb(110, 210, 110));
                }
            }
        }
        // unit counters in the NATO style: the type inside the frame, the size above it; a stack
        // shows its top unit, or the selected unit if that is one of its members
        for (members, pos) in &stacks {
            let chosen = self.selected_unit.and_then(|sel| members.iter().copied().find(|&i| (self.snapshot.units[i].0, self.snapshot.units[i].1) == sel));
            let i = chosen.unwrap_or(members[0]);
            let c = modes::country_color(self.snapshot.units[i].1);
            let (kind, size) = self.snapshot.unit_kinds.get(i).copied().unwrap_or((0, 0));
            client::counters::draw_stack(&painter, to_pt(*pos), COUNTER_SCALE, kind, size, [c[0], c[1], c[2]], chosen.is_some(), members.len());
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
                        self.camera.pan_pixels((self.cursor.x, self.cursor.y), (position.x, position.y));
                    }
                }
                self.cursor = position;
                self.hover_cell = if over_ui { None } else { self.pick(position) };
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed if !over_ui => {
                    self.press = Some(self.cursor);
                    self.dragging = false;
                }
                ElementState::Released => {
                    if self.press.is_some() && !self.dragging {
                        let k = self.gfx.as_ref().map_or(1.0, |g| g.window.scale_factor() * self.ui_scale as f64);
                        let hit = self.unit_stacks().into_iter().find(|m| (m.1 .0 - self.cursor.x).abs() < (client::counters::WIDTH * COUNTER_SCALE * 0.5) as f64 * k && (m.1 .1 - self.cursor.y).abs() < (client::counters::HEIGHT * COUNTER_SCALE * 0.5) as f64 * k);
                        match hit {
                            Some((members, _)) => {
                                // a click on a stack picks its top unit; clicking again goes down the stack
                                let ids: Vec<(u32, u16)> = members.iter().map(|&i| (self.snapshot.units[i].0, self.snapshot.units[i].1)).collect();
                                let at = self.selected_unit.and_then(|sel| ids.iter().position(|&u| u == sel));
                                self.selected_unit = Some(ids[at.map_or(0, |k| (k + 1) % ids.len())]);
                            }
                            None => self.selected_cell = self.pick(self.cursor),
                        }
                    }
                    self.press = None;
                    self.dragging = false;
                }
                _ => {}
            },
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } if !over_ui => {
                // units move from hexagon to hexagon: the order names the one under the cursor, and
                // the simulation plans the route (roads, rivers, bridges) and marches the unit along it
                if let (Some((unit, owner)), Some(to)) = (self.selected_unit, self.pick(self.cursor)) {
                    self.session.issue(Command::MoveToCell { issuer: CountryId(owner), unit: UnitId(unit), to: meridian_core::CellId(to) });
                    self.status = format!("ordered unit {unit} to hexagon {to}");
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
