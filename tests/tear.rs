//! **The geometry kernel's contract.**
//!
//! Everything here is a property of [`bevy_carnage::laceration::tear`] that a consumer could notice: the gap
//! is genuinely empty, the neighbours genuinely moved, the bed's floor is genuinely at the authored
//! depth, and bad input is genuinely refused rather than approximated. The last test freezes the
//! output bit for bit, because a tear that is not reproducible cannot be part of a hashed simulation.

use bevy::math::Vec3;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy_carnage::laceration::{GAPE_TAPER, Layers, Region, Scale, TearShape, digest, skin_patch, tear};

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

    // The gap is empty: no vertex the surviving triangles reference is inside the mouth, which is a
    // spindle — the half-width at a point of the middle row is `half · sin(π·u)^GAPE_TAPER`, `u` the
    // fraction of the 0.8 m path the point is along.
    let open_at = |v: Vec3| {
        let u = ((v.x + 0.4) / 0.8).clamp(0.0, 1.0);
        half * (std::f32::consts::PI * u).sin().max(0.0).powf(GAPE_TAPER)
    };
    let moved = positions(&torn.skin);
    for tri in &after {
        for i in tri {
            let Some(v) = moved.get(*i as usize) else {
                panic!("kept index {i} is out of range");
            };
            let d = flat_distance(*v, &path, Vec3::Y);
            let open = open_at(*v);
            assert!(d >= open - 1.0e-5, "vertex {i} at {v:?} sits {d} from the cut, inside a {open} half-gape there");
        }
    }
    // And it is a spindle, not a slot: in the middle the mouth is the full half-gape wide, and in the
    // last eighth of the cut, where `sin(π·u)^GAPE_TAPER` is under 0.57, the surviving skin comes well
    // inside it. A constant-width slot holds the full half-gape there too, and fails.
    let closest = |lo: f32, hi: f32| {
        after.iter().flatten().filter_map(|i| moved.get(*i as usize)).filter(|v| v.x >= lo && v.x <= hi).map(|v| flat_distance(*v, &path, Vec3::Y)).fold(f32::INFINITY, f32::min)
    };
    let (middle, end) = (closest(-0.05, 0.05), closest(0.3, 0.4));
    assert!(middle >= half - 1.0e-5, "the middle of the mouth is {middle} wide, under the {half} half-gape");
    assert!(end < 0.75 * half, "the mouth is {end} wide near its end against {middle} in its middle: a slot, not a spindle");

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

    // The floor is deepest in the middle, at the authored depth; it shallows toward the ends with the
    // gape. Whatever its depth, every bed vertex reads that depth in the strip — the rails `0`, the
    // floor at the middle exactly `bed_depth_mm`.
    let deepest = pos.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let want_floor = shape.bed_depth_mm / layers.span_mm();
    let expect_depth = -shape.bed_depth_mm / scale.mm_per_unit;
    assert!(
        (deepest - expect_depth).abs() < 1.0e-6,
        "the floor must sit {expect_depth} below the surface at its deepest, found {deepest}"
    );

    let mut floors = 0;
    let mut rails = 0;
    for (p, uv) in pos.iter().zip(uv.iter()) {
        let depth_mm = -p.y * scale.mm_per_unit;
        let want = depth_mm / layers.span_mm();
        assert!((uv[0] - want).abs() < 1.0e-4, "a bed vertex {depth_mm} mm down reads {} in the strip, expected {want}", uv[0]);
        if (p.y - deepest).abs() < 1.0e-6 {
            floors += 1;
            assert!((uv[0] - want_floor).abs() < 1.0e-4, "the floor reads {} in the strip, expected {want_floor}", uv[0]);
        } else if p.y.abs() < 1.0e-6 {
            rails += 1;
        }
        assert!(p.y <= 1.0e-6 && p.y >= deepest - 1.0e-6, "a bed vertex is above the skin or below the floor: {p:?}");
    }
    assert!(floors > 0 && rails > 0, "the bed must have both a floor and rails, got {floors}/{rails}");

    // The bands are the anatomy: 6 mm into a limb is fat, which is the whole reason for going through
    // `bevy_cross_section` rather than writing a depth fraction here.
    let (layer, _) = layers.at(shape.bed_depth_mm);
    assert_eq!(layer, bevy_carnage::cross_section::Layer::Fat, "6 mm into a limb is subcutaneous fat");
}

