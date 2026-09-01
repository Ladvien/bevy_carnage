//! **The bleed schedule.** How long a wound keeps throwing blood, and on which ticks.
//!
//! # Ticks, not seconds — and this module is where that rule is load-bearing
//!
//! Every function here takes `tick: u32` and `hz: u32`. Nothing reads [`bevy::time::Time`], its
//! virtual clock or its real one. A pulse train driven by an accumulated `delta_secs()` drifts, and
//! worse than drifting it *collapses*: a float accumulator large enough stops advancing at all, which
//! is the failure the consuming game records against its own decal tiebreak after ~1.7e7 events. An
//! integer tick counter with an integer modulo cannot drift, cannot collapse, and reproduces exactly
//! under a replay.
//!
//! **The shipped tick counts assume a 60 Hz fixed tick.** `hz` is a parameter precisely so a caller on
//! another rate is not wrong, but [`CarnageSettings::spurt_ticks`](crate::CarnageSettings::spurt_ticks)
//! and [`clot_ticks`](crate::CarnageSettings::clot_ticks) are counts rather than durations, so a game
//! on 30 Hz re-derives those two (and `shake_ticks`, and `hitstop_seconds`' conversion) in its own
//! config. That is one place to change, in data, and it is stated here because a silent
//! `hz`-dependence in a *duration* is exactly the kind of thing that only shows up on someone else's
//! machine.
//!
//! # The physics this is a reduction of
//!
//! Lai & Xiang, *"Bleeding simulation of virtual surgery implemented on GPU"* (2014),
//! `doi:10.11834/jig.20141016`. Their SPH blood raises viscosity with falling temperature as
//! `μ = b·exp(−a·T)` (a = 200, b = 0.5) and **stops the particles entirely below 0 °C** — which is a
//! clot, expressed as a fluid property.
//!
//! At game scale the useful content of that is its shape rather than its state variable: flow falls
//! monotonically and reaches **exactly** zero, and after it does it never resumes. So there is no
//! temperature field here and no SPH; there is one monotone ramp and one integer comparison, and
//! `clotted` is a fact about tick arithmetic rather than a threshold that could be crossed twice.
//!
//! # One code path from the first jet to the last seep
//!
//! [`pulse_wound`] returns the wound with its `severity` scaled by [`flow`]. Everything downstream —
//! droplet count, spray, stains — reads `severity` and nothing else, so an arterial spurt and a
//! dying trickle are the same model at two numbers. There is deliberately no separate "seep" path.

use bevy::ecs::component::Component;

use crate::CarnageSettings;
use crate::wound::Wound;

/// A wound's bleed state. Ticks, not seconds — see the module docs.
///
/// Two fields, and neither is a running total: `opened_at` is when, `area` is how much. Everything
/// else is derived from `(tick - opened_at)`, so this struct cannot drift out of step with the clock
/// that drives it and a caller can serialize it into a save with no fixups.
///
/// **A `Component`, because a bleeding thing is an entity.** A caller attaches one to a detached
/// chunk and the schedule below reads it; removing the component when [`clotted`] is what makes
/// "once clotted, never again" true of the scene as well as of the arithmetic. It is still a plain
/// value with pure functions over it — nothing here is a system, and the crate registers none.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bleed {
    /// The fixed tick the wound opened on.
    pub opened_at: u32,
    /// The wound's area, carried so a caller can rebuild a [`Wound`] for a pulse without keeping one.
    pub area: f32,
}

impl Bleed {
    /// Open a bleed at `tick` with a wound's area.
    pub fn new(opened_at: u32, area: f32) -> Self {
        Bleed { opened_at, area }
    }

    /// Ticks elapsed since it opened.
    ///
    /// **`wrapping_sub`, deliberately.** A `u32` fixed-tick counter wraps after about 2.3 years of
    /// continuous 60 Hz play; a wound opened just before the wrap then reads as freshly opened rather
    /// than panicking or saturating to "eternally clotted". That is the least surprising of the three
    /// and it cannot panic, which is the property that matters — the alternative is a subtraction
    /// overflow that takes the process down in a debug build at a moment nobody can reproduce.
    fn age(&self, tick: u32) -> u32 {
        tick.wrapping_sub(self.opened_at)
    }
}

/// Ticks between heartbeats, at least 1.
///
/// **Integer, so the train cannot drift.** A period computed once and compared with `%` puts every
/// pulse exactly `period` ticks after the last one, forever, at any frame rate. `hz = 0` or a
/// nonsense `spurt_bpm` floors to 1 rather than dividing by zero — and a settings block that would
/// produce one is refused by [`CarnageSettings::validate`] before it gets here.
pub fn pulse_period(hz: u32, s: &CarnageSettings) -> u32 {
    if !(s.spurt_bpm > 0.0) || !s.spurt_bpm.is_finite() {
        return 1;
    }
    let ticks = (hz as f32 * 60.0 / s.spurt_bpm).round();
    if !ticks.is_finite() || ticks < 1.0 {
        return 1;
    }
    (ticks as u32).max(1)
}

