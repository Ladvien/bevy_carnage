//! **A skinned body breaks where its skeleton is.**
//!
//! [`fracture_mesh`](crate::fracture_mesh) splits a mesh by convex proxy cells, and
//! [`BondGraph::of`] bonds only cells that share coplanar faces, so a body cut per bone comes out as
//! islands: a bone meets the next at a joint, not over a polygon. This module is the other way in.
//! The caller says which joints make one piece ([`JointGroup`]) and which pieces meet at which joint
//! ([`JointLink`]); [`fracture_skeleton`] cuts the body's meshes into one piece per group, once, and
//! declares the bonds ([`BondGraph::declared`]), so the same region queries
//! ([`radial`](crate::radial), [`capsule`](crate::capsule), [`spread`](crate::spread)) and
//! [`BondGraph::islands`] that work on a cell fracture work on a skeleton.
//!
//! # Rigid bodies only, and that is a refusal
//!
//! A vertex belongs to the group of its dominant joint ([`Rig::dominant_joint`]). A triangle whose
//! three vertices fall in different groups would have to be cut or assigned by a guess, and a guess
//! here would tear a limb or weld a piece to its neighbour; [`SkeletalRefusal::SplitTriangle`] says so
//! instead. A skeleton whose bones are separate meshes skinned rigidly (each vertex one joint at
//! weight one) never trips it. A soft-skinned body is refused, as the crate's other bakes refuse what
//! they cannot do exactly.
//!
//! # What comes out
//!
//! Pieces are in the meshes' own bind space, **not recentred**, so a piece drawn with its joint's
//! `joint_global · inverse_bind` matrix lands exactly where the whole body drew it. The skin
//! attributes are dropped: a piece is a rigid body of its group's joints, carried by [`anchor`].
//!
//! [`anchor`]: SkeletalPiece::anchor

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::math::{Mat4, Vec3};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};

use crate::bond::{Bond, BondGraph};
use crate::skin::Rig;
use crate::tree::FragmentId;

/// **The joints that make one piece**, and the one that carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JointGroup {
    /// The skin's joint indices whose vertices belong to this piece. A joint in two groups belongs to
    /// the first.
    pub joints: Vec<u16>,
    /// The joint whose pose carries the piece once it is cut free.
    pub anchor: u16,
}

/// **Two pieces that meet at a joint**: indices into the `groups` slice, and the joint whose bind
/// position is where they are bonded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JointLink {
    pub a: usize,
    pub b: usize,
    pub joint: u16,
}

/// **One piece of the skeleton**: group `i` is piece `i`, `id` is `FragmentId(i)`.
#[derive(Clone, Debug)]
pub struct SkeletalPiece {
    pub id: FragmentId,
    /// The group's [`JointGroup::anchor`].
    pub anchor: u16,
    /// One mesh per input part: that part's triangles in this group, compacted, with `POSITION`,
    /// `NORMAL` and `UV_0` where the part has them. `None` when the part has no triangle here.
    pub meshes: Vec<Option<Mesh>>,
    /// The mean of the group's vertices over every part, in bind space. A group with no vertex takes
    /// its anchor's bind position.
    pub center: Vec3,
}

/// **What [`fracture_skeleton`] makes**: the pieces, and the bonds between them as a graph over
/// `FragmentId(0)..FragmentId(pieces.len())`.
#[derive(Clone, Debug)]
pub struct SkeletalFracture {
    pub pieces: Vec<SkeletalPiece>,
    /// Declared, not found: one bond per [`JointLink`], with `area` 1.0 (the caller's, unitless).
    pub bonds: BondGraph,
}

