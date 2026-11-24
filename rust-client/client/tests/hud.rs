//! M7.3: the HUD layout in settings.cfg (console round trip) and widget placement at the
//! nine anchors with offsets, at two DPI scales.

use std::cell::Cell;
use std::rc::Rc;

use ddnet::net::client::{Client, PlayerInfo};
use ddnet_rs::config::Settings;
use ddnet_rs::hud::{self, Anchor, HudConfig, HudData, Placement, Widget};
use egui::{Rect, pos2, vec2};

#[test]
fn layout_round_trips_through_settings_cfg() {
    let lines = [
        "hud_widget watermark top_left 16 16 1",
        "hud_widget speed bottom_right -16 -24 0",
        r#"hud_set watermark order "user,fps,ping""#,
        r#"hud_set "island" "label" "say \"hi\" \\ bye""#,
        "hud_set future_widget some_key 3", // unknown to this version: kept
    ];
    let s = Settings::load(&[lines.join("\n").as_bytes()]);
    let p = s.hud.placements["speed"];
    assert_eq!(
        p,
        Placement {
            anchor: Anchor::BottomRight,
            x: -16,
            y: -24,
            enabled: false
        }
    );
    assert_eq!(s.hud.value("watermark", "order"), Some("user,fps,ping"));
    assert_eq!(s.hud.value("island", "label"), Some(r#"say "hi" \ bye"#));
    assert_eq!(s.hud.value("future_widget", "some_key"), Some("3"));
    assert_eq!(s.skipped, 0);
    // what we save loads back to the same
    let again = Settings::load(&[s.serialize().as_bytes()]);
    assert_eq!(again.hud, s.hud);
}

#[test]
fn malformed_hud_lines_change_nothing() {
    let mut c = HudConfig::default();
    for (cmd, args) in [
        ("hud_widget", "w middle 1 2 1"), // no such anchor
        ("hud_widget", "w top 1 x 1"),    // not a number
        ("hud_widget", "w top 1 2"),      // missing argument
        ("hud_set", "w key"),             // no value
    ] {
        assert!(c.execute(cmd, args));
    }
    assert_eq!(c, HudConfig::default());
    assert!(!c.execute("bind", "a +left"));
}

/// A widget of a fixed size that records where egui put it.
struct Probe {
    placement: Placement,
    rect: Rc<Cell<Rect>>,
}

impl Widget for Probe {
    fn id(&self) -> &'static str {
        "probe"
    }
    fn default_placement(&self) -> Placement {
        self.placement
    }
    fn show(&mut self, ui: &mut egui::Ui, _: &HudData, _: &HudConfig) {
        let (r, _) = ui.allocate_exact_size(vec2(120.0, 40.0), egui::Sense::hover());
        self.rect.set(r);
    }
}

/// Runs `placement` through `hud::show` (three passes: egui sizes a new area first) and
/// returns where the probe widget ended up, in points.
fn place(placement: Placement, ppp: f32, screen: egui::Vec2) -> Rect {
    let client = Client::new(PlayerInfo::default(), "", || [0; 16]);
    let data = HudData {
        client: &client,
        predictor: None,
        tee: None,
        events: &[],
    };
    let rect = Rc::new(Cell::new(Rect::NOTHING));
    let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(Probe {
        placement,
        rect: rect.clone(),
    })];
    let ctx = egui::Context::default();
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), screen)),
            ..Default::default()
        };
        ctx.set_pixels_per_point(ppp);
        let mut out = ctx.run_pass(input, |ctx| {
            hud::show(ctx, &mut widgets, &data, &HudConfig::default())
        });
        // no GPU here: egui checks that texture updates are not just dropped
        out.textures_delta.clear();
    }
    rect.get()
}

/// The widget's anchor point is the screen's anchor point plus the offset (in points,
/// whatever the DPI); offsets point inwards here.
#[test]
fn widgets_sit_at_their_anchor_plus_offset() {
    let screen = vec2(800.0, 600.0);
    for ppp in [1.0, 2.0] {
        // (anchor, its point as a fraction of the screen), written out independently
        let points = [
            (Anchor::TopLeft, vec2(0.0, 0.0)),
            (Anchor::Top, vec2(0.5, 0.0)),
            (Anchor::TopRight, vec2(1.0, 0.0)),
            (Anchor::Left, vec2(0.0, 0.5)),
            (Anchor::Center, vec2(0.5, 0.5)),
            (Anchor::Right, vec2(1.0, 0.5)),
            (Anchor::BottomLeft, vec2(0.0, 1.0)),
            (Anchor::Bottom, vec2(0.5, 1.0)),
            (Anchor::BottomRight, vec2(1.0, 1.0)),
        ];
        for (anchor, f) in points {
            let inward = |f: f32, d: i32| if f == 1.0 { -d } else { d };
            let (x, y) = (inward(f.x, 10), inward(f.y, 6));
            let placement = Placement {
                anchor,
                x,
                y,
                enabled: true,
            };
            let got = place(placement, ppp, screen).min;
            let size = vec2(120.0, 40.0);
            let want = (screen * f - size * f + vec2(x as f32, y as f32)).to_pos2();
            assert!(
                (got - want).length() < 0.5,
                "{anchor:?} at {ppp}: {got:?}, want {want:?}"
            );
        }
    }
}

/// An offset that would push a widget off the screen keeps it at the edge (egui
/// constrains areas to the screen).
#[test]
fn widgets_stay_on_screen() {
    let placement = Placement {
        anchor: Anchor::TopLeft,
        x: -30,
        y: -6,
        enabled: true,
    };
    assert_eq!(
        place(placement, 1.0, vec2(800.0, 600.0)).min,
        pos2(0.0, 0.0)
    );
}
