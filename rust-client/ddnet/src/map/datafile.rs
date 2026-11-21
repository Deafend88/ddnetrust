//! Reader for the datafile container that maps are stored in.
//! C++: `CDataFileReader` (`src/engine/shared/datafile.cpp`). Little-endian only, like
//! every supported target, so the C++ swap paths are no-ops here.

use std::collections::HashSet;

use flate2::{Decompress, FlushDecompress, Status};

pub const ITEMTYPE_EX: i32 = 0xFFFF;
const MAX_ITEM_TYPE: i32 = 0xFFFF;
const OFFSET_UUID_TYPE: i32 = 0x8000;
const HEADER_SIZE: usize = 36; // C++: sizeof(CDatafileHeader)
const HEADER_SIZE_OFFSET: i64 = 16; // C++: CDatafileHeader::SizeOffset()

/// An item: `type << 16 | id` and its data as ints. C++: `CDatafileItem`.
#[derive(Clone, Debug)]
pub struct Item {
    pub type_and_id: u32,
    pub data: Vec<i32>,
}

impl Item {
    pub fn type_(&self) -> i32 {
        ((self.type_and_id >> 16) & MAX_ITEM_TYPE as u32) as i32
    }

    pub fn id(&self) -> i32 {
        (self.type_and_id & 0xFFFF) as i32
    }
}

pub struct Datafile {
    file: Vec<u8>,
    item_types: Vec<[i32; 3]>, // type, start, num
    /// Items after any changes made while loading (C++ `OverrideItemData` and the
    /// in-place edits of `CMap::Load`).
    pub items: Vec<Item>,
    data_offsets: Vec<i32>,
    data_sizes: Option<Vec<i32>>, // v4: uncompressed sizes
    data_start: usize,
    data_size: i32,
}

fn int_at(b: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(b[offset..offset + 4].try_into().unwrap())
}

