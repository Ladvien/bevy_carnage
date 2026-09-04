//! **The geometry kernel: a cut along a surface, opened.**
//!
//! One function, [`tear`], and it is a pure function of its arguments — no clock, no RNG crate, no
//! global. That is what lets [`digest`] freeze its output in a test.
//!
//! # The model, and whose it is
//!
//! Kamarianakis, Protopsaltis, Angelis, Tamiolakis & Papagiannakis, *"Progressive tearing and
//! cutting of soft-bodies in high-performance virtual reality"*, ICAT-EGVE 2022,
//! `doi:10.48550/arXiv.2209.08531`, describes a tear as three things, and this module is those three
//! things:
//!
//! 1. **A user-defined width along a sampled path** (§3.1). The path is a polyline the caller hands
//!    in; the width is [`TearShape::half_width`] either side of it. Not a physical crack criterion —
//!    the paper's tear is authored, and so is this one, which is why a gape can be driven from a
//!    time curve instead of a stress solve.
//! 2. **Faces inside the gap are clipped** (§3.1). Here: a triangle whose three vertices all lie
//!    within `half_width` of the polyline is removed outright. The paper subdivides the partially
//!    covered faces; this crate snaps their inside vertices to the rail instead, which keeps the
//!    vertex buffer index-stable and therefore keeps skinning weights valid — see below.
//! 3. **Auxiliary particles displaced normal to and away from the tear segments** (§3.3.4), which is
//!    what actually *opens* the wound rather than merely punching a hole in it. Here: every vertex
//!    within [`TearShape::influence`] of the polyline moves along `±side` — the in-surface
//!    perpendicular `normal × segment_direction` — by `half_width · (1 - d/influence)²`, so the
//!    displacement falls to zero with a zero derivative at the edge of the influence radius and the
//!    surface does not crease there.
//!
//! The lips do not spring back: a laceration is the ductile case of O'Brien, Bargteil & Hodgins,
//! *"Graphical modeling and animation of ductile fracture"*, SIGGRAPH 2002,
//! `doi:10.1145/566570.566579`, where plastic deformation is retained ahead of separation. The
//! caller drives `half_width` monotonically (see [`crate::laceration::gape`]) and this module never closes
//! anything.
//!
//! # Why the vertex buffer keeps its length
//!
//! **A vertex is never added to or removed from the skin mesh — only moved.** A skinned character's
//! `ATTRIBUTE_JOINT_INDEX` and `ATTRIBUTE_JOINT_WEIGHT` are per-vertex, and so is every other
//! attribute a consumer might have authored; carrying them through a re-indexing is a copy per
//! attribute per retear, and getting it wrong is a limb that follows the wrong bone. Keeping the
//! buffer identical and rewriting only positions and indices makes that class of bug unreachable.
//! The cost is that vertices inside the gap can end up referenced by nothing, which wastes a few
//! bytes of vertex buffer and nothing else.
//!
//! # Where the numbers come from
//!
//! [`RAIL_WANDER`] and [`WANDER_MM`] are **this crate's own** — no paper in the corpus tabulates the
//! raggedness of a wound margin. Everything else geometric is the caller's authoring.

use bevy::asset::RenderAssetUsages;
use bevy::log::warn_once;
use bevy::math::Vec3;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use crate::cross_section::{Layers, Region, Scale, SkinPlane, uv1_at};

/// **How far the rail wanders outward, as a fraction of `half_width`.** This crate's own number.
///
/// A blade does not leave a straight line: the margin of a laceration is ragged because skin fails
/// through a collagen network, not along a ruler. Fifteen percent is enough to read as torn at arm's
/// length and small enough that the mouth still looks like the cut the author placed.
///
/// **Outward only, deliberately.** The wander is added to `half_width`, never subtracted, so no
/// vertex ever ends up *inside* the gap the removal pass just cleared — the invariant
/// `tearing_a_grid_removes_faces_and_displaces_neighbours` pins.
pub const RAIL_WANDER: f32 = 0.15;

