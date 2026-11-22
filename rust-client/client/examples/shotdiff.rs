//! Compares two screenshots like PLAN.md §9.4: the share of pixels whose channels all
//! differ by at most 8/255, ignoring masked rectangles, and a diff image (differing
//! pixels red, matching ones dimmed, masked ones blue).
//!
//! Usage: cargo run --example shotdiff -- <a.png> <b.png> [diff.png] [--mask X0,Y0,X1,Y1]...

use ddnet_rs::image::load_png;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut files = Vec::new();
    let mut masks: Vec<[u32; 4]> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--mask" {
            let m: Vec<u32> = it
                .next()
                .expect("--mask X0,Y0,X1,Y1")
                .split(',')
                .map(|v| v.parse().unwrap())
                .collect();
            masks.push([m[0], m[1], m[2], m[3]]);
        } else {
            files.push(a.clone());
        }
    }
    let load = |p: &str| {
        load_png(&std::fs::read(p).unwrap_or_else(|e| panic!("{p}: {e}")))
            .unwrap_or_else(|| panic!("{p}: not a PNG"))
    };
    let (a, b) = (load(&files[0]), load(&files[1]));
    assert_eq!((a.width, a.height), (b.width, b.height), "sizes differ");
    let (pa, pb) = (a.to_rgba(), b.to_rgba());
    let (mut same, mut total, mut max) = (0u64, 0u64, 0u8);
    let mut diff = Vec::with_capacity(pa.len() / 4 * 3);
    for (i, (x, y)) in pa.chunks(4).zip(pb.chunks(4)).enumerate() {
        let (px, py) = (i as u32 % a.width, i as u32 / a.width);
        if masks
            .iter()
            .any(|m| px >= m[0] && px < m[2] && py >= m[1] && py < m[3])
        {
            diff.extend_from_slice(&[0, 0, 160]);
            continue;
        }
        let d = (0..3).map(|c| x[c].abs_diff(y[c])).max().unwrap();
        max = max.max(d);
        total += 1;
        if d <= 8 {
            same += 1;
            diff.extend_from_slice(&[x[0] / 3, x[1] / 3, x[2] / 3]);
        } else {
            diff.extend_from_slice(&[255, 0, 0]);
        }
    }
    println!(
        "{}x{}: {:.3}% of {} pixels within 8/255, max channel difference {}",
        a.width,
        a.height,
        100.0 * same as f64 / total.max(1) as f64,
        total,
        max
    );
    if let Some(out) = files.get(2) {
        let mut e = png::Encoder::new(std::fs::File::create(out).unwrap(), a.width, a.height);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&diff).unwrap();
        println!("diff image: {out}");
    }
}
