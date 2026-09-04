//! **Blood as a material.** Shear-thinning viscosity and a yield stress that grows into a clot.
//!
//! # Why a constitutive model at all
//!
//! Everything a game usually does with blood — a decal, a particle, a timer that stops the particle —
//! treats it as a texture with a lifetime. Blood is a **shear-thinning yield-stress fluid**, and the
//! two facts a player can actually see follow from that and from nothing else: a fast rivulet is thin
//! and races, a slow one thickens and beads; and a flow **stops where it is** when its yield stress
//! overtakes the stress driving it. The second is what clotting is, and modelling it as a material
//! property rather than as a boolean beside one is what makes a clot happen in the right place.
//!
//! # Carreau–Yasuda, with Cho & Kensey's constants
//!
//! Cho & Kensey (1991), *"Effects of the non-Newtonian viscosity of blood on flows in a diseased
//! arterial vessel. Part 1: Steady flows"*, fit the four-parameter Carreau–Yasuda model to whole human
//! blood at a hematocrit of about 45 %:
//!
//! > `μ(γ̇) = μ∞ + (μ₀ − μ∞) · [1 + (λ γ̇)^a]^((n − 1) / a)`
//!
//! and those are [`MU_ZERO`], [`MU_INF`], [`CY_LAMBDA`], [`CY_N`] and [`CY_A`] below. The Casson yield
//! stress of whole blood is a separate measurement, [`CASSON_YIELD_PA`], and it is the *fresh* end of
//! the ramp [`yield_stress`] walks.

use crate::bloodstain::settings::BloodSettings;
use crate::bloodstain::m;

/// Zero-shear viscosity of whole blood, Pa·s (Cho & Kensey 1991).
pub const MU_ZERO: f32 = 0.056;
/// Infinite-shear viscosity of whole blood, Pa·s (ibid.). About four times water's.
pub const MU_INF: f32 = 0.003_45;
/// Carreau–Yasuda relaxation time λ, seconds (ibid.).
pub const CY_LAMBDA: f32 = 3.313;
/// Carreau–Yasuda power-law index `n` (ibid.). Below 1, which *is* shear thinning.
pub const CY_N: f32 = 0.3568;
/// Carreau–Yasuda transition exponent `a` (ibid.).
pub const CY_A: f32 = 2.0;
/// Casson yield stress of fresh whole human blood, Pa — 13.8 mPa.
///
/// Small, and that is the point: fresh blood yields to almost anything, so a fresh wound flows. The
/// interesting number is how far this climbs as it clots, which is [`BloodSettings::clot_yield_pa`].
pub const CASSON_YIELD_PA: f32 = 0.0138;
/// The hematocrit Carreau–Yasuda's constants above were fitted at.
pub const HCT_REF: f32 = 0.45;

/// **Driving shear stress at a fully perfused wound, Pa.**
///
/// **TUNED, not measured.** Mean arterial pressure is a measured 12–13 kPa, but the quantity that
/// decides whether blood leaves a wound is the shear stress at the wound surface, which depends on an
/// aperture geometry this crate does not model. So this is a scale, chosen for one property that is
/// visible and checkable: it is nearly three orders above [`CASSON_YIELD_PA`], so a fresh wound
/// flows, and twice the shipped [`BloodSettings::clot_yield_pa`], so the crossing where a clot
/// arrests the flow lands **inside** the taper rather than at either end of it.
pub const PERFUSION_STRESS_PA: f32 = 10.0;

/// **Apparent viscosity at a shear rate, Pa·s.** Carreau–Yasuda, scaled for hematocrit.
///
/// At `γ̇ → 0` this is [`MU_ZERO`] and at `γ̇ → ∞` it is [`MU_INF`], with the knee near `1/λ ≈ 0.3` 1/s.
/// A negative or non-finite shear rate is treated as zero shear: it has no physical meaning, and the
/// zero-shear limit is the honest answer rather than a NaN propagating into a renderer.
///
/// # The hematocrit scale is a dial, not a law
///
/// Cho & Kensey's parameters were fitted at Hct ≈ 45 %, and hematocrit-dependent Carreau–Yasuda
/// variants exist in the literature but are **not adopted here** — they would need their own fit and
/// their own citation, and pretending one dial is that fit would be a measurement dressed as a guess.
/// So the scale is `((1 − Hct_ref) / (1 − Hct))^k` with `k` = [`BloodSettings::hct_exponent`], which
/// has the right shape (viscosity diverges as the cells crowd out the plasma) and says in its own doc
/// comment that the shape is all it has.
pub fn viscosity(shear_rate: f32, hematocrit: f32, s: &BloodSettings) -> f32 {
    let g = if shear_rate.is_finite() && shear_rate > 0.0 { shear_rate } else { 0.0 };
    let inner = 1.0 + m::powf(CY_LAMBDA * g, CY_A);
    let mu = MU_INF + (MU_ZERO - MU_INF) * m::powf(inner, (CY_N - 1.0) / CY_A);
    mu * hct_scale(hematocrit, s.hct_exponent)
}

