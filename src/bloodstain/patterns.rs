//! **The forensic pattern taxonomy, as generators.** Six classes, each a different mechanism.
//!
//! # Why six generators and not one cone with six presets
//!
//! The SWGSTAIN / ASB TR-033 taxonomy is a *classification* of mechanisms: an analyst reads a scene
//! backwards from stain morphology to what produced it, and the classes exist because they are
//! distinguishable. A game that throws one isotropic cone for every event has exactly one pattern at
//! six intensities, and a player reads it as one thing happening repeatedly.
//!
//! So each generator here is a different mechanism, and the differences are the measured ones:
//!
//! - [`impact_spatter`] — the percolation cone. Many droplets, size inversely correlated with speed.
//! - [`arterial_arc`] — **one arc of large discrete droplets per systole**, not another cone, with a
//!   reach that decays as the body loses pressure.
//! - [`cast_off`] — released **tangentially** to a swinging tip's path, with diameter *inversely*
//!   proportional to tangential speed, from a pendant volume that is capped.
//! - [`expirated`] — a fine mist, sometimes with bubble rings, usually without.
//! - [`drip_trail`] — spacing that encodes walking speed, from a budget that runs out.
//! - [`transfer`] — a contact smear that moves blood out of that same budget.
//!
//! # The budget is what makes a pattern tell a story
//!
//! [`cast_off`], [`drip_trail`] and [`transfer`] all take `load_ml: &mut f32` and **decrement it**. A
//! dragged body runs out of blood; a knife swung six times sheds less each swing. That conservation is
//! the difference between a pattern that reads as evidence and one that reads as a particle emitter
//! with a lifetime.

use core::f32::consts::PI;

use crate::bloodstain::bleed::{self, Bleed};
use crate::bloodstain::droplet::{Droplet, droplets};
use crate::bloodstain::settings::BloodSettings;
use crate::bloodstain::stain::{Stain, stain_radius};
use crate::bloodstain::{V3, WELD, Wound, hash_f32, m, plane_basis, rheo, to_radians, vec};

/// **The pattern classes, as the taxonomy names them.**
///
/// **Append-only.** These discriminants are mixed into seeds and travel in saved data; a new class
/// goes on the end with the next number, and nothing is ever renumbered or reordered. Renumbering
/// silently moves every seed derived from a class and every golden that depends on one — the same
/// trap [`crate::bloodstain::WoundKind`] carries the same warning about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u32)]
pub enum PatternClass {
    /// A force applied to wet blood: the percolation cone.
    Impact = 0,
    /// A breached artery, one arc per heartbeat.
    ArterialSpurt = 1,
    /// Blood flung from a moving object as it changes direction.
    CastOff = 2,
    /// Blood forced from the nose, mouth or a chest wound by air.
    Expirated = 3,
    /// Free-falling drops from a carried or bleeding source in motion.
    DripTrail = 4,
    /// A wet, bloodied surface in contact with another.
    Transfer = 5,
}

impl PatternClass {
    /// Every class, in discriminant order — for a caller that wants to sweep them (a demo, a test,
    /// an expressive-range histogram) without writing the list out again.
    pub const ALL: [PatternClass; 6] = [
        PatternClass::Impact,
        PatternClass::ArterialSpurt,
        PatternClass::CastOff,
        PatternClass::Expirated,
        PatternClass::DripTrail,
        PatternClass::Transfer,
    ];
}

/// Diameter of one arterial droplet, metres.
///
/// **Large and discrete, which is the point.** An arterial pattern is a handful of big stains along an
/// arc, not a mist: the vessel delivers a bolus per systole rather than atomising it.
pub const ARTERIAL_DROPLET_M: f32 = 0.005;

/// Launch speed of an arterial jet at full pressure, m/s.
///
/// Sized against the reach it has to produce: under this crate's shipped 18 m/s² gravity a 45° launch
/// at 8 m/s carries about 3.5 m, which is the "sprayed the far wall" a breached carotid does.
pub const ARTERIAL_SPEED: f32 = 8.0;

/// Half-angle the arterial arc sweeps, degrees.
pub const ARTERIAL_ARC_DEG: f32 = 22.0;

