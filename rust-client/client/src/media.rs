//! Now playing (PLAN §10.3, 7.10): the OS media session, read and controlled on a
//! background thread. Windows: System Media Transport Controls (SMTC), polled every
//! 0.5 s; the user's own play/pause, next, previous and seek go to the chosen session.
//! Other OSes: nothing (every media part hides).

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::lyrics::{self, Lyrics};

/// A cover image, RGBA8 straight alpha; `id` changes with every new image.
#[derive(Clone, Debug, PartialEq)]
pub struct Cover {
    pub id: u64,
    pub size: [usize; 2],
    pub rgba: Arc<Vec<u8>>,
}

/// The playing track of the chosen session.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Position and duration in seconds, when the source reports a timeline.
    pub timeline: Option<(f64, f64)>,
    /// When `timeline` was read (the position moves on from there while playing).
    pub at: Instant,
    pub cover: Option<Cover>,
    pub can_prev: bool,
    pub can_next: bool,
    pub can_seek: bool,
}

impl Track {
    /// The position now, in seconds, within the duration.
    pub fn position(&self, now: Instant) -> Option<(f64, f64)> {
        let (pos, dur) = self.timeline?;
        let moved = if self.playing {
            now.saturating_duration_since(self.at).as_secs_f64()
        } else {
            0.0
        };
        Some(((pos + moved).clamp(0.0, dur.max(0.0)), dur))
    }
}

/// What the thread last saw: the sessions (app ids) and the chosen one's track.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub sessions: Vec<String>,
    /// The session shown (the chosen one, else the system's current one).
    pub source: Option<String>,
    /// The session the user chose (`None`: automatic).
    pub chosen: Option<String>,
    pub track: Option<Track>,
    /// The track's synced lyrics while they are on (`Command::Lyrics`).
    pub lyrics: Option<Lyrics>,
}

/// A user's command for the chosen session.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    PlayPause,
    Next,
    Prev,
    /// To a position in seconds.
    Seek(f64),
    /// Choose a session by app id (`None`: the system's current one).
    Source(Option<String>),
    /// Fetch synced lyrics for the tracks (LRCLIB) or not.
    Lyrics(bool),
}

/// The handle the game holds.
pub struct Media {
    state: Arc<Mutex<State>>,
    commands: Sender<Command>,
}

impl Media {
    /// Starts the background thread (on Windows) with the saved source (`None`:
    /// automatic).
    pub fn start(source: Option<String>) -> Self {
        let state = State {
            chosen: source.clone(),
            ..State::default()
        };
        let state = Arc::new(Mutex::new(state));
        let (commands, rx) = channel();
        let shared = state.clone();
        let _ = std::thread::Builder::new()
            .name("media".into())
            .spawn(move || run(&shared, &rx, source));
        Self { state, commands }
    }

    /// The last state seen.
    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

/// (title, artist) and their lyrics so far.
type TrackLyrics = ((String, String), Lyrics);

/// The lyrics of the playing track: fetched on their own thread when the track changes
/// (a slow request never stalls the polling).
#[derive(Default)]
struct LyricsJob {
    enabled: bool,
    shared: Arc<Mutex<Option<TrackLyrics>>>,
}

impl LyricsJob {
    #[cfg_attr(not(windows), allow(dead_code))]
    fn update(&mut self, track: Option<&Track>) -> Option<Lyrics> {
        let track = track.filter(|_| self.enabled)?;
        let key = (track.title.clone(), track.artist.clone());
        let mut shared = self.shared.lock().ok()?;
        match &*shared {
            Some((k, l)) if *k == key => return Some(l.clone()),
            _ => *shared = Some((key.clone(), Lyrics::Loading)),
        }
        let (slot, duration) = (self.shared.clone(), track.timeline.map(|t| t.1));
        let dir = crate::storage::save_dir().map(|d| d.join("lyrics"));
        let _ = std::thread::Builder::new()
            .name("lyrics".into())
            .spawn(move || {
                let found = lyrics::fetch(&key.0, &key.1, duration, dir);
                if let Ok(mut s) = slot.lock()
                    && s.as_ref().is_some_and(|(k, _)| *k == key)
                {
                    *s = Some((key, found));
                }
            });
        Some(Lyrics::Loading)
    }
}

/// m:ss, or h:mm:ss from an hour on (like the iOS player).
pub fn time(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// SMTC's timeline as seconds: `position` and the span `start..end` in 100 ns ticks;
/// `updated` and `now` as Windows file times (100 ns since 1601). A playing source
/// reports the position at `updated`; it has moved on since. `None` without a timeline.
pub fn timeline(
    position: i64,
    start: i64,
    end: i64,
    updated: i64,
    now: i64,
    playing: bool,
) -> Option<(f64, f64)> {
    const TICK: f64 = 1e-7;
    let duration = end.saturating_sub(start) as f64 * TICK;
    if duration <= 0.0 {
        return None;
    }
    let moved = if playing && updated > 0 {
        now.saturating_sub(updated).max(0) as f64 * TICK
    } else {
        0.0
    };
    let pos = position.saturating_sub(start) as f64 * TICK + moved;
    Some((pos.clamp(0.0, duration), duration))
}

#[cfg(not(windows))]
fn run(_: &Mutex<State>, _: &Receiver<Command>, _: Option<String>) {}

#[cfg(windows)]
fn run(shared: &Mutex<State>, rx: &Receiver<Command>, source: Option<String>) {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;
    let mut smtc = smtc::Smtc::default();
    smtc.command(Command::Source(source));
    let mut lyrics = LyricsJob::default();
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Command::Lyrics(on)) => lyrics.enabled = on,
            Ok(command) => smtc.command(command),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let mut state = smtc.poll();
        state.lyrics = lyrics.update(state.track.as_ref());
        if let Ok(mut s) = shared.lock() {
            *s = state;
        }
    }
}

