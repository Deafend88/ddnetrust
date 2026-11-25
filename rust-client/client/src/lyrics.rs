//! Synced lyrics (7.10 review): time-stamped LRC lyrics from LRCLIB (lrclib.net) for the
//! playing track, searched by title and artist and matched by duration; off by default
//! (`hud_set island lyrics 1`). Results, also "none found", are cached in our save
//! folder (`lyrics/`); a failed request (offline) is not cached and retried on the
//! next track. Windows: the request goes through WinRT's `HttpClient` and `JsonArray`.

use std::path::PathBuf;
use std::sync::Arc;

/// The lyrics of the playing track.
#[derive(Clone, Debug, PartialEq)]
pub enum Lyrics {
    Loading,
    /// LRCLIB has no synced lyrics for the track.
    NotFound,
    /// The request failed (offline, server error).
    Unavailable,
    /// (start in seconds, text), sorted by time.
    Lines(Arc<Vec<(f64, String)>>),
}

/// The lines of an LRC text: "[mm:ss.xx]text" (several tags on one line allowed;
/// metadata tags like "[ar:...]" and lines without a time are skipped), sorted by time.
pub fn parse(lrc: &str) -> Vec<(f64, String)> {
    let mut lines = Vec::new();
    for line in lrc.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(tag) = rest.strip_prefix('[') {
            let Some((stamp, after)) = tag.split_once(']') else {
                break;
            };
            let Some(t) = stamp_seconds(stamp) else {
                break;
            };
            times.push(t);
            rest = after;
        }
        let text = rest.trim().to_string();
        lines.extend(times.into_iter().map(|t| (t, text.clone())));
    }
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    lines
}

/// "mm:ss.xx" (or "mm:ss") as seconds.
fn stamp_seconds(stamp: &str) -> Option<f64> {
    let (m, s) = stamp.split_once(':')?;
    let (m, s): (u32, f64) = (m.trim().parse().ok()?, s.trim().parse().ok()?);
    (s.is_finite() && s >= 0.0).then_some(m as f64 * 60.0 + s)
}

/// The index of the line sung at `position` (the last one started), if any.
pub fn current(lines: &[(f64, String)], position: f64) -> Option<usize> {
    lines.partition_point(|l| l.0 <= position).checked_sub(1)
}

/// The search result that fits: synced lyrics, and the duration within 3 s of the
/// track's when both are known (the closest one); `(duration, synced lyrics)` pairs.
pub fn best(results: &[(Option<f64>, Option<String>)], duration: Option<f64>) -> Option<&str> {
    let synced = results
        .iter()
        .filter_map(|(d, l)| Some((*d, l.as_deref().filter(|l| !l.trim().is_empty())?)));
    match duration {
        Some(want) => synced
            .filter_map(|(d, l)| Some(((d? - want).abs(), l)))
            .filter(|(off, _)| *off <= 3.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, l)| l),
        None => synced.map(|(_, l)| l).next(),
    }
}

/// The cache file of a track: an FNV-1a hash of artist, title and whole seconds.
fn cache_file(dir: &std::path::Path, title: &str, artist: &str, duration: Option<f64>) -> PathBuf {
    let key = format!(
        "{artist}\n{title}\n{}",
        duration.map_or(-1, |d| d.round() as i64)
    );
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    });
    dir.join(format!("{hash:016x}.lrc"))
}

/// The lyrics of a track: from the cache, else from LRCLIB (blocking; call it on a
/// background thread). `dir`: the cache folder (none: no cache).
pub fn fetch(title: &str, artist: &str, duration: Option<f64>, dir: Option<PathBuf>) -> Lyrics {
    let file = dir.as_ref().map(|d| cache_file(d, title, artist, duration));
    if let Some(text) = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()) {
        return lines_or_not_found(&text);
    }
    let Some(found) = search(title, artist, duration) else {
        return Lyrics::Unavailable;
    };
    let text = found.unwrap_or_default(); // empty: none found, cached as such
    if let (Some(dir), Some(file)) = (&dir, &file) {
        let _ = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(file, &text));
    }
    lines_or_not_found(&text)
}

fn lines_or_not_found(text: &str) -> Lyrics {
    let lines = parse(text);
    if lines.is_empty() {
        Lyrics::NotFound
    } else {
        Lyrics::Lines(Arc::new(lines))
    }
}