/// Tangential speed below which a wet tip does **not** shed, m/s.
///
/// A slow move carries the pendant drop with it; shedding needs the tip to change direction faster
/// than surface tension can follow.
pub const CAST_OFF_MIN_V: f32 = 2.0;

/// Volume of one free-falling drip, millilitres.
///
/// A pendant drop detaches at a size set by surface tension against gravity, so this is a property of
/// the fluid rather than a dial — about 0.05 ml for blood from a smooth edge.
pub const DRIP_ML: f32 = 0.05;

/// Diameter of one free-falling drip at detachment, metres.
pub const DRIP_DIAMETER_M: f32 = 0.0045;

/// Terminal-ish fall speed used to size a drip's stain, m/s.
pub const DRIP_FALL_SPEED: f32 = 4.0;

/// Diameter of one expirated mist droplet, metres. Fine — that is what air does to blood.
pub const EXPIRATED_DROPLET_M: f32 = 0.000_9;

/// Half-angle of the expirated cone, degrees. Wide, because a breath is not a jet.
pub const EXPIRATED_CONE_DEG: f32 = 45.0;

/// **A seed from a place**, quantised on [`crate::bloodstain::WELD`] and salted by the pattern class.
///
/// The same rule [`crate::bloodstain::droplet::wound_seed`] uses and for the same reason: two runs that place a
/// contact a float ULP apart must seed identically, and a class must not collide with another class
/// at the same point.
fn site_seed(at: V3, class: PatternClass) -> u32 {
    let q = |x: f32| m::round(x / WELD) as i64 as u32;
    q(at[0])
        ^ q(at[1]).wrapping_mul(0x9E37_79B9)
        ^ q(at[2]).wrapping_mul(2_654_435_761)
        ^ (class as u32).wrapping_mul(0x27D4_EB2F)
}

/// **Impact spatter**: the percolation cone, unchanged.
///
/// A thin delegation on purpose — [`crate::bloodstain::droplet::droplets`] *is* this pattern, and wrapping it here
/// means [`PatternClass`] has one entry point per class with no class missing and no second cone
/// implementation to drift from the first.
pub fn impact_spatter(w: &Wound, s: &BloodSettings) -> Vec<Droplet> {
    droplets(w, s)
}

/// **One arterial arc per systole.** Large discrete droplets along a plane arc, reach falling as the
/// body loses pressure.
///
/// Fires only on a heartbeat tick ([`crate::bloodstain::bleed::pulses_on`]) and only while the blood still flows
/// ([`crate::bloodstain::rheo::flows`]) — the *same* two predicates the rest of the crate uses, so an arterial
/// wound clots by the same mechanism as any other and there is no arterial-specific stopping rule.
///
/// The arc lies in the plane spanned by the wound normal and the in-plane axis nearest "up", so a
/// spurt from a neck sweeps a wall the way a photograph shows rather than painting a disc.
///
/// `b` is the wound's bleed state, and it is required rather than derived: the pressure envelope is
/// measured from when *this wound* opened, and taking the age from an absolute tick would make the
/// first spurt of a late wound arrive already decayed.
pub fn arterial_arc(
    w: &Wound,
    b: &Bleed,
    tick: u32,
    hz: u32,
    s: &BloodSettings,
) -> Vec<Droplet> {
    if !bleed::pulses_on(b, tick, hz, s) {
        return Vec::new();
    }
    let age = b.age(tick);
    if !rheo::flows(bleed::driving_stress(b, tick, hz, s), rheo::yield_stress(age, hz, s)) {
        return Vec::new();
    }
    // Pressure falls linearly to zero over `pressure_decay_ticks`: exsanguination, in one number.
    let decay = s.pressure_decay_ticks.max(1) as f32;
    let pressure = (1.0 - age as f32 / decay).clamp(0.0, 1.0);
    if pressure <= 0.0 {
        return Vec::new();
    }

    let axis = vec::normalize_or_zero(w.normal);
    let (tangent, _) = plane_basis(axis);
    // **The stain count falls with pressure, it is not a fixed dial.** A systole ejects a bolus whose
    // volume drops as the body empties, so a spent artery places fewer discrete marks as well as
    // shorter ones. Held at `arc_stains` while the pressure is full, so the authored dial is still
    // exactly what a fresh spurt produces — and never below one, because an artery that is still
    // flowing at all throws something.
    let n = (m::round(s.arc_stains.max(1) as f32 * pressure).max(1.0) as u32).min(s.arc_stains.max(1));
    let half = to_radians(ARTERIAL_ARC_DEG);
    let seed = site_seed(w.at, PatternClass::ArterialSpurt) ^ age;

    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        // Evenly spaced across the arc, then jittered inside its own slot — even spacing alone reads
        // as a comb, pure jitter alone clumps and stops reading as an arc at all.
        let centred = if n > 1 { i as f32 / (n as f32 - 1.0) - 0.5 } else { 0.0 };
        let jitter = (hash_f32(seed ^ i.wrapping_mul(0x85EB_CA6B)) - 0.5) * (half / n as f32);
        let theta = centred * 2.0 * half + jitter;
        let dir = vec::normalize_or_zero(vec::add(
            vec::scale(axis, m::cos(theta)),
            vec::scale(tangent, m::sin(theta)),
        ));
        // Speed carries the pressure, and the droplets nearest the arc's centre carry the most — a
        // jet is fastest along its own axis.
        let along = 1.0 - 0.25 * m::abs(centred) * 2.0;
        out.push(Droplet {
            dir,
            speed: ARTERIAL_SPEED * pressure * along * s.spatter_speed_scale,
            diameter: ARTERIAL_DROPLET_M,
        });
    }
    out
}

