//! M6.4: tee animation, colours and skin preparation against the C++ code. Data from
//! `rs_parity anims`, `rs_parity colors` and `rs_parity skins` (see PROGRESS.md).

use ddnet_rs::image::{Format, load_png_file};
use ddnet_rs::render::anim::AnimState;
use ddnet_rs::render::content::{ANIM_BASE, ANIMATIONS, Keyframe};
use ddnet_rs::render::map::hsl_packed_to_rgb;
use ddnet_rs::render::players::walk_run_times;
use ddnet_rs::render::tee::skin_color;
use ddnet_rs::skin::{Metric, load_skin_data};

fn repo(path: &str) -> String {
    format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"))
}

fn golden(name: &str) -> String {
    std::fs::read_to_string(repo(&format!("rust-client/tests/golden/{name}"))).unwrap()
}

fn crc(data: &[u8]) -> String {
    let mut c = flate2::Crc::new();
    c.update(data);
    format!("{:08x}", c.sum())
}

fn bits(f: f32) -> String {
    format!("{:08x}", f.to_bits())
}

fn float(hex: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(hex, 16).unwrap())
}

fn state_bits(s: &AnimState) -> String {
    let k = |f: &Keyframe| format!("{} {} {}", bits(f.x), bits(f.y), bits(f.angle));
    [&s.body, &s.back_foot, &s.front_foot, &s.attach]
        .map(k)
        .join(" ")
}

fn check(wrong: &mut Vec<String>, got: String, want: &str) {
    if got != want && wrong.len() < 20 {
        wrong.push(format!("got  {got}\nwant {want}"));
    }
}

/// `CAnimState` Set/Add/GetIdle and RenderPlayer's blend: bit-identical.
#[test]
fn animations_match_cpp() {
    let text = golden("anims.txt");
    let mut wrong = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let base = || {
            let mut s = AnimState::default();
            s.set(&ANIMATIONS[ANIM_BASE], 0.0);
            s
        };
        let got = match f[0] {
            "idle" => format!("idle {}", state_bits(&AnimState::idle())),
            "set" => {
                let mut s = AnimState::default();
                s.set(&ANIMATIONS[f[1].parse::<usize>().unwrap()], float(f[2]));
                format!("set {} {} {}", f[1], f[2], state_bits(&s))
            }
            "add" => {
                let mut s = base();
                s.add(
                    &ANIMATIONS[f[1].parse::<usize>().unwrap()],
                    float(f[2]),
                    float(f[3]),
                );
                format!("add {} {} {} {}", f[1], f[2], f[3], state_bits(&s))
            }
            "player" => {
                let (walk, run) = walk_run_times(float(f[1]));
                let movement: usize = f[2].parse().unwrap();
                let time = match ANIMATIONS[movement].name {
                    "walk" => walk,
                    "run_left" | "run_right" => run,
                    _ => 0.0,
                };
                let mut s = base();
                s.add(&ANIMATIONS[movement], time, 1.0);
                if let Ok(swing) = usize::try_from(f[3].parse::<i32>().unwrap()) {
                    s.add(&ANIMATIONS[swing], float(f[4]), 1.0);
                }
                format!(
                    "player {} {} {} {} {} {} {}",
                    f[1],
                    f[2],
                    f[3],
                    f[4],
                    bits(walk),
                    bits(run),
                    state_bits(&s)
                )
            }
            _ => panic!("{line}"),
        };
        check(&mut wrong, got, line);
    }
    assert!(text.lines().count() > 7000);
    assert!(wrong.is_empty(), "differ:\n{}", wrong.join("\n"));
}

/// `ApplyColors` (UnclampLighting) and plain `color_cast` of packed HSL: bit-identical.
#[test]
fn tee_colors_match_cpp() {
    let text = golden("colors.txt");
    let mut wrong = Vec::new();
    for line in text.lines() {
        let c: i32 = line.split(' ').next().unwrap().parse().unwrap();
        let a = skin_color(c);
        let [r, g, b] = hsl_packed_to_rgb(c as u32);
        let got = [c.to_string()]
            .into_iter()
            .chain(a.iter().chain(&[r, g, b, 1.0]).map(|v| bits(*v)))
            .collect::<Vec<_>>()
            .join(" ");
        check(&mut wrong, got, line);
    }
    assert_eq!(text.lines().count(), 5269);
    assert!(wrong.is_empty(), "differ:\n{}", wrong.join("\n"));
}

