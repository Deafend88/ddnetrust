//! M7.10 check: prints what the media thread sees (sessions, track, timeline, cover
//! size) once a second.
//!
//! Usage: cargo run -p ddnet-rs --example media -- [seconds]
//!        cargo run -p ddnet-rs --example media -- lyrics <title> <artist> <seconds>
//!        (one LRCLIB lookup, without the cache)

use ddnet_rs::media::{Media, time};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().map(String::as_str) == Some("lyrics") {
        let found = ddnet_rs::lyrics::fetch(&a[1], &a[2], a[3].parse().ok(), None);
        match found {
            ddnet_rs::lyrics::Lyrics::Lines(l) => {
                println!("{} lines", l.len());
                for (t, s) in l.iter().take(5) {
                    println!("{} {s}", time(*t));
                }
            }
            other => println!("{other:?}"),
        }
        return;
    }
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5);
    let media = Media::start(None);
    for _ in 0..secs {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let s = media.state();
        print!(
            "sessions {:?}, source {:?}, volume {:?}",
            s.sessions, s.source, s.volume
        );
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