/// **Cast-off**: blood flung from a moving tip, released **tangentially** to its path.
///
/// # Two measured facts, and the folklore they replace
///
/// Williams et al., *"Blood drop release from swinging objects"*, J. Forensic Sci. 65(1),
/// `doi:10.1111/1556-4029.13855`, establish that release is **centripetal in origin and tangential in
/// direction** — the drop leaves along the tip's instantaneous velocity, *not* radially outward from
/// the swing centre. "Centrifugal cast-off" is the folklore, and a radial release paints an arc with
/// the wrong orientation everywhere except at the swing's extremes.
///
/// Adam, *"Release of blood droplets from a weapon tip"*,
/// `doi:10.1016/j.forsciint.2019.109934`, measures the pendant volume: a tip carries at most about
/// 150 µL, which is [`BloodSettings::cast_off_max_ml`]. So a knife swung repeatedly sheds *less each
/// time*, and this function decrements `load_ml` to make that true.
///
/// Diameter is **inversely** proportional to tangential speed: a faster tip breaks the ligament
/// earlier and sheds finer drops.
///
/// `hz` is the caller's fixed-tick rate, required to turn a per-tick displacement into a speed in m/s
/// — the units every constant here is quoted in.
pub fn cast_off(
    tip_prev: V3,
    tip_now: V3,
    load_ml: &mut f32,
    hz: u32,
    s: &BloodSettings,
) -> Vec<Droplet> {
    let delta = vec::sub(tip_now, tip_prev);
    let step = vec::length(delta);
    let rate = if hz == 0 { 60.0 } else { hz as f32 };
    let v = step * rate;
    if !v.is_finite() || v < CAST_OFF_MIN_V || !(*load_ml > 0.0) {
        return Vec::new();
    }
    let dir = vec::normalize_or_zero(delta);
    if dir == vec::ZERO {
        return Vec::new();
    }

    // Inversely proportional to tangential speed, clamped so an absurd swing cannot ask for a droplet
    // smaller than the model's own resolution or larger than the pendant drop itself.
    let diameter =
        (s.cast_off_d_ref * (s.cast_off_v_ref / v).clamp(0.25, 4.0)).clamp(s.droplet_size_min, s.droplet_size_max);
    // Sphere volume in millilitres: `π/6 · d³` m³, and 1 m³ is 1e6 ml.
    let per_drop_ml = (PI / 6.0) * diameter * diameter * diameter * 1.0e6;
    if !(per_drop_ml > 0.0) {
        return Vec::new();
    }

    // The pendant cap: however much blood is on the object, only what the tip can hold is available
    // to shed this tick.
    let available = load_ml.min(s.cast_off_max_ml);
    let count = (available / per_drop_ml) as u32;
    if count == 0 {
        return Vec::new();
    }
    let count = count.min(s.max_droplets_per_wound);
    *load_ml = (*load_ml - count as f32 * per_drop_ml).max(0.0);

    let seed = site_seed(tip_now, PatternClass::CastOff);
    let mut out = Vec::with_capacity(count as usize);
    let (tangent, bitangent) = plane_basis(dir);
    for i in 0..count {
        // A narrow spread about the tangent: the ligament breaks in a plane, not a point, so drops
        // leave within a few degrees of the path rather than exactly along it.
        let key = seed ^ i.wrapping_mul(0x9E37_79B9);
        let a = (hash_f32(key) - 0.5) * 0.14;
        let bt = (hash_f32(key ^ 0xC2B2_AE35) - 0.5) * 0.14;
        let jittered = vec::normalize_or_zero(vec::add(
            dir,
            vec::add(vec::scale(tangent, a), vec::scale(bitangent, bt)),
        ));
        out.push(Droplet { dir: jittered, speed: v, diameter });
    }
    out
}

