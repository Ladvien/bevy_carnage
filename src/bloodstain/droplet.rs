//! **The spatter model.** A wound, and the blood that leaves it.
//!
//! Pure functions, integer-seeded, frozen by a golden. Nothing here spawns anything, reads a clock or
//! touches an ECS: a caller passes a [`Wound`] and gets values back.
//!
//! # The physics this is a reduction of
//!
//! Comiskey, Yarin & Attinger, *"Theoretical and experimental investigation of forward spatter of
//! blood from a gunshot"*, Phys. Rev. Fluids **3**, 063901 (2018), `doi:10.1103/physrevfluids.3.063901`,
//! and its companion on back spatter, `doi:10.1103/physrevfluids.2.073906`.
//!
//! Their model is not "blood is sprayed": a blood layer accelerated off a surface disintegrates by
//! **percolation**. The layer breaks into clusters of an indivisible droplet `a₀`, whose size is set
//! by the balance of the kinetic energy of the stretching layer against the surface energy it must pay
//! to make new interface,
//!
//! > `½ ρ a₀³ (ε̇ a₀)² = γ a₀²`
//!
//! and a cluster of `n` such droplets coalesces into one droplet of diameter `∝ n^(1/3)`. The
//! consequence that matters is a **correlation, not a distribution**: a large droplet is a large
//! cluster, a large cluster took longer to assemble and carries more mass per unit of the same
//! impulse, so **many small droplets leave fast and few large ones leave slow**. Their measurements
//! bracket it — forward spatter at ~40 m/s and back spatter at ~8 m/s, 0.45 ms after impact.
//!
//! That inverse size–speed correlation is what makes a spray read as blood rather than as confetti,
//! and reproducing the exact PDF would not add to it at game scale. So the correlation is what the
//! code implements and what [`tests::size_and_speed_are_inversely_correlated`] asserts, rather than
//! something a comment claims.
//!
//! # Determinism
//!
//! [`wound_seed`] is a hash of **where the wound is**, quantised on [`crate::bloodstain::WELD`]. Nothing is
//! threaded down and nothing accumulates, so any droplet of any wound can be recomputed alone, in any
//! order, on any machine, and [`droplets`] is only a convenience over [`droplet`]. [`crate::bloodstain::hash_f32`]
//! is the only source of randomness in this module, as it is in the whole crate.

use core::f32::consts::TAU;

use crate::bloodstain::settings::BloodSettings;
use crate::bloodstain::{V3, WELD, Wound, hash_f32, m, plane_basis, to_radians, vec};

/// Blood density, kg/m³ — the `ρ` of Comiskey et al. 2018's energy balance.
///
/// Recorded because it is what sets the indivisible droplet size the whole percolation argument rests
/// on. Not read by the game-scale reduction below, which takes the *measured* droplet speeds instead
/// of re-deriving them; kept so the constant a later derivation would need is here with its source
/// rather than looked up again.
pub const BLOOD_DENSITY: f32 = 1060.0;
/// Blood surface tension, N/m — the `γ` of the same balance (ibid., 60.45 mN/m).
pub const BLOOD_SURFACE_TENSION: f32 = 0.060_45;
/// Measured forward-spatter droplet speed 0.45 ms after impact, m/s (ibid., §IV).
///
/// The **fast** end of the span, and it belongs to the **smallest** droplets — see the module docs.
pub const FORWARD_SPATTER_SPEED: f32 = 40.0;
/// Measured backward-spatter droplet speed at the same instant, m/s (ibid., §IV).
///
/// The **slow** end of the span, and it belongs to the **largest** droplets.
pub const BACK_SPATTER_SPEED: f32 = 8.0;

/// One ejected droplet, subject-local. No entity, no lifetime — a value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Droplet {
    /// Unit direction it left along, inside the spray cone about the wound normal.
    pub dir: V3,
    /// Initial speed, m/s.
    pub speed: f32,
    /// Diameter, metres. Inversely correlated with [`speed`](Self::speed) — that is the model.
    pub diameter: f32,
}