/// `CSkins::LoadSkinData` on every skin in `data/skins`, also cropped (bicubic resize)
/// and without alpha: same size, pixels, grayscale, blood colour and metrics.
#[test]
fn skin_data_matches_cpp() {
    let text = golden("skins.txt");
    let mut wrong = Vec::new();
    let metric = |m: &Metric| {
        format!(
            "{} {} {} {} {} {}",
            m.width, m.height, m.offset_x, m.offset_y, m.max_width, m.max_height
        )
    };
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let file = std::fs::read(repo(&format!("data/skins/{}", f[0]))).unwrap();
        let Some(mut img) = load_png_file(&file) else {
            check(&mut wrong, format!("{} - fail", f[0]), line);
            continue;
        };
        let variant: u32 = f[1].parse().unwrap();
        match variant {
            1 => img = img.sub_image(0, 0, img.width * 3 / 4 + 1, img.height),
            2 => img = img.sub_image(0, 0, img.width, img.height * 3 / 4 + 1),
            3 if img.format == Format::Rgba => {
                img.data = img
                    .data
                    .chunks(4)
                    .flat_map(|p| [p[0], p[1], p[2]])
                    .collect();
                img.format = Format::Rgb;
            }
            _ => {}
        }
        let got = match load_skin_data(img) {
            None => format!("{} {} fail", f[0], f[1]),
            Some(d) => format!(
                "{} {} {} {} {} {} {} {} {}",
                f[0],
                f[1],
                d.image.width,
                d.image.height,
                d.blood.map(bits).join(" "),
                metric(&d.body),
                metric(&d.feet),
                crc(&d.image.data),
                crc(&d.grayscale.data)
            ),
        };
        check(&mut wrong, got, line);
    }
    assert_eq!(text.lines().count(), 404);
    assert!(wrong.is_empty(), "differ:\n{}", wrong.join("\n"));
}

/// A custom-coloured default tee drawn on the GPU: the body's center is the colourable
/// texel times the body colour, like C++ (`SetColor` with 8-bit colours, texture times
/// vertex colour).
#[test]
fn custom_colored_tee_on_gpu() {
    use ddnet::physics::vmath::vec2;
    use ddnet_rs::gfx::batch::Batch;
    use ddnet_rs::gfx::{Gfx, Size};
    use ddnet_rs::render::content::{SPRITE_TEE_BODY, SPRITES};
    use ddnet_rs::render::tee::{Quads, Skin, TeeRenderInfo, render_tee};
    use ddnet_rs::skin::sprite_image;

    let size = Size {
        width: 128,
        height: 128,
    };
    let Some(mut g) = Gfx::headless(size) else {
        eprintln!("SKIPPED: no graphics adapter");
        return;
    };
    let file = std::fs::read(repo("data/skins/default.png")).unwrap();
    let data = load_skin_data(load_png_file(&file).unwrap()).unwrap();
    let mut info = TeeRenderInfo::new(Skin::upload(&mut g, &data));
    info.apply_colors(true, 65408, 65408);
    info.size = 64.0;
    let mut batch = Batch::default();
    batch.map_screen(-92.0, -92.0, 92.0, 92.0); // the live scale: 1150 units on 800 px
    let mut idle = AnimState::idle();
    idle.body.y = 0.0;
    let dir = vec2(1.0, 0.0);
    render_tee(
        &mut batch,
        &Quads::default(),
        &idle,
        &info,
        0,
        dir,
        vec2(0.0, 0.0),
        1.0,
    );
    let world = g.world_view();
    g.render(&mut batch, &world, size);
    let px = g.read_world();
    // world (-18, 10), clear of the eyes: target pixel (51, 71), body texel (21, 63)
    // (the 96 px sprite is drawn 64 units wide)
    let body = sprite_image(&data.grayscale, None, &SPRITES[SPRITE_TEE_BODY]);
    let t = &body.data[(63 * body.width + 21) as usize * 4..][..4];
    let got = &px[(71 * 128 + 51) as usize * 4..][..3];
    let want = [t[0] as f32, t[1] as f32 * 0.502, t[2] as f32 * 0.502];
    assert!(
        got.iter()
            .zip(want)
            .all(|(&g, w)| (g as f32 - w).abs() <= 3.0),
        "got {got:?}, want {want:?} (texel {t:?})"
    );
}
