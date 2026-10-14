//! **A skinned, animated character, made here so the demo needs no asset.**
//!
//! Every other body in these examples is a stack of cuboids: rigid, and always in the pose it was
//! built in. A game's character is neither. It is a skinned mesh whose asset is a *bind pose* —
//! arms hung at its sides — and a skeleton that carries the vertices somewhere else every frame.
//! This is that, in a few hundred lines: eleven joints, six skinned meshes sharing them (torso, head,
//! two arms, two legs), smooth limbs with one `UV_0` island per tube, two-bone weights blended
//! through the elbows, knees and waist, and a walk that raises the left arm in a wave.
//!
//! **What it is for is the question the cuboid body cannot ask:** when a shot lands on a forearm
//! that is held above the head, does the wound open where the forearm *is*, or where the asset
//! says it would be if the arm were hanging down? And once it is open, does it ride the arm?
//!
//! The character faces `+Z`; its left is `+X`. Joints are bind-pose-axis-aligned, so every joint
//! rotation is a swing about the world axes it hung along: rotation about `X` swings a hanging
//! limb forward (negative) or back, about `Z` out to its side.

use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy_carnage::cross_section::{Region, Scale};
use bevy_carnage::preset::{Gore, GoreBreakable, GoreHit, GoreScene};
use bevy_carnage::{CutSettings, ProxyCell, fracture_mesh};

use super::preset_scene;

/// The character's root: its pose is the one place the whole figure is moved from.
#[derive(Component)]
pub struct Mannequin;

/// A joint, by name.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Joint(pub Bone);

/// One of the six skinned meshes.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part(pub Limb);

/// The eleven joints, in the order the skinned meshes index them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bone {
    Hips,
    Spine,
    Head,
    ArmUpL,
    ArmLowL,
    ArmUpR,
    ArmLowR,
    LegUpL,
    LegLowL,
    LegUpR,
    LegLowR,
}

impl Bone {
    /// Every joint, in index order.
    pub const ALL: [Bone; 11] = [
        Bone::Hips,
        Bone::Spine,
        Bone::Head,
        Bone::ArmUpL,
        Bone::ArmLowL,
        Bone::ArmUpR,
        Bone::ArmLowR,
        Bone::LegUpL,
        Bone::LegLowL,
        Bone::LegUpR,
        Bone::LegLowR,
    ];

    fn parent(self) -> Option<Bone> {
        match self {
            Bone::Hips => None,
            Bone::Spine | Bone::LegUpL | Bone::LegUpR => Some(Bone::Hips),
            Bone::Head | Bone::ArmUpL | Bone::ArmUpR => Some(Bone::Spine),
            Bone::ArmLowL => Some(Bone::ArmUpL),
            Bone::ArmLowR => Some(Bone::ArmUpR),
            Bone::LegLowL => Some(Bone::LegUpL),
            Bone::LegLowR => Some(Bone::LegUpR),
        }
    }

    /// Where the joint is in the bind pose, metres from the root, `+Y` up.
    pub fn bind(self) -> Vec3 {
        match self {
            Bone::Hips => Vec3::new(0.0, 0.95, 0.0),
            Bone::Spine => Vec3::new(0.0, 1.12, 0.0),
            Bone::Head => Vec3::new(0.0, 1.48, 0.0),
            Bone::ArmUpL => Vec3::new(0.235, 1.42, 0.0),
            Bone::ArmLowL => Vec3::new(0.235, 1.13, 0.0),
            Bone::ArmUpR => Vec3::new(-0.235, 1.42, 0.0),
            Bone::ArmLowR => Vec3::new(-0.235, 1.13, 0.0),
            Bone::LegUpL => Vec3::new(0.095, 0.93, 0.0),
            Bone::LegLowL => Vec3::new(0.095, 0.50, 0.0),
            Bone::LegUpR => Vec3::new(-0.095, 0.93, 0.0),
            Bone::LegLowR => Vec3::new(-0.095, 0.50, 0.0),
        }
    }
}

/// The six skinned meshes, each its own `Gore` surface with its own canvases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limb {
    Torso,
    Head,
    ArmL,
    ArmR,
    LegL,
    LegR,
}

impl Limb {
    /// Which thickness row the part's wounds and cut faces use.
    pub fn region(self) -> Region {
        match self {
            Limb::Torso => Region::Torso,
            Limb::Head => Region::Head,
            _ => Region::Limb,
        }
    }
}

// ---- The pose --------------------------------------------------------------------------------