/// **Seed for one wound — a pure function of WHERE it is, never of history.**
///
/// Positions are snapped to [`crate::bloodstain::WELD`] before hashing, so two runs that place the wound a float
/// ULP apart still seed identically, and a wound a tenth of a millimetre away is a different spray.
///
/// **[`WoundKind`](crate::bloodstain::WoundKind) is mixed in**, so a severance and a channel that happen to open
/// at the same point do not throw the same blood. That is why the enum's discriminants are written
/// out: reordering them would silently move every seed.
///
/// Deliberately *not* seeded from an accumulator, an entity id, an asset id or a clock. Each of those
/// has its own recorded failure in this family of crates — an arena slot is assigned by load order, a
/// drain counter desynchronises permanently after any single difference.
pub fn wound_seed(w: &Wound) -> u32 {
    let q = |x: f32| m::round(x / WELD) as i64 as u32;
    q(w.at[0])
        ^ q(w.at[1]).wrapping_mul(0x9E37_79B9)
        ^ q(w.at[2]).wrapping_mul(2_654_435_761)
        ^ (w.kind as u32).wrapping_mul(0x85EB_CA6B)
}

/// How many droplets a wound throws: area × density × severity, clamped.
///
/// **Area, not per hit** — a wound is a surface, and how much blood leaves it is a property of how
/// much of it is open. The clamp is what keeps one enormous cut inside a particle effect's fixed
/// capacity; a severity of zero throws nothing, which is what a fully clotted wound is.
pub fn droplet_count(w: &Wound, s: &BloodSettings) -> u32 {
    if !(w.area > 0.0) || !(w.severity > 0.0) || !w.area.is_finite() {
        return 0;
    }
    let n = w.area * s.droplets_per_m2 * w.severity.clamp(0.0, 1.0);
    if !n.is_finite() {
        return 0;
    }
    (m::round(n).max(0.0) as u32).min(s.max_droplets_per_wound)
}

/// One droplet of the spray, by ordinal.
///
/// `index` is the droplet's own number, so the whole set is a pure function of `(wound, settings)`
/// and **any subset can be recomputed without the rest** — which is what lets a caller drop half a
/// spray for budget and still have the other half be the same blood it would have been.
///
/// Three draws, from three rotations of one key:
///
/// 1. **Size fraction.** `diameter` lerps min→max across it, and `speed` lerps fast→slow across the
///    *same* fraction. That single inversion is the paper's correlation, and it is the whole reason
///    this function is not three independent random numbers.
/// 2. **Azimuth** about the wound normal, over the full circle.
/// 3. **Polar angle**, as `cone · √v` rather than `cone · v` — the square root is what makes the
///    directions uniform per unit solid angle instead of piling up at the cone's rim.
///
/// The cone is built on [`crate::bloodstain::plane_basis`], the same basis every other direction in this family
/// is derived against, so a spray and a cut face agree about what "sideways" means.
pub fn droplet(w: &Wound, index: u32, s: &BloodSettings) -> Droplet {
    Spray::of(w, s).droplet(index, s)
}

/// **Everything about a wound's spray that does not depend on which droplet you ask for.**
///
/// Built once and reused, because every field below used to be recomputed per droplet ordinal, and
/// the callers that want a whole spray ([`droplets`] and [`crate::bloodstain::stain::stains`]) ask for hundreds
/// each. Per droplet that was: one normalisation of the wound normal, one [`crate::bloodstain::plane_basis`] —
/// itself two cross products and a second normalisation — one degree conversion, and one
/// [`wound_seed`].
///
/// **Bit-identical, which is the only reason this is a hoist rather than a change.** Same inputs,
/// same operations, same order; only the number of times they run differs. [`droplet`] still builds
/// one and throws it away, so the single-shot public path computes exactly what it always did.
#[derive(Clone, Copy)]
pub(crate) struct Spray {
    /// Unit spray axis, or zero — see the note in [`Spray::of`].
    pub(crate) axis: V3,
    tangent: V3,
    bitangent: V3,
    /// Cone half-angle, radians.
    theta_max: f32,
    /// This wound's seed, mixed with the droplet ordinal to key each draw.
    pub(crate) seed: u32,
}

impl Spray {
    pub(crate) fn of(w: &Wound, s: &BloodSettings) -> Self {
        // A wound with no normal has no direction to spray along; `plane_basis` would hand back a
        // degenerate frame. Spraying straight up is a fabricated answer, so the honest one is the
        // axis itself, which for a zero normal is zero and throws blood nowhere.
        let axis = vec::normalize_or_zero(w.normal);
        let (tangent, bitangent) = plane_basis(axis);
        Self {
            axis,
            tangent,
            bitangent,
            theta_max: to_radians(s.spatter_cone_deg),
            seed: wound_seed(w),
        }
    }

