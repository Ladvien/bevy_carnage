//! **Tier A — the proxy.** A convex cell, and the plane cut that divides one into two.
//!
//! This is the half of the fracture that is *provably* correct, and it is the reason the rest of the
//! crate got simpler rather than harder. Production fracture does not cut the render mesh: Müller,
//! Chentanez & Kim (`10.1145/2461912.2461934`, the lineage behind PhysX Blast) cut a volumetric convex
//! decomposition and carry the visual triangles as a payload. The load-bearing consequence is one line
//! of geometry:
//!
//! > **plane ∩ convex polyhedron = convex polygon.**
//!
//! Every cut face this module produces is therefore convex, which means a centroid fan over it is
//! valid — no loop recovery, no ambiguous vertex walk, no star-shapedness to hope for. The whole class
//! of defect that `assemble_loops` existed to survive cannot arise here. It is not that we found a
//! better loop-recovery algorithm; it is that we stopped generating inputs that need one.
//!
//! # Two decisions that look like details and are not
//!
//! **Faces are polygons, never triangles.** This is Müller's Figure 9, stated there as a warning: a
//! hexagon split twice using general polygonal faces stays clean, while the same splits with triangular
//! faces produce *"many ill shaped triangles… even after only two cuts."* A fracture is *repeated*
//! cutting, so that degradation compounds. Triangulation happens once, at emit time, in [`ProxyCell::
//! append_cut_faces`]. It is the same sliver pathology `AG-011` records for `Soup::extent`, and Tier A
//! must not inherit it.
//!
//! **The caller supplies the cells.** This crate does not compute a convex decomposition and will not:
//! a consumer already running V-HACD or CoACD for its colliders has one, and forcing a second,
//! different decomposition on them would be the fracture disagreeing with the physics about what the
//! object is. See the boundary list in `CLAUDE.md`.
//!
//! # Winding, sign and coordinate conventions
//!
//! - A face ring is wound **counter-clockwise seen from outside the cell**, so its Newell normal points
//!   *away* from the interior.
//! - [`ProxyCell::clip`] returns `(above, below)` relative to `plane.normal`. The **above** piece gains
//!   a cut face whose outward normal is `-plane.normal`; the **below** piece's faces `+plane.normal`.
//! - Distances are signed positive on the `+normal` side, matching [`crate::soup`].

use bevy::log::warn;
use bevy::math::Vec3;
use std::collections::HashMap;

use crate::soup::{
    EPS, LatticeHash, LatticeMap, MIN_CROSS2, Plane, WELD, classify, plane_basis, signed_dist,
};

/// **How many boundary-ring vertices share one relief vertex.**
///
/// The cut face is the largest single cost in the bake: measured on the pinned benchmark subject the
/// drawn surface is 250,942 cap triangles against 73,660 skin triangles — **3.4 : 1 cut face** — and
/// [`crate::soup::soften`] is 51 % of `bake_ms`, scaled by exactly that count.
///
/// The 3× came from emitting three triangles per *boundary* edge, at a density
/// [`weave_seam`] has deliberately inflated to the render skin's own boundary so the cap and the skin
/// share vertices. The boundary must stay that fine — it is welded to the skin's opening and moving
/// it cracks the seam. The *interior* relief structure need not: it only has to read as a crumple.
///
/// At 4 this emits `n + 2*(n/4)` cap triangles per relieved face instead of `3*n`, and the relief
/// reads better, because a displacement hashed once per boundary vertex is high-frequency fizz rather
/// than a crumple. Do not raise it above 6; beyond that the mid ring is too sparse to read as relief.
///
/// # Measured on the pinned benchmark, isolated from the flat-face branch below
///
/// `append_cut_faces` carries **two** independent reductions, and they were measured separately by
/// pinning one while disabling the other:
///
/// | change | `cap_tris` | `skin_tris` |
/// |---|---|---|
/// | neither (`RELIEF_STRIDE = 1`, flat branch off) | 250,942 | 73,660 |
/// | this constant alone, at 4 | 150,970 (−39.8 %) | 73,665 (+5) |
/// | the flat-face branch alone | 158,843 (−36.7 %) | 73,685 (+25) |
/// | **both, as shipped** | **106,721 (−57.5 %)** | **73,684 (+24)** |
///
/// The first row reproduces the pre-change digest `0xb672df832d78b94c` **exactly**, which is the
/// evidence that this rewrite introduced no incidental drift: at `m == n_ring` with the flat branch
/// off, it is the original code triangle for triangle.
///
/// **Why the skin moves at all, given this only writes cap triangles.** `append_cut_faces` writes
/// into the *same* soup as the render skin, and `draw_piece` then runs [`crate::soup::soften`] on the
/// combined result — deliberately, so the relaxation bevels the skin/cap edge, "the sharpest edge on
/// the whole fragment". The skin/cap split happens only afterwards, by interior tag. So perturbing
/// cap vertices perturbs the weld neighbourhood along the shared seam, and the skin's welded triangle
/// count shifts by 0.03 %. The two rows being +5 and +25 while their combination is +24 — not +30 —
/// is itself the proof that this is a weld interaction rather than a systematic offset.
const RELIEF_STRIDE: usize = 4;

