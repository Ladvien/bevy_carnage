//! **The conversion boundary, and the only file allowed to be it.**
//!
//! `bloodstain` names no math library: every vector in its public API is `[f32; 3]`. That is not an
//! omission, it is the property that lets it be a leaf — the workspace this crate lives in resolves
//! **two** `glam` versions at once, so a crate naming either could collide with a consumer naming the
//! other, and naming neither cannot.
//!
//! The cost of that is a conversion, and the cost is deliberately paid **here and nowhere else**. Two
//! `#[inline]` functions over three floats compile to a register shuffle, and they are called **per
//! wound**, not per vertex — a wound is an event that happens a handful of times a frame, while a
//! vertex happens a hundred thousand times a bake. `isomesh` was admitted to this crate on the same
//! terms and pays the same toll in `audit.rs`.
//!
//! **If a second file starts converting, the boundary has leaked.** The rule is enforceable by
//! grepping for `to_v3`/`from_v3` outside this module's callers, and it exists because a conversion
//! scattered across ten files is how a `[f32; 3]` API ends up with a `glam` shim beside it.

use bevy::math::Vec3;

/// `glam::Vec3` → `bloodstain`'s `[f32; 3]`.
#[inline]
pub(crate) fn to_v3(v: Vec3) -> bloodstain::V3 {
    [v.x, v.y, v.z]
}

/// `bloodstain`'s `[f32; 3]` → `glam::Vec3`.
#[inline]
pub(crate) fn from_v3(v: bloodstain::V3) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}

/// This crate's [`Wound`](crate::Wound) as the leaf's mirror of it.
///
/// **Two structs for one idea, and the duplication is the boundary rather than an oversight.** This
/// crate's `Wound` is the type its fracture, bond graph and ECS already speak, in `glam`;
/// `bloodstain::Wound` is the one a leaf with no math library can speak. The conversion is five field
/// copies and it happens once per wound.
/// **Allowed to be unused with `vfx` off, and that is a fact about the readers rather than about the
/// boundary.** The only caller today is the particle reader, which is cosmetic; the conversion still
/// has to exist and still has to be right, which is what the tests below pin. Deleting it under the
/// feature would mean the boundary appeared and disappeared with a renderer.
#[cfg_attr(not(feature = "vfx"), allow(dead_code))]
#[inline]
pub(crate) fn wound(w: &crate::Wound) -> bloodstain::Wound {
    bloodstain::Wound {
        at: to_v3(w.at),
        normal: to_v3(w.normal),
        area: w.area,
        severity: w.severity,
        kind: w.kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The round trip is exact. Not "close": these are the same bits, because the conversion is a
    /// copy and nothing in it computes.
    #[test]
    fn the_round_trip_is_bit_exact() {
        for v in [
            Vec3::ZERO,
            Vec3::new(0.1, -0.9, 1.0e-7),
            Vec3::new(f32::MIN_POSITIVE, 1.0e30, -0.0),
        ] {
            let back = from_v3(to_v3(v));
            assert_eq!(
                (back.x.to_bits(), back.y.to_bits(), back.z.to_bits()),
                (v.x.to_bits(), v.y.to_bits(), v.z.to_bits()),
                "the conversion must be a copy, not an approximation"
            );
        }
    }

    /// A wound crosses the boundary unchanged, including the discriminant the blood seed is mixed
    /// from — a mirror that dropped `kind` would seed every wound identically.
    #[test]
    fn a_wound_crosses_the_boundary_unchanged() {
        let w = crate::Wound {
            at: Vec3::new(0.1, 0.9, -0.2),
            normal: Vec3::X,
            area: 0.004,
            severity: 0.5,
            kind: crate::WoundKind::Channel,
        };
        let m = wound(&w);
        assert_eq!(m.at, [0.1, 0.9, -0.2]);
        assert_eq!(m.normal, [1.0, 0.0, 0.0]);
        assert_eq!(m.area, w.area);
        assert_eq!(m.severity, w.severity);
        assert_eq!(m.kind, crate::WoundKind::Channel);
        assert_eq!(
            bloodstain::wound_seed(&m),
            bloodstain::wound_seed(&wound(&w)),
            "the mirror must be a pure function of the wound"
        );
    }
}
