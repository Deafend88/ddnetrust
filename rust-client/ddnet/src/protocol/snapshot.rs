//! Snapshots and snapshot deltas. C++: `src/engine/shared/snapshot.{h,cpp}`, plus
//! `CClient::UnpackAndValidateSnapshot` (client.cpp:2432).

use super::generated::OBJ_SPECS;
use super::netobj::secure_unpack_obj;

pub const MAX_TYPE: i32 = 0x7fff;
pub const MAX_ID: i32 = 0xffff;
pub const MAX_ITEMS: usize = 1024;
pub const MAX_PARTS: usize = 64;
/// Bytes. C++: `CSnapshot::MAX_SIZE`.
pub const MAX_SIZE: usize = MAX_PARTS * 1024;
/// Internal item types from here on are extended (UUID) types.
pub const OFFSET_UUID_TYPE: i32 = 0x4000;
/// Static item sizes exist for types below this. C++: `CSnapshotDelta::MAX_NETOBJSIZES`.
pub const MAX_NETOBJSIZES: usize = 64;
/// C++: `CSnapshotDeltaBuffer` holds `2 * MAX_SIZE` bytes.
pub const MAX_DELTA_INTS: usize = 2 * MAX_SIZE / 4;

/// C++: `CSnapshot`, in its exact memory layout as `i32`s:
/// `[data_size, num_items, offsets.., items..]`, each item `[type << 16 | id, data..]`,
/// offsets in bytes from the start of the item data. Only built by `unpack_delta`,
/// so the layout is always valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    buf: Vec<i32>,
}

impl Default for Snapshot {
    /// C++: `CSnapshot::EmptySnapshot()`.
    fn default() -> Self {
        Self { buf: vec![0, 0] }
    }
}

impl Snapshot {
    pub fn num_items(&self) -> usize {
        self.buf[1] as usize
    }

    fn item_start(&self, index: usize) -> usize {
        2 + self.num_items() + self.buf[2 + index] as usize / 4
    }

    /// `type << 16 | id`. C++: `CSnapshotItem::Key`.
    pub fn key(&self, index: usize) -> i32 {
        self.buf[self.item_start(index)]
    }

    /// C++: `CSnapshotItem::Data` with `CSnapshot::GetItemSize`.
    pub fn data(&self, index: usize) -> &[i32] {
        let end = if index + 1 == self.num_items() {
            2 + self.num_items() + self.buf[0] as usize / 4
        } else {
            self.item_start(index + 1)
        };
        &self.buf[self.item_start(index) + 1..end]
    }

    /// C++: `CSnapshot::GetItemIndex` (linear search, first match).
    pub fn index_of(&self, key: i32) -> Option<usize> {
        (0..self.num_items()).find(|&i| self.key(i) == key)
    }

    /// C++: `CSnapshot::GetItemType` + `GetExternalItemType` (snapshot.cpp:36). Extended
    /// types resolve through the UUID stored in the type-0 item whose id is the internal
    /// type; returns `None` for unknown UUIDs (C++: `UUID_UNKNOWN`).
    pub fn item_type(&self, index: usize) -> Option<i32> {
        let internal = self.key(index) >> 16;
        if internal < OFFSET_UUID_TYPE {
            return Some(internal);
        }
        let data = self.data(self.index_of(internal)?);
        let uuid: [u8; 16] = data
            .get(..4)?
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect::<Vec<_>>()
            .try_into()
            .ok()?;
        OBJ_SPECS
            .iter()
            .find(|s| s.uuid == Some(uuid))
            .map(|s| s.type_id)
    }

    /// C++: `CSnapshot::Crc`, wrapping sum of all item data ints.
    pub fn crc(&self) -> u32 {
        (0..self.num_items())
            .flat_map(|i| self.data(i))
            .fold(0u32, |crc, &v| crc.wrapping_add(v as u32))
    }

    /// The raw layout, as the C++ `CSnapshot` bytes would read.
    pub fn as_ints(&self) -> &[i32] {
        &self.buf
    }
}

/// C++: `CSnapshotBuilder` as used by `UnpackDelta` (plain internal types only).
#[derive(Default)]
struct Builder {
    items: Vec<(i32, Vec<i32>)>,
    data_size: usize,
}

