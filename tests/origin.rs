//! **The forward model is physically invertible.** The strongest statement this crate can make about
//! itself, and one no golden can make.
//!
//! A frozen table says two runs agree. This says the *physics* is right in the forward direction:
//! blood thrown from a known point, landed by [`bevy_carnage::bloodstain::stain::stains`], read back through the
//! published bloodstain-pattern-analysis method, arrives at the point it was thrown from. If the
//! impact-angle relation were wrong, or the landing solver's angle were wrong, or the stain's travel
//! direction were fabricated rather than measured, this test would fail and every golden would still
//! be green.

use bevy_carnage::bloodstain::origin::{Landing, area_of_origin};
use bevy_carnage::bloodstain::stain::{impact_at_plane, stain_shape};
use bevy_carnage::bloodstain::{BloodSettings, Wound, WoundKind, droplet, droplet_count, landing};

/// The wound the whole crate's goldens describe, raised so it throws a wide field of stains.
fn wound() -> Wound {
    Wound {
        at: [0.35, 1.15, -0.20],
        normal: [0.0, 1.0, 0.0],
        area: 0.30,
        severity: 1.0,
        kind: WoundKind::Severance,
    }
}

/// Distance between two points, spelled out rather than reaching for a math library the crate
/// deliberately does not depend on.
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Every droplet of one wound that reached the floor, as the solver wants it.
fn scene(s: &BloodSettings) -> Vec<Landing> {
    let w = wound();
    let n = droplet_count(&w, s);
    let mut out = Vec::new();
    for i in 0..n {
        let d = droplet(&w, i, s);
        let Some(at) = landing(w.at, &d, s.gravity, 0.0) else {
            continue;
        };
        let impact = impact_at_plane(&d, w.at, 0.0, s);
        let shape = stain_shape(&impact, s, i);
        out.push(Landing { at, shape, impact_speed: impact.speed });
    }
    out
}

/// **Two hundred stains recover the wound to within five centimetres.**
///
/// The tolerance is the plan's, and it is a real one: the wound sits 1.15 m up and its stains land
/// metres away, so 5 cm is well under a per-cent error on the geometry being inverted.
#[test]
fn the_solver_recovers_the_wound_from_the_stains_it_threw() {
    // A slower spray, so the field lands on a floor rather than leaving the building — the same
    // reason both of this crate's reference demos set it.
    let s = BloodSettings { spatter_speed_scale: 0.25, ..Default::default() };
    let scene = scene(&s);
    assert!(
        scene.len() >= 200,
        "the fixture must land at least 200 stains to be the test the plan asks for, got {}",
        scene.len()
    );

    let got = area_of_origin(&scene, s.gravity).expect("hundreds of stains must determine an origin");
    let err = dist(got, wound().at);
    assert!(
        err < 0.05,
        "the solver placed the wound at {got:?}, {err:.4} m from the true {:?}. The forward model \
         is not invertible by the published method, which means the impact-angle relation, the \
         landing solver's angle, or the stain's travel direction is wrong.",
        wound().at
    );
}

/// **Half the stains are enough**, which is what makes this a solver rather than a lookup: a real
/// scene never has every stain, and an analyst works from what survived.
#[test]
fn a_partial_scene_still_locates_the_wound() {
    let s = BloodSettings { spatter_speed_scale: 0.25, ..Default::default() };
    let full = scene(&s);
    let half: Vec<Landing> = full.iter().step_by(2).copied().collect();
    assert!(half.len() > 20, "precondition: the half-scene is still a scene");
    let got = area_of_origin(&half, s.gravity).expect("half a scene must still solve");
    let err = dist(got, wound().at);
    assert!(err < 0.08, "half the stains placed the wound {err:.4} m out, at {got:?}");
}

/// The solver is a pure function of the stains: shuffling the scene cannot move the answer. Reading
/// an origin off an ECS query order would be exactly this bug.
#[test]
fn the_answer_does_not_depend_on_the_order_of_the_stains() {
    let s = BloodSettings { spatter_speed_scale: 0.25, ..Default::default() };
    let forward = scene(&s);
    let mut reversed = forward.clone();
    reversed.reverse();
    let a = area_of_origin(&forward, s.gravity).expect("solves");
    let b = area_of_origin(&reversed, s.gravity).expect("solves");
    // Least squares sums in a different order, so this is an equality up to float summation rather
    // than bit-identity — a millimetre, over a scene metres across.
    assert!(
        dist(a, b) < 0.001,
        "reversing the stain list moved the answer from {a:?} to {b:?}"
    );
}