/// **Millimetres between wander samples.** This crate's own number, in millimetres rather than mesh
/// units so the raggedness is the same physical size whatever a caller's mesh is authored at.
pub const WANDER_MM: f32 = 8.0;

/// **The shape of the wound right now** — a snapshot, not a schedule. The caller advances
/// `half_width` from [`crate::laceration::gape`] and hands in a new one each time the gape moves.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TearShape {
    /// Half the current gape, in mesh units: the distance from the path to each lip.
    pub half_width: f32,
    /// How far from the path the displacement still reaches, in mesh units. Vertices further out
    /// than this do not move at all, which is what keeps a tear local to the wound instead of
    /// deforming the whole limb.
    pub influence: f32,
    /// How deep the wound bed's floor sits below the surface, in **millimetres** — the unit the
    /// anatomy is measured in, converted through [`Scale::mm_per_unit`] at the last moment.
    pub bed_depth_mm: f32,
}

impl Default for TearShape {
    /// A closed wound: no width, no bed. The identity the plugin starts every laceration from.
    fn default() -> Self {
        Self { half_width: 0.0, influence: 0.02, bed_depth_mm: 6.0 }
    }
}

/// **What one tear produced**: the surface with a hole in it, the trough behind the hole, and the
/// two counts that make a test able to say the tear did something.
#[derive(Clone, Debug)]
pub struct Torn {
    /// The subject's surface, minus the cleared faces, with the surviving lips pushed apart. Carries
    /// every attribute the input had, at the same vertex count.
    pub skin: Mesh,
    /// The wound bed: two walls and a floor per path segment, `UV_1` banded by depth so
    /// `bevy_cross_section`'s strip material paints skin, fat and muscle at their measured depths.
    pub bed: Mesh,
    /// Triangles the gap swallowed.
    pub removed_faces: u32,
    /// Vertices that moved — snapped to a rail, pushed by the influence falloff, or both.
    pub displaced_vertices: u32,
}