/// **Expirated blood**: a fine mist, and a bubble-ring count that is usually zero.
///
/// Donaldson et al., *"Expirated bloodstain pattern formation"*,
/// `doi:10.1007/s00414-010-0498-5`, report the two facts this function is shaped around: rings occur
/// only in stains **larger than about 3 mm**, and only about **20 % of expirated patterns show them at
/// all**. So the second return value is usually `0`, deliberately — a generator that put a bubble ring
/// on every expirated stain would be drawing a diagnostic feature four times more often than it
/// occurs, and an analyst-legible pattern would become a decoration.
///
/// `dir` is the breath direction; the mist leaves in a wide cone about it. Required rather than
/// assumed, because "which way was the victim facing" is exactly the fact a scene records and no
/// default axis could stand in for.
pub fn expirated(
    dir: V3,
    volume_ml: f32,
    impulse: f32,
    seed: u32,
    s: &BloodSettings,
) -> (Vec<Droplet>, u8) {
    if !(volume_ml > 0.0) || !volume_ml.is_finite() || !impulse.is_finite() || impulse <= 0.0 {
        return (Vec::new(), 0);
    }
    let axis = vec::normalize_or_zero(dir);
    if axis == vec::ZERO {
        return (Vec::new(), 0);
    }
    let (tangent, bitangent) = plane_basis(axis);

    // **Polydisperse, and that is what makes the bubble ring reachable at all.** An expirated pattern
    // is a fine mist with a tail of larger drops, not a monodisperse spray: Donaldson et al. report
    // rings only in the stains above 3 mm, which are made by the tail. A monodisperse mist at the
    // median diameter can never produce one, so it would have made a measured feature unreachable.
    let per_drop_ml =
        (PI / 6.0) * EXPIRATED_DROPLET_M * EXPIRATED_DROPLET_M * EXPIRATED_DROPLET_M * 1.0e6;
    let count = ((volume_ml / per_drop_ml) as u32).min(s.max_droplets_per_wound);
    let cone = to_radians(EXPIRATED_CONE_DEG);

    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let key = seed ^ i.wrapping_mul(0x9E37_79B9);
        let phi = core::f32::consts::TAU * hash_f32(key);
        // `√u` for a distribution uniform per unit solid angle rather than piled at the rim — the
        // same correction the impact cone makes, for the same reason.
        let theta = cone * m::sqrt(hash_f32(key ^ 0x85EB_CA6B));
        let d = vec::normalize_or_zero(vec::add(
            vec::scale(axis, m::cos(theta)),
            vec::scale(
                vec::add(vec::scale(tangent, m::cos(phi)), vec::scale(bitangent, m::sin(phi))),
                m::sin(theta),
            ),
        ));
        // Size, skewed small: `h²` puts most of the mass in the fine mist and leaves a thin tail of
        // larger drops, which is the shape a photographed expirated pattern has.
        let h = hash_f32(key ^ 0x27D4_EB2F);
        let diameter = EXPIRATED_DROPLET_M * (0.6 + 1.9 * h * h);
        // A breath's droplets are slow and air-carried; the impulse sets the spread of speeds.
        let sp = impulse * (0.5 + 0.5 * hash_f32(key ^ 0xC2B2_AE35));
        out.push(Droplet { dir: d, speed: sp, diameter });
    }

    // Rings: only in the minority of patterns, and only when the stains are big enough to hold one.
    // The size test reads the **largest** stain the mist can leave, through the same morphology model
    // a renderer will use — so "bigger than 3 mm" means the same thing here and on screen.
    let shows_rings = hash_f32(seed ^ 0x1656_67B1) < s.expirated_ring_fraction;
    let biggest = out.iter().map(|d| d.diameter).fold(0.0f32, f32::max);
    let largest_stain = crate::bloodstain::stain::stain_shape(
        &crate::bloodstain::stain::Impact {
            speed: impulse,
            diameter: biggest,
            angle_rad: PI * 0.5,
            roughness: s.substrate_roughness,
            // A mist droplet falling onto a floor has no measured in-plane direction of its own.
            travel: [0.0, 0.0],
        },
        s,
        seed,
    );
    let stain_mm = largest_stain.major * 1000.0;
    let rings = if shows_rings && stain_mm >= s.expirated_ring_min_mm {
        // A handful, scaled by how much air drove it. Never the whole stain count.
        let n = m::round(1.0 + 5.0 * hash_f32(seed ^ 0x27D4_EB2F));
        n.clamp(1.0, 8.0) as u8
    } else {
        0
    };

    (out, rings)
}