/// **The walk, `t` seconds in**: a rotation for every joint, and how far the hips are lifted. The left
/// arm is held up and waves; the right swings against the left leg; the legs walk in place.
pub fn pose(t: f32) -> ([Quat; 11], f32) {
    let w = TAU * 0.8 * t;
    let (rx, ry, rz) = (Quat::from_rotation_x, Quat::from_rotation_y, Quat::from_rotation_z);
    let step = |phase: f32| -> (Quat, Quat) {
        let swing = -0.42 * (w + phase).sin();
        // The knee bends backward only, and most while the leg is coming forward.
        let knee = 0.08 + 0.85 * (w + phase).cos().max(0.0);
        (rx(swing), rx(knee))
    };
    let (thigh_l, shin_l) = step(0.0);
    let (thigh_r, shin_r) = step(core::f32::consts::PI);
    let mut q = [Quat::IDENTITY; 11];
    q[Bone::Hips as usize] = ry(0.07 * w.sin());
    q[Bone::Spine as usize] = ry(-0.10 * w.sin()) * rx(0.03);
    q[Bone::Head as usize] = ry(0.14 * (0.5 * w).sin()) * rx(-0.04);
    // Raised out to the side and a little above the horizontal, the forearm up and waving.
    q[Bone::ArmUpL as usize] = rz(1.85);
    q[Bone::ArmLowL as usize] = rz(1.05 + 0.34 * (2.9 * t).sin());
    q[Bone::ArmUpR as usize] = rx(0.38 * w.sin()) * rz(-0.06);
    q[Bone::ArmLowR as usize] = rx(-0.28 - 0.18 * w.sin());
    q[Bone::LegUpL as usize] = thigh_l;
    q[Bone::LegLowL as usize] = shin_l;
    q[Bone::LegUpR as usize] = thigh_r;
    q[Bone::LegLowR as usize] = shin_r;
    (q, 0.012 * (2.0 * w).sin().abs())
}

/// **Drive the skeleton from the clock.** One system, on `Update`: it writes joint `Transform`s and
/// nothing else, so Bevy's own propagation carries the pose to every joint's `GlobalTransform` and
/// the renderer carries it to the vertices.
pub fn animate(time: Res<Time>, mut joints: Query<(&Joint, &mut Transform)>) {
    let (rotations, lift) = pose(time.elapsed_secs());
    for (joint, mut tf) in &mut joints {
        tf.rotation = rotations[joint.0 as usize];
        if joint.0 == Bone::Hips {
            tf.translation = Bone::Hips.bind() + Vec3::Y * lift;
        }
    }
}

/// **When [`guard`] starts lowering the forearm**, seconds, and how long it takes to reach the side.
pub const GUARD_DROP: f32 = 1.25;
pub const GUARD_LOWER: f32 = 0.9;

/// **Standing on guard, `t` seconds in**: a rotation for every joint, and how far the hips are lifted.
/// The right forearm is held level across the chest, a hand's breadth in front of it, until
/// [`GUARD_DROP`]; then the arm is lowered to the side over [`GUARD_LOWER`], so whatever is behind
/// the forearm is seen. The body breathes and sways a little, and the legs stand.
///
/// The arm is posed by where its joints go rather than by angles: the elbow below and in front of the
/// shoulder, the wrist across the body, each bone turned the shortest way from hanging, and each
/// joint's rotation read back into its parent's frame.
pub fn guard(t: f32) -> ([Quat; 11], f32) {
    let (rx, ry) = (Quat::from_rotation_x, Quat::from_rotation_y);
    let breath = (TAU * 0.25 * t).sin();
    let hips = ry(0.03 * (TAU * 0.13 * t).sin());
    let spine = rx(0.02 + 0.01 * breath);
    let hang = |d: Vec3| Quat::from_rotation_arc(Vec3::NEG_Y, d.normalize());
    let shoulder = Bone::ArmUpR.bind();
    let (elbow, wrist) = (Vec3::new(-0.19, 1.27, 0.17), Vec3::new(0.10, 1.32, 0.17));
    let u = ((t - GUARD_DROP) / GUARD_LOWER).clamp(0.0, 1.0);
    let s = u * u * (3.0 - 2.0 * u);
    let upper = hang(elbow - shoulder).slerp(hang(Vec3::new(-0.03, -1.0, 0.04)), s);
    let lower = hang(wrist - elbow).slerp(hang(Vec3::new(0.0, -1.0, 0.14)), s);
    let mut q = [Quat::IDENTITY; 11];
    q[Bone::Hips as usize] = hips;
    q[Bone::Spine as usize] = spine;
    q[Bone::Head as usize] = rx(-0.03) * ry(-0.06);
    q[Bone::ArmUpR as usize] = (hips * spine).inverse() * upper;
    q[Bone::ArmLowR as usize] = upper.inverse() * lower;
    q[Bone::ArmUpL as usize] = (hips * spine).inverse() * hang(Vec3::new(0.05, -1.0, 0.03 + 0.01 * breath));
    q[Bone::ArmLowL as usize] = rx(-0.15);
    for (thigh, shin) in [(Bone::LegUpL, Bone::LegLowL), (Bone::LegUpR, Bone::LegLowR)] {
        q[thigh as usize] = rx(-0.04);
        q[shin as usize] = rx(0.08);
    }
    (q, 0.004 * breath)
}

/// **Drive the skeleton through [`guard`] from the clock**, as [`animate`] does the walk.
pub fn guard_animate(time: Res<Time>, mut joints: Query<(&Joint, &mut Transform)>) {
    let (rotations, lift) = guard(time.elapsed_secs());
    for (joint, mut tf) in &mut joints {
        tf.rotation = rotations[joint.0 as usize];
        if joint.0 == Bone::Hips {
            tf.translation = Bone::Hips.bind() + Vec3::Y * lift;
        }
    }
}