/// Is this the tick a heartbeat pulse lands on? `hz` is the caller's fixed-tick rate.
///
/// The tick the wound opened on is itself a pulse — a wound starts bleeding when it opens, not one
/// heartbeat later.
pub fn pulses_on(b: &Bleed, tick: u32, hz: u32, s: &CarnageSettings) -> bool {
    b.age(tick) % pulse_period(hz, s) == 0
}

/// Flow at `tick`, in the units [`droplet_count`](crate::droplet_count) scales by: full while
/// spurting, tapering to **exactly** `0.0` at the clot tick and staying there.
///
/// `hz` is accepted for symmetry with the rest of the module and is deliberately unused: the taper is
/// authored in ticks, so making it depend on the rate as well would be two dials for one thing.
pub fn flow(b: &Bleed, tick: u32, hz: u32, s: &CarnageSettings) -> f32 {
    let _ = hz;
    let age = b.age(tick);
    if age >= s.clot_ticks {
        return 0.0;
    }
    if age < s.spurt_ticks {
        return 1.0;
    }
    // Exactly zero at `clot_ticks` is guaranteed by the branch above rather than by this arithmetic,
    // which is why the branch is first: a `1.0 - x/x` would land on zero only if the float agreed.
    let taper = s.clot_ticks.saturating_sub(s.spurt_ticks);
    if taper == 0 {
        return 0.0;
    }
    let through = (age - s.spurt_ticks) as f32 / taper as f32;
    (1.0 - through).clamp(0.0, 1.0)
}

/// Has it clotted? **Once true, never false again** — for any tick at or past the clot.
pub fn clotted(b: &Bleed, tick: u32, hz: u32, s: &CarnageSettings) -> bool {
    let _ = hz;
    b.age(tick) >= s.clot_ticks
}