impl Builder {
    /// C++: `CSnapshotBuilder::NewItemRaw` capacity checks (snapshot.cpp:880). Returns the
    /// new item's index, or `None` when the snapshot would be full. C++ asserts (crashes)
    /// on sizes above the snapshot size; here those are just full, see DECISIONS.md.
    fn new_item(&mut self, key: i32, size: usize) -> Option<usize> {
        if self.items.len() >= MAX_ITEMS {
            return None;
        }
        let item_size = 4 + size;
        if 8 + (self.items.len() + 1) * 4 + self.data_size + item_size > MAX_SIZE {
            return None;
        }
        self.data_size += item_size;
        self.items.push((key, vec![0; size / 4]));
        Some(self.items.len() - 1)
    }

    /// C++: `CSnapshotBuilder::Finish`.
    fn finish(self) -> Snapshot {
        let mut buf = vec![self.data_size as i32, self.items.len() as i32];
        let mut offset = 0;
        for (_, data) in &self.items {
            buf.push(offset);
            offset += 4 + 4 * data.len() as i32;
        }
        for (key, data) in self.items {
            buf.push(key);
            buf.extend(data);
        }
        Snapshot { buf }
    }
}

/// Applies a decompressed delta to `from`. C++: `CSnapshotDelta::UnpackDelta`
/// (snapshot.cpp:513); errors are the C++ codes. `static_sizes[type]` is the item size in
/// bytes for types sent without size (C++ `SetStaticsize`), 0 if none.
pub fn unpack_delta(
    from: &Snapshot,
    delta: &[i32],
    static_sizes: &[usize; MAX_NETOBJSIZES],
) -> Result<Snapshot, i32> {
    // header: num deleted, num updated, num temp (unused)
    let num_deleted = *delta.first().ok_or(-101)?;
    if num_deleted < 0 {
        return Err(-201);
    }
    let rest = delta.get(3..).ok_or(-101)?;
    let deleted = rest.get(..num_deleted as usize).ok_or(-101)?;
    let num_updates = delta[1];
    let mut data = &rest[deleted.len()..];

    let mut builder = Builder::default();
    for i in 0..from.num_items() {
        if !deleted.contains(&from.key(i)) {
            let index = builder
                .new_item(from.key(i), from.data(i).len() * 4)
                .ok_or(-301)?;
            builder.items[index].1.copy_from_slice(from.data(i));
        }
    }

    for _ in 0..num_updates {
        let [ty, id, ..] = *data else {
            return Err(-102);
        };
        data = &data[2..];
        if !(0..=MAX_TYPE).contains(&ty) {
            return Err(-202);
        }
        if !(0..=MAX_ID).contains(&id) {
            return Err(-203);
        }
        let size = match static_sizes.get(ty as usize) {
            Some(&size) if size != 0 => size,
            _ => {
                let &n = data.first().ok_or(-103)?;
                if n < 0 || n as usize > i32::MAX as usize / 4 {
                    return Err(-204);
                }
                data = &data[1..];
                n as usize * 4
            }
        };
        if data.len() * 4 < size {
            return Err(-205);
        }
        let (diff, tail) = data.split_at(size / 4);
        data = tail;

        let key = (ty << 16) | id;
        let index = match builder.items.iter().position(|(k, _)| *k == key) {
            Some(index) if builder.items[index].1.len() * 4 != size => return Err(-206),
            Some(index) => index,
            None => builder.new_item(key, size).ok_or(-302)?,
        };
        let out = &mut builder.items[index].1;
        match from.index_of(key) {
            Some(from_index) => {
                let past = from.data(from_index);
                if past.len() * 4 != size {
                    return Err(-207);
                }
                // C++ `UndiffItem`: addition with wrapping
                for ((o, p), d) in out.iter_mut().zip(past).zip(diff) {
                    *o = p.wrapping_add(*d);
                }
            }
            None => out.copy_from_slice(diff),
        }
    }
    Ok(builder.finish())
}

/// A validated snapshot item with its external type. C++: an item of the "alt"
/// snapshot built by `CClient::UnpackAndValidateSnapshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub type_id: i32,
    pub id: i32,
    pub data: Vec<i32>,
}

/// C++: `CClient::UnpackAndValidateSnapshot` (client.cpp:2432): drops type-0 (UUID
/// description) and unknown items, runs `SecureUnpackObj` on the rest. C++ rebuilds a
/// `CSnapshot` from the result; a plain list is equivalent, see DECISIONS.md.
pub fn unpack_and_validate(snap: &Snapshot) -> Vec<Item> {
    (0..snap.num_items())
        .filter_map(|i| {
            let type_id = snap.item_type(i).filter(|&t| t > 0)?;
            let spec = OBJ_SPECS.iter().find(|s| s.type_id == type_id)?;
            Some(Item {
                type_id,
                id: snap.key(i) & 0xffff,
                data: secure_unpack_obj(spec, snap.data(i))?,
            })
        })
        .collect()
}
