//! M7.3: the HUD layout in settings.cfg (console round trip) and widget placement at the
//! nine anchors with offsets, at two DPI scales.

use std::cell::Cell;
use std::rc::Rc;

use ddnet::net::client::{Client, PlayerInfo};
use ddnet_rs::config::Settings;
use ddnet_rs::hud::{self, Anchor, HudConfig, HudData, Placement, Widget};
use ddnet_rs::hud_editor::{self as editor, Editor};
use ddnet_rs::style::{self, Glass};
use ddnet_rs::watermark;
use ddnet_rs::widgets as w;
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
        editing: false,
        predictor: None,
        tee: None,
        media: None,
        player_open: false,
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
    // defaults: local time, ping and fps on (the name is off: it shows above the tee)
    let on = |c: &HudConfig| -> Vec<&str> {
        watermark::order(c)
            .into_iter()
            .filter(|k| watermark::shown(c, k))
            .collect()
    };
    assert_eq!(on(&c), ["time", "ping", "fps"]);
    // a saved order (with an unknown key and a duplicate) comes first, the rest follow
    c.execute("hud_set", r#"watermark order "fps, nope,ping,fps""#);
    assert_eq!(
        watermark::order(&c),
        [
            "fps", "ping", "time", "user", "server", "speed", "uid", "tps"
        ]
    );
    c.execute("hud_set", "watermark user 1");
    c.execute("hud_set", "watermark ping 0");
    c.execute("hud_set", "watermark tps 1");
    assert_eq!(on(&c), ["fps", "time", "user", "tps"]);
    // the time item: local HH:MM, 24-hour, zero-padded
    let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 59).unwrap();
    assert_eq!(watermark::clock(&t(7, 5)), "07:05");
    assert_eq!(watermark::clock(&t(23, 59)), "23:59");
}