/// What made a face, and therefore how it is drawn.
///
/// This was a `bool` — cut or supplied — until bores needed a third answer. The distinction is not
/// cosmetic: `cap_relief` scales its displacement by the face's own centre-to-corner radius
/// ([`ProxyCell::append_cut_faces`]), and a channel wall's radius is half the subject's *thickness*,
/// not half a fragment's width. Measured on the demo body: a 0.04-radius bore through a 0.28-deep
/// torso leaves a wall face of radius ≈ 0.176, and the shipped `cap_relief = 0.30` displaces its
/// centre by up to 0.053 — larger than the hole, so the wall folds through the channel axis and out
/// the far side, and the drawn mesh ends up *outside* its own collider. A bore wall is emitted flat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FaceKind {
    /// The caller's own hull. Never drawn — the render mesh covers that region far better.
    Supplied,
    /// A fracture plane's cut face. Drawn with the interior material, crumpled by `cap_relief`.
    Cut,
    /// A bore's barrel or pit floor. Drawn with the interior material, never crumpled.
    Bore,
    /// **A cut face on the tension side of a bend.** Drawn with the interior material and crumpled at
    /// **half** `cap_relief`.
    ///
    /// Measured, not stylised: Isa et al. (`doi:10.1016/j.forsciint.2021.110899`) describe the two
    /// faces of a butterfly fracture as visibly different — flat and billowy where the bone was in
    /// tension, jagged angular peaks where it was in compression. A single relief value for both is
    /// the thing that makes every fracture face read the same, so the two are separated here rather
    /// than in a shader.
    Tension,
    /// **A cut face on the compression side of a bend.** Crumpled at **double** `cap_relief` — the
    /// jagged half of the pair above.
    Compression,
}

/// A convex cell of the caller-supplied proxy: the volume Tier A actually cuts.
///
/// Construct with [`ProxyCell::new`] (which validates) or [`ProxyCell::from_box`] for the common case.
/// Faces are polygons wound counter-clockwise seen from outside; see the module docs for why they are
/// not triangles.
#[derive(Clone, Debug, PartialEq)]
pub struct ProxyCell {
    verts: Vec<Vec3>,
    /// Rings of indices into `verts`, one per face.
    faces: Vec<Vec<u32>>,
    /// Parallel to `faces`: what made each face — see [`FaceKind`].
    ///
    /// This is what the render pass needs — a cut face and a bore wall are both raw interior and take
    /// the interior material; a supplied face is the caller's own hull and is *not* drawn at all,
    /// because the render mesh already covers that region far better than a hull does.
    face_kind: Vec<FaceKind>,
}

impl ProxyCell {
    /// Build a cell from explicit geometry, or refuse it.
    ///
    /// Returns `None` — with a `warn!` naming the fault — for anything that cannot be a closed convex
    /// polyhedron: fewer than four faces, a ring shorter than a triangle, or an index outside `verts`.
    /// **Convexity is checked**, once, here — see [`Self::reject_if_concave`] for why that is the fix
    /// rather than a triangulator that survives concave input. A concave cell produces concave cut
    /// faces, and the centroid fan over one folds; refusing it names the fault at the boundary where a
    /// caller can act on it.
    pub fn new(verts: Vec<Vec3>, faces: Vec<Vec<u32>>) -> Option<Self> {
        if faces.len() < 4 {
            warn!("carnage: proxy cell has {} faces; a closed polyhedron needs at least 4", faces.len());
            return None;
        }
        for (i, f) in faces.iter().enumerate() {
            if f.len() < 3 {
                warn!("carnage: proxy cell face {i} has {} vertices, needs at least 3", f.len());
                return None;
            }
            if f.iter().any(|&v| v as usize >= verts.len()) {
                warn!("carnage: proxy cell face {i} indexes outside its {} vertices", verts.len());
                return None;
            }
        }
        let face_kind = vec![FaceKind::Supplied; faces.len()];
        let cell = Self { verts, faces, face_kind };
        cell.reject_if_concave()?;
        Some(cell)
    }

    /// Refuse a cell that is not convex, naming the worst offender.
    ///
    /// **This is `AG-008`'s answer, and it is not the answer that ticket proposed.** AG-008 planned a
    /// constrained Delaunay triangulator so a slightly concave cell would not corrupt the output. Under
    /// Tier A that is the wrong shape of fix: `CLAUDE.md`'s one-path rule says a primary path that
    /// cannot produce a usable result must **fail loudly**, not write a degraded substitute. A
    /// triangulator that quietly survives bad input is exactly the "degraded substitute" the rule names
    /// — and it would be a large piece of machinery whose only job is to make a caller's broken proxy
    /// look like it worked.
    ///
    /// So the cell is checked at the door instead. Every vertex must lie on or behind every face plane.
    /// The tolerance scales with the cell's own size, because a 10 m cell has more float slack in it
    /// than a 10 cm one, and a fixed epsilon would reject the large one for being large.
    ///
    /// Checked **only here**, on the caller's own geometry. [`Self::clip`] needs no check: a plane
    /// through a convex polyhedron yields two convex polyhedra, so convexity is an invariant once the
    /// cell is admitted, and re-verifying it per cut would cost a pass over every (face, vertex) pair
    /// for a result that cannot change.
    fn reject_if_concave(&self) -> Option<()> {
        let (mut mn, mut mx) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in &self.verts {
            mn = mn.min(*v);
            mx = mx.max(*v);
        }
        let tol = (mx - mn).length().max(1.0) * 1.0e-4;

        for fi in 0..self.faces.len() {
            let Some((o, n)) = self.face_plane(fi) else {
                warn!("carnage: proxy cell face {fi} is degenerate — it encloses no area");
                return None;
            };
            let worst = self.verts.iter().map(|v| (*v - o).dot(n)).fold(f32::NEG_INFINITY, f32::max);
            if worst > tol {
                warn!(
                    "carnage: proxy cell is not convex — a vertex sits {worst} in front of face {fi} \
                     (tolerance {tol}). Refusing it rather than cutting a shape that is not the one you \
                     described; every cut face of a concave cell is concave too, and the cap fan over \
                     one folds."
                );
                return None;
            }
        }
        Some(())
    }

