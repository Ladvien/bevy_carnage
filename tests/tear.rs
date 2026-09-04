//! **The geometry kernel's contract.**
//!
//! Everything here is a property of [`bevy_carnage::laceration::tear`] that a consumer could notice: the gap
//! is genuinely empty, the neighbours genuinely moved, the bed's floor is genuinely at the authored
//! depth, and bad input is genuinely refused rather than approximated. The last test freezes the
//! output bit for bit, because a tear that is not reproducible cannot be part of a hashed simulation.

use bevy::math::Vec3;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy_carnage::laceration::{Layers, Region, Scale, TearShape, digest, skin_patch, tear};

/// The grid every geometric test cuts: 21 × 21 vertices over one metre, so a cell is 50 mm and a
/// 60 mm half-gape swallows whole triangles rather than clipping single ones.
const CELLS: u32 = 20;
const SIZE: f32 = 1.0;
const CELL: f32 = SIZE / CELLS as f32;

/// A straight slash down the middle row of the patch (`z = 0`), stopping short of both edges so the
/// end-cap behaviour is exercised rather than falling off the mesh.
fn middle_row() -> Vec<Vec3> {
    vec![Vec3::new(-0.4, 0.0, 0.0), Vec3::new(0.4, 0.0, 0.0)]
}

fn shape(half_width: f32) -> TearShape {
    TearShape { half_width, influence: 0.15, bed_depth_mm: 6.0 }
}

fn positions(mesh: &Mesh) -> Vec<Vec3> {
    match mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) {
        Ok(Some(VertexAttributeValues::Float32x3(p))) => p.iter().map(|p| Vec3::from_array(*p)).collect(),
        _ => Vec::new(),
    }
}

fn uv1(mesh: &Mesh) -> Vec<[f32; 2]> {
    match mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_1) {
        Ok(Some(VertexAttributeValues::Float32x2(uv))) => uv.clone(),
        _ => Vec::new(),
    }
}

fn triples(mesh: &Mesh) -> Vec<[u32; 3]> {
    let flat: Vec<u32> = match mesh.try_indices_option() {
        Ok(Some(Indices::U32(i))) => i.clone(),
        Ok(Some(Indices::U16(i))) => i.iter().map(|i| u32::from(*i)).collect(),
        _ => (0..positions(mesh).len() as u32).collect(),
    };
    flat.chunks_exact(3).filter_map(|t| <[u32; 3]>::try_from(t).ok()).collect()
}

/// Distance from a point to the polyline **measured in the plane of the normal** — the same quantity
/// the kernel thresholds on, reimplemented here so the test is not asserting against the code it is
/// testing.
fn flat_distance(v: Vec3, path: &[Vec3], normal: Vec3) -> f32 {
    let mut best = f32::INFINITY;
    for pair in path.windows(2) {
        let [a, b] = pair else { continue };
        let d = *b - *a;
        let len = d.length();
        if len <= 0.0 {
            continue;
        }
        let dir = d / len;
        let s = (v - *a).dot(dir).clamp(0.0, len);
        let off = v - (*a + dir * s);
        best = best.min((off - normal * off.dot(normal)).length());
    }
    best
}

#[test]
fn tearing_a_grid_removes_faces_and_displaces_neighbours() {
    let patch = skin_patch(CELLS, SIZE);
    assert_eq!(positions(&patch).len(), 21 * 21, "the helper must build a 21x21 grid");
    let before = triples(&patch).len();
    let path = middle_row();
    let half = CELL * 1.2;

    let torn = tear(
        &patch,
        &path,
        Vec3::Y,
        &shape(half),
        Region::Limb,
        &Layers::for_region(Region::Limb),
        &Scale::default(),
    )
    .expect("a flat grid cut down its middle row is the simplest tear there is");

    assert!(torn.removed_faces > 0, "a 60 mm half-gape across 50 mm cells must swallow whole triangles");
    assert!(torn.displaced_vertices > 0, "the lips have to move, or nothing opened");

    let after = triples(&torn.skin);
    assert_eq!(
        after.len() + torn.removed_faces as usize,
        before,
        "every original triangle is either kept or counted as removed — nothing may vanish silently"
    );
    assert_eq!(
        positions(&torn.skin).len(),
        positions(&patch).len(),
        "the vertex buffer must keep its length, or a skinned mesh's joint weights stop lining up"
    );

    // The gap is empty: no vertex the surviving triangles reference is inside the mouth.
    let moved = positions(&torn.skin);
    for tri in &after {
        for i in tri {
            let Some(v) = moved.get(*i as usize) else {
                panic!("kept index {i} is out of range");
            };
            let d = flat_distance(*v, &path, Vec3::Y);
            assert!(
                d >= half - 1.0e-5,
                "vertex {i} at {v:?} sits {d} from the cut, inside a {half} half-gape"
            );
        }
    }

    // The displacement is local: a vertex outside the influence radius is exactly where it started.
    let original = positions(&patch);
    let far = original
        .iter()
        .zip(moved.iter())
        .filter(|(o, _)| flat_distance(**o, &path, Vec3::Y) > 0.15)
        .count();
    assert!(far > 0, "the patch must have vertices outside the influence radius for this to mean anything");
    for (o, m) in original.iter().zip(moved.iter()) {
        if flat_distance(*o, &path, Vec3::Y) > 0.15 {
            assert_eq!(o, m, "a vertex beyond `influence` moved; the tear is not local");
        }
    }
}