/// **How long two steps of [`stroll`] take**, seconds: a slow, deliberate walk.
pub const STROLL_PERIOD: f32 = 1.3;
/// **The stance thigh's sweep either side of upright** in [`stroll`], radians: from `−` at touchdown,
/// the foot ahead, to `+` at toe-off, the foot behind.
pub const STROLL_SWING: f32 = 0.16;
/// Where the first planted mid-sole stands along `+z`, metres from the root.
pub const STROLL_START_Z: f32 = -1.3;
/// The mid-sole under a straight leg, from the hip joint: the point a planted foot keeps still.
const MID_SOLE: Vec3 = Vec3::new(0.0, 0.008 - 0.93, 0.04);
/// The heel's, mid-sole's and ball's undersides from the hip joint: whichever is lowest is on the ground.
const SOLE: [Vec3; 3] = [Vec3::new(0.0, 0.014 - 0.93, -0.06), MID_SOLE, Vec3::new(0.0, 0.02 - 0.93, 0.135)];
/// The knee from the hip joint.
const KNEE: Vec3 = Vec3::new(0.0, 0.50 - 0.93, 0.0);

/// A point `p` of the leg, from the hip joint in the bind pose, with the thigh turned `thigh` and the
/// knee `knee` about `x`.
fn leg_point(thigh: f32, knee: f32, p: Vec3) -> Vec3 {
    Quat::from_rotation_x(thigh) * (KNEE + Quat::from_rotation_x(knee) * (p - KNEE))
}

/// How far one step of [`stroll`] carries the body.
fn stride() -> f32 {
    leg_point(-STROLL_SWING, 0.0, MID_SOLE).z - leg_point(STROLL_SWING, 0.0, MID_SOLE).z
}

/// **A stroll along `+z`, `t` seconds in**: a rotation for every joint, and the hips' translation.
/// One leg stands while the other swings, swapping every half [`STROLL_PERIOD`]. The standing leg is
/// straight and its mid-sole does not move; the hips are carried over it, and set as low as keeps the
/// lowest point of either sole on the ground, so the body rises over a straight leg and dips as the
/// feet change. The arms swing against the legs.
pub fn stroll(t: f32) -> ([Quat; 11], Vec3) {
    let half = STROLL_PERIOD * 0.5;
    let step = (t.max(0.0) / half).floor();
    let u = t.max(0.0) / half - step;
    let a = STROLL_SWING;
    let stance = -a + 2.0 * a * u;
    let lift = (core::f32::consts::PI * u).sin();
    let swing = a - 2.0 * a * (u * u * (3.0 - 2.0 * u)) - 0.22 * lift;
    let (stance_knee, swing_knee) = (0.03, 0.95 * lift);
    let left_stands = (step as i64) % 2 == 0;
    let ((thigh_l, knee_l), (thigh_r, knee_r)) =
        if left_stands { ((stance, stance_knee), (swing, swing_knee)) } else { ((swing, swing_knee), (stance, stance_knee)) };
    let planted = STROLL_START_Z + step * stride();
    let hip_z = planted - leg_point(stance, stance_knee, MID_SOLE).z;
    let lowest = SOLE
        .iter()
        .flat_map(|p| [leg_point(thigh_l, knee_l, *p).y, leg_point(thigh_r, knee_r, *p).y])
        .fold(f32::MAX, f32::min);
    let hip_y = 0.004 - lowest;
    let rx = Quat::from_rotation_x;
    let mut q = [Quat::IDENTITY; 11];
    q[Bone::Spine as usize] = rx(0.05);
    q[Bone::Head as usize] = rx(-0.05);
    q[Bone::ArmUpL as usize] = rx(-1.6 * thigh_l) * Quat::from_rotation_z(0.06);
    q[Bone::ArmUpR as usize] = rx(-1.6 * thigh_r) * Quat::from_rotation_z(-0.06);
    q[Bone::ArmLowL as usize] = rx(-0.3);
    q[Bone::ArmLowR as usize] = rx(-0.3);
    q[Bone::LegUpL as usize] = rx(thigh_l);
    q[Bone::LegLowL as usize] = rx(knee_l);
    q[Bone::LegUpR as usize] = rx(thigh_r);
    q[Bone::LegLowR as usize] = rx(knee_r);
    // The leg joints hang 2 cm under the hips joint.
    (q, Vec3::new(0.0, hip_y + (Bone::Hips.bind().y - Bone::LegUpL.bind().y), hip_z))
}

/// **Drive the skeleton through [`stroll`] from the clock**, as [`animate`] does the walk in place.
pub fn stroll_animate(time: Res<Time>, mut joints: Query<(&Joint, &mut Transform)>) {
    let (rotations, hips) = stroll(time.elapsed_secs());
    for (joint, mut tf) in &mut joints {
        tf.rotation = rotations[joint.0 as usize];
        if joint.0 == Bone::Hips {
            tf.translation = hips;
        }
    }
}