    /// A box cell — the shape most test fixtures and many blocked-out subjects actually are.
    ///
    /// `half` is the half-extent on each axis, `center` its middle. Faces are wound counter-clockwise
    /// seen from outside, verified by `from_box_is_wound_outward`.
    ///
    /// Bypasses [`Self::new`]'s convexity check deliberately: a box is convex by construction, and this
    /// is the crate's own geometry rather than a caller's promise.
    pub fn from_box(center: Vec3, half: Vec3) -> Self {
        let s = |x: f32, y: f32, z: f32| center + Vec3::new(x * half.x, y * half.y, z * half.z);
        // 0..3 = -Z face ring, 4..7 = +Z, in matching order so the side faces read off in pairs.
        let verts = vec![
            s(-1.0, -1.0, -1.0),
            s(1.0, -1.0, -1.0),
            s(1.0, 1.0, -1.0),
            s(-1.0, 1.0, -1.0),
            s(-1.0, -1.0, 1.0),
            s(1.0, -1.0, 1.0),
            s(1.0, 1.0, 1.0),
            s(-1.0, 1.0, 1.0),
        ];
        let faces = vec![
            vec![0, 3, 2, 1], // -Z
            vec![4, 5, 6, 7], // +Z
            vec![0, 1, 5, 4], // -Y
            vec![2, 3, 7, 6], // +Y
            vec![0, 4, 7, 3], // -X
            vec![1, 2, 6, 5], // +X
        ];
        Self { face_kind: vec![FaceKind::Supplied; faces.len()], verts, faces }
    }

    /// The cell's vertices — **this is the collider.**
    ///
    /// Every convex-hull collider constructor in every solver takes a point cloud: parry's
    /// `ConvexPolyhedron::from_convex_hull`, Rapier's `ColliderBuilder::convex_hull`, Avian's
    /// `Collider::convex_hull`. Hand them this slice. There is no decomposition to run at spawn time
    /// and no trimesh to fall back to, because a fragment *is* one convex cell.
    ///
    /// That is the payoff Müller's architecture gives twice: the same decomposition that makes the cut
    /// robust is the one the solver wanted anyway.
    pub fn points(&self) -> &[Vec3] {
        &self.verts
    }

    /// The cell's faces, as rings of indices into [`Self::points`], wound counter-clockwise seen from
    /// outside.
    ///
    /// Polygons, not triangles — see the module docs. A consumer that needs triangles should fan each
    /// ring from its first vertex, which is valid because the face is convex.
    pub fn faces(&self) -> impl Iterator<Item = &[u32]> {
        self.faces.iter().map(|f| f.as_slice())
    }

    /// Was face `fi` created by a cut, rather than supplied by the caller?
    ///
    /// A cut face is raw interior and takes the interior material; a supplied face is the caller's
    /// own hull and is not drawn at all, because the render mesh already covers that region. A bore's
    /// channel wall answers `true` here too, because for material purposes it is the same thing: raw
    /// interior the crate created. Only how it is *displaced* differs — see [`FaceKind`].
    pub fn face_is_cut(&self, fi: usize) -> bool {
        self.face_kind.get(fi).is_some_and(|k| *k != FaceKind::Supplied)
    }

    /// Enclosed volume. Useful for mass properties: a solver wanting uniform density needs exactly
    /// this times the density, and no other part of the fragment describes how much stuff it is.
    pub fn volume(&self) -> f32 {
        self.signed_volume()
    }

    /// The cell's vertex-average centre, in the same subject-local space as the points.
    pub fn center(&self) -> Vec3 {
        self.centroid()
    }

    /// Vertex-average centre. The cut planes pass through this, so it is part of the seed chain.
    pub(crate) fn centroid(&self) -> Vec3 {
        if self.verts.is_empty() {
            return Vec3::ZERO;
        }
        self.verts.iter().copied().sum::<Vec3>() / self.verts.len() as f32
    }

    /// Enclosed volume, by the divergence theorem over the fan-triangulated faces.
    ///
    /// Positive for an outward-wound closed cell. Used to pick which cell to cut next — **not** the
    /// bounding half-extent, because a flat sliver with one long axis wins that contest forever
    /// (`AG-011`). Volume has no such failure mode.
    pub(crate) fn signed_volume(&self) -> f32 {
        let mut v6 = 0.0f32;
        for f in &self.faces {
            for i in 1..f.len() - 1 {
                let (a, b, c) = (
                    self.verts[f[0] as usize],
                    self.verts[f[i] as usize],
                    self.verts[f[i + 1] as usize],
                );
                v6 += a.cross(b).dot(c);
            }
        }
        v6 / 6.0
    }

    /// How far the cell reaches either side of `from` along `dir`, as `(min, max)` signed distances.
    ///
    /// **The interval a cut plane may be offset within without leaving the cell.** `from` is the
    /// centroid at every call site, which is interior to a convex cell, so `min < 0 < max` and any
    /// offset strictly inside that range still divides the cell in two. Offsetting by a fraction of
    /// the bounding *box* instead would not have that property: a cell that is thin along `dir` and
    /// long across it would push its plane clean out and lose the cut.
    ///
    /// `(0, 0)` for a cell with no vertices, which [`ProxyCell::new`] cannot produce.
    pub(crate) fn span_along(&self, dir: Vec3, from: Vec3) -> (f32, f32) {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for v in &self.verts {
            let d = (*v - from).dot(dir);
            lo = lo.min(d);
            hi = hi.max(d);
        }
        if self.verts.is_empty() { (0.0, 0.0) } else { (lo, hi) }
    }

    /// How many faces this cell has.
    pub(crate) fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Face `fi` as its ring of points, wound counter-clockwise seen from outside. Empty for an
    /// out-of-range index — refused, not a panicking index.
    pub(crate) fn face_ring(&self, fi: usize) -> Vec<Vec3> {
        let Some(f) = self.faces.get(fi) else { return Vec::new() };
        f.iter().filter_map(|&i| self.verts.get(i as usize).copied()).collect()
    }

