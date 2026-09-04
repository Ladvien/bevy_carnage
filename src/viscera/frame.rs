//! One orthonormal frame builder, shared by the spill fan and the tube sweep.
//!
//! Both need "a pair of axes perpendicular to this direction", and both need the *same* pair every
//! run. Deriving it twice would give two chances to disagree.

use bevy::math::Vec3;

/// The world axis least aligned with `t`, chosen by smallest absolute component.
///
/// This is what keeps the tube frame from flipping between runs: the cross product below is at worst
/// `√(2/3) ≈ 0.816` long, so it is never near-degenerate, and the choice is a total function of `t`
/// with ties broken in a fixed X-then-Y-then-Z order rather than by whatever the optimiser did.
#[inline]
pub(crate) fn least_aligned_axis(t: Vec3) -> Vec3 {
    let a = t.abs();
    if a.x <= a.y && a.x <= a.z {
        Vec3::X
    } else if a.y <= a.z {
        Vec3::Y
    } else {
        Vec3::Z
    }
}

/// A right-handed orthonormal pair perpendicular to the unit vector `t`.
///
/// Returns `(u, v)` with `u × v` pointing along `t`.
#[inline]
pub(crate) fn perpendicular_basis(t: Vec3) -> (Vec3, Vec3) {
    let reference = least_aligned_axis(t);
    let u = t.cross(reference).normalize_or_zero();
    // `t` is unit and `reference` is at least 54° from it, so `u` is unit and this cross is too.
    let v = t.cross(u);
    (u, v)
}