#[test]
fn the_bed_floor_sits_at_the_authored_depth() {
    let patch = skin_patch(CELLS, SIZE);
    let layers = Layers::for_region(Region::Limb);
    let scale = Scale::default();
    let shape = shape(CELL * 1.2);
    let torn = tear(&patch, &middle_row(), Vec3::Y, &shape, Region::Limb, &layers, &scale)
        .expect("the tear must produce a bed");

    let pos = positions(&torn.bed);
    let uv = uv1(&torn.bed);
    assert!(!pos.is_empty(), "the bed has no geometry");
    assert_eq!(pos.len(), uv.len(), "every bed vertex needs a UV_1, or the strip cannot paint it");

    // The floor is the deepest set of vertices along the surface normal; the rails are on the skin.
    let deepest = pos.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let want_floor = shape.bed_depth_mm / layers.span_mm();
    let expect_depth = -shape.bed_depth_mm / scale.mm_per_unit;
    assert!(
        (deepest - expect_depth).abs() < 1.0e-6,
        "the floor must sit {expect_depth} below the surface, found {deepest}"
    );

    let mut floors = 0;
    let mut rails = 0;
    for (p, uv) in pos.iter().zip(uv.iter()) {
        if (p.y - deepest).abs() < 1.0e-6 {
            floors += 1;
            assert!(
                (uv[0] - want_floor).abs() < 1.0e-4,
                "a floor vertex reads depth {} in the strip, expected {want_floor}",
                uv[0]
            );
        } else if p.y.abs() < 1.0e-6 {
            rails += 1;
            assert!(uv[0].abs() < 1.0e-4, "a rail is ON the skin, so its depth must be 0, found {}", uv[0]);
        } else {
            panic!("a bed vertex is neither on the skin nor on the floor: {p:?}");
        }
    }
    assert!(floors > 0 && rails > 0, "the bed must have both a floor and rails, got {floors}/{rails}");

    // The bands are the anatomy: 6 mm into a limb is fat, which is the whole reason for going through
    // `bevy_cross_section` rather than writing a depth fraction here.
    let (layer, _) = layers.at(shape.bed_depth_mm);
    assert_eq!(layer, bevy_carnage::cross_section::Layer::Fat, "6 mm into a limb is subcutaneous fat");
}

/// **Every face of the bed is wound the way its normal says.** A wound bed is looked at from
/// inside the trough with back-face culling on, so a triangle wound against its authored normal is
/// not a shading artefact — it is invisible. The end caps shipped that way in 0.1.0 and nothing here
/// noticed, because the floor and rails were checked by position alone.
#[test]
fn every_bed_face_is_wound_to_its_normal() {
    let patch = skin_patch(CELLS, SIZE);
    let layers = Layers::for_region(Region::Limb);
    let scale = Scale::default();
    let torn = tear(&patch, &middle_row(), Vec3::Y, &shape(CELL * 1.2), Region::Limb, &layers, &scale)
        .expect("the tear must produce a bed");
    let pos = positions(&torn.bed);
    let nrm: Vec<Vec3> = match torn.bed.try_attribute_option(Mesh::ATTRIBUTE_NORMAL) {
        Ok(Some(VertexAttributeValues::Float32x3(n))) => n.iter().map(|n| Vec3::from_array(*n)).collect(),
        _ => Vec::new(),
    };
    assert_eq!(pos.len(), nrm.len(), "every bed vertex needs a normal");
    let tris = triples(&torn.bed);
    assert!(!tris.is_empty(), "the bed has no faces");
    for [a, b, c] in tris {
        let (Some(pa), Some(pb), Some(pc), Some(na)) =
            (pos.get(a as usize), pos.get(b as usize), pos.get(c as usize), nrm.get(a as usize))
        else {
            panic!("a bed index is out of range");
        };
        let geometric = (*pb - *pa).cross(*pc - *pa);
        assert!(
            geometric.dot(*na) > 0.0,
            "face {a}-{b}-{c} is wound against its normal: geometric {geometric:?} vs authored {na:?}"
        );
    }
}

#[test]
fn a_wider_gape_removes_at_least_as_many_faces() {
    let patch = skin_patch(CELLS, SIZE);
    let path = middle_row();
    let layers = Layers::for_region(Region::Limb);
    let mut last = 0u32;
    let mut widest = 0u32;
    for step in 0..12u32 {
        let half = CELL * 0.1 * step as f32;
        let torn = tear(&patch, &path, Vec3::Y, &shape(half), Region::Limb, &layers, &Scale::default())
            .expect("every half-width in this sweep is valid");
        assert!(
            torn.removed_faces >= last,
            "removal is not monotone in half_width: {half} removed {} after {last}",
            torn.removed_faces
        );
        last = torn.removed_faces;
        widest = torn.removed_faces;
    }
    assert!(widest > 0, "the widest gape in the sweep removed nothing at all");
}