/// **Cut `mesh` along `path` and open it.**
///
/// `path` is a polyline **in the mesh's own space**, sampled densely enough that the surface does not
/// curve much between points; `normal` is the surface normal there, and defines what "sideways"
/// means — every displacement is along `normal × segment_direction`, never along the normal itself,
/// so a tear slides the skin apart rather than inflating it.
///
/// `region`, `layers` and `scale` are the anatomy: they decide what depth the bed's floor reads as
/// in the cross-section strip, and `region` also seeds the rail wander so a torso and a limb do not
/// tear with the identical margin.
///
/// Returns `None`, having warned once, when there is nothing honest to do: no `Float32x3` positions,
/// a path shorter than two points, a non-finite value anywhere in the inputs, a degenerate normal, a
/// topology that is not a triangle list, or a mesh whose vertex data has been extracted to the render
/// world (author the source with `RenderAssetUsages::default()`, which keeps the main-world copy).
pub fn tear(
    mesh: &Mesh,
    path: &[Vec3],
    normal: Vec3,
    shape: &TearShape,
    region: Region,
    layers: &Layers,
    scale: &Scale,
) -> Option<Torn> {
    if path.len() < 2 {
        warn_once!("bevy_laceration: a tear needs at least two path points, got {}", path.len());
        return None;
    }
    if path.iter().any(|p| !p.is_finite()) {
        warn_once!("bevy_laceration: the tear path holds a non-finite point; refusing to cut");
        return None;
    }
    if !shape.half_width.is_finite() || shape.half_width < 0.0 {
        warn_once!("bevy_laceration: half_width {} is not a width", shape.half_width);
        return None;
    }
    if !shape.influence.is_finite() || shape.influence < 0.0 || !shape.bed_depth_mm.is_finite() {
        warn_once!("bevy_laceration: the tear shape holds a non-finite dial; refusing to cut");
        return None;
    }
    let Some(n) = unit(normal) else {
        warn_once!("bevy_laceration: the surface normal {normal:?} has no direction");
        return None;
    };
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        warn_once!(
            "bevy_laceration: only a TriangleList can be torn, this mesh is {:?}",
            mesh.primitive_topology()
        );
        return None;
    }

    let positions = match mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) {
        Ok(Some(VertexAttributeValues::Float32x3(p))) => p,
        Ok(Some(_)) => {
            warn_once!("bevy_laceration: positions must be Float32x3; this mesh's are not");
            return None;
        }
        Ok(None) => {
            warn_once!("bevy_laceration: the mesh has no position attribute");
            return None;
        }
        Err(e) => {
            // The mesh was authored render-world-only and its vertex data is gone from the main
            // world. Bevy's `Mesh::attribute` would panic here; this crate refuses instead.
            warn_once!("bevy_laceration: cannot read positions ({e}) — author the source mesh with RenderAssetUsages::default()");
            return None;
        }
    };
    let verts: Vec<Vec3> = positions.iter().map(|p| Vec3::from_array(*p)).collect();
    if verts.is_empty() {
        warn_once!("bevy_laceration: the mesh has no vertices");
        return None;
    }

    let segs = segments(path, n);
    if segs.is_empty() {
        warn_once!("bevy_laceration: every path segment is degenerate or parallel to the normal");
        return None;
    }
    let scale = sane(scale);
    let seed = region_seed(region);

    // Pass one: measure. Every later decision reads these, so the distance is computed once.
    let near: Vec<Option<Near>> = verts.iter().map(|v| nearest(*v, &segs, n)).collect();

    // Pass two: move. The snap opens the mouth out to the rail; the falloff is the bulge behind it.
    let inv_influence = if shape.influence > 1.0e-9 { shape.influence.recip() } else { 0.0 };
    let mut moved = 0u32;
    let mut out: Vec<[f32; 3]> = Vec::with_capacity(verts.len());
    for (v, near) in verts.iter().zip(near.iter()) {
        let mut p = *v;
        if let Some(near) = near {
            let lateral = near.sign * n.cross(near.dir);
            if near.dist < shape.half_width {
                // **Pushed sideways only, never repositioned.** Setting the vertex to
                // `nearest + rail·lateral` would look right in the middle of the cut and pinch at
                // its ends, where the nearest point is the last path vertex and every vertex beyond
                // it would be dragged back onto that one point. Moving along `lateral` until the
                // lateral coordinate *is* the rail leaves the along-path and out-of-surface
                // components exactly as authored, so a slash ends in a slot rather than a purse.
                let lat = (*v - near.point).dot(lateral);
                let want = rail(shape.half_width, near.along, near.sign, seed, &scale);
                if lat < want {
                    p += lateral * (want - lat);
                }
            }
            if inv_influence > 0.0 && near.dist < shape.influence {
                let falloff = 1.0 - near.dist * inv_influence;
                p += lateral * (shape.half_width * falloff * falloff);
            }
        }
        if p != *v {
            moved += 1;
        }
        out.push(p.to_array());
    }

    // Pass three: clip. A triangle entirely inside the gap has nothing left to draw.
    let inside = |i: usize| near.get(i).and_then(|n| n.as_ref()).is_some_and(|n| n.dist < shape.half_width);
    let mut kept: Vec<u32> = Vec::new();
    let mut removed = 0u32;
    let mut u16_indices = false;
    let mut walk = |a: u32, b: u32, c: u32| {
        if inside(a as usize) && inside(b as usize) && inside(c as usize) {
            removed += 1;
        } else {
            kept.extend_from_slice(&[a, b, c]);
        }
    };
    match mesh.try_indices_option() {
        Ok(Some(Indices::U32(idx))) => {
            for tri in idx.chunks_exact(3) {
                if let [a, b, c] = *tri {
                    walk(a, b, c);
                }
            }
        }
        Ok(Some(Indices::U16(idx))) => {
            u16_indices = true;
            for tri in idx.chunks_exact(3) {
                if let [a, b, c] = *tri {
                    walk(u32::from(a), u32::from(b), u32::from(c));
                }
            }
        }
        Ok(None) => {
            // Unindexed: the triples are implicit, and the output gains an index buffer.
            for t in 0..(verts.len() / 3) as u32 {
                walk(t * 3, t * 3 + 1, t * 3 + 2);
            }
        }
        Err(e) => {
            warn_once!("bevy_laceration: cannot read indices ({e}) — author the source mesh with RenderAssetUsages::default()");
            return None;
        }
    }

    // The skin is the input with two buffers rewritten, so every other attribute — normals, `UV_0`,
    // joint indices, joint weights, vertex colours, anything custom — arrives untouched by
    // construction rather than by a copy this crate has to remember to make.
    let mut skin = mesh.clone();
    skin.try_insert_attribute(Mesh::ATTRIBUTE_POSITION, out).ok()?;
    if u16_indices && verts.len() <= usize::from(u16::MAX) {
        skin.try_insert_indices(Indices::U16(kept.iter().map(|i| *i as u16).collect())).ok()?;
    } else {
        skin.try_insert_indices(Indices::U32(kept)).ok()?;
    }

    let bed = bed_mesh(&segs, n, shape, seed, layers, &scale);
    Some(Torn { skin, bed, removed_faces: removed, displaced_vertices: moved })
}

