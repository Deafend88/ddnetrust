//! Draws the design-system gallery (M7.1) headless and saves it as a PNG.
//!
//! Usage: cargo run -p ddnet-rs --example gallery -- [out.png]

fn main() {
    let out = std::env::args().nth(1).unwrap_or("gallery.png".into());
    let Some(mut gfx) = ddnet_rs::gfx::Gfx::headless(ddnet_rs::gallery::SIZE) else {
        eprintln!("no graphics adapter");
        std::process::exit(1);
    };
    ddnet_rs::gallery::render(&mut gfx);
    match gfx.screenshot(std::path::Path::new(&out)) {
        Ok(()) => println!("saved {out}"),
        Err(e) => eprintln!("saving {out} failed: {e}"),
    }
}
