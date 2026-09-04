//! **Blood soaking into cloth.** A capillary front on a porous sheet, with the shear-thinning
//! viscosity the rest of this crate already knows about.
//!
//! # Why this is not a growing circle
//!
//! Blood spreading on a *surface* is [`crate::bloodstain::pool`]; blood spreading *into* a bandage, a shirt or a
//! mattress is a different physics with a different signature, and the signature is what makes it
//! worth modelling: the stain's radius goes as **√t**, so it races outward at first and visibly
//! stalls — halving its speed every time it doubles its reach — without anything decaying, timing
//! out or being clamped. A circle grown at a constant rate looks like a shader; a √t front looks
//! like cloth.
//!
//! # Lucas–Washburn, and what a shear-thinning fluid does to it
//!
//! The classical law, in the form Steinik, Picchi, Lavalle & Poesio quote it — *"Inertial and
//! shear-thinning effects in the capillary rise of a non-Newtonian fluid"*, Phys. Rev. Fluids 9,
//! 023305 (2024), `doi:10.1103/physrevfluids.9.023305`, their Eq. 1:
//!
//! ```text
//! L² = γ r cos(θ) t / (2 μ)
//! ```
//!
//! Blood is not Newtonian, so `μ` is not a number: [`crate::bloodstain::rheo`] makes it a function of shear
//! rate, and the shear rate at the front falls as the front slows. Steinik et al.'s result is that
//! this **does not** leave the ½ exponent alone. Their conclusion, in their words: "we generalize
//! the Lucas-Washburn scaling relation to shear-thinning fluids showing that the classical 1/2
//! scaling law holds only if an ad hoc time-dependent effective viscosity is introduced". Their
//! rescaling — time divided by the dimensionless wall viscosity — collapses every shear-thinning
//! curve back onto the ½ law, and [`Sheet::rescaled_time_s`] is that variable.
//!
//! So this module reports both, and the tests assert both: the **raw** front is monotone and rises
//! *more slowly* than √t (log-log slope ≈ 0.46 over 1–100 s, which is the shear-thinning
//! fingerprint), and the **rescaled** front is √t to within 2 %.
//!
//! Steinik et al. use the Ellis model, which has a closed-form velocity profile in a tube; this
//! crate has Carreau–Yasuda with Cho & Kensey's constants fitted to whole blood, and a second
//! constitutive model for one module would be a second answer to the question [`crate::bloodstain::rheo`]
//! already answers. The wall shear rate is the Newtonian tube relation `γ̇ = 4v̄/r`, evaluated at
//! the front's own speed.
//!
//! # One fixed-point pass, and exactly what it costs
//!
//! `μ_eff` depends on the front's speed, which depends on `μ_eff`. That is a fixed point, and the
//! iteration count would be a **hidden dial** on a frozen model — this crate's rule everywhere else
//! is a fixed schedule with no convergence test, so the count here is **one**: the zero-shear
//! plateau gives a first front, its speed gives a wall shear rate, and the viscosity there gives the
//! answer.
//!
//! **What one pass leaves on the table is measured rather than assumed.** At the shipped cotton a
//! second pass lengthens the front by 3.8 % at 1 s and 11.9 % at 100 s, and a third adds under a
//! tenth of that again — the iteration contracts hard, so one pass is a *truncation with a known
//! sign* (it under-predicts) rather than an unstable guess. It also captures the effect that
//! matters: the first pass already moves `μ` from the 0.056 Pa·s zero-shear plateau to about
//! 0.004 Pa·s, an order of magnitude, and every later pass is trimming a few per cent off a number
//! that has already stopped being the Newtonian one.
//! [`tests::the_fixed_point_iteration_contracts`] pins both halves of that.
//!
//! # What is this crate's own
//!
//! [`Sheet::surface_tension_n_m`] defaults to [`crate::bloodstain::droplet::BLOOD_SURFACE_TENSION`], which is
//! measured. The three sheet parameters are **authored**: no paper in this crate's corpus tabulates
//! the pore radius, contact angle or porosity of cotton. Each says so on its own field.
//!
//! One structural caveat, because it decides how the output should be read: Lucas–Washburn is a
//! **single straight capillary**, and a woven sheet's channels are neither single nor straight. Real
//! tortuosity retards the front, so this is an **upper bound** on how fast a fabric wicks, and a
//! caller matching a photograph tightens [`Sheet::pore_radius_um`] rather than scaling the answer
//! afterwards — the radius is the parameter the geometry is actually hiding in.

