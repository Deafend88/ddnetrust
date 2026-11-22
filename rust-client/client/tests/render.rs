//! M6.3: map rendering maths against the C++ code: envelopes (`rs_parity envelopes`,
//! `rs_parity envelopes-synthetic`).

use ddnet::map::Map;
use ddnet_rs::render::envelope::{Bezier, EnvPoint, Envelopes, eval_points};

fn repo(path: &str) -> String {
    format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"))
}

fn bits(r: &[f32; 4]) -> String {
    r.iter()
        .map(|v| format!("{:08x}", v.to_bits()))
        .collect::<Vec<_>>()
        .join(" ")
}

const MAPS: [&str; 7] = [
    "coverage",
    "LearnToPlay",
    "Gold Mine",
    "Tutorial",
    "Tsunami",
    "ctf1",
    "dm1",
];

/// Every envelope of the 7 maps, evaluated like `CEnvelopeState::EnvelopeEval`.
#[test]
fn map_envelopes_match_cpp() {
    let mut n = 0;
    for name in MAPS {
        let map =
            Map::load(std::fs::read(repo(&format!("data/maps/{name}.map"))).unwrap()).unwrap();
        let env = Envelopes::from_map(&map);
        let text = std::fs::read_to_string(repo(&format!(
            "rust-client/tests/golden/maps/{}.envelopes.txt",
            name.replace(' ', "_")
        )))
        .unwrap();
        let mut lines = text.lines();
        let head: Vec<&str> = lines.next().unwrap().split(' ').collect();
        assert_eq!(
            (env.envelopes.len(), env.points.len()),
            (head[1].parse().unwrap(), head[3].parse().unwrap()),
            "{name}"
        );
        for line in lines {
            let f: Vec<&str> = line.split(' ').collect();
            let (index, time, offset, channels, init): (i32, i64, i32, usize, bool) = (
                f[0].parse().unwrap(),
                f[1].parse().unwrap(),
                f[2].parse().unwrap(),
                f[3].parse().unwrap(),
                f[4] == "1",
            );
            let mut r = [if init { 1.0 } else { 0.0 }; 4];
            env.eval(time, offset, index, &mut r, channels);
            assert_eq!(bits(&r), f[5..].join(" "), "{name}: {line}");
            n += 1;
        }
    }
    assert_eq!(n, (36 + 2 * 7) * 200); // 36 envelopes plus the invalid indices -1 and N per map
}

/// Random envelopes of every curve type through `RenderEvalEnvelope`.
#[test]
fn synthetic_envelopes_match_cpp() {
    let text = std::fs::read_to_string(repo("rust-client/tests/golden/envelopes.txt")).unwrap();
    let (mut points, mut beziers, mut has_bezier, mut evals) = (Vec::new(), Vec::new(), false, 0);
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "env" => {
                (points, beziers) = (Vec::new(), Vec::new());
                has_bezier = f[2] == "1";
            }
            "p" => {
                let v: Vec<i32> = f[1..].iter().map(|v| v.parse().unwrap()).collect();
                points.push(EnvPoint {
                    time: v[0],
                    curve: v[1],
                    values: [v[2], v[3], v[4], v[5]],
                });
                if has_bezier {
                    let b = |o: usize| [v[o], v[o + 1], v[o + 2], v[o + 3]];
                    beziers.push(Bezier {
                        in_dx: b(6),
                        in_dy: b(10),
                        out_dx: b(14),
                        out_dy: b(18),
                    });
                }
            }
            _ => {
                let (time, channels): (i64, usize) = (f[1].parse().unwrap(), f[2].parse().unwrap());
                let mut r = [0.0; 4];
                eval_points(
                    &points,
                    has_bezier.then_some(&beziers[..]),
                    time,
                    &mut r,
                    channels,
                );
                assert_eq!(bits(&r), f[3..].join(" "), "{line} points {points:?}");
                evals += 1;
            }
        }
    }
    assert_eq!(evals, 20_000);
}

/// `IGraphics::MapScreenToWorld` (with `CalcScreenParams`) for 5,000 random cameras and
/// group parallax/offsets (`rs_parity screen`).
#[test]
fn screen_mapping_matches_cpp() {
    use ddnet_rs::render::map_screen_to_world;
    let text = std::fs::read_to_string(repo("rust-client/tests/golden/screen.txt")).unwrap();
    let fl = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let int = |i: usize| f[i].parse::<i32>().unwrap() as f32;
        let parallax_zoom = f[4]
            .parse::<i32>()
            .unwrap()
            .max(f[5].parse().unwrap())
            .clamp(0, 100) as f32;
        let r = map_screen_to_world(
            fl(f[2]),
            fl(f[3]),
            int(4),
            int(5),
            parallax_zoom,
            int(6),
            int(7),
            fl(f[0]),
            fl(f[1]),
        );
        let got: Vec<String> = r.iter().map(|v| format!("{:08x}", v.to_bits())).collect();
        assert_eq!(got.join(" "), f[8..].join(" "), "{line}");
    }
    assert_eq!(text.lines().count(), 5000);
}
