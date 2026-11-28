//! 13.7: the vote state against `votes.txt` from `rs_parity votes` (copies of
//! `CVoting::OnMessage` and its option list code): random vote messages, then the whole
//! state after each.

use ddnet::protocol::generated::{self as g, GameMsg};
use ddnet_rs::vote::Voting;

fn unhex(s: &str) -> String {
    if s == "-" {
        return String::new();
    }
    let b: Vec<u8> = (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    String::from_utf8(b).unwrap()
}

fn hex(s: &str) -> String {
    if s.is_empty() {
        return "-".into();
    }
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn votes_match_cpp() {
    let path = format!("{}/../tests/golden/votes.txt", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(path).unwrap();
    let mut v = Voting::default();
    let mut kinds = std::collections::BTreeSet::new();
    for (n, line) in text.lines().enumerate() {
        let (input, want) = line.split_once(" = ").unwrap();
        let f: Vec<&str> = input.split(' ').collect();
        let now: i64 = f[0].parse().unwrap();
        let int = |i: usize| f[i].parse::<i32>().unwrap();
        kinds.insert(f[1]);
        let msg = match f[1] {
            "set" => Some(GameMsg::SvVoteSet(g::MsgSvVoteSet {
                timeout: int(2),
                description: unhex(f[3]),
                reason: unhex(f[4]),
            })),
            "status" => Some(GameMsg::SvVoteStatus(g::MsgSvVoteStatus {
                yes: int(2),
                no: int(3),
                pass: int(4),
                total: int(5),
            })),
            "clear" => Some(GameMsg::SvVoteClearOptions(g::MsgSvVoteClearOptions {})),
            "listadd" => {
                let d = |i: usize| unhex(f[3 + i]);
                Some(GameMsg::SvVoteOptionListAdd(g::MsgSvVoteOptionListAdd {
                    num_options: int(2),
                    description0: d(0),
                    description1: d(1),
                    description2: d(2),
                    description3: d(3),
                    description4: d(4),
                    description5: d(5),
                    description6: d(6),
                    description7: d(7),
                    description8: d(8),
                    description9: d(9),
                    description10: d(10),
                    description11: d(11),
                    description12: d(12),
                    description13: d(13),
                    description14: d(14),
                }))
            }
            "add" => Some(GameMsg::SvVoteOptionAdd(g::MsgSvVoteOptionAdd {
                description: unhex(f[2]),
            })),
            "remove" => Some(GameMsg::SvVoteOptionRemove(g::MsgSvVoteOptionRemove {
                description: unhex(f[2]),
            })),
            "yourvote" => Some(GameMsg::SvYourVote(g::MsgSvYourVote { voted: int(2) })),
            "groupstart" => Some(GameMsg::SvVoteOptionGroupStart(
                g::MsgSvVoteOptionGroupStart {},
            )),
            "groupend" => Some(GameMsg::SvVoteOptionGroupEnd(g::MsgSvVoteOptionGroupEnd {})),
            _ => None,
        };
        if let Some(msg) = msg {
            v.on_message(&msg, now);
        }
        let mut got = format!(
            "{} {} {} {} {} {} {} {} {} {} {}",
            v.open_time,
            v.close_time,
            v.voted,
            v.yes,
            v.no,
            v.pass,
            v.total,
            v.receiving_options as i32,
            v.options.len(),
            hex(&v.description),
            hex(&v.reason)
        );
        for o in &v.options {
            got += &format!(" {}", hex(o));
        }
        assert_eq!(got, want, "line {}: {input}", n + 1);
    }
    assert_eq!(kinds.len(), 10, "{kinds:?}");
}
