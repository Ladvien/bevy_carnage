//! The slicer: a CPU triangle soup and the plane cuts that break it apart.
//!
//! No asset types, no ECS, no `App` — this half is pure geometry and unit-tests without any of them.
//! Everything Bevy-shaped lives one module over, in [`crate::mesh`].

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::hash::{BuildHasher, Hasher};

use bevy::log::{info, warn};
use bevy::math::{Vec2, Vec3};

use crate::CutSettings;
use crate::proxy::ProxyCell;
use crate::tree::{FragmentId, FragmentTree, TreeNode};

/// Multiplier for [`LatticeHasher`]. The fractional part of the golden ratio scaled to 64 bits — an
/// odd constant with well-distributed bits, which is all a multiply-shift mixer needs.
const LATTICE_MIX: u64 = 0x517c_c1b7_2722_0a95;

/// **The hasher every lattice-keyed map in this crate uses**, because the default one dominated the bake.
///
/// It lives beside [`WELD`] rather than at its first call site, because what it hashes is a position
/// quantised onto that lattice, and five separate maps do exactly that: the attribute weld's cell index
/// and the relief pass's coarse-vertex canon (both [`crate::mesh`]), the shell-welding vertex ids
/// below, and [`crate::proxy`]'s `CellBuilder` table and its point dedupe.
///
/// **Measured before it was written.** `sample` put 41 % of the benchmark's self time in
/// `mesh::soup_to_mesh`, and the reason is arithmetic rather than mysterious: the weld's 27-cell probe
/// performs **27 hash lookups per emitted vertex**, over 180 000 vertices per bake. `SipHash-1-3` —
/// what `RandomState` gives you — is a keyed MAC designed to be unforgeable by a remote attacker
/// sending crafted keys. Nothing here is reachable by an attacker: the keys are lattice coordinates
/// this crate computed itself from its own geometry, so that property cost cycles and bought nothing.
/// Swapping it on the weld alone was worth -24 % of the whole carnage frame.
///
/// **Output is unaffected, and the argument matters because the alternative would be a silent geometry
/// change.** A hasher decides which *bucket* a key lands in, never what `get` returns. So it is safe
/// exactly where a map's *iteration order* cannot reach emitted geometry — and every map listed above
/// is a pure lookup whose ids come from a `Vec` length, never from traversal. **A map that is iterated
/// into output must not be moved onto this hasher** without re-checking that argument; `bond.rs`'s
/// coplanar-face table and this module's shell grouping are the two to be careful with.
///
/// Not `rustc-hash` or `ahash` for the reason `tests/leaf.rs` exists: the dependency list is closed at
/// four crates, and twenty lines of mixer is not worth widening it.
#[derive(Default, Clone, Copy)]
pub(crate) struct LatticeHasher {
    h: u64,
}

impl LatticeHasher {
    #[inline]
    fn mix(&mut self, v: u64) {
        self.h = (self.h.rotate_left(5) ^ v).wrapping_mul(LATTICE_MIX);
    }
}

impl Hasher for LatticeHasher {
    /// Byte-wise fallback. Never taken for the tuple key below — `i64` hashes through
    /// [`Self::write_i64`] — but a `Hasher` must be total.
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.mix(u64::from(b));
        }
    }

    #[inline]
    fn write_i64(&mut self, i: i64) {
        self.mix(i as u64);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.mix(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.mix(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // One final avalanche, so the low bits HashMap indexes with carry information from the high
        // ones. Without it a lattice walk in x alone would stride buckets in lockstep.
        let mut h = self.h;
        h ^= h >> 32;
        h = h.wrapping_mul(LATTICE_MIX);
        h ^= h >> 29;
        h
    }
}

#[derive(Default, Clone, Copy)]
pub(crate) struct LatticeHash;

impl BuildHasher for LatticeHash {
    type Hasher = LatticeHasher;

    #[inline]
    fn build_hasher(&self) -> LatticeHasher {
        LatticeHasher::default()
    }
}

/// A lattice-keyed map, spelled once.
pub(crate) type LatticeMap<K, V> = HashMap<K, V, LatticeHash>;

/// Classification tolerance: a vertex within `EPS` of the cut plane is treated as lying *on* it, so
/// slicing near-coincident geometry doesn't spawn zero-area slivers. Positions are in subject-local
/// units (~1.0 tall for a character), so this is a tight tolerance.
pub(crate) const EPS: f32 = 1.0e-5;
/// Endpoint-weld lattice step for boundary-loop assembly (quantize positions to this grid so cut
/// segments from adjacent triangles share canonical vertex ids even on non-watertight input).
///
/// `pub(crate)` so [`crate::audit`] can derive its validation tolerance *from* it rather than pick a
/// second one. An audit that welded on a different lattice than the cap assembly used would be asking
/// about a different mesh: finer, and the cap↔skin seam reads as open purely from the mismatch;
/// coarser, and it closes seams the slicer left open.
pub(crate) const WELD: f32 = 1.0e-4;
/// Squared length of the cross product below which a triangle is not worth emitting — the
/// zero-area filter, in one place so the three sites that apply it cannot drift apart.
///
/// **Two of them agreeing is load-bearing.** [`crate::proxy::ProxyCell`] refuses to *build* a face
/// this small, precisely because [`crate::proxy::ProxyCell::append_cut_faces`] and [`soup_to_mesh`]
/// would refuse to *draw* it — and a face that exists in the cell but not in the emitted mesh is a
/// hole in something the crate promises is closed. Measured before the two were tied together: a
/// cube cut into 8 produced one fragment in 320 with `boundary_edges != 0`, seed-dependent, which
/// is why the pinned seeds never showed it.
pub(crate) const MIN_CROSS2: f32 = 1.0e-12;
/// How far behind its own surface a triangle is tested for cell membership.
///
/// Must exceed [`EPS`], or [`ProxyCell::contains`](crate::proxy::ProxyCell)'s "on a face counts as
/// inside" tolerance swallows it and the test is unchanged; must stay well under the thinnest cell
/// any caller supplies, or a triangle is nudged clean through its own solid and comes back homeless.
/// Positions are subject-local — roughly a metre for a character — so a tenth of a millimetre sits
/// two orders of magnitude clear on both sides.
pub(crate) const INWARD_NUDGE: f32 = 1.0e-3;

/// Outward unit normal of a triangle, or zero for a degenerate one — which nudges nothing and leaves
/// the centroid test exactly as it was.
fn face_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    (b - a).cross(c - a).normalize_or_zero()
}

