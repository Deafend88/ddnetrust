//! Map loading. C++: `CMap::Load` (`src/engine/shared/map.cpp`) on top of the datafile,
//! and the physics layer selection of `CLayers::Init` (`src/game/layers.cpp`).

pub mod datafile;

use std::collections::{HashMap, HashSet};

use datafile::Datafile;

// C++: game/mapitems.h
const MAPITEMTYPE_VERSION: i32 = 0;
const MAPITEMTYPE_GROUP: i32 = 4;
const MAPITEMTYPE_LAYER: i32 = 5;
const LAYERTYPE_TILES: i32 = 2;
const LAYERFLAG_DETAIL: i32 = 1;
pub const TILESLAYERFLAG_GAME: i32 = 1 << 0;
pub const TILESLAYERFLAG_TELE: i32 = 1 << 1;
pub const TILESLAYERFLAG_SPEEDUP: i32 = 1 << 2;
pub const TILESLAYERFLAG_FRONT: i32 = 1 << 3;
pub const TILESLAYERFLAG_SWITCH: i32 = 1 << 4;
pub const TILESLAYERFLAG_TUNE: i32 = 1 << 5;
const PHYSICS_FLAGS: i32 = 0x3F;

// Int indices into a CMapItemLayerTilemap item.
const T_LAYER_FLAGS: usize = 2;
const T_VERSION: usize = 3;
const T_WIDTH: usize = 4;
const T_HEIGHT: usize = 5;
const T_FLAGS: usize = 6;
const T_COLOR: usize = 7; // r, g, b, a
const T_DATA: usize = 14;
const T_NAME: usize = 15; // 3 ints
/// tele, speedup, front, switch, tune
const T_PHYSICS: usize = 18;
const PHYSICS_LAYERS: [(i32, &str); 5] = [
    (TILESLAYERFLAG_TELE, "Tele"),
    (TILESLAYERFLAG_SPEEDUP, "Speedup"),
    (TILESLAYERFLAG_FRONT, "Front"),
    (TILESLAYERFLAG_SWITCH, "Switch"),
    (TILESLAYERFLAG_TUNE, "Tune"),
];
const TILEMAP_V2_INTS: usize = 15; // sizeof(CMapItemLayerTilemap_v2) / 4
const TILEMAP_V3_INTS: usize = 18; // sizeof(CMapItemLayerTilemap_v3Teeworlds) / 4
const TILEMAP_INTS: usize = 23; // sizeof(CMapItemLayerTilemap) / 4

/// What a physics layer's data index points at. C++: the `LayerType` of
/// `ValidateAndUnpackTilesLayerData`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TileKind {
    Tiles,
    Game,
    Tele,
    Speedup,
    Front,
    Switch,
    Tune,
}

impl TileKind {
    fn tile_size(self) -> usize {
        match self {
            Self::Tiles | Self::Game | Self::Front | Self::Switch => 4, // CTile, CSwitchTile
            Self::Tele | Self::Tune => 2,                               // CTeleTile, CTuneTile
            Self::Speedup => 6,                                         // CSpeedupTile
        }
    }
}

/// What a tilemap holds and which of its ints is its data index, in the order of the
/// C++ `if/else if` chains (at most one physics flag is set anyway).
fn physics_slot(flags: i32) -> (TileKind, usize) {
    const SLOTS: [(i32, TileKind, usize); 6] = [
        (TILESLAYERFLAG_GAME, TileKind::Game, T_DATA),
        (TILESLAYERFLAG_TELE, TileKind::Tele, T_PHYSICS),
        (TILESLAYERFLAG_SPEEDUP, TileKind::Speedup, T_PHYSICS + 1),
        (TILESLAYERFLAG_FRONT, TileKind::Front, T_PHYSICS + 2),
        (TILESLAYERFLAG_SWITCH, TileKind::Switch, T_PHYSICS + 3),
        (TILESLAYERFLAG_TUNE, TileKind::Tune, T_PHYSICS + 4),
    ];
    SLOTS
        .into_iter()
        .find(|(flag, _, _)| flags & flag != 0)
        .map_or((TileKind::Tiles, T_DATA), |(_, kind, slot)| (kind, slot))
}

