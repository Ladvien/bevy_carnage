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
use bevy_carnage::cross_section::Region;
use bevy_carnage::preset::{Gore, GoreHit};

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

/// **Spawn the character**: a root, eleven joints under it, and six skinned meshes sharing them, each
/// a `Gore` surface. Its atlas density is not stated: the preset measures it from the pose.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
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
    let root = commands.spawn((Transform::IDENTITY, Visibility::default(), Mannequin)).id();
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
            Gore::skin(limb.region()).with_gravity_uv(gravity_uv(limb)),
            Part(limb),
            ChildOf(root),
        ));
    }
    root
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
