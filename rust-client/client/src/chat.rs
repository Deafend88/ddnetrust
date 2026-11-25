//! What the HUD needs from chat lines before the chat itself is ported (PLAN §11): a line
//! that mentions the local player (C++ `CChat::AddLine`, its trim and repeat rules, and
//! `CChat::LineShouldHighlight`).

use ddnet::protocol::generated::MsgSvChat;
use ddnet::protocol::packer::utf8_decode;

use crate::tolower::tolower;

/// C++ `MAX_CHAT_LENGTH`.
const MAX_CHAT_LENGTH: usize = 256;

fn byte_at(s: &[u8], pos: usize) -> u8 {
    s.get(pos).copied().unwrap_or(0)
}

/// C++: `str_utf8_isspace`.
fn isspace(code: i32) -> bool {
    code <= 0x0020
        || code == 0x0085
        || code == 0x00A0
        || code == 0x034F
        || code == 0x115F
        || code == 0x1160
        || code == 0x1680
        || code == 0x180E
        || (0x2000..=0x200F).contains(&code)
        || (0x2028..=0x202F).contains(&code)
        || (0x205F..=0x2064).contains(&code)
        || (0x206A..=0x206F).contains(&code)
        || code == 0x2800
        || code == 0x3000
        || code == 0x3164
        || (0xFE00..=0xFE0F).contains(&code)
        || code == 0xFEFF
        || code == 0xFFA0
        || (0xFFF9..=0xFFFC).contains(&code)
}

/// C++ `CChat::AddLine`: trailing whitespace cut, at most 256 code points.
pub fn trim(line: &[u8]) -> &[u8] {
    let (mut pos, mut end, mut length) = (0, None, 0);
    let mut cut = line.len();
    while byte_at(line, pos) != 0 {
        let old = pos;
        let code = utf8_decode(line, &mut pos);
        if !isspace(code) {
            end = None;
        } else if end.is_none() {
            end = Some(old);
        }
        length += 1;
        if length >= MAX_CHAT_LENGTH {
            cut = pos;
            break;
        }
    }
    let cut = cut.min(line.iter().position(|&b| b == 0).unwrap_or(line.len()));
    &line[..end.map_or(cut, |e| e.min(cut))]
}

/// C++: `str_utf8_find_nocase` from byte `from`: the byte offset of the first match.
pub fn find_nocase(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let mut h = from;
    while byte_at(haystack, h) != 0 {
        let (mut a, mut b) = (h, 0);
        let (mut a_next, mut b_next) = (h, 0);
        while byte_at(haystack, a) != 0
            && byte_at(needle, b) != 0
            && tolower(utf8_decode(haystack, &mut a_next))
                == tolower(utf8_decode(needle, &mut b_next))
        {
            a = a_next;
            b = b_next;
        }
        if byte_at(needle, b) == 0 {
            return Some(h);
        }
        utf8_decode(haystack, &mut h);
    }
    None
}

/// C++: `CChat::LineShouldHighlight`: `name` as a word of its own (a space before it, the
/// end, a space or `.!,?:` after it), any case.
pub fn should_highlight(line: &[u8], name: &[u8]) -> bool {
    let length = name.len();
    let mut hit = find_nocase(line, 0, name);
    while let Some(p) = hit {
        let after = byte_at(line, p + length);
        if length > 0
            && (p == 0 || line[p - 1] == b' ')
            && matches!(after, 0 | b' ' | b'.' | b'!' | b',' | b'?' | b':')
        {
            return true;
        }
        hit = find_nocase(line, p + 1, name);
    }
    false
}

/// The last added line (C++ `m_aLines[m_CurrentLine]`) for the repeat rule.
#[derive(Debug, Default)]
pub struct Mentions {
    previous: Option<(i32, i32, Vec<u8>)>,
}

impl Mentions {
    /// C++ `CChat::AddLine` up to its highlight: the line if it mentions `local_name`.
    /// `author` is the sender's name (empty: unknown client, the line is dropped).
    /// Not ported: friends/foes/ignore filters (no friends list yet), system lines
    /// (never highlighted), the dummy's name.
    pub fn on_chat(
        &mut self,
        msg: &MsgSvChat,
        author: &str,
        local: Option<i32>,
        local_name: &str,
    ) -> Option<String> {
        let line = msg.message.as_bytes();
        if line.is_empty() || (msg.client_id >= 0 && author.is_empty()) {
            return None;
        }
        let line = trim(line);
        if line.is_empty() {
            return None;
        }
        let key = (msg.team, msg.client_id, line.to_vec());
        if self.previous.as_ref() == Some(&key) {
            return None; // C++ counts a repeat on the previous line
        }
        self.previous = Some(key);
        let mention = msg.client_id >= 0
            && Some(msg.client_id) != local
            && local.is_some()
            && should_highlight(line, local_name.as_bytes());
        mention.then(|| String::from_utf8_lossy(line).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cases worked out from `LineShouldHighlight` and `AddLine`.
    #[test]
    fn highlight_follows_cchat() {
        let h = |l: &str, n: &str| should_highlight(l.as_bytes(), n.as_bytes());
        assert!(h("hi tee", "tee"));
        assert!(h("TEE: hi", "tee"));
        assert!(h("ok Tee!", "tee"));
        assert!(h("a teeth tee?", "tee"));
        assert!(!h("teeth", "tee"));
        assert!(!h("a-tee", "tee"));
        assert!(!h("tee;", "tee"));
        assert!(!h("anything", ""));
        // Unicode case folding from the C++ table
        assert!(h("привет ТИ", "ти"));
        assert!(h("ÄÖ", "äö"));
        // malformed UTF-8 does not panic
        assert!(!h("\u{0}", "a"));
        should_highlight(&[0xE0, 0x80, b'a', 0xFF], b"a");
        assert!(should_highlight(&[0xFF, b' ', b'a'], b"a"));

        assert_eq!(trim(b"hi  \t "), b"hi");
        assert_eq!(trim(b"   "), b"");
        let long = "x".repeat(300);
        assert_eq!(trim(long.as_bytes()).len(), 256);

        let mut m = Mentions::default();
        let msg = |id, text: &str| MsgSvChat {
            team: 0,
            client_id: id,
            message: text.into(),
        };
        assert_eq!(
            m.on_chat(&msg(3, "hey tee  "), "a", Some(0), "tee")
                .as_deref(),
            Some("hey tee")
        );
        assert_eq!(m.on_chat(&msg(3, "hey tee"), "a", Some(0), "tee"), None); // repeat
        assert_eq!(m.on_chat(&msg(0, "me, tee"), "tee", Some(0), "tee"), None); // own
        assert_eq!(m.on_chat(&msg(-1, "tee joined"), "", Some(0), "tee"), None); // server
        assert_eq!(m.on_chat(&msg(4, "tee"), "", Some(0), "tee"), None); // unknown
    }
}
