//! File lookup. C++: `CStorage` (`src/engine/shared/storage.cpp`), read only: the same
//! search paths (`storage.cfg` or `$USERDIR`, `$DATADIR`, `$CURRENTDIR`) in the same
//! order, the same data-dir discovery and the same refusal of paths that leave a
//! storage directory. The C++ client writes into `$USERDIR` (`%APPDATA%\DDNet`); we never
//! do: everything DDNet-rs writes goes to its own directory (`save_dir`).

use std::path::{Path, PathBuf};

/// C++: `IStorage::MAX_PATHS`.
const MAX_PATHS: usize = 16;

pub struct Storage {
    /// C++: `m_aaStoragePaths`, searched in order; an empty path is the current dir.
    pub paths: Vec<PathBuf>,
    pub user_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
}

/// C++: `fs_storage_path`.
pub fn storage_path(app: &str) -> Option<PathBuf> {
    if cfg!(windows) {
        return Some(PathBuf::from(std::env::var_os("APPDATA")?).join(app));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        return Some(home.join("Library/Application Support").join(app));
    }
    let app = app.to_lowercase();
    if app == "teeworlds" {
        return Some(home.join(".teeworlds"));
    }
    Some(match std::env::var_os("XDG_DATA_HOME") {
        Some(d) => PathBuf::from(d).join(app),
        None => home.join(".local/share").join(app),
    })
}

/// Where DDNet-rs keeps its own files (settings, screenshots).
pub fn save_dir() -> Option<PathBuf> {
    storage_path("DDNet-rs")
}

/// C++: `FindUserDirectory`: `DDNet`, or `Teeworlds` if only that one exists.
fn find_user_dir() -> Option<PathBuf> {
    let user = storage_path("DDNet");
    match storage_path("Teeworlds") {
        Some(old) if !user.as_ref().is_some_and(|u| u.is_dir()) && old.is_dir() => Some(old),
        _ => user,
    }
}

/// C++: `FindDataDirectory`, after our explicit choices (`--data-dir`,
/// `DDNET_DATA_DIR`): `data/` in the current dir, the compiled-in data dir (this
/// repo's `data/`, like C++ `DATA_DIR`), `data/` next to the executable, then Steam's
/// default install.
pub fn find_data_dir(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(d) = explicit.or_else(|| std::env::var_os("DDNET_DATA_DIR").map(PathBuf::from)) {
        return d.is_dir().then_some(d);
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.join("data")));
    let steam = r"C:\Program Files (x86)\Steam\steamapps\common\DDraceNetwork\ddnet\data";
    [
        Some(PathBuf::from("data")),
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")),
        exe_dir,
        cfg!(windows).then(|| PathBuf::from(steam)),
    ]
    .into_iter()
    .flatten()
    .find(|d| d.join("mapres").is_dir())
}

