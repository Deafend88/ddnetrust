//! Map envelopes. C++: `CMapBasedEnvelopePointAccess`, `IEnvelopePointAccess::FindPointIndex`,
//! `SolveBezier` and `CRenderMap::RenderEvalEnvelope` (`src/game/map/render_map.cpp`), and
//! `CEnvelopeState::EnvelopeEval` (`src/game/client/components/envelope_state.cpp`).
//! Float and double steps are kept where C++ has them.

use ddnet::map::Map;

const MAPITEMTYPE_ENVELOPE: i32 = 3;
const MAPITEMTYPE_ENVPOINTS: i32 = 6;
/// C++: `CMapItemEnvelope::VERSION_TEEWORLDS_BEZIER`.
const VERSION_TEEWORLDS_BEZIER: i32 = 3;
/// UUID of `mapitemtype-envpoints-bezier@ddnet.tw` (MD5 v3 in the Teeworlds namespace).
const ENVPOINTS_BEZIER_UUID: [u8; 16] = [
    0x3a, 0xb4, 0xf8, 0x4d, 0xd9, 0xcc, 0x3c, 0x78, 0xb5, 0x85, 0x0d, 0x6c, 0xcd, 0xb2, 0xe2, 0x5c,
];
const CURVETYPE_STEP: i32 = 0;
const CURVETYPE_SLOW: i32 = 2;
const CURVETYPE_FAST: i32 = 3;
const CURVETYPE_SMOOTH: i32 = 4;
const CURVETYPE_BEZIER: i32 = 5;

/// C++: `CEnvPoint` (time in ms, values in 22.10 fixed point).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnvPoint {
    pub time: i32,
    pub curve: i32,
    pub values: [i32; 4],
}

/// C++: `CEnvPointBezier`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bezier {
    pub in_dx: [i32; 4],
    pub in_dy: [i32; 4],
    pub out_dx: [i32; 4],
    pub out_dy: [i32; 4],
}

impl EnvPoint {
    fn from_ints(d: &[i32]) -> Self {
        Self {
            time: d[0],
            curve: d[1],
            values: [d[2], d[3], d[4], d[5]],
        }
    }
}

impl Bezier {
    fn from_ints(d: &[i32]) -> Self {
        let four = |o: usize| [d[o], d[o + 1], d[o + 2], d[o + 3]];
        Self {
            in_dx: four(0),
            in_dy: four(4),
            out_dx: four(8),
            out_dy: four(12),
        }
    }
}

/// C++: `fx2f`.
fn fx2f(v: i32) -> f32 {
    v as f32 / 1024.0
}

/// C++ `std::clamp` for floats, which keeps NaN (Rust's `clamp` would panic on bad bounds).
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if hi < v {
        hi
    } else {
        v
    }
}

/// C++: `mix` + `bezier` (math.h): De Casteljau.
fn bezier(p0: f32, p1: f32, p2: f32, p3: f32, a: f32) -> f32 {
    let mix = |x: f32, y: f32| x + (y - x) * a;
    let (c10, c11, c12) = (mix(p0, p1), mix(p1, p2), mix(p2, p3));
    let (c20, c21) = (mix(c10, c11), mix(c11, c12));
    mix(c20, c21)
}

