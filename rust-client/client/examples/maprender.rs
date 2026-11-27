//! Renders a map's layers at a fixed view on the GPU, without a server, and saves the
//! picture: for comparing the map renderer against a C++ screenshot of the same view.
//!
//! Usage: cargo run --example maprender -- <map file> <center x> <center y> <WxH> <out.png>

use ddnet::map::Map;
use ddnet_rs::assets;
use ddnet_rs::gfx::batch::Batch;
use ddnet_rs::gfx::{Gfx, Size};
use ddnet_rs::render::map::{MapRender, View, hsl_packed_to_rgb};
use ddnet_rs::storage::Storage;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (w, h) = a[3].split_once('x').expect("WxH");
    let size = Size {
        width: w.parse().unwrap(),
        height: h.parse().unwrap(),
    };
    let storage = Storage::new(None).unwrap();
    let map = Map::load(std::fs::read(&a[0]).unwrap()).unwrap();
    let mut gfx = Gfx::headless(size).expect("graphics adapter");
    let render = MapRender::new(&mut gfx, &map, &assets::load_map_images(&storage, &map));
    let view = View {
        center: (a[1].parse().unwrap(), a[2].parse().unwrap()),
        zoom: 1.0,
        width: size.width,
        height: size.height,
        envelope_time: 0,
        overlay: 0,
    };
    let mut batch = Batch::default();
    let [r, g, b] = hsl_packed_to_rgb(128); // C++ cl_background_color
    batch.clear(r, g, b);
    render.render_background(&mut batch, &view);
    render.render_foreground(&mut batch, &view);
    let world = gfx.world_view();
    gfx.render(&mut batch, &world, size);
    gfx.screenshot(std::path::Path::new(&a[4])).unwrap();
}