/// **Every foot [`stroll`] puts down after step `after` and by `t` seconds**: which leg, where its sole's
/// centre will lie flat, root space, and which way its toe points. What a game's animation raises as
/// a footstep event, here in closed form.
pub fn touchdowns(after: Option<i64>, t: f32) -> Vec<(i64, Limb, Vec3, Vec3)> {
    let now = (t.max(0.0) / (STROLL_PERIOD * 0.5)).floor() as i64;
    (after.map_or(0, |a| a + 1)..=now)
        .map(|k| {
            let (limb, x) = if k % 2 == 0 { (Limb::LegL, Bone::LegUpL.bind().x) } else { (Limb::LegR, Bone::LegUpR.bind().x) };
            // The sole's centre is a centimetre toe-ward of the mid-sole the stance keeps still.
            (k, limb, Vec3::new(x, 0.0, STROLL_START_Z + k as f32 * stride() + 0.01), Vec3::Z)
        })
        .collect()
}

// ---- The meshes ------------------------------------------------------------------------------

/// Sides of every tube. 24 reads round at the demos' framing.
const SIDES: usize = 24;
/// Rings in a tube's straight part, and in each of its two rounded ends.
const BODY_RINGS: usize = 28;
const CAP_RINGS: usize = 6;

/// A cubic Hermite through `(t, value)` knots, its slopes the finite differences of the neighbours:
/// smooth through every knot, which a piecewise-linear profile is not, and with none of the flat
/// spots a cosine blend leaves at each one.
fn smooth(knots: &[(f32, f32)], t: f32) -> f32 {
    let n = knots.len();
    let (Some(first), Some(last)) = (knots.first(), knots.last()) else { return 0.0 };
    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let i = knots.windows(2).position(|w| t < w[1].0).unwrap_or(n - 2);
    let slope = |k: usize| {
        let (a, b) = (k.saturating_sub(1), (k + 1).min(n - 1));
        (knots[b].1 - knots[a].1) / (knots[b].0 - knots[a].0).max(1.0e-6)
    };
    let (t0, y0) = knots[i];
    let (t1, y1) = knots[i + 1];
    let h = t1 - t0;
    let u = (t - t0) / h;
    let (u2, u3) = (u * u, u * u * u);
    (2.0 * u3 - 3.0 * u2 + 1.0) * y0 + (u3 - 2.0 * u2 + u) * slope(i) * h + (-2.0 * u3 + 3.0 * u2) * y1 + (u3 - u2) * slope(i + 1) * h
}

/// An elliptical tube along a straight axis with a radius profile and a rounded end at each end.
struct Tube {
    from: Vec3,
    to: Vec3,
    /// The direction `rx` is measured along: perpendicular to the axis.
    side: Vec3,
    /// `(t, radius)` knots along the axis for the `side` direction and for the one perpendicular to it.
    rx: Vec<(f32, f32)>,
    rz: Vec<(f32, f32)>,
    /// How far each rounded end reaches past the axis' end.
    caps: [f32; 2],
}

impl Tube {
    /// From `(t, rx, rz)` knots.
    fn new(from: Vec3, to: Vec3, side: Vec3, knots: &[(f32, f32, f32)], caps: [f32; 2]) -> Self {
        Self {
            from,
            to,
            side,
            rx: knots.iter().map(|k| (k.0, k.1)).collect(),
            rz: knots.iter().map(|k| (k.0, k.2)).collect(),
            caps,
        }
    }

    fn len(&self) -> f32 {
        self.from.distance(self.to)
    }

    fn axis(&self) -> Vec3 {
        (self.to - self.from).normalize_or_zero()
    }

    /// Ring parameter `q` in `[-1, 2]` → how far along the axis, and how much of the profile's radius.
    /// `[-1, 0]` is the start's rounded end from its pole, `[0, 1]` the straight part, `[1, 2]` the
    /// far end out to its pole.
    fn ring(&self, q: f32) -> (f32, f32) {
        let end = |q: f32| (q.clamp(0.0, 1.0) * FRAC_PI_2, q >= 1.0);
        if q < 0.0 {
            let (phi, pole) = end(-q);
            (-self.caps[0] * phi.sin(), if pole { 0.0 } else { phi.cos() })
        } else if q > 1.0 {
            let (phi, pole) = end(q - 1.0);
            (self.len() + self.caps[1] * phi.sin(), if pole { 0.0 } else { phi.cos() })
        } else {
            (q * self.len(), 1.0)
        }
    }

    fn at(&self, q: f32, theta: f32) -> Vec3 {
        let (p, scale) = self.ring(q);
        let t = (p / self.len().max(1.0e-6)).clamp(0.0, 1.0);
        let axis = self.axis();
        let other = axis.cross(self.side);
        self.from
            + axis * p
            + self.side * (smooth(&self.rx, t) * scale * theta.cos())
            + other * (smooth(&self.rz, t) * scale * theta.sin())
    }