/// The wound a pulse throws this tick, or `None` between beats and after the clot.
///
/// The returned wound is `w` with `severity` scaled by [`flow`], so the same spatter model serves the
/// first arterial jet and the last seep with **no second code path**. `None` rather than a
/// zero-severity wound on an off-beat tick, because "no pulse happened" and "a pulse happened that
/// threw nothing" are different facts and a caller should not have to tell them apart by inspecting a
/// float.
pub fn pulse_wound(
    b: &Bleed,
    w: &Wound,
    tick: u32,
    hz: u32,
    s: &CarnageSettings,
) -> Option<Wound> {
    if !pulses_on(b, tick, hz, s) {
        return None;
    }
    let f = flow(b, tick, hz, s);
    if f <= 0.0 {
        return None;
    }
    Some(Wound { severity: (w.severity * f).clamp(0.0, 1.0), ..*w })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wound::WoundKind;
    use bevy::math::Vec3;

    const HZ: u32 = 60;

    fn wound() -> Wound {
        Wound {
            at: Vec3::new(0.1, 0.9, -0.2),
            normal: Vec3::X,
            area: 0.004,
            severity: 1.0,
            kind: WoundKind::Severance,
        }
    }

    /// **Pulse spacing is exact over a long run.** Every gap between consecutive pulses equals the
    /// period, with no accumulated error — which is the whole argument for integer ticks over a float
    /// accumulator, checked rather than asserted in a comment.
    #[test]
    fn the_pulse_train_never_drifts() {
        let s = CarnageSettings::default();
        let b = Bleed::new(0, 0.004);
        let period = pulse_period(HZ, &s);
        assert_eq!(period, 38, "96 bpm at 60 Hz is a pulse every 38 ticks");

        let beats: Vec<u32> = (0..600u32).filter(|t| pulses_on(&b, *t, HZ, &s)).collect();
        assert!(beats.len() > 10, "600 ticks must hold more than ten beats");
        assert_eq!(beats[0], 0, "the tick a wound opens on is itself a beat");
        for w in beats.windows(2) {
            assert_eq!(w[1] - w[0], period, "a gap of {} broke the train", w[1] - w[0]);
        }
    }

    /// The train is anchored to the wound, not to the world clock: a wound that opened on an odd tick
    /// beats on odd ticks. Two wounds opened at different moments must not sync up.
    #[test]
    fn the_train_is_anchored_to_the_wound_that_opened() {
        let s = CarnageSettings::default();
        let early = Bleed::new(0, 0.004);
        let late = Bleed::new(7, 0.004);
        let period = pulse_period(HZ, &s);
        assert!(pulses_on(&late, 7, HZ, &s), "it beats when it opens");
        assert!(pulses_on(&late, 7 + period, HZ, &s), "and a period later");
        assert!(!pulses_on(&late, 7 + 1, HZ, &s), "and not in between");
        assert_ne!(
            pulses_on(&early, 7, HZ, &s),
            pulses_on(&late, 7, HZ, &s),
            "two wounds opened at different ticks must not share a heartbeat"
        );
    }

    /// **Flow falls monotonically and is exactly zero at and after the clot.** Both halves matter: a
    /// non-monotone flow would have a wound bleed harder as it died, and a flow that only *approached*
    /// zero would leave a fragment emitting one droplet forever.
    #[test]
    fn flow_falls_monotonically_to_exactly_zero() {
        let s = CarnageSettings::default();
        let b = Bleed::new(0, 0.004);
        let mut last = f32::INFINITY;
        for t in 0..(s.clot_ticks + 240) {
            let f = flow(&b, t, HZ, &s);
            assert!(f <= last + 1.0e-7, "flow rose from {last} to {f} at tick {t}");
            assert!((0.0..=1.0).contains(&f), "flow {f} at tick {t} is outside [0, 1]");
            last = f;
        }
        assert_eq!(flow(&b, s.spurt_ticks - 1, HZ, &s), 1.0, "full flow right up to the taper");
        assert_eq!(flow(&b, s.clot_ticks, HZ, &s), 0.0, "exactly zero at the clot tick");
        assert_eq!(flow(&b, s.clot_ticks + 10_000, HZ, &s), 0.0, "and it stays there");
    }

    /// Clotting is a one-way door.
    #[test]
    fn clotting_never_reverses() {
        let s = CarnageSettings::default();
        let b = Bleed::new(0, 0.004);
        assert!(!clotted(&b, 0, HZ, &s), "it does not open already clotted");
        assert!(!clotted(&b, s.clot_ticks - 1, HZ, &s));
        for t in s.clot_ticks..(s.clot_ticks + 5_000) {
            assert!(clotted(&b, t, HZ, &s), "unclotted again at tick {t}");
        }
    }

    /// **A wound opened just before the tick counter wraps must not panic.** `u32` ticks wrap after
    /// about 2.3 years at 60 Hz; a subtraction overflow there is a debug-build crash nobody can
    /// reproduce. `wrapping_sub` makes it read as freshly opened instead, which is documented on
    /// [`Bleed::age`] as the accepted behaviour.
    #[test]
    fn a_wound_opened_before_the_tick_wrap_does_not_panic() {
        let s = CarnageSettings::default();
        let b = Bleed::new(u32::MAX - 3, 0.004);
        for t in [u32::MAX - 3, u32::MAX - 1, u32::MAX, 0, 1, 2, 40] {
            let f = flow(&b, t, HZ, &s);
            assert!((0.0..=1.0).contains(&f), "flow {f} at wrapped tick {t}");
            let _ = pulses_on(&b, t, HZ, &s);
            let _ = clotted(&b, t, HZ, &s);
            let _ = pulse_wound(&b, &wound(), t, HZ, &s);
        }
        assert_eq!(
            flow(&b, u32::MAX - 3, HZ, &s),
            1.0,
            "the tick it opened on is full flow, wrap or not"
        );
    }

    /// **The one code path.** A pulse wound is the wound with a scaled severity and nothing else
    /// changed, so the spatter model needs no notion of "late bleeding".
    #[test]
    fn a_pulse_is_the_same_wound_at_a_lower_severity() {
        let s = CarnageSettings::default();
        let b = Bleed::new(0, 0.004);
        let w = wound();

        let first = pulse_wound(&b, &w, 0, HZ, &s).expect("it pulses the tick it opens");
        assert_eq!(first.severity, 1.0, "the first jet is full severity");
        assert_eq!(first.at, w.at, "and it is the same wound");
        assert_eq!(first.normal, w.normal);
        assert_eq!(first.area, w.area);
        assert_eq!(first.kind, w.kind);

        assert!(pulse_wound(&b, &w, 1, HZ, &s).is_none(), "nothing between beats");

        // The last beat strictly inside the taper must still throw something, and it must throw less.
        let period = pulse_period(HZ, &s);
        let late = (s.clot_ticks - 1) / period * period;
        let seep = pulse_wound(&b, &w, late, HZ, &s).expect("the last beat before the clot bleeds");
        assert!(
            seep.severity < first.severity && seep.severity > 0.0,
            "the last seep was {} against a first jet of {}",
            seep.severity,
            first.severity
        );

        for t in s.clot_ticks..(s.clot_ticks + period * 4) {
            assert!(pulse_wound(&b, &w, t, HZ, &s).is_none(), "a clotted wound pulsed at {t}");
        }
    }

    /// A degenerate rate cannot divide by zero — the floor is 1, and the door check refuses the
    /// settings that would reach it.
    #[test]
    fn a_degenerate_rate_floors_instead_of_dividing_by_zero() {
        let mut s = CarnageSettings::default();
        assert_eq!(pulse_period(0, &s), 1, "zero hz floors to every tick");
        s.spurt_bpm = 0.0;
        assert_eq!(pulse_period(HZ, &s), 1, "zero bpm floors rather than dividing by zero");
        assert!(s.validate().is_err(), "and such a block is refused at the door anyway");
    }

    /// The rate is a parameter, so a game on another fixed tick gets a proportional period rather
    /// than a wrong one.
    #[test]
    fn the_period_scales_with_the_callers_rate() {
        let s = CarnageSettings::default();
        assert_eq!(pulse_period(30, &s), 19, "half the rate is half the ticks per beat");
        assert_eq!(pulse_period(120, &s), 75, "double the rate is double");
    }
}