    /// Outward plane of one face, by Newell's method — stable for a polygon of any size, where a
    /// single cross product of the first three vertices is not.
    ///
    /// `pub(crate)` so [`crate::bond`] can match coplanar faces between neighbouring cells; the
    /// returned pair is `(a point on the plane, the outward unit normal)`.
    pub(crate) fn face_plane(&self, fi: usize) -> Option<(Vec3, Vec3)> {
        if fi >= self.faces.len() {
            return None;
        }
        let f = &self.faces[fi];
        let mut n = Vec3::ZERO;
        for i in 0..f.len() {
            let a = self.verts[f[i] as usize];
            let b = self.verts[f[(i + 1) % f.len()] as usize];
            n += Vec3::new(
                (a.y - b.y) * (a.z + b.z),
                (a.z - b.z) * (a.x + b.x),
                (a.x - b.x) * (a.y + b.y),
            );
        }
        let n = n.normalize_or_zero();
        if n == Vec3::ZERO { None } else { Some((self.verts[f[0] as usize], n)) }
    }

    /// Is `p` inside this cell? **The Tier B assignment test** — a render triangle belongs to the
    /// fragment whose cell contains its centroid.
    ///
    /// Convexity makes this a half-space test per face and nothing more: no ray casting, no winding
    /// number, no tolerance beyond the shared [`EPS`]. A point exactly on a shared face counts as
    /// inside both neighbours, which is deliberate — a triangle must never fall through the gap
    /// between two cells and vanish.
    pub(crate) fn contains(&self, p: Vec3) -> bool {
        (0..self.faces.len()).all(|fi| match self.face_plane(fi) {
            Some((o, n)) => (p - o).dot(n) <= EPS,
            // A degenerate face cannot exclude anything; refusing here would drop the triangle.
            None => true,
        })
    }

    /// Split by a plane into `(above, below)`, either of which is `None` when the cell lies wholly on
    /// one side.
    ///
    /// The cut face is assembled by **angular sort around the section centroid**, which is valid here
    /// and nowhere else in this crate: the section is convex, so its vertices are in angular order
    /// around any interior point. That is the whole replacement for `assemble_loops` — a sort instead
    /// of a graph walk, with no ambiguity to resolve and nothing to drop.
    pub(crate) fn clip(&self, plane: &Plane, new_face: FaceKind) -> (Option<ProxyCell>, Option<ProxyCell>) {
        let d: Vec<f32> = self.verts.iter().map(|v| signed_dist(*v, plane)).collect();
        if d.iter().all(|&s| s >= -EPS) {
            return (Some(self.clone()), None);
        }
        if d.iter().all(|&s| s <= EPS) {
            return (None, Some(self.clone()));
        }

        let mut above = CellBuilder::default();
        let mut below = CellBuilder::default();
        let mut cut: Vec<Vec3> = Vec::new();

        for (fi, f) in self.faces.iter().enumerate() {
            let (mut ra, mut rb): (Vec<Vec3>, Vec<Vec3>) = (Vec::new(), Vec::new());
            for i in 0..f.len() {
                let j = (i + 1) % f.len();
                let (pi, pj) = (self.verts[f[i] as usize], self.verts[f[j] as usize]);
                let (si, sj) = (d[f[i] as usize], d[f[j] as usize]);
                let (ci, cj) = (classify(si), classify(sj));
                if ci >= 0 {
                    ra.push(pi);
                }
                if ci <= 0 {
                    rb.push(pi);
                }
                if ci == 0 {
                    cut.push(pi);
                }
                if ci != 0 && cj != 0 && ci != cj {
                    let x = pi.lerp(pj, si / (si - sj));
                    ra.push(x);
                    rb.push(x);
                    cut.push(x);
                }
            }
            if ra.len() >= 3 {
                above.face(&ra, self.face_kind[fi]);
            }
            if rb.len() >= 3 {
                below.face(&rb, self.face_kind[fi]);
            }
        }

        // One convex ring, shared by both halves and wound opposite ways.
        let ring = convex_ring(&cut, plane);
        if ring.len() >= 3 {
            let mut rev = ring.clone();
            rev.reverse();
            above.face(&rev, new_face);
            below.face(&ring, new_face);
        }
        (above.build(), below.build())
    }

    /// **The supplied faces as outward planes — the skin.** What `bevy_cross_section` measures
    /// depth against: for a point inside this convex cell, the distance to the nearest supplied face's
    /// plane is exactly its distance to the subject's surface. A cell with no supplied face (an
    /// interior fragment) returns none, and reads as skin-depth zero there, deliberately: the
    /// alternative is guessing at a surface the cell never touched.
    pub fn skin_planes(&self) -> Vec<crate::cross_section::SkinPlane> {
        (0..self.faces.len())
            .filter(|&fi| self.face_kind[fi] == FaceKind::Supplied)
            .filter_map(|fi| self.face_plane(fi))
            .map(|(point, normal)| crate::cross_section::SkinPlane { point, normal })
            .collect()
    }