#[cfg(windows)]
mod smtc {
    use std::sync::Arc;
    use std::time::Instant;

    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
        ExifOrientationMode,
    };
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::core::Result;

    use super::{Command, Cover, State, Track};

    /// The thread's WinRT side; errors (no session, a player closing) just mean nothing
    /// to show this poll.
    #[derive(Default)]
    pub struct Smtc {
        manager: Option<Manager>,
        chosen: Option<String>,
        /// The last track's (title, artist, source) and its cover.
        last: Option<((String, String, String), Option<Cover>)>,
        covers: u64,
    }

    impl Smtc {
        fn manager(&mut self) -> Option<&Manager> {
            if self.manager.is_none() {
                self.manager = Manager::RequestAsync().and_then(|op| op.join()).ok();
            }
            self.manager.as_ref()
        }

        fn sessions(&mut self) -> Vec<Session> {
            let Some(list) = self.manager().and_then(|m| m.GetSessions().ok()) else {
                return Vec::new();
            };
            let n = list.Size().unwrap_or(0);
            (0..n).filter_map(|i| list.GetAt(i).ok()).collect()
        }

        /// The chosen session if it still exists, else the system's current one.
        fn session(&mut self) -> Option<Session> {
            let chosen = self.chosen.clone();
            let sessions = self.sessions();
            let pick = chosen.and_then(|c| {
                (sessions.into_iter()).find(|s| s.SourceAppUserModelId().is_ok_and(|id| id == c))
            });
            pick.or_else(|| self.manager()?.GetCurrentSession().ok())
        }

        pub fn command(&mut self, command: Command) {
            if let Command::Source(id) = command {
                self.chosen = id;
                return;
            }
            let Some(s) = self.session() else { return };
            let op = match command {
                Command::PlayPause => s.TryTogglePlayPauseAsync(),
                Command::Next => s.TrySkipNextAsync(),
                Command::Prev => s.TrySkipPreviousAsync(),
                // seconds -> 100 ns ticks from the timeline's start
                Command::Seek(secs) => {
                    let start = s.GetTimelineProperties().and_then(|t| t.StartTime());
                    let start = start.map_or(0, |t| t.Duration);
                    s.TryChangePlaybackPositionAsync(start + (secs.max(0.0) * 1e7) as i64)
                }
                Command::Source(_) | Command::Lyrics(_) => return,
            };
            let _ = op.and_then(|op| op.join());
        }

        pub fn poll(&mut self) -> State {
            let sessions = (self.sessions().iter())
                .filter_map(|s| s.SourceAppUserModelId().ok())
                .map(|id| id.to_string())
                .collect();
            let session = self.session();
            let source = (session.as_ref())
                .and_then(|s| s.SourceAppUserModelId().ok())
                .map(|id| id.to_string());
            let track = session.and_then(|s| self.track(&s, source.clone()?).ok());
            State {
                sessions,
                source,
                chosen: self.chosen.clone(),
                track,
                lyrics: None,
            }
        }

        fn track(&mut self, s: &Session, source: String) -> Result<Track> {
            let props = s.TryGetMediaPropertiesAsync()?.join()?;
            let (title, artist) = (props.Title()?.to_string(), props.Artist()?.to_string());
            let key = (title.clone(), artist.clone(), source);
            // the cover is decoded once per track
            let cover = match &self.last {
                Some((k, cover)) if *k == key => cover.clone(),
                _ => {
                    self.covers += 1;
                    let cover = props.Thumbnail().and_then(|t| decode(&t, self.covers)).ok();
                    self.last = Some((key, cover.clone()));
                    cover
                }
            };
            let info = s.GetPlaybackInfo()?;
            let playing = info.PlaybackStatus()? == Status::Playing;
            let controls = info.Controls()?;
            let t = s.GetTimelineProperties()?;
            let at = Instant::now();
            let timeline = super::timeline(
                t.Position()?.Duration,
                t.StartTime()?.Duration,
                t.EndTime()?.Duration,
                t.LastUpdatedTime()?.UniversalTime,
                now_filetime(),
                playing,
            );
            Ok(Track {
                title,
                artist,
                playing,
                timeline,
                at,
                cover,
                can_prev: controls.IsPreviousEnabled()?,
                can_next: controls.IsNextEnabled()?,
                can_seek: controls.IsPlaybackPositionEnabled()?,
            })
        }
    }

    /// Now as a Windows file time (100 ns since 1601-01-01 UTC).
    fn now_filetime() -> i64 {
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() / 100);
        unix as i64 + 116_444_736_000_000_000
    }

    /// The thumbnail through Windows' own image decoder (JPEG, PNG, ...), as RGBA8.
    fn decode(
        thumb: &windows::Storage::Streams::IRandomAccessStreamReference,
        id: u64,
    ) -> Result<Cover> {
        let stream = thumb.OpenReadAsync()?.join()?;
        let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
        let (w, h) = (
            decoder.OrientedPixelWidth()?,
            decoder.OrientedPixelHeight()?,
        );
        let pixels = decoder
            .GetPixelDataTransformedAsync(
                BitmapPixelFormat::Rgba8,
                BitmapAlphaMode::Straight,
                &BitmapTransform::new()?,
                ExifOrientationMode::RespectExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )?
            .join()?
            .DetachPixelData()?;
        if pixels.len() != w as usize * h as usize * 4 {
            return Err(windows::core::Error::empty());
        }
        Ok(Cover {
            id,
            size: [w as usize, h as usize],
            rgba: Arc::new(pixels.to_vec()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_timeline() {
        assert_eq!(time(0.0), "0:00");
        assert_eq!(time(133.9), "2:13");
        assert_eq!(time(-5.0), "0:00");
        assert_eq!(time(3725.0), "1:02:05");
        const S: i64 = 10_000_000;
        // paused: the reported position
        assert_eq!(
            timeline(30 * S, 0, 180 * S, 1, 100 * S, false),
            Some((30.0, 180.0))
        );
        // playing: moved on since the update, clamped to the end
        assert_eq!(
            timeline(30 * S, 0, 180 * S, 90 * S, 100 * S, true),
            Some((40.0, 180.0))
        );
        assert_eq!(
            timeline(30 * S, 0, 180 * S, 0, 1000 * S, true),
            Some((30.0, 180.0))
        );
        assert_eq!(
            timeline(179 * S, 0, 180 * S, 0, 10, true).map(|t| t.0),
            Some(179.0)
        );
        assert_eq!(
            timeline(500 * S, 0, 180 * S, 1, 1, false),
            Some((180.0, 180.0))
        );
        // a start offset; no timeline
        assert_eq!(
            timeline(15 * S, 10 * S, 70 * S, 1, 1, false),
            Some((5.0, 60.0))
        );
        assert_eq!(timeline(0, 0, 0, 0, 0, true), None);
        // hostile values do not panic
        timeline(i64::MIN, i64::MAX, i64::MIN, i64::MAX, i64::MIN, true);
        // a playing track moves on locally
        let t = Track {
            title: String::new(),
            artist: String::new(),
            playing: true,
            timeline: Some((10.0, 12.0)),
            at: Instant::now(),
            cover: None,
            can_prev: true,
            can_next: true,
            can_seek: true,
        };
        let later = t.at + std::time::Duration::from_secs(5);
        assert_eq!(t.position(later), Some((12.0, 12.0)));
    }
}
