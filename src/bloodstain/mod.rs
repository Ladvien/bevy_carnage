#![doc = include_str!("../../docs/bloodstain.md")]



pub mod bag;
pub mod bleed;
pub mod bruise;
pub mod burn;
pub mod droplet;
pub mod dry;
pub mod origin;
pub mod patterns;
pub mod pool;
pub mod rheo;
pub mod settings;
pub mod spectral;
pub mod stain;
pub mod wick;

pub use bag::pick;
pub use bleed::{Bleed, flow, pulse_period, pulse_phase, pulse_wound, pulses_on};
pub use bruise::Bruise;
pub use burn::{Burn, Degree};
pub use droplet::{
    BACK_SPATTER_SPEED, BLOOD_DENSITY, BLOOD_SURFACE_TENSION, Droplet, FORWARD_SPATTER_SPEED,
    droplet, droplet_count, droplets, landing, wound_seed,
};
pub use dry::{Appearance, appearance, appearance_of, appearance_with_fresh};
pub use origin::area_of_origin;
pub use patterns::{
    PatternClass, arterial_arc, cast_off, drip_trail, expirated, impact_spatter, transfer,
};
pub use pool::{POOL_CAP_SLACK, Pool, absorb, spread_pools};
pub use rheo::{flows, viscosity, yield_stress};
pub use settings::BloodSettings;
pub use spectral::{Film, SO2_ARTERIAL, SO2_VENOUS};
pub use stain::{Impact, Stain, StainShape, rasterise, reynolds, stain_shape, stains, weber};
pub use wick::Sheet;

/// **The whole public vector type.** Three floats, no math library.
///
/// `isomesh` earned its place in this family of crates on exactly this term and it is worth restating:
/// the workspace this crate was carved out of resolves **two** `glam` versions at once, so a leaf that
/// named either one could collide with a consumer that named the other. Naming neither is the only
/// choice that cannot. A consumer converts at its own boundary, once per wound rather than once per
/// vertex — see `bevy_carnage`'s `src/v3.rs`, which is the only file in that crate allowed to do it.
pub type V3 = [f32; 3];

/// **Every transcendental this crate evaluates, in one place, from one implementation.**
///
/// `libm` unconditionally rather than `std`'s math behind a feature, because a second math path is a
/// second set of bits and this crate's product is a frozen model. Measured before adopting it: at the
/// spatter golden's own inputs `libm::{sinf, cosf, sqrtf}` are bit-identical to the platform libm the
/// model was blessed against. `powf`/`expf` differ by one ULP at some inputs and are read only by code
/// written here, never by a moved golden.
pub(crate) mod m {
    #[inline]
    pub(crate) fn sqrt(x: f32) -> f32 {
        libm::sqrtf(x)
    }
    #[inline]
    pub(crate) fn sin(x: f32) -> f32 {
        libm::sinf(x)
    }
    #[inline]
    pub(crate) fn cos(x: f32) -> f32 {
        libm::cosf(x)
    }
    #[inline]
    pub(crate) fn atan2(y: f32, x: f32) -> f32 {
        libm::atan2f(y, x)
    }
    #[inline]
    pub(crate) fn exp(x: f32) -> f32 {
        libm::expf(x)
    }
    #[inline]
    pub(crate) fn powf(x: f32, y: f32) -> f32 {
        libm::powf(x, y)
    }
    #[inline]
    pub(crate) fn abs(x: f32) -> f32 {
        libm::fabsf(x)
    }
    #[inline]
    pub(crate) fn round(x: f32) -> f32 {
        libm::roundf(x)
    }
}

/// Degrees to radians, spelled out rather than taken from `f32::to_radians`.
///
/// Identical arithmetic — one multiply by `PI / 180` — and available in `core`, which
/// `f32::to_radians` is not on every toolchain this must build on.
#[inline]
pub(crate) fn to_radians(deg: f32) -> f32 {
    deg * (core::f32::consts::PI / 180.0)
}

/// **`[f32; 3]` arithmetic, mirroring `glam::Vec3` operation for operation.**
///
/// Not "equivalent maths" — the *same* operations in the *same* order, because this module's output
/// feeds a frozen golden that was blessed against glam. Each function names the glam source it
/// mirrors so a future edit can check it rather than assume it.
pub mod vec {
    use crate::bloodstain::{V3, m};