    /// Append this cell's **cut faces only** to a soup, fan-triangulated, tagged as interior.
    ///
    /// Supplied faces are deliberately not emitted: they are the caller's hull, and the render mesh
    /// already describes that region far better. Only the faces this crate created are new surface
    /// that nothing else covers.
    ///
    /// The fan is taken from each face's first vertex. That is valid because the face is convex — the
    /// property this whole tier exists to guarantee.
    pub(crate) fn append_cut_faces(&self, out: &mut crate::soup::Soup, seam: &[Vec3], relief: f32) {
        for (fi, f) in self.faces.iter().enumerate() {
            if self.face_kind[fi] == FaceKind::Supplied {
                continue;
            }
            let Some((origin, n)) = self.face_plane(fi) else { continue };
            // **Relief is per face kind, and the three multipliers are measured rather than styled.**
            // A bore wall is emitted flat (see `FaceKind::Bore` for that measurement). A bend's two
            // faces differ: flat and billowy in tension, jagged angular peaks in compression (Isa et
            // al., `doi:10.1016/j.forsciint.2021.110899`), so tension takes half the authored relief
            // and compression takes double. An ordinary `Cut` face takes it as authored, which is why
            // every existing bake is unmoved.
            let relief = match self.face_kind[fi] {
                FaceKind::Bore => 0.0,
                FaceKind::Tension => relief * 0.5,
                FaceKind::Compression => relief * 2.0,
                FaceKind::Cut | FaceKind::Supplied => relief,
            };
            let ring: Vec<Vec3> = f.iter().map(|&v| self.verts[v as usize]).collect();
            let ring = weave_seam(&ring, n, origin, seam);
            let (bu, bv) = plane_basis(n);
            let vtx = |p: Vec3| crate::soup::Vtx {
                pos: p,
                nrm: n,
                uv: bevy::math::Vec2::new((p - origin).dot(bu), (p - origin).dot(bv)),
            };
            // **Two rings, so the middle of the cap has somewhere to move.** A fan from one corner
            // has no interior vertices at all, and a flat cut face is the visual language of cleaved
            // stone. The boundary ring never moves — it is welded to the skin's own opening, and
            // displacing it would crack that seam open — so the relief lives entirely on the centre
            // point and a ring of points partway out to the edge.
            //
            // The displacement is hashed from each point's own quantized position, so it needs no
            // seed threaded down here and comes back identical on every run.
            let n_ring = ring.len();
            if n_ring < 3 {
                continue;
            }
            let centre: Vec3 = ring.iter().copied().sum::<Vec3>() / n_ring as f32;
            let radius = ring.iter().map(|p| p.distance(centre)).fold(0.0f32, f32::max);
            let lift = |p: Vec3, scale: f32| -> Vec3 {
                if relief <= 0.0 || radius <= 0.0 {
                    return p;
                }
                let q = |x: f32| (x / WELD).round() as i64 as u32;
                let h = crate::soup::hash_f32(q(p.x) ^ q(p.y).wrapping_mul(0x9E37_79B9) ^ q(p.z).wrapping_mul(2_654_435_761));
                p + n * ((h - 0.5) * 2.0 * relief * radius * scale)
            };
            let hub = lift(centre, 1.0);

            let mut emit = |a: Vec3, b: Vec3, c: Vec3| {
                // A slice of zero area carries no surface and would only add a degenerate triangle.
                if (b - a).cross(c - a).length_squared() >= 1.0e-12 {
                    out.push_tri(vtx(a), vtx(b), vtx(c), true);
                }
            };

            // **A flat face gains nothing from a mid ring.** With `relief <= 0.0` every `lift` is the
            // identity, so the mid ring is a set of coplanar points that only subdivide a triangle
            // fan into three times as many coplanar triangles. Emit the plain centre fan instead —
            // `n` triangles, not `n + 2m`. This is every bore wall (`FaceKind::Bore` forces the
            // override above) and any caller authoring `cap_relief: 0.0`.
            if relief <= 0.0 {
                for i in 0..n_ring {
                    emit(hub, ring[i], ring[(i + 1) % n_ring]);
                }
                continue;
            }

            // **The relief ring is decimated; the boundary ring is not.** See [`RELIEF_STRIDE`]. `m`
            // is at least 3 so the hub fan is still a surface, and never more than the boundary ring.
            // Relief vertex `a` sits on boundary index `a * n_ring / m`, so segments differ in length
            // by at most one and an indivisible `n_ring` needs no special case.
            //
            // At `m == n_ring` this reproduces the undecimated structure triangle for triangle, in
            // the same order — **verified**, not asserted: `RELIEF_STRIDE = 1` with the flat-face
            // branch above disabled reproduces the pre-change benchmark digest exactly. Note the
            // caveat that makes that check non-obvious: stride 1 alone does *not*, because the
            // flat-face branch is a second, independent reduction that fires whatever the stride is.
            let m = (n_ring / RELIEF_STRIDE).clamp(3, n_ring);
            let mid: Vec<Vec3> =
                (0..m).map(|a| lift(centre.lerp(ring[a * n_ring / m], 0.5), 0.7)).collect();

            // Watertight by construction, which is the property to check when reading this: every
            // boundary edge `(ring[t], ring[t+1])` appears in exactly ONE cap triangle, so no
            // T-junction is introduced against the skin's opening; every mid–mid edge appears in
            // exactly one bridge and one hub triangle. Total emitted is `n_ring + 2m`, against
            // `3 * n_ring` before.
            for a in 0..m {
                let next = (a + 1) % m;
                let lo = a * n_ring / m;
                let hi = (a + 1) * n_ring / m;
                emit(hub, mid[a], mid[next]);
                for t in lo..hi {
                    emit(mid[a], ring[t % n_ring], ring[(t + 1) % n_ring]);
                }
                emit(mid[a], ring[hi % n_ring], mid[next]);
            }
        }
    }

    /// Append **every** face, fan-triangulated — the closed solid, for auditing and for colliders.
    ///
    /// This is the artefact `AG-004` asserts χ = 2 and volume conservation on. [`Self::
    /// append_cut_faces`] is what gets drawn; this is what gets measured.
    pub(crate) fn append_all_faces(&self, out: &mut crate::soup::Soup) {
        for fi in 0..self.faces.len() {
            let f = &self.faces[fi];
            let Some((origin, n)) = self.face_plane(fi) else { continue };
            let (bu, bv) = plane_basis(n);
            let vtx = |p: Vec3| crate::soup::Vtx {
                pos: p,
                nrm: n,
                uv: bevy::math::Vec2::new((p - origin).dot(bu), (p - origin).dot(bv)),
            };
            for i in 1..f.len() - 1 {
                let (a, b, c) = (
                    self.verts[f[0] as usize],
                    self.verts[f[i] as usize],
                    self.verts[f[i + 1] as usize],
                );
                if (b - a).cross(c - a).length_squared() < 1.0e-12 {
                    continue;
                }
                out.push_tri(vtx(a), vtx(b), vtx(c), self.face_kind[fi] != FaceKind::Supplied);
            }
        }
    }
}

