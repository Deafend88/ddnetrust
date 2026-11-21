//! Byte-exact checks against vectors written by the C++ tool `src/tools/rs_golden.cpp`.
//!
//! Regenerate (from the repo root, after building the `rs_golden` CMake target):
//!     build-rs/rs_golden.exe rust-client/tests/golden

use ddnet::protocol::{generated as g, huffman, netobj, packer, packet, snapshot, varint};
use std::collections::HashMap;

fn golden(name: &str) -> String {
    let path = format!("{}/../tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(b: &[u8]) -> String {
    if b.is_empty() {
        return "-".into();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn varint() {
    let text = golden("varint.txt");
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "pack" => {
                let mut buf = [0u8; varint::MAX_BYTES_PACKED];
                let n = varint::pack(&mut buf, f[1].parse().unwrap()).unwrap();
                assert_eq!(hex(&buf[..n]), f[2], "{line}");
            }
            "unpack" => {
                let got = varint::unpack(&unhex(f[1]));
                let want = match f[2] {
                    "ERR" => None,
                    v => Some((v.parse().unwrap(), f[3].parse().unwrap())),
                };
                assert_eq!(got, want, "{line}");
            }
            _ => panic!("bad line {line}"),
        }
    }
}

#[test]
fn varint_pack_needs_room() {
    let mut buf = [0u8; 2];
    assert_eq!(varint::pack(&mut buf, 8192), None); // needs 3 bytes
    assert_eq!(varint::pack(&mut [], 0), None);
}

#[test]
fn packer() {
    let text = golden("packer.txt");
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "pack" => {
                let mut p = packer::Packer::default();
                for op in f[1].split(',') {
                    let a: Vec<&str> = op.split(':').collect();
                    match a[0] {
                        "i" => p.add_int(a[1].parse().unwrap()),
                        "s" => p.add_string(&unhex(a[3]), a[1].parse().unwrap(), a[2] == "1"),
                        "r" => p.add_raw(&unhex(a[1])),
                        _ => panic!("bad op {op}"),
                    }
                }
                assert_eq!(hex(p.data()), f[2], "{line}");
                assert_eq!(p.error(), f[3] == "1", "{line}");
            }
            "unpack" => {
                let data = unhex(f[1]);
                let mut u = packer::Unpacker::new(&data);
                let mut got = Vec::new();
                for op in f[2].split(',') {
                    got.push(match op.as_bytes()[0] {
                        b'i' => u.get_int().to_string(),
                        b'd' => u.get_int_or_default(op[1..].parse().unwrap()).to_string(),
                        b'u' if op.len() > 1 => u
                            .get_uncompressed_int_or_default(op[2..].parse().unwrap())
                            .to_string(),
                        b'u' => u.get_uncompressed_int().to_string(),
                        b's' => hex(u.get_string(op[1..].parse().unwrap()).as_bytes()),
                        b'r' => u.get_raw(op[1..].parse().unwrap()).map_or("N".into(), hex),
                        _ => panic!("bad op {op}"),
                    });
                }
                got.push(if u.error() { "1" } else { "0" }.into());
                assert_eq!(got.join(","), f[3], "{line}");
            }
            _ => panic!("bad line {line}"),
        }
    }
}

#[test]
fn huffman() {
    let text = golden("huffman.txt");
    let mut out = vec![0u8; 4096];
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let out = &mut out[..f[1].parse::<usize>().unwrap()];
        let got = match f[0] {
            "c" => huffman::compress(&unhex(f[2]), out),
            "d" => huffman::decompress(&unhex(f[2]), out),
            _ => panic!("bad line {line}"),
        };
        let got = got.map_or("ERR".into(), |n| hex(&out[..n]));
        assert_eq!(got, f[3], "{line}");
    }
}

