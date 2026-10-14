//! **A skinned mesh, posed on the CPU.**
//!
//! A skinned mesh in Bevy is two things at once: the bind-pose vertices in its asset, and a pose the
//! GPU reads from the joint entities every frame. Everything this crate casts a ray at — a hit, a
//! droplet, a drip — has to land on the *second* one, because that is the body the player sees. This
//! module is the CPU half of what `bevy_pbr`'s vertex shader does, so a ray can be cast at the pose.
//!
//! **It is the shader's arithmetic, not an approximation of it.** `bevy_pbr::skinning::skin_model`
//! blends the four joint matrices by the vertex's weights, with no renormalisation, and the result is
//! the vertex's world matrix: the mesh entity's own `GlobalTransform` plays no part in it. A joint
//! matrix is `joint_global · inverse_bindpose` (`bevy_pbr::render::skin`, `extract_joints_for_skin`).
//! [`Pose`] holds exactly those matrices and [`Pose::blend`] exactly that sum, so a posed position is
//! the world position the GPU draws, to float rounding.
//!
//! **Nothing here reads a clock, an entity or an asset arena.** A [`Pose`] is built from matrices the
//! caller hands in, and every function is a pure function of its arguments — which is what lets the
//! preset hash a canvas whose wounds were found on an animated body.
//!
//! **Malformed rigs are refused, not repaired.** A vertex whose joint index is past the joint list, a
//! non-finite weight, or a joint list that does not match the inverse bind poses returns `None`; the
//! shader would read past the end of its buffer, and a posed position invented in its place would be
//! the one place this crate guessed where a body is.

use bevy::asset::RenderAssetUsages;
use bevy::ecs::entity::Entity;
use bevy::math::{Mat3, Mat4, Vec3};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};

/// **The joint matrices of one skinned mesh at one instant**, each `joint_global · inverse_bindpose`.
/// Multiplying a bind-pose position by one carries it to where that joint alone would put it, in
/// world space.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    matrices: Vec<Mat4>,
}

impl Pose {
    /// **A pose from a skin, its inverse bind poses and the joints' world matrices.** `world` is asked
    /// for each joint entity in `skin.joints` order and returns its world matrix, or `None` when it
    /// does not have one yet (a scene still streaming in) — in which case there is no pose. `None`
    /// too when the joint list and the inverse bind poses are not the same length: Bevy zips them and
    /// would silently pose the shorter of the two.
    pub fn of(
        skin: &SkinnedMesh,
        bindposes: &SkinnedMeshInverseBindposes,
        mut world: impl FnMut(Entity) -> Option<Mat4>,
    ) -> Option<Self> {
        if skin.joints.len() != bindposes.len() {
            return None;
        }
        let mut matrices = Vec::with_capacity(skin.joints.len());
        for (joint, inverse_bind) in skin.joints.iter().zip(bindposes.iter()) {
            matrices.push(world(*joint)? * *inverse_bind);
        }
        Some(Self { matrices })
    }

    /// A pose from joint matrices already multiplied out. Each must be `joint_global ·
    /// inverse_bindpose`.
    pub fn from_matrices(matrices: Vec<Mat4>) -> Self {
        Self { matrices }
    }

    /// How many joints.
    pub fn len(&self) -> usize {
        self.matrices.len()
    }

    /// Whether the pose has no joints at all.
    pub fn is_empty(&self) -> bool {
        self.matrices.is_empty()
    }

    /// The matrix joint `joint` moves its vertices by — `joint_global · inverse_bindpose` — or `None`
    /// past the end of the pose.
    pub fn joint(&self, joint: u16) -> Option<&Mat4> {
        self.matrices.get(joint as usize)
    }

    /// **A vertex's world matrix**: its four joint matrices blended by its four weights, exactly as
    /// `skin_model` sums them — no renormalisation, so a vertex whose weights do not add to one is
    /// scaled the way the GPU scales it. `None` when a weight is not finite or a joint index is past
    /// the end of the pose. A weight of zero never reads its joint, so a vertex with fewer than four
    /// influences may carry any index in the unused slots.
    pub fn blend(&self, joints: [u16; 4], weights: [f32; 4]) -> Option<Mat4> {
        let mut sum = Mat4::ZERO;
        for (joint, weight) in joints.into_iter().zip(weights) {
            if !weight.is_finite() {
                return None;
            }
            if weight == 0.0 {
                continue;
            }
            sum += *self.matrices.get(joint as usize)? * weight;
        }
        Some(sum)
    }
}

/// **A mesh's per-vertex joint indices and weights**, borrowed. A mesh with both attributes in the
/// formats Bevy's glTF loader writes (`Uint16x4`, `Float32x4`) and one entry per vertex is a rig.
#[derive(Clone, Copy, Debug)]
pub struct Rig<'a> {
    joints: &'a [[u16; 4]],
    weights: &'a [[f32; 4]],
}

