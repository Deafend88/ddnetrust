//! Which map images to load and from where. C++: `CMapImages::OnMapLoadImpl`
//! (`src/game/client/components/mapimages.cpp:59`) without the texture upload, and
//! `CDataFileReader::GetDataString` (datafile.cpp:795).

use super::Map;
use crate::protocol::packer::utf8_check;

const MAPITEMTYPE_IMAGE: i32 = 2;
const MAPITEMTYPE_GROUP: i32 = 4;
const MAPITEMTYPE_LAYER: i32 = 5;
const LAYERTYPE_TILES: i32 = 2;
const LAYERTYPE_QUADS: i32 = 3;
/// C++: `MAX_MAPIMAGES`.
pub const MAX_MAPIMAGES: usize = 64;
/// Int index of `m_Image` in `CMapItemLayerTilemap` and `CMapItemLayerQuads`.
const TILES_IMAGE: usize = 13;
const QUADS_IMAGE: usize = 6;

/// What the C++ client does with one map image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapImage {
    /// No tile or quad layer uses it, so it is not loaded.
    Unused,
    /// External image without a usable name.
    NoName,
    /// Version > 1 with `m_MustBe1 != 1`.
    BadType,
    /// Load `mapres/<name>.png` from storage.
    External { path: String },
    /// Non-positive width or height.
    BadSize,
    /// RGBA pixels stored in the map.
    Embedded {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    /// The pixel data is missing or too short.
    NoData,
    /// The item is shorter than the ints C++ reads (C++ reads past it; see DECISIONS.md).
    Truncated,
}

impl MapImage {
    /// The texture needs a tile layer (2D array texture) and/or a quad layer (2D).
    pub fn usage(map: &Map) -> Vec<u8> {
        let df = &map.datafile;
        let count = df.get_type(MAPITEMTYPE_IMAGE).len().min(MAX_MAPIMAGES);
        let mut used = vec![0u8; count];
        let layers = df.get_type(MAPITEMTYPE_LAYER);
        for g in df.get_type(MAPITEMTYPE_GROUP) {
            let g = &df.items[g].data; // validated by Map::load
            for l in layers.start + g[5] as usize..layers.start + (g[5] + g[6]) as usize {
                let l = &df.items[l].data;
                let (flag, slot) = match l[1] {
                    LAYERTYPE_TILES => (1, TILES_IMAGE),
                    LAYERTYPE_QUADS => (2, QUADS_IMAGE),
                    _ => continue,
                };
                // ponytail: a quads item too short for m_Image counts as no image; C++
                // reads past the item (DECISIONS.md)
                if let Some(u) = l
                    .get(slot)
                    .and_then(|&i| usize::try_from(i).ok())
                    .and_then(|i| used.get_mut(i))
                {
                    *u |= flag;
                }
            }
        }
        used
    }
}

/// C++: `CDataFileReader::GetDataString`: a NUL-terminated UTF-8 string blob without
/// inner NULs; index -1 is the empty string.
pub fn data_string(map: &Map, index: i32) -> Option<String> {
    if index == -1 {
        return Some(String::new());
    }
    let data = map.client_data(index)?;
    let (last, text) = data.split_last()?;
    if *last != 0 || text.contains(&0) || !utf8_check(text) {
        return None;
    }
    String::from_utf8(text.to_vec()).ok()
}

/// C++: the per-image decisions of `CMapImages::OnMapLoadImpl`.
pub fn map_images(map: &Map) -> Vec<MapImage> {
    let df = &map.datafile;
    let start = df.get_type(MAPITEMTYPE_IMAGE).start;
    MapImage::usage(map)
        .into_iter()
        .enumerate()
        .map(|(i, used)| {
            if used == 0 {
                return MapImage::Unused;
            }
            let img = &df.items[start + i].data;
            // CMapItemImage_v1: version, width, height, external, name, data; v2: must_be_1
            if img.len() < 6 || (img[0] > 1 && img.len() < 7) {
                return MapImage::Truncated;
            }
            let (version, width, height, external) = (img[0], img[1], img[2], img[3] != 0);
            let name = data_string(map, img[4]).filter(|n| !n.is_empty());
            if name.is_none() && external {
                return MapImage::NoName;
            }
            if version > 1 && img[6] != 1 {
                return MapImage::BadType;
            }
            if external {
                return MapImage::External {
                    path: format!("mapres/{}.png", name.unwrap_or_default()),
                };
            }
            if width <= 0 || height <= 0 {
                return MapImage::BadSize;
            }
            let size = width as usize * height as usize * 4;
            match map.client_data(img[5]) {
                Some(mut rgba) if rgba.len() >= size => {
                    rgba.truncate(size);
                    MapImage::Embedded {
                        width: width as u32,
                        height: height as u32,
                        rgba,
                    }
                }
                _ => MapImage::NoData,
            }
        })
        .collect()
}