/// The crate's only random source: a 32-bit integer hash mapped into `[0, 1)`.
///
/// **Hand-rolled, and pinned.** There is deliberately no RNG crate here. The fracture's whole
/// reproducibility argument rests on this function returning the same bits on every machine and every
/// toolchain, and a dependency that reserves the right to change its stream between minor versions
/// cannot promise that. Its exact output is frozen by a test in this crate, so the fracture cannot move
/// underneath you without something going red.
pub fn hash_f32(x: u32) -> f32 {
    let mut h = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    h = ((h >> ((h >> 28).wrapping_add(4))) ^ h).wrapping_mul(277_803_737);
    h = (h >> 22) ^ h;
    (h as f32) / (u32::MAX as f32)
}

/// A vertex sample carried through clipping (interpolated at edge–plane crossings).
#[derive(Clone, Copy)]
pub(crate) struct Vtx {
    pub(crate) pos: Vec3,
    pub(crate) nrm: Vec3,
    pub(crate) uv: Vec2,
}

/// A cut plane: a point on the plane and a unit normal.
pub(crate) struct Plane {
    pub(crate) point: Vec3,
    pub(crate) normal: Vec3,
}

/// CPU triangle soup. Parallel per-vertex arrays plus one triangle per `idx` entry; `tri_interior`
/// tags a triangle as a **cut-cap** face (gets the interior material) vs original **skin** (the
/// subject's own surface). Every vertex always carries a UV (zero-filled when the source lacked
/// `UV_0`).
#[derive(Default, Clone)]
pub(crate) struct Soup {
    pub(crate) pos: Vec<Vec3>,
    pub(crate) nrm: Vec<Vec3>,
    pub(crate) uv: Vec<Vec2>,
    pub(crate) idx: Vec<[u32; 3]>,
    pub(crate) tri_interior: Vec<bool>,
}

impl Soup {
    pub(crate) fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }

    /// A soup sized for a known triangle count, so [`Self::push_tri`] never regrows.
    ///
    /// Five vectors grown from empty is five reallocation ladders, and each rung copies everything
    /// written so far — which is bytes touched, not merely an allocator call. `soften`'s midpoint
    /// subdivision knows its output size exactly: four triangles per input triangle.
    pub(crate) fn with_capacity(tris: usize) -> Self {
        Self {
            pos: Vec::with_capacity(tris * 3),
            nrm: Vec::with_capacity(tris * 3),
            uv: Vec::with_capacity(tris * 3),
            idx: Vec::with_capacity(tris),
            tri_interior: Vec::with_capacity(tris),
        }
    }

    pub(crate) fn vtx(&self, i: u32) -> Vtx {
        let i = i as usize;
        Vtx { pos: self.pos[i], nrm: self.nrm[i], uv: self.uv[i] }
    }

    /// Append one triangle as three fresh vertices.
    ///
    /// **Three appends of a slice, not nine of a scalar.** This is the innermost write of the whole
    /// bake — the midpoint subdivision alone calls it four times per input triangle — and the loop it
    /// replaces performed nine separate `Vec::push`es, each re-checking capacity and re-reading the
    /// length. `extend_from_slice` of a three-element array does that once per buffer and copies the
    /// three together. Same values, same order, same resulting vectors.
    pub(crate) fn push_tri(&mut self, a: Vtx, b: Vtx, c: Vtx, interior: bool) {
        let base = self.pos.len() as u32;
        self.pos.extend_from_slice(&[a.pos, b.pos, c.pos]);
        self.nrm.extend_from_slice(&[a.nrm, b.nrm, c.nrm]);
        self.uv.extend_from_slice(&[a.uv, b.uv, c.uv]);
        self.idx.push([base, base + 1, base + 2]);
        self.tri_interior.push(interior);
    }

    /// Axis-aligned bounds over all vertices (min, max). `(ZERO, ZERO)` when empty.
    pub(crate) fn bbox(&self) -> (Vec3, Vec3) {
        let mut mn = Vec3::splat(f32::INFINITY);
        let mut mx = Vec3::splat(f32::NEG_INFINITY);
        for p in &self.pos {
            mn = mn.min(*p);
            mx = mx.max(*p);
        }
        if self.pos.is_empty() {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            (mn, mx)
        }
    }

    /// Largest bounding half-dimension — the "how big is this piece" measure driving fragment sizing.
    pub(crate) fn extent(&self) -> f32 {
        let (mn, mx) = self.bbox();
        ((mx - mn) * 0.5).max_element()
    }
}

