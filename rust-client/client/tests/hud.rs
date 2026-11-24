//! M7.3: the HUD layout in settings.cfg (console round trip) and widget placement at the
//! nine anchors with offsets, at two DPI scales.

use std::cell::Cell;
use std::rc::Rc;

use ddnet::net::client::{Client, PlayerInfo};
use ddnet_rs::config::Settings;
use ddnet_rs::hud::{self, Anchor, HudConfig, HudData, Placement, Widget};
use ddnet_rs::hud_editor::{self as editor, Editor};
use ddnet_rs::style::Glass;
use ddnet_rs::watermark;
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
    fn name(&self) -> &'static str {
        "Probe"
    }
    fn icon(&self) -> &'static str {
        ddnet_rs::icons::INFO
    }
    fn default_placement(&self) -> Placement {
        self.placement
    }
    fn show(&mut self, ui: &mut egui::Ui, _: &HudData, _: &HudConfig) {
        let (r, _) = ui.allocate_exact_size(vec2(120.0, 40.0), egui::Sense::hover());
        // text over the whole widget, like the watermark's (the editor drags through it)
        ui.put(r, egui::Label::new("probe probe probe probe"));
        self.rect.set(r);
    }
}

/// Runs `placement` through `hud::show` (three passes: egui sizes a new area first) and
/// returns where the probe widget ended up, in points.
fn place(placement: Placement, ppp: f32, screen: egui::Vec2) -> Rect {
    let client = Client::new(PlayerInfo::default(), "", || [0; 16]);
    let data = HudData {
        client: &client,
        server: "127.0.0.1:8303",
        player_name: "",
        frame_time_average: 0.0001,
        glass: Glass::Subtle,
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

#[test]
fn watermark_order_and_items() {
    let mut c = HudConfig::default();
    // SPEC §1 defaults: user, ping and fps on, in the default order
    let on = |c: &HudConfig| -> Vec<&str> {
        watermark::order(c)
            .into_iter()
            .filter(|k| watermark::shown(c, k))
            .collect()
    };
    assert_eq!(on(&c), ["user", "ping", "fps"]);
    // a saved order (with an unknown key and a duplicate) comes first, the rest follow
    c.execute("hud_set", r#"watermark order "fps, nope,ping,fps""#);
    assert_eq!(
        watermark::order(&c),
        ["fps", "ping", "user", "server", "uid", "tps"]
    );
    c.execute("hud_set", "watermark user 0");
    c.execute("hud_set", "watermark tps 1");
    assert_eq!(on(&c), ["fps", "ping", "tps"]);
}

#[test]
fn watermark_fps_rounds_like_cpp() {
    let client = Client::new(PlayerInfo::default(), "", || [0; 16]);
    let data = |avg| HudData {
        client: &client,
        server: "",
        player_name: "brainless tee",
        frame_time_average: avg,
        glass: Glass::Off,
        predictor: None,
        tee: None,
        events: &[],
    };
    let fps = |avg| watermark::stat("fps", &data(avg)).map(|s| s.1);
    // C++ round_to_int(1.0f / m_FrameTimeAverage); the start value 0.0001 is 10000 fps
    assert_eq!(fps(0.0001).as_deref(), Some("10000"));
    assert_eq!(fps(1.0 / 59.6).as_deref(), Some("60"));
    assert_eq!(fps(1.0 / 144.4).as_deref(), Some("144"));
    // offline the name is the one we asked for; no id, no ping
    let user = watermark::stat("user", &data(1.0)).map(|s| s.1);
    assert_eq!(user.as_deref(), Some("brainless tee"));
    for k in ["uid", "ping"] {
        assert!(watermark::stat(k, &data(1.0)).is_none(), "{k}");
    }
}

/// Where egui puts a widget of `size` placed at `p` (the anchor table of the test above).
fn rect_of(p: Placement, size: egui::Vec2, screen: Rect) -> Rect {
    let a = p.anchor.align();
    let f = vec2(a.x().to_factor(), a.y().to_factor());
    let min = screen.min + screen.size() * f - size * f + vec2(p.x as f32, p.y as f32);
    Rect::from_min_size(min, size)
}

#[test]
fn editor_snaps_to_edges_centre_widgets_and_grid() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let size = vec2(120.0, 40.0);
    let at = |x: f32, y: f32| Rect::from_min_size(pos2(x, y), size);
    let drop = |r: Rect, others: &[Rect], free: bool| editor::drop(r, screen, others, free);
    // 5 pt from the left/top edges: flush, with guides on both edges
    let (p, guides) = drop(at(5.0, 7.0), &[], false);
    assert_eq!((p.anchor, p.x, p.y), (Anchor::TopLeft, 0, 0));
    assert_eq!(guides, [Some(0.0), Some(0.0)]);
    // centre 6 pt off the screen's centre line: centred on the top anchor
    let (p, guides) = drop(at(346.0, 98.0), &[], false);
    assert_eq!((p.anchor, p.x, p.y), (Anchor::Top, 0, 96));
    assert_eq!(guides, [Some(400.0), None]);
    // left edge 4 pt from another widget's right edge: touching it
    let other = Rect::from_min_size(pos2(100.0, 300.0), vec2(80.0, 30.0));
    let (p, _) = drop(at(184.0, 260.0), &[other], false);
    assert_eq!(rect_of(p, size, screen).left(), 180.0);
    // nothing near: offsets on the 8 pt grid
    let (p, guides) = drop(at(37.0, 70.0), &[], false);
    assert_eq!((p.anchor, p.x, p.y), (Anchor::TopLeft, 40, 72));
    assert_eq!(guides, [None, None]);
    // Shift: exactly where it was dropped
    let (p, guides) = drop(at(37.0, 70.0), &[], true);
    assert_eq!((p.x, p.y), (37, 70));
    assert_eq!(guides, [None, None]);
    // dragged past the screen: kept on it
    let (p, _) = drop(at(-50.0, 900.0), &[], true);
    assert_eq!(rect_of(p, size, screen), at(0.0, 560.0));
}

#[test]
fn editor_reassigns_the_anchor_and_keeps_the_place_on_resize() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let size = vec2(120.0, 40.0);
    // dropped 18 pt from the right edge, near the vertical middle
    let r = Rect::from_min_size(pos2(800.0 - 18.0 - 120.0, 290.0), size);
    let (p, _) = editor::drop(r, screen, &[], false);
    assert_eq!(p.anchor, Anchor::Right);
    assert_eq!(p.x, -16); // -18 on the 8 pt grid
    // a wider and taller window: still 16 pt from the right edge, still centred
    let big = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
    let moved = rect_of(p, size, big);
    assert_eq!(big.right() - moved.right(), 16.0);
    // the centre was 10 pt below the screen's: 8 on the grid
    assert_eq!(p.y, 8);
    assert_eq!(moved.center().y - big.center().y, 8.0);
    // every anchor: a free drop puts the widget exactly where it was
    for (x, y) in [(10.0, 10.0), (400.0, 20.0), (660.0, 500.0), (20.0, 290.0)] {
        let r = Rect::from_min_size(pos2(x, y), size);
        let (p, _) = editor::drop(r, screen, &[], true);
        assert_eq!(rect_of(p, size, screen), r, "{:?}", p.anchor);
    }
}

/// The real editor in egui: a drag with the mouse moves the probe and saves where it
/// ended up into the config.
#[test]
fn editor_drags_a_widget_into_the_config() {
    let client = Client::new(PlayerInfo::default(), "", || [0; 16]);
    let data = HudData {
        client: &client,
        server: "",
        player_name: "",
        frame_time_average: 0.01,
        glass: Glass::Off,
        predictor: None,
        tee: None,
        events: &[],
    };
    let rect = Rc::new(Cell::new(Rect::NOTHING));
    let start = Placement {
        anchor: Anchor::TopLeft,
        x: 16,
        y: 16,
        enabled: true,
    };
    let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(Probe {
        placement: start,
        rect: rect.clone(),
    })];
    let mut config = HudConfig::default();
    let mut ed = Editor::default();
    ed.open = true;
    let ctx = egui::Context::default();
    let mut run = |config: &mut HudConfig, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_pass(input, |ctx| {
            ed.show(ctx, &mut widgets, &data, config);
        });
        out.textures_delta.clear();
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    for _ in 0..3 {
        run(&mut config, vec![]);
    }
    // drag from the probe's middle 600 pt right and 300 pt down, in steps
    let from = rect.get().center();
    run(
        &mut config,
        vec![egui::Event::PointerMoved(from), button(from, true)],
    );
    let mut at = from;
    for _ in 0..10 {
        at += vec2(60.0, 30.0);
        run(&mut config, vec![egui::Event::PointerMoved(at)]);
    }
    run(&mut config, vec![button(at, false)]);
    run(&mut config, vec![]);
    // now at (616, 316): the right third and the middle, so the right anchor; offsets on
    // the grid: right edge 800 - 736 = 64, centre 336 - 300 = 36 -> 40
    let p = config.placements["probe"];
    assert_eq!((p.anchor, p.x, p.y), (Anchor::Right, -64, 40), "{p:?}");
    assert_eq!(rect.get().min, pos2(616.0, 320.0));
}