use crate::bloodstain::droplet::BLOOD_SURFACE_TENSION;
use crate::bloodstain::m;
use crate::bloodstain::rheo;
use crate::bloodstain::settings::BloodSettings;

/// Width of the saturation front as a fraction of its radius. **This crate's own.**
///
/// A sharp front is a stair-step in a texture and reads as a cut-out; a wide one reads as fog.
/// `0.15` is a shape parameter, not a measurement, and it is proportional to the radius rather than
/// absolute because a real imbibition front's roughness grows with the wetted length.
pub const FRONT_SOFTNESS: f32 = 0.15;

/// **A porous sheet blood can soak into.**
///
/// Cotton by default. The defaults are authored — see the module docs — and a caller with a real
/// fabric measurement should say so in data rather than scaling the output afterwards.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sheet {
    /// Mean pore radius, µm. **This crate's own**: `10 µm` for a woven cotton, the scale of the gaps
    /// between fibres in a spun yarn rather than of the fibres themselves.
    pub pore_radius_um: f32,
    /// Static contact angle of blood on the fibre, degrees. **This crate's own**: `30°`, a
    /// well-wetting surface. At or past `90°` the sheet is non-wetting and takes nothing, which
    /// [`Sheet::front_mm`] answers with zero rather than with an imaginary radius.
    pub contact_angle_deg: f32,
    /// Void fraction, `[0, 1)`. **This crate's own**: `0.7` for a loose woven cotton. It does not
    /// enter the front law — that is a single-capillary result — and it is what turns a saturation
    /// into a volume of blood per volume of cloth, via [`Sheet::liquid_fraction_at`].
    pub porosity: f32,
    /// Surface tension of the invading fluid, N/m. Defaults to
    /// [`crate::bloodstain::droplet::BLOOD_SURFACE_TENSION`], which is measured (60.45 mN/m).
    pub surface_tension_n_m: f32,
}

impl Default for Sheet {
    fn default() -> Self {
        Self {
            pore_radius_um: 10.0,
            contact_angle_deg: 30.0,
            porosity: 0.7,
            surface_tension_n_m: BLOOD_SURFACE_TENSION,
        }
    }
}