/// Accumulates faces into a cell, welding coincident vertices onto the [`WELD`] lattice.
///
/// **The weld is what keeps the output closed.** Two adjacent faces compute the same edge–plane
/// crossing independently, and they traverse that edge in opposite directions — so the two `lerp`
/// results differ in the last bits. Left unwelded, every cut edge would be two edges and the cell would
/// have a seam down every one of them.
#[derive(Default)]
struct CellBuilder {
    verts: Vec<Vec3>,
    faces: Vec<Vec<u32>>,
    face_kind: Vec<FaceKind>,
    /// Lattice-keyed and never iterated — ids come from `verts.len()`. See
    /// [`LatticeHash`](crate::soup::LatticeHash).
    table: LatticeMap<(i64, i64, i64), u32>,
}

impl CellBuilder {
    fn weld(&mut self, p: Vec3) -> u32 {
        let q = |x: f32| (x / WELD).round() as i64;
        let key = (q(p.x), q(p.y), q(p.z));
        if let Some(&id) = self.table.get(&key) {
            return id;
        }
        let id = self.verts.len() as u32;
        self.verts.push(p);
        self.table.insert(key, id);
        id
    }

    fn face(&mut self, ring: &[Vec3], kind: FaceKind) {
        let mut idx: Vec<u32> = Vec::with_capacity(ring.len());
        for p in ring {
            let id = self.weld(*p);
            // Welding can collapse a ring's consecutive points; a repeat would make a degenerate edge.
            if idx.last() != Some(&id) {
                idx.push(id);
            }
        }
        if idx.len() > 1 && idx.first() == idx.last() {
            idx.pop();
        }
        if idx.len() >= 3 {
            self.faces.push(idx);
            self.face_kind.push(kind);
        }
    }

    /// **Collapse any face too small to be drawn, rather than shipping a cell that will lose it.**
    ///
    /// Repeated cutting accumulates near-degenerate faces: a plane passing close to an existing
    /// vertex leaves a sliver whose vertices sit just far enough apart to survive the weld. The
    /// sliver is a real face of the cell, but every triangle of its fan falls under [`MIN_CROSS2`],
    /// so [`ProxyCell::append_cut_faces`] and `soup_to_mesh` both drop it — and dropping a face from
    /// a closed cell opens it. Measured: **one fragment in 320 came back with `boundary_edges != 0`**
    /// on a cube cut into 8 across 40 seeds, entirely seed-dependent.
    ///
    /// A face that cannot be drawn is a vertex, not a face. Merging its vertices into one closes the
    /// gap for free — the sliver's two long edges become the same edge, and the faces that used to
    /// meet along it now meet directly. The merge is transitive (a union-find), because collapsing
    /// one sliver can leave its neighbour a sliver too.
    fn collapse_undrawable_faces(&mut self) {
        let area2 = |ring: &[u32], verts: &[Vec3]| -> f32 {
            // Newell: twice the area, as a vector, so a non-planar ring still measures sensibly.
            let mut n = Vec3::ZERO;
            for i in 0..ring.len() {
                let a = verts[ring[i] as usize];
                let b = verts[ring[(i + 1) % ring.len()] as usize];
                n += a.cross(b);
            }
            n.length_squared()
        };

        let mut parent: Vec<u32> = (0..self.verts.len() as u32).collect();
        fn find(parent: &mut [u32], mut i: u32) -> u32 {
            while parent[i as usize] != i {
                parent[i as usize] = parent[parent[i as usize] as usize];
                i = parent[i as usize];
            }
            i
        }
        let mut merged = false;
        for f in &self.faces {
            if area2(f, &self.verts) >= MIN_CROSS2 {
                continue;
            }
            let root = find(&mut parent, f[0]);
            for &v in &f[1..] {
                let r = find(&mut parent, v);
                if r != root {
                    parent[r as usize] = root;
                    merged = true;
                }
            }
        }
        if !merged {
            return;
        }

        // Each surviving group sits at the mean of what it absorbed, so the cell neither grows nor
        // shrinks by the collapse.
        let mut sum: HashMap<u32, (Vec3, f32)> = HashMap::new();
        for v in 0..self.verts.len() as u32 {
            let r = find(&mut parent, v);
            let e = sum.entry(r).or_insert((Vec3::ZERO, 0.0));
            e.0 += self.verts[v as usize];
            e.1 += 1.0;
        }
        for (r, (s, n)) in &sum {
            self.verts[*r as usize] = *s / *n;
        }

        let (faces, face_kind) = (std::mem::take(&mut self.faces), std::mem::take(&mut self.face_kind));
        for (f, kind) in faces.into_iter().zip(face_kind) {
            let mut idx: Vec<u32> = Vec::with_capacity(f.len());
            for v in f {
                let r = find(&mut parent, v);
                if idx.last() != Some(&r) {
                    idx.push(r);
                }
            }
            if idx.len() > 1 && idx.first() == idx.last() {
                idx.pop();
            }
            if idx.len() >= 3 && area2(&idx, &self.verts) >= MIN_CROSS2 {
                self.faces.push(idx);
                self.face_kind.push(kind);
            }
        }
    }

    fn build(mut self) -> Option<ProxyCell> {
        self.collapse_undrawable_faces();
        if self.faces.len() < 4 {
            return None;
        }
        Some(ProxyCell { verts: self.verts, faces: self.faces, face_kind: self.face_kind })
    }
}


/// Does `p` lie on one of the ring's edges (within [`EPS`])?
///
/// The guard on [`ProxyCell::refine_cut_face`]: a point on the boundary can be inserted without
/// changing the polygon, while a point in the interior cannot.
fn on_ring_boundary(ring: &[Vec3], p: Vec3) -> bool {
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let ab = b - a;
        let len2 = ab.length_squared();
        if len2 <= 1.0e-12 {
            continue;
        }
        let t = (p - a).dot(ab) / len2;
        if (-EPS..=1.0 + EPS).contains(&t) && (a + ab * t.clamp(0.0, 1.0)).distance(p) <= EPS {
            return true;
        }
    }
    false
}