/// Signed distance from `p` to the plane (positive on the `+normal` side).
pub(crate) fn signed_dist(p: Vec3, plane: &Plane) -> f32 {
    (p - plane.point).dot(plane.normal)
}

/// `+1` above / `-1` below / `0` on the plane (within `EPS`).
pub(crate) fn classify(s: f32) -> i32 {
    if s > EPS {
        1
    } else if s < -EPS {
        -1
    } else {
        0
    }
}

/// Vertex interpolated where segment `a→b` crosses the plane at parameter `t`.
fn lerp_vtx(a: Vtx, b: Vtx, t: f32) -> Vtx {
    Vtx {
        pos: a.pos.lerp(b.pos, t),
        nrm: a.nrm.lerp(b.nrm, t).normalize_or_zero(),
        uv: a.uv.lerp(b.uv, t),
    }
}

/// Clip one triangle to the half-space we keep (Sutherland–Hodgman on the 3-gon), fan-triangulate
/// the kept polygon, and append it to `out`. On-plane vertices (`classify == 0`) are kept for *both*
/// half-spaces so the seam geometry is shared. Original `interior` tag is inherited.
fn clip_half(v: [Vtx; 3], s: [f32; 3], keep_above: bool, interior: bool, out: &mut Soup) {
    let mut poly: Vec<Vtx> = Vec::with_capacity(4);
    for i in 0..3 {
        let j = (i + 1) % 3;
        let (ci, cj) = (classify(s[i]), classify(s[j]));
        let keep_i = if keep_above { ci >= 0 } else { ci <= 0 };
        if keep_i {
            poly.push(v[i]);
        }
        // Strict crossing (opposite strict sides) → insert the intersection vertex.
        if ci != 0 && cj != 0 && ci != cj {
            let t = s[i] / (s[i] - s[j]);
            poly.push(lerp_vtx(v[i], v[j], t));
        }
    }
    if poly.len() >= 3 {
        for i in 1..poly.len() - 1 {
            out.push_tri(poly[0], poly[i], poly[i + 1], interior);
        }
    }
}

/// Two orthonormal in-plane axes for a given plane normal (for cross-section UVs).
pub(crate) fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let a = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = n.cross(a).normalize_or_zero();
    let v = n.cross(u);
    (u, v)
}

/// Random unit vector on the sphere from a hash seed (always exactly unit length — never zero).
fn random_dir(seed: u32) -> Vec3 {
    let h1 = hash_f32(seed.wrapping_add(0x1234_5678));
    let h2 = hash_f32(seed.wrapping_add(0x9E37_79B9));
    let z = 2.0 * h1 - 1.0;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = h2 * TAU;
    Vec3::new(r * phi.cos(), z, r * phi.sin())
}



/// One fragment mid-fracture: the cell being cut, the render surface clipped alongside it, and any
/// open shells riding whole.
pub(crate) struct Piece {
    pub(crate) cell: ProxyCell,
    pub(crate) render: Soup,
    /// Open shells assigned to this fragment and **never clipped** — see [`Shell`].
    pub(crate) sheets: Vec<Soup>,
    /// [`CutSettings::cap_relief`], carried to emit time. It changes nothing about the cut — it is
    /// Tier B, read only when the drawn cap is built — but riding along on the piece keeps it out of
    /// four intermediate signatures that have no other reason to know about it.
    pub(crate) relief: f32,
    /// [`CutSettings::soften`], carried the same way and for the same reason.
    pub(crate) soften: f32,
}