/// 7.4 review: the editor panel covers no widget, and a settings popover opens next to
/// its widget without covering the panel.
#[test]
fn editor_panel_and_popover_keep_out_of_the_way() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let panel = vec2(320.0, 240.0);
    // nothing in the way: bottom centre, 24 pt from the edge
    let r = editor::panel_rect(panel, screen, &[]);
    assert_eq!(r, Rect::from_min_size(pos2(240.0, 336.0), panel));
    // a widget at the bottom centre: the next free place (top centre)
    let bottom = Rect::from_min_size(pos2(350.0, 560.0), vec2(100.0, 24.0));
    let r = editor::panel_rect(panel, screen, &[bottom]);
    assert_eq!(r.min, pos2(240.0, 24.0));
    // a widget near the bottom centre, touching it only with its outline: avoided too
    let near = Rect::from_min_size(pos2(350.0, 300.0), vec2(100.0, 33.0));
    let r = editor::panel_rect(panel, screen, &[near]);
    assert!(!r.intersects(near.expand(4.0)), "{r:?}");
    // every place taken: bottom centre
    let all = [screen.shrink(30.0)];
    let r = editor::panel_rect(panel, screen, &all);
    assert_eq!(r.min, pos2(240.0, 336.0));

    // the popover of a widget top left: below it, off the panel
    let size = vec2(260.0, 300.0);
    let panel = editor::panel_rect(vec2(320.0, 240.0), screen, &[]);
    let widget = Rect::from_min_size(pos2(16.0, 16.0), vec2(300.0, 28.0));
    let at = |w: Rect| editor::popover_align(w, size, screen, panel).align_rect(&w, size, 8.0);
    let p = at(widget);
    assert!(screen.contains_rect(p) && !p.intersects(panel), "{p:?}");
    // a widget just above the panel (where egui put the popover over it in an 800x600
    // window): beside it, on a side that keeps the panel free
    for x in [16.0, 484.0] {
        let widget = Rect::from_min_size(pos2(x, 300.0), vec2(300.0, 28.0));
        let p = at(widget);
        assert!(!p.intersects(panel), "{x}: {p:?}");
        assert!(screen.contains_rect(p), "{x}: {p:?}");
    }
    // no side on screen keeps a small panel free: below the widget, and the panel moves
    // out of its way (the editor passes the popover to panel_rect)
    let panel = editor::panel_rect(vec2(320.0, 160.0), screen, &[]);
    let (widget, size) = (
        Rect::from_min_size(pos2(400.0, 16.0), vec2(300.0, 28.0)),
        vec2(260.0, 450.0),
    );
    let a = editor::popover_align(widget, size, screen, panel);
    let p = a.align_rect(&widget, size, 8.0);
    assert_eq!(p.min, pos2(400.0, 52.0));
    let moved = editor::panel_rect(vec2(320.0, 160.0), screen, &[widget, p]);
    assert_eq!(moved.min, pos2(24.0, 220.0));
}