/// The hematocrit multiplier on [`viscosity`]. **A tuning dial — see that function's doc comment.**
///
/// Clamped just below 1, because a hematocrit of exactly 1 is blood with no plasma and would divide by
/// zero. [`BloodSettings::validate`] refuses such a settings block at the door; this clamp is what
/// keeps a hand-built call from panicking anyway.
pub fn hct_scale(hematocrit: f32, exponent: f32) -> f32 {
    let hct = if hematocrit.is_finite() { hematocrit.clamp(0.0, 0.99) } else { HCT_REF };
    m::powf((1.0 - HCT_REF) / (1.0 - hct), exponent)
}

/// **Yield stress at a wound's age, Pa.** [`CASSON_YIELD_PA`] fresh, climbing to
/// [`BloodSettings::clot_yield_pa`] at [`BloodSettings::clot_ticks`], and staying there.
///
/// **Monotone, and exact at both ends.** The same integer-tick ramp shape [`crate::bloodstain::bleed::flow`]
/// uses, mirrored: flat while the wound is spurting, then linear across the taper. Monotone matters
/// for the same reason it matters in a tear or a clot anywhere else — a yield stress that could fall
/// again is a clot that can un-form, and nothing downstream would be able to tell that from a bug.
///
/// `hz` is accepted for symmetry with the rest of the crate and is deliberately unused: the ramp is
/// authored in ticks, so making it depend on the rate as well would be two dials for one thing.
pub fn yield_stress(age_ticks: u32, hz: u32, s: &BloodSettings) -> f32 {
    let _ = hz;
    let span = s.clot_yield_pa - CASSON_YIELD_PA;
    if age_ticks >= s.clot_ticks {
        return s.clot_yield_pa;
    }
    if age_ticks < s.spurt_ticks {
        return CASSON_YIELD_PA;
    }
    let taper = s.clot_ticks.saturating_sub(s.spurt_ticks);
    if taper == 0 {
        return s.clot_yield_pa;
    }
    let through = (age_ticks - s.spurt_ticks) as f32 / taper as f32;
    CASSON_YIELD_PA + span * through.clamp(0.0, 1.0)
}

