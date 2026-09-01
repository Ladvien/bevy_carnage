//! **The spatter model.** A wound, and the blood that leaves it.
//!
//! Pure functions, integer-seeded, frozen by a golden. Nothing here spawns anything, reads a clock or
//! touches the ECS: a caller passes a [`Wound`] and gets values back.
//!
//! # The physics this is a reduction of
//!
//! Comiskey, Yarin & Attinger, *"Theoretical and experimental investigation of forward spatter of
//! blood from a gunshot"*, Phys. Rev. Fluids **3**, 063901 (2018), `doi:10.1103/physrevfluids.3.063901`.
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
//! [`wound_seed`] is a hash of **where the wound is**, quantized on the crate's own weld lattice,
//! exactly as `bore::prism` derives its raggedness. Nothing is threaded down and nothing accumulates,
//! so any droplet of any wound can be recomputed alone, in any order, on any machine, and
//! [`droplets`] is only a convenience over [`droplet`]. [`crate::soup::hash_f32`] is the only source
//! of randomness in this module, as it is in the whole crate.

use bevy::math::Vec3;
use std::f32::consts::TAU;

use crate::CarnageSettings;
use crate::soup::{WELD, hash_f32, plane_basis};
use crate::wound::Wound;

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
    pub dir: Vec3,
    /// Initial speed, m/s.
    pub speed: f32,
    /// Diameter, metres. Inversely correlated with [`speed`](Self::speed) — that is the model.
    pub diameter: f32,
}

/// A stain the caller may stamp: where a droplet landed and how wide it reads.
///
/// **Core, not a visual.** Where blood lands is read by simulation on the consuming side — a blood
/// pool is a chemoattractant source there — so stain *placement* must exist headless and be
/// deterministic. Turning one into an entity is the cosmetic half and lives behind the `vfx` feature.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Stain {
    /// Where it landed, subject-local, with `y` exactly on the plane it landed on.
    pub at: Vec3,
    /// How wide it reads, metres.
    pub radius: f32,
    /// A per-stain seed, for a caller choosing between splat variants without adding randomness.
    pub seed: u32,
}

/// **Seed for one wound — a pure function of WHERE it is, never of history.**
///
/// The quantization is `bore::prism`'s verbatim: positions are snapped to the crate's weld lattice
/// before hashing, so two runs that place the wound a float ULP apart still seed identically, and a
/// wound a tenth of a millimetre away is a different spray.
///
/// **[`WoundKind`](crate::WoundKind) is mixed in**, so a severance and a channel that happen to open
/// at the same point do not throw the same blood. That is why the enum's discriminants are written
/// out: reordering them would silently move every seed.
///
/// Deliberately *not* seeded from an accumulator, an `Entity`, an `AssetId` or a clock. Each of those
/// has its own recorded failure in this crate or its consumer — an arena slot is assigned by load
/// order, a drain counter desynchronises permanently after any single difference.
pub fn wound_seed(w: &Wound) -> u32 {
    let q = |x: f32| (x / WELD).round() as i64 as u32;
    q(w.at.x)
        ^ q(w.at.y).wrapping_mul(0x9E37_79B9)
        ^ q(w.at.z).wrapping_mul(2_654_435_761)
        ^ (w.kind as u32).wrapping_mul(0x85EB_CA6B)
}