/// C++: `SolveBezier` (render_map.cpp): t with x(t) = `x`, in double precision.
fn solve_bezier(x: f32, p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
    let (p0, p1, p2, p3) = (p0 as f64, p1 as f64, p2 as f64, p3 as f64);
    let x3 = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let x2 = 3.0 * p0 - 6.0 * p1 + 3.0 * p2;
    let x1 = -3.0 * p0 + 3.0 * p1;
    let x0 = p0 - x as f64;
    if x3 == 0.0 && x2 == 0.0 {
        if x1 == 0.0 {
            return 0.0;
        }
        return (-x0 / x1) as f32;
    }
    let in_range = |t: f64| (0.0..=1.0001).contains(&t);
    if x3 == 0.0 {
        let (b, c) = (x1 / x2, x0 / x2);
        if c == 0.0 {
            return 0.0;
        }
        let sqrt_d = (b * b - 4.0 * c).sqrt();
        let t = (-b + sqrt_d) / 2.0;
        return if in_range(t) { t } else { (-b - sqrt_d) / 2.0 } as f32;
    }
    let (a, b, c) = (x2 / x3, x1 / x3, x0 / x3);
    let substitute = a / 3.0;
    let p = b / 3.0 - a * a / 9.0;
    let q = (2.0 * a * a * a / 27.0 - a * b / 3.0 + c) / 2.0;
    let d = q * q + p * p * p;
    if d > 0.0 {
        let s = d.sqrt();
        return ((s - q).cbrt() - (s + q).cbrt() - substitute) as f32;
    }
    if d == 0.0 {
        let s = (-q).cbrt();
        let t = 2.0 * s - substitute;
        return if in_range(t) { t } else { -s - substitute } as f32;
    }
    // C++ uses the float `pi` promoted to double here
    let pi = ddnet::physics::vmath::PI as f64;
    let phi = (-q / (-(p * p * p)).sqrt()).acos() / 3.0;
    let s = 2.0 * (-p).sqrt();
    let t1 = s * phi.cos() - substitute;
    if in_range(t1) {
        return t1 as f32;
    }
    let t2 = -s * (phi + pi / 3.0).cos() - substitute;
    if in_range(t2) {
        return t2 as f32;
    }
    (-s * (phi - pi / 3.0).cos() - substitute) as f32
}

/// C++: `IEnvelopePointAccess::FindPointIndex`: the point before `time`, by binary search.
fn find_point_index(points: &[EnvPoint], time: i32) -> Option<usize> {
    let (mut low, mut high) = (0i32, points.len() as i32 - 2);
    while low <= high {
        let mid = low + (high - low) / 2;
        let (m, next) = (points[mid as usize].time, points[mid as usize + 1].time);
        if time >= m && time < next {
            return Some(mid as usize);
        } else if time < m {
            high = mid - 1;
        } else {
            low = mid + 1;
        }
    }
    None
}

/// C++: `CRenderMap::RenderEvalEnvelope`: writes the first `channels` values of
/// `result` at `time_ns`; `beziers` lines up with `points` when the map has them.
pub fn eval_points(
    points: &[EnvPoint],
    beziers: Option<&[Bezier]>,
    time_ns: i64,
    result: &mut [f32; 4],
    channels: usize,
) {
    let set = |result: &mut [f32; 4], p: &EnvPoint| {
        for (r, &v) in result.iter_mut().zip(&p.values).take(channels) {
            *r = fx2f(v);
        }
    };
    let Some(last) = points.last() else { return };
    if points.len() == 1 {
        return set(result, last);
    }
    let max_time = last.time as i64 * 1_000_000;
    let time_ns = if max_time > 0 { time_ns % max_time } else { 0 };
    let millis = time_ns as f64 / 1_000_000.0;
    // C++ CFixedTime(int) from a double: truncation
    let Some(i) = find_point_index(points, millis as i32) else {
        return set(result, last);
    };
    let (cur, next) = (&points[i], &points[i + 1]);
    let delta = next.time.wrapping_sub(cur.time);
    if delta <= 0 {
        return set(result, cur);
    }
    let mut a = (millis - cur.time as f64) as f32 / delta as f32;
    match cur.curve {
        CURVETYPE_STEP => a = 0.0,
        CURVETYPE_SLOW => a = a * a * a,
        CURVETYPE_FAST => {
            a = 1.0 - a;
            a = 1.0 - a * a * a;
        }
        CURVETYPE_SMOOTH => a = -2.0 * a * a * a + 3.0 * a * a,
        CURVETYPE_BEZIER => {
            if let Some((cb, nb)) = beziers.and_then(|b| Some((b.get(i)?, b.get(i + 1)?))) {
                for (c, r) in result.iter_mut().enumerate().take(channels) {
                    let p0 = (cur.time as f32, fx2f(cur.values[c]));
                    let p3 = (next.time as f32, fx2f(next.values[c]));
                    let out = (cb.out_dx[c] as f32, fx2f(cb.out_dy[c]));
                    let inn = (nb.in_dx[c] as f32, fx2f(nb.in_dy[c]));
                    let mut p1 = (p0.0 + out.0, p0.1 + out.1);
                    let mut p2 = (p3.0 + inn.0, p3.1 + inn.1);
                    p1.0 = clamp(p1.0, p0.0, p3.0);
                    p2.0 = clamp(p2.0, p0.0, p3.0);
                    let t = clamp(
                        solve_bezier(millis as f32, p0.0, p1.0, p2.0, p3.0),
                        0.0,
                        1.0,
                    );
                    *r = bezier(p0.1, p1.1, p2.1, p3.1, t);
                }
                return;
            }
        }
        _ => {} // linear, and unknown curve types
    }
    for ((r, &v0), &v1) in result
        .iter_mut()
        .zip(&cur.values)
        .zip(&next.values)
        .take(channels)
    {
        let (v0, v1) = (fx2f(v0), fx2f(v1));
        *r = v0 + (v1 - v0) * a;
    }
}