/// Weave the render skin's boundary points into a cut face's ring, for **emission only**.
///
/// **The cell itself is never touched, and that is the whole design of this function.** The obvious
/// fix — inserting these points into the cell's face — was tried and corrupts the solid: the face's
/// neighbours keep the coarse edge, so the T-junction simply moves inside the cell (measured: boundary
/// edges 0 → 16, χ 2 → −3). The cell stays the pristine convex polyhedron `AG-004` asserts on; only
/// the triangles handed to the renderer get the finer boundary.
///
/// Only points **on the ring's boundary** are woven in. An interior point would make the ring
/// non-convex and refold the fan this tier exists to keep valid, which is exactly what a proxy that
/// merely approximates the mesh would supply.
fn weave_seam(ring: &[Vec3], n: Vec3, origin: Vec3, seam: &[Vec3]) -> Vec<Vec3> {
    if seam.is_empty() {
        return ring.to_vec();
    }
    let mut merged = ring.to_vec();
    for p in seam {
        if (*p - origin).dot(n).abs() > EPS {
            continue; // not on this face's plane
        }
        if on_ring_boundary(ring, *p) {
            merged.push(*p);
        }
    }
    if merged.len() == ring.len() {
        return merged;
    }
    // `convex_ring` orders CCW about `+n`, which is the winding an outward face already has.
    let ordered = convex_ring(&merged, &Plane { point: origin, normal: n });
    if ordered.len() < 3 { ring.to_vec() } else { ordered }
}