/// **Why a body could not be cut along its skeleton.** Nothing is repaired or guessed.
#[derive(Clone, Debug, PartialEq)]
pub enum SkeletalRefusal {
    /// Part `part` has no `POSITION`, `JOINTS_0` or `WEIGHTS_0` in the formats glTF loads, or a vertex
    /// with no positive weight.
    NotSkinned { part: usize },
    /// Part `part` has no indices.
    NotIndexed { part: usize },
    /// A vertex of part `part` is carried by `joint`, which no group holds.
    UngroupedJoint { part: usize, joint: u16 },
    /// Triangle `triangle` of part `part` has vertices in different groups: the body is not rigid.
    SplitTriangle { part: usize, triangle: u32 },
    /// Link number `link` joins a group to itself or names a group that does not exist.
    BadLink { link: usize },
    /// `joint` has no inverse bind matrix.
    MissingBind { joint: u16 },
}

impl std::fmt::Display for SkeletalRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSkinned { part } => write!(f, "part {part} is not skinned"),
            Self::NotIndexed { part } => write!(f, "part {part} has no indices"),
            Self::UngroupedJoint { part, joint } => write!(f, "part {part} has a vertex on joint {joint}, which no group holds"),
            Self::SplitTriangle { part, triangle } => write!(f, "triangle {triangle} of part {part} spans two groups: the body is not rigid"),
            Self::BadLink { link } => write!(f, "link {link} joins a group to itself or to one that does not exist"),
            Self::MissingBind { joint } => write!(f, "joint {joint} has no inverse bind matrix"),
        }
    }
}

impl std::error::Error for SkeletalRefusal {}

/// One part's attributes, borrowed, and the group of each of its vertices.
struct Part<'a> {
    positions: &'a [[f32; 3]],
    normals: Option<&'a [[f32; 3]]>,
    uvs: Option<&'a [[f32; 2]]>,
    indices: Vec<u32>,
    group_of: Vec<usize>,
}

fn read_part<'a>(
    p: usize,
    mesh: &'a Mesh,
    group_of_joint: &HashMap<u16, usize>,
) -> Result<Part<'a>, SkeletalRefusal> {
    let rig = Rig::of(mesh).ok_or(SkeletalRefusal::NotSkinned { part: p })?;
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return Err(SkeletalRefusal::NotSkinned { part: p });
    };
    let normals = match mesh.try_attribute_option(Mesh::ATTRIBUTE_NORMAL) {
        Ok(Some(VertexAttributeValues::Float32x3(n))) if n.len() == positions.len() => Some(n.as_slice()),
        _ => None,
    };
    let uvs = match mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) {
        Ok(Some(VertexAttributeValues::Float32x2(uv))) if uv.len() == positions.len() => Some(uv.as_slice()),
        _ => None,
    };
    let Some(index) = mesh.indices() else { return Err(SkeletalRefusal::NotIndexed { part: p }) };
    let indices: Vec<u32> = index.iter().map(|i| i as u32).collect();

    let mut group_of = Vec::with_capacity(positions.len());
    for v in 0..positions.len() {
        let joint = rig.dominant_joint(v).ok_or(SkeletalRefusal::NotSkinned { part: p })?;
        let group = *group_of_joint.get(&joint).ok_or(SkeletalRefusal::UngroupedJoint { part: p, joint })?;
        group_of.push(group);
    }
    for (t, tri) in indices.chunks_exact(3).enumerate() {
        let g = |i: u32| group_of.get(i as usize).copied();
        match (g(tri[0]), g(tri[1]), g(tri[2])) {
            (Some(a), Some(b), Some(c)) if a == b && b == c => {}
            _ => return Err(SkeletalRefusal::SplitTriangle { part: p, triangle: t as u32 }),
        }
    }
    Ok(Part { positions, normals, uvs, indices, group_of })
}

