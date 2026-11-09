//! Dev probe: does a click on one of the number buttons reach the code that reads it, and how
//! many times does egui call the UI closure per frame?
fn main() {
    let ctx = egui::Context::default();
    let ppp = 2.3f32;
    let mut rects: Vec<egui::Rect> = Vec::new();
    let mut speed = 0u8;
    for frame in 0..8 {
        let mut events = Vec::new();
        if frame == 3 {
            let r = rects.get(2).copied().unwrap_or(egui::Rect::ZERO); // the button "3"
            events.push(egui::Event::PointerMoved(r.center()));
        }
        if frame == 4 {
            events.push(egui::Event::PointerButton { pos: rects[2].center(), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        }
        if frame == 5 {
            events.push(egui::Event::PointerButton { pos: rects[2].center(), button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        }
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1250.0, 130.0))), events, ..Default::default() };
        raw.viewports.get_mut(&raw.viewport_id).unwrap().native_pixels_per_point = Some(ppp);
        let mut calls = 0;
        let mut got: Option<u8> = None;
        let _ = ctx.run_ui(raw, |ui| {
            calls += 1;
            got = None;
            egui::Panel::top("t").show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    rects.clear();
                    for n in 1..=5u8 {
                        let r = ui.selectable_label(speed == n, format!("{n}"));
                        rects.push(r.rect);
                        if r.clicked() {
                            got = Some(n);
                        }
                    }
                });
            });
        });
        if let Some(n) = got {
            speed = n;
        }
        eprintln!("frame {frame}: closure called {calls}x, click seen: {got:?}, speed {speed}");
    }
}
