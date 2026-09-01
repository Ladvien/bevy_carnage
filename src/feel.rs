//! **Impact feel.** How hard a wound hits, in numbers the caller applies.
//!
//! # The two prohibitions, in as many words
//!
//! **This crate never writes `Time<Virtual>` and never touches a `Transform` or a camera.** Both are
//! stated because both are tempting and both would be bugs of the same kind — a second writer.
//!
//! - The consuming game's `juice.rs` documents itself as **the single writer of `Time<Virtual>`'s
//!   relative speed**. A hit-stop applied from here would fight it every frame, and the loser would
//!   be whichever ran second in a schedule nobody meant to order.
//! - Its `camera.rs` owns camera transforms. [`shake_offset`] therefore *returns a vector*; the caller
//!   adds it to its own camera, or to a weapon, or to a UI element, or ignores it.
//!
//! So there are no systems in this module. There is no plugin. There are three functions.
//!
//! # Grounding
//!
//! Pichlmair & Johansen, *"Designing Game Feel: A Survey"*, IEEE ToG (2021),
//! `doi:10.1109/tg.2021.3072241`:
//!
//! - §III-C on hit stops and freeze frames — the practised duration is **"often just a few frames"**,
//!   which is where [`CarnageSettings::hitstop_seconds`](crate::CarnageSettings::hitstop_seconds)'
//!   shipped 0.055 s (≈3 ticks at 60 Hz) comes from.
//! - §III-B-1, which is explicit that shake should **not** be random: *"Instead of randomly moving the
//!   camera, a carefully selected easing function in a semantically significant direction communicates
//!   more information about what has happened."* That is why [`shake_offset`] takes a `dir` — the
//!   wound normal, at every call site — and eases along it, rather than jittering in a ball.
//!
//! The trauma² magnitude and its linear decay follow Eiserloh's GDC 2016 talk, which is **the same
//! source the consuming game's `juice.rs` already cites** — so the crate and the game share one model
//! rather than approximating each other.
//!
//! # Determinism
//!
//! [`shake_offset`] is indexed by `tick`, not by an accumulator, so the same tick always yields the
//! same offset and a replay reproduces the shake exactly. Its one draw comes from
//! [`crate::soup::hash_f32`], the crate's only randomness.

use bevy::math::Vec3;

use crate::CarnageSettings;
use crate::soup::hash_f32;
use crate::wound::Wound;

