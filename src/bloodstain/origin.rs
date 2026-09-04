//! **The inverse solver.** Given stains, where was the wound?
//!
//! This is the strongest test the crate can have on itself. A hash proves two runs agree; **this
//! proves the forward model is physically invertible by a published method.** If
//! [`area_of_origin`] cannot recover a wound from the stains [`crate::bloodstain::stain::stains`] produced, then
//! the impact-angle relation is wrong in the forward direction and no golden would have said so.
//!
//! It is also a gameplay tool. An investigation scene, a detective mechanic, a forensics minigame —
//! all of them want exactly this function, and it comes free with modelling the forward direction
//! honestly.
//!
//! # The method, and why it needs the impact speed
//!
//! Classical bloodstain-pattern analysis back-projects each stain along its own long axis at its own
//! impact angle and intersects the lines. The **horizontal** part of that is exact: a ballistic
//! droplet's whole flight lies in one vertical plane, so the azimuth of the stain's long axis points
//! at the origin's ground position no matter what gravity does. The **height** is where the classical
//! "tangent method" is known to overestimate, because a straight line is not a parabola.
//!
//! The correction is exact, and deriving it is what shows a stain's *shape alone* cannot give a
//! height. For a droplet that left an origin at height `h₀` and landed a horizontal distance `R` away
//! with flight time `t`, impact angle `θ` and horizontal speed `vₓ`:
//!
//! > `vy = ½ g t − h₀ / t`  (from `h₀ = ½ g t² − vy t`)
//! >
//! > `tan θ = t (g t − vy) / R`, and substituting: **`R tan θ = ½ g t² + h₀`**
//!
//! So `h₀ = R tan θ − ½ g t²` with `t` free. **The angle and the landing point alone leave `t`
//! undetermined** — every height is consistent with some flight time — which is why this function
//! takes the impact speed and the plan's two-field signature could not work. `t = R / (v cos θ)`
//! closes it, and the height becomes exact rather than an over-estimate to be fudged.


use crate::bloodstain::stain::StainShape;
use crate::bloodstain::{V3, m};

/// One landed stain, as the solver needs it.
///
/// A named struct rather than a tuple because the third field is the one whose *absence* makes the
/// problem unsolvable, and an unlabelled `f32` in a tuple is exactly how it would go missing.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Landing {
    /// Where it landed, on the plane.
    pub at: V3,
    /// Its silhouette — the impact angle and the travel direction are read out of this.
    pub shape: StainShape,
    /// Impact speed, m/s. See the module docs for the derivation that makes this necessary.
    pub impact_speed: f32,
}

