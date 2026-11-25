//! The line parsing of the C++ console (engine/shared/console.cpp): splitting a line
//! into commands, `ParseArgs` and the number parsers it uses. Used for settings files
//! (client crate) and map settings (`predict.rs`).

/// C++: `CONSOLE_MAX_STR_LENGTH`: a command is cut to this many bytes.
const MAX_STR_LENGTH: usize = 8192;

/// C++: `str_isspace`.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\r' | b'\t')
}

fn skip_whitespaces(s: &str) -> &str {
    s.trim_start_matches(|c: char| c.is_ascii() && is_space(c as u8))
}

/// C++: `str_skip_to_whitespace`, as the index of the first whitespace.
fn to_whitespace(s: &str) -> usize {
    s.bytes().position(is_space).unwrap_or(s.len())
}

/// C++: the command loop of `CConsole::ExecuteLineStroked`: splits a line at `;`
/// outside quotes (a `#` outside quotes ends it) and does `ParseStart` on each part.
/// Returns (command, arguments) pairs; parts without a command are skipped.
pub fn split_line(line: &str) -> Vec<(String, String)> {
    // "mc;" only forces semicolon interpretation, which is already on (C++ default)
    let mut s = line.strip_prefix("mc;").unwrap_or(line).as_bytes();
    let mut out = Vec::new();
    while !s.is_empty() {
        let (mut end, mut next) = (0, None);
        let (mut in_string, mut escaping) = (false, false);
        while end < s.len() {
            let c = s[end];
            if escaping {
                escaping = false;
            } else if c == b'"' {
                in_string = !in_string;
            } else if in_string && c == b'\\' {
                escaping = true;
            }
            if !in_string {
                if c == b';' {
                    next = Some(end + 1);
                    break;
                } else if c == b'#' {
                    break;
                }
            }
            end += 1;
        }
        // C++ ParseStart: str_copy into CONSOLE_MAX_STR_LENGTH + 1 bytes
        let mut part = s[..end.min(MAX_STR_LENGTH)].to_vec();
        crate::protocol::packer::utf8_fix_truncation(&mut part);
        let part = String::from_utf8_lossy(&part);
        let part = skip_whitespaces(&part);
        let cmd_end = to_whitespace(part);
        let command = &part[..cmd_end];
        let args = part.get(cmd_end + 1..).unwrap_or("");
        if !command.is_empty() {
            out.push((command.to_string(), args.to_string()));
        }
        match next {
            Some(n) => s = &s[n..],
            None => break,
        }
    }
    out
}

/// C++: `CConsole::NextParam`.
fn next_param(format: &[u8], pos: &mut usize) -> u8 {
    let at = |p: usize| format.get(p).copied().unwrap_or(0);
    if at(*pos) != 0 {
        *pos += 1;
        if at(*pos) == b'[' {
            while at(*pos) != b']' {
                if at(*pos) == 0 {
                    return 0;
                }
                *pos += 1;
            }
            *pos += 1;
            if at(*pos) == b' ' {
                *pos += 1;
            }
        }
    }
    at(*pos)
}

/// C `isspace` (what `strtol`/`strtod` skip).
fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | 0x09..=0x0d)
}

/// C++ `strtol(s, &end, 10)` with `long` = `i32` (Windows): the value (clamped) and how
/// many bytes it consumed (0 if there are no digits).
pub fn strtol(s: &str) -> (i32, usize) {
    let b = s.as_bytes();
    let mut i = b.iter().take_while(|&&c| is_c_space(c)).count();
    let neg = b.get(i) == Some(&b'-');
    if matches!(b.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let digits = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return (0, 0);
    }
    let v = b[i..i + digits]
        .iter()
        .fold(0i64, |v, c| (v * 10 + (c - b'0') as i64).min(1 << 40));
    let v = if neg { -v } else { v };
    (v.clamp(i32::MIN as i64, i32::MAX as i64) as i32, i + digits)
}

/// C++: `str_toint(str, &out)`: `strtol` that must consume the whole string.
pub fn str_toint(s: &str) -> Option<i32> {
    let (v, end) = strtol(s);
    (end == s.len()).then_some(v)
}

/// C++: `str_tofloat`: `strtod` (to double, then float) that must consume the whole
/// string. An empty string is 0 (strtod converts nothing and stops at the end).
pub fn str_tofloat(s: &str) -> Option<f32> {
    if s.is_empty() {
        return Some(0.0);
    }
    let t = s.trim_start_matches(|c: char| c.is_ascii() && is_c_space(c as u8));
    let (neg, body) = match t.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let lower = body.to_ascii_lowercase();
    let v = if let Some(hex) = lower.strip_prefix("0x") {
        hex_float(hex)?
    } else if let Some(chars) = lower.strip_prefix("nan(").and_then(|r| r.strip_suffix(')')) {
        // strtod: "nan(n-char-sequence)"
        chars
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
            .then_some(f64::NAN)?
    } else if body.starts_with(['+', '-']) {
        return None;
    } else {
        body.parse::<f64>().ok()?
    };
    Some(if neg { -v } else { v } as f32)
}