/// **FNV-1a over a mesh's position bit patterns.** The determinism oracle: two runs of the same tear
/// agree bit for bit, so a golden can pin the number and a retune has to re-bless it deliberately.
///
/// Positions only — they are what the tear computes. A mesh with no readable `Float32x3` positions
/// hashes to the bare FNV offset basis, so "no geometry" is a stable value rather than a panic.
pub fn digest(mesh: &Mesh) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    if let Ok(Some(VertexAttributeValues::Float32x3(pos))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) {
        for p in pos {
            for c in p {
                for byte in c.to_bits().to_le_bytes() {
                    h ^= u64::from(byte);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
    }
    h
}

/// **A flat patch of skin to cut**, `cells × cells` quads across `size` mesh units in the XZ plane,
/// centred on the origin, facing `+Y`, with `UV_0` running `[0, 1]` both ways.
///
/// Shipped rather than left to the examples because a crate whose input is a mesh should hand you one
/// you can feed it, and because a frozen digest needs geometry that this crate — not Bevy's mesh
/// generators, which are free to retessellate between versions — is responsible for.
///
/// `cells` is clamped to at least one and `size` to a positive finite number, so there is no input
/// that produces an empty or non-finite mesh.
pub fn skin_patch(cells: u32, size: f32) -> Mesh {
    let cells = cells.max(1);
    let size = if size.is_finite() && size > 0.0 { size } else { 1.0 };
    let n = cells + 1;
    let step = size / cells as f32;
    let half = size * 0.5;
    let mut pos = Vec::with_capacity((n * n) as usize);
    let mut nrm = Vec::with_capacity((n * n) as usize);
    let mut uv = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            pos.push([i as f32 * step - half, 0.0, j as f32 * step - half]);
            nrm.push([0.0, 1.0, 0.0]);
            uv.push([i as f32 / cells as f32, j as f32 / cells as f32]);
        }
    }
    let mut idx = Vec::with_capacity((cells * cells * 6) as usize);
    for j in 0..cells {
        for i in 0..cells {
            let a = j * n + i;
            let (b, c, d) = (a + 1, a + n, a + n + 1);
            // Counter-clockwise seen from +Y, which is where the normal points.
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_POSITION, pos).ok();
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm).ok();
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_UV_0, uv).ok();
    mesh.try_insert_indices(Indices::U32(idx)).ok();
    mesh
}