    /// `a + b`, componentwise.
    #[inline]
    pub fn add(a: V3, b: V3) -> V3 {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }

    /// `a - b`, componentwise.
    #[inline]
    pub fn sub(a: V3, b: V3) -> V3 {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    /// `a * s`, componentwise.
    #[inline]
    pub fn scale(a: V3, s: f32) -> V3 {
        [a[0] * s, a[1] * s, a[2] * s]
    }

    /// `glam::Vec3::dot`: `(x·x) + (y·y) + (z·z)`, summed left to right.
    #[inline]
    pub fn dot(a: V3, b: V3) -> f32 {
        (a[0] * b[0]) + (a[1] * b[1]) + (a[2] * b[2])
    }

    /// `glam::Vec3::cross`, componentwise in glam's own order.
    #[inline]
    pub fn cross(a: V3, b: V3) -> V3 {
        [
            a[1] * b[2] - b[1] * a[2],
            a[2] * b[0] - b[2] * a[0],
            a[0] * b[1] - b[0] * a[1],
        ]
    }

    /// `glam::Vec3::length`: `sqrt(dot(self, self))`.
    #[inline]
    pub fn length(a: V3) -> f32 {
        m::sqrt(dot(a, a))
    }

    /// `glam::Vec3::length_squared`.
    #[inline]
    pub fn length_squared(a: V3) -> f32 {
        dot(a, a)
    }

    /// `glam::Vec3::distance_squared`.
    #[inline]
    pub fn distance_squared(a: V3, b: V3) -> f32 {
        length_squared(sub(a, b))
    }

    /// `glam::Vec3::normalize_or_zero`: multiply by `length().recip()` when that reciprocal is finite
    /// and positive, and hand back zero otherwise.
    ///
    /// The branch is the whole point and it is glam's, not an addition: a zero-length input has no
    /// direction, and inventing one for it would be a fabricated answer.
    #[inline]
    pub fn normalize_or_zero(a: V3) -> V3 {
        let rcp = 1.0 / length(a);
        if rcp.is_finite() && rcp > 0.0 { scale(a, rcp) } else { [0.0, 0.0, 0.0] }
    }

    /// `glam::Vec3::lerp`: `self * (1 - s) + rhs * s`.
    #[inline]
    pub fn lerp(a: V3, b: V3, s: f32) -> V3 {
        add(scale(a, 1.0 - s), scale(b, s))
    }

    /// The `+Y` axis. Blood falls along `-Y`; every function here that needs "up" reads this.
    pub const Y: V3 = [0.0, 1.0, 0.0];
    /// The `+X` axis.
    pub const X: V3 = [1.0, 0.0, 0.0];
    /// The origin.
    pub const ZERO: V3 = [0.0, 0.0, 0.0];
}

/// **Endpoint-weld lattice step**, and the quantisation every seed in this crate is taken on.
///
/// One home, shared with the fracture that used to own it: `bevy_carnage`'s `soup::WELD` *is* this
/// constant, re-exported. Two copies of a quantisation step is how a wound seeds one way in the blood
/// model and another way in the geometry that opened it.
pub const WELD: f32 = 1.0e-4;

/// **The crate's only random source: a 32-bit integer hash mapped into `[0, 1)`.**
///
/// **Hand-rolled, and pinned.** There is deliberately no RNG crate here. The whole reproducibility
/// argument rests on this function returning the same bits on every machine and every toolchain, and a
/// dependency that reserves the right to change its stream between minor versions cannot promise that.
/// Its exact output is frozen by [`tests::hash_f32_is_frozen`] below.
///
/// **Moved here from `bevy_carnage::soup` verbatim**, and that crate re-exports this symbol — its
/// consumer's `tests/rng_guard.rs` asserts the game's own `util::hash_f32` is bit-identical to it, so
/// the function had to keep both its name and its bits.
pub fn hash_f32(x: u32) -> f32 {
    let mut h = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    h = ((h >> ((h >> 28).wrapping_add(4))) ^ h).wrapping_mul(277_803_737);
    h = (h >> 22) ^ h;
    (h as f32) / (u32::MAX as f32)
}

/// Two orthonormal in-plane axes for a given plane normal.
///
/// **One home, and `bevy_carnage::soup::plane_basis` delegates to it.** Every direction in both crates
/// is derived against this basis, so a spray and a cut face agree about what "sideways" means — and a
/// second copy of these four lines is how they would stop agreeing.
pub fn plane_basis(n: V3) -> (V3, V3) {
    let a = if m::abs(n[0]) < 0.9 { vec::X } else { vec::Y };
    let u = vec::normalize_or_zero(vec::cross(n, a));
    let v = vec::cross(n, u);
    (u, v)
}

/// What opened a wound.
///
/// **`u32`-valued and part of the seed**, so a severance and a channel at the same point do not throw
/// the same blood. The discriminants are written out because [`droplet::wound_seed`] casts this, and a
/// reordered enum would silently move every seed and every golden that depends on one.
///
/// **Append-only.** A new variant goes on the end with the next number; nothing is ever renumbered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u32)]
pub enum WoundKind {
    /// Two fragments stopped sharing a face.
    Severance = 0,
    /// A bore left an interior wall open to the air.
    Channel = 1,
}

/// **A wound, engine-free.** Where a subject came open, which way it faces, how wide and how badly.
///
/// The mirror of `bevy_carnage::Wound`, which is the same five fields in `glam::Vec3`. Two structs for
/// one idea is a real cost, paid deliberately and exactly once: the consumer's type is the one its
/// fracture, bond graph and ECS already speak, this one is the one a leaf with no math library can
/// speak, and the conversion between them lives in a single file (`bevy_carnage/src/v3.rs`) so it
/// cannot spread.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Wound {
    /// Where it is, subject-local — the centre of the opened surface.
    pub at: V3,
    /// Which way it faces, unit. Blood leaves along this.
    pub normal: V3,
    /// How much surface came open, subject-local units squared. Drives droplet count.
    pub area: f32,
    /// How badly, in `[0, 1]`. `1.0` is fully open; a pulse's taper scales this and nothing else.
    pub severity: f32,
    /// Which of the two things happened.
    pub kind: WoundKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The generator is frozen.**
    ///
    /// These bits are the whole reproducibility story: [`hash_f32`] drives every cut plane's direction
    /// in the fracture crate downstream and every draw in this one, so a changed constant
    /// re-partitions every mesh and re-throws every spray. Treat this as a lock, not a snapshot to
    /// re-bless: if it goes red, the model moved.
    ///
    /// **The same eight values this test asserted in `bevy_carnage::soup`**, under the same name. A
    /// moved golden that shifts means the move was wrong.
    #[test]
    fn hash_f32_is_frozen() {
        let got: std::vec::Vec<u32> = (0..8u32).map(|i| hash_f32(i).to_bits()).collect();
        assert_eq!(
            got,
            [
                1022846460, 1059634922, 1056243097, 1056841197, 1042407458, 1057018071, 1064390834,
                1056755236
            ],
            "the generator moved. Every cut plane's direction and every droplet draw comes from \
             these bits."
        );
        // Every value must land in [0, 1) — the contract every caller multiplies against.
        for i in 0..1024u32 {
            let v = hash_f32(i);
            assert!((0.0..1.0).contains(&v), "hash_f32({i}) escaped [0, 1)");
        }
    }

    /// The basis is orthonormal and right-handed for any unit normal, which is what every cone and
    /// every stain orientation in the crate assumes.
    #[test]
    fn the_plane_basis_is_orthonormal() {
        for n in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [0.6, 0.8, 0.0]] {
            let n = vec::normalize_or_zero(n);
            let (u, v) = plane_basis(n);
            for (name, len) in [("u", vec::length(u)), ("v", vec::length(v))] {
                assert!(m::abs(len - 1.0) < 1.0e-5, "{name} axis length {len} for normal {n:?}");
            }
            assert!(m::abs(vec::dot(u, v)) < 1.0e-5, "u and v are not perpendicular");
            assert!(m::abs(vec::dot(u, n)) < 1.0e-5, "u is not in the plane");
            assert!(m::abs(vec::dot(v, n)) < 1.0e-5, "v is not in the plane");
        }
    }

    /// A degenerate normal has no basis, and the honest answer is a zero axis rather than an invented
    /// direction — the same refusal `vec::normalize_or_zero` makes.
    #[test]
    fn a_zero_normal_has_no_basis() {
        let (u, _) = plane_basis(vec::ZERO);
        assert_eq!(u, vec::ZERO, "a zero normal must not be given a fabricated basis");
    }
}