/// How many droplets a wound throws: area × density × severity, clamped.
///
/// **Area, not per hit** — a wound is a surface, and how much blood leaves it is a property of how
/// much of it is open. The clamp is what keeps one enormous cut inside a particle effect's fixed
/// capacity; a severity of zero throws nothing, which is what a fully clotted wound is.
pub fn droplet_count(w: &Wound, s: &CarnageSettings) -> u32 {
    if !(w.area > 0.0) || !(w.severity > 0.0) || !w.area.is_finite() {
        return 0;
    }
    let n = w.area * s.droplets_per_m2 * w.severity.clamp(0.0, 1.0);
    if !n.is_finite() {
        return 0;
    }
    (n.round().max(0.0) as u32).min(s.max_droplets_per_wound)
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
/// The cone is built on [`crate::soup::plane_basis`], the same basis every other direction in this
/// crate is derived against, so a spray and a cut face agree about what "sideways" means.
pub fn droplet(w: &Wound, index: u32, s: &CarnageSettings) -> Droplet {
    Spray::of(w, s).droplet(index, s)
}

/// **Everything about a wound's spray that does not depend on which droplet you ask for.**
///
/// Built once and reused, because every field below used to be recomputed per droplet ordinal, and the
/// two callers that want a whole spray ([`droplets`] and [`stains`]) ask for hundreds each. Per droplet
/// that was: one normalisation of the wound normal, one [`plane_basis`] — itself two cross products and
/// a second normalisation — one `to_radians`, and one [`wound_seed`]. Two square roots and a hash,
/// repeated for a value that is identical every time. Both callers run over the same ordinals, so the
/// benchmark's 285 000 droplets cost roughly 570 000 redundant square-root pairs.
///
/// **Bit-identical, which is the only reason this is a hoist rather than a change.** Same inputs, same
/// operations, same order; only the number of times they run differs. [`droplet`] still builds one and
/// throws it away, so the single-shot public path computes exactly what it always did.
#[derive(Clone, Copy)]
struct Spray {
    /// Unit spray axis, or zero — see the note in [`Spray::of`].
    axis: Vec3,
    tangent: Vec3,
    bitangent: Vec3,
    /// Cone half-angle, radians.
    theta_max: f32,
    /// This wound's seed, mixed with the droplet ordinal to key each draw.
    seed: u32,
}

impl Spray {
    fn of(w: &Wound, s: &CarnageSettings) -> Self {
        // A wound with no normal has no direction to spray along; `plane_basis` would hand back a
        // degenerate frame. Spraying straight up is a fabricated answer, so the honest one is the axis
        // itself, which for a zero normal is zero and throws blood nowhere.
        let axis = w.normal.normalize_or_zero();
        let (tangent, bitangent) = plane_basis(axis);
        Self {
            axis,
            tangent,
            bitangent,
            theta_max: s.spatter_cone_deg.to_radians(),
            seed: wound_seed(w),
        }
    }

    fn droplet(&self, index: u32, s: &CarnageSettings) -> Droplet {
        let key = self.seed ^ index.wrapping_mul(0x9E37_79B9);
        let t = hash_f32(key);
        let u = hash_f32(key ^ 0x85EB_CA6B);
        let v = hash_f32(key ^ 0xC2B2_AE35);

        let diameter = s.droplet_size_min + (s.droplet_size_max - s.droplet_size_min) * t;
        // The inversion. Largest droplet, slowest speed.
        let speed = (FORWARD_SPATTER_SPEED + (BACK_SPATTER_SPEED - FORWARD_SPATTER_SPEED) * t)
            * s.spatter_speed_scale;

        let phi = TAU * u;
        let theta = self.theta_max * v.clamp(0.0, 1.0).sqrt();
        let dir = (self.axis * theta.cos()
            + (self.tangent * phi.cos() + self.bitangent * phi.sin()) * theta.sin())
        .normalize_or_zero();

        Droplet { dir, speed, diameter }
    }
}

/// The whole spray, in droplet-index order.
pub fn droplets(w: &Wound, s: &CarnageSettings) -> Vec<Droplet> {
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
/// a timestep, and a timestep is a clock — which the determinism contract forbids on this side of the
/// crate. The landing `y` is *assigned* `plane_y` rather than computed, so a stain is exactly on the
/// plane it stained instead of a float's width above or below it.
///
/// `None` for a droplet that starts at or under the plane, or whose discriminant is negative: both
/// mean it never crosses, and inventing a landing point for it would be a fabricated result.
pub fn landing(from: Vec3, d: &Droplet, gravity: f32, plane_y: f32) -> Option<Vec3> {
    let h = from.y - plane_y;
    if !(h > 0.0) || !h.is_finite() {
        return None;
    }
    let vy = d.dir.y * d.speed;
    let t = if gravity.abs() <= f32::EPSILON {
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
        (vy + disc.sqrt()) / gravity
    };
    if !(t > 0.0) || !t.is_finite() {
        return None;
    }
    let mut at = from + d.dir * d.speed * t;
    at.y = plane_y;
    Some(at)
}

/// Stain radius from droplet diameter and impact speed.
///
/// A spreading droplet's splat is wider than the droplet, growing with impact speed — the spread
/// factor. This is the game-scale form of it: the diameter's own position in the settings' size span
/// sets the base, impact speed widens it across the measured speed span, and the result is clamped
/// into the authored radius range so a stain is never smaller than a pixel or wider than a puddle.
///
/// The correlation the literature states is valid for viscosity 1–300 mPa·s, which blood sits inside,
/// so the shape is applicable rather than borrowed.
pub fn stain_radius(d: &Droplet, impact_speed: f32, s: &CarnageSettings) -> f32 {
    let span = (s.droplet_size_max - s.droplet_size_min).max(f32::MIN_POSITIVE);
    let size_frac = ((d.diameter - s.droplet_size_min) / span).clamp(0.0, 1.0);
    let speed_span = (FORWARD_SPATTER_SPEED - BACK_SPATTER_SPEED).max(f32::MIN_POSITIVE);
    let speed_frac = ((impact_speed - BACK_SPATTER_SPEED) / speed_span).clamp(0.0, 1.0);
    // Size dominates and speed widens: a big slow droplet still makes the bigger mark, which is what
    // the spread-factor correlation says and what a photograph of one shows.
    let frac = (0.7 * size_frac + 0.3 * speed_frac).clamp(0.0, 1.0);
    s.stain_radius_min + (s.stain_radius_max - s.stain_radius_min) * frac
}

/// The stains one wound leaves on a horizontal plane, in droplet-index order.
///
/// Droplets that never reach the plane are skipped rather than pinned to it. The order is the droplet
/// ordinal, which is total by construction, so this needs no sort — and a caller folding a hash over
/// the result gets the same digest every run.
pub fn stains(w: &Wound, s: &CarnageSettings, plane_y: f32) -> Vec<Stain> {
    let spray = Spray::of(w, s);
    // Invariant across the droplet ordinal, and `fall` was being recomputed inside the closure.
    let fall = (w.at.y - plane_y).max(0.0);
    let n = droplet_count(w, s);
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let d = spray.droplet(i, s);
        let Some(at) = landing(w.at, &d, s.gravity, plane_y) else {
            continue;
        };
        // Impact speed from the same closed form the landing came from: vertical speed gained
        // over the drop, horizontal speed unchanged, because the drag dial is a look control on
        // the particles rather than a second integrator here.
        let vy = d.dir.y * d.speed;
        let impact = (vy * vy + 2.0 * s.gravity * fall).max(0.0).sqrt();
        let horizontal = (d.dir * d.speed - Vec3::Y * vy).length();
        let impact_speed = (impact * impact + horizontal * horizontal).sqrt();
        out.push(Stain {
            at,
            radius: stain_radius(&d, impact_speed, s),
            seed: spray.seed ^ i,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wound::WoundKind;

    /// The wound the golden is taken against. A named constant so the golden and every property test
    /// are measuring the same geometry.
    fn fixed_wound() -> Wound {
        Wound {
            at: Vec3::new(0.1, 0.9, -0.2),
            normal: Vec3::X,
            area: 0.004,
            severity: 1.0,
            kind: WoundKind::Severance,
        }
    }

    /// **These bits are the API.**
    ///
    /// The same lock `hash_f32_is_frozen` puts on the fracture, for the same reason: a caller's
    /// replay, a recorded demo and a golden digest downstream are all defined against these exact
    /// values. A change to the draw order, the key rotations, the lerp direction or the cone
    /// construction moves every one of them.
    ///
    /// **This is not a snapshot to re-bless.** If it fails, the model moved, and the question is
    /// whether that was intended — not whether the numbers should be updated to match.
    ///
    /// # The one exception, and what it cost to establish
    ///
    /// **Re-blessed once, 2026-09-01, for a change of reference profile rather than a change of
    /// model.** These values were blessed at **opt-level 0**, which is what a bare `cargo test` in this
    /// crate's standalone repository compiles at. Inside `foundation_vs_slop` — now the crate's source
    /// of truth — `[profile.dev] opt-level = 1` applies, and three of the forty values below differ by
    /// **exactly one ULP** (rows 2 and 5). The table now holds the opt-level-1 values, because that is
    /// the profile the shipping build and the workspace gate actually use.
    ///
    /// Diagnosed rather than assumed. Standalone on the same machine, same architecture, both frozen
    /// tests pass; the *only* variable that reproduces the failure is
    /// `CARGO_PROFILE_TEST_OPT_LEVEL=1`. So it is not architecture, and it is not this crate's
    /// arithmetic changing. Mechanism still unconfirmed: `-Cllvm-args=--fp-contract=off` did **not**
    /// restore the old values, so it is not simple FMA contraction — the next suspect is
    /// constant-folding of transcendentals at higher opt levels through LLVM's own implementation
    /// rather than the platform libm.
    ///
    /// **What this means for the promise, stated plainly:** the crate guarantees bit-identity for two
    /// runs of *the same build* (`docs/research-brief.md`), and that is unaffected. A table of literal
    /// bits is a fact about a build configuration, not about the source, and pretending otherwise is
    /// what made this fail. It also falsifies the claim at the consumer's `Cargo.toml:242` that its
    /// release profile's flags "do NOT alter IEEE-754 results".
    ///
    /// So the rule for a future failure is unchanged in substance and sharper in form: **if these move
    /// while the profile is held fixed, the model moved.** Re-bless only for a profile change, and say
    /// which profile.
    #[test]
    fn the_spatter_model_is_frozen() {
        let w = fixed_wound();
        let s = CarnageSettings::default();

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

        let mut actual = Vec::new();
        for i in 0..8u32 {
            let d = droplet(&w, i, &s);
            actual.push((
                [d.dir.x.to_bits(), d.dir.y.to_bits(), d.dir.z.to_bits()],
                d.speed.to_bits(),
                d.diameter.to_bits(),
            ));
        }
        let rendered: Vec<String> = actual
            .iter()
            .map(|(dir, sp, di)| {
                format!(
                    "([0x{:08X}, 0x{:08X}, 0x{:08X}], 0x{sp:08X}, 0x{di:08X}),",
                    dir[0], dir[1], dir[2]
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

    /// The stains the same wound leaves, frozen alongside the droplets — because a caller's digest is
    /// taken over stain positions, not over droplet directions, so this is the value that actually
    /// travels downstream.
    ///
    /// **Re-blessed with [`the_spatter_model_is_frozen`], for the same reason and on the same terms** —
    /// a change of reference profile from opt-level 0 to opt-level 1, not a change of model. Two of
    /// these sixteen values moved by one ULP. Read that test's doc comment for the diagnosis and for
    /// the rule that still binds: if these move while the profile is held fixed, the model moved.
    #[test]
    fn the_stain_placement_is_frozen() {
        let w = fixed_wound();
        let s = CarnageSettings::default();
        let stains = stains(&w, &s, 0.0);
        assert_eq!(stains.len(), 10, "area x density must give this many droplets");

        let expect: [([u32; 3], u32); 4] = [
            ([0x40879314, 0x00000000, 0x3FDEEF19], 0x3D7E1738),
            ([0x4169C84C, 0x00000000, 0xC0ABEC1B], 0x3D9EE6FE),
            ([0x410B1577, 0x00000000, 0x3FBEFBA5], 0x3DAA9FCC),
            ([0x413B5528, 0x00000000, 0xBFF37568], 0x3D641D9F),
        ];
        let actual: Vec<([u32; 3], u32)> = stains
            .iter()
            .take(4)
            .map(|st| {
                ([st.at.x.to_bits(), st.at.y.to_bits(), st.at.z.to_bits()], st.radius.to_bits())
            })
            .collect();
        let rendered: Vec<String> = actual
            .iter()
            .map(|(at, r)| {
                format!("([0x{:08X}, 0x{:08X}, 0x{:08X}], 0x{r:08X}),", at[0], at[1], at[2])
            })
            .collect();
        assert_eq!(
            actual.as_slice(),
            expect.as_slice(),
            "stain placement moved. If that was deliberate, the new bits are:\n{}",
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
        let s = CarnageSettings::default();
        let n = 256usize;
        let d: Vec<Droplet> = (0..n as u32).map(|i| droplet(&w, i, &s)).collect();

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
        let s = CarnageSettings::default();
        let all = droplets(&w, &s);
        for (i, d) in all.iter().enumerate() {
            assert_eq!(*d, droplet(&w, i as u32, &s), "droplet {i} depends on its neighbours");
        }
    }

    /// Every direction is inside the authored cone, and every one is unit length. A direction outside
    /// the cone is blood leaving the *back* of a wound.
    #[test]
    fn every_droplet_leaves_inside_the_cone() {
        let s = CarnageSettings::default();
        for (nx, ny, nz) in [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, -1.0), (0.6, 0.8, 0.0)] {
            let normal = Vec3::new(nx, ny, nz).normalize();
            let w = Wound { normal, ..fixed_wound() };
            let cos_cone = s.spatter_cone_deg.to_radians().cos();
            for i in 0..512u32 {
                let d = droplet(&w, i, &s);
                assert!(
                    (d.dir.length() - 1.0).abs() < 1.0e-5,
                    "droplet {i} direction length {}",
                    d.dir.length()
                );
                assert!(
                    d.dir.dot(normal) >= cos_cone - 1.0e-4,
                    "droplet {i} left at {:.4} rad from the normal, outside the {} deg cone",
                    d.dir.dot(normal).acos(),
                    s.spatter_cone_deg
                );
            }
        }
    }

    /// A wound moved by less than the weld lattice seeds identically; moved by more, it does not.
    /// That is the whole point of quantizing — a float ULP must not change the blood.
    #[test]
    fn the_seed_is_quantized_to_the_weld_lattice() {
        let w = fixed_wound();
        let nudged = Wound { at: w.at + Vec3::splat(WELD * 0.1), ..w };
        assert_eq!(wound_seed(&w), wound_seed(&nudged), "a sub-lattice nudge must not move the seed");

        let moved = Wound { at: w.at + Vec3::X * WELD * 40.0, ..w };
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

    /// Count scales with area and severity, and is clamped — including the two ways it can be nothing.
    #[test]
    fn the_droplet_count_scales_with_area_and_clamps() {
        let s = CarnageSettings::default();
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
        let s = CarnageSettings::default();
        let up = Droplet { dir: Vec3::Y, speed: 10.0, diameter: 0.002 };
        assert!(
            landing(Vec3::new(0.0, 0.5, 0.0), &up, s.gravity, 0.5).is_none(),
            "a droplet starting on the plane has not landed on it"
        );
        assert!(
            landing(Vec3::new(0.0, 0.2, 0.0), &up, s.gravity, 0.5).is_none(),
            "a droplet starting below the plane never lands on it"
        );
        assert!(
            landing(Vec3::new(0.0, 1.0, 0.0), &up, 0.0, 0.0).is_none(),
            "with no gravity an upward droplet never comes down"
        );
        let hit = landing(Vec3::new(0.0, 1.0, 0.0), &up, s.gravity, 0.0)
            .expect("thrown up under gravity, it lands");
        assert_eq!(hit.y, 0.0, "the landing must be exactly on the plane, not a float above it");
    }

    /// A landing is downrange along the droplet's own direction, not under the wound. A spray that
    /// stained the floor beneath the body would be the bug this asserts against.
    #[test]
    fn blood_lands_downrange_of_the_wound() {
        let s = CarnageSettings::default();
        let w = fixed_wound();
        let stains = stains(&w, &s, 0.0);
        assert!(!stains.is_empty(), "a severity-1 wound of this area must stain the floor");
        for st in &stains {
            assert!(
                st.at.x > w.at.x,
                "a wound facing +X stained at x = {} which is not downrange of {}",
                st.at.x,
                w.at.x
            );
            assert!(
                (s.stain_radius_min..=s.stain_radius_max).contains(&st.radius),
                "stain radius {} is outside the authored range",
                st.radius
            );
        }
    }

    /// A bigger droplet leaves a bigger mark at the same impact speed — the spread factor's own
    /// direction, and the one thing about `stain_radius` a caller can see.
    #[test]
    fn a_bigger_droplet_stains_wider() {
        let s = CarnageSettings::default();
        let small = Droplet { dir: Vec3::X, speed: 30.0, diameter: s.droplet_size_min };
        let large = Droplet { dir: Vec3::X, speed: 30.0, diameter: s.droplet_size_max };
        assert!(
            stain_radius(&large, 30.0, &s) > stain_radius(&small, 30.0, &s),
            "the larger droplet must leave the wider stain"
        );
        let slow = Droplet { dir: Vec3::X, speed: 8.0, diameter: 0.003 };
        assert!(
            stain_radius(&slow, FORWARD_SPATTER_SPEED, &s) > stain_radius(&slow, BACK_SPATTER_SPEED, &s),
            "a faster impact must spread wider at the same size"
        );
    }
}