#[test]
fn bad_input_is_refused() {
    let patch = skin_patch(CELLS, SIZE);
    let layers = Layers::for_region(Region::Limb);
    let s = shape(CELL);
    let ok = |path: &[Vec3], mesh: &Mesh, normal: Vec3| {
        tear(mesh, path, normal, &s, Region::Limb, &layers, &Scale::default()).is_some()
    };

    // No positions at all.
    let empty = Mesh::new(PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
    assert!(!ok(&middle_row(), &empty, Vec3::Y), "a mesh with no positions must be refused");

    // A path that is not a path.
    assert!(!ok(&[Vec3::ZERO], &patch, Vec3::Y), "one point is not a cut");
    assert!(!ok(&[], &patch, Vec3::Y), "an empty path is not a cut");

    // Non-finite input, in each of the three places it can arrive.
    assert!(!ok(&[Vec3::ZERO, Vec3::new(f32::NAN, 0.0, 0.0)], &patch, Vec3::Y), "NaN in the path must be refused");
    assert!(
        !ok(&[Vec3::ZERO, Vec3::new(f32::INFINITY, 0.0, 0.0)], &patch, Vec3::Y),
        "an infinite path point must be refused"
    );
    assert!(!ok(&middle_row(), &patch, Vec3::ZERO), "a zero normal has no side, so there is no tear");
    assert!(!ok(&middle_row(), &patch, Vec3::new(f32::NAN, 1.0, 0.0)), "a NaN normal must be refused");

    // A degenerate shape, and a path running straight into the surface.
    let nan_shape = TearShape { half_width: f32::NAN, influence: 0.1, bed_depth_mm: 6.0 };
    assert!(
        tear(&patch, &middle_row(), Vec3::Y, &nan_shape, Region::Limb, &layers, &Scale::default()).is_none(),
        "a NaN half_width must be refused"
    );
    let negative = TearShape { half_width: -0.1, influence: 0.1, bed_depth_mm: 6.0 };
    assert!(
        tear(&patch, &middle_row(), Vec3::Y, &negative, Region::Limb, &layers, &Scale::default()).is_none(),
        "a negative half_width is not a width"
    );
    assert!(
        !ok(&[Vec3::ZERO, Vec3::Y], &patch, Vec3::Y),
        "a path along the normal defines no side, so every segment is dropped and there is nothing to cut"
    );

    // A zero-width tear is legal and does nothing: this is the state every wound starts in.
    let closed = TearShape { half_width: 0.0, influence: 0.1, bed_depth_mm: 6.0 };
    let torn = tear(&patch, &middle_row(), Vec3::Y, &closed, Region::Limb, &layers, &Scale::default())
        .expect("a closed wound is a valid wound");
    assert_eq!(torn.removed_faces, 0, "a closed wound removes nothing");
    assert_eq!(triples(&torn.skin).len(), triples(&patch).len(), "a closed wound keeps every triangle");
}

/// **The golden.** A tear is a pure function of its arguments, so this number is the whole
/// determinism claim: same grid, same path, same shape, same bits, on every machine and every run.
///
/// If a change moves it, that change re-blesses this constant deliberately, in the same commit, with
/// the reason — exactly as `bevy_carnage::cross_section::the_strips_are_frozen` does for its strips.
#[test]
fn the_tear_is_frozen() {
    // Both halves of the output are frozen: `examples/laceration_curve.rs` prints exactly these two
    // lines, so a reader can check the crate against its own golden without running the suite.
    const FROZEN: u64 = 0xafb3_d3cb_bc85_b028;
    // Re-blessed in 0.1.1: the two end caps were wound against their normals in 0.1.0 and are now
    // wound to them (`every_bed_face_is_wound_to_its_normal`). Only the cap index order moved.
    const FROZEN_BED: u64 = 0x18da_2f85_86e7_9831;
    let patch = skin_patch(CELLS, SIZE);
    let torn = tear(
        &patch,
        &middle_row(),
        Vec3::Y,
        &shape(CELL * 1.2),
        Region::Limb,
        &Layers::for_region(Region::Limb),
        &Scale::default(),
    )
    .expect("the frozen case must tear");
    let got = digest(&torn.skin);
    assert_eq!(got, FROZEN, "the tear moved: expected {FROZEN:#018x}, got {got:#018x}");
    let got_bed = digest(&torn.bed);
    assert_eq!(got_bed, FROZEN_BED, "the bed moved: expected {FROZEN_BED:#018x}, got {got_bed:#018x}");

    // And it is stable within a run, which is the cheap half of the same claim.
    let again = tear(
        &patch,
        &middle_row(),
        Vec3::Y,
        &shape(CELL * 1.2),
        Region::Limb,
        &Layers::for_region(Region::Limb),
        &Scale::default(),
    )
    .expect("the frozen case must tear twice");
    assert_eq!(digest(&again.skin), got, "two tears of the same input disagreed");
    assert_eq!(digest(&again.bed), digest(&torn.bed), "two beds of the same input disagreed");
}