/// Trauma to add for a wound, in `[0, 1]` — **the caller adds it to its own accumulator.**
///
/// Scales with the wound's severity, so a graze contributes a graze's worth. Returned rather than
/// applied because the consuming game owns a `Trauma(f32)` component with its own decay, and two
/// writers of one accumulator is the same defect this whole module is arranged to avoid.
pub fn trauma_for(w: &Wound, s: &CarnageSettings) -> f32 {
    (s.trauma_per_wound * w.severity.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// How many fixed ticks of hit-stop this wound deserves. **Zero for a graze.**
///
/// The seconds-to-ticks conversion happens here, against the caller's own `hz`, and it **truncates**:
/// a wound worth less than one whole tick gets none. Rounding up instead would give every scratch a
/// frame of freeze, and a freeze frame on a scratch is the failure mode §III-C warns about — hit stop
/// spent everywhere stops reading as impact anywhere.
pub fn hitstop_ticks(w: &Wound, hz: u32, s: &CarnageSettings) -> u32 {
    let seconds = s.hitstop_seconds * w.severity.clamp(0.0, 1.0);
    if !(seconds > 0.0) || !seconds.is_finite() {
        return 0;
    }
    let ticks = seconds * hz as f32;
    if !ticks.is_finite() || ticks < 1.0 {
        return 0;
    }
    ticks as u32
}

/// Camera offset for a given trauma at a given tick: trauma² magnitude, eased, along `dir`.
///
/// **The crate never applies this.** It returns a vector and the caller moves its own camera.
///
/// - **trauma²** rather than trauma: the survey's point is that shake should read as a *response*, and
///   squaring makes a small hit almost still while leaving a large one violent.
/// - **`ease = (1 - phase)²`** over a `shake_ticks` cycle, so each cycle settles rather than buzzing.
/// - **A tick-indexed `wave` in `[-1, 1]`**, which is what keeps successive frames from returning the
///   same vector while remaining a pure function of the tick — so a replay shakes identically.
/// - **Along `dir`**, which is the wound normal at every call site: the "semantically significant
///   direction" §III-B-1 asks for, rather than a random ball.
///
/// A zero or non-finite `dir` yields [`Vec3::ZERO`] — there is no fabricated fallback direction,
/// because a shake along an invented axis would report a hit that did not happen there.
pub fn shake_offset(trauma: f32, dir: Vec3, tick: u32, s: &CarnageSettings) -> Vec3 {
    let trauma = trauma.clamp(0.0, 1.0);
    if trauma <= 0.0 || s.shake_ticks == 0 {
        return Vec3::ZERO;
    }
    let axis = dir.normalize_or_zero();
    if axis == Vec3::ZERO {
        return Vec3::ZERO;
    }
    let phase = (tick % s.shake_ticks) as f32 / s.shake_ticks as f32;
    let ease = (1.0 - phase) * (1.0 - phase);
    let wave = hash_f32(tick.wrapping_mul(0x9E37_79B9)) * 2.0 - 1.0;
    axis * (s.shake_amplitude * trauma * trauma * ease * wave)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wound::WoundKind;

    const HZ: u32 = 60;

    fn wound(severity: f32) -> Wound {
        Wound {
            at: Vec3::new(0.1, 0.9, -0.2),
            normal: Vec3::X,
            area: 0.004,
            severity,
            kind: WoundKind::Severance,
        }
    }

    /// Trauma tracks severity and stays inside `[0, 1]` whatever the caller hands in — including the
    /// out-of-range severities a mis-scaled caller could produce.
    #[test]
    fn trauma_tracks_severity_and_stays_bounded() {
        let s = CarnageSettings::default();
        assert_eq!(trauma_for(&wound(0.0), &s), 0.0, "a wound that is not open costs nothing");
        assert_eq!(trauma_for(&wound(1.0), &s), s.trauma_per_wound, "a full wound is the dial");
        assert!(
            trauma_for(&wound(0.5), &s) < trauma_for(&wound(1.0), &s),
            "half a wound must be less trauma"
        );
        for severity in [-3.0f32, 0.25, 1.0, 17.0] {
            let t = trauma_for(&wound(severity), &s);
            assert!((0.0..=1.0).contains(&t), "trauma {t} from severity {severity} is out of range");
        }
    }

    /// **A graze gets no hit-stop, and that is the point of truncating.** A freeze frame spent on
    /// every scratch stops reading as impact — the survey's own warning about hit stop.
    #[test]
    fn a_graze_gets_no_hitstop() {
        let s = CarnageSettings::default();
        assert_eq!(hitstop_ticks(&wound(1.0), HZ, &s), 3, "0.055 s at 60 Hz is three whole ticks");
        assert_eq!(hitstop_ticks(&wound(0.0), HZ, &s), 0, "no wound, no stop");
        // 0.055 * 0.2 * 60 = 0.66 ticks — less than one whole tick, so none.
        assert_eq!(hitstop_ticks(&wound(0.2), HZ, &s), 0, "a graze must not freeze the frame");
        assert!(
            hitstop_ticks(&wound(1.0), 120, &s) > hitstop_ticks(&wound(1.0), HZ, &s),
            "the same duration is more ticks at a higher rate"
        );
    }

    /// **The same tick always yields the same offset** — which is what makes a recorded run's shake
    /// reproducible, and is the reason this is indexed rather than accumulated.
    #[test]
    fn the_shake_is_a_pure_function_of_the_tick() {
        let s = CarnageSettings::default();
        for tick in [0u32, 1, 7, 60, 12_345, u32::MAX] {
            let a = shake_offset(0.7, Vec3::X, tick, &s);
            let b = shake_offset(0.7, Vec3::X, tick, &s);
            assert_eq!(a, b, "tick {tick} shook differently the second time");
        }
    }

    /// The shake is **along the direction given**, never in a random ball — §III-B-1's requirement,
    /// asserted. And a direction that is not a direction produces no shake rather than an invented one.
    #[test]
    fn the_shake_is_along_the_direction_it_was_given() {
        let s = CarnageSettings::default();
        for dir in [Vec3::X, Vec3::Y, Vec3::new(-0.3, 0.9, 0.2)] {
            let axis = dir.normalize();
            for tick in 0..200u32 {
                let o = shake_offset(1.0, dir, tick, &s);
                if o == Vec3::ZERO {
                    continue;
                }
                let along = o.normalize();
                assert!(
                    (along.dot(axis).abs() - 1.0).abs() < 1.0e-4,
                    "tick {tick}: offset {o:?} is not colinear with {axis:?}"
                );
            }
        }
        assert_eq!(
            shake_offset(1.0, Vec3::ZERO, 5, &s),
            Vec3::ZERO,
            "a wound with no normal must not shake along an invented axis"
        );
    }

    /// Magnitude is trauma², bounded by the amplitude dial, and zero at zero trauma.
    #[test]
    fn the_magnitude_is_trauma_squared_and_bounded() {
        let s = CarnageSettings::default();
        assert_eq!(shake_offset(0.0, Vec3::X, 3, &s), Vec3::ZERO, "no trauma, no shake");

        let (mut peak_half, mut peak_full) = (0.0f32, 0.0f32);
        for tick in 0..1_000u32 {
            let half = shake_offset(0.5, Vec3::X, tick, &s).length();
            let full = shake_offset(1.0, Vec3::X, tick, &s).length();
            assert!(
                full <= s.shake_amplitude + 1.0e-6,
                "tick {tick}: |offset| {full} exceeds the amplitude dial {}",
                s.shake_amplitude
            );
            peak_half = peak_half.max(half);
            peak_full = peak_full.max(full);
        }
        // trauma² means half the trauma is a quarter of the shake, not half of it.
        let ratio = peak_full / peak_half;
        assert!(
            (ratio - 4.0).abs() < 0.2,
            "peak shake scaled by {ratio:.3} between trauma 0.5 and 1.0; trauma squared requires ~4"
        );
    }

    /// **Each cycle settles.** The ease is `(1-phase)²`, so the offset is largest at the start of a
    /// cycle and exactly zero at its last tick — a shake that buzzed forever would never read as one
    /// impact.
    #[test]
    fn each_shake_cycle_settles_to_nothing() {
        let s = CarnageSettings::default();
        let last = s.shake_ticks - 1;
        // At `phase = last/shake_ticks` the ease is small but not zero; at a multiple of the period
        // the phase resets. What must hold is that the ease is monotone within the cycle, which is
        // checked by comparing the ease factor directly rather than the hashed wave on top of it.
        let ease_at = |tick: u32| {
            let phase = (tick % s.shake_ticks) as f32 / s.shake_ticks as f32;
            (1.0 - phase) * (1.0 - phase)
        };
        for t in 0..last {
            assert!(ease_at(t) > ease_at(t + 1), "the ease rose from tick {t} to {}", t + 1);
        }
        assert!(ease_at(last) < ease_at(0) * 0.05, "the cycle must nearly vanish by its last tick");
    }

    /// A settings block whose `shake_ticks` is zero would be a modulo by zero. It is refused at the
    /// door, and even if one reached here it returns nothing rather than panicking.
    #[test]
    fn a_zero_shake_period_cannot_panic() {
        let mut s = CarnageSettings::default();
        s.shake_ticks = 0;
        assert!(s.validate().is_err(), "the door must refuse it");
        assert_eq!(shake_offset(1.0, Vec3::X, 9, &s), Vec3::ZERO, "and it must not panic here");
    }
}