/// No jitter: Inter's digits are tabular in every weight, a number in a field keeps what
/// follows it in place, and the watermark shows a new framerate at most every 0.5 s.
#[test]
fn hud_numbers_do_not_jitter() {
    let ctx = egui::Context::default();
    style::install(&ctx, Glass::Off);
    let pass = |t: f64, f: &mut dyn FnMut(&egui::Context)| {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            time: Some(t),
            ..Default::default()
        };
        let mut out = ctx.run_pass(input, |ctx| f(ctx));
        out.textures_delta.clear();
        out.shapes
    };
    let mut after = Vec::new();
    pass(0.0, &mut |ctx| {
        for w in [
            style::Weight::Regular,
            style::Weight::Medium,
            style::Weight::SemiBold,
        ] {
            let width = |s: &str| {
                ctx.fonts_mut(|f| {
                    let g = f.layout_no_wrap(s.into(), style::font(15.0, w), egui::Color32::WHITE);
                    g.size().x
                })
            };
            let ones = width("1111");
            for d in "023456789".chars() {
                assert_eq!(width(&d.to_string().repeat(4)), ones, "{d} in {w:?}");
            }
        }
        egui::Area::new(egui::Id::unique("numbers")).show(ctx, |ui| {
            for v in ["7", "1234", "60"] {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    w::number(
                        ui,
                        v,
                        "0000",
                        15.0,
                        style::Weight::Medium,
                        egui::Color32::WHITE,
                    );
                    after.push(ui.label("fps").rect.left());
                });
            }
        });
    });
    assert!(after.windows(2).all(|p| p[0] == p[1]), "{after:?}");

    let client = Client::new(PlayerInfo::default(), "", || [0; 16]);
    let data = |avg| HudData {
        client: &client,
        server: "",
        player_name: "",
        frame_time_average: avg,
        glass: Glass::Off,
        editing: false,
        predictor: None,
        tee: None,
        media: None,
        player_open: false,
        events: &[],
    };
    let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(watermark::Watermark::default())];
    let config = HudConfig::default();
    let mut fps_at = |t: f64, fps: f32| {
        // two passes: egui's first pass of a new area only sizes it
        let mut shapes = Vec::new();
        for _ in 0..2 {
            shapes = pass(t, &mut |ctx| {
                hud::show(ctx, &mut widgets, &data(1.0 / fps), &config)
            });
        }
        (shapes.iter())
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text().parse::<u32>().is_ok() => {
                    Some(t.galley.text().to_string())
                }
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(fps_at(0.0, 60.0), "60");
    assert_eq!(fps_at(0.2, 30.0), "60");
    assert_eq!(fps_at(0.49, 144.0), "60");
    assert_eq!(fps_at(0.5, 144.0), "144");
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
        editing: false,
        predictor: None,
        tee: None,
        media: None,
        player_open: false,
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
        editing: false,
        predictor: None,
        tee: None,
        media: None,
        player_open: false,
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

/// 7.5 layout: the watermark's measurements as drawn (offset 16, the gaps measured from
/// the reference, the type scale's sizes). Each text's place
/// and size comes from egui's output shapes.
#[test]
fn watermark_layout_measurements() {
    use ddnet_rs::style::{self, size, space};
    let client = Client::new(PlayerInfo::default(), "", || [0; 16]);
    let data = HudData {
        client: &client,
        server: "",
        player_name: "brainless tee",
        frame_time_average: 0.01,
        glass: Glass::Off,
        editing: false,
        predictor: None,
        tee: None,
        media: None,
        player_open: false,
        events: &[],
    };
    let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(watermark::Watermark::default())];
    // the name is off by default; on here to measure the gaps between two stats
    let mut config = HudConfig::default();
    config.execute("hud_set", "watermark user 1");
    let ctx = egui::Context::default();
    style::install(&ctx, Glass::Off);
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            ..Default::default()
        };
        let mut out = ctx.run_pass(input, |ctx| hud::show(ctx, &mut widgets, &data, &config));
        out.textures_delta.clear();
        shapes = out.shapes;
    }
    // (text, font size, rect) of every text shape, left to right
    let texts: Vec<(String, f32, Rect)> = (shapes.iter())
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((
                t.galley.text().to_string(),
                t.galley.job.sections[0].format.font_id.size,
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            _ => None,
        })
        .collect();
    let find = |s: &str| {
        (texts.iter().find(|t| t.0 == s)).unwrap_or_else(|| panic!("{s} not in {texts:?}"))
    };
    let area = ctx
        .memory(|m| m.area_rect(hud::area_id("watermark")))
        .unwrap();
    assert_eq!(area.min, pos2(16.0, 16.0));
    // compact (user review 2026-11-25): bar margin S, logo margin S, the tee mark
    // (icon size), gap S; 49 tall (logo 33 + 2 x 8)
    let logo = find("DDNet-rs");
    assert_eq!(logo.1, size::CAPTION);
    assert_eq!(
        logo.2.left(),
        16.0 + space::S + space::S + size::ICON + space::S
    );
    assert_eq!(area.height(), size::ICON + 4.0 * space::S);
    // stats: icon (icon size), 8 pt, value (body), unit right after it (body)
    let icon = |glyph: &str| find(glyph).2;
    let user = find("brainless tee");
    assert_eq!(user.1, size::BODY);
    assert_eq!(
        user.2.left() - icon(ddnet_rs::icons::USER).right(),
        space::S
    );
    assert_eq!(find(ddnet_rs::icons::USER).1, size::ICON);
    let fps = find("100");
    let unit = find("fps");
    assert_eq!((fps.1, unit.1), (size::BODY, size::BODY));
    assert_eq!(unit.2.left(), fps.2.right());
    // logo to first stat: the logo's margin and 16; stat to stat: 24, no dots (the ping
    // is unknown offline, so user and fps are neighbours)
    assert_eq!(
        icon(ddnet_rs::icons::CLOCK).left() - logo.2.right(),
        space::S + space::L
    );
    assert_eq!(
        icon(ddnet_rs::icons::MONITOR).left() - user.2.right(),
        space::XL
    );
    // (the separator dot is a 1.5 pt circle; the tee mark's body is a larger one)
    let dots =
        (shapes.iter()).filter(|c| matches!(&c.shape, egui::Shape::Circle(d) if d.radius < 2.0));
    assert_eq!(dots.count(), 0);
    // each icon has a soft glow centred behind it
    let glows: Vec<Rect> = (shapes.iter())
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.blur_width > 0.0 && r.rect.width() < 40.0 => Some(r.rect),
            _ => None,
        })
        .collect();
    for glyph in [
        ddnet_rs::icons::CLOCK,
        ddnet_rs::icons::USER,
        ddnet_rs::icons::MONITOR,
    ] {
        let c = icon(glyph).center();
        assert!(
            glows.iter().any(|g| (g.center() - c).length() < 1.0),
            "{glyph:?}"
        );
    }
    // the bar ends 16 after the last unit
    assert_eq!(area.right() - unit.2.right(), space::L);
}

