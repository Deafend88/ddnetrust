//! Per-app volume (7.10 review): the volume of one app's audio sessions, like the
//! Windows Volume Mixer, never the game's own sound. Windows offers this only through
//! Core Audio COM interfaces (`IAudioSessionManager2`, `IAudioSessionControl2`,
//! `ISimpleAudioVolume`), which the `windows` crate exposes as `unsafe fn`s, and no
//! maintained crate wraps session volume safely (DECISIONS.md). This module is the one
//! exception to the project's no-`unsafe` rule (CLAUDE.md): FFI to OS APIs only, behind
//! the safe `volume` / `set_volume`, each call with its SAFETY comment.

/// Whether an audio session belongs to the media session's app. `aumid` is SMTC's app id
/// ("Spotify.exe", "Chrome", or a packaged app's "Publisher.Name_hash!App"); the session
/// identifier holds the executable's path ("...|\Device\...\Spotify.exe%b{...}"), which
/// for packaged apps lies under the package's folder.
pub fn matches(aumid: &str, identifier: &str) -> bool {
    let id = identifier.to_lowercase();
    let package = aumid
        .split(['!', '_'])
        .next()
        .unwrap_or(aumid)
        .to_lowercase();
    let name = format!("\\{}.exe", crate::island::source_name(aumid).to_lowercase());
    (!package.is_empty() && package.len() >= 4 && id.contains(&package))
        || (name.len() > "\\.exe".len() && id.contains(&name))
}

/// The app's volume (0..1) on the default output device: its first session's; `None`
/// when it has no audio session (or Windows says no).
pub fn volume(aumid: &str) -> Option<f32> {
    imp::sessions(aumid).into_iter().find_map(|s| imp::get(&s))
}

/// Sets the volume (0..1) of every audio session of the app; false when it has none.
pub fn set_volume(aumid: &str, level: f32) -> bool {
    let sessions = imp::sessions(aumid);
    let level = level.clamp(0.0, 1.0);
    !sessions.is_empty() && sessions.iter().all(|s| imp::set(s, level))
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Media::Audio::{
        IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume,
        MMDeviceEnumerator, eMultimedia, eRender,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    };
    use windows::core::Interface;

    /// The app's sessions on the default output device (looked up each time, so a
    /// changed default device is followed).
    pub fn sessions(aumid: &str) -> Vec<ISimpleAudioVolume> {
        let all = || -> windows::core::Result<Vec<ISimpleAudioVolume>> {
            // SAFETY: COM initialisation for this thread with no reserved pointer; a
            // thread already initialised (as MTA) returns S_FALSE, which is harmless.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            // SAFETY: creates the system's device enumerator by its documented CLSID; no
            // outer object, the result is an owned COM interface.
            let devices: IMMDeviceEnumerator =
                unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
            // SAFETY: plain enum arguments; returns an owned IMMDevice.
            let device = unsafe { devices.GetDefaultAudioEndpoint(eRender, eMultimedia)? };
            // SAFETY: activates the documented session manager interface; no
            // activation parameters (None is allowed).
            let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
            // SAFETY: no arguments; returns an owned enumerator.
            let list = unsafe { manager.GetSessionEnumerator()? };
            // SAFETY: no arguments.
            let n = unsafe { list.GetCount()? };
            let mut found = Vec::new();
            for i in 0..n {
                // SAFETY: `i` is within 0..GetCount().
                let Ok(control) = (unsafe { list.GetSession(i) }) else {
                    continue;
                };
                let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                // SAFETY: returns a CoTaskMemAlloc'd string, read once and then freed
                // with CoTaskMemFree as the API requires; not used afterwards.
                let id = unsafe {
                    control.GetSessionIdentifier().map(|p| {
                        let s = p.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(p.0 as *const _));
                        s
                    })
                };
                if id.is_ok_and(|id| super::matches(aumid, &id))
                    && let Ok(volume) = control.cast::<ISimpleAudioVolume>()
                {
                    found.push(volume);
                }
            }
            Ok(found)
        };
        all().unwrap_or_default()
    }

    pub fn get(v: &ISimpleAudioVolume) -> Option<f32> {
        // SAFETY: no arguments; reads the session's volume level.
        unsafe { v.GetMasterVolume() }.ok()
    }

    pub fn set(v: &ISimpleAudioVolume, level: f32) -> bool {
        // SAFETY: `level` is clamped to 0..1 by the caller; a null event context is
        // allowed (no GUID is reported to the session's event listeners).
        unsafe { v.SetMasterVolume(level, std::ptr::null()) }.is_ok()
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct Volume;
    pub fn sessions(_: &str) -> Vec<Volume> {
        Vec::new()
    }
    pub fn get(_: &Volume) -> Option<f32> {
        None
    }
    pub fn set(_: &Volume, _: f32) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_match_their_app() {
        let spotify = r"{0.0.0.00000000}.{8d4a}|\Device\HarddiskVolume3\Users\x\AppData\Roaming\Spotify\Spotify.exe%b{00000000-0000-0000-0000-000000000000}";
        let store = r"{0.0.0.00000000}.{1f2e}|\Device\HarddiskVolume3\Program Files\WindowsApps\A025C540.Yandex.Music_4.40.0.0_x64__vfvw9svesycw6\Yandex Music.exe%b{00000000-0000-0000-0000-000000000000}";
        let chrome = r"{0.0.0.00000000}.{aa}|\Device\HarddiskVolume3\Program Files\Google\Chrome\Application\chrome.exe%b{00000000-0000-0000-0000-000000000000}";
        assert!(matches("Spotify.exe", spotify));
        assert!(matches(
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            spotify
        ));
        assert!(matches("A025C540.Yandex.Music_vfvw9svesycw6!App", store));
        assert!(matches("Chrome", chrome));
        assert!(!matches("Spotify.exe", chrome));
        assert!(!matches("Chrome", spotify));
        assert!(!matches("A025C540.Yandex.Music_vfvw9svesycw6!App", spotify));
        assert!(!matches("", spotify));
    }
}