/// One plug on its way out, mid-fracture: the same [`Piece`] shape as a fragment, plus where it left.
///
/// A `Piece` because it is emitted through exactly the same path — a convex cell, a subset of the
/// subject's skin, cut faces from the cell, and the same two Tier B dials. The difference is what it
/// is *not*: it never enters the [`crate::tree::FragmentTree`] and never reaches
/// [`crate::bond::BondGraph`], so it cannot bond itself back into the hole it came from.
pub(crate) struct Ejected {
    pub(crate) piece: Piece,
    pub(crate) exit: Vec3,
    pub(crate) direction: Vec3,
}

/// One connected component of a triangle soup, and whether it is a *solid's* surface or a sheet.
///
/// **The distinction AG-003 exists for.** A cape, a hair card, a decal or any single-sided sheet has no
/// interior. Clipping one against a cut plane cuts it in half, which is wrong twice over: the piece has
/// no volume for the plane to divide, and the halves fly apart along a seam the artist drew as
/// continuous. Such a shell is assigned to exactly one fragment and carried **whole**.
struct Shell {
    tris: Vec<usize>,
    /// `true` when the shell has boundary edges — an edge used by exactly one triangle.
    open: bool,
    centroid: Vec3,
}

/// Partition a soup into connected components, welding positions so triangles that merely *share a
/// corner value* are recognised as adjacent.
///
/// **This is the island detection Müller lists as a required step**, not an optimisation — his §3.3
/// calls it "crucial… it is this step that makes sure that objects collapse in the correct way". Here
/// it does double duty: it finds the open sheets AG-003 must protect, and it is the same pass a future
/// compound-fracture would need.
fn shells(soup: &Soup) -> Vec<Shell> {
    let q = |x: f32| (x / WELD).round() as i64;
    // Lattice-keyed and never iterated — ids come from `vid.len()`. See [`LatticeHash`].
    let mut vid: LatticeMap<(i64, i64, i64), usize> =
        LatticeMap::with_capacity_and_hasher(soup.pos.len(), LatticeHash);
    let mut canon: Vec<usize> = Vec::with_capacity(soup.pos.len());
    for p in &soup.pos {
        let key = (q(p.x), q(p.y), q(p.z));
        let next = vid.len();
        canon.push(*vid.entry(key).or_insert(next));
    }

    // Union-find over welded vertices; triangles inherit their component from any corner.
    let mut parent: Vec<usize> = (0..vid.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for tri in &soup.idx {
        let (a, b, c) = (canon[tri[0] as usize], canon[tri[1] as usize], canon[tri[2] as usize]);
        for (x, y) in [(a, b), (b, c)] {
            let (rx, ry) = (find(&mut parent, x), find(&mut parent, y));
            if rx != ry {
                parent[rx] = ry;
            }
        }
    }

    // Group triangles by root, in first-seen order so the result does not depend on hash iteration.
    let mut order: Vec<usize> = Vec::new();
    let mut slot: HashMap<usize, usize> = HashMap::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (t, tri) in soup.idx.iter().enumerate() {
        let root = find(&mut parent, canon[tri[0] as usize]);
        let idx = *slot.entry(root).or_insert_with(|| {
            order.push(root);
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[idx].push(t);
    }

    groups
        .into_iter()
        .map(|tris| {
            // An edge used once is a boundary edge; any of them makes the shell a sheet.
            let mut edges: HashMap<(usize, usize), u32> = HashMap::new();
            let mut sum = Vec3::ZERO;
            let mut n = 0.0f32;
            for &t in &tris {
                let tri = soup.idx[t];
                let v = [canon[tri[0] as usize], canon[tri[1] as usize], canon[tri[2] as usize]];
                for i in 0..3 {
                    let (a, b) = (v[i], v[(i + 1) % 3]);
                    *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
                for &i in &tri {
                    sum += soup.pos[i as usize];
                    n += 1.0;
                }
            }
            Shell {
                open: edges.values().any(|&c| c == 1),
                centroid: if n > 0.0 { sum / n } else { Vec3::ZERO },
                tris,
            }
        })
        .collect()
}

/// **The fracture: Tier A cuts, Tier B rides along.**
///
/// Returns one `(cell, render)` pair per fragment. Each cut picks the largest remaining cell by
/// **volume**, puts a plane through its centroid, splits the cell, and splits that cell's render
/// payload with the *same* plane — by clipping only, never by capping. The cap is the cell's new face.
///
/// # Why volume and not extent
///
/// The soup cutter used `Soup::extent`, the largest bounding half-dimension, and that metric has a
/// standing failure: a flat sliver with one long axis keeps winning "largest piece" and being re-cut
/// forever, while compact pieces are never touched. Volume has no such degenerate case. This is the
/// first half of `AG-011`, delivered here because Tier A would otherwise have inherited the bug.
///
/// # Why each fragment is exactly one cell
///
/// A cut splits one cell into two, so a fragment is always a single convex cell rather than a set of
/// them. A bore makes several cells out of one *before* the loop starts, and each of those is still
/// exactly one convex cell, so the property holds either way. That is a deliberate narrowing of the
/// architecture note, and it pays twice: the fragment is trivially closed and convex, and `AG-007`
/// gets a solver-ready collider with no decomposition at spawn. Cells are never unioned across
/// shells, so a head cannot weld itself to a torso.
///
/// A bore's shards count toward [`CutSettings::target`](crate::CutSettings::target), so a heavily
/// bored subject may reach `target` with no fracture cuts at all. That is the honest reading: the
/// channel already divided it, and cutting further would only shrink pieces the bore made small.
///
/// # The returned forest
///
/// Every piece the loop ever held is returned, not just the final ones: a cut *adds* two children
/// and leaves the parent in place rather than overwriting it. The [`FragmentTree`] says which is
/// which, and [`FragmentTree::frontier_of`] reads the set back at any granularity from
/// `proxy.len()` pieces up to `target`. Keeping the parents is the whole hierarchy feature and it
/// costs no extra geometry work — only the memory of the payloads, which
/// [`max_depth`](crate::FractureSettings::max_depth) bounds.
///
/// **The cut sequence is unchanged by that bookkeeping.** Selection still runs over the live
/// frontier by slot, with the same volume metric and the same lower-slot tie-break, and the seed
/// still mixes in the frontier size — so a bake taken before the tree existed and one taken after
/// partition the mesh identically.
pub(crate) fn fracture(
    render: Soup,
    proxy: &[ProxyCell],
    cut: &CutSettings,
) -> (Vec<Piece>, FragmentTree, Vec<Ejected>) {
    let CutSettings {
        target,
        min_fraction,
        max_depth,
        plane_jitter,
        size_spread,
        weak_axis,
        cap_relief,
        soften,
        ejecta_soften,
        seed,
        ref bores,
    } = *cut;
    // **Bores first, because a channel is part of the subject's shape and not part of its breakage.**
    // Subtracting here means a bored cell reaches the loop as ordinary convex root cells, so the tree
    // stays binary and `fragments[id.index()]` stays parallel with `tree.node(id)`. The plugs come
    // back separately and stay out of both, so nothing can bond one into the hole it came from.
    //
    // The **skin** is carved a few lines down, per closed shell, rather than here: a carved skin has
    // boundary edges at every hole rim, so classifying it after the carve reads a bored solid as a
    // sheet and carries the whole subject to one fragment. See [`crate::bore::carve`].
    let (bored, prisms, plugs) = crate::bore::apply(proxy, bores);
    let proxy: &[ProxyCell] = &bored;
    // One slot per landed prism, filled by the per-shell carve below with the skin that channel took.
    let mut torn: Vec<Soup> = (0..prisms.len()).map(|_| Soup::default()).collect();
    // Tier B assignment. Every triangle goes to the first cell containing its centroid — first, not
    // nearest, because overlapping shells (a head sunk into a torso) are the normal case and a
    // deterministic tie-break beats a distance that can flip on a rounding difference.
    //
    // **Open shells are assigned as a unit and never clipped.** A cape, a hair card or a decal has no
    // interior for a plane to divide; cutting one in half separates geometry the artist drew as
    // continuous. See [`Shell`].
    let mut pieces: Vec<Piece> = proxy
        .iter()
        .map(|c| Piece { cell: c.clone(), render: Soup::default(), sheets: Vec::new(), relief: cap_relief, soften })
        .collect();
    let mut homeless = 0usize;
    let mut carried = 0usize;
    let mut considered = 0usize;

    for shell in shells(&render) {
        if shell.open {
            let mut whole = Soup::default();
            for &t in &shell.tris {
                let tri = render.idx[t];
                whole.push_tri(
                    render.vtx(tri[0]),
                    render.vtx(tri[1]),
                    render.vtx(tri[2]),
                    render.tri_interior[t],
                );
            }
            match pieces.iter().position(|p| p.cell.contains(shell.centroid)) {
                Some(i) => {
                    pieces[i].sheets.push(whole);
                    carried += 1;
                }
                None => homeless += shell.tris.len(),
            }
            considered += shell.tris.len();
            continue;
        }
        // **A bore opens a solid's skin, and only a solid's.** The shell was classified above on the
        // artist's own geometry and is carved here, so the opening is the width of the channel rather
        // than the width of a triangle, and its new boundary lands exactly on the barrel planes where
        // the wall's cut-face ring already is. A sheet is carried unbored: a bore is a subtraction
        // from the proxy, and a sheet is not in the proxy.
        let mut solid = Soup::default();
        for &t in &shell.tris {
            let tri = render.idx[t];
            solid.push_tri(
                render.vtx(tri[0]),
                render.vtx(tri[1]),
                render.vtx(tri[2]),
                render.tri_interior[t],
            );
        }
        let solid = crate::bore::carve(solid, &prisms, &mut torn);
        for (t, tri) in solid.idx.iter().enumerate() {
            let (a, b, c) = (solid.vtx(tri[0]), solid.vtx(tri[1]), solid.vtx(tri[2]));
            let mid = (a.pos + b.pos + c.pos) / 3.0;
            // **Test just inside the surface, not on it.** A triangle's outward normal points away
            // from the solid it bounds, so a point a hair behind it is inside the cell that triangle
            // actually belongs to — and outside the neighbour it merely touches.
            //
            // Testing the bare centroid gets this wrong whenever two cells share a face, which is
            // exactly what a joint is. `contains` counts "on a face" as inside, deliberately, so a
            // triangle on the boundary is inside *both* cells and the first one wins. Measured on a
            // six-part humanoid: every limb's inward face was assigned to the torso, leaving each
            // limb with a hole precisely where it met the body — head short by 0.0676, the neck's
            // own area; each arm by 0.1040, the shoulder's; each leg by 0.0528, the hip's; and the
            // torso holding all 0.3812 of it. Which is to say the hole was exactly joint-shaped.
            let mid = mid - face_normal(a.pos, b.pos, c.pos) * INWARD_NUDGE;
            match pieces.iter().position(|p| p.cell.contains(mid)) {
                Some(i) => pieces[i].render.push_tri(a, b, c, solid.tri_interior[t]),
                None => homeless += 1,
            }
            considered += 1;
        }
    }
    if carried > 0 {
        info!("carnage: carrying {carried} open shell(s) whole rather than cutting them");
    }
    if homeless > 0 {
        warn!(
            "carnage: {homeless} of {} triangles lie outside every proxy cell and were dropped — the \
             proxy does not cover the mesh",
            considered
        );
    }

    // **`min_fraction` is a *linear* fraction, cubed here to compare volumes.** Callers think in
    // sizes — "stop at about 15% of the subject" — and the soup cutter's `min_extent` meant exactly
    // that. Comparing 0.15 against a volume ratio instead would be roughly four times stricter and
    // would silently return far fewer fragments than any existing caller asked for.
    let whole: f32 = pieces.iter().map(|p| p.cell.volume()).sum();
    let f = min_fraction.max(0.0);
    let floor = whole * f * f * f;

    // One node per proxy cell to start: the roots of the forest, uncut.
    let mut nodes: Vec<TreeNode> =
        (0..pieces.len()).map(|_| TreeNode { parent: None, children: None, depth: 0, split_at: None }).collect();
    // **The live frontier, by slot.** `live[slot]` is the node id currently occupying that slot, and
    // the slot layout mirrors what the pre-hierarchy loop did to its `pieces` vector exactly: a cut
    // reuses its own slot for the `above` half and pushes a new slot for `below`. Selection and the
    // seed mix both read slots, never node ids, which is what keeps the cut sequence unmoved now
    // that ids no longer coincide with frontier positions.
    let mut live: Vec<usize> = (0..pieces.len()).collect();
    let mut unsplittable = vec![false; live.len()];
    let mut cuts: u32 = 0;

    let hard_cap = target * 16 + 32;
    for cut_index in 0..hard_cap {
        if live.len() >= target.max(1) {
            break;
        }
        // **Which piece to cut next.** Strictly the largest by volume marches down a size order and
        // levels everything toward the same size, which is the uniform-shard look; `size_spread`
        // nudges the ranking by a stable per-node hash so a slightly smaller piece can win. The
        // nudge keys on the *node id*, not the frontier slot, so it is a fixed property of the piece
        // rather than something that shifts as the frontier grows.
        let ranked = |node: usize| -> f32 {
            let v = pieces[node].cell.volume();
            if size_spread <= 0.0 {
                return v;
            }
            let h = hash_f32(seed ^ (node as u32).wrapping_mul(0x9E37_79B9));
            v * (1.0 - size_spread * 0.5 + size_spread * h)
        };
        // SORT-OK: `total_cmp` over the ranking with the slot as tie-break — a total order, so the
        // choice is a function of the geometry alone and not of the vector's incidental layout.
        let Some(slot) = (0..live.len())
            .filter(|&s| !unsplittable[s])
            .max_by(|&a, &b| ranked(live[a]).total_cmp(&ranked(live[b])).then(b.cmp(&a)))
        else {
            break;
        };
        let parent = live[slot];
        if pieces[parent].cell.volume() < floor {
            unsplittable[slot] = true;
            continue;
        }
        // The depth bound is a memory bound: total payload across the forest is roughly the deepest
        // path times the subject's own triangle count, because every level holds the whole subject
        // over again. A piece at the limit is retired exactly like one below the volume floor.
        if nodes[parent].depth >= max_depth {
            unsplittable[slot] = true;
            continue;
        }

        // Seed mixing is unchanged from the soup cutter, including the frontier-size term: the plane
        // sequence is a function of how many fragments exist so far, and changing that would move
        // every asset this crate has ever fractured.
        let s = seed
            .wrapping_add((cut_index as u32).wrapping_mul(2_654_435_761))
            .wrapping_add(live.len() as u32);
        let plane = choose_plane(&pieces[parent].cell, s, weak_axis, plane_jitter);

        let (Some(above), Some(below)) = pieces[parent].cell.clip(&plane, crate::proxy::FaceKind::Cut)
        else {
            unsplittable[slot] = true;
            continue;
        };
        // Tier B: clip only. No `cap_side`, no loop recovery — the cap is `above`/`below`'s new face.
        //
        // **Both halves are sized from the parent rather than grown from empty.** A clip sends each
        // input triangle to one side or splits it across both, so neither half can exceed the parent's
        // triangle count by more than the straddling ones — reserving that much means `push_tri` never
        // climbs a reallocation ladder, and every rung of one recopies the whole buffer written so far.
        // Same mechanism that paid in `mesh::soften`'s subdivision buffer.
        let reserve = pieces[parent].render.idx.len();
        let (mut ra, mut rb) = (Soup::with_capacity(reserve), Soup::with_capacity(reserve));
        split_render(&pieces[parent].render, &plane, &mut ra, &mut rb);

        // **A sheet goes wholly to one side.** Its centroid lay in the parent cell, so the sign of its
        // distance to this plane picks a half without ambiguity and without a fallback branch.
        //
        // Cloned rather than moved out: the parent piece survives as an interior node of the forest,
        // and a coarser frontier will spawn it with its sheets still attached.
        let (mut sa, mut sb): (Vec<Soup>, Vec<Soup>) = (Vec::new(), Vec::new());
        for sheet in &pieces[parent].sheets {
            let c = sheet.pos.iter().copied().sum::<Vec3>() / sheet.pos.len().max(1) as f32;
            if signed_dist(c, &plane) >= 0.0 { sa.push(sheet.clone()) } else { sb.push(sheet.clone()) }
        }

        let (above_id, below_id) = (nodes.len(), nodes.len() + 1);
        let depth = nodes[parent].depth.saturating_add(1);
        let kid = |parent: usize| TreeNode {
            parent: Some(FragmentId(parent as u32)),
            children: None,
            depth,
            split_at: None,
        };
        nodes.push(kid(parent));
        nodes.push(kid(parent));
        nodes[parent].children = Some([FragmentId(above_id as u32), FragmentId(below_id as u32)]);
        nodes[parent].split_at = Some(cuts);

        pieces.push(Piece { cell: above, render: ra, sheets: sa, relief: cap_relief, soften });
        pieces.push(Piece { cell: below, render: rb, sheets: sb, relief: cap_relief, soften });

        live[slot] = above_id;
        live.push(below_id);
        unsplittable.push(false);
        cuts += 1;
    }

    // **The plugs, assembled last and kept out of everything above.** Each takes the skin its own
    // channel tore out — the entry and exit patches — assigned by the same inward-nudged centroid test
    // the fragments used, so a shot that crossed two cells gives each plug its own share. The cut faces
    // come from the plug's cell at emit time like any other piece, which is what makes the barrel wall
    // red without a second material path.
    let ejecta: Vec<Ejected> = plugs
        .into_iter()
        .enumerate()
        .flat_map(|(n, plug)| {
            let mut render = Soup::default();
            if let Some(skin) = torn.get(plug.prism) {
                for (t, tri) in skin.idx.iter().enumerate() {
                    let (a, b, c) = (skin.vtx(tri[0]), skin.vtx(tri[1]), skin.vtx(tri[2]));
                    let mid = (a.pos + b.pos + c.pos) / 3.0
                        - face_normal(a.pos, b.pos, c.pos) * INWARD_NUDGE;
                    if plug.cell.contains(mid) {
                        render.push_tri(a, b, c, skin.tri_interior[t]);
                    }
                }
            }
            // **And then it comes apart.** A plug is one convex prism, which is what a corer leaves;
            // breaking it with the crate's own cut policy is what turns cored material into gore. The
            // seed folds in the plug's index so two channels through one subject do not shatter
            // identically, and the shape dials are the bake's own — one look, one set of dials.
            let (exit, direction) = (plug.exit, plug.direction);
            crate::bore::shatter(
                plug.cell,
                render,
                plug.shatter,
                seed.wrapping_add((n as u32).wrapping_mul(0x27D4_EB2F)),
                weak_axis,
                plane_jitter,
                size_spread,
            )
            .into_iter()
            .map(move |(cell, render)| Ejected {
                piece: Piece {
                    cell,
                    render,
                    sheets: Vec::new(),
                    relief: cap_relief,
                    // **Debris is rounded on its own dial.** See `CutSettings::ejecta_soften`: a
                    // bored subject must be drawn flat or its wedges separate, and gore has no
                    // neighbour to separate from.
                    soften: ejecta_soften,
                },
                exit,
                direction,
            })
            .collect::<Vec<_>>()
        })
        .collect();

    (pieces, FragmentTree::from_nodes(nodes, cuts), ejecta)
}

/// **Where to cut one convex cell, given a mixed seed and the shape dials.** The crate's single cut
/// policy.
///
/// Lifted verbatim out of [`fracture`]'s loop when the bore learned to shatter its plug, and shared
/// rather than copied for one reason: the *look* of every broken thing in this crate comes from these
/// twenty lines, and two copies would drift the moment either was tuned. Gore that came apart along a
/// different rule than the body it came out of would be a second answer to one question.
///
/// `s` is the caller's already-mixed seed. Mixing stays with the caller because the two loops count
/// different things — [`fracture`] folds in the live frontier size, which is load-bearing for every
/// bake this crate has ever produced and must not be reinterpreted here.
///
/// # Cut across the piece's narrow dimension, not at a random angle
///
/// The cut face is perpendicular to the normal, so the direction the piece is *longest* along is the
/// one that gives the smallest cross-section — which is where a real thing comes apart. Sampling a
/// few candidates and keeping the longest is most of Sellán et al.'s "break across weak regions" for
/// the cost of a few dot products, and `weak_axis = 0` samples once, which is exactly the behaviour
/// every bake before that dial existed had.
///
/// # Slide the plane off centre
///
/// A plane through the centroid halves the piece, and halving every piece every time is what makes
/// the output read as uniform shards. The offset is measured against how far *this* piece reaches
/// along *this* normal, scaled back toward the centre by `plane_jitter` — so with jitter below 1.0
/// the plane is always strictly inside the cell and a cut can never be silently lost to a plane that
/// missed.
pub(crate) fn choose_plane(cell: &ProxyCell, s: u32, weak_axis: f32, plane_jitter: f32) -> Plane {
    let centroid = cell.centroid();
    let candidates = 1 + (weak_axis.clamp(0.0, 1.0) * 7.0).round() as u32;
    let mut normal = random_dir(s);
    if candidates > 1 {
        let span_of = |n: Vec3| {
            let (lo, hi) = cell.span_along(n, centroid);
            hi - lo
        };
        let mut best = span_of(normal);
        for k in 1..candidates {
            let d = random_dir(s ^ k.wrapping_mul(0x9E37_79B9));
            let span = span_of(d);
            // SORT-OK: strictly greater, scanned in candidate order, so a tie keeps the first —
            // a total order over the candidate list and a function of the geometry alone.
            if span > best {
                best = span;
                normal = d;
            }
        }
    }
    let offset = if plane_jitter > 0.0 {
        let (lo, hi) = cell.span_along(normal, centroid);
        (lo + (hi - lo) * hash_f32(s ^ 0x5BD1_E995)) * plane_jitter
    } else {
        0.0
    };
    Plane { point: centroid + normal * offset, normal }
}

/// Split a render payload by a plane into both half-spaces. **Clipping only** — a render fragment is a
/// surface subset, not a solid, and giving it a cap here would duplicate the one the cell carries.
pub(crate) fn split_render(src: &Soup, plane: &Plane, above: &mut Soup, below: &mut Soup) {
    for (t, tri) in src.idx.iter().enumerate() {
        let v = [src.vtx(tri[0]), src.vtx(tri[1]), src.vtx(tri[2])];
        let d = [
            signed_dist(v[0].pos, plane),
            signed_dist(v[1].pos, plane),
            signed_dist(v[2].pos, plane),
        ];
        let interior = src.tri_interior[t];
        clip_half(v, d, true, interior, above);
        clip_half(v, d, false, interior, below);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The fracture RNG is frozen.**
    ///
    /// These bits are the whole reproducibility story: [`hash_f32`] drives every cut plane's direction,
    /// so a changed constant re-partitions every mesh this crate has ever fractured. Treat this test as
    /// a lock, not a snapshot to re-bless: if it goes red, the fracture moved.
    #[test]
    fn hash_f32_is_frozen() {
        let got: Vec<u32> = (0..8u32).map(|i| hash_f32(i).to_bits()).collect();
        assert_eq!(
            got,
            [1022846460, 1059634922, 1056243097, 1056841197, 1042407458, 1057018071, 1064390834, 1056755236],
            "the fracture RNG moved. Every cut plane's direction comes from these bits, so a change \
             here re-partitions every mesh this crate has ever fractured."
        );
        // Every value must land in [0, 1) — the contract `random_dir` multiplies against.
        for i in 0..1024u32 {
            let v = hash_f32(i);
            assert!((0.0..1.0).contains(&v), "hash_f32({i}) = {v} escaped [0, 1)");
        }
    }

    #[test]
    fn random_dir_is_unit_length_and_never_zero() {
        for i in 0..512u32 {
            let d = random_dir(i.wrapping_mul(2_654_435_761));
            assert!((d.length() - 1.0).abs() < 1.0e-5, "random_dir({i}) length {}", d.length());
        }
    }
}
