//! Asset lookups of the C++ client: game/particles/emoticons/hud/extras images
//! (`src/game/client/assets.cpp`), skins (`components/skins.cpp`) and map images
//! (`components/mapimages.cpp`). Files come from `Storage`; nothing is copied.
//! Cutting images into sprites and checking skin layouts come with the renderers.

use ddnet::map::Map;
use ddnet::map::images::{MapImage, map_images};
use ddnet::net::client::valid_filename;

use crate::image::{Image, load_png_file};
use crate::storage::Storage;

/// C++: `g_pData->m_aImages[...]`: asset directory and default file of each kind.
pub const GAME: (&str, &str) = ("game", "game.png");
pub const PARTICLES: (&str, &str) = ("particles", "particles.png");
pub const EMOTICONS: (&str, &str) = ("emoticons", "emoticons.png");
pub const HUD: (&str, &str) = ("hud", "hud.png");
pub const EXTRAS: (&str, &str) = ("extras", "extras.png");

/// The chosen packs: C++ `cl_asset_game`, `cl_asset_particles`, `cl_asset_emoticons`,
/// `cl_asset_hud`, `cl_asset_extras` (read from `settings_ddnet.cfg`).
#[derive(Clone, Debug, PartialEq)]
pub struct Packs {
    pub game: String,
    pub particles: String,
    pub emoticons: String,
    pub hud: String,
    pub extras: String,
}

impl Default for Packs {
    fn default() -> Self {
        let d = || "default".to_string();
        Packs {
            game: d(),
            particles: d(),
            emoticons: d(),
            hud: d(),
            extras: d(),
        }
    }
}

impl Packs {
    pub fn from_config(c: &crate::config::Config) -> Self {
        Packs {
            game: c.cl_asset_game.clone(),
            particles: c.cl_asset_particles.clone(),
            emoticons: c.cl_asset_emoticons.clone(),
            hud: c.cl_asset_hud.clone(),
            extras: c.cl_asset_extras.clone(),
        }
    }

    /// The pack chosen for an asset kind (`GAME`, `PARTICLES`, ...).
    pub fn of(&self, kind: (&str, &str)) -> &str {
        match kind.0 {
            "game" => &self.game,
            "particles" => &self.particles,
            "emoticons" => &self.emoticons,
            "hud" => &self.hud,
            _ => &self.extras,
        }
    }
}

/// C++: `CGameClient::CImageAsset`.
pub struct Asset {
    pub path: String,
    pub image: Option<Image>,
    /// The default image, for sprites the custom one leaves fully transparent.
    pub fallback: Option<Image>,
}

fn load_file(storage: &Storage, path: &str) -> Option<Image> {
    load_png_file(&storage.read(path)?)
}

/// C++: `CGameClient::LoadAssetFromPath`.
fn load_asset_from_path(storage: &Storage, kind: (&str, &str), name: &str, as_dir: bool) -> Asset {
    let (dir, file) = kind;
    let is_default = name == "default";
    let path = if is_default {
        file.to_string()
    } else if as_dir {
        format!("assets/{dir}/{name}/{file}")
    } else {
        format!("assets/{dir}/{name}.png")
    };
    let image = load_file(storage, &path);
    let fallback = (!is_default && image.is_some())
        .then(|| load_file(storage, file))
        .flatten();
    Asset {
        path,
        image,
        fallback,
    }
}

/// C++: the retry chain of `LoadGameSkin`, `LoadParticlesSkin`, ...: `name`, then
/// `name` as a directory, then `default`.
pub fn load_asset(storage: &Storage, kind: (&str, &str), name: &str) -> Asset {
    let (mut name, mut as_dir) = (name, false);
    loop {
        let asset = load_asset_from_path(storage, kind, name, as_dir);
        if asset.image.is_some() || name == "default" {
            return asset;
        }
        (name, as_dir) = if as_dir {
            ("default", false)
        } else {
            (name, true)
        };
    }
}

/// C++ `AssetScan`/`EntitiesScan` over `assets/<dir>` in every storage path:
/// "default", then the `.png` files (without the extension) and the directories, sorted;
/// names starting with '.' and "default" itself are skipped.
pub fn list_packs(storage: &Storage, dir: &str) -> Vec<String> {
    let mut names: Vec<String> = storage
        .list(&format!("assets/{dir}"))
        .into_iter()
        .filter_map(|(n, is_dir)| match is_dir {
            true => (!n.starts_with('.')).then_some(n),
            false => n.strip_suffix(".png").map(String::from),
        })
        .filter(|n| n != "default")
        .collect();
    names.sort();
    names.dedup();
    names.insert(0, "default".into());
    names
}

/// C++: `CSkin::IsValidName`: a valid filename shorter than `MAX_SKIN_LENGTH` (24) bytes.
pub fn valid_skin_name(name: &str) -> bool {
    name.len() < 24 && valid_filename(name)
}

/// C++: `CSkins::SkinScan` over `ListDirectory(TYPE_ALL, "skins")`: `.png` files with a
/// valid skin name, sorted.
pub fn list_skins(storage: &Storage) -> Vec<String> {
    let mut names: Vec<String> = storage
        .list("skins")
        .into_iter()
        .filter(|(_, is_dir)| !is_dir)
        .filter_map(|(n, _)| Some(n.strip_suffix(".png")?.to_string()))
        .filter(|n| valid_skin_name(n))
        .collect();
    names.sort();
    names
}

/// A skin's PNG: `skins/<name>.png` from any storage path, else a skin the C++ client
/// already downloaded (`downloadedskins/<name>.png` in its user dir). No downloads.
pub fn load_skin(storage: &Storage, name: &str) -> Option<Image> {
    if !valid_skin_name(name) {
        return None;
    }
    load_file(storage, &format!("skins/{name}.png")).or_else(|| {
        let user = storage.paths.first()?; // C++ TYPE_SAVE
        load_png_file(&std::fs::read(user.join(format!("downloadedskins/{name}.png"))).ok()?)
    })
}

/// The map's images (upload with `to_rgba`), `None` where the C++ client loads no texture (unused,
/// invalid, or the external file is missing or broken).
pub fn load_map_images(storage: &Storage, map: &Map) -> Vec<Option<Image>> {
    map_images(map)
        .into_iter()
        .map(|img| match img {
            MapImage::External { path } => load_file(storage, &path),
            MapImage::Embedded {
                width,
                height,
                rgba,
            } => Some(Image {
                width,
                height,
                format: crate::image::Format::Rgba,
                data: rgba,
            }),
            _ => None,
        })
        .collect()
}
