//! **A skinned body cut along its skeleton.** Two closed boxes, each skinned rigidly to one joint:
//! the pieces are the boxes, the bond is the joint between them, and a severed bond leaves two
//! islands. A body that is not rigid, or whose joint no group holds, is refused rather than guessed.

use bevy::asset::RenderAssetUsages;
use bevy::math::{Mat4, Vec3};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy_carnage::{Bond, BondGraph, BondId, BondSet, FragmentId, JointGroup, JointLink, SkeletalRefusal, fracture_skeleton};

/// The twelve triangles of a box over eight vertices starting at `base`.
fn box_indices(base: u32) -> Vec<u32> {
    const CUBE: [[u32; 3]; 12] =
        [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [3, 7, 6], [3, 6, 2], [0, 4, 7], [0, 7, 3], [1, 2, 6], [1, 6, 5]];
    CUBE.iter().flatten().map(|i| base + i).collect()
}

fn box_positions(centre_x: f32) -> Vec<[f32; 3]> {
    let h = 0.5;
    [
        [-h, -h, -h],
        [h, -h, -h],
        [h, h, -h],
        [-h, h, -h],
        [-h, -h, h],
        [h, -h, h],
        [h, h, h],
        [-h, h, h],
    ]
    .iter()
    .map(|p| [p[0] + centre_x, p[1], p[2]])
    .collect()
}

/// Two boxes (x = 0 on joint 0, x = 2 on joint 1), each vertex at weight 1 on its own joint, plus an
/// optional triangle across both.
fn two_boxes(span: bool) -> Mesh {
    let mut positions = box_positions(0.0);
    positions.extend(box_positions(2.0));
    let joints: Vec<[u16; 4]> = (0..16).map(|v| [if v < 8 { 0 } else { 1 }, 0, 0, 0]).collect();
    let weights = vec![[1.0, 0.0, 0.0, 0.0]; 16];
    let mut indices = box_indices(0);
    indices.extend(box_indices(8));
    if span {
        indices.extend([0, 1, 8]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Joint 0 at the origin, joint 1 at x = 2: inverse binds are the inverse of those placements.
fn binds() -> Vec<Mat4> {
    vec![Mat4::IDENTITY, Mat4::from_translation(Vec3::new(-2.0, 0.0, 0.0))]
}

fn groups() -> Vec<JointGroup> {
    vec![JointGroup { joints: vec![0], anchor: 0 }, JointGroup { joints: vec![1], anchor: 1 }]
}

fn link() -> Vec<JointLink> {
    vec![JointLink { a: 1, b: 0, joint: 1 }]
}

fn positions(mesh: &Mesh) -> Vec<[f32; 3]> {
    match mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) {
        Ok(Some(VertexAttributeValues::Float32x3(p))) => p.clone(),
        _ => Vec::new(),
    }
}

#[test]
fn two_groups_and_a_link_are_two_pieces_one_bond_and_two_islands_once_severed() {
    let body = two_boxes(false);
    let made = fracture_skeleton(&[&body], &binds(), &groups(), &link()).expect("a rigid two-box body is cut");

    assert_eq!(made.pieces.len(), 2);
    for (i, piece) in made.pieces.iter().enumerate() {
        assert_eq!(piece.id, FragmentId(i as u32));
        let mesh = piece.meshes[0].as_ref().expect("each piece has its box");
        assert_eq!(positions(mesh).len(), 8, "a box's eight vertices, compacted");
        assert_eq!(mesh.indices().map(|i| i.len()), Some(36));
        assert!(mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).is_none(), "the skin attributes are dropped");
    }
    assert_eq!(made.pieces[0].center, Vec3::ZERO);
    assert_eq!(made.pieces[1].center, Vec3::new(2.0, 0.0, 0.0), "positions stay in bind space, not recentred");

    assert_eq!(made.bonds.len(), 1);
    let bond = made.bonds.bond(BondId(0)).expect("one bond");
    assert_eq!((bond.a, bond.b), (FragmentId(0), FragmentId(1)), "a < b whatever order the link named them");
    assert_eq!(bond.centroid, Vec3::new(2.0, 0.0, 0.0), "the joint's bind position");
    assert_eq!(bond.normal, Vec3::X, "from the lower piece toward the higher");

    let ids = [FragmentId(0), FragmentId(1)];
    let mut broken = BondSet::new(&made.bonds);
    assert_eq!(made.bonds.islands(&ids, &broken).len(), 1, "bonded, they are one body");
    broken.sever(BondId(0));
    assert_eq!(made.bonds.islands(&ids, &broken).len(), 2);
}

#[test]
fn a_triangle_across_two_groups_is_refused_not_guessed() {
    let body = two_boxes(true);
    let refused = fracture_skeleton(&[&body], &binds(), &groups(), &link()).expect_err("a split triangle is refused");
    assert_eq!(refused, SkeletalRefusal::SplitTriangle { part: 0, triangle: 24 });
}

#[test]
fn a_joint_no_group_holds_is_refused() {
    let body = two_boxes(false);
    let only_first = vec![JointGroup { joints: vec![0], anchor: 0 }];
    let refused = fracture_skeleton(&[&body], &binds(), &only_first, &[]).expect_err("joint 1 is in no group");
    assert_eq!(refused, SkeletalRefusal::UngroupedJoint { part: 0, joint: 1 });
}

#[test]
fn bad_input_is_refused_by_name() {
    let body = two_boxes(false);
    let loop_link = vec![JointLink { a: 0, b: 0, joint: 0 }];
    assert_eq!(fracture_skeleton(&[&body], &binds(), &groups(), &loop_link).unwrap_err(), SkeletalRefusal::BadLink { link: 0 });
    let lost = vec![JointLink { a: 0, b: 1, joint: 9 }];
    assert_eq!(fracture_skeleton(&[&body], &binds(), &groups(), &lost).unwrap_err(), SkeletalRefusal::MissingBind { joint: 9 });

    let mut unindexed = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    unindexed.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, 0.0, 0.0]; 3]);
    unindexed.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(vec![[0; 4]; 3]));
    unindexed.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[1.0, 0.0, 0.0, 0.0]; 3]);
    assert_eq!(fracture_skeleton(&[&unindexed], &binds(), &groups(), &[]).unwrap_err(), SkeletalRefusal::NotIndexed { part: 0 });

    let mut bare = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    bare.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, 0.0, 0.0]; 3]);
    assert_eq!(fracture_skeleton(&[&bare], &binds(), &groups(), &[]).unwrap_err(), SkeletalRefusal::NotSkinned { part: 0 });
}

