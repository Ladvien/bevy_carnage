//! **The bleed schedule.** How long a wound keeps throwing blood, and on which ticks.
//!
//! # Ticks, not seconds — and this module is where that rule is load-bearing
//!
//! Every function here takes `tick: u32` and `hz: u32`. Nothing reads a clock, virtual or real. A
//! pulse train driven by an accumulated `delta_secs()` drifts, and worse than drifting it *collapses*:
//! a float accumulator large enough stops advancing at all, which is the failure the consuming game
//! records against its own decal tiebreak after ~1.7e7 events. An integer tick counter with an integer
//! modulo cannot drift, cannot collapse, and reproduces exactly under a replay.
//!
//! **The shipped tick counts assume a 60 Hz fixed tick.** `hz` is a parameter precisely so a caller on
//! another rate is not wrong, but [`BloodSettings::spurt_ticks`] and
//! [`clot_ticks`](BloodSettings::clot_ticks) are counts rather than durations, so a game on 30 Hz
//! re-derives them in its own config.
//!
//! # Cessation is a material property, not a timer
//!
//! **There is no `clotted` boolean here, and its removal is the design.** A wound stops bleeding when
//! [`rheo::flows`] says the stress driving blood out of it no longer exceeds the blood's own yield
//! stress — the same comparison that decides whether a rivulet on a wall runs or arrests. So the clot,
//! the arrested rivulet and the pool that stopped creeping are **one mechanism at three ages**, and
//! [`flow`] is demoted from "the schedule" to what it always physically was: the perfusion envelope
//! that supplies the driving stress.
//!
//! What that changes, stated plainly: a wound now arrests **inside** the taper rather than exactly at
//! [`BloodSettings::clot_ticks`], because a falling driving stress meets a rising yield stress before
//! the envelope reaches zero. That crossing is the clot, and it is visible — which the old integer
//! comparison never was.
//!
//! # The physics this is a reduction of
//!
//! Lai & Xiang, *"Bleeding simulation of virtual surgery implemented on GPU"* (2014),
//! `doi:10.11834/jig.20141016`. Their SPH blood raises viscosity with falling temperature as
//! `μ = b·exp(−a·T)` and **stops the particles entirely below 0 °C** — a clot, expressed as a fluid
//! property. At game scale the useful content of that is its shape rather than its state variable, and
//! [`rheo`](crate::bloodstain::rheo) is where that shape now lives.

use crate::bloodstain::rheo::{self, PERFUSION_STRESS_PA};
use crate::bloodstain::settings::BloodSettings;
use crate::bloodstain::{Wound, m};

/// A wound's bleed state. Ticks, not seconds — see the module docs.
///
/// Three fields, and none is a running total: `opened_at` is when, `area` is how much, `seed` is
/// *which* wound. Everything else is derived from `(tick - opened_at)`, so this struct cannot drift
/// out of step with the clock that drives it and a caller can serialize it into a save with no
/// fixups.
///
/// **A plain value, deliberately.** It carries no ECS derive because this crate has no engine in it;
/// a consumer that wants a bleeding *entity* wraps it in its own component — `bevy_carnage::Bleeding`
/// is that wrapper, and it exists so this type can stay engine-free without anyone losing the
/// component.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bleed {
    /// The fixed tick the wound opened on.
    pub opened_at: u32,
    /// The wound's area, carried so a caller can rebuild a [`Wound`] for a pulse without keeping one.
    pub area: f32,
    /// **Which wound this is**, and the only reason it exists is [`pulse_phase`].
    ///
    /// One blow detaches many fragments in one frame, so every wound it opens shares `opened_at` —
    /// and a schedule keyed on age alone then beats all of them in lockstep, forever, which reads as
    /// one machine rather than as a body. This offsets each wound's beat within the period.
    ///
    /// **Seed it from the wound's own geometry, never from an entity id**: an id is a slot index
    /// assigned by allocation order, which is the one thing this crate refuses to seed from. See
    /// [`crate::bloodstain::hash_f32`].
    pub seed: u32,
}

impl Bleed {
    /// Open a bleed at `tick` for a wound.
    ///
    /// **The wound is the only argument because both derived fields must come from it**: the area is
    /// the wound's, and the seed is [`crate::bloodstain::wound_seed`] of the same wound — its quantised position
    /// and kind. A constructor taking a loose seed would let a caller hand it an entity id, which is
    /// a slot index assigned by allocation order and the one thing this crate refuses to seed from.
    pub fn new(opened_at: u32, w: &Wound) -> Self {
        Bleed { opened_at, area: w.area, seed: crate::bloodstain::wound_seed(w) }
    }