    /// The outward surface normal, by central differences of [`Tube::at`] — one function for the
    /// position and the normal, so the two cannot disagree — and the axis at a pole, where the
    /// differences in `theta` vanish.
    fn normal(&self, q: f32, theta: f32) -> Vec3 {
        let (dq, dt) = (1.0e-3, 1.0e-3);
        let d_theta = self.at(q, theta + dt) - self.at(q, theta - dt);
        let d_q = self.at(q + dq, theta) - self.at(q - dq, theta);
        let raw = d_theta.cross(d_q);
        if raw.length() < 1.0e-9 {
            return if q < 0.5 { -self.axis() } else { self.axis() };
        }
        let centre = self.from + self.axis() * self.ring(q).0;
        let n = raw.normalize();
        if n.dot(self.at(q, theta) - centre) < 0.0 { -n } else { n }
    }

    /// The ring parameters, pole to pole.
    fn rings(&self) -> Vec<f32> {
        let start = (0..CAP_RINGS).map(|i| -1.0 + i as f32 / CAP_RINGS as f32);
        let body = (0..=BODY_RINGS).map(|j| j as f32 / BODY_RINGS as f32);
        let end = (1..=CAP_RINGS).map(|i| 1.0 + i as f32 / CAP_RINGS as f32);
        start.chain(body).chain(end).collect()
    }

    /// The tube's `UV_0` island in metres: the longest ring's circumference by the surface length
    /// along the axis, so a unit of `u` or `v` is the same distance on the skin.
    fn island(&self) -> Vec2 {
        let max_r = self.rx.iter().zip(&self.rz).map(|(a, b)| (0.5 * (a.1 * a.1 + b.1 * b.1)).sqrt()).fold(0.0, f32::max);
        let rings = self.rings();
        let length: f32 = rings.windows(2).map(|w| self.at(w[0], 0.0).distance(self.at(w[1], 0.0))).sum();
        Vec2::new(TAU * max_r, length)
    }
}

/// Four joints and their weights.
type Weights = ([u16; 4], [f32; 4]);

fn solo(bone: Bone) -> Weights {
    ([bone as u16, 0, 0, 0], [1.0, 0.0, 0.0, 0.0])
}

/// Blend `upper` into `lower` across `half` metres either side of `joint_y`, by a smoothstep.
fn blend(y: f32, joint_y: f32, half: f32, upper: Bone, lower: Bone) -> Weights {
    let t = ((joint_y + half - y) / (2.0 * half)).clamp(0.0, 1.0);
    let w = t * t * (3.0 - 2.0 * t);
    ([upper as u16, lower as u16, 0, 0], [1.0 - w, w, 0.0, 0.0])
}

/// A tube with the rule that weights it.
struct Piece {
    tube: Tube,
    weights: Box<dyn Fn(Vec3) -> Weights>,
}

