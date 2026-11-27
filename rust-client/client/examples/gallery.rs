//! Draws the design-system gallery (M7.1) headless and saves it as a PNG; with a second
//! path, also the race status review page (7.7, needs the DDNet data dir for weapons);
//! with a third, the settings page (8.8); with a fourth, the asset picker (13.4).
//!
//! Usage: cargo run -p ddnet-rs --example gallery -- [out.png] [status.png] [settings.png]
//! [assets.png]

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or("gallery.png".into());
    let Some(mut gfx) = ddnet_rs::gfx::Gfx::headless(ddnet_rs::gallery::SIZE) else {
        eprintln!("no graphics adapter");
        std::process::exit(1);
    };
    ddnet_rs::gallery::render(&mut gfx);
    save(&gfx, &out);
    if let Some(status) = args.next() {
        let storage = ddnet_rs::storage::Storage::new(None).expect("storage");
        let mut gfx =
            ddnet_rs::gfx::Gfx::headless(ddnet_rs::gallery::STATUS_SIZE).expect("graphics adapter");
        ddnet_rs::gallery::render_status(&mut gfx, &storage);
        save(&gfx, &status);
        if let Some(settings) = args.next() {
            let mut gfx = ddnet_rs::gfx::Gfx::headless(ddnet_rs::gallery::SETTINGS_SIZE)
                .expect("graphics adapter");
            ddnet_rs::gallery::render_settings(&mut gfx, &storage);
            save(&gfx, &settings);
            if let Some(assets) = args.next() {
                ddnet_rs::gallery::render_assets(&mut gfx, &storage);
                save(&gfx, &assets);
            }
        }
    }
}

fn save(gfx: &ddnet_rs::gfx::Gfx, out: &str) {
    match gfx.screenshot(std::path::Path::new(out)) {
        Ok(()) => println!("saved {out}"),
        Err(e) => eprintln!("saving {out} failed: {e}"),
    }
}