/// C++: `CLineReader`: skips a UTF-8 BOM, splits at `\n` or `\r\n`, drops lines that are
/// not valid UTF-8; a file containing a NUL byte is not read at all.
fn lines(bytes: &[u8]) -> Vec<&str> {
    if bytes.contains(&0) {
        return Vec::new();
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut out: Vec<&str> = bytes
        .split(|&b| b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .filter_map(|l| std::str::from_utf8(l).ok())
        .collect();
    if out.last() == Some(&"") {
        out.pop(); // trailing empty line
    }
    out
}

impl Storage {
    /// C++: `CStorage::Init` with the real user/data/current dirs and `storage.cfg`
    /// from the current dir or the executable's dir.
    pub fn new(data_dir: Option<PathBuf>) -> Result<Self, String> {
        let cfg = std::fs::read("storage.cfg").ok().or_else(|| {
            let exe = std::env::current_exe().ok()?;
            std::fs::read(exe.parent()?.join("storage.cfg")).ok()
        });
        Self::from_parts(find_user_dir(), find_data_dir(data_dir), cfg.as_deref())
    }

    /// The path setup of `CStorage::Init` (`LoadPathsFromFile`, `AddDefaultPaths`).
    pub fn from_parts(
        user_dir: Option<PathBuf>,
        data_dir: Option<PathBuf>,
        cfg: Option<&[u8]>,
    ) -> Result<Self, String> {
        let mut s = Self {
            paths: Vec::new(),
            user_dir,
            data_dir,
        };
        for line in cfg.map(lines).unwrap_or_default() {
            if let Some(path) = line.strip_prefix("add_path ")
                && !s.add_path(path)
                && s.paths.is_empty()
            {
                return Err("failed to add path for the user directory".into());
            }
        }
        if s.paths.is_empty() {
            if !s.add_path("$USERDIR") {
                return Err("failed to add default path for the user directory".into());
            }
            s.add_path("$DATADIR");
            s.add_path("$CURRENTDIR");
        }
        Ok(s)
    }

    /// C++: `AddPath`.
    fn add_path(&mut self, path: &str) -> bool {
        if path.is_empty() || self.paths.len() >= MAX_PATHS {
            return false;
        }
        let dir = match path {
            "$USERDIR" => self.user_dir.clone(),
            "$DATADIR" => self.data_dir.clone(),
            "$CURRENTDIR" => Some(PathBuf::new()),
            _ => Some(PathBuf::from(path)).filter(|p| p.is_dir()),
        };
        dir.map(|d| self.paths.push(d)).is_some()
    }

    /// C++: the checks of `OpenFile` before it touches the disk: a name may not leave a
    /// storage directory. `mapres/../skins/` is the one allowed exception.
    fn safe_name(name: &str) -> Option<&str> {
        let name = name
            .strip_prefix("mapres/../")
            .filter(|n| n.starts_with("skins/"))
            .unwrap_or(name);
        let escapes = name.starts_with(['/', '\\'])
            || name.contains("../")
            || name.contains("..\\")
            || (cfg!(windows) && name.as_bytes().get(1) == Some(&b':'));
        (!escapes).then_some(name)
    }

    /// C++: `OpenFile(Name, IOFLAG_READ, TYPE_ALL)` + read: the first storage path that
    /// has the file.
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let name = Self::safe_name(name)?;
        self.paths
            .iter()
            .find_map(|p| std::fs::read(p.join(name)).ok())
    }

    /// C++: `ListDirectory(TYPE_ALL, Dir)`: entry names of `dir` in every storage path,
    /// each name once (the first path that has it wins), as (name, is_dir).
    pub fn list(&self, dir: &str) -> Vec<(String, bool)> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for p in &self.paths {
            let Ok(entries) = std::fs::read_dir(p.join(dir)) else {
                continue;
            };
            for e in entries.flatten() {
                let Some(name) = e.file_name().to_str().map(String::from) else {
                    continue;
                };
                if seen.insert(name.clone()) {
                    out.push((name, e.file_type().is_ok_and(|t| t.is_dir())));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_lines_like_cline_reader() {
        assert_eq!(lines(b"\xEF\xBB\xBFa\r\nb\nc\rd\n"), ["a", "b", "c\rd"]);
        assert_eq!(lines(b"a\n\xFF\nb"), ["a", "b"]);
        assert!(lines(b"a\0b").is_empty());
    }

    #[test]
    fn names_cannot_escape() {
        for bad in ["/x", "\\x", "a/../b", "..\\b", "../b", "mapres/../maps/x"] {
            assert_eq!(Storage::safe_name(bad), None, "{bad}");
        }
        assert_eq!(
            Storage::safe_name("mapres/../skins/x.png"),
            Some("skins/x.png")
        );
        assert_eq!(Storage::safe_name("skins/a..b.png"), Some("skins/a..b.png"));
        if cfg!(windows) {
            assert_eq!(Storage::safe_name("C:x"), None);
        }
    }
}