/// **Which way the cut runs**, as the chord of its path — the direction [`crate::laceration::anisotropy`] wants.
///
/// The chord rather than a per-segment direction because the anisotropy scales the wound as a whole:
/// a curved laceration has one net orientation against the Langer lines, and averaging the segments
/// would let an S-shaped path cancel itself out to nothing.
///
/// Falls back to the first non-degenerate segment when the endpoints coincide (a closed loop), and to
/// `[0, 0, 0]` when there is no direction at all — which [`crate::laceration::anisotropy`] reads as isotropic.
pub fn tear_direction(path: &[Vec3]) -> [f32; 3] {
    let (Some(first), Some(last)) = (path.first(), path.last()) else {
        return [0.0; 3];
    };
    if let Some(d) = unit(*last - *first) {
        return d.to_array();
    }
    for pair in path.windows(2) {
        if let [a, b] = pair {
            if let Some(d) = unit(*b - *a) {
                return d.to_array();
            }
        }
    }
    [0.0; 3]
}

/// One segment of the path, with everything the passes need precomputed once.
struct Seg {
    /// Start point.
    a: Vec3,
    /// Unit direction.
    dir: Vec3,
    /// Length in mesh units.
    len: f32,
    /// Arc length at `a`, accumulated over the segments that survived.
    along: f32,
}

/// The nearest point on the polyline to a vertex, and which lip that vertex belongs to.
struct Near {
    /// In-surface distance: the offset with its `normal` component removed. A vertex sitting a
    /// centimetre *above* the cut is not inside it.
    dist: f32,
    /// The nearest point itself.
    point: Vec3,
    /// The direction of the segment the nearest point lies on.
    dir: Vec3,
    /// `+1` or `-1`: which side of the path, and therefore which lip. Never zero — a vertex exactly
    /// on the path is assigned to `+1` so the choice is deterministic rather than left to a float.
    sign: f32,
    /// Arc length at the nearest point, for the wander lookup.
    along: f32,
}

/// Split the path into usable segments, dropping the two that cannot define a side: a zero-length
/// step, and one running along the surface normal (where `normal × dir` vanishes).
fn segments(path: &[Vec3], normal: Vec3) -> Vec<Seg> {
    let mut segs = Vec::with_capacity(path.len().saturating_sub(1));
    let mut along = 0.0;
    for pair in path.windows(2) {
        let [a, b] = pair else { continue };
        let d = *b - *a;
        let len = d.length();
        if !(len > 1.0e-9) {
            continue;
        }
        let dir = d / len;
        if normal.cross(dir).length_squared() < 1.0e-12 {
            continue;
        }
        segs.push(Seg { a: *a, dir, len, along });
        along += len;
    }
    segs
}

/// Distance to the polyline, measured in the plane of `normal`, plus the lip and the arc length.
fn nearest(v: Vec3, segs: &[Seg], normal: Vec3) -> Option<Near> {
    let mut best: Option<Near> = None;
    for seg in segs {
        let s = (v - seg.a).dot(seg.dir).clamp(0.0, seg.len);
        let point = seg.a + seg.dir * s;
        let off = v - point;
        let flat = off - normal * off.dot(normal);
        let dist = flat.length();
        if best.as_ref().is_some_and(|b| b.dist <= dist) {
            continue;
        }
        let lateral = normal.cross(seg.dir);
        let sign = if flat.dot(lateral) < 0.0 { -1.0 } else { 1.0 };
        best = Some(Near { dist, point, dir: seg.dir, sign, along: seg.along + s });
    }
    best
}

/// Where the lip actually sits: `half_width` plus the outward wander at this arc length.
fn rail(half_width: f32, along: f32, sign: f32, seed: u32, scale: &Scale) -> f32 {
    half_width * (1.0 + wander(along, lip_seed(seed, sign), scale))
}