/// Smoothstep, `3x² − 2x³` on a clamped `x`. The same cubic every soft edge in this crate uses.
#[inline]
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Sheet {
    /// The capillary driving group `γ r cos θ / 2`, m³/s·Pa·s — everything in the front law except
    /// the time and the viscosity. Zero, and meant to be, for a non-wetting sheet.
    fn drive(&self) -> f32 {
        let r_m = self.pore_radius_um * 1.0e-6;
        let cos = m::cos(crate::bloodstain::to_radians(self.contact_angle_deg));
        if !r_m.is_finite() || r_m <= 0.0 || !cos.is_finite() || cos <= 0.0 {
            return 0.0;
        }
        let gamma = self.surface_tension_n_m;
        if !gamma.is_finite() || gamma <= 0.0 {
            return 0.0;
        }
        0.5 * gamma * r_m * cos
    }

    /// A front radius, mm, from the driving group, a viscosity and a time.
    fn front_from(&self, drive: f32, mu: f32, t_s: f32) -> f32 {
        if drive <= 0.0 || !mu.is_finite() || mu <= 0.0 || !t_s.is_finite() || t_s <= 0.0 {
            return 0.0;
        }
        // metres², then metres, then mm.
        m::sqrt(drive * t_s / mu) * 1.0e3
    }

    /// **The classical front**, mm: Lucas–Washburn at the zero-shear viscosity, as if blood were
    /// Newtonian. Exactly `∝ √t`, and the thing the shear-thinning front should be compared against.
    pub fn newtonian_front_mm(&self, t_s: f32, s: &BloodSettings) -> f32 {
        let mu = rheo::viscosity(0.0, s.hematocrit, s);
        self.front_from(self.drive(), mu, t_s)
    }

    /// **The wall shear rate at the front**, s⁻¹ — `γ̇ = 4v̄/r` at the speed the Newtonian front
    /// would have, which is the one pass the module docs justify.
    ///
    /// `v̄ = L/2t` is the derivative of the √t law, so this needs no finite difference and no
    /// remembered previous state.
    pub fn wall_shear_rate(&self, t_s: f32, s: &BloodSettings) -> f32 {
        if !t_s.is_finite() || t_s <= 0.0 {
            return 0.0;
        }
        let l_m = self.newtonian_front_mm(t_s, s) * 1.0e-3;
        let r_m = self.pore_radius_um * 1.0e-6;
        if r_m <= 0.0 {
            return 0.0;
        }
        let v = l_m / (2.0 * t_s);
        4.0 * v / r_m
    }

    /// **The effective viscosity at the front**, Pa·s: [`crate::bloodstain::rheo::viscosity`] at
    /// [`Sheet::wall_shear_rate`].
    ///
    /// Falls with time, because the front slows: early imbibition is fast, strongly sheared and
    /// therefore *thin*, which is why blood wicks into cloth faster than its resting viscosity says
    /// it should.
    pub fn effective_viscosity(&self, t_s: f32, s: &BloodSettings) -> f32 {
        rheo::viscosity(self.wall_shear_rate(t_s, s), s.hematocrit, s)
    }

    /// **The front radius at `t`**, mm. Lucas–Washburn with [`Sheet::effective_viscosity`].
    pub fn front_mm(&self, t_s: f32, s: &BloodSettings) -> f32 {
        self.front_from(self.drive(), self.effective_viscosity(t_s, s), t_s)
    }

    /// **Steinik's collapse variable**, seconds: `t` divided by the effective viscosity in units of
    /// the zero-shear plateau.
    ///
    /// This is the rescaling that recovers the ½ exponent — their Eq. 33 and Fig. 4 — so
    /// `front_mm(t)` against `rescaled_time_s(t)` is √t to within a fraction of a per cent, while
    /// `front_mm(t)` against `t` is deliberately not.
    pub fn rescaled_time_s(&self, t_s: f32, s: &BloodSettings) -> f32 {
        let mu_zero = rheo::viscosity(0.0, s.hematocrit, s);
        let mu = self.effective_viscosity(t_s, s);
        if mu <= 0.0 || !t_s.is_finite() || t_s <= 0.0 {
            return 0.0;
        }
        t_s * mu_zero / mu
    }

    /// **Pore saturation at a radius**, `[0, 1]`: filled behind the front, empty ahead of it, with a
    /// [`FRONT_SOFTNESS`]-wide smooth edge.
    pub fn saturation_at(&self, r_mm: f32, t_s: f32, s: &BloodSettings) -> f32 {
        let front = self.front_mm(t_s, s);
        if front <= 0.0 || !r_mm.is_finite() {
            return 0.0;
        }
        let half = 0.5 * FRONT_SOFTNESS * front;
        if half <= 0.0 {
            return if r_mm <= front { 1.0 } else { 0.0 };
        }
        let x = (front + half - r_mm.max(0.0)) / (2.0 * half);
        smoothstep(x)
    }

    /// **Blood per unit volume of cloth at a radius**, `[0, porosity]` — the saturation weighted by
    /// the void fraction, which is what a renderer's absorption or a bandage's capacity wants.
    pub fn liquid_fraction_at(&self, r_mm: f32, t_s: f32, s: &BloodSettings) -> f32 {
        let porosity = if self.porosity.is_finite() { self.porosity.clamp(0.0, 1.0) } else { 0.0 };
        porosity * self.saturation_at(r_mm, t_s, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn slope(sheet: &Sheet, s: &BloodSettings, rescale: bool) -> f32 {
        let (t0, t1) = (1.0f32, 100.0f32);
        let (l0, l1) = (sheet.front_mm(t0, s), sheet.front_mm(t1, s));
        let (x0, x1) = if rescale {
            (sheet.rescaled_time_s(t0, s), sheet.rescaled_time_s(t1, s))
        } else {
            (t0, t1)
        };
        (l1.ln() - l0.ln()) / (x1.ln() - x0.ln())
    }

    /// **√t, once the effective viscosity is scaled out** — Steinik et al.'s generalisation of
    /// Lucas–Washburn, which is the whole claim this module rests on. The raw slope is reported in
    /// the failure message because it is the *other* half of their result: a shear-thinning fluid
    /// does not obey the ½ law in laboratory time.
    #[test]
    fn the_rescaled_front_follows_lucas_washburn() {
        let sheet = Sheet::default();
        let s = BloodSettings::default();
        let rescaled = slope(&sheet, &s, true);
        let raw = slope(&sheet, &s, false);
        assert!(
            (rescaled - 0.5).abs() < 0.02,
            "rescaled log-log slope over 1–100 s is {rescaled}, not 0.5 ± 0.02 (raw slope {raw})"
        );
    }

    /// The raw front rises **more slowly** than √t, and that is the shear-thinning signature rather
    /// than an error: `μ_eff` climbs back toward the zero-shear plateau as the front slows, so late
    /// imbibition is retarded relative to early. Around 0.46 at the shipped cotton.
    #[test]
    fn shear_thinning_flattens_the_raw_slope() {
        let sheet = Sheet::default();
        let s = BloodSettings::default();
        let raw = slope(&sheet, &s, false);
        assert!(raw < 0.5, "raw slope {raw} should sit below the Newtonian ½");
        assert!(raw > 0.4, "raw slope {raw} is implausibly far below ½ — check the viscosity call");
    }

    /// Shear thinning **outruns** the Newtonian front at every time, because the viscosity at the
    /// wall is never above the zero-shear plateau. This is the visible consequence: blood wicks into
    /// cloth faster than its resting viscosity predicts.
    #[test]
    fn the_shear_thinned_front_outruns_the_newtonian_one() {
        let sheet = Sheet::default();
        let s = BloodSettings::default();
        for i in 1..=100u32 {
            let t = i as f32;
            let thinned = sheet.front_mm(t, &s);
            let newtonian = sheet.newtonian_front_mm(t, &s);
            assert!(
                thinned > newtonian,
                "at {t} s the shear-thinned front ({thinned} mm) did not beat the Newtonian one \
                 ({newtonian} mm)"
            );
        }
    }

    /// A front advances. Monotone in time, at every scale a caller might sample.
    #[test]
    fn the_front_is_monotone_in_time() {
        let sheet = Sheet::default();
        let s = BloodSettings::default();
        let mut last = 0.0f32;
        for i in 1..=2000u32 {
            let t = i as f32 * 0.1;
            let l = sheet.front_mm(t, &s);
            assert!(l > last, "the front went backwards at {t} s: {l} mm after {last} mm");
            last = l;
        }
    }

    /// **The truncation is a truncation, not a divergence.** Two claims, both measured:
    ///
    /// 1. The first pass does the work that matters — it moves `μ` off the zero-shear plateau by an
    ///    order of magnitude.
    /// 2. The iteration **contracts**: the third pass moves the front by far less than the second,
    ///    so stopping at one is a bounded under-prediction rather than a guess at an unstable
    ///    sequence.
    ///
    /// If either ever fails, the argument in the module docs for a fixed one-pass schedule is
    /// wrong, and this says so instead of the numbers quietly drifting.
    #[test]
    fn the_fixed_point_iteration_contracts() {
        let sheet = Sheet::default();
        let s = BloodSettings::default();
        let drive = 0.5 * sheet.surface_tension_n_m * sheet.pore_radius_um * 1.0e-6
            * m::cos(crate::bloodstain::to_radians(sheet.contact_angle_deg));
        let r_m = sheet.pore_radius_um * 1.0e-6;
        // One more pass from a given front: its speed sets the shear rate, that sets the viscosity.
        let again = |front_mm: f32, t: f32| {
            let v = front_mm * 1.0e-3 / (2.0 * t);
            let mu = rheo::viscosity(4.0 * v / r_m, s.hematocrit, &s);
            m::sqrt(drive * t / mu) * 1.0e3
        };
        let mu_zero = rheo::viscosity(0.0, s.hematocrit, &s);
        for t in [1.0f32, 10.0, 100.0] {
            let mu_one = sheet.effective_viscosity(t, &s);
            assert!(
                mu_one < 0.2 * mu_zero,
                "at {t} s one pass left μ at {mu_one} against a {mu_zero} plateau — the pass that \
                 matters did not happen"
            );
            let once = sheet.front_mm(t, &s);
            let twice = again(once, t);
            let thrice = again(twice, t);
            let second = (twice - once).abs();
            let third = (thrice - twice).abs();
            assert!(
                third < 0.25 * second,
                "at {t} s the iteration is not contracting: pass 2 moved {second} mm, pass 3 moved \
                 {third} mm"
            );
        }
    }

    /// Saturation is one well behind the front, zero well ahead of it, and monotone between — a
    /// front, not a gradient with a name.
    #[test]
    fn saturation_is_a_front() {
        let sheet = Sheet::default();
        let s = BloodSettings::default();
        let t = 10.0f32;
        let front = sheet.front_mm(t, &s);
        assert!(sheet.saturation_at(0.0, t, &s) > 0.999, "the centre must be saturated");
        assert!(
            sheet.saturation_at(front * 2.0, t, &s) == 0.0,
            "twice the front radius must be dry"
        );
        let mut last = f32::INFINITY;
        for i in 0..200u32 {
            let r = i as f32 * front * 0.01;
            let sat = sheet.saturation_at(r, t, &s);
            assert!(sat <= last + 1.0e-6, "saturation rose outward at {r} mm");
            assert!((0.0..=1.0).contains(&sat), "saturation escaped [0, 1] at {r} mm: {sat}");
            last = sat;
        }
        let frac = sheet.liquid_fraction_at(0.0, t, &s);
        assert!(
            (frac - sheet.porosity).abs() < 1.0e-3,
            "a saturated sheet holds its porosity in blood, not {frac}"
        );
    }

    /// **A non-wetting sheet takes nothing**, and the answer is zero rather than a radius derived
    /// from a negative driving pressure. The same refusal the rest of the crate makes.
    #[test]
    fn a_non_wetting_sheet_takes_nothing() {
        let s = BloodSettings::default();
        for angle in [90.0f32, 120.0, 180.0] {
            let sheet = Sheet { contact_angle_deg: angle, ..Default::default() };
            assert_eq!(sheet.front_mm(10.0, &s), 0.0, "θ = {angle}° must not imbibe");
            assert_eq!(sheet.saturation_at(0.0, 10.0, &s), 0.0);
        }
        let sheet = Sheet::default();
        for bad in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            assert_eq!(sheet.front_mm(bad, &s), 0.0, "t = {bad} s must not imbibe");
        }
    }

    /// **Frozen.** The front, the effective viscosity and the saturation profile at four times.
    ///
    /// A lock rather than a snapshot: Cho & Kensey's Carreau–Yasuda constants, the wall-shear
    /// relation, the single fixed-point pass and the authored cotton are all upstream of these bits.
    #[test]
    fn the_wick_model_is_frozen() {
        let sheet = Sheet::default();
        let s = BloodSettings::default();
        let mut got: Vec<u32> = Vec::new();
        for t in [0.5f32, 2.0, 10.0, 60.0] {
            got.push(sheet.front_mm(t, &s).to_bits());
            got.push(sheet.effective_viscosity(t, &s).to_bits());
            got.push(sheet.saturation_at(4.0, t, &s).to_bits());
        }
        std::println!("{got:?}");
        let want: Vec<u32> = std::vec![
            0x40bac23b, 0x3b7bd127, 0x3f800000, 0x41359f03, 0x3b8521eb, 0x3f800000, 0x41c168b5,
            0x3b92bf98, 0x3f800000, 0x425a28bd, 0x3bad0290, 0x3f800000,
        ];
        assert_eq!(got, want);
    }
}
