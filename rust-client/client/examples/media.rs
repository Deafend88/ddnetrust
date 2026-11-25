//! M7.10 check: prints what the media thread sees (sessions, track, timeline, cover
//! size) once a second.
//!
//! Usage: cargo run -p ddnet-rs --example media -- [seconds]

use ddnet_rs::media::{Media, time};

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5);
    let media = Media::start(None);
    for _ in 0..secs {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let s = media.state();
        print!("sessions {:?}, source {:?}", s.sessions, s.source);
        if let Some(t) = &s.track {
            let pos = t.position(std::time::Instant::now());
            let pos = pos.map(|(p, d)| format!("{} / {}", time(p), time(d)));
            let cover = t.cover.as_ref().map(|c| c.size);
            print!(
                ": {:?} - {:?}, playing {}, {pos:?}, cover {cover:?}",
                t.title, t.artist, t.playing
            );
        }
        println!();
    }
}