/// **Cut a skinned body along its skeleton**, once.
///
/// `parts` are the body's meshes (a skeleton's primitives), `inverse_binds` the skin's inverse bind
/// matrices by joint index, `groups` the pieces and `links` where they meet. See the [module](self)
/// for what is refused. Deterministic: the same inputs give the same meshes, bit for bit.
pub fn fracture_skeleton(
    parts: &[&Mesh],
    inverse_binds: &[Mat4],
    groups: &[JointGroup],
    links: &[JointLink],
) -> Result<SkeletalFracture, SkeletalRefusal> {
    let bind_of = |joint: u16| -> Result<Vec3, SkeletalRefusal> {
        inverse_binds.get(joint as usize).map(|m| m.inverse().w_axis.truncate()).ok_or(SkeletalRefusal::MissingBind { joint })
    };
    for (i, link) in links.iter().enumerate() {
        if link.a >= groups.len() || link.b >= groups.len() || link.a == link.b {
            return Err(SkeletalRefusal::BadLink { link: i });
        }
        bind_of(link.joint)?;
    }
    for group in groups {
        bind_of(group.anchor)?;
    }

    let mut group_of_joint: HashMap<u16, usize> = HashMap::new();
    for (g, group) in groups.iter().enumerate() {
        for &joint in &group.joints {
            group_of_joint.entry(joint).or_insert(g);
        }
    }
    let read: Vec<Part> = parts
        .iter()
        .enumerate()
        .map(|(p, mesh)| read_part(p, mesh, &group_of_joint))
        .collect::<Result<_, _>>()?;

    // Pieces: for each group and part, the part's vertices of that group compacted in source order.
    let mut sums = vec![(Vec3::ZERO, 0u32); groups.len()];
    let mut meshes: Vec<Vec<Option<Mesh>>> = (0..groups.len()).map(|_| Vec::with_capacity(read.len())).collect();
    for part in &read {
        let mut remap: Vec<u32> = vec![u32::MAX; part.positions.len()];
        let mut built: Vec<Option<Built>> = (0..groups.len()).map(|_| None).collect();
        for tri in part.indices.chunks_exact(3) {
            let g = part.group_of[tri[0] as usize];
            let slot = built[g].get_or_insert_with(Built::default);
            for &v in tri {
                let vi = v as usize;
                if remap[vi] == u32::MAX {
                    remap[vi] = slot.positions.len() as u32;
                    slot.positions.push(part.positions[vi]);
                    if let Some(n) = part.normals {
                        slot.normals.push(n[vi]);
                    }
                    if let Some(uv) = part.uvs {
                        slot.uvs.push(uv[vi]);
                    }
                }
                slot.indices.push(remap[vi]);
            }
        }
        // The centre counts every vertex of the group, not only the ones a triangle uses.
        for (v, &g) in part.group_of.iter().enumerate() {
            sums[g].0 += Vec3::from_array(part.positions[v]);
            sums[g].1 += 1;
        }
        for (g, slot) in built.into_iter().enumerate() {
            meshes[g].push(slot.map(|b| b.into_mesh(part.normals.is_some(), part.uvs.is_some())));
        }
    }

    let mut pieces = Vec::with_capacity(groups.len());
    for (g, group) in groups.iter().enumerate() {
        let (sum, n) = sums[g];
        let center = if n > 0 { sum / n as f32 } else { bind_of(group.anchor)? };
        pieces.push(SkeletalPiece { id: FragmentId(g as u32), anchor: group.anchor, meshes: std::mem::take(&mut meshes[g]), center });
    }

    let mut bonds = Vec::with_capacity(links.len());
    for link in links {
        let (a, b) = (link.a.min(link.b), link.a.max(link.b));
        let normal = (pieces[b].center - pieces[a].center).normalize_or(Vec3::Y);
        bonds.push(Bond { a: FragmentId(a as u32), b: FragmentId(b as u32), centroid: bind_of(link.joint)?, normal, area: 1.0 });
    }
    let centers: Vec<(FragmentId, Vec3)> = pieces.iter().map(|p| (p.id, p.center)).collect();
    let bonds = BondGraph::declared(&centers, bonds, groups.len());
    Ok(SkeletalFracture { pieces, bonds })
}

#[derive(Default)]
struct Built {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Built {
    fn into_mesh(self, normals: bool, uvs: bool) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        if normals {
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        }
        if uvs {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        }
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}