/// **A trough is lit through its mouth.** A bed's colour is the blood it is wet with times the light
/// that reaches it down the trough. Two cuts alike but for their depth lay the same vertices with the
/// same blood, so the deeper one must be darker at every vertex below the skin and no lighter at any
/// lip. Until 0.5.50 a bed was lit as if it were open skin, and a slit read as a pale ridge.
#[test]
fn a_deeper_trough_is_darker_below_the_skin() {
    let patch = skin_patch(CELLS, SIZE);
    let layers = Layers::for_region(Region::Limb);
    let scale = Scale::default();
    let bed = |bed_depth_mm: f32| {
        let shape = TearShape { half_width: CELL * 1.2, influence: 0.15, bed_depth_mm };
        let torn = tear(&patch, &middle_row(), Vec3::Y, &shape, Region::Limb, &layers, &scale).expect("the tear must produce a bed");
        let colour = match torn.bed.try_attribute_option(Mesh::ATTRIBUTE_COLOR) {
            Ok(Some(VertexAttributeValues::Float32x4(c))) => c.clone(),
            _ => Vec::new(),
        };
        let uv0 = match torn.bed.try_attribute_option(Mesh::ATTRIBUTE_UV_0) {
            Ok(Some(VertexAttributeValues::Float32x2(u))) => u.clone(),
            _ => Vec::new(),
        };
        (positions(&torn.bed), colour, uv0)
    };
    let (_, shallow, uv) = bed(6.0);
    let (pos, deep, uv_deep) = bed(120.0);
    assert!(!shallow.is_empty() && shallow.len() == deep.len() && uv == uv_deep, "two cuts alike but for depth must lay the same vertices");
    let (mut below, mut lips) = (0, 0);
    for ((p, d), s) in pos.iter().zip(&deep).zip(&shallow) {
        if p.y < -1.0e-6 {
            below += 1;
            assert!(d[0] < s[0] && d[1] < s[1] && d[2] < s[2], "a vertex {} mm down is no darker than its twin 6 mm deep: {d:?} against {s:?}", -p.y * 1000.0);
        } else {
            lips += 1;
            assert!(d[0] <= s[0] && d[1] <= s[1] && d[2] <= s[2], "a deeper trough's lip is lighter: {d:?} against {s:?}");
        }
    }
    assert!(below > 0 && lips > 0, "the bed must have both a floor and lips, got {below}/{lips}");
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

/// **No skin is left across the cut.** A slash at an angle to the mesh crosses triangles that do not
/// lie wholly inside the gape; kept, their vertices were pushed to opposite lips and each one was
/// stretched over the gap, so the wound opened as a row of pits with skin bridging them. Every point
/// of the path must be uncovered once the wound is open, at a gape narrower than a cell as well as
/// one wider.
#[test]
fn no_skin_is_left_across_a_diagonal_cut() {
    let patch = skin_patch(CELLS, SIZE);
    // Not along a mesh line and not through its diagonals: 3 cells across for every 1.3 down.
    let path = vec![Vec3::new(-0.36, 0.0, -0.17), Vec3::new(0.38, 0.0, 0.15)];
    let layers = Layers::for_region(Region::Limb);
    for half in [CELL * 0.3, CELL * 1.2] {
        let torn = tear(&patch, &path, Vec3::Y, &shape(half), Region::Limb, &layers, &Scale::default())
            .expect("a diagonal cut across a flat grid must tear");
        let moved = positions(&torn.skin);
        let flat = |i: u32| moved.get(i as usize).map(|v| (v.x, v.z));
        for [a, b, c] in triples(&torn.skin) {
            let (Some(a), Some(b), Some(c)) = (flat(a), flat(b), flat(c)) else {
                panic!("a kept index is out of range");
            };
            for k in 0..=200 {
                let p = path[0].lerp(path[1], k as f32 / 200.0);
                let edge = |(x0, z0): (f32, f32), (x1, z1): (f32, f32)| (x1 - x0) * (p.z - z0) - (z1 - z0) * (p.x - x0);
                let (e0, e1, e2) = (edge(a, b), edge(b, c), edge(c, a));
                let covered = (e0 > 1.0e-9 && e1 > 1.0e-9 && e2 > 1.0e-9) || (e0 < -1.0e-9 && e1 < -1.0e-9 && e2 < -1.0e-9);
                assert!(!covered, "at half-gape {half}, a kept triangle {a:?} {b:?} {c:?} lies over the cut at {p:?}");
            }
        }
    }
}

/// **A face bigger than the influence radius keeps its skin.** Its corners are too far from the cut
/// to be moved onto the lip, so clipping it would leave a hole the bed cannot fill. `0.5.3` did, and a
/// slash across a blockout's leg removed the leg's whole face, which read as a severed limb. A mesh
/// that coarse is too coarse to open, and the wound stays closed.
#[test]
fn a_cut_across_a_face_coarser_than_the_influence_removes_nothing() {
    // One cell: two triangles over a metre, with corners at least 0.5 m from a cut through the middle.
    let patch = skin_patch(1, SIZE);
    let path = vec![Vec3::new(-0.06, 0.0, -0.02), Vec3::new(0.06, 0.0, 0.02)];
    let torn = tear(&patch, &path, Vec3::Y, &shape(0.004), Region::Limb, &Layers::for_region(Region::Limb), &Scale::default())
        .expect("a cut across a flat patch must tear");
    assert_eq!(torn.removed_faces, 0, "a face with every corner beyond `influence` was clipped");
    assert_eq!(triples(&torn.skin).len(), triples(&patch).len(), "the coarse face lost a triangle");
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
    // Re-blessed in 0.5.3 (`AG-048`), both halves. The skin: a triangle with a vertex in the gape and
    // vertices on both lips is now clipped (this cut runs along a mesh line, so it grazes every
    // triangle beside it), and every vertex on the mouth lands on one lip curve. The bed: its walls
    // rise to that curve, follow the rail's wander and run past both ends as far as the clip did.
    // Before, a diagonal cut opened as a row of pits bridged by skin, with nothing between the walls
    // and the lips. Re-blessed again in 0.5.33 (`AG-106`), both halves: the gape is a spindle,
    // `sin(π·u)^GAPE_TAPER` of the half-width along the cut, and the bed's floor shallows with it, where
    // it had been a constant-width slot with square ends that read as a ribbon laid on the skin.
    const FROZEN: u64 = 0x5d7e_ee46_aa39_3f74;
    const FROZEN_BED: u64 = 0xef18_0e14_5aae_8af6;
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

