//! **The spill: a fan of strands out of a wound, a pure function of its seed.**

use core::f32::consts::{FRAC_PI_6, TAU};

use bevy::math::Vec3;

use crate::viscera::frame::perpendicular_basis;
use crate::viscera::hash::{hash_pair, hash_unit};
use crate::viscera::settings::ViscSettings;
use crate::viscera::strand::{axis_or_down, Strand};

/// Segments per spilled strand. 24 segments is 25 nodes, inside [`crate::viscera::MAX_NODES`].
pub const SPILL_SEGMENTS: u32 = 24;
/// Rest length of a spilled segment, metres. 24 × 35 mm is an 84 cm run of bowel.
pub const SPILL_REST_LEN: f32 = 0.035;
/// Tube radius of a spilled strand, metres.
pub const SPILL_RADIUS: f32 = 0.02;
/// Half-angle of the cone the fan is drawn from, radians (30°).
pub const SPILL_CONE: f32 = FRAC_PI_6;

/// **Spill `count` strands out of `from`, fanned around `dir`.**
///
/// `count` is clamped to [`ViscSettings::max_strands`]; every other quantity — each strand's direction
/// inside the cone and its offset across the wound — comes out of `seed` through the crate's own
/// integer hash, so this is a pure function of `(from, dir, count, seed, max_strands)` and nothing
/// else. There is no RNG state to carry, so calling it twice in a frame cannot make the second call
/// different from the first.
///
/// Directions are drawn uniformly *over the spherical cap* — polar angle `θ = cone · √u` rather than
/// `cone · u` — so the fan does not pile up along the axis.
pub fn spill(from: Vec3, dir: Vec3, count: u32, seed: u32, s: &ViscSettings) -> Vec<Strand> {
    let count = count.min(s.max_strands) as usize;
    let axis = axis_or_down(dir);
    let (u, v) = perpendicular_basis(axis);

    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let k = k as u32;
        let azimuth = TAU * hash_unit(hash_pair(seed, k * 3));
        let polar = SPILL_CONE * hash_unit(hash_pair(seed, k * 3 + 1)).sqrt();
        let spread = hash_unit(hash_pair(seed, k * 3 + 2));

        let (sin_a, cos_a) = azimuth.sin_cos();
        let (sin_p, cos_p) = polar.sin_cos();
        let radial = u * cos_a + v * sin_a;
        let heading = axis * cos_p + radial * sin_p;
        // Strands leave through the same hole but not through the same point of it.
        let mouth = from + radial * (SPILL_RADIUS * 2.0 * spread);

        out.push(Strand::new(mouth, heading, SPILL_SEGMENTS, SPILL_REST_LEN, SPILL_RADIUS));
    }
    out
}