/// **A drip trail**: free-falling drops from a source in motion, spaced by how fast it moved.
///
/// Spacing is [`BloodSettings::drip_spacing_ref`] metres per m/s, so a walk leaves drips a hand's
/// width apart and a run leaves them a stride apart. **That spacing is the story** — an analyst reads
/// speed off it, and so does a player.
///
/// Each drip costs [`DRIP_ML`] out of `load_ml`, so a trail **ends** rather than continuing forever.
pub fn drip_trail(
    from: V3,
    to: V3,
    speed: f32,
    load_ml: &mut f32,
    s: &BloodSettings,
) -> Vec<Stain> {
    let path = vec::sub(to, from);
    let dist = vec::length(path);
    if !(dist > 0.0) || !dist.is_finite() || !(*load_ml > 0.0) || !speed.is_finite() {
        return Vec::new();
    }
    let spacing = (s.drip_spacing_ref * speed.max(0.0)).max(s.drip_spacing_ref * 0.1);
    let n = (dist / spacing) as u32;
    if n == 0 {
        return Vec::new();
    }

    // Sized once from the drip's own diameter and fall speed, through the same `stain_radius` the
    // spatter model uses — so a drip and a spatter droplet of one size stain the same width, which is
    // the property that keeps a trail and a spray on one scale.
    let drop = Droplet { dir: [0.0, -1.0, 0.0], speed: DRIP_FALL_SPEED, diameter: DRIP_DIAMETER_M };
    let radius = stain_radius(&drop, DRIP_FALL_SPEED, s);

    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        if *load_ml < DRIP_ML {
            break;
        }
        *load_ml -= DRIP_ML;
        let t = (i as f32 + 1.0) / n as f32;
        let at = vec::add(from, vec::scale(path, t));
        out.push(Stain { at, radius, seed: site_seed(at, PatternClass::DripTrail) });
    }
    out
}