/// The deferred check of one tile data blob. C++: the data processor lambda
/// registered by `ValidateAndUnpackTilesLayerData` (map.cpp:656).
#[derive(Clone, Copy)]
struct TileData {
    kind: TileKind,
    version: i32,
    count: usize, // width * height
}

/// A physics tile layer chosen by `CLayers::Init`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    pub width: i32,
    pub height: i32,
    pub data_index: i32,
}

pub struct Map {
    pub datafile: Datafile,
    tile_data: HashMap<i32, TileData>,
    pub game: Layer,
    pub tele: Option<Layer>,
    pub speedup: Option<Layer>,
    pub front: Option<Layer>,
    pub switch: Option<Layer>,
    pub tune: Option<Layer>,
}

/// C++: `IntsToStr` (gamecore.cpp:92): `None` if the name is not valid UTF-8.
fn ints_to_str(ints: &[i32]) -> Option<String> {
    let mut bytes: Vec<u8> = ints
        .iter()
        .flat_map(|v| v.to_be_bytes().map(|b| b ^ 0x80))
        .collect();
    *bytes.last_mut()? = 0; // ensure null termination
    bytes.truncate(bytes.iter().position(|&b| b == 0).unwrap());
    String::from_utf8(bytes).ok()
}

/// C++: `StrToInts` (gamecore.cpp:72), for names that fit.
fn str_to_ints(s: &str, n: usize) -> Vec<i32> {
    let mut bytes = s.as_bytes().to_vec();
    bytes.resize(n * 4, 0);
    let mut ints: Vec<i32> = bytes
        .chunks(4)
        .map(|c| i32::from_be_bytes([c[0] ^ 0x80, c[1] ^ 0x80, c[2] ^ 0x80, c[3] ^ 0x80]))
        .collect();
    // last byte is always zero and unused in this format
    *ints.last_mut().unwrap() &= !0xFF;
    ints
}

/// C++: `EnsureTileLayerProperties` (map.cpp:321) on a full-size tilemap item.
fn ensure_tile_layer_properties(t: &mut [i32]) -> Result<(), String> {
    if t[T_WIDTH] < 2 || t[T_HEIGHT] < 2 {
        return Err(format!(
            "tile layer has invalid size {}x{}",
            t[T_WIDTH], t[T_HEIGHT]
        ));
    }
    let flags = t[T_FLAGS];
    let name = ints_to_str(&t[T_NAME..T_NAME + 3]).ok_or("tile layer has invalid name")?;
    let physics_name = [(TILESLAYERFLAG_GAME, "Game")]
        .iter()
        .chain(&PHYSICS_LAYERS)
        .find(|(f, _)| flags & f != 0);
    match physics_name {
        Some((_, expected)) => {
            if name != *expected {
                t[T_NAME..T_NAME + 3].copy_from_slice(&str_to_ints(expected, 3));
            }
            t[T_COLOR..T_COLOR + 4].fill(255);
            t[T_LAYER_FLAGS] &= !LAYERFLAG_DETAIL;
        }
        None => {
            if t[T_COLOR..T_COLOR + 4]
                .iter()
                .any(|c| !(0..=255).contains(c))
            {
                return Err("tile layer has invalid color".into());
            }
        }
    }
    for (i, (flag, _)) in PHYSICS_LAYERS.iter().enumerate() {
        if flags & flag == 0 {
            t[T_PHYSICS + i] = -1; // unused data index
        }
    }
    Ok(())
}