/// strtod's hexadecimal form after "0x": hex digits with an optional point, then an
/// optional binary exponent `p[+-]digits`. ponytail: exact up to 15 significant hex
/// digits (60 bits) and exponents within the normal range; longer mantissas are cut,
/// not rounded.
fn hex_float(s: &str) -> Option<f64> {
    let (mantissa, exp) = match s.split_once('p') {
        Some((m, e)) => (m, Some(e)),
        None => (s, None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    let mut m: u64 = 0;
    let mut shift: i32 = 0;
    let mut used = 0;
    for (i, c) in int.chars().chain(frac.chars()).enumerate() {
        let d = c.to_digit(16)? as u64;
        let in_frac = i >= int.len();
        if m == 0 && d == 0 {
            shift -= 4 * in_frac as i32;
            continue;
        }
        if used < 15 {
            m = m << 4 | d;
            used += 1;
            shift -= 4 * in_frac as i32;
        } else if !in_frac {
            shift += 4;
        }
    }
    let e: i32 = match exp {
        Some(e) => {
            let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
            if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            e.parse::<i64>()
                .unwrap_or(if e.starts_with('-') {
                    -100_000
                } else {
                    100_000
                })
                .clamp(-100_000, 100_000) as i32
        }
        None => 0,
    };
    Some(m as f64 * 2f64.powi(shift + e))
}

/// C++: the packed-number branch of `CConsole::ColorParse` (`str_isallnum`, optionally
/// signed, then `strtoul`), the only form the C++ client saves. Hex (`$RRGGBB`) and
/// colour names are not supported. ponytail: add them if a config ever uses them.
pub fn color_parse(s: &str) -> Option<u32> {
    let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // strtoul: negation wraps, overflow gives ULONG_MAX, which ColorParse rejects
    let parsed = digits.bytes().try_fold(0u64, |v, b| {
        Some(v * 10 + (b - b'0') as u64).filter(|&v| v <= u32::MAX as u64)
    });
    let v = match parsed {
        None => u32::MAX,
        Some(v) if s.starts_with('-') => (v as u32).wrapping_neg(),
        Some(v) => v as u32,
    };
    (v != u32::MAX).then_some(v)
}

/// C++: `CConsole::ParseArgs` for the parameter kinds used here (`s`, `r`, `i`, `c`, `f`;
/// others are strings). `args` may already hold the stroke argument.
pub fn parse_args(mut s: &str, format: &str, args: &mut Vec<String>) -> bool {
    let format = format.as_bytes();
    let mut pos = 0;
    let mut optional = false;
    let mut command = format.first().copied().unwrap_or(0);
    while command != 0 {
        if command == b'?' {
            optional = true;
            command = next_param(format, &mut pos);
            continue;
        }
        s = skip_whitespaces(s);
        if s.is_empty() {
            return optional;
        }
        if let Some(rest) = s.strip_prefix('"') {
            let (mut arg, mut chars) = (String::new(), rest.char_indices());
            loop {
                match chars.next() {
                    None => return false,
                    Some((i, '"')) => {
                        s = &rest[i + 1..];
                        break;
                    }
                    Some((_, '\\')) if rest[chars.offset()..].starts_with(['\\', '"']) => {
                        arg.push(chars.next().unwrap().1);
                    }
                    Some((_, c)) => arg.push(c),
                }
            }
            args.push(arg);
        } else {
            if command == b'r' {
                args.push(s.to_string());
                return true;
            }
            let end = to_whitespace(s);
            args.push(s[..end].to_string());
            s = s.get(end + 1..).unwrap_or("");
        }
        let last = args.last().unwrap();
        let valid = match command {
            b'i' => str_toint(last).is_some_and(|v| v != i32::MAX && v != i32::MIN),
            b'c' => color_parse(last).is_some(),
            b'f' => str_tofloat(last).is_some_and(|v| v != f32::MAX && v != f32::MIN_POSITIVE),
            _ => true,
        };
        if !valid {
            return false;
        }
        command = next_param(format, &mut pos);
    }
    true
}

/// C++: `CResult::GetFloat` (`str_tofloat` result, 0 when invalid).
pub fn arg_float(args: &[String], i: usize) -> f32 {
    args.get(i).and_then(|a| str_tofloat(a)).unwrap_or(0.0)
}
