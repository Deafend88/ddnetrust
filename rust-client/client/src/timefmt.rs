//! Time texts. C++: `str_time` and `str_time_float` (src/base/time.cpp:204, 248).

/// C++ `ETimeFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeFormat {
    Days,
    Hours,
    Mins,
    HoursCentisecs,
    MinsCentisecs,
    SecsCentisecs,
}

impl TimeFormat {
    pub const ALL: [Self; 6] = [
        Self::Days,
        Self::Hours,
        Self::Mins,
        Self::HoursCentisecs,
        Self::MinsCentisecs,
        Self::SecsCentisecs,
    ];
}

/// C++: `str_time`: negative times show as 0; each format falls through to the next
/// shorter one below its unit.
pub fn str_time(centisecs: i64, format: TimeFormat) -> String {
    const SEC: i64 = 100;
    const MIN: i64 = 60 * SEC;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    let c = centisecs.max(0);
    use TimeFormat::*;
    match format {
        Days if c >= DAY => format!(
            "{}d {:02}:{:02}:{:02}",
            c / DAY,
            (c % DAY) / HOUR,
            (c % HOUR) / MIN,
            (c % MIN) / SEC
        ),
        Days | Hours if c >= HOUR => format!(
            "{:02}:{:02}:{:02}",
            c / HOUR,
            (c % HOUR) / MIN,
            (c % MIN) / SEC
        ),
        Days | Hours | Mins => format!("{:02}:{:02}", c / MIN, (c % MIN) / SEC),
        HoursCentisecs if c >= HOUR => format!(
            "{:02}:{:02}:{:02}.{:02}",
            c / HOUR,
            (c % HOUR) / MIN,
            (c % MIN) / SEC,
            c % SEC
        ),
        HoursCentisecs | MinsCentisecs if c >= MIN => {
            format!("{:02}:{:02}.{:02}", c / MIN, (c % MIN) / SEC, c % SEC)
        }
        HoursCentisecs | MinsCentisecs | SecsCentisecs => {
            format!("{:02}.{:02}", (c % MIN) / SEC, c % SEC)
        }
    }
}

/// C++: `time_even_centiseconds_from_seconds`: rounded to an even centisecond.
pub fn even_centiseconds(seconds: f32) -> i64 {
    // ponytail: `as` saturates where C++'s cast of a huge or NaN double is undefined;
    // finite network times (an int / 1000) never get there
    (((seconds as f64) * 50.0).round() as i64).wrapping_mul(2)
}

/// C++: `str_time_float`.
pub fn str_time_float(seconds: f32, format: TimeFormat) -> String {
    str_time(even_centiseconds(seconds), format)
}