/// **A transfer stain**: a wet surface touching a dry one, moving blood out of a finite load.
///
/// [`BloodSettings::transfer_rate`] millilitres leave the load per contact, so **a dragged body runs
/// out of blood** — the smear fades along the drag and then stops, which is the whole reason the
/// budget is conserved rather than assumed infinite.
///
/// `None` when there is nothing left to transfer. Not a zero-radius stain: "nothing was transferred"
/// and "something was transferred that happens to be tiny" are different facts, and a caller should
/// not have to tell them apart by inspecting a float.
pub fn transfer(
    contact: V3,
    tangent: V3,
    load_ml: &mut f32,
    s: &BloodSettings,
) -> Option<Stain> {
    if !(*load_ml > 0.0) || !load_ml.is_finite() {
        return None;
    }
    let moved = load_ml.min(s.transfer_rate.max(0.0));
    if !(moved > 0.0) {
        return None;
    }
    *load_ml = (*load_ml - moved).max(0.0);

    // A smear's width comes from how much blood was actually available: the last contact of a
    // drag leaves a fraction of the first one's mark. Normalised against the per-contact rate, so a
    // full load reads at the authored maximum and a nearly-empty one at the minimum.
    let frac = (moved / s.transfer_rate.max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    let radius = s.stain_radius_min + (s.stain_radius_max - s.stain_radius_min) * frac;
    // The tangent is carried into the seed rather than the radius: two contacts at one point moving
    // in different directions are different stains, and a smear's silhouette comes from
    // `stain::stain_shape` with the tangent as its direction.
    let dir_key = site_seed(vec::add(contact, vec::normalize_or_zero(tangent)), PatternClass::Transfer);
    Some(Stain { at: contact, radius, seed: dir_key })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bloodstain::WoundKind;

    fn wound() -> Wound {
        Wound {
            at: [0.0, 1.4, 0.0],
            normal: vec::X,
            area: 0.004,
            severity: 1.0,
            kind: WoundKind::Severance,
        }
    }

    /// **An arterial arc is an arc, not a cone.** Measured as the ratio of spread across the arc
    /// plane to spread out of it: a cone would be roughly isotropic, an arc is flat.
    #[test]
    fn an_arterial_spurt_is_a_flat_arc_not_a_cone() {
        let s = BloodSettings::default();
        let b = Bleed::new(0, &wound());
        let w = wound();
        let phase = bleed::pulse_phase(&b, 60, &s);
        let arc = arterial_arc(&w, &b, phase, 60, &s);
        assert_eq!(arc.len(), s.arc_stains as usize, "one arc per systole, at the authored count");

        let axis = vec::normalize_or_zero(w.normal);
        let (tangent, bitangent) = plane_basis(axis);
        let in_plane =
            arc.iter().map(|d| m::abs(vec::dot(d.dir, tangent))).fold(0.0f32, f32::max);
        let out_of_plane =
            arc.iter().map(|d| m::abs(vec::dot(d.dir, bitangent))).fold(0.0f32, f32::max);
        assert!(
            in_plane > 0.1 && out_of_plane < 1.0e-5,
            "the arc must spread in its own plane ({in_plane}) and not out of it ({out_of_plane})"
        );
        for d in &arc {
            assert_eq!(
                d.diameter, ARTERIAL_DROPLET_M,
                "arterial droplets are large and discrete, not a size distribution"
            );
        }
    }

    /// The arc fires **only** on heartbeat ticks, and its reach decays as the body loses pressure.
    #[test]
    fn the_arc_fires_on_the_heartbeat_and_decays() {
        let s = BloodSettings::default();
        let b = Bleed::new(0, &wound());
        let w = wound();
        let period = bleed::pulse_period(60, &s);
        let phase = bleed::pulse_phase(&b, 60, &s);
        if period > 1 {
            assert!(
                arterial_arc(&w, &b, phase + 1, 60, &s).is_empty(),
                "no systole on this tick, so no arc"
            );
        }
        let first = arterial_arc(&w, &b, phase, 60, &s);
        // Sampled while the wound is still flowing: past the arrest there is no arc to compare, and a
        // comparison against an empty arc would pass for the wrong reason.
        let later = arterial_arc(&w, &b, period * 4 + phase, 60, &s);
        let peak = |v: &[Droplet]| v.iter().map(|d| d.speed).fold(0.0f32, f32::max);
        assert!(
            !later.is_empty() && peak(&later) < peak(&first),
            "arterial reach must fall as pressure is lost: {} then {}",
            peak(&first),
            peak(&later)
        );
        // And it stops for good by the same predicate every other pattern uses.
        let dead = arterial_arc(&w, &b, s.clot_ticks + period, 60, &s);
        assert!(dead.is_empty(), "a clotted artery does not spurt");
    }

    /// **Cast-off is tangential, not radial** — the fact Williams et al. established and the folklore
    /// this asserts against. A radial release would point away from the swing centre.
    #[test]
    fn cast_off_releases_along_the_path_not_outward() {
        let s = BloodSettings::default();
        let mut load = 0.5f32;
        // A tip at radius 1 on +X, moving in +Z: tangential is +Z, radial would be +X.
        let prev = [1.0, 1.0, 0.0];
        let now = [1.0, 1.0, 0.1];
        let drops = cast_off(prev, now, &mut load, 60, &s);
        assert!(!drops.is_empty(), "a 6 m/s tip must shed");
        for d in &drops {
            assert!(
                d.dir[2] > 0.9,
                "a droplet left at {:?}, which is not along the tip's path",
                d.dir
            );
            assert!(d.dir[0] < 0.2, "a droplet left radially, which is the folklore this refuses");
        }
    }

    /// Faster tip, finer droplets — the inverse size law — and a tip that barely moves sheds nothing.
    #[test]
    fn a_faster_tip_sheds_finer_droplets_and_a_slow_one_sheds_none() {
        let s = BloodSettings::default();
        let dia = |step: f32| {
            let mut load = 1.0f32;
            cast_off([0.0, 1.0, 0.0], [0.0, 1.0, step], &mut load, 60, &s)
                .first()
                .map(|d| d.diameter)
        };
        let slow = dia(0.06).expect("3.6 m/s must shed");
        let fast = dia(0.30).expect("18 m/s must shed");
        assert!(fast < slow, "a faster tip must shed finer droplets: {slow} then {fast}");

        let mut load = 1.0f32;
        assert!(
            cast_off([0.0, 1.0, 0.0], [0.0, 1.0, 0.01], &mut load, 60, &s).is_empty(),
            "a 0.6 m/s move must not shed — surface tension keeps the drop"
        );
        assert_eq!(load, 1.0, "a swing that sheds nothing must not spend the load");
    }

    /// **A swung weapon runs out of blood**, which is the conservation the whole module rests on.
    #[test]
    fn a_repeatedly_swung_tip_sheds_less_each_time() {
        let s = BloodSettings::default();
        let mut load = 0.30f32;
        let mut counts = std::vec::Vec::new();
        for k in 0..40u32 {
            let z = k as f32 * 0.2;
            let drops = cast_off([0.0, 1.0, z], [0.0, 1.0, z + 0.2], &mut load, 60, &s);
            counts.push(drops.len());
            if drops.is_empty() {
                break;
            }
        }
        assert!(counts.len() > 1, "the first swings must shed something");
        assert_eq!(
            *counts.last().unwrap_or(&1),
            0,
            "a finite load must eventually shed nothing at all"
        );
        // Spent down to less than one droplet's worth. **Not to exactly zero, and that is correct**:
        // blood leaves a tip in whole drops, so a remainder smaller than the drop the current swing
        // speed would shed is blood that physically cannot detach.
        assert!(load < 0.005, "the load must be spent down to under one droplet, got {load}");
    }

    /// Bubble rings are **rare**: over many patterns, the fraction showing them tracks the measured
    /// 20 %, and none appears in a stain too small to hold one.
    #[test]
    fn bubble_rings_are_the_minority_they_are_measured_to_be() {
        let s = BloodSettings::default();
        let mut with_rings = 0usize;
        let n = 2000usize;
        for i in 0..n {
            let (_, rings) = expirated(vec::X, 0.5, 4.0, i as u32, &s);
            if rings > 0 {
                with_rings += 1;
            }
        }
        let frac = with_rings as f32 / n as f32;
        assert!(
            (frac - s.expirated_ring_fraction).abs() < 0.05,
            "rings appeared in {:.1} % of patterns, but the measurement is {:.0} %",
            frac * 100.0,
            s.expirated_ring_fraction * 100.0
        );

        // A gentle breath makes stains below the 3 mm ring threshold, so it can never show one.
        for i in 0..200u32 {
            let (_, rings) = expirated(vec::X, 0.2, 0.05, i, &s);
            assert_eq!(rings, 0, "a stain below the ring threshold cannot carry a ring");
        }
    }

    /// Expirated blood is a fine mist in a wide cone — the two things that distinguish it from impact
    /// spatter at a glance.
    #[test]
    fn expirated_blood_is_a_fine_wide_mist() {
        let s = BloodSettings::default();
        let (mist, _) = expirated(vec::X, 1.0, 3.0, 11, &s);
        assert!(!mist.is_empty(), "a millilitre of blood in air must make a mist");
        let cone = m::cos(to_radians(EXPIRATED_CONE_DEG));
        for d in &mist {
            // Polydisperse, with a tail: no droplet may be coarse, and the distribution must still be
            // *mostly* fine, which is checked below rather than per droplet.
            assert!(d.diameter < 0.0025, "an expirated droplet was coarse: {}", d.diameter);
            assert!(
                vec::dot(d.dir, vec::X) >= cone - 1.0e-4,
                "a droplet left outside the breath cone"
            );
        }
        let fine = mist.iter().filter(|d| d.diameter < 0.0012).count();
        assert!(
            fine * 2 > mist.len(),
            "the mist must be mostly fine — {fine} of {} droplets were",
            mist.len()
        );
        let spread = mist.iter().map(|d| m::abs(d.dir[1])).fold(0.0f32, f32::max);
        assert!(spread > 0.3, "the mist must actually be wide, got a spread of {spread}");
    }

    /// **Drip spacing encodes speed**, and the trail ends when the blood does.
    #[test]
    fn drip_spacing_tracks_speed_and_the_trail_runs_out() {
        let s = BloodSettings::default();
        let count = |speed: f32, load: f32| {
            let mut l = load;
            drip_trail([0.0, 0.0, 0.0], [4.0, 0.0, 0.0], speed, &mut l, &s).len()
        };
        let walking = count(1.0, 100.0);
        let running = count(4.0, 100.0);
        assert!(
            walking > running,
            "a slower walk must drip more often over the same path: {walking} then {running}"
        );

        let mut load = 3.0f32 * DRIP_ML;
        let trail = drip_trail([0.0, 0.0, 0.0], [40.0, 0.0, 0.0], 1.0, &mut load, &s);
        assert_eq!(trail.len(), 3, "a three-drip load must leave exactly three drips");
        assert!(load < DRIP_ML, "the load must be spent");
        for pair in trail.windows(2) {
            assert!(pair[1].at[0] > pair[0].at[0], "drips must advance along the path");
            assert_ne!(pair[0].seed, pair[1].seed, "two drips must not share a seed");
        }
    }

    /// **A dragged body runs out of blood.** The smear fades and then stops, from a conserved budget.
    #[test]
    fn a_transfer_smear_fades_and_then_stops() {
        let s = BloodSettings::default();
        let mut load = s.transfer_rate * 3.5;
        let mut radii = std::vec::Vec::new();
        for k in 0..10 {
            match transfer([k as f32 * 0.1, 0.0, 0.0], vec::X, &mut load, &s) {
                Some(st) => radii.push(st.radius),
                None => break,
            }
        }
        assert_eq!(radii.len(), 4, "3.5 contacts' worth of blood must leave four marks");
        assert!(
            radii[3] < radii[0],
            "the last mark of a drag must be smaller than the first: {:?}",
            radii
        );
        assert!(transfer([1.0, 0.0, 0.0], vec::X, &mut load, &s).is_none(), "spent means spent");
    }

    /// Impact spatter is the percolation cone and nothing else — the delegation is the contract, so
    /// this asserts there is no second cone.
    #[test]
    fn impact_spatter_is_the_percolation_cone() {
        let s = BloodSettings::default();
        let w = wound();
        assert_eq!(impact_spatter(&w, &s), droplets(&w, &s), "a second cone appeared");
    }
}