/// LRCLIB's search: `Some(None)` when it answered without fitting synced lyrics,
/// `None` when the request failed.
#[cfg(windows)]
fn search(title: &str, artist: &str, duration: Option<f64>) -> Option<Option<String>> {
    use windows::Data::Json::{JsonArray, JsonValueType};
    use windows::Foundation::Uri;
    use windows::Web::Http::HttpClient;
    use windows::core::HSTRING;

    let request = || -> windows::core::Result<Option<String>> {
        let escape = |s: &str| Uri::EscapeComponent(&HSTRING::from(s));
        let url = format!(
            "https://lrclib.net/api/search?track_name={}&artist_name={}",
            escape(title)?,
            escape(artist)?
        );
        let client = HttpClient::new()?;
        // LRCLIB asks for the client's name; nothing about the user
        client
            .DefaultRequestHeaders()?
            .UserAgent()?
            .TryParseAdd(&HSTRING::from("DDNet-rs/0.1.0"))?;
        let response = client
            .GetAsync(&Uri::CreateUri(&HSTRING::from(url))?)?
            .join()?;
        if !response.IsSuccessStatusCode()? {
            return Err(windows::core::Error::empty());
        }
        let text = response.Content()?.ReadAsStringAsync()?.join()?;
        let list = JsonArray::Parse(&text)?;
        let mut results = Vec::new();
        for i in 0..list.Size()? {
            let Ok(o) = list.GetObjectAt(i) else { continue };
            let get = |k: &str| o.GetNamedValue(&HSTRING::from(k)).ok();
            let duration = get("duration")
                .filter(|v| v.ValueType().is_ok_and(|t| t == JsonValueType::Number))
                .and_then(|v| v.GetNumber().ok());
            let synced = get("syncedLyrics")
                .filter(|v| v.ValueType().is_ok_and(|t| t == JsonValueType::String))
                .and_then(|v| v.GetString().ok())
                .map(|s| s.to_string());
            results.push((duration, synced));
        }
        Ok(best(&results, duration).map(str::to_string))
    };
    request().ok()
}

#[cfg(not(windows))]
fn search(_: &str, _: &str, _: Option<f64>) -> Option<Option<String>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_lines_and_the_current_one() {
        let lrc = "[ar:Someone]\n[00:12.50]first\n[00:05.00][01:00.00]twice\n\nno time\n[xx:1]bad\n[00:20]  third  ";
        let lines = parse(lrc);
        let want = [
            (5.0, "twice"),
            (12.5, "first"),
            (20.0, "third"),
            (60.0, "twice"),
        ];
        assert_eq!(lines.len(), want.len());
        for ((t, s), (wt, ws)) in lines.iter().zip(want) {
            assert_eq!((*t, s.as_str()), (wt, ws));
        }
        assert_eq!(current(&lines, 0.0), None);
        assert_eq!(current(&lines, 5.0), Some(0));
        assert_eq!(current(&lines, 19.9), Some(1));
        assert_eq!(current(&lines, 600.0), Some(3));
        assert!(parse("").is_empty());
    }

    #[test]
    fn best_result_matches_the_duration() {
        let r = |d: Option<f64>, l: Option<&str>| (d, l.map(str::to_string));
        let results = [
            r(Some(200.0), Some("[00:01]far")),
            r(Some(181.0), None),
            r(Some(182.0), Some("[00:01]near")),
            r(Some(179.5), Some(" ")),
        ];
        assert_eq!(best(&results, Some(180.0)), Some("[00:01]near"));
        assert_eq!(best(&results, Some(100.0)), None);
        assert_eq!(best(&results, None), Some("[00:01]far"));
        assert_eq!(best(&[], None), None);
    }

    #[test]
    fn cache_hits_and_none_found() {
        let dir = std::env::temp_dir().join(format!("ddnet-rs-lyrics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(cache_file(&dir, "t", "a", Some(180.2)), "[00:01.00]hi").unwrap();
        std::fs::write(cache_file(&dir, "none", "a", None), "").unwrap();
        let hit = fetch("t", "a", Some(179.8), Some(dir.clone()));
        assert_eq!(hit, Lyrics::Lines(Arc::new(vec![(1.0, "hi".into())])));
        assert_eq!(
            fetch("none", "a", None, Some(dir.clone())),
            Lyrics::NotFound
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