    pub(crate) fn droplet(&self, index: u32, s: &BloodSettings) -> Droplet {
        let key = self.seed ^ index.wrapping_mul(0x9E37_79B9);
        let t = hash_f32(key);
        let u = hash_f32(key ^ 0x85EB_CA6B);
        let v = hash_f32(key ^ 0xC2B2_AE35);

        let diameter = s.droplet_size_min + (s.droplet_size_max - s.droplet_size_min) * t;
        // The inversion. Largest droplet, slowest speed.
        let speed = (FORWARD_SPATTER_SPEED + (BACK_SPATTER_SPEED - FORWARD_SPATTER_SPEED) * t)
            * s.spatter_speed_scale;

        let phi = TAU * u;
        let theta = self.theta_max * m::sqrt(v.clamp(0.0, 1.0));
        let dir = vec::normalize_or_zero(vec::add(
            vec::scale(self.axis, m::cos(theta)),
            vec::scale(
                vec::add(
                    vec::scale(self.tangent, m::cos(phi)),
                    vec::scale(self.bitangent, m::sin(phi)),
                ),
                m::sin(theta),
            ),
        ));

        Droplet { dir, speed, diameter }
    }
}

/// The whole spray, in droplet-index order.
pub fn droplets(w: &Wound, s: &BloodSettings) -> Vec<Droplet> {
    let spray = Spray::of(w, s);
    let n = droplet_count(w, s);
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        out.push(spray.droplet(i, s));
    }
    out
}