/// **A ragged margin, frozen.** Smooth 1-D value noise over `crate::bloodstain::hash_f32` — the one random
/// source every crate in this family shares, hand-rolled and pinned precisely so a wound tears the
/// same way on two machines.
///
/// Smoothstep between samples rather than a nearest lookup, because a staircase rail reads as a
/// modelling error rather than as torn tissue.
fn wander(along: f32, seed: u32, scale: &Scale) -> f32 {
    let cell = (WANDER_MM / scale.mm_per_unit).max(1.0e-6);
    let x = (along / cell).clamp(0.0, 1.0e6);
    let i = x.floor();
    let f = x - i;
    let smooth = f * f * (3.0 - 2.0 * f);
    let cell_i = i as u32;
    let a = crate::bloodstain::hash_f32(cell_i.wrapping_mul(1_664_525).wrapping_add(seed));
    let b = crate::bloodstain::hash_f32(cell_i.wrapping_add(1).wrapping_mul(1_664_525).wrapping_add(seed));
    RAIL_WANDER * (a + (b - a) * smooth)
}

/// The two lips wander independently, because a cut's two edges are not mirror images of each other.
fn lip_seed(seed: u32, sign: f32) -> u32 {
    if sign < 0.0 { seed ^ 0x9E37_79B9 } else { seed }
}

/// A stable per-region seed. Written as a match rather than an `as u32` cast so reordering
/// `Region`'s variants upstream cannot silently move this crate's frozen digest.
fn region_seed(region: Region) -> u32 {
    match region {
        Region::Limb => 0x0000_1157,
        Region::Torso => 0x0000_2263,
        Region::Head => 0x0000_3371,
    }
}

/// A vertex of the bed, carrying what its two UV channels need.
struct BedVert {
    /// Position, mesh space.
    p: Vec3,
    /// The point on the surface directly above it — the skin plane `UV_1` is measured from.
    surface: Vec3,
    /// Arc length along the wound.
    along: f32,
    /// Signed lateral offset in mesh units: `+half` on one lip, `-half` on the other, `0` on the
    /// floor.
    across: f32,
}

/// The trough behind the hole: two walls and a floor per segment, capped at both ends.
///
/// **`UV_1` is `bevy_cross_section`'s**, computed through [`uv1_at`] against a skin plane through the
/// surface point directly above each vertex — so a rail reads depth `0` (it *is* on the skin) and the
/// floor reads exactly `bed_depth_mm`, whatever the mesh's unit scale. That is the whole point of
/// going through that function rather than writing the fraction here: one definition of depth, shared
/// with every cut face in the family.
///
/// **Normals face into the gap**, and the winding is chosen to match rather than corrected
/// afterwards — a wound bed is seen from outside the body, so the walls a viewer sees are the ones
/// whose fronts point at each other.
fn bed_mesh(segs: &[Seg], n: Vec3, shape: &TearShape, seed: u32, layers: &Layers, scale: &Scale) -> Mesh {
    let depth = (shape.bed_depth_mm / scale.mm_per_unit).max(0.0);
    let mut b = Bed::default();
    let last = segs.len().saturating_sub(1);
    for (i, seg) in segs.iter().enumerate() {
        let side = n.cross(seg.dir);
        let far = seg.along + seg.len;
        let end = seg.a + seg.dir * seg.len;
        let hl0 = rail(shape.half_width, seg.along, 1.0, seed, scale);
        let hl1 = rail(shape.half_width, far, 1.0, seed, scale);
        let hr0 = rail(shape.half_width, seg.along, -1.0, seed, scale);
        let hr1 = rail(shape.half_width, far, -1.0, seed, scale);

        let l0 = BedVert { p: seg.a + side * hl0, surface: seg.a, along: seg.along, across: hl0 };
        let l1 = BedVert { p: end + side * hl1, surface: end, along: far, across: hl1 };
        let r0 = BedVert { p: seg.a - side * hr0, surface: seg.a, along: seg.along, across: -hr0 };
        let r1 = BedVert { p: end - side * hr1, surface: end, along: far, across: -hr1 };
        let f0 = BedVert { p: seg.a - n * depth, surface: seg.a, along: seg.along, across: 0.0 };
        let f1 = BedVert { p: end - n * depth, surface: end, along: far, across: 0.0 };

        // Left wall, wound counter-clockwise about `h·n - depth·side`, which points at the trough's
        // axis: down and inward from the `+side` lip.
        b.quad([&l0, &f0, &f1, &l1], (n * shape.half_width - side * depth).normalize_or_zero(), n, layers, scale);
        // Right wall, the mirror: `h·n + depth·side`.
        b.quad([&r0, &r1, &f1, &f0], (n * shape.half_width + side * depth).normalize_or_zero(), n, layers, scale);
        // The end caps: `side = n × dir`, so `(r − l) × (f − l)` is `+dir` at the near end and the
        // far cap is its mirror. Wound to agree with the authored normal, because a back-face-culled
        // cap is invisible from exactly where a wound is looked at — inside the trough.
        if i == 0 {
            b.tri([&l0, &r0, &f0], seg.dir, n, layers, scale);
        }
        if i == last {
            b.tri([&l1, &f1, &r1], -seg.dir, n, layers, scale);
        }
    }
    b.finish()
}

