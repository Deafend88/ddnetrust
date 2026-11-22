//! M6.1: the renderer's test pattern drawn on the GPU and read back. Every pixel is
//! checked against what the C++ OpenGL backend produces for the same calls (8-bit
//! vertex colours, `glBlendFunc` factors, 1:1 texel sampling, quad rotation and
//! freeform corner order), except the colour gradient, whose corners are checked.

use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{self, Gfx, Size};

const W: u32 = 64;
const H: u32 = 32;

fn expected(x: u32, y: u32, tex: &[u8]) -> Option<[u8; 4]> {
    let texel = |i: u32, j: u32| {
        let o = (4 * (j * 4 + i)) as usize;
        [tex[o], tex[o + 1], tex[o + 2], tex[o + 3]]
    };
    Some(match (x, y) {
        (0..8, 0..8) => [255, 0, 0, 128],    // BlendNone keeps alpha
        (8..16, 0..8) => [128, 0, 127, 64],  // src*a + dst*(1-a), alpha a*a
        (16..24, 0..8) => [0, 128, 255, 64], // src*a + dst
        (24..28, 0..4) => texel(x - 24, y),
        (28..30, 0..2) => texel(x - 28 + 2, y),
        (35..37, 4..12) => [255, 255, 0, 255], // 8x2 rotated by pi/2
        (40..48, 0..8) => [255, 0, 255, 255],
        (48..56, 8..16) => texel((x - 48) % 4, (y - 8) % 4),
        (48..56, 16..24) => texel((x - 48).min(3), (y - 16).min(3)),
        (0..16, 16..32) => return None, // gradient, corners checked below
        _ => [0, 0, 255, 0],            // C++ Clear: alpha 0
    })
}

#[test]
fn test_pattern_pixels() {
    let Some(mut g) = Gfx::headless(Size {
        width: W,
        height: H,
    }) else {
        eprintln!("SKIPPED: no graphics adapter");
        return;
    };
    let tex = gfx::pattern_texture();
    let id = g.create_texture(4, 4, &tex);
    let mut batch = Batch::default();
    gfx::test_pattern(&mut batch, id);
    let world = g.world_view();
    g.render(&mut batch, &world);
    let px = g.read_world();
    assert_eq!(px.len(), (4 * W * H) as usize);
    let at = |x: u32, y: u32| {
        let o = (4 * (y * W + x)) as usize;
        [px[o], px[o + 1], px[o + 2], px[o + 3]]
    };

    let mut wrong = Vec::new();
    for y in 0..H {
        for x in 0..W {
            if let Some(want) = expected(x, y, &tex)
                && at(x, y) != want
            {
                wrong.push(format!("({x},{y}) got {:?} want {want:?}", at(x, y)));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong pixels:\n{}",
        wrong.len(),
        wrong.join("\n")
    );

    // SetColor4(top-left red, top-right green, bottom-left black, bottom-right white)
    let near = |p: [u8; 4], c: [u8; 3]| p[..3].iter().zip(c).all(|(&a, b)| a.abs_diff(b) < 48);
    assert!(near(at(0, 16), [255, 0, 0]), "{:?}", at(0, 16));
    assert!(near(at(15, 16), [0, 255, 0]), "{:?}", at(15, 16));
    assert!(near(at(0, 31), [0, 0, 0]), "{:?}", at(0, 31));
    assert!(near(at(15, 31), [255, 255, 255]), "{:?}", at(15, 31));
}

#[test]
fn resize_and_screenshot() {
    let Some(mut g) = Gfx::headless(Size {
        width: 8,
        height: 8,
    }) else {
        eprintln!("SKIPPED: no graphics adapter");
        return;
    };
    g.resize(Size {
        width: W,
        height: H,
    });
    assert_eq!(
        g.world_size(),
        Size {
            width: W,
            height: H
        }
    );
    let mut batch = Batch::default();
    batch.clear(1.0, 0.0, 0.0);
    let world = g.world_view();
    g.render(&mut batch, &world);
    let path = std::env::temp_dir().join(format!("ddnet-rs-shot-{}.png", std::process::id()));
    g.screenshot(&path).unwrap();
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(
        (info.width, info.height, info.color_type),
        (W, H, png::ColorType::Rgb)
    );
    assert!(
        buf[..info.buffer_size()]
            .chunks(3)
            .all(|p| p == [255, 0, 0])
    );
    let _ = std::fs::remove_file(path);
}