impl Datafile {
    /// C++: `CDataFileReader::Open` (datafile.cpp:508) + `CDatafile::Validate` (:380).
    pub fn parse(file: Vec<u8>) -> Result<Self, String> {
        if file.len() < HEADER_SIZE {
            return Err("could not read file header. file truncated or not a datafile.".into());
        }
        if &file[..4] != b"ATAD" && &file[..4] != b"DATA" {
            return Err("wrong header magic".into());
        }
        let [
            version,
            size,
            swaplen,
            num_item_types,
            num_items,
            num_raw_data,
            item_size,
            data_size,
        ] = std::array::from_fn(|i| int_at(&file, 4 + 4 * i));
        if version != 3 && version != 4 {
            return Err(format!("unsupported header version. version={version}"));
        }
        if !(0..=MAX_ITEM_TYPE + 1).contains(&num_item_types)
            || num_items < 0
            || num_raw_data < 0
            || item_size < 0
            || item_size % 4 != 0
            || data_size < 0
        {
            return Err("invalid header information".into());
        }

        // calculate and validate sizes
        let size_fix = if version == 4 {
            num_raw_data as i64 * 4
        } else {
            0
        };
        let info_size = num_item_types as i64 * 12
            + num_items as i64 * 4
            + num_raw_data as i64 * 4
            + size_fix
            + item_size as i64;
        let file_size = file.len() as i64;
        if HEADER_SIZE as i64 + info_size + data_size as i64 != file_size {
            return Err("invalid header data size or truncated file".into());
        }
        // old v4 maps did not count the data sizes in `size`/`swaplen`; C++ fixes that up
        let header_file_size = size as i64 + HEADER_SIZE_OFFSET;
        if header_file_size != file_size
            && !(size_fix != 0 && header_file_size + size_fix == file_size)
        {
            return Err("invalid header size or truncated file".into());
        }
        let header_swaplen = swaplen as i64 + HEADER_SIZE_OFFSET;
        let file_size_swaplen = file_size - data_size as i64;
        if header_swaplen != file_size_swaplen
            && !(swaplen % 4 == 0
                && size_fix != 0
                && header_swaplen + size_fix == file_size_swaplen)
        {
            return Err("invalid header swaplen or truncated file".into());
        }
        // ponytail: C++ also rejects files whose bookkeeping would need over 2 GiB of RAM;
        // such a file is far beyond any map, so that check is not reproduced.

        let ints = |start: usize, n: i32| -> Vec<i32> {
            (0..n as usize)
                .map(|i| int_at(&file, start + 4 * i))
                .collect()
        };
        let mut pos = HEADER_SIZE;
        let item_types: Vec<[i32; 3]> = ints(pos, num_item_types * 3)
            .chunks(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        pos += num_item_types as usize * 12;
        let item_offsets = ints(pos, num_items);
        pos += num_items as usize * 4;
        let data_offsets = ints(pos, num_raw_data);
        pos += num_raw_data as usize * 4;
        let data_sizes = (version == 4).then(|| ints(pos, num_raw_data));
        pos += size_fix as usize;
        let items_start = pos;
        let data_start = items_start + item_size as usize;

        // validate item types
        let mut counted = 0i64;
        let mut used_types = HashSet::new();
        for (index, &[type_, start, num]) in item_types.iter().enumerate() {
            if !(0..=MAX_ITEM_TYPE).contains(&type_)
                || !used_types.insert(type_)
                || num <= 0
                || start as i64 != counted
            {
                return Err(format!("invalid item type. index={index}"));
            }
            counted += num as i64;
            if counted > num_items as i64 {
                break;
            }
        }
        if counted != num_items as i64 {
            return Err("mismatched number of items in item types".into());
        }

        // validate item offsets
        for (index, &offset) in item_offsets.iter().enumerate() {
            let ordered = if index == 0 {
                offset == 0
            } else {
                offset > item_offsets[index - 1]
            };
            if !ordered || offset >= item_size {
                return Err(format!("invalid item offset. index={index}"));
            }
        }

        // validate item sizes, types and ids; the item types cover all items in order
        let mut items = Vec::with_capacity(num_items as usize);
        for &[type_, start, num] in &item_types {
            let mut used_ids = HashSet::new();
            for index in start as usize..(start + num) as usize {
                let end = item_offsets.get(index + 1).copied().unwrap_or(item_size);
                let file_item_size = end - item_offsets[index];
                if file_item_size < 8 {
                    return Err(format!("map item too small for header. item_index={index}"));
                }
                let at = items_start + item_offsets[index] as usize;
                let item = Item {
                    type_and_id: int_at(&file, at) as u32,
                    data: Vec::new(),
                };
                let size = int_at(&file, at + 4);
                if item.type_() != type_ {
                    return Err(format!("mismatched item type. item_index={index}"));
                }
                // many old maps contain duplicate ITEMTYPE_EX items due to a bug in DDNet tools
                if item.type_() != ITEMTYPE_EX && !used_ids.insert(item.id()) {
                    return Err(format!("map item has duplicate ID. item_index={index}"));
                }
                if size < 0 || size % 4 != 0 || size != file_item_size - 8 {
                    return Err(format!("map item size invalid. item_index={index}"));
                }
                items.push(Item {
                    data: ints(at + 8, size / 4),
                    ..item
                });
            }
        }

        // validate data offsets and sizes
        for (index, &offset) in data_offsets.iter().enumerate() {
            let ordered = if index == 0 {
                offset == 0
            } else {
                offset > data_offsets[index - 1]
            };
            if !ordered || offset >= data_size {
                return Err(format!("invalid data offset. index={index}"));
            }
        }
        // size 0 is accepted here (old maps have it) and fails when the data is loaded
        if data_sizes
            .as_ref()
            .is_some_and(|sizes| sizes.iter().any(|&s| s < 0))
        {
            return Err("data size invalid".into());
        }

        Ok(Self {
            file,
            item_types,
            items,
            data_offsets,
            data_sizes,
            data_start,
            data_size,
        })
    }

    /// zlib CRC-32 of the whole file. C++: `CDataFileReader::Crc`.
    pub fn crc(&self) -> u32 {
        let mut crc = flate2::Crc::new();
        crc.update(&self.file);
        crc.sum()
    }

    pub fn size(&self) -> usize {
        self.file.len()
    }

    pub fn num_data(&self) -> usize {
        self.data_offsets.len()
    }

    /// Index range of the items of `type_`. C++: `CDataFileReader::GetType` for the
    /// plain (non-UUID) types that maps use.
    pub fn get_type(&self, type_: i32) -> std::ops::Range<usize> {
        self.item_types
            .iter()
            .find(|t| t[0] == type_)
            .map_or(0..0, |t| t[1] as usize..(t[1] + t[2]) as usize)
    }

    /// C++: `CDataFileReader::FindItemIndex`.
    pub fn find_item_index(&self, type_: i32, id: i32) -> Option<usize> {
        self.get_type(type_).find(|&i| self.items[i].id() == id)
    }

    /// The UUID behind an extended item type, if any. C++: `GetExternalItemType`.
    pub fn item_uuid(&self, index: usize) -> Option<[u8; 16]> {
        let internal = self.items[index].type_();
        if internal <= OFFSET_UUID_TYPE || internal == ITEMTYPE_EX {
            return None;
        }
        let ex = &self.items[self.find_item_index(ITEMTYPE_EX, internal)?];
        let uuid: Vec<u8> = ex
            .data
            .get(..4)?
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        uuid.try_into().ok()
    }

    /// Bytes of data blob `index` as stored in the file. C++: `GetFileDataSize`.
    fn file_data(&self, index: usize) -> &[u8] {
        let end = self
            .data_offsets
            .get(index + 1)
            .copied()
            .unwrap_or(self.data_size);
        let start = self.data_start + self.data_offsets[index] as usize;
        &self.file[start..self.data_start + end as usize]
    }

    /// Decompressed data blob, `None` if it is invalid. C++: `CDatafile::GetData`
    /// (datafile.cpp:190) without data processors (those live in `Map`).
    pub fn data(&self, index: i32) -> Option<Vec<u8>> {
        let index = usize::try_from(index)
            .ok()
            .filter(|&i| i < self.num_data())?;
        let stored = self.file_data(index);
        let Some(sizes) = &self.data_sizes else {
            return Some(stored.to_vec()); // v3: uncompressed
        };
        let wanted = sizes[index] as usize;
        if wanted == 0 {
            return None; // "data size invalid. data will be ignored."
        }
        // C++ `malloc` fails on absurd sizes; avoid aborting on allocation failure
        let mut out = Vec::new();
        out.try_reserve_exact(wanted).ok()?;
        out.resize(wanted, 0);
        // C++ zlib `uncompress`: the stream must end within `wanted` bytes and produce
        // exactly `wanted` bytes; trailing input after the stream end is ignored
        let mut z = Decompress::new(true);
        let status = z
            .decompress(stored, &mut out, FlushDecompress::Finish)
            .ok()?;
        (status == Status::StreamEnd && z.total_out() as usize == wanted).then_some(out)
    }
}