/// Accumulator for [`bed_mesh`]. Vertices are duplicated per face on purpose: the walls meet the
/// floor at a hard crease, and a shared vertex there would average the two normals into a rounded
/// gutter that no cut has.
#[derive(Default)]
struct Bed {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    uv0: Vec<[f32; 2]>,
    uv1: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

impl Bed {
    fn push(&mut self, v: &BedVert, face: Vec3, n: Vec3, layers: &Layers, scale: &Scale) -> u32 {
        let at = self.pos.len() as u32;
        self.pos.push(v.p.to_array());
        self.nrm.push(face.to_array());
        self.uv0.push([v.along, v.across]);
        let plane = [SkinPlane { point: v.surface, normal: n }];
        self.uv1.push(uv1_at(v.p, v.along, &plane, layers, scale).to_array());
        at
    }

    fn tri(&mut self, v: [&BedVert; 3], face: Vec3, n: Vec3, layers: &Layers, scale: &Scale) {
        let a = self.push(v[0], face, n, layers, scale);
        let b = self.push(v[1], face, n, layers, scale);
        let c = self.push(v[2], face, n, layers, scale);
        self.idx.extend_from_slice(&[a, b, c]);
    }

    fn quad(&mut self, v: [&BedVert; 4], face: Vec3, n: Vec3, layers: &Layers, scale: &Scale) {
        let a = self.push(v[0], face, n, layers, scale);
        let b = self.push(v[1], face, n, layers, scale);
        let c = self.push(v[2], face, n, layers, scale);
        let d = self.push(v[3], face, n, layers, scale);
        self.idx.extend_from_slice(&[a, b, c, a, c, d]);
    }

    fn finish(self) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.try_insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos).ok();
        mesh.try_insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.nrm).ok();
        mesh.try_insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uv0).ok();
        mesh.try_insert_attribute(Mesh::ATTRIBUTE_UV_1, self.uv1).ok();
        mesh.try_insert_indices(Indices::U32(self.idx)).ok();
        mesh
    }
}

/// A [`Scale`] that cannot divide by zero. A caller who authored nonsense gets the metres default
/// rather than a mesh full of infinities, and the *same* sanitised value reaches both the depth
/// conversion and [`uv1_at`], so the floor's UV and the floor's position cannot disagree.
fn sane(scale: &Scale) -> Scale {
    Scale {
        mm_per_unit: if scale.mm_per_unit.is_finite() && scale.mm_per_unit > 0.0 { scale.mm_per_unit } else { 1000.0 },
        tile_units: if scale.tile_units.is_finite() && scale.tile_units > 0.0 { scale.tile_units } else { 0.05 },
    }
}

/// Normalise, or `None` when there is no direction to normalise.
fn unit(v: Vec3) -> Option<Vec3> {
    let len2 = v.length_squared();
    if !len2.is_finite() || len2 <= 1.0e-24 {
        return None;
    }
    Some(v / len2.sqrt())
}