    /// Ticks elapsed since it opened.
    ///
    /// **`wrapping_sub`, deliberately.** A `u32` fixed-tick counter wraps after about 2.3 years of
    /// continuous 60 Hz play; a wound opened just before the wrap then reads as freshly opened rather
    /// than panicking or saturating to "eternally clotted". That is the least surprising of the three
    /// and it cannot panic, which is the property that matters — the alternative is a subtraction
    /// overflow that takes the process down in a debug build at a moment nobody can reproduce.
    pub fn age(&self, tick: u32) -> u32 {
        tick.wrapping_sub(self.opened_at)
    }
}

/// Ticks between heartbeats, at least 1.
///
/// **Integer, so the train cannot drift.** A period computed once and compared with `%` puts every
/// pulse exactly `period` ticks after the last one, forever, at any frame rate. `hz = 0` or a nonsense
/// `spurt_bpm` floors to 1 rather than dividing by zero — and a settings block that would produce one
/// is refused by [`BloodSettings::validate`] before it gets here.
pub fn pulse_period(hz: u32, s: &BloodSettings) -> u32 {
    if !(s.spurt_bpm > 0.0) || !s.spurt_bpm.is_finite() {
        return 1;
    }
    let ticks = m::round(hz as f32 * 60.0 / s.spurt_bpm);
    if !ticks.is_finite() || ticks < 1.0 {
        return 1;
    }
    (ticks as u32).max(1)
}

/// **This wound's offset within the heartbeat**, in ticks, in `0..pulse_period`.
///
/// One heart, one rate, wounds out of step — which is the physiological reading as well as the
/// legible one: the pressure wave reaches a shin later than a shoulder, so two wounds on one body do
/// not spurt on the same frame. Giving each wound its own *rate* instead would be giving the body
/// several hearts.
///
/// Derived from [`Bleed::seed`] by the crate's own hash, so it is a function of the wound's geometry
/// and reproduces exactly under a replay.
pub fn pulse_phase(b: &Bleed, hz: u32, s: &BloodSettings) -> u32 {
    let period = pulse_period(hz, s);
    // `hash_f32` is in `[0, 1)`, so the product is in `[0, period)` before the `min` — which is
    // there for the one input where a float rounds to exactly `period`.
    ((crate::bloodstain::hash_f32(b.seed) * period as f32) as u32).min(period - 1)
}

/// Is this the tick a heartbeat pulse lands on? `hz` is the caller's fixed-tick rate.
///
/// The tick the wound opened on is **no longer necessarily a beat**: it is one only for a wound whose
/// [`pulse_phase`] is zero, and that is the point — a blow that opens twelve wounds on one tick must
/// not make twelve wounds that beat together. A wound therefore waits up to one period before its
/// first spurt, which at the shipped 72 bpm and 60 Hz is at most 50 ticks.
pub fn pulses_on(b: &Bleed, tick: u32, hz: u32, s: &BloodSettings) -> bool {
    let period = pulse_period(hz, s);
    b.age(tick) % period == pulse_phase(b, hz, s)
}

/// **The perfusion envelope by age**: full while spurting, tapering to exactly `0.0` at
/// [`BloodSettings::clot_ticks`] and staying there.
///
/// One implementation, two spellings: [`flow`] is this function with a [`Bleed`] to take the age from.
/// A caller reasoning about ages — [`crate::bloodstain::rheo`]'s own tests, a drying curve — reads this one.
pub fn envelope(age_ticks: u32, s: &BloodSettings) -> f32 {
    if age_ticks >= s.clot_ticks {
        return 0.0;
    }
    if age_ticks < s.spurt_ticks {
        return 1.0;
    }
    // Exactly zero at `clot_ticks` is guaranteed by the branch above rather than by this arithmetic,
    // which is why the branch is first: a `1.0 - x/x` would land on zero only if the float agreed.
    let taper = s.clot_ticks.saturating_sub(s.spurt_ticks);
    if taper == 0 {
        return 0.0;
    }
    let through = (age_ticks - s.spurt_ticks) as f32 / taper as f32;
    (1.0 - through).clamp(0.0, 1.0)
}

/// The perfusion envelope at `tick`, in the units [`crate::bloodstain::droplet_count`] scales by.
///
/// `hz` is accepted for symmetry with the rest of the module and is deliberately unused: the taper is
/// authored in ticks, so making it depend on the rate as well would be two dials for one thing.
pub fn flow(b: &Bleed, tick: u32, hz: u32, s: &BloodSettings) -> f32 {
    let _ = hz;
    envelope(b.age(tick), s)
}