#[test]
fn a_part_with_no_triangle_in_a_group_has_no_mesh_there() {
    let body = two_boxes(false);
    // A second part skinned only to joint 0.
    let mut half = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    half.insert_attribute(Mesh::ATTRIBUTE_POSITION, box_positions(0.0));
    half.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(vec![[0; 4]; 8]));
    half.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, vec![[1.0, 0.0, 0.0, 0.0]; 8]);
    half.insert_indices(Indices::U32(box_indices(0)));
    let made = fracture_skeleton(&[&body, &half], &binds(), &groups(), &link()).expect("two parts are cut");
    assert!(made.pieces[0].meshes[1].is_some());
    assert!(made.pieces[1].meshes[1].is_none(), "the second part has nothing on joint 1");
}

#[test]
fn two_cuts_of_one_body_are_the_same_bits() {
    let body = two_boxes(false);
    let a = fracture_skeleton(&[&body], &binds(), &groups(), &link()).expect("first");
    let b = fracture_skeleton(&[&body], &binds(), &groups(), &link()).expect("second");
    for (pa, pb) in a.pieces.iter().zip(&b.pieces) {
        let (ma, mb) = (pa.meshes[0].as_ref().expect("a"), pb.meshes[0].as_ref().expect("b"));
        let bits = |m: &Mesh| positions(m).iter().flatten().map(|f| f.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(ma), bits(mb));
        assert_eq!(pa.center.to_array().map(f32::to_bits), pb.center.to_array().map(f32::to_bits));
    }
    assert_eq!(a.bonds, b.bonds);
}

#[test]
fn a_declared_graph_drops_a_duplicate_and_an_out_of_range_bond() {
    let bond = |a: u32, b: u32, area: f32| Bond { a: FragmentId(a), b: FragmentId(b), centroid: Vec3::ZERO, normal: Vec3::Y, area };
    let centers: Vec<(FragmentId, Vec3)> = (0..3).map(|i| (FragmentId(i), Vec3::new(i as f32, 0.0, 0.0))).collect();
    let graph = BondGraph::declared(
        &centers,
        vec![bond(1, 2, 1.0), bond(0, 1, 2.0), bond(0, 1, 9.0), bond(0, 3, 1.0), bond(2, 1, 1.0), bond(1, 1, 1.0)],
        3,
    );
    assert_eq!(graph.len(), 2, "the duplicate, the out-of-range, the reversed and the self bond are dropped");
    let bonds = graph.bonds();
    assert_eq!((bonds[0].a, bonds[0].b, bonds[0].area), (FragmentId(0), FragmentId(1), 2.0), "the first of a duplicate wins, and bonds sort by (a, b)");
    assert_eq!((bonds[1].a, bonds[1].b), (FragmentId(1), FragmentId(2)));
    assert_eq!(graph.incident(FragmentId(1)), &[BondId(0), BondId(1)]);
    assert_eq!(graph.members(), &[FragmentId(0), FragmentId(1), FragmentId(2)]);
    assert_eq!(graph.center(FragmentId(2)), Some(Vec3::new(2.0, 0.0, 0.0)));
}