/// C++: `CMap::UpgradeAndValidateTilesLayerItem` (map.cpp:462). Returns the tilemap item
/// in its latest layout.
fn upgrade_tiles_layer_item(item: &[i32]) -> Result<Vec<i32>, String> {
    let size = item.len() * 4;
    if item.len() < TILEMAP_V2_INTS {
        return Err("tile layer is truncated".into());
    }
    let version = item[T_VERSION];
    if !(2..=4).contains(&version) {
        return Err(format!("tile layer has unsupported version {version}"));
    }
    let flags = item[T_FLAGS];
    let physics = flags & PHYSICS_FLAGS;
    if physics & (physics - 1) != 0 {
        return Err("at most one physics tile layer flag can be set".into());
    }
    // Old maps contain only a prefix of the physics data indices; missing ones are -1
    // unless that layer's flag is set.
    let physics_index = |first_int: usize| -> Result<Vec<i32>, String> {
        PHYSICS_LAYERS
            .iter()
            .enumerate()
            .map(|(i, (flag, name))| match item.get(first_int + i) {
                Some(&v) => Ok(v),
                None if flags & flag != 0 => {
                    Err(format!("{name} layer is truncated (size {size})"))
                }
                None => Ok(-1),
            })
            .collect()
    };
    let mut t = if version == 2 {
        // version 2 has no name (empty), its physics indices follow the base item
        let mut t = item[..TILEMAP_V2_INTS].to_vec();
        t.extend(str_to_ints("", 3));
        t.extend(physics_index(TILEMAP_V2_INTS)?);
        t
    } else if item.len() < TILEMAP_V3_INTS {
        return Err("tile layer is truncated".into());
    } else if item.len() < TILEMAP_INTS {
        let mut t = item[..TILEMAP_V3_INTS].to_vec();
        t.extend(physics_index(TILEMAP_V3_INTS)?);
        t
    } else {
        item.to_vec() // latest version, edited in place (keeps any trailing ints)
    };
    ensure_tile_layer_properties(&mut t)?;
    Ok(t)
}

impl Map {
    /// C++: `CMap::Load` (map.cpp:89) followed by `CLayers::Init`.
    pub fn load(file: Vec<u8>) -> Result<Self, String> {
        let mut df = Datafile::parse(file)?;

        // C++: ValidateMapVersion
        let versions = df.get_type(MAPITEMTYPE_VERSION);
        if versions.len() != 1 {
            return Err(format!(
                "map must have exactly one version item, has {}",
                versions.len()
            ));
        }
        if df.items[versions.start].data.first() != Some(&1) {
            return Err("map version is missing or not supported".into());
        }

        let groups = df.get_type(MAPITEMTYPE_GROUP);
        let layers = df.get_type(MAPITEMTYPE_LAYER);
        // (group start layer, num layers), validated
        let mut group_layers = Vec::new();
        let mut used_layers = HashSet::new();
        let mut game_layer = None;
        for group in groups.clone() {
            let g = &df.items[group].data;
            if g.len() < 7 {
                return Err(format!("group {group} is truncated"));
            }
            let (start, num) = (g[5], g[6]);
            if start < 0 || num < 0 || start as i64 + num as i64 > layers.len() as i64 {
                return Err(format!("group {group} uses invalid layers"));
            }
            group_layers.push((start as usize, num as usize));
            for layer in layers.start + start as usize..layers.start + (start + num) as usize {
                if !used_layers.insert(layer) {
                    return Err(format!("layer {layer} is used by several groups"));
                }
                let l = &mut df.items[layer].data;
                if l.len() < 3 {
                    return Err(format!("layer {layer} is truncated"));
                }
                l[0] = 0; // unused version, C++ resets it
                if l[1] == LAYERTYPE_TILES {
                    *l = upgrade_tiles_layer_item(l)?;
                    if l[T_FLAGS] & TILESLAYERFLAG_GAME != 0 {
                        game_layer = Some(layer);
                    }
                }
            }
        }
        let game_layer = game_layer.ok_or("game layer is missing")?;

        // C++: ValidateAndUnpackTilesLayerData (map.cpp:575) for every tile layer
        let (game_w, game_h) = (
            df.items[game_layer].data[T_WIDTH],
            df.items[game_layer].data[T_HEIGHT],
        );
        let mut tile_data = HashMap::new();
        for &(start, num) in &group_layers {
            for layer in layers.start + start..layers.start + start + num {
                let t = &df.items[layer].data;
                if t[1] != LAYERTYPE_TILES {
                    continue;
                }
                let (kind, slot) = physics_slot(t[T_FLAGS]);
                let data_index = t[slot];
                if data_index < 0 || data_index as usize >= df.num_data() {
                    return Err(format!(
                        "tile data index {data_index} of layer {layer} is invalid"
                    ));
                }
                if tile_data.contains_key(&data_index) {
                    return Err(format!(
                        "tile data index {data_index} of layer {layer} is not unique"
                    ));
                }
                let (w, h) = (t[T_WIDTH], t[T_HEIGHT]);
                let count = w as u64 * h as u64;
                // C++ checks `(int)TilemapCount / Width != Height` and the size_t product
                if (count as u32 as i32) / w != h
                    || count.checked_mul(kind.tile_size() as u64).is_none()
                {
                    return Err(format!("tile layer {layer} is too big"));
                }
                if kind != TileKind::Tiles
                    && kind != TileKind::Game
                    && count < game_w as u64 * game_h as u64
                {
                    return Err(format!(
                        "physics layer {layer} is smaller than the game layer"
                    ));
                }
                let version = t[T_VERSION];
                tile_data.insert(
                    data_index,
                    TileData {
                        kind,
                        version,
                        count: count as usize,
                    },
                );
            }
        }

        // C++: CLayers::Init; the last layer with each physics flag wins
        let mut map = Self {
            datafile: df,
            tile_data,
            game: Layer {
                width: 0,
                height: 0,
                data_index: -1,
            },
            tele: None,
            speedup: None,
            front: None,
            switch: None,
            tune: None,
        };
        for &(start, num) in &group_layers {
            for layer in layers.start + start..layers.start + start + num {
                let t = &map.datafile.items[layer].data;
                if t[1] != LAYERTYPE_TILES {
                    continue;
                }
                let (kind, slot) = physics_slot(t[T_FLAGS]);
                let layer = Layer {
                    width: t[T_WIDTH],
                    height: t[T_HEIGHT],
                    data_index: t[slot],
                };
                match kind {
                    TileKind::Game => map.game = layer,
                    TileKind::Tele => map.tele = Some(layer),
                    TileKind::Speedup => map.speedup = Some(layer),
                    TileKind::Front => map.front = Some(layer),
                    TileKind::Switch => map.switch = Some(layer),
                    TileKind::Tune => map.tune = Some(layer),
                    TileKind::Tiles => {}
                }
            }
        }

        // load and validate the game layer data now because it is needed
        if map.data(map.game.data_index).is_none() {
            return Err("game layer data is invalid".into());
        }
        Ok(map)
    }