/// 7.6: the movement information against the real `CHud::GetMovementInformation` and
/// `CPlayers::GetPlayerTargetAngle` (`rs_parity movement`, 20,000 random characters, local
/// and other players, with and without DDNet targets, random velocity ramps): every float
/// bit-identical and every shown text equal.
#[test]
fn movement_information_matches_cpp() {
    use ddnet::physics::tuning::{Param, Tuning};
    use ddnet::predict::Rendered;
    use ddnet::protocol::generated::{ObjCharacter, ObjDDNetCharacter};
    use ddnet_rs::movement;
    use ddnet_rs::render::players::target_angle;

    let path = format!(
        "{}/../tests/golden/movement.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).unwrap();
    let mut client = Client::new(PlayerInfo::default(), "", || [0; 16]);
    let mut wrong = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let i = |k: usize| f[k].parse::<i32>().unwrap();
        let float = |k: usize| f32::from_bits(u32::from_str_radix(f[k], 16).unwrap());
        let ch = |k: usize| ObjCharacter {
            x: i(k),
            y: i(k + 1),
            vel_x: i(k + 2),
            vel_y: i(k + 3),
            angle: i(k + 4),
            ..Default::default()
        };
        let ext = |x, y| ObjDDNetCharacter {
            target_x: x,
            target_y: y,
            ..Default::default()
        };
        let mode = i(12);
        let tee = Rendered {
            id: 0,
            prev: ch(2),
            cur: ch(7),
            pos: Default::default(),
            predicted: false,
            predicted_local: false,
            ext: (mode != 0).then(|| ext(i(17), i(18))),
            prev_ext: (mode == 2).then(|| ext(i(13), i(14))),
        };
        // C++ sends (int)m_aMousePos as the target
        client.input.target_x = float(15) as i32;
        client.input.target_y = float(16) as i32;
        client.intra_tick = float(19);
        let mut tuning = Tuning::default();
        for (n, p) in [
            Param::VelrampStart,
            Param::VelrampRange,
            Param::VelrampCurvature,
        ]
        .into_iter()
        .enumerate()
        {
            tuning.0[p as usize] = i(20 + n);
        }
        let local = Some(if i(1) != 0 { 0 } else { 1 });
        let aim = target_angle(&client, &tee, local);
        let m = movement::info(&tee.prev, &tee.cur, client.intra_tick, &tuning, aim);
        let values = [m.pos.x, m.pos.y, m.speed.x, m.speed.y, m.angle];
        let got = format!(
            "{} | {} | {}",
            f[..23].join(" "),
            values.map(|v| format!("{:08x}", v.to_bits())).join(" "),
            values.map(movement::shown).join(" ")
        );
        if got != line {
            wrong.push(format!("got  {got}\nwant {line}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} differ:\n{}",
        wrong.len(),
        text.lines().count(),
        wrong[..wrong.len().min(5)].join("\n")
    );
}

/// 7.5 review: a settings popover taller than the room above and below its widget (the
/// watermark's eight items in an 800x600 window) opens beside the widget, moved onto the
/// screen, covering neither the widget nor the panel.
#[test]
fn tall_popover_opens_beside_the_widget() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let widget = Rect::from_min_size(pos2(272.0, 362.0), vec2(528.0, 64.0));
    let size = vec2(246.0, 490.0);
    let panel = editor::panel_rect(vec2(270.0, 190.0), screen, &[widget]);
    let a = editor::popover_align(widget, size, screen, panel);
    let r = a.align_rect(&widget, size, 8.0);
    // egui moves it onto the screen vertically
    let r = r.translate(vec2(
        0.0,
        (-r.top()).max(0.0) - (r.bottom() - 600.0).max(0.0),
    ));
    assert!(screen.contains_rect(r), "{r:?}");
    assert!(!r.intersects(widget), "{r:?} {widget:?}");
    let moved = editor::panel_rect(vec2(270.0, 190.0), screen, &[widget, r]);
    assert!(
        !moved.intersects(r) && !moved.intersects(widget),
        "{moved:?} {r:?}"
    );
}

/// 7.7: `CHud::RenderPlayerState`'s decisions, on cases worked out from the C++ code
/// (hud.cpp:842): the jump count in every branch and its quirks, weapons, the ninja
/// timer, the order of the indicators, and the gun indicator quirk.
#[test]
fn status_bar_follows_render_player_state() {
    use ddnet::physics::core::CharacterCore;
    use ddnet::protocol::generated::{self as g, ObjCharacter, ObjDDNetCharacter};
    use ddnet_rs::statusbar::{Indicator as I, Input, status};

    let run = |c: &CharacterCore,
               p: &ObjCharacter,
               grounded: bool,
               ext: &ObjDDNetCharacter,
               display: bool,
               tick: i32| {
        status(&Input {
            core: c,
            player: p,
            grounded,
            ext,
            display_info: display,
            game_tick: tick,
        })
    };
    let ext = ObjDDNetCharacter::default();
    let jumps = |jumps: i32, total: i32, jumped: i32, grounded: bool, endless: bool| {
        let mut c = CharacterCore::default();
        c.jumps = jumps;
        c.jumped_total = total;
        c.endless_jump = endless;
        let p = ObjCharacter {
            jumped,
            ..Default::default()
        };
        run(&c, &p, grounded, &ext, true, 0).jumps
    };
    // two jumps: grounded both left; in the air the ground jump counts as used
    assert_eq!(jumps(2, 0, 0, true, false), (2, 2));
    assert_eq!(jumps(2, 0, 1, false, false), (1, 2));
    // all used and the air jump flag (Jumped & 2) set: none; without it C++ shows 1
    assert_eq!(jumps(2, 1, 3, false, false), (0, 2));
    assert_eq!(jumps(2, 1, 1, false, false), (1, 2));
    // one jump: Jumped & 2 is the used count (2), so -1 unused -> 0
    assert_eq!(jumps(1, 0, 2, true, false), (0, 1));
    assert_eq!(jumps(1, 5, 0, false, false), (1, 1));
    // only a ground jump (-1): in the air with no air jump flag the "edge case" shows 1
    assert_eq!(jumps(-1, 0, 0, true, false), (1, 1));
    assert_eq!(jumps(-1, 0, 0, false, false), (1, 1));
    assert_eq!(jumps(-1, 0, 2, false, false), (0, 1));
    // endless jump: never all used
    assert_eq!(jumps(2, 7, 3, false, true), (1, 2));
    // at most 10 shown; zero jumps
    assert_eq!(jumps(20, 0, 0, true, false), (10, 10));
    assert_eq!(jumps(0, 0, 0, true, false), (0, 0));
    // malformed extremes wrap like C++ instead of panicking
    assert_eq!(jumps(i32::MIN, i32::MAX, 0, false, true).1, 0);
    // without display info: the snapped jump count, both values
    let e = ObjDDNetCharacter {
        jumps: -3,
        ..Default::default()
    };
    let c = CharacterCore::default();
    assert_eq!(
        run(&c, &ObjCharacter::default(), true, &e, false, 0).jumps,
        (3, 3)
    );

    // weapons in id order with the active one; the ninja timer: 750 ticks, shown only
    // while running and with display info
    let mut c = CharacterCore::default();
    for w in [0, 1, 5] {
        c.weapons[w].got = true;
    }
    c.ninja.activation_tick = 100;
    let p = ObjCharacter {
        weapon: 5,
        ..Default::default()
    };
    let s = run(&c, &p, true, &ext, true, 100);
    assert_eq!(s.weapons, [(0, false), (1, false), (5, true)]);
    assert_eq!(s.ninja, Some(1.0));
    assert_eq!(run(&c, &p, true, &ext, true, 475).ninja, Some(0.5));
    assert_eq!(run(&c, &p, true, &ext, true, 850).ninja, None);
    assert_eq!(run(&c, &p, true, &ext, false, 100).ninja, None);

    // every indicator on, in C++'s order; telegun and hit indicators need the weapon
    let mut c = CharacterCore::default();
    c.endless_jump = true;
    c.endless_hook = true;
    c.jetpack = true;
    c.has_telegun_gun = true;
    c.has_telegun_grenade = true;
    c.has_telegun_laser = true;
    c.solo = true;
    c.collision_disabled = true;
    c.hook_hit_disabled = true;
    c.hammer_hit_disabled = true;
    c.shotgun_hit_disabled = true;
    c.grenade_hit_disabled = true;
    c.laser_hit_disabled = true;
    c.deep_frozen = true;
    c.live_frozen = true;
    c.jumps = 2;
    let modes = ObjDDNetCharacter {
        flags: g::CHARACTERFLAG_LOCK_MODE
            | g::CHARACTERFLAG_PRACTICE_MODE
            | g::CHARACTERFLAG_TEAM0_MODE,
        ..Default::default()
    };
    let s = run(&c, &ObjCharacter::default(), true, &modes, true, 0);
    assert_eq!(s.capabilities, [I::EndlessJump, I::EndlessHook, I::Jetpack]);
    assert_eq!(
        s.prohibited,
        [
            I::Solo,
            I::CollisionDisabled,
            I::HookHitDisabled,
            I::HammerHitDisabled
        ]
    );
    assert_eq!(
        s.modes,
        [
            I::LockMode,
            I::PracticeMode,
            I::Team0Mode,
            I::DeepFrozen,
            I::LiveFrozen
        ]
    );
    // modes need display info
    let s = run(&c, &ObjCharacter::default(), true, &modes, false, 0);
    assert_eq!(s.modes, [I::DeepFrozen, I::LiveFrozen]);
    for w in &mut c.weapons {
        w.got = true;
    }
    let s = run(&c, &ObjCharacter::default(), true, &ext, true, 0);
    assert_eq!(
        s.capabilities,
        [
            I::EndlessJump,
            I::EndlessHook,
            I::Jetpack,
            I::TeleGun,
            I::TeleGrenade,
            I::TeleLaser
        ]
    );
    assert_eq!(s.prohibited.len(), 8);
    // the C++ quirk: the gun indicator follows the grenade flag (and the gun telegun)
    let mut quirk = c.clone();
    quirk.grenade_hit_disabled = true;
    quirk.has_telegun_gun = true;
    let only = |c: &CharacterCore| {
        let mut c = c.clone();
        (
            c.solo,
            c.collision_disabled,
            c.hook_hit_disabled,
            c.hammer_hit_disabled,
        ) = (false, false, false, false);
        (c.shotgun_hit_disabled, c.laser_hit_disabled) = (false, false);
        run(&c, &ObjCharacter::default(), true, &ext, true, 0).prohibited
    };
    assert_eq!(only(&quirk), [I::GunHitDisabled, I::GrenadeHitDisabled]);
    let mut no_telegun = quirk.clone();
    no_telegun.has_telegun_gun = false;
    assert_eq!(only(&no_telegun), [I::GrenadeHitDisabled]);
}

/// 7.8/7.9: `str_time` in every format and `str_time_float` against the real C++
/// (`rs_golden` str_time.txt: unit boundaries, extremes, random values).
#[test]
fn str_time_matches_cpp() {
    use ddnet_rs::timefmt::{TimeFormat, str_time, str_time_float};
    let path = format!(
        "{}/../tests/golden/str_time.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).unwrap();
    let mut n = 0;
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(4, ' ').collect();
        let format = TimeFormat::ALL[f[1].parse::<usize>().unwrap()];
        let got = match f[0] {
            "t" => str_time(f[2].parse().unwrap(), format),
            _ => str_time_float(
                f32::from_bits(u32::from_str_radix(f[2], 16).unwrap()),
                format,
            ),
        };
        assert_eq!(got, f[3], "{line}");
        n += 1;
    }
    assert_eq!(n, 6 * (19 + 300 + 300));
}

/// 7.8: `CInfoMessages`' rules: five entries at most (a new one overwrites the oldest),
/// oldest first, gone 500 ticks after arriving, kills that would be empty skipped,
/// inactive clients unnamed, finishes only for active clients, C++'s time texts.
#[test]
fn killfeed_follows_info_messages() {
    use ddnet::protocol::generated::{GameMsg, MsgSvKillMsg, MsgSvKillMsgTeam, MsgSvRaceFinish};
    use ddnet_rs::killfeed::{Feed, Kind, LIFETIME_TICKS, Player};
    use ddnet_rs::teehead::TeeLook;

    // clients 0..3 are active, named "p0".."p3"
    let player = |id: i32| Player {
        id,
        name: format!("p{id}"),
        tee: TeeLook {
            skin: "default".into(),
            custom_color: id == 2,
            color_body: id,
        },
    };
    let name = |id: i32| (0..4).contains(&id).then(|| player(id));
    let kill = |killer, victim, weapon| {
        GameMsg::SvKillMsg(MsgSvKillMsg {
            killer,
            victim,
            weapon,
            mode_special: 0,
        })
    };
    let mut feed = Feed::default();
    for (n, tick) in (0..7).zip(100..) {
        feed.on_message(&kill(1, 2, n % 6), name, tick);
    }
    // seven arrived, the last five show, oldest first
    let ticks: Vec<i32> = feed.entries(106).iter().map(|e| e.tick).collect();
    assert_eq!(ticks, [102, 103, 104, 105, 106]);
    // an entry lives through tick + 500 and is gone after
    assert_eq!(feed.entries(102 + LIFETIME_TICKS).len(), 5);
    assert_eq!(feed.entries(103 + LIFETIME_TICKS).len(), 4);
    assert_eq!(feed.entries(107 + LIFETIME_TICKS).len(), 0);

    let mut feed = Feed::default();
    // inactive killer and victim with a world kill (weapon -2): empty, skipped
    feed.on_message(&kill(40, 41, -2), name, 0);
    assert!(feed.entries(0).is_empty());
    // inactive victim, active killer: kept, the killer named
    feed.on_message(&kill(3, 41, 1), name, 0);
    // self kill: same id for both
    feed.on_message(&kill(2, 2, -1), name, 0);
    let kinds: Vec<Kind> = feed
        .entries(0)
        .into_iter()
        .map(|e| e.kind.clone())
        .collect();
    assert_eq!(
        kinds[0],
        Kind::Kill {
            victim: None,
            killer: Some(player(3)),
            weapon: 1
        }
    );
    assert_eq!(
        kinds[1],
        Kind::Kill {
            victim: Some(player(2)),
            killer: Some(player(2)),
            weapon: -1
        }
    );

    // team kills always add an entry; finishes need an active client
    let mut feed = Feed::default();
    feed.on_message(
        &GameMsg::SvKillMsgTeam(MsgSvKillMsgTeam { team: 3, first: -1 }),
        name,
        0,
    );
    let finish = |client_id, time, diff, record_personal| {
        GameMsg::SvRaceFinish(MsgSvRaceFinish {
            client_id,
            time,
            diff,
            record_personal,
            record_server: 0,
        })
    };
    feed.on_message(&finish(40, 1000, 0, 0), name, 0);
    feed.on_message(&finish(1, 3_723_450, -1_500, 1), name, 0);
    feed.on_message(&finish(2, 42_100, 360, 0), name, 0);
    feed.on_message(&finish(3, 5, 0, 0), name, 0);
    let kinds: Vec<Kind> = feed
        .entries(0)
        .into_iter()
        .map(|e| e.kind.clone())
        .collect();
    assert_eq!(kinds.len(), 4);
    assert_eq!(kinds[0], Kind::TeamKill { team: 3 });
    // C++ str_time_float(.., HOURS_CENTISECS) of time / 1000 and |diff| / 1000
    assert_eq!(
        kinds[1],
        Kind::Finish {
            player: player(1),
            // 3723.45 as f32 is 3723.4499..: C++ rounds it to the even 03.44
            time: "01:02:03.44".into(),
            diff: Some("(-01.50)".into()),
            faster: true,
            record: true
        }
    );
    assert_eq!(
        kinds[2],
        Kind::Finish {
            player: player(2),
            time: "42.10".into(),
            diff: Some("(+00.36)".into()),
            faster: false,
            record: false
        }
    );
    // 5 ms rounds to an even centisecond: 0
    assert!(matches!(&kinds[3], Kind::Finish { time, diff: None, .. } if time == "00.00"));
}

/// The C++ HUD switches become the first values of our HUD items; our own settings win.
#[test]
fn cpp_hud_settings_are_imported() {
    let cpp = |var: &str| matches!(var, "cl_showhud_player_speed" | "cl_showkillmessages");
    let default = |id: &str| {
        Some(Placement {
            anchor: Anchor::TopLeft,
            x: 0,
            y: id.len() as i32,
            enabled: true,
        })
    };
    let mut c = HudConfig::default();
    c.execute("hud_set", "killfeed kills 0");
    c.import_cpp(cpp, default);
    // the speed line only; the panel on because one of its lines is
    assert!(!c.on("movement", "position", true) && c.on("movement", "speed", false));
    assert!(c.placements["movement"].enabled);
    // cl_showhud_ddrace 0: the status bar off; jumps from cl_showhud_jumps_indicator 0
    assert!(!c.placements["status"].enabled && !c.on("status", "jumps", true));
    // ours (kills 0) kept; finishes from C++ (0)
    assert!(!c.on("killfeed", "kills", true) && !c.on("killfeed", "finishes", true));
}