/// **The stress driving blood out of this wound at `tick`, Pa.**
///
/// [`rheo::PERFUSION_STRESS_PA`] scaled by the envelope — so it falls exactly as perfusion does, and
/// [`rheo::flows`] compares it against a yield stress that is rising over the same ticks. Their
/// crossing is the clot.
pub fn driving_stress(b: &Bleed, tick: u32, hz: u32, s: &BloodSettings) -> f32 {
    PERFUSION_STRESS_PA * flow(b, tick, hz, s)
}

/// The wound a pulse throws this tick, or `None` between beats and once the blood has yielded.
///
/// The returned wound is `w` with `severity` scaled by [`flow`], so the same spatter model serves the
/// first arterial jet and the last seep with **no second code path**. `None` rather than a
/// zero-severity wound on an off-beat tick, because "no pulse happened" and "a pulse happened that
/// threw nothing" are different facts and a caller should not have to tell them apart by inspecting a
/// float.
///
/// **One predicate decides cessation** — [`rheo::flows`] against [`rheo::yield_stress`]. There is no
/// second `f <= 0.0` guard, because a second guard would be a second answer to the same question.
pub fn pulse_wound(
    b: &Bleed,
    w: &Wound,
    tick: u32,
    hz: u32,
    s: &BloodSettings,
) -> Option<Wound> {
    if !pulses_on(b, tick, hz, s) {
        return None;
    }
    let age = b.age(tick);
    if !rheo::flows(driving_stress(b, tick, hz, s), rheo::yield_stress(age, hz, s)) {
        return None;
    }
    let f = flow(b, tick, hz, s);
    Some(Wound { severity: (w.severity * f).clamp(0.0, 1.0), ..*w })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bloodstain::WoundKind;
    use std::vec::Vec as StdVec;

    fn wound() -> Wound {
        Wound {
            at: [0.0, 1.0, 0.0],
            normal: crate::bloodstain::vec::X,
            area: 0.004,
            severity: 1.0,
            kind: WoundKind::Severance,
        }
    }

    /// The pulse train is exactly periodic, forever, and starts at the wound's own phase.
    #[test]
    fn the_pulse_train_cannot_drift() {
        let s = BloodSettings::default();
        let b = Bleed::new(1000, &wound());
        let period = pulse_period(60, &s);
        let phase = pulse_phase(&b, 60, &s);
        assert!(period >= 1, "the period is a modulus and must never be zero");
        assert!(phase < period, "a phase outside the period would silence the wound forever");
        for k in 0..64u32 {
            assert!(
                pulses_on(&b, 1000 + phase + k * period, 60, &s),
                "beat {k} must land exactly {period} ticks after the last"
            );
            if period > 1 {
                assert!(
                    !pulses_on(&b, 1000 + phase + k * period + 1, 60, &s),
                    "the tick after a beat must not also be one"
                );
            }
        }
    }

    /// **Wounds opened by one blow on one tick must not beat together.**
    ///
    /// This is the defect the phase exists for: a blow detaches many fragments in a single frame, so
    /// every wound it opens shares `opened_at`, and a schedule keyed on age alone spurts all of them
    /// on the same frame forever — which reads as one machine rather than as a body. Prediction: a
    /// spread of wounds across a body occupies many distinct phases, and the rate is still one rate.
    #[test]
    fn wounds_opened_on_one_tick_do_not_beat_together() {
        let s = BloodSettings::default();
        let period = pulse_period(60, &s);
        let bleeds: StdVec<Bleed> = (0..24u32)
            .map(|i| {
                let t = i as f32;
                let at = [0.03 * t - 0.3, 1.0 - 0.05 * t, 0.02 * t];
                Bleed::new(1000, &Wound { at, ..wound() })
            })
            .collect();
        let phases: StdVec<u32> = bleeds.iter().map(|b| pulse_phase(b, 60, &s)).collect();
        let mut distinct = phases.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(
            distinct.len() >= 16,
            "24 wounds across a body must not share a beat; got {} distinct phases of {period} \
             ({phases:?})",
            distinct.len()
        );
        // One heart, one rate: every wound's period is still the shared one.
        for (i, b) in bleeds.iter().enumerate() {
            let beats: StdVec<u32> =
                (1000..1000 + 4 * period).filter(|&t| pulses_on(b, t, 60, &s)).collect();
            assert_eq!(beats.len(), 4, "wound {i} must beat exactly four times in four periods");
            for w in beats.windows(2) {
                assert_eq!(w[1] - w[0], period, "wound {i} must keep the shared rate");
            }
        }
    }

    /// The phase is a function of the wound's geometry, so a replay reproduces it exactly.
    #[test]
    fn one_wound_always_gets_the_same_phase() {
        let s = BloodSettings::default();
        let w = wound();
        let a = pulse_phase(&Bleed::new(1000, &w), 60, &s);
        let b = pulse_phase(&Bleed::new(7777, &w), 60, &s);
        assert_eq!(a, b, "the phase is the wound's, not the tick it opened on");
    }

    /// A nonsense heart rate floors the period rather than dividing by zero.
    #[test]
    fn a_nonsense_heart_rate_cannot_divide_by_zero() {
        for bpm in [0.0f32, -10.0, f32::NAN, f32::INFINITY] {
            let s = BloodSettings { spurt_bpm: bpm, ..Default::default() };
            assert_eq!(pulse_period(60, &s), 1, "bpm = {bpm} must floor the period to 1");
        }
        let s = BloodSettings::default();
        assert_eq!(pulse_period(0, &s), 1, "a zero tick rate must floor the period to 1");
    }

    /// The envelope is full, then monotonically falling, then exactly zero — and it never rises.
    #[test]
    fn the_envelope_falls_monotonically_to_exactly_zero() {
        let s = BloodSettings::default();
        assert_eq!(envelope(0, &s), 1.0, "a fresh wound is at full perfusion");
        assert_eq!(envelope(s.spurt_ticks - 1, &s), 1.0, "full until the taper starts");
        assert_eq!(envelope(s.clot_ticks, &s), 0.0, "exactly zero at the clot tick");
        assert_eq!(envelope(s.clot_ticks + 5000, &s), 0.0, "and it stays there");
        let mut last = f32::INFINITY;
        for age in 0..=s.clot_ticks {
            let f = envelope(age, &s);
            assert!(f <= last, "the envelope rose at age {age}: {last} then {f}");
            last = f;
        }
    }

    /// A pulse throws blood while the wound flows, throws none between beats, and stops for good once
    /// the blood has yielded — the one predicate, seen from the caller's side.
    #[test]
    fn a_pulse_stops_for_good_when_the_blood_yields() {
        let s = BloodSettings::default();
        let b = Bleed::new(0, &wound());
        let w = wound();
        let phase = pulse_phase(&b, 60, &s);
        let first = pulse_wound(&b, &w, phase, 60, &s)
            .expect("a fresh wound must throw blood on its own first beat");
        assert_eq!(first.severity, 1.0, "a fresh wound throws at full severity");

        let period = pulse_period(60, &s);
        if period > 1 {
            assert!(
                pulse_wound(&b, &w, phase + 1, 60, &s).is_none(),
                "no pulse happened on this tick, so there is no wound to throw"
            );
        }

        let mut stopped_at = None;
        for tick in 0..=(s.clot_ticks + 600) {
            let p = pulse_wound(&b, &w, tick, 60, &s);
            match (stopped_at, p) {
                (Some(t), Some(_)) => {
                    panic!("blood resumed at tick {tick} after stopping at {t}")
                }
                (None, None) if tick % period == phase && tick > s.spurt_ticks => {
                    stopped_at = Some(tick)
                }
                _ => {}
            }
        }
        let stopped = stopped_at.expect("the wound must stop bleeding");
        assert!(
            stopped > s.spurt_ticks && stopped < s.clot_ticks,
            "the arrest must land inside the taper ({}..{}), got {stopped}",
            s.spurt_ticks,
            s.clot_ticks
        );
    }

    /// Severity is the envelope's, so one spatter model serves the first jet and the last seep. A
    /// separate "seep" path is exactly what this asserts does not exist.
    #[test]
    fn severity_tapers_through_one_code_path() {
        let s = BloodSettings::default();
        let b = Bleed::new(0, &wound());
        let w = wound();
        let period = pulse_period(60, &s);
        let phase = pulse_phase(&b, 60, &s);
        let mut seen: std::vec::Vec<f32> = std::vec::Vec::new();
        for k in 0..(s.clot_ticks / period) {
            if let Some(p) = pulse_wound(&b, &w, k * period + phase, 60, &s) {
                seen.push(p.severity);
            }
        }
        assert!(seen.len() > 4, "the wound must pulse several times before arresting");
        for pair in seen.windows(2) {
            assert!(pair[1] <= pair[0], "severity rose between pulses: {pair:?}");
        }
        assert!(*seen.last().unwrap_or(&1.0) < 1.0, "the last pulse must be a taper, not a jet");
    }

    /// The tick counter wraps after about 2.3 years of play, and a wound that spans the wrap must
    /// read as freshly opened rather than panic. `wrapping_sub` is the one path here.
    #[test]
    fn a_wound_that_spans_the_tick_wrap_does_not_panic() {
        let s = BloodSettings::default();
        let b = Bleed::new(u32::MAX - 10, &wound());
        assert_eq!(b.age(u32::MAX - 10), 0);
        assert_eq!(b.age(9), 20, "the wrap must be arithmetic, not a panic");
        assert!(pulse_wound(&b, &wound(), 9, 60, &s).is_some() || true, "must not panic");
    }
}