/// C++: `CMapItemEnvelope` fields the evaluation reads.
#[derive(Clone, Copy, Debug)]
pub struct Envelope {
    pub channels: i32,
    pub start: i32,
    pub num: i32,
}

/// All envelopes of a map with their points. C++: `CMapBasedEnvelopePointAccess`.
pub struct Envelopes {
    /// `None` for an item too short to hold the fields (C++ reads past it).
    pub envelopes: Vec<Option<Envelope>>,
    pub points: Vec<EnvPoint>,
    pub beziers: Option<Vec<Bezier>>,
}

impl Envelopes {
    pub fn from_map(map: &Map) -> Self {
        let df = &map.datafile;
        let items = |range: std::ops::Range<usize>| range.map(|i| &df.items[i].data);
        let mut upstream = false;
        let envelopes = items(df.get_type(MAPITEMTYPE_ENVELOPE))
            .map(|d| {
                upstream |= d.first().is_some_and(|&v| v >= VERSION_TEEWORLDS_BEZIER);
                Some(Envelope {
                    channels: *d.get(1)?,
                    start: *d.get(2)?,
                    num: *d.get(3)?,
                })
            })
            .collect();
        // C++ uses the first item of the type (sizes are whole items, in bytes / point size)
        let first = |range: std::ops::Range<usize>| range.clone().next().map(|i| &df.items[i].data);
        let point_ints = first(df.get_type(MAPITEMTYPE_ENVPOINTS)).map_or(&[][..], |d| &d[..]);
        let (points, beziers) = if upstream {
            // CEnvPointBezier_upstream: a CEnvPoint followed by its CEnvPointBezier
            let chunks = point_ints.as_chunks::<22>().0;
            (
                chunks.iter().map(|c| EnvPoint::from_ints(c)).collect(),
                Some(chunks.iter().map(|c| Bezier::from_ints(&c[6..])).collect()),
            )
        } else {
            let points: Vec<EnvPoint> = point_ints
                .as_chunks::<6>()
                .0
                .iter()
                .map(|c| EnvPoint::from_ints(c))
                .collect();
            let bezier_item = (0..df.items.len())
                .find(|&i| df.item_uuid(i) == Some(ENVPOINTS_BEZIER_UUID))
                .map(|i| &df.items[i].data);
            let beziers = bezier_item
                .filter(|d| d.len() / 16 == points.len())
                .map(|d| {
                    d.as_chunks::<16>()
                        .0
                        .iter()
                        .map(|c| Bezier::from_ints(c))
                        .collect()
                });
            (points, beziers)
        };
        Self {
            envelopes,
            points,
            beziers,
        }
    }

    /// C++: `CEnvelopeState::EnvelopeEval` at `time_ns` (the envelope clock) plus the
    /// layer's or quad's offset in ms. Leaves `result` alone for invalid envelopes.
    pub fn eval(
        &self,
        time_ns: i64,
        offset_ms: i32,
        index: i32,
        result: &mut [f32; 4],
        channels: usize,
    ) {
        let Some(Some(env)) = usize::try_from(index)
            .ok()
            .and_then(|i| self.envelopes.get(i))
        else {
            return;
        };
        if env.channels <= 0 {
            return;
        }
        let channels = channels.min(env.channels as usize).min(4);
        // C++: SetPointsRange
        let max = self.points.len() as i32;
        let start = env.start.clamp(0, max);
        let num = env.num.clamp(0, max - start);
        if num == 0 {
            return;
        }
        let range = start as usize..(start + num) as usize;
        let beziers = self.beziers.as_ref().map(|b| &b[range.clone()]);
        let time = time_ns.wrapping_add(offset_ms as i64 * 1_000_000);
        eval_points(&self.points[range], beziers, time, result, channels);
    }
}
