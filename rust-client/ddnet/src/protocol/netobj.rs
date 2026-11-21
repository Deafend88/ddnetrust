//! Snapshot object validation. C++: generated `CNetObjHandler` (`src/generated/protocol.cpp`,
//! produced by `datasrc/compile.py`); the tables come from `generated.rs`.

use super::generated::ObjSpec;

/// C++: `CNetObjHandler::SecureUnpackObj` for item types > 0. Reads one int per
/// field (a missing trailing field takes its default, or fails without one), ignores
/// extra ints, then clamps ranged fields. Returns the unpacked object, or `None`.
pub fn secure_unpack_obj(spec: &ObjSpec, data: &[i32]) -> Option<Vec<i32>> {
    let mut out = Vec::with_capacity(spec.fields.len());
    let mut data = data.iter();
    for field in spec.fields {
        out.push(match data.next() {
            Some(&v) => v,
            None => field.default?,
        });
    }
    for (v, field) in out.iter_mut().zip(spec.fields) {
        if let Some((min, max)) = field.range {
            // C++ `ClampInt`; not `i32::clamp`, which panics if min > max
            if *v < min {
                *v = min;
            } else if *v > max {
                *v = max;
            }
        }
    }
    Some(out)
}