/// Closed-form landing point on a horizontal plane. `None` if it starts at or below the plane.
///
/// Solves `plane_y = from.y + v_y·t − ½·g·t²` for the positive root and evaluates the horizontal
/// motion at that `t`. **Closed form rather than stepped**, because a stepped integration would need
/// a timestep, and a timestep is a clock — which the determinism contract forbids on this side. The
/// landing `y` is *assigned* `plane_y` rather than computed, so a stain is exactly on the plane it
/// stained instead of a float's width above or below it.
///
/// `None` for a droplet that starts at or under the plane, or whose discriminant is negative: both
/// mean it never crosses, and inventing a landing point for it would be a fabricated result.
pub fn landing(from: V3, d: &Droplet, gravity: f32, plane_y: f32) -> Option<V3> {
    let h = from[1] - plane_y;
    if !(h > 0.0) || !h.is_finite() {
        return None;
    }
    let vy = d.dir[1] * d.speed;
    let t = if m::abs(gravity) <= f32::EPSILON {
        // No gravity: it only ever reaches the plane if it is already heading down.
        if vy >= 0.0 {
            return None;
        }
        h / -vy
    } else {
        let disc = vy * vy + 2.0 * gravity * h;
        if disc < 0.0 {
            return None;
        }
        (vy + m::sqrt(disc)) / gravity
    };
    if !(t > 0.0) || !t.is_finite() {
        return None;
    }
    let mut at = vec::add(from, vec::scale(vec::scale(d.dir, d.speed), t));
    at[1] = plane_y;
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bloodstain::WoundKind;
    use std::vec::Vec as StdVec;

    /// The wound the golden is taken against. A named constant so the golden and every property test
    /// are measuring the same geometry.
    pub(crate) fn fixed_wound() -> Wound {
        Wound {
            at: [0.1, 0.9, -0.2],
            normal: vec::X,
            area: 0.004,
            severity: 1.0,
            kind: WoundKind::Severance,
        }
    }

    /// **These bits are the API.**
    ///
    /// The same lock [`crate::bloodstain::tests::hash_f32_is_frozen`] puts on the generator, for the same reason:
    /// a caller's replay, a recorded demo and a golden digest downstream are all defined against
    /// these exact values. A change to the draw order, the key rotations, the lerp direction or the
    /// cone construction moves every one of them.
    ///
    /// **This is not a snapshot to re-bless.** If it fails, the model moved, and the question is
    /// whether that was intended — not whether the numbers should be updated to match.
    ///
    /// # Two things this table has already survived, both worth knowing before touching it
    ///
    /// **A reference-profile change.** These values were first blessed at opt-level 0; three of the
    /// forty differ by exactly one ULP at opt-level 1, which is what the workspace's `[profile.dev]`
    /// uses, and the table holds the opt-level-1 values because that is the profile the shipping
    /// build and the gate actually use. Diagnosed rather than assumed: standalone at opt-level 0 both
    /// frozen tests pass, and `CARGO_PROFILE_TEST_OPT_LEVEL=1` is the only variable that reproduces
    /// the failure. `-Cllvm-args=--fp-contract=off` did **not** restore the old values, so it is not
    /// simple FMA contraction.
    ///
    /// **A move between crates, and a change of math library with it.** This model came out of
    /// `bevy_carnage::spatter`, which called `std`'s `sinf`/`cosf`/`sqrtf` on `glam::Vec3`; here it
    /// calls `libm`'s on `[f32; 3]`. **Measured before the move, not after:** at these inputs the two
    /// libraries agree bit for bit, and `crate::bloodstain::vec` mirrors glam operation for operation. The table
    /// below is unchanged from the one that shipped in that crate, which is the evidence the port was
    /// a move rather than a rewrite.
    ///
    /// So the rule is unchanged in substance: **if these move while the profile is held fixed, the
    /// model moved.** Re-bless only for a profile change, and say which profile.
    #[test]
    fn the_spatter_model_is_frozen() {
        let w = fixed_wound();
        let s = BloodSettings::default();

        assert_eq!(wound_seed(&w), 2_698_380_592, "the wound seed itself is part of the contract");

        let expect: [([u32; 3], u32, u32); 8] = [
            ([0x3F6517F7, 0xBE1A5BEA, 0x3ED70FF4], 0x41F61DBD, 0x3B16C870),
            ([0x3F5EF5C2, 0x3EC30692, 0xBE9EF29A], 0x41948C24, 0x3B8C554F),
            ([0x3F6E4F76, 0x3EA1C3E5, 0x3E3BB593], 0x4162CCF4, 0x3BA3BA27),
            ([0x3F7D49A7, 0x3BBC9904, 0xBE148C9F], 0x420F2261, 0x3AC2AA04),
            ([0x3F616999, 0x3EF132EC, 0x3D573EAE], 0x41C13CCA, 0x3B5D2CD3),
            ([0x3F6F2E11, 0xBE44246A, 0xBE99F0F8], 0x41BF2962, 0x3B5FF03C),
            ([0x3F784E4D, 0x3D42166C, 0x3E74650E], 0x41CEE206, 0x3B4B02A2),
            ([0x3F73318E, 0x3E5C82D3, 0xBE67A60D], 0x4213499A, 0x3AAC8C8E),
        ];

        let mut actual = StdVec::new();
        for i in 0..8u32 {
            let d = droplet(&w, i, &s);
            actual.push((
                [d.dir[0].to_bits(), d.dir[1].to_bits(), d.dir[2].to_bits()],
                d.speed.to_bits(),
                d.diameter.to_bits(),
            ));
        }
        let rendered: StdVec<std::string::String> = actual
            .iter()
            .map(|(dir, sp, di)| {
                std::format!(
                    "([0x{:08X}, 0x{:08X}, 0x{:08X}], 0x{sp:08X}, 0x{di:08X}),",
                    dir[0],
                    dir[1],
                    dir[2]
                )
            })
            .collect();
        assert_eq!(
            actual.as_slice(),
            expect.as_slice(),
            "the spatter model moved. If that was deliberate, the new bits are:\n{}",
            rendered.join("\n")
        );
    }

    /// **The paper's invariant, asserted rather than assumed.**
    ///
    /// Many small droplets fast, few large ones slow. Measured as a Pearson correlation over a real
    /// sample, because that is what the property is — one droplet proves nothing, and a spray whose
    /// sizes and speeds were independent would still pass any single-droplet check.
    #[test]
    fn size_and_speed_are_inversely_correlated() {
        let w = fixed_wound();
        let s = BloodSettings::default();
        let n = 256usize;
        let d: StdVec<Droplet> = (0..n as u32).map(|i| droplet(&w, i, &s)).collect();

        let mean = |f: &dyn Fn(&Droplet) -> f32| d.iter().map(f).sum::<f32>() / n as f32;
        let (md, ms) = (mean(&|x: &Droplet| x.diameter), mean(&|x: &Droplet| x.speed));
        let mut cov = 0.0f64;
        let (mut vd, mut vs) = (0.0f64, 0.0f64);
        for x in &d {
            let (a, b) = ((x.diameter - md) as f64, (x.speed - ms) as f64);
            cov += a * b;
            vd += a * a;
            vs += b * b;
        }
        let r = cov / (vd.sqrt() * vs.sqrt());
        assert!(
            r < -0.9,
            "diameter and speed correlate at r = {r:.4}, but the percolation model requires a \
             strong inverse relation (r < -0.9) — small droplets leave fast, large ones leave slow. \
             A spray without it reads as confetti."
        );
    }

    /// Any subset of a spray is the same blood as the whole spray. This is what makes a caller's
    /// budget cut safe, and it is a property of index-seeding rather than of the numbers.
    #[test]
    fn a_droplet_does_not_depend_on_the_ones_before_it() {
        let w = fixed_wound();
        let s = BloodSettings::default();
        let all = droplets(&w, &s);
        for (i, d) in all.iter().enumerate() {
            assert_eq!(*d, droplet(&w, i as u32, &s), "droplet {i} depends on its neighbours");
        }
    }

    /// Every direction is inside the authored cone, and every one is unit length. A direction outside
    /// the cone is blood leaving the *back* of a wound.
    #[test]
    fn every_droplet_leaves_inside_the_cone() {
        let s = BloodSettings::default();
        for n in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [0.6, 0.8, 0.0]] {
            let normal = vec::normalize_or_zero(n);
            let w = Wound { normal, ..fixed_wound() };
            let cos_cone = m::cos(to_radians(s.spatter_cone_deg));
            for i in 0..512u32 {
                let d = droplet(&w, i, &s);
                assert!(
                    m::abs(vec::length(d.dir) - 1.0) < 1.0e-5,
                    "droplet {i} direction length {}",
                    vec::length(d.dir)
                );
                assert!(
                    vec::dot(d.dir, normal) >= cos_cone - 1.0e-4,
                    "droplet {i} left outside the {} deg cone",
                    s.spatter_cone_deg
                );
            }
        }
    }

    /// A wound moved by less than the weld lattice seeds identically; moved by more, it does not.
    /// That is the whole point of quantising — a float ULP must not change the blood.
    #[test]
    fn the_seed_is_quantized_to_the_weld_lattice() {
        let w = fixed_wound();
        let nudged = Wound { at: vec::add(w.at, [WELD * 0.1; 3]), ..w };
        assert_eq!(
            wound_seed(&w),
            wound_seed(&nudged),
            "a sub-lattice nudge must not move the seed"
        );

        let moved = Wound { at: vec::add(w.at, vec::scale(vec::X, WELD * 40.0)), ..w };
        assert_ne!(wound_seed(&w), wound_seed(&moved), "a real move must be a different spray");
    }

    /// A severance and a channel at the same point are different wounds and must throw different
    /// blood — which is what mixing the kind into the seed buys.
    #[test]
    fn the_kind_is_part_of_the_seed() {
        let a = fixed_wound();
        let b = Wound { kind: WoundKind::Channel, ..a };
        assert_ne!(
            wound_seed(&a),
            wound_seed(&b),
            "a cut and a bullet channel at one point must not spray identically"
        );
    }

    /// Count scales with area and severity, and is clamped — including the two ways it can be
    /// nothing.
    #[test]
    fn the_droplet_count_scales_with_area_and_clamps() {
        let s = BloodSettings::default();
        let w = fixed_wound();
        assert_eq!(droplet_count(&Wound { area: 0.0, ..w }, &s), 0, "no area, no blood");
        assert_eq!(droplet_count(&Wound { severity: 0.0, ..w }, &s), 0, "clotted, no blood");
        let small = droplet_count(&Wound { area: 0.001, ..w }, &s);
        let big = droplet_count(&Wound { area: 0.01, ..w }, &s);
        assert!(big > small, "a wider wound must throw more blood: {small} then {big}");
        assert_eq!(
            droplet_count(&Wound { area: 1.0e6, ..w }, &s),
            s.max_droplets_per_wound,
            "an enormous wound must be clamped to the authored ceiling"
        );
        assert_eq!(
            droplet_count(&Wound { severity: 0.5, ..w }, &s) * 2,
            droplet_count(&w, &s),
            "half severity is half the blood"
        );
    }

    /// The landing solver's refusals are the honest ones: it never invents a crossing.
    #[test]
    fn a_droplet_that_never_reaches_the_plane_has_no_landing() {
        let s = BloodSettings::default();
        let up = Droplet { dir: vec::Y, speed: 10.0, diameter: 0.002 };
        assert!(
            landing([0.0, 0.5, 0.0], &up, s.gravity, 0.5).is_none(),
            "a droplet starting on the plane has not landed on it"
        );
        assert!(
            landing([0.0, 0.2, 0.0], &up, s.gravity, 0.5).is_none(),
            "a droplet starting below the plane never lands on it"
        );
        assert!(
            landing([0.0, 1.0, 0.0], &up, 0.0, 0.0).is_none(),
            "with no gravity an upward droplet never comes down"
        );
        let hit = landing([0.0, 1.0, 0.0], &up, s.gravity, 0.0)
            .expect("thrown up under gravity, it lands");
        assert_eq!(hit[1], 0.0, "the landing must be exactly on the plane, not a float above it");
    }
}
