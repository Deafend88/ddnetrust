//! Byte-exact checks against vectors written by the C++ tool `src/tools/rs_golden.cpp`.
//!
//! Regenerate (from the repo root, after building the `rs_golden` CMake target):
//!     build-rs/rs_golden.exe rust-client/tests/golden

use ddnet::protocol::{huffman, packer, packet, varint};

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
