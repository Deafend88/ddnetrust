//! M7.5 HUD on/off input check: prints the inputs a DDNet server received, from its
//! teehistorian file (`sv_tee_historian 1`), one line per change: client id and the 10
//! `CNetObj_PlayerInput` ints. Format: C++ `CTeeHistorian` (src/game/server/teehistorian.cpp).
//!
//! Usage: cargo run -p ddnet-rs --example teehistorian_inputs -- <file.teehistorian>

use ddnet::protocol::varint;

/// C++ `TEEHISTORIAN_*`, written negated.
const FINISH: i32 = 1;
const TICK_SKIP: i32 = 2;
const PLAYER_NEW: i32 = 3;
const PLAYER_OLD: i32 = 4;
const INPUT_DIFF: i32 = 5;
const INPUT_NEW: i32 = 6;
const MESSAGE: i32 = 7;
const JOIN: i32 = 8;
const DROP: i32 = 9;
const CONSOLE_COMMAND: i32 = 10;
const EX: i32 = 11;

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn int(&mut self) -> i32 {
        let (v, n) = varint::unpack(self.0).expect("truncated int");
        self.0 = &self.0[n..];
        v
    }
    fn raw(&mut self, n: usize) {
        self.0 = &self.0[n..];
    }
    fn string(&mut self) {
        let end = self
            .0
            .iter()
            .position(|&b| b == 0)
            .expect("unterminated string");
        self.raw(end + 1);
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: teehistorian_inputs <file>");
    let data = std::fs::read(path).unwrap();
    // header: the teehistorian UUID, then JSON up to a 0 byte
    let json_end = 16 + data[16..].iter().position(|&b| b == 0).unwrap();
    let mut r = Reader(&data[json_end + 1..]);
    let mut inputs: Vec<[i32; 10]> = vec![[0; 10]; 64];
    while !r.0.is_empty() {
        let kind = r.int();
        if kind >= 0 {
            // player position diff: dx, dy
            r.int();
            r.int();
            continue;
        }
        match -kind {
            FINISH => break,
            TICK_SKIP | PLAYER_OLD | JOIN => {
                r.int();
            }
            PLAYER_NEW => {
                r.int();
                r.int();
                r.int();
            }
            kind @ (INPUT_DIFF | INPUT_NEW) => {
                let cid = r.int() as usize;
                for v in &mut inputs[cid] {
                    let d = r.int();
                    // C++ CSnapshotDelta::DiffItem: the new value minus the old one
                    *v = if kind == INPUT_DIFF {
                        v.wrapping_add(d)
                    } else {
                        d
                    };
                }
                let line: Vec<String> = inputs[cid].iter().map(i32::to_string).collect();
                println!("{cid} {}", line.join(" "));
            }
            MESSAGE => {
                r.int();
                let n = r.int() as usize;
                r.raw(n);
            }
            DROP => {
                r.int();
                r.string();
            }
            CONSOLE_COMMAND => {
                r.int();
                r.int();
                r.string();
                for _ in 0..r.int() {
                    r.string();
                }
            }
            EX => {
                r.raw(16);
                let n = r.int() as usize;
                r.raw(n);
            }
            k => panic!("unknown teehistorian record {k}"),
        }
    }
}