/// **Does it flow?** The Casson criterion: a yield-stress fluid moves only while the driving stress
/// exceeds its yield stress.
///
/// **This replaced a separate `clotted` boolean, and the replacement is the point.** A clot used to be
/// "the age is past a number"; it is now the same material comparison that decides whether any flow
/// moves at all, which means a clot, a rivulet arresting on a wall, and a pool that has stopped
/// creeping are one mechanism at three ages instead of three special cases.
///
/// Strictly greater, so a driving stress exactly at the yield stress does **not** flow — the
/// convention that makes "arrested" the stable state rather than one that chatters on a float
/// comparison.
pub fn flows(driving_stress: f32, yield_stress: f32) -> bool {
    driving_stress.is_finite() && yield_stress.is_finite() && driving_stress > yield_stress
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shear thinning, in the direction the model exists to have: faster shear, thinner blood, with
    /// the measured limits at both ends.
    #[test]
    fn blood_thins_as_it_shears() {
        let s = BloodSettings::default();
        let hct = HCT_REF;
        let at_rest = viscosity(0.0, hct, &s);
        let walking = viscosity(10.0, hct, &s);
        let arterial = viscosity(1000.0, hct, &s);
        assert!(
            at_rest > walking && walking > arterial,
            "viscosity must fall monotonically with shear rate: {at_rest} then {walking} then \
             {arterial}"
        );
        assert!(
            (at_rest - MU_ZERO).abs() < 1.0e-6,
            "at zero shear the model must be exactly the measured zero-shear viscosity, got \
             {at_rest}"
        );
        assert!(
            arterial < MU_ZERO * 0.25 && arterial > MU_INF,
            "at 1000 1/s blood must be near its infinite-shear limit, got {arterial}"
        );
    }

    /// A negative or non-finite shear rate has no meaning, and the zero-shear limit is the honest
    /// answer — a NaN reaching a renderer is the failure this refuses.
    #[test]
    fn a_nonsense_shear_rate_is_treated_as_rest() {
        let s = BloodSettings::default();
        for bad in [-5.0f32, f32::NAN, f32::INFINITY] {
            let mu = viscosity(bad, HCT_REF, &s);
            assert!(mu.is_finite(), "viscosity({bad}) was not finite");
            assert!((mu - MU_ZERO).abs() < 1.0e-6, "viscosity({bad}) = {mu}, expected rest");
        }
    }

    /// Crowding the cells thickens the blood, and the reference hematocrit is exactly the fitted
    /// value — which is what makes the scale a scale rather than an offset.
    #[test]
    fn hematocrit_thickens_and_is_neutral_at_the_fitted_value() {
        let s = BloodSettings::default();
        assert!(
            (hct_scale(HCT_REF, s.hct_exponent) - 1.0).abs() < 1.0e-6,
            "the scale must be exactly 1 at the hematocrit the fit was taken at"
        );
        let thin = viscosity(10.0, 0.30, &s);
        let normal = viscosity(10.0, 0.45, &s);
        let thick = viscosity(10.0, 0.60, &s);
        assert!(
            thin < normal && normal < thick,
            "viscosity must rise with hematocrit: {thin} then {normal} then {thick}"
        );
        assert!(hct_scale(1.0, s.hct_exponent).is_finite(), "Hct = 1 must not divide by zero");
    }

    /// The yield ramp is monotone and exact at both ends. **Monotone is the contract**: a clot that
    /// could un-form is indistinguishable from a bug.
    #[test]
    fn the_yield_stress_climbs_monotonically_to_the_clot() {
        let s = BloodSettings::default();
        assert_eq!(yield_stress(0, 60, &s), CASSON_YIELD_PA, "fresh blood is at the Casson yield");
        assert_eq!(
            yield_stress(s.clot_ticks, 60, &s),
            s.clot_yield_pa,
            "at the clot tick the yield stress must be exactly the authored clot value"
        );
        let mut last = 0.0f32;
        for age in 0..=(s.clot_ticks + 120) {
            let y = yield_stress(age, 60, &s);
            assert!(y >= last, "yield stress fell at age {age}: {last} then {y}");
            last = y;
        }
        assert_eq!(
            yield_stress(s.clot_ticks * 4, 60, &s),
            s.clot_yield_pa,
            "past the clot it stays clotted"
        );
    }

    /// The criterion arrests **inside** the taper — the property `PERFUSION_STRESS_PA` was chosen
    /// for, and the visible consequence: a rivulet stops mid-wall instead of fading out.
    #[test]
    fn flow_arrests_inside_the_taper() {
        let s = BloodSettings::default();
        let driving = |age: u32| PERFUSION_STRESS_PA * crate::bloodstain::bleed::envelope(age, &s);
        assert!(flows(driving(0), yield_stress(0, 60, &s)), "a fresh wound must flow");
        assert!(
            !flows(driving(s.clot_ticks), yield_stress(s.clot_ticks, 60, &s)),
            "a clotted wound must not flow"
        );
        let arrest = (0..=s.clot_ticks)
            .find(|&age| !flows(driving(age), yield_stress(age, 60, &s)))
            .expect("the flow must arrest at some age before the clot tick");
        assert!(
            arrest > s.spurt_ticks && arrest < s.clot_ticks,
            "the arrest must land inside the taper ({}..{}), got {arrest}",
            s.spurt_ticks,
            s.clot_ticks
        );
    }

    /// Once arrested, always arrested — because the driving stress only falls and the yield stress
    /// only rises, so the comparison cannot re-cross.
    #[test]
    fn an_arrested_flow_never_resumes() {
        let s = BloodSettings::default();
        let mut arrested = false;
        for age in 0..=(s.clot_ticks + 600) {
            let f = flows(PERFUSION_STRESS_PA * crate::bloodstain::bleed::envelope(age, &s), yield_stress(age, 60, &s));
            if arrested {
                assert!(!f, "flow resumed at age {age} after arresting");
            }
            arrested |= !f;
        }
        assert!(arrested, "the flow must arrest somewhere in this window");
    }

    /// A non-finite stress on either side is not a flow. Blood that flowed on a NaN would be a
    /// wound that never stops bleeding, on a machine nobody can reproduce.
    #[test]
    fn a_nonsense_stress_does_not_flow() {
        assert!(!flows(f32::NAN, 1.0));
        assert!(!flows(1.0, f32::NAN));
        assert!(!flows(1.0, 1.0), "equal stresses must not flow — arrested is the stable state");
    }
}