/// **Where the blood came from**, or `None` when the stains cannot say.
///
/// Two passes, in this order:
///
/// 1. **Horizontal, by least squares.** Each stain's long axis, projected onto the plane, is a line
///    through the origin's ground position. The point minimising the squared perpendicular distance to
///    all of them is the closed-form solution of a 2×2 system.
/// 2. **Height, exactly.** `h₀ = R tan θ − ½ g t²` per stain, averaged. Not a correction factor on the
///    tangent method — the tangent method's own error term, evaluated.
///
/// `None` rather than a guess when: fewer than two stains; every stain's axis is parallel, so the
/// lines never intersect and the system is singular; or the arithmetic leaves the finite numbers.
/// Each of those is a scene that genuinely does not determine an origin, and returning a plausible
/// point for it would be the fabricated answer this crate refuses everywhere else.
pub fn area_of_origin(stains: &[Landing], gravity: f32) -> Option<V3> {
    if stains.len() < 2 || !gravity.is_finite() {
        return None;
    }

    // ---- Pass 1: the ground position, by least squares over the azimuth lines. -----------------
    //
    // For a line through `p` with unit direction `u`, the squared distance from a point `x` is
    // `|(I − u uᵀ)(x − p)|²`. Summing and differentiating gives `A x = b` with
    // `A = Σ(I − u uᵀ)` and `b = Σ(I − u uᵀ) p`, which in 2-D is a 2×2 solve.
    let mut a00 = 0.0f32;
    let mut a01 = 0.0f32;
    let mut a11 = 0.0f32;
    let mut b0 = 0.0f32;
    let mut b1 = 0.0f32;
    let mut plane_y = 0.0f32;
    let mut used = 0usize;

    let mut axes: Vec<(f32, f32, f32, V3)> = Vec::with_capacity(stains.len());
    for l in stains {
        let (dx, dz) = (l.shape.direction[0], l.shape.direction[1]);
        let len = m::sqrt(dx * dx + dz * dz);
        if !(len > 0.0) || !l.at[0].is_finite() || !l.at[2].is_finite() {
            continue;
        }
        let (ux, uz) = (dx / len, dz / len);
        // `I − u uᵀ` for a 2-D unit vector.
        let (p00, p01, p11) = (1.0 - ux * ux, -ux * uz, 1.0 - uz * uz);
        a00 += p00;
        a01 += p01;
        a11 += p11;
        b0 += p00 * l.at[0] + p01 * l.at[2];
        b1 += p01 * l.at[0] + p11 * l.at[2];
        plane_y += l.at[1];
        used += 1;
        axes.push((ux, uz, l.shape.impact_angle(), l.at));
    }
    if used < 2 {
        return None;
    }
    let det = a00 * a11 - a01 * a01;
    // Singular means every axis is parallel: the lines are a pencil with no intersection, and the
    // stains genuinely do not determine a ground position. The threshold is scaled by the number of
    // stains, because `A` grows with it.
    if !det.is_finite() || m::abs(det) < 1.0e-6 * used as f32 {
        return None;
    }
    let x = (b0 * a11 - b1 * a01) / det;
    let z = (b1 * a00 - b0 * a01) / det;
    if !x.is_finite() || !z.is_finite() {
        return None;
    }
    let plane_y = plane_y / used as f32;

    // ---- Pass 2: the height, from the parabola rather than from the tangent. --------------------
    let mut height_sum = 0.0f32;
    let mut height_n = 0usize;
    for (l, (_, _, theta, at)) in stains.iter().zip(&axes) {
        let (rx, rz) = (at[0] - x, at[2] - z);
        let r = m::sqrt(rx * rx + rz * rz);
        let cos_t = m::cos(*theta);
        let tan_t = m::sin(*theta) / cos_t;
        if !(r > 0.0) || !(cos_t > 1.0e-3) || !(l.impact_speed > 0.0) || !tan_t.is_finite() {
            continue;
        }
        let vx = l.impact_speed * cos_t;
        let t = r / vx;
        let h = r * tan_t - 0.5 * gravity * t * t;
        if !h.is_finite() {
            continue;
        }
        height_sum += h;
        height_n += 1;
    }
    if height_n == 0 {
        return None;
    }
    let h = height_sum / height_n as f32;

    Some([x, plane_y + h, z])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bloodstain::stain::{Impact, stain_shape};
    use crate::bloodstain::{BloodSettings, vec};

    /// Fabricate a landing from a known origin and a known launch, so the test knows the answer it
    /// is asking for. Mirrors the forward model's own closed form.
    fn thrown(origin: V3, azimuth: f32, speed: f32, elevation: f32, gravity: f32) -> Option<Landing> {
        let s = BloodSettings::default();
        let (ax, az) = (m::cos(azimuth), m::sin(azimuth));
        let vx = speed * m::cos(elevation);
        let vy = speed * m::sin(elevation);
        // Flight to y = 0 from `origin[1]`.
        let disc = vy * vy + 2.0 * gravity * origin[1];
        if disc < 0.0 {
            return None;
        }
        let t = (vy + m::sqrt(disc)) / gravity;
        if !(t > 0.0) {
            return None;
        }
        let r = vx * t;
        let at = [origin[0] + ax * r, 0.0, origin[2] + az * r];
        let down = gravity * t - vy;
        let impact_speed = m::sqrt(vx * vx + down * down);
        let angle = m::atan2(down, vx);
        let shape = stain_shape(
            &Impact {
                speed: impact_speed,
                diameter: 0.004,
                angle_rad: angle,
                roughness: 0.0,
                travel: [ax, az],
            },
            &s,
            7,
        );
        Some(Landing { at, shape, impact_speed })
    }

    /// **The solver recovers a known wound to within a centimetre.** This is the crate's strongest
    /// self-check: it says the forward model is invertible by the published method, which no hash can.
    #[test]
    fn the_solver_recovers_a_known_wound() {
        let gravity = 18.0f32;
        let truth: V3 = [0.35, 1.15, -0.20];
        let mut landings = Vec::new();
        for i in 0..200u32 {
            let azimuth = core::f32::consts::TAU * (i as f32 / 200.0);
            // A spread of launch elevations and speeds, so the sample is not one special trajectory.
            let elevation = crate::bloodstain::to_radians(-10.0 + 40.0 * crate::bloodstain::hash_f32(i));
            let speed = 4.0 + 4.0 * crate::bloodstain::hash_f32(i ^ 0x5EED);
            if let Some(l) = thrown(truth, azimuth, speed, elevation, gravity) {
                landings.push(l);
            }
        }
        assert!(landings.len() > 150, "the fixture must actually land most of its droplets");
        let got = area_of_origin(&landings, gravity).expect("200 stains must determine an origin");
        let err = vec::length(vec::sub(got, truth));
        assert!(
            err < 0.01,
            "the solver placed the wound at {got:?}, {:.3} m from the true {truth:?}",
            err
        );
    }

    /// **The gravity term is load-bearing.** Without it the classical tangent method overestimates
    /// the height, and this measures by how much — so nobody can delete the correction and still see
    /// green.
    #[test]
    fn the_tangent_method_alone_would_overestimate_the_height() {
        let gravity = 18.0f32;
        let truth: V3 = [0.0, 1.2, 0.0];
        let mut landings = Vec::new();
        for i in 0..120u32 {
            let azimuth = core::f32::consts::TAU * (i as f32 / 120.0);
            if let Some(l) = thrown(truth, azimuth, 6.0, crate::bloodstain::to_radians(5.0), gravity) {
                landings.push(l);
            }
        }
        let with_gravity =
            area_of_origin(&landings, gravity).expect("the fixture must solve").as_slice()[1];
        let without = area_of_origin(&landings, 0.0).expect("the fixture must solve").as_slice()[1];
        assert!(
            (with_gravity - truth[1]).abs() < 0.02,
            "with gravity the height must be right, got {with_gravity}"
        );
        assert!(
            without > truth[1] + 0.1,
            "without gravity the tangent method must visibly overestimate, got {without}"
        );
    }

    /// A scene that does not determine an origin gets `None`, not a plausible-looking point.
    #[test]
    fn an_underdetermined_scene_has_no_origin() {
        let gravity = 18.0f32;
        let one = thrown([0.0, 1.0, 0.0], 0.3, 6.0, 0.1, gravity).expect("fixture");
        assert!(area_of_origin(&[one], gravity).is_none(), "one stain cannot triangulate");

        // Two parallel axes: a pencil of lines with no intersection.
        let mut a = one;
        let mut b = one;
        a.at = [0.0, 0.0, 0.0];
        b.at = [1.0, 0.0, 0.0];
        a.shape.direction = [1.0, 0.0];
        b.shape.direction = [1.0, 0.0];
        assert!(
            area_of_origin(&[a, b], gravity).is_none(),
            "parallel axes must be refused, not averaged into a guess"
        );

        assert!(area_of_origin(&[a, b], f32::NAN).is_none(), "a nonsense gravity must be refused");
    }
}