    /// Data blob `index`, decompressed and, for tile layers, validated or unpacked;
    /// `None` if invalid. C++: `CMap::GetData` with the registered data processors.
    pub fn data(&self, index: i32) -> Option<Vec<u8>> {
        let raw = self.datafile.data(index)?;
        let Some(spec) = self.tile_data.get(&index) else {
            return Some(raw);
        };
        let tile_size = spec.kind.tile_size();
        let saved = raw.len() / tile_size;
        if spec.version >= 4 {
            // only upstream Teeworlds writes these: tiles repeated by their m_Skip
            if !matches!(spec.kind, TileKind::Tiles | TileKind::Game) {
                return None;
            }
            let mut tiles = Vec::new();
            tiles.try_reserve_exact(spec.count * 4).ok()?;
            for t in raw.as_chunks::<4>().0.iter().take(saved) {
                if tiles.len() == spec.count * 4 {
                    return None; // too much tile layer data
                }
                if t[3] != 0 {
                    return None; // non-zero padding
                }
                for _ in 0..=t[2] {
                    if tiles.len() == spec.count * 4 {
                        break;
                    }
                    tiles.extend_from_slice(&[t[0], t[1], 0, 0]);
                }
            }
            return (tiles.len() == spec.count * 4).then_some(tiles);
        }
        if saved < spec.count {
            return None; // truncated
        }
        let tiles = raw[..spec.count * tile_size].chunks_exact(tile_size);
        let valid = match spec.kind {
            TileKind::Tiles | TileKind::Game | TileKind::Front => {
                tiles.clone().all(|t| t[2] == 0 && t[3] == 0)
            }
            TileKind::Speedup => tiles.clone().all(|t| t[3] == 0),
            _ => true,
        };
        valid.then_some(raw)
    }
}