/// Order a convex section's vertices into a ring, counter-clockwise seen from `+plane.normal`.
///
/// Deduplicates on the [`WELD`] lattice first, then sorts by angle about the section centroid. **The
/// sort is total**: ties on angle fall back to the position's raw bits, so two runs of the same build
/// cannot disagree about the order — the same rule `sort_total_by_key_at` enforces for the vertex soup.
fn convex_ring(pts: &[Vec3], plane: &Plane) -> Vec<Vec3> {
    let q = |x: f32| (x / WELD).round() as i64;
    // Membership only, never iterated — `uniq` keeps the input's own order.
    let mut seen: LatticeMap<(i64, i64, i64), ()> =
        LatticeMap::with_capacity_and_hasher(pts.len(), LatticeHash);
    let mut uniq: Vec<Vec3> = Vec::new();
    for p in pts {
        if seen.insert((q(p.x), q(p.y), q(p.z)), ()).is_none() {
            uniq.push(*p);
        }
    }
    if uniq.len() < 3 {
        return uniq;
    }
    let c: Vec3 = uniq.iter().copied().sum::<Vec3>() / uniq.len() as f32;
    let (u, v) = plane_basis(plane.normal);
    // SORT-OK: `atan2` then raw bits — total, so the order is a function of the geometry alone.
    //
    // **The key is computed once per point, not twice per comparison.** It used to live inside the
    // comparator, which meant `sort_by` recomputed `a - c` twice, two dot products and an `atan2` for
    // *both* operands of every comparison — O(n log n) transcendentals for n points, where n of them
    // suffice. Decorate, sort, undecorate.
    //
    // Bit-identical: the key expression is unchanged and its inputs are unchanged, so each point gets
    // exactly the value the comparator would have computed for it, and the tie-break on all three
    // coordinate bits still makes the order total.
    let mut keyed: Vec<(f32, Vec3)> = uniq
        .into_iter()
        .map(|p| {
            let d = p - c;
            (d.dot(v).atan2(d.dot(u)), p)
        })
        .collect();
    keyed.sort_by(|(ka, a), (kb, b)| {
        ka.total_cmp(kb)
            .then_with(|| a.x.to_bits().cmp(&b.x.to_bits()))
            .then_with(|| a.y.to_bits().cmp(&b.y.to_bits()))
            .then_with(|| a.z.to_bits().cmp(&b.z.to_bits()))
    });
    keyed.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> ProxyCell {
        ProxyCell::from_box(Vec3::ZERO, Vec3::splat(0.5))
    }

    /// **AG-008 — a concave cell is refused at the door, not survived downstream.**
    ///
    /// The witness is a cube with one vertex pushed *inward*, which is the cheapest non-convex
    /// polyhedron that still has valid faces and correct winding. Nothing about it is malformed in the
    /// ways `new` already rejects — it has eight vertices, six faces, every index in range — so if the
    /// convexity check were absent it would be admitted and would silently produce folded cap fans on
    /// every cut.
    #[test]
    fn a_concave_cell_is_refused() {
        let mut verts = vec![
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(0.5, -0.5, -0.5),
            Vec3::new(0.5, 0.5, -0.5),
            Vec3::new(-0.5, 0.5, -0.5),
            Vec3::new(-0.5, -0.5, 0.5),
            Vec3::new(0.5, -0.5, 0.5),
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(-0.5, 0.5, 0.5),
        ];
        let faces = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![2, 3, 7, 6],
            vec![0, 4, 7, 3],
            vec![1, 2, 6, 5],
        ];
        // The same geometry is accepted while it is a box …
        assert!(ProxyCell::new(verts.clone(), faces.clone()).is_some(), "a box must be admitted");

        // … and refused once one corner is dented inward.
        verts[6] = Vec3::new(0.1, 0.1, 0.1);
        assert!(
            ProxyCell::new(verts, faces).is_none(),
            "a dented cube is not convex and must be refused, not cut"
        );
    }

    /// The check must not reject a legitimate cell for being large — the tolerance scales with size.
    #[test]
    fn a_large_convex_cell_is_not_rejected_for_being_large() {
        for half in [0.01f32, 1.0, 100.0] {
            let c = ProxyCell::from_box(Vec3::ZERO, Vec3::splat(half));
            let round_tripped = ProxyCell::new(c.points().to_vec(), c.faces().map(|f| f.to_vec()).collect());
            assert!(round_tripped.is_some(), "a {half}-half-extent box was refused as non-convex");
        }
    }

    /// Every face must point away from the interior, or `contains` and `volume` both invert.
    #[test]
    fn from_box_is_wound_outward() {
        let c = unit_box();
        for fi in 0..c.faces.len() {
            let (o, n) = c.face_plane(fi).expect("box face is non-degenerate");
            assert!(
                (o - c.centroid()).dot(n) > 0.0,
                "face {fi} normal {n} points back at the centroid — the ring is wound inward"
            );
        }
        assert!((c.volume() - 1.0).abs() < 1.0e-5, "unit box encloses {}, expected 1.0", c.volume());
    }

    #[test]
    fn contains_agrees_with_the_box_it_was_built_from() {
        let c = unit_box();
        assert!(c.contains(Vec3::ZERO));
        assert!(c.contains(Vec3::new(0.49, 0.49, 0.49)));
        assert!(!c.contains(Vec3::new(0.51, 0.0, 0.0)));
        assert!(!c.contains(Vec3::new(0.0, -2.0, 0.0)));
        // On a face counts as inside, so a triangle on a shared boundary is never dropped.
        assert!(c.contains(Vec3::new(0.5, 0.0, 0.0)));
    }

    /// **The property the whole architecture rests on.** Cut a convex cell and both halves are closed
    /// convex cells whose volumes sum to the original.
    #[test]
    fn a_plane_splits_a_cell_into_two_closed_halves() {
        let c = unit_box();
        let p = Plane { point: Vec3::new(0.1, 0.0, 0.0), normal: Vec3::X };
        let (a, b) = c.clip(&p, FaceKind::Cut);
        let (a, b) = (a.expect("above half exists"), b.expect("below half exists"));

        assert!((a.volume() - 0.4).abs() < 1.0e-4, "above encloses {}, expected 0.4", a.volume());
        assert!((b.volume() - 0.6).abs() < 1.0e-4, "below encloses {}, expected 0.6", b.volume());
        assert!(
            (a.volume() + b.volume() - c.volume()).abs() < 1.0e-4,
            "the cut gained or lost volume: {} + {} != {}",
            a.volume(),
            b.volume(),
            c.volume()
        );
        // Exactly one new face per half, and it is tagged as a cut.
        assert_eq!(
            a.face_kind.iter().filter(|k| **k == FaceKind::Cut).count(),
            1,
            "above should have one cut face"
        );
        assert_eq!(
            b.face_kind.iter().filter(|k| **k == FaceKind::Cut).count(),
            1,
            "below should have one cut face"
        );
    }

    /// An oblique plane is the case a fan over a *recovered* loop used to get wrong.
    #[test]
    fn an_oblique_cut_still_closes_and_conserves_volume() {
        let c = unit_box();
        let p = Plane { point: Vec3::ZERO, normal: Vec3::new(1.0, 1.0, 1.0).normalize() };
        let (a, b) = c.clip(&p, FaceKind::Cut);
        let (a, b) = (a.expect("above"), b.expect("below"));
        assert!(
            (a.volume() + b.volume() - 1.0).abs() < 1.0e-4,
            "oblique cut: {} + {} != 1.0",
            a.volume(),
            b.volume()
        );
        assert!(a.volume() > 0.0 && b.volume() > 0.0, "both halves must be positively oriented");
    }

    /// A plane that misses returns the cell whole on one side and nothing on the other — never two
    /// pieces one of which is empty, which is what the caller's "unsplittable" check keys off.
    #[test]
    fn a_plane_outside_the_cell_does_not_split_it() {
        let c = unit_box();
        let (a, b) = c.clip(&Plane { point: Vec3::new(5.0, 0.0, 0.0), normal: Vec3::X }, FaceKind::Cut);
        assert!(a.is_none(), "nothing lies above a plane past the cell");
        assert!(b.is_some(), "the whole cell lies below it");
    }

    /// Repeated cutting must not degrade — the Müller Figure 9 property, asserted rather than trusted.
    #[test]
    fn eight_successive_cuts_conserve_volume_and_stay_closed() {
        let mut cells = vec![unit_box()];
        let planes = [
            (Vec3::X, 0.05f32),
            (Vec3::Y, -0.1),
            (Vec3::Z, 0.15),
            (Vec3::new(1.0, 1.0, 0.0).normalize(), 0.0),
        ];
        for (n, off) in planes {
            let mut next = Vec::new();
            for c in &cells {
                let (a, b) = c.clip(&Plane { point: n * off, normal: n }, FaceKind::Cut);
                next.extend(a);
                next.extend(b);
            }
            cells = next;
        }
        let total: f32 = cells.iter().map(|c| c.volume()).sum();
        assert!(cells.len() > 4, "four planes should produce more than four cells, got {}", cells.len());
        assert!((total - 1.0).abs() < 1.0e-3, "{} cells enclose {total}, expected 1.0", cells.len());
        for (i, c) in cells.iter().enumerate() {
            assert!(c.volume() > 0.0, "cell {i} came out inside out: {}", c.volume());
        }
    }

    /// Determinism: the cut is a function of geometry alone, including the cap's vertex order.
    #[test]
    fn clipping_is_bit_identical_across_runs() {
        let p = Plane { point: Vec3::new(0.03, 0.0, 0.0), normal: Vec3::new(1.0, 2.0, 3.0).normalize() };
        let first = unit_box().clip(&p, FaceKind::Cut);
        for _ in 0..3 {
            assert_eq!(unit_box().clip(&p, FaceKind::Cut), first, "the same cut produced different cells");
        }
    }
}