#[test]
fn chunk_header() {
    let text = golden("chunk_header.txt");
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "chpack" => {
                let h = packet::ChunkHeader {
                    flags: f[1].parse().unwrap(),
                    size: f[2].parse().unwrap(),
                    sequence: f[3].parse().unwrap(),
                };
                let mut out = Vec::new();
                h.pack(&mut out);
                assert_eq!(hex(&out), f[4], "{line}");
            }
            "chunpack" => {
                let (h, n) = packet::ChunkHeader::unpack(&unhex(f[1])).unwrap();
                let got = format!("{} {} {} {n}", h.flags, h.size, h.sequence);
                assert_eq!(got, f[2..].join(" "), "{line}");
            }
            _ => panic!("bad line {line}"),
        }
    }
}

#[test]
fn chunk_header_short_input() {
    assert_eq!(packet::ChunkHeader::unpack(&[0x40]), None);
    assert_eq!(packet::ChunkHeader::unpack(&[0x40, 0x00]), None); // vital needs 3 bytes
    assert!(packet::ChunkHeader::unpack(&[0x00, 0x00]).is_some());
}

#[test]
fn packet() {
    let text = golden("packet.txt");
    let mut sixup_rejected = 0;
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "penc" => {
                let p: Vec<i32> = f[1..5].iter().map(|x| x.parse().unwrap()).collect();
                let out = packet::encode(p[0], p[1], p[2], &unhex(f[5]), p[3]);
                assert_eq!(hex(&out), f[6], "{line}");
            }
            "cenc" => {
                let extra: [u8; 4] = unhex(f[2]).try_into().unwrap();
                let out = packet::encode_connless(&unhex(f[3]), (f[1] == "1").then_some(extra));
                assert_eq!(hex(&out), f[4], "{line}");
            }
            "pdec" => {
                let got = packet::decode(&unhex(f[1]));
                if f[3] == "1" {
                    // 0.7 packet: C++ parses it, the 0.6-only client drops it
                    assert_eq!(got, None, "{line}");
                    sixup_rejected += 1;
                    continue;
                }
                let got = got.map_or("ERR 0".into(), |p| {
                    let extra = if p.flags & packet::PACKETFLAG_EXTENDED != 0 {
                        hex(&p.extra)
                    } else {
                        "-".into()
                    };
                    let data = hex(&p.data);
                    format!("OK 0 {} {} {} {data} {extra}", p.flags, p.ack, p.num_chunks)
                });
                assert_eq!(got, f[2..].join(" "), "{line}");
            }
            "backroom" => {
                let got = packet::is_seq_in_backroom(f[1].parse().unwrap(), f[2].parse().unwrap());
                assert_eq!(got, f[3] == "1", "{line}");
            }
            _ => panic!("bad line {line}"),
        }
    }
    assert!(sixup_rejected > 0);
}

#[test]
fn protocol() {
    let text = golden("protocol.txt");
    let mut cpp_uuids: HashMap<String, String> = HashMap::new();
    let (mut objs, mut msgs) = (0, 0);
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(4, ' ').collect();
        match f[0] {
            "uuid" => {
                cpp_uuids.insert(f[1].into(), f[2].into());
            }
            "objsize" => {
                if f[1] == "EX/UUID" {
                    continue;
                }
                let spec = g::OBJ_SPECS.iter().find(|s| s.name == f[1]).expect(line);
                let size = (spec.fields.len() * 4).to_string();
                assert_eq!((size.as_str(), size.as_str()), (f[2], f[3]), "{line}");
            }
            "obj" => {
                let spec = g::OBJ_SPECS.iter().find(|s| s.name == f[1]).expect(line);
                let ints: Vec<i32> = unhex(f[2])
                    .chunks(4)
                    .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                    .collect();
                let got = netobj::secure_unpack_obj(spec, &ints).map_or("FAIL".into(), |out| {
                    hex(&out
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<u8>>())
                });
                assert_eq!(got, f[3], "{line}");
                objs += 1;
            }
            "msg" => {
                let &(id, _, _) = g::MSG_TYPES.iter().find(|m| m.1 == f[1]).expect(line);
                let data = unhex(f[2]);
                let got = g::unpack_game_msg(id, &mut packer::Unpacker::new(&data));
                assert_eq!(got.err().unwrap_or("OK"), f[3], "{line}");
                msgs += 1;
            }
            _ => panic!("bad line {line}"),
        }
    }
    assert_eq!((objs, msgs), (1520, 2240));

    // every UUID we generate exists in C++ (keyed by hex, since our tables hold the
    // object/message names, not the UUID name strings)
    let cpp: std::collections::HashSet<&String> = cpp_uuids.values().collect();
    let ours = g::OBJ_SPECS.iter().filter_map(|s| s.uuid);
    let ours = ours.chain(g::MSG_TYPES.iter().filter_map(|m| m.2));
    let mut n = 0;
    for uuid in ours {
        assert!(cpp.contains(&hex(&uuid)), "unknown uuid {}", hex(&uuid));
        n += 1;
    }
    for (_, name, uuid) in g::SYS_EX_MSGS {
        assert_eq!(cpp_uuids.get(*name), Some(&hex(uuid)), "{name}");
        n += 1;
    }
    assert!(n > 40, "only {n} uuids checked");
}