impl<'a> Rig<'a> {
    /// The rig of `mesh`, or `None` when it is not skinned: either attribute missing, in another
    /// format, or not one entry per position.
    pub fn of(mesh: &'a Mesh) -> Option<Self> {
        let Ok(Some(VertexAttributeValues::Uint16x4(joints))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_JOINT_INDEX) else {
            return None;
        };
        let Ok(Some(VertexAttributeValues::Float32x4(weights))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_JOINT_WEIGHT) else {
            return None;
        };
        let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
            return None;
        };
        (joints.len() == positions.len() && weights.len() == positions.len()).then_some(Self { joints, weights })
    }

    /// The vertex's world matrix under `pose`. See [`Pose::blend`].
    pub fn matrix(&self, pose: &Pose, vertex: usize) -> Option<Mat4> {
        pose.blend(*self.joints.get(vertex)?, *self.weights.get(vertex)?)
    }

    /// **The joint that carries `vertex`**: the one with the most weight, ties to the earlier slot.
    /// `None` for a vertex past the end or one with no positive finite weight. A body that breaks
    /// carries each piece with the bone of the vertex nearest it, as a rigid piece must be carried by
    /// one bone and not a blend of several.
    pub fn dominant_joint(&self, vertex: usize) -> Option<u16> {
        let (joints, weights) = (self.joints.get(vertex)?, self.weights.get(vertex)?);
        let mut best: Option<(f32, u16)> = None;
        for (joint, weight) in joints.iter().zip(weights) {
            if weight.is_finite() && *weight > 0.0 && best.is_none_or(|(w, _)| *weight > w) {
                best = Some((*weight, *joint));
            }
        }
        best.map(|(_, joint)| joint)
    }
}

/// **Where one vertex of `mesh` is under `pose`**, world space. `None` for a mesh with no rig, a vertex
/// past the end, or a joint the pose does not have.
pub fn posed_vertex(mesh: &Mesh, pose: &Pose, vertex: usize) -> Option<Vec3> {
    let rig = Rig::of(mesh)?;
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let at = Vec3::from_array(*positions.get(vertex)?);
    Some(rig.matrix(pose, vertex)?.transform_point3(at))
}

/// **The mesh as the pose draws it, as a mesh a ray can be cast at.** World-space positions, plus
/// `UV_0` and the indices — the three attributes a hit needs and nothing else, so posing a character
/// does not copy its normals, tangents and skin weights on every shot. Cast it with
/// `GlobalTransform::IDENTITY`: the positions are already where the GPU draws them.
///
/// `None` when the mesh is not a triangle list, has no rig, or any vertex cannot be posed. The
/// result's vertex order is the source's, so a triangle index found on it names the same triangle on
/// the bind mesh.
pub fn posed_mesh(mesh: &Mesh, pose: &Pose) -> Option<Mesh> {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return None;
    }
    let rig = Rig::of(mesh)?;
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let mut posed = Vec::with_capacity(positions.len());
    // The last blend, by its exact inputs: runs of vertices on the same joints with the same weights —
    // every vertex of a part carried by one bone — blend once. The same inputs give the same matrix to
    // the bit, so this is the blend `rig.matrix` would have made.
    let mut last: Option<([u16; 4], [u32; 4], Mat4)> = None;
    for (vertex, at) in positions.iter().enumerate() {
        let (joints, weights) = (*rig.joints.get(vertex)?, *rig.weights.get(vertex)?);
        let key = weights.map(f32::to_bits);
        let blended = match last {
            Some((j, w, m)) if j == joints && w == key => m,
            _ => {
                let m = pose.blend(joints, weights)?;
                last = Some((joints, key, m));
                m
            }
        };
        let world = blended.transform_point3(Vec3::from_array(*at));
        if !world.is_finite() {
            return None;
        }
        posed.push(world.to_array());
    }
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD);
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, posed);
    if let Ok(Some(VertexAttributeValues::Float32x2(uvs))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) {
        out.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.clone());
    }
    match mesh.try_indices_option() {
        Ok(Some(Indices::U16(i))) => out.insert_indices(Indices::U16(i.clone())),
        Ok(Some(Indices::U32(i))) => out.insert_indices(Indices::U32(i.clone())),
        _ => {}
    }
    Some(out)
}