/// Shelf-pack `sizes` (metres) into rows no wider than the widest of them or the square root of their
/// area, and return each one's origin and the sheet's longest edge. `UV` is then metres over that edge:
/// one density for the whole mesh, and no island stretched.
fn pack(sizes: &[Vec2]) -> (Vec<Vec2>, f32) {
    let area: f32 = sizes.iter().map(|s| s.x * s.y).sum();
    let widest = sizes.iter().map(|s| s.x).fold(0.0, f32::max);
    let row_limit = widest.max(area.sqrt() * 1.2);
    let (mut origins, mut x, mut y, mut row_h, mut right) = (Vec::new(), 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for s in sizes {
        if x > 0.0 && x + s.x > row_limit {
            y += row_h + 0.01;
            x = 0.0;
            row_h = 0.0;
        }
        origins.push(Vec2::new(x, y));
        x += s.x + 0.01;
        row_h = row_h.max(s.y);
        right = right.max(x);
    }
    (origins, right.max(y + row_h))
}

/// One skinned mesh from several tubes.
fn build(pieces: &[Piece]) -> Mesh {
    let sizes: Vec<Vec2> = pieces.iter().map(|p| p.tube.island()).collect();
    let (origins, edge) = pack(&sizes);
    let (mut positions, mut normals, mut uvs, mut joints, mut weights) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut indices: Vec<u32> = Vec::new();
    for (piece, (origin, size)) in pieces.iter().zip(origins.iter().zip(&sizes)) {
        let tube = &piece.tube;
        let rings = tube.rings();
        let base = positions.len() as u32;
        // Distance down the tube at each ring, along the seam, for `v`.
        let mut along = vec![0.0f32; rings.len()];
        for j in 1..rings.len() {
            along[j] = along[j - 1] + tube.at(rings[j - 1], 0.0).distance(tube.at(rings[j], 0.0));
        }
        let total = along.last().copied().unwrap_or(1.0).max(1.0e-6);
        for (j, q) in rings.iter().enumerate() {
            for s in 0..=SIDES {
                let u = s as f32 / SIDES as f32;
                let theta = u * TAU;
                let p = tube.at(*q, theta);
                let (joint, weight) = (piece.weights)(p);
                positions.push(p.to_array());
                normals.push(tube.normal(*q, theta).to_array());
                uvs.push(((*origin + Vec2::new(u * size.x, along[j] / total * size.y)) / edge).to_array());
                joints.push(joint);
                weights.push(weight);
            }
        }
        // Wound so the geometric normal of the first quad that has area agrees with the analytic one.
        let quad = |j: usize, s: usize| {
            let a = base + (j * (SIDES + 1) + s) as u32;
            (a, a + 1, a + (SIDES + 1) as u32, a + (SIDES + 2) as u32)
        };
        let mid = CAP_RINGS + BODY_RINGS / 2;
        let (a, b, c, _) = quad(mid, 0);
        let at = |i: u32| Vec3::from(positions[i as usize]);
        let geometric = (at(c) - at(a)).cross(at(b) - at(a));
        let flip = geometric.dot(Vec3::from(normals[a as usize])) < 0.0;
        for j in 0..rings.len() - 1 {
            for s in 0..SIDES {
                let (a, b, c, d) = quad(j, s);
                if flip {
                    indices.extend([a, b, c, b, d, c]);
                } else {
                    indices.extend([a, c, b, b, c, d]);
                }
            }
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// The pieces of one part. Torso and head are built from the bottom up, so blood on them needs `v`
/// to run the other way: see [`gravity_uv`].
fn pieces(limb: Limb) -> Vec<Piece> {
    let x = Vec3::X;
    match limb {
        Limb::Torso => vec![Piece {
            tube: Tube::new(
                Vec3::new(0.0, 0.90, 0.0),
                Vec3::new(0.0, 1.50, 0.0),
                x,
                &[
                    (0.00, 0.150, 0.095),
                    (0.15, 0.158, 0.100),
                    (0.40, 0.130, 0.088),
                    (0.65, 0.165, 0.105),
                    (0.85, 0.178, 0.108),
                    (0.95, 0.120, 0.085),
                    (1.00, 0.052, 0.050),
                ],
                [0.06, 0.02],
            ),
            weights: Box::new(|p| blend(p.y, 1.05, 0.10, Bone::Spine, Bone::Hips)),
        }],
        Limb::Head => vec![Piece {
            tube: Tube::new(
                Vec3::new(0.0, 1.46, 0.0),
                Vec3::new(0.0, 1.70, 0.0),
                x,
                &[
                    (0.00, 0.043, 0.043),
                    (0.22, 0.042, 0.042),
                    (0.42, 0.058, 0.066),
                    (0.60, 0.080, 0.092),
                    (0.78, 0.089, 0.102),
                    (0.92, 0.084, 0.097),
                    (1.00, 0.075, 0.088),
                ],
                [0.03, 0.09],
            ),
            weights: Box::new(|_| solo(Bone::Head)),
        }],
        Limb::ArmL | Limb::ArmR => {
            let (side, up, low) = if limb == Limb::ArmL {
                (0.235, Bone::ArmUpL, Bone::ArmLowL)
            } else {
                (-0.235, Bone::ArmUpR, Bone::ArmLowR)
            };
            vec![Piece {
                tube: Tube::new(
                    Vec3::new(side, 1.455, 0.0),
                    Vec3::new(side, 0.80, 0.0),
                    x,
                    &[
                        (0.00, 0.048, 0.048),
                        (0.12, 0.052, 0.050),
                        (0.30, 0.046, 0.045),
                        (0.46, 0.040, 0.040),
                        (0.60, 0.043, 0.041),
                        (0.85, 0.029, 0.027),
                        (0.93, 0.037, 0.022),
                        (1.00, 0.030, 0.016),
                    ],
                    [0.048, 0.03],
                ),
                weights: Box::new(move |p| blend(p.y, 1.13, 0.08, up, low)),
            }]
        }
        Limb::LegL | Limb::LegR => {
            let (side, up, low) = if limb == Limb::LegL {
                (0.095, Bone::LegUpL, Bone::LegLowL)
            } else {
                (-0.095, Bone::LegUpR, Bone::LegLowR)
            };
            vec![
                Piece {
                    tube: Tube::new(
                        Vec3::new(side, 0.975, 0.0),
                        Vec3::new(side, 0.035, 0.0),
                        x,
                        &[
                            (0.00, 0.080, 0.085),
                            (0.10, 0.090, 0.095),
                            (0.30, 0.075, 0.078),
                            (0.50, 0.055, 0.058),
                            (0.62, 0.060, 0.066),
                            (0.85, 0.042, 0.045),
                            (0.95, 0.036, 0.038),
                            (1.00, 0.036, 0.038),
                        ],
                        [0.08, 0.036],
                    ),
                    weights: Box::new(move |p| blend(p.y, 0.50, 0.09, up, low)),
                },
                Piece {
                    tube: Tube::new(
                        Vec3::new(side, 0.05, -0.06),
                        Vec3::new(side, 0.05, 0.17),
                        x,
                        &[(0.00, 0.036, 0.036), (0.40, 0.045, 0.042), (0.85, 0.040, 0.030), (1.00, 0.030, 0.020)],
                        [0.036, 0.03],
                    ),
                    weights: Box::new(move |_| solo(low)),
                },
            ]
        }
    }
}

/// **A sleeveless shirt**: the torso's tube a centimetre out all round, weighted as the torso is so it
/// moves with it, closed at the neck and flat across the waist where the legs leave it. A centimetre
/// is what keeps the skin from showing through it as the spine bends; it is also the gap a shot
/// crosses before it reaches the skin, so the hole it opens is in front of the wound.
fn shirt_pieces() -> Vec<Piece> {
    let mut pieces = pieces(Limb::Torso);
    for piece in &mut pieces {
        for knot in piece.tube.rx.iter_mut().chain(piece.tube.rz.iter_mut()) {
            knot.1 += 0.010;
        }
        piece.tube.caps = [0.0, piece.tube.caps[1] + 0.010];
    }
    pieces
}

/// **Which way blood runs on this part's atlas.** Every limb is built from the shoulder or the hip
/// down, so `v` grows downward and the default `(0, 1)` is right; the torso and the head are built
/// from the bottom up, so theirs is `(0, -1)`.
pub fn gravity_uv(limb: Limb) -> Vec2 {
    match limb {
        Limb::Torso | Limb::Head => Vec2::new(0.0, -1.0),
        _ => Vec2::new(0.0, 1.0),
    }
}

/// Every part, in spawn order.
pub const LIMBS: [Limb; 6] = [Limb::Torso, Limb::Head, Limb::ArmL, Limb::ArmR, Limb::LegL, Limb::LegR];

/// **Spawn the character**: a root, eleven joints under it, and six skinned meshes sharing them. The
/// meshes carry no `Gore` of their own: the root carries a [`GoreScene`] that dresses each by its name,
/// as a game would a loaded glTF. Its atlas density is not stated: the preset measures it from the pose.
/// With `shirt`, a seventh skinned mesh named `Shirt` is worn over the torso and dressed as cloth.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
    shirt: bool,
) -> Entity {
    // Textured under a white factor, as a glTF exporter writes it.
    let skin = materials.add(StandardMaterial {
        base_color_texture: Some(preset_scene::skin_texture(images)),
        perceptual_roughness: 0.55,
        ..default()
    });
    let inverse_bindposes = bindposes.add(SkinnedMeshInverseBindposes::from(
        Bone::ALL.iter().map(|b| Mat4::from_translation(-b.bind())).collect::<Vec<_>>(),
    ));
    // **Its skin canvases are 256 a side**, the preset's ceiling, where 128 ships: 3.6 mm a texel on
    // these limbs, against 7.2. A showcase's choice — at 128 every run of blood on a thigh blurred
    // into the patch it came from — and it costs 4 times the shipped bytes per part.
    let scene = LIMBS.iter().fold(GoreScene::new(Gore::floor()).otherwise_nothing(), |scene, limb| {
        scene.with(&format!("{limb:?}"), Gore::skin(limb.region()).with_gravity_uv(gravity_uv(*limb)).with_canvas(256))
    });
    let scene = scene.with("Shirt", Gore::cloth().with_gravity_uv(gravity_uv(Limb::Torso)));
    let root = commands.spawn((Transform::IDENTITY, Visibility::default(), Mannequin, scene)).id();
    let mut entities: Vec<Entity> = Vec::new();
    for bone in Bone::ALL {
        let parent = bone.parent().map(|p| entities[p as usize]).unwrap_or(root);
        let local = bone.bind() - bone.parent().map(Bone::bind).unwrap_or(Vec3::ZERO);
        entities.push(commands.spawn((Transform::from_translation(local), Joint(bone), ChildOf(parent))).id());
    }
    for limb in LIMBS {
        commands.spawn((
            Mesh3d(meshes.add(build(&pieces(limb)))),
            MeshMaterial3d(skin.clone()),
            SkinnedMesh { inverse_bindposes: inverse_bindposes.clone(), joints: entities.clone() },
            // A skinned mesh is drawn where its joints put it, which is nowhere near its asset's
            // bounds, so it is never culled by them.
            NoFrustumCulling,
            Transform::IDENTITY,
            Name::new(format!("{limb:?}")),
            Part(limb),
            ChildOf(root),
        ));
    }
    if shirt {
        // A pale cotton, so blood on it reads; matte, as a weave is.
        let cotton = materials.add(StandardMaterial { base_color: Color::srgb(0.62, 0.66, 0.70), perceptual_roughness: 0.92, ..default() });
        commands.spawn((
            Mesh3d(meshes.add(build(&shirt_pieces()))),
            MeshMaterial3d(cotton),
            SkinnedMesh { inverse_bindposes: inverse_bindposes.clone(), joints: entities.clone() },
            NoFrustumCulling,
            Transform::IDENTITY,
            Name::new("Shirt"),
            ChildOf(root),
        ));
    }
    root
}

/// **The mannequin as something that breaks.** Its six meshes are cut in their own bind space, which
/// is `Mat4::IDENTITY` for a skinned mesh, since it is drawn from its joints and its entity's transform
/// plays no part. **The proxy is eleven boxes that hug the figure**, face to face so the bond graph joins
/// the arms and legs to the torso and the head to its neck. It is what a game's own convex
/// decomposition would be: a cut face is the proxy cell's cross-section, so a box wider than the limb
/// it holds grows a flap beyond it, and one box to a whole tapering leg did exactly that. Insert it on
/// the root, and a body that takes a hit to bone breaks where its skeleton is that frame, not in the T of
/// its bind pose.
pub fn breakable() -> GoreBreakable {
    let meshes: Vec<Mesh> = LIMBS.iter().map(|limb| build(&pieces(*limb))).collect();
    let parts: Vec<(&Mesh, Mat4)> = meshes.iter().map(|m| (m, Mat4::IDENTITY)).collect();
    let cell = |centre: [f32; 3], half: [f32; 3]| ProxyCell::from_box(Vec3::from_array(centre), Vec3::from_array(half));
    let mut proxy = vec![cell([0.0, 1.225, 0.0], [0.18, 0.255, 0.11]), cell([0.0, 1.635, 0.0], [0.092, 0.155, 0.105])];
    for side in [1.0f32, -1.0] {
        proxy.push(cell([0.238 * side, 1.3165, 0.0], [0.058, 0.1865, 0.058]));
        proxy.push(cell([0.235 * side, 0.95, 0.0], [0.048, 0.18, 0.048]));
        proxy.push(cell([0.092 * side, 0.735, 0.0], [0.092, 0.235, 0.097]));
        proxy.push(cell([0.095 * side, 0.2975, 0.0], [0.065, 0.2025, 0.07]));
        proxy.push(cell([0.095 * side, 0.0465, 0.052], [0.048, 0.0485, 0.148]));
    }
    let cut = CutSettings { soften: 0.0, ..CutSettings::new(16, 0.08, 7) };
    GoreBreakable::from_fracture(fracture_mesh(&parts, &proxy, &cut), Region::Torso, &Scale::default())
}

// ---- Aiming ----------------------------------------------------------------------------------

/// Where `muzzle` shoots from: in front of the character, at chest height, a few metres off.
pub const MUZZLE: Vec3 = Vec3::new(0.35, 1.35, 3.6);

/// **The camera the demo and its clip share**: in front, far enough back to hold the whole figure and
/// close enough that a wound is more than a speck. Level with the muzzle's side of the room rather
/// than off to one side of it, because from the far left the fill light's mirror angle on the floor
/// lands on the pools and turns the blood there white.
pub fn view() -> Transform {
    Transform::from_xyz(0.25, 1.25, 2.9).looking_at(Vec3::new(0.12, 0.93, 0.1), Vec3::Y)
}

/// **What aiming at the body reads**: the joints' `GlobalTransform`s — the ones the renderer last
/// drew with, which is the pose a player aiming at the screen is aiming at — and the character's
/// root. A `SystemParam` so a system takes it and a recorder reads it through a `SystemState`.
///
/// **Every hit is addressed to the root**, which is all a game's physics would tell it: the capsule it
/// found belongs to the character, and which mesh the ray then reaches is the preset's to say.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Aim<'w, 's> {
    joints: Query<'w, 's, (&'static Joint, &'static GlobalTransform)>,
    root: Query<'w, 's, Entity, With<Mannequin>>,
}

impl Aim<'_, '_> {
    /// **A point `along` metres down a joint's own `-Y`, as the body is posed now.**
    pub fn on_bone(&self, bone: Bone, along: f32) -> Option<Vec3> {
        self.joints.iter().find(|(j, _)| j.0 == bone).map(|(_, g)| g.transform_point(Vec3::new(0.0, -along, 0.0)))
    }

    /// **Aim a ray from the muzzle at `along` metres down `bone`**: the ray a rifle would fire, so
    /// what the preset does with it is what a game's own raycast would get.
    fn ray(&self, bone: Bone, along: f32) -> Option<(Vec3, Vec3, Entity)> {
        let target = self.on_bone(bone, along)?;
        Some((MUZZLE, (target - MUZZLE).normalize_or_zero(), self.root.iter().next()?))
    }

    /// A shot at a joint of the body, `radius_m` wide and `depth_mm` deep.
    pub fn shot(&self, bone: Bone, along: f32, radius_m: f32, depth_mm: f32) -> Option<GoreHit> {
        self.ray(bone, along).map(|(from, dir, entity)| GoreHit::impact(entity, from, dir, radius_m, depth_mm))
    }

    /// A slash along `along`, centred where [`Aim::shot`] would land.
    pub fn slash(&self, bone: Bone, at: f32, along: Vec3, radius_m: f32, depth_mm: f32) -> Option<GoreHit> {
        self.ray(bone, at).map(|(from, dir, entity)| GoreHit::slash(entity, from, dir, along, radius_m, depth_mm))
    }

    /// A blow that bruises.
    pub fn blow(&self, bone: Bone, along: f32, radius_m: f32) -> Option<GoreHit> {
        self.ray(bone, along).map(|(from, dir, entity)| GoreHit::blunt(entity, from, dir, radius_m))
    }

    /// A hot iron.
    pub fn iron(&self, bone: Bone, along: f32, radius_m: f32) -> Option<GoreHit> {
        self.ray(bone, along).map(|(from, dir, entity)| GoreHit::burn(entity, from, dir, radius_m, 150.0, 1.0))
    }
}