#[test]
fn snapshot() {
    let mut static_sizes = [0usize; snapshot::MAX_NETOBJSIZES];
    for spec in g::OBJ_SPECS
        .iter()
        .filter(|s| s.type_id < g::NUM_NETOBJTYPES)
    {
        static_sizes[spec.type_id as usize] = spec.fields.len() * 4;
    }
    let ints_hex = |s: &snapshot::Snapshot| {
        hex(&s
            .as_ints()
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>())
    };

    let text = golden("snapshot.txt");
    let mut snap_hex: HashMap<String, usize> = HashMap::new(); // C++ bytes -> n
    let mut snaps: HashMap<usize, snapshot::Snapshot> = HashMap::new();
    let (mut decoded, mut alts) = (0, 0);
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "snap" => {
                snap_hex.insert(f[2].into(), f[1].parse().unwrap());
            }
            "delta" => {
                let from = match f[1] {
                    "-1" => snapshot::Snapshot::default(),
                    n => snaps[&n.parse::<usize>().unwrap()].clone(),
                };
                let delta = if f[2] == "-" {
                    Some(vec![0, 0, 0]) // C++ `EmptyDelta()`
                } else {
                    varint::decompress(&unhex(f[2]), snapshot::MAX_DELTA_INTS)
                };
                let got = match delta {
                    None => "ERR decompress".to_string(),
                    Some(delta) => match snapshot::unpack_delta(&from, &delta, &static_sizes) {
                        Err(code) => format!("ERR {code}"),
                        Ok(s) => {
                            let h = ints_hex(&s);
                            if let Some(&n) = snap_hex.get(&h) {
                                snaps.insert(n, s.clone());
                            }
                            format!("{h} {} 1", s.crc())
                        }
                    },
                };
                assert_eq!(got, f[3..].join(" "), "{line}");
                decoded += 1;
            }
            "alt" => {
                let n: usize = f[1].parse().unwrap();
                let items: Vec<String> = snapshot::unpack_and_validate(&snaps[&n])
                    .iter()
                    .map(|it| {
                        let spec = g::OBJ_SPECS
                            .iter()
                            .find(|s| s.type_id == it.type_id)
                            .unwrap();
                        let data: Vec<u8> = it.data.iter().flat_map(|v| v.to_le_bytes()).collect();
                        format!("{}:{}:{}", spec.name, it.id, hex(&data))
                    })
                    .collect();
                let got = if items.is_empty() {
                    "-".into()
                } else {
                    items.join(";")
                };
                assert_eq!(got, f[2], "{line}");
                alts += 1;
            }
            _ => panic!("bad line {line}"),
        }
    }
    assert_eq!((snaps.len(), decoded, alts), (60, 978, 60));
}

#[test]
fn snapshot_oversized_item_is_an_error() {
    // one update: type 100 (no static size), id 0, 20,000 ints (80 KB > 64 KiB snapshot);
    // the C++ client hits a dbg_assert here (DECISIONS.md)
    let n = 20_000;
    let mut delta = vec![0, 1, 0, 100, 0, n];
    delta.resize(6 + n as usize, 7);
    let got = snapshot::unpack_delta(
        &snapshot::Snapshot::default(),
        &delta,
        &[0; snapshot::MAX_NETOBJSIZES],
    );
    assert_eq!(got, Err(-302));
}