/// **The largest stretch a matrix applies along any axis**: the longest column of its linear part. What
/// a length in the bind pose is multiplied by on the way to the world, at worst.
pub fn max_scale(m: &Mat4) -> f32 {
    let linear = Mat3::from_mat4(*m);
    linear.x_axis.length().max(linear.y_axis.length()).max(linear.z_axis.length())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Quat;

    /// Two vertices on one joint each, plus a third halfway between them.
    fn rigged() -> Mesh {
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD);
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1.0, 0.0]]);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]);
        m.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(vec![[0, 0, 0, 0], [1, 0, 0, 0], [0, 1, 0, 0]]));
        m.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]]);
        m.insert_indices(Indices::U32(vec![0, 1, 2]));
        m
    }

    #[test]
    fn a_vertex_follows_its_joint() {
        let mesh = rigged();
        // Joint 1 turns a quarter-turn about Z; joint 0 stays. Both bind poses are the identity.
        let pose = Pose::from_matrices(vec![Mat4::IDENTITY, Mat4::from_quat(Quat::from_rotation_z(core::f32::consts::FRAC_PI_2))]);
        let a = posed_vertex(&mesh, &pose, 0).unwrap_or(Vec3::NAN);
        let b = posed_vertex(&mesh, &pose, 1).unwrap_or(Vec3::NAN);
        assert!(a.distance(Vec3::ZERO) < 1.0e-6, "a vertex on the still joint moved: {a:?}");
        assert!(b.distance(Vec3::new(0.0, 1.0, 0.0)) < 1.0e-6, "x = 1 turned a quarter about Z lands on y = 1: {b:?}");
    }

    #[test]
    fn two_weights_blend_the_matrices_not_the_angles() {
        let mesh = rigged();
        let pose = Pose::from_matrices(vec![Mat4::IDENTITY, Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0))]);
        // Vertex 2 is half joint 0, half joint 1: halfway between where each would put (0.5, 1, 0).
        let c = posed_vertex(&mesh, &pose, 2).unwrap_or(Vec3::NAN);
        assert!(c.distance(Vec3::new(1.5, 1.0, 0.0)) < 1.0e-6, "{c:?}");
    }

    #[test]
    fn weights_are_not_renormalised() {
        let mut mesh = rigged();
        // Half a weight on one joint: the GPU scales the vertex by a half, and so does this.
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[0.5, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]]);
        let pose = Pose::from_matrices(vec![Mat4::from_translation(Vec3::new(0.0, 0.0, 4.0)), Mat4::IDENTITY]);
        let a = posed_vertex(&mesh, &pose, 0).unwrap_or(Vec3::NAN);
        assert!(a.distance(Vec3::new(0.0, 0.0, 2.0)) < 1.0e-6, "{a:?}");
    }

    #[test]
    fn a_joint_past_the_pose_is_refused() {
        let mesh = rigged();
        // Vertex 1 names joint 1; a one-joint pose has no such joint.
        let pose = Pose::from_matrices(vec![Mat4::IDENTITY]);
        assert!(posed_vertex(&mesh, &pose, 1).is_none());
        assert!(posed_mesh(&mesh, &pose).is_none(), "a mesh with one unposable vertex must not be partly posed");
    }

    #[test]
    fn an_unused_slot_may_name_any_joint() {
        let mut mesh = rigged();
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(vec![[0, 99, 99, 99], [0, 99, 99, 99], [0, 99, 99, 99]]));
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[1.0, 0.0, 0.0, 0.0]; 3]);
        let pose = Pose::from_matrices(vec![Mat4::IDENTITY]);
        assert!(posed_mesh(&mesh, &pose).is_some());
    }

    #[test]
    fn a_posed_mesh_keeps_its_triangles_and_uvs() {
        let mesh = rigged();
        let pose = Pose::from_matrices(vec![Mat4::from_translation(Vec3::Y), Mat4::from_translation(Vec3::Y)]);
        let posed = posed_mesh(&mesh, &pose);
        let Some(posed) = posed else { panic!("a valid rig must pose") };
        assert!(matches!(posed.try_indices_option(), Ok(Some(Indices::U32(i))) if i == &vec![0, 1, 2]));
        assert!(matches!(
            posed.try_attribute_option(Mesh::ATTRIBUTE_UV_0),
            Ok(Some(VertexAttributeValues::Float32x2(u))) if u == &vec![[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]
        ));
        assert!(matches!(
            posed.try_attribute_option(Mesh::ATTRIBUTE_POSITION),
            Ok(Some(VertexAttributeValues::Float32x3(p))) if p[0] == [0.0, 1.0, 0.0]
        ));
    }

    #[test]
    fn a_mesh_with_no_rig_is_not_posed() {
        let mut mesh = rigged();
        mesh.remove_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT);
        assert!(Rig::of(&mesh).is_none());
        assert!(posed_mesh(&mesh, &Pose::from_matrices(vec![Mat4::IDENTITY; 2])).is_none());
    }

    #[test]
    fn a_pose_needs_one_inverse_bind_per_joint() {
        let skin = SkinnedMesh { inverse_bindposes: Default::default(), joints: vec![Entity::PLACEHOLDER; 2] };
        let two = SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY; 2]);
        let one = SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY; 1]);
        assert!(Pose::of(&skin, &two, |_| Some(Mat4::IDENTITY)).is_some());
        assert!(Pose::of(&skin, &one, |_| Some(Mat4::IDENTITY)).is_none(), "Bevy would pose the shorter list silently");
        assert!(Pose::of(&skin, &two, |_| None).is_none(), "a joint with no transform yet is no pose");
    }

    #[test]
    fn max_scale_is_the_longest_column() {
        let m = Mat4::from_scale(Vec3::new(1.0, 3.0, 2.0));
        assert!((max_scale(&m) - 3.0).abs() < 1.0e-6);
    }
}
