//! **Tier A — the bore.** A channel subtracted from the proxy, in closed form, as plane cuts only.
//!
//! A bullet hole is not a decal here. It is a convex prism removed from the subject's own solid, so
//! the channel has a wall that is real geometry, takes the interior material like every other cut
//! face, and is bounded by the same convex-hull colliders the fracture already produces.
//!
//! # The subtraction, in one line
//!
//! For a convex cell `C` and a convex prism `P = ⋂ₖ Hₖ⁻` (each `Hₖ` one of the prism's outward
//! half-spaces), the difference decomposes exactly:
//!
//! > `C \ P = ⋃ₖ ( C ∩ Hₖ⁺ ∩ H₁⁻ ∩ … ∩ Hₖ₋₁⁻ )`
//!
//! which is *k* sequential calls to [`ProxyCell::clip`] — keep every `above`, feed every `below`
//! forward, and discard the final `below`, which is the plug inside the channel. Every shard is an
//! intersection of the original cell with half-spaces, so **every shard is convex**: exactly the
//! invariant `ProxyCell::reject_if_concave` stops re-checking after a cut. No CSG kernel, no
//! boundary-loop recovery, no new dependency.
//!
//! # Why it runs before the cut loop and not after
//!
//! A hole changes the subject's *shape*, which is Tier A input, not its breakage. Subtracting here
//! means [`crate::soup::fracture`] sees the shards as ordinary **root** cells, so
//! `fragments[id.index()] == tree.node(id)` stays parallel with no tree surgery, `TreeNode::children`
//! stays binary, and "each cut grows the frontier by exactly one piece" is untouched. A post-bake
//! bore would have needed an N-ary node or a "voided node" concept; this needs neither. The plug is
//! never a fragment, so nothing downstream has to remember not to spawn it.
//!
//! # Why the shards stay bonded
//!
//! `clip` hands the *same* cut ring to both halves, reversed for one, and that bit-exact
//! coplanarity is precisely what [`crate::bond::BondGraph::of`] keys on. Shard *k* and shard *k+1*
//! share plane *k*'s face region, so the ring of shards is a connected chain and a bored subject is
//! still one island. The barrel faces have no partner — the material there is gone — and that is
//! correct rather than something to handle.

use bevy::log::{info, warn};
use bevy::math::Vec3;

use crate::proxy::{FaceKind, ProxyCell};
use crate::soup::{EPS, Plane, Soup, WELD, choose_plane, hash_f32, plane_basis, split_render};

/// **A channel to subtract from the proxy** — a bullet hole, a drill, a spear thrust.
///
/// The segment `from → to` is the channel's axis and its extent: a shot that goes clean through is a
/// segment longer than the subject, and one that lodges is a short segment whose far end becomes the
/// pit floor. There is no `depth` dial and no infinite ray, because a segment already says both
/// things and a sentinel value would be a second way to mean one of them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bore {
    /// Where the channel starts, in subject-local space. Its entry radius is [`Self::radius`].
    pub from: Vec3,
    /// Where it ends. Inside the subject this is the pit floor; outside it, the exit.
    pub to: Vec3,
    /// **Entry radius, subject-local — the bound on the hole, not its average.** The barrel polygon
    /// is *inscribed* in this radius: its corners touch it and its faces sit a shade inside, so no
    /// point of the rim exceeds it at any [`Self::sides`]. **Bounded below by `MIN_RADIUS`** — see
    /// that constant.
    pub radius: f32,
    /// How many faces the barrel has: the channel is an `n`-gon prism, not a cylinder, because a
    /// plane is the only cut this crate makes. `3` is a triangular gouge, `8` reads as round at any
    /// size a bullet is, `24` is smooth and costs 24 shards per cell. Refused below 3 or above
    /// `MAX_SIDES`.
    pub sides: u32,
    /// **How ragged the barrel is**, in `[0, 1]`. Each of the `sides` planes is pulled *inward* by up
    /// to this fraction of the radius, by a hash of its own index and the entry point — so the tear
    /// is a property of where the shot landed, needs no seed threaded down, and comes back identical
    /// on every run. `0.0` is a clean bore. The bite is inward only, so `radius` stays the bound on
    /// the entry hole rather than becoming its average.
    pub jaggedness: f32,
    /// **How much wider the far end is**, as a fraction of `radius`: the exit radius is
    /// `radius * (1 + flare)`. Tilts each barrel plane instead of adding any, so the channel stays a
    /// convex prism and the shard count does not change. `0.0` is a straight bore.
    pub flare: f32,
    /// **How many pieces the ejected plug breaks into**, in `1..=MAX_SHATTER`.
    ///
    /// `1` leaves the plug whole, and whole is what a plug *is*: one convex prism. That reads as a
    /// dowel — the channel was cut by a corer, so the material it removed looks cored. Anything above
    /// 1 breaks it with the crate's own cut policy ([`crate::soup::choose_plane`], the same twenty
    /// lines the body fracture uses), so the pieces come apart across their narrow dimension and
    /// inherit [`plane_jitter`](crate::CutSettings::plane_jitter),
    /// [`size_spread`](crate::CutSettings::size_spread) and
    /// [`weak_axis`](crate::CutSettings::weak_axis) from the bake that produced them.
    ///
    /// The shatter faces are ordinary cut faces, so [`cap_relief`](crate::CutSettings::cap_relief)
    /// crumples them and [`soften`](crate::CutSettings::soften) rounds them — which is most of the
    /// difference between four rod segments and four lumps. Clamped rather than refused: a count out
    /// of range has an obvious nearest meaning, where a barrel with two sides does not.
    ///
    /// **Volume is conserved either way.** The pieces are half-space intersections of the plug, so
    /// they tile it exactly; shattering changes how the material leaves, never how much.
    pub shatter: u32,
}

impl Bore {
    /// A bore with the shipped channel dials — 8 sides, a little raggedness, a little flare, and a
    /// plug that comes apart into four.
    ///
    /// Assign to the rest, the way [`crate::CutSettings::new`] is used:
    /// `Bore { jaggedness: 0.0, ..Bore::new(a, b, 0.04) }`.
    pub fn new(from: Vec3, to: Vec3, radius: f32) -> Self {
        Bore { from, to, radius, sides: 8, jaggedness: 0.35, flare: 0.25, shatter: 4 }
    }
}

/// **The narrowest bore that can be cut without losing render triangles**, and the number comes from
/// `crate::soup::INWARD_NUDGE` rather than from taste. Tier B assigns a triangle to a cell by
/// testing its centroid pushed `1e-3` *behind* its own surface; a channel narrower than a few times
/// that pushes the skin at the hole's rim clean into the void and it comes back homeless — warned
/// and dropped. An order of magnitude of headroom at the shipped 8 sides, in subject-local units
/// where a character is about 1.0 tall.
///
/// Checked against [`Bore::radius`], which is the barrel polygon's *circumradius*; its narrowest
/// half-width is `radius · cos(π/sides)`, so a 3-sided gouge at this floor still clears
/// `INWARD_NUDGE` five times over.
const MIN_RADIUS: f32 = 1.0e-2;
/// Barrel faces beyond which the shard count is the cost and the roundness is not the benefit: 24
/// planes already read as smooth, and each one is a shard with its own two meshes.
const MAX_SIDES: u32 = 24;
/// **The most pieces one plug may become.** Each is an entity with two meshes and its own trajectory,
/// and a plug is small: past a dozen the pieces are below the size anything can be seen at, so the
/// cost is real and the benefit is not.
const MAX_SHATTER: u32 = 12;

/// The bore's outward half-space planes: `sides` barrel planes, then the entry and exit caps.
///
/// The prism is `{ p : signed_dist(p, plane) <= 0 }` for every returned plane, so the subtraction is
/// a run of `clip`s keeping each `above`. `None` — with a `warn!` naming the fault — for a bore that
/// cannot describe a channel; a caller mistake is named at the boundary rather than clamped into
/// something that silently is not what was asked for.
pub(crate) fn prism(bore: &Bore) -> Option<Vec<Plane>> {
    let axis = bore.to - bore.from;
    let len = axis.length();
    if !len.is_finite() || len <= EPS {
        warn!("carnage: bore from {:?} to {:?} has no length; refusing it", bore.from, bore.to);
        return None;
    }
    if !bore.radius.is_finite() || bore.radius < MIN_RADIUS {
        warn!(
            "carnage: bore radius {} is below MIN_RADIUS {MIN_RADIUS}; a channel that narrow loses \
             the skin at its own rim. Refusing it.",
            bore.radius
        );
        return None;
    }
    if !(3..=MAX_SIDES).contains(&bore.sides) {
        warn!("carnage: bore has {} sides; a channel needs 3..={MAX_SIDES}", bore.sides);
        return None;
    }
    // Clamped rather than refused, matching `weak_axis` and `soften`: a *look* dial out of range has
    // an obvious nearest meaning, where a structural one does not.
    let jaggedness = bore.jaggedness.clamp(0.0, 1.0);
    let flare = bore.flare.clamp(0.0, 1.0);

    let a = axis / len;
    let (u, v) = plane_basis(a);
    // **The planes sit at the polygon's apothem, not at `radius`.** A plane at distance `radius`
    // from the axis would *circumscribe* the circle: its corners would reach `radius / cos(π/n)` and
    // the hole would be 8.2% wider than asked for at 8 sides. Scaling by `cos(π/n)` inscribes the
    // polygon instead, so `radius` is the bound on the entry hole everywhere on its rim — which is
    // what [`Bore::radius`] promises and what makes `jaggedness`'s inward-only bite meaningful.
    // Measured before the correction: an 8-gon bore of radius 0.1 through the 1×2×1 fixture removed
    // 0.066274 (= 8·0.1²·tan(22.5°)·2, the circumscribed area) where the inscribed channel is
    // 0.056569.
    let apothem = (std::f32::consts::PI / bore.sides as f32).cos();
    let mut planes: Vec<Plane> = Vec::with_capacity(bore.sides as usize + 2);
    for i in 0..bore.sides {
        let theta = std::f32::consts::TAU * i as f32 / bore.sides as f32;
        let dir = u * theta.cos() + v * theta.sin(); // outward radial, unit
        // Hashed from the entry point's own WELD-quantized position and the side index — the same
        // trick `append_cut_faces` uses for `cap_relief`, so no seed has to reach down here.
        let q = |x: f32| (x / WELD).round() as i64 as u32;
        let key = q(bore.from.x)
            ^ q(bore.from.y).wrapping_mul(0x9E37_79B9)
            ^ q(bore.from.z).wrapping_mul(2_654_435_761)
            ^ (i.wrapping_mul(0x85EB_CA6B));
        let r0 = bore.radius * apothem * (1.0 - jaggedness * hash_f32(key));
        let r1 = r0 * (1.0 + flare);
        let p0 = bore.from + dir * r0;
        let tangent = a.cross(dir);
        // `tangent × (p1 - p0)` where `p1` is the far rim of this facet. With `flare = 0` that
        // reduces to `(a × dir) × (a·len) = len·dir`, i.e. exactly `dir`; with flare it picks up a
        // `-(r1 - r0)·a` term, which is the tilt that puts the far rim at `r1` instead of `r0` and
        // so widens the half-space toward `to`.
        let n = tangent.cross((bore.to + dir * r1) - p0).normalize_or_zero();
        if n == Vec3::ZERO {
            continue;
        }
        let n = if n.dot(dir) < 0.0 { -n } else { n };
        planes.push(Plane { point: p0, normal: n });
    }
    // "Inside" is *beyond the entry* and *before the exit*. For a shot that goes clean through, both
    // cap planes have the whole cell on their inside and contribute no shard; for a lodged shot the
    // exit cap is the pit floor. Same code either way.
    planes.push(Plane { point: bore.from, normal: -a });
    planes.push(Plane { point: bore.to, normal: a });

    if planes.len() < 5 {
        warn!(
            "carnage: bore from {:?} to {:?} collapsed to {} usable planes; that is not a channel. \
             Refusing it.",
            bore.from,
            bore.to,
            planes.len()
        );
        return None;
    }
    Some(planes)
}

/// What one bore took out of one cell: the shards that stay behind, and the plug that leaves.
///
/// **The plug is deliberately not a `ProxyCell` in the returned proxy, and that is load-bearing.** Its
/// barrel faces are the *same* rings [`ProxyCell::clip`] handed the shards, reversed — bit-identically
/// coplanar, which is exactly what [`crate::bond::BondGraph::of`] matches on. Put it back in the proxy
/// and the graph would correctly bond it to every shard around it, the plug would be part of the body
/// island, and the hole would be *filled*. It comes back as its own thing so that cannot be typed.
pub(crate) struct Cut {
    pub(crate) shards: Vec<ProxyCell>,
    /// The channel's own material. Convex, closed, and exactly the volume the hole removed.
    pub(crate) plug: ProxyCell,
}

/// Divide `cell` into the convex shards of `cell \ prism` plus the plug inside it, or leave it alone.
///
/// `None` means the prism does not reach this cell and the caller keeps it as it was. `Some` means the
/// cell is replaced; an **empty** `shards` means the bore consumed the cell whole, and then the plug
/// *is* the cell — a small part shot clean off, which is the honest reading and needs no special case.
///
/// Note what this does *not* do: it never re-hulls, never re-welds independently, never nudges a
/// vertex. Any of those destroys the bit-exact coplanarity [`crate::bond`] matches on, and the crate
/// refuses to paper that over with a proximity heuristic. `clip` is the only route.
pub(crate) fn subtract(cell: &ProxyCell, prism: &[Plane]) -> Option<Cut> {
    let mut shards: Vec<ProxyCell> = Vec::new();
    let mut rest = cell.clone();
    for plane in prism {
        match rest.clip(plane, FaceKind::Bore) {
            (Some(outside), Some(inside)) => {
                shards.push(outside);
                rest = inside;
            }
            // Everything left is outside this plane, so nothing is inside the prism: the channel
            // misses this cell entirely and the cell is untouched.
            (Some(_), None) => return None,
            // Everything left is inside this plane, so this plane carves no shard. Keep going.
            (None, Some(_)) => {}
            // `clip` cannot return neither half for a non-empty cell; treat it as a miss rather than
            // as a reason to emit a cell we cannot describe.
            (None, None) => return None,
        }
    }
    // `rest` is the plug. It used to be dropped here, and dropping it was the hole — which also meant
    // the bore was the one operation in this crate that did not conserve volume. It comes back now:
    // shards + plug is the cell, exactly.
    Some(Cut { shards, plug: rest })
}

/// One closed shell's skin with everything inside the landed prisms removed, clipped not culled.
///
/// Culling by triangle would be the wrong granularity by orders of magnitude: the demo subject's
/// torso is six triangles per face, so a bullet-sized channel would delete either nothing or the
/// whole side of the body. [`split_render`] is Sutherland–Hodgman per triangle, so the skin's new
/// boundary lands exactly on the barrel planes — which is where the wall's cut-face ring is, so
/// `weave_seam` closes the seam with no T-junction and no extra machinery.
///
/// **Called per closed shell, never on the whole soup, and that is not an optimisation.** A carved
/// skin has boundary edges at every hole rim, so `Shell::open` reads it as a *sheet* — and a sheet is
/// carried whole to one fragment rather than assigned triangle by triangle. Classified after the
/// carve, a bored box's skin goes homeless in its entirety: measured on the 1×2×1 fixture, all 10.0
/// of skin area dropped. So [`crate::soup::fracture`] decides open-versus-closed on the artist's own
/// geometry and calls this only for the shells that bound a solid. An open sheet is carried unbored,
/// which is the same answer from the other direction: a bore is a subtraction from the *proxy*, and a
/// sheet is not in the proxy.
///
/// The skin the channel took is **kept**, appended into `removed[i]` for prism `i` — those are the
/// entry and exit patches, and they are what makes the ejected plug read as a chunk of the subject
/// rather than a bare red rod. `removed` is sized by the caller from `prisms.len()`; a short slice
/// drops those patches rather than panicking, because a length mismatch here is a crate bug and not
/// something a caller can cause.
///
/// With no prisms this is exactly `src`, so an unbored bake is byte-identical.
pub(crate) fn carve(src: Soup, prisms: &[Vec<Plane>], removed: &mut [Soup]) -> Soup {
    let mut skin = src;
    for (i, prism) in prisms.iter().enumerate() {
        let mut kept = Soup::default();
        let mut rest = skin;
        for plane in prism {
            let mut inside = Soup::default();
            split_render(&rest, plane, &mut kept, &mut inside);
            rest = inside;
        }
        // `rest` is the skin inside the channel: the entry and exit patches. Prism `i + 1` splits
        // only what prism `i` kept, so the patches collected here are disjoint across prisms and no
        // triangle can be claimed by two plugs.
        if let Some(slot) = removed.get_mut(i) {
            for (t, tri) in rest.idx.iter().enumerate() {
                slot.push_tri(rest.vtx(tri[0]), rest.vtx(tri[1]), rest.vtx(tri[2]), rest.tri_interior[t]);
            }
        }
        skin = kept;
    }
    skin
}

/// **The material one channel removed from one cell**, on its way out of the subject.
///
/// One per (bore, cell) pair, so a shot through a torso and an arm pushes two of these out — which is
/// the right answer and needed no extra code. It is not a fragment: see [`Cut`] for why it must not be.
pub(crate) struct Plug {
    pub(crate) cell: ProxyCell,
    /// Which landed prism made it — the index into the returned prism list, so the skin
    /// [`carve`] took can be matched back to the plug it belongs to.
    pub(crate) prism: usize,
    /// Where the channel left the subject, in subject-local space: the bore's own `to`.
    pub(crate) exit: Vec3,
    /// The channel's axis, unit — which way the plug was travelling when it came out.
    ///
    /// A geometric fact about the [`Bore`], not a velocity: the crate still moves nothing. It rides
    /// along because a caller with several plugs from several shots would otherwise have to correlate
    /// two parallel arrays to work out which way each one should go.
    pub(crate) direction: Vec3,
    /// [`Bore::shatter`], carried so the caller can break the plug up after its skin is assigned.
    pub(crate) shatter: u32,
}

/// **Break one plug into `want` convex pieces**, carrying its skin along.
///
/// The plug is a convex prism, which is exactly what an apple corer leaves — so a plug ejected whole
/// reads as a dowel however good the channel is. This breaks it with [`choose_plane`], the crate's own
/// and only cut policy, so gore comes apart by the same rule as the body it came out of.
///
/// **A cut loop, but deliberately not [`crate::soup::fracture`]'s.** Calling that on a plug was tried
/// and is wrong: a plug's skin is the two disconnected patches where the channel crossed the surface,
/// and `Shell::open` reads each of them as a *sheet* — AG-003's protection for capes — so they would be
/// carried whole to one piece instead of being clipped. What the two loops share is the part that
/// decides the *look*, which is `choose_plane`; what differs is bookkeeping this has none of, because
/// debris needs no tree, no bonds and no frontier.
///
/// `want <= 1` returns the plug untouched, so the shatter costs nothing when it is off. Volume is
/// conserved exactly: every piece is a half-space intersection of the plug.
pub(crate) fn shatter(
    cell: ProxyCell,
    render: Soup,
    want: u32,
    seed: u32,
    weak_axis: f32,
    plane_jitter: f32,
    size_spread: f32,
) -> Vec<(ProxyCell, Soup)> {
    let mut live: Vec<(ProxyCell, Soup)> = vec![(cell, render)];
    let want = want.clamp(1, MAX_SHATTER) as usize;
    if want == 1 {
        return live;
    }
    // The same shape as the main loop's: a bound on attempts, so a plug that refuses to divide
    // (every plane missing it) ends rather than spinning.
    let hard_cap = want as u32 * 4 + 8;
    let mut cut = 0u32;
    while live.len() < want && cut < hard_cap {
        let ranked = |i: usize| -> f32 {
            let v = live[i].0.volume();
            if size_spread <= 0.0 {
                return v;
            }
            let h = hash_f32(seed ^ (i as u32).wrapping_mul(0x9E37_79B9));
            v * (1.0 - size_spread * 0.5 + size_spread * h)
        };
        // SORT-OK: `total_cmp` over the ranking with the index as tie-break — a total order, so which
        // piece breaks next is a function of the geometry alone.
        let Some(i) = (0..live.len()).max_by(|&a, &b| ranked(a).total_cmp(&ranked(b)).then(b.cmp(&a)))
        else {
            break;
        };
        let s = seed
            .wrapping_add(cut.wrapping_mul(2_654_435_761))
            .wrapping_add(live.len() as u32);
        let plane = choose_plane(&live[i].0, s, weak_axis, plane_jitter);
        cut += 1;
        // **`FaceKind::Cut`, not `Bore`.** A shatter face is a fracture face on a piece of debris, so
        // it should be crumpled and rounded like every other one. The plug's *barrel* faces stay
        // `Bore` and stay flat: they are as long as the subject is deep, and `cap_relief` scaled by
        // their radius would fold a piece through itself. Both kinds ride along on the same cell.
        let (Some(above), Some(below)) = live[i].0.clip(&plane, FaceKind::Cut) else { continue };
        let (mut ra, mut rb) = (Soup::default(), Soup::default());
        split_render(&live[i].1, &plane, &mut ra, &mut rb);
        live[i] = (above, ra);
        live.push((below, rb));
    }
    live
}

/// Subtract every bore from the proxy; hand back the prisms that landed and the plugs they pushed out.
///
/// Bores are applied in the order given, each to the previous one's output, so two shots that
/// overlap produce one channel rather than two contradictory ones. Cells keep their input order and
/// a bored cell is replaced in place by its shards in plane order, so the result is a function of
/// the geometry alone — no hash iteration, no sort.
///
/// The returned prisms are the ones the caller must also [`carve`] out of the skin, in the same
/// order. A bore that reached no proxy cell is **not** in that list and contributes no plug: a
/// channel that touched no solid must not open the skin, and it cannot have ejected anything either.
pub(crate) fn apply(
    cells: &[ProxyCell],
    bores: &[Bore],
) -> (Vec<ProxyCell>, Vec<Vec<Plane>>, Vec<Plug>) {
    // With an empty `bores` this is one clone of a handful of cells and the pipeline is otherwise
    // byte-identical. One path, no `if bores.is_empty()` branch to diverge.
    let mut cells: Vec<ProxyCell> = cells.to_vec();
    let mut landed: Vec<Vec<Plane>> = Vec::new();
    let mut plugs: Vec<Plug> = Vec::new();
    let cells_before = cells.len();
    let mut ejected = 0.0f32;
    let mut consumed = 0usize;

    for bore in bores {
        let Some(prism) = prism(bore) else { continue }; // already warned
        let axis = (bore.to - bore.from).normalize_or_zero();
        let mut next: Vec<ProxyCell> = Vec::with_capacity(cells.len());
        // Staged, because a bore that turns out to have reached nothing must contribute no plug —
        // and that is only known after every cell has been tried.
        let mut mine: Vec<Plug> = Vec::new();
        for cell in &cells {
            match subtract(cell, &prism) {
                None => next.push(cell.clone()),
                Some(Cut { shards, plug }) => {
                    if shards.is_empty() {
                        consumed += 1;
                    }
                    ejected += plug.volume();
                    next.extend(shards);
                    mine.push(Plug {
                        cell: plug,
                        prism: landed.len(),
                        exit: bore.to,
                        direction: axis,
                        shatter: bore.shatter,
                    });
                }
            }
        }
        if mine.is_empty() {
            warn!(
                "carnage: a bore from {:?} to {:?} (radius {}) reached no proxy cell; nothing was \
                 carved",
                bore.from, bore.to, bore.radius
            );
            continue;
        }
        cells = next;
        landed.push(prism);
        plugs.append(&mut mine);
    }

    if !landed.is_empty() {
        info!(
            "carnage: bored {} channel(s); {cells_before} cells became {}, ejecting {} plug(s) \
             holding {ejected} of volume",
            landed.len(),
            cells.len(),
            plugs.len()
        );
        if consumed > 0 {
            info!("carnage: {consumed} of those cells were swallowed whole and left as plugs");
        }
    }
    (cells, landed, plugs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CutSettings;
    use crate::bond::{BondGraph, BondSet};
    use crate::mesh::fracture_mesh;
    use crate::soup::signed_dist;
    use bevy::math::{Mat4, Vec3, primitives::Cuboid};
    use bevy::mesh::{Mesh, VertexAttributeValues};

    /// A 1×2×1 cuboid and the single convex cell that is exactly its own shape.
    ///
    /// The proxy being *exact* is the point: any defect this fixture shows is the cutter's, not an
    /// artefact of a proxy that approximates the mesh. Same fixture as `audit`'s `cube_parts`.
    fn cube_parts() -> (Mesh, Vec<ProxyCell>) {
        (
            Mesh::from(Cuboid::new(1.0, 2.0, 1.0)),
            vec![ProxyCell::from_box(Vec3::ZERO, Vec3::new(0.5, 1.0, 0.5))],
        )
    }

    /// A bore straight up the box's `Y` axis, entering below it and leaving above it.
    ///
    /// `shatter: 1` on purpose, so every test built on this measures **one** plug: the plug's own
    /// closure, its volume against the channel's, its skin, its exit. Shattering is a separate claim
    /// with its own tests, and mixing the two would mean no test measured either cleanly.
    fn through_y(radius: f32, sides: u32, jaggedness: f32, flare: f32) -> Bore {
        Bore {
            from: Vec3::new(0.0, -1.5, 0.0),
            to: Vec3::new(0.0, 1.5, 0.0),
            radius,
            sides,
            jaggedness,
            flare,
            shatter: 1,
        }
    }

    /// The same bore, broken into `shatter` pieces.
    fn shattered_y(radius: f32, shatter: u32) -> Bore {
        Bore { shatter, ..through_y(radius, 8, 0.0, 0.0) }
    }

    fn bake(cube: &Mesh, proxy: &[ProxyCell], target: usize, bores: Vec<Bore>) -> crate::Fracture {
        fracture_mesh(
            &[(cube, Mat4::IDENTITY)],
            proxy,
            &CutSettings { bores, ..CutSettings::new(target, 0.04, 0x5EED) },
        )
    }

    /// Is `p` inside every one of the prism's half-spaces?
    fn inside(p: Vec3, prism: &[Plane]) -> bool {
        prism.iter().all(|plane| signed_dist(p, plane) <= EPS)
    }

    /// **The theorem the whole design rests on: `C ∩ H` is convex, so every shard of `C \ P` is a
    /// closed convex solid.**
    ///
    /// Prediction: for every combination of dials, every leaf of a bored bake audits as watertight,
    /// manifold, consistently oriented, χ = 2 and solid enough for a collider — because a bore is
    /// nothing but a run of half-space intersections, and there is no input for which that can fail.
    ///
    /// Swept rather than pinned, and the seed sweep is the load-bearing part: slivers are the failure
    /// mode here as everywhere in this crate, and `audit`'s own sweep records that "a pinned seed is
    /// what missed it".
    #[test]
    fn every_shard_of_a_bored_cell_is_still_a_closed_convex_solid() {
        let (cube, proxy) = cube_parts();
        for radius in [0.02f32, 0.05, 0.12] {
            for sides in [3u32, 8, 24] {
                for jaggedness in [0.0f32, 0.35, 1.0] {
                    for flare in [0.0f32, 0.5] {
                        for seed in 0..20u32 {
                            let bore = through_y(radius, sides, jaggedness, flare);
                            let cut = CutSettings {
                                bores: vec![bore],
                                ..CutSettings::new(6, 0.04, seed.wrapping_mul(2_654_435_761))
                            };
                            let pieces =
                                fracture_mesh(&[(&cube, Mat4::IDENTITY)], &proxy, &cut).into_leaves();
                            let what = format!(
                                "radius {radius}, {sides} sides, jaggedness {jaggedness}, flare \
                                 {flare}, seed {seed}"
                            );
                            assert!(!pieces.is_empty(), "{what}: the bored bake produced nothing");
                            for (i, p) in pieces.iter().enumerate() {
                                let a = crate::audit::audit_proxy(p).unwrap_or_else(|e| {
                                    panic!("{what}: shard {i} could not be audited: {e}")
                                });
                                assert_eq!(a.boundary_edges, 0, "{what}: shard {i} is open: {a:?}");
                                assert!(a.is_manifold(), "{what}: shard {i} is not a manifold: {a:?}");
                                assert_eq!(
                                    a.inconsistently_oriented_edges, 0,
                                    "{what}: shard {i} has an inside-out face: {a:?}"
                                );
                                assert_eq!(
                                    a.euler_characteristic, 2,
                                    "{what}: shard {i} is not a topological sphere: {a:?}"
                                );
                                assert!(
                                    a.supports_inside_outside,
                                    "{what}: shard {i} is not solid enough for a collider: {a:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// **The channel is exactly the prism, and the prism is the inscribed polygon.**
    ///
    /// A clean 24-gon bore of circumradius `r` through the 1×2×1 box removes
    /// `0.5 · 24 · r² · sin(TAU/24) · 2.0` = 0.0621 at `r = 0.1`. Prediction: the measured loss lands
    /// within `1e-3` of that, and is strictly *less* than `π r² · 2.0`, because an inscribed polygon
    /// cannot remove more than its circumscribed cylinder.
    #[test]
    fn a_bore_removes_the_channel_and_nothing_else() {
        let (cube, proxy) = cube_parts();
        let r = 0.1f32;
        // `target: 1` so no fracture cut runs and the only geometry change is the bore.
        let pieces = bake(&cube, &proxy, 1, vec![through_y(r, 24, 0.0, 0.0)]).into_leaves();
        let left: f32 = pieces.iter().map(|p| p.cell.volume()).sum();
        let lost = 2.0 - left;

        let polygon = 0.5 * 24.0 * r * r * (std::f32::consts::TAU / 24.0).sin() * 2.0;
        assert!(
            (lost - polygon).abs() < 1.0e-3,
            "the bore removed {lost}, but the inscribed 24-gon channel is {polygon}"
        );
        let cylinder = std::f32::consts::PI * r * r * 2.0;
        assert!(
            lost < cylinder,
            "the bore removed {lost}, more than its circumscribed cylinder {cylinder} — an \
             inscribed polygon cannot do that"
        );
    }

    /// **A miss is a miss: the cell comes back the same object, not a re-clipped copy of itself.**
    ///
    /// This is the test that catches a `subtract` returning `Some(vec![cell.clone()])` where it
    /// should return `None` — the fragment count would still be right while every vertex had been
    /// through the weld an extra time.
    #[test]
    fn a_bore_that_misses_the_subject_changes_nothing() {
        let (cube, proxy) = cube_parts();
        let miss = Bore {
            from: Vec3::new(5.0, -1.5, 0.0),
            to: Vec3::new(5.0, 1.5, 0.0),
            ..Bore::new(Vec3::ZERO, Vec3::Y, 0.05)
        };
        let plain = bake(&cube, &proxy, 8, Vec::new()).into_leaves();
        let bored = bake(&cube, &proxy, 8, vec![miss]).into_leaves();
        assert_eq!(plain.len(), bored.len(), "a missed bore changed the fragment count");
        for (i, (a, b)) in plain.iter().zip(&bored).enumerate() {
            let pa: Vec<u32> = a.cell.points().iter().flat_map(|p| p.to_array()).map(f32::to_bits).collect();
            let pb: Vec<u32> = b.cell.points().iter().flat_map(|p| p.to_array()).map(f32::to_bits).collect();
            assert_eq!(pa, pb, "fragment {i} moved because of a bore that reached nothing");
        }
    }

    /// **Jaggedness bites inward only, so `radius` stays the bound on the entry hole.**
    ///
    /// Prediction: a ragged bore removes strictly *less* than a clean one of the same radius. An
    /// ordering, not a magnitude — the hash's exact bite is not a promise.
    #[test]
    fn jaggedness_only_bites_inward_so_the_entry_never_exceeds_the_radius() {
        let (cube, proxy) = cube_parts();
        let lost = |jaggedness: f32| -> f32 {
            let pieces =
                bake(&cube, &proxy, 1, vec![through_y(0.1, 8, jaggedness, 0.0)]).into_leaves();
            2.0 - pieces.iter().map(|p| p.cell.volume()).sum::<f32>()
        };
        let clean = lost(0.0);
        let ragged = lost(1.0);
        assert!(clean > 0.0, "the clean bore removed nothing");
        assert!(ragged > 0.0, "the ragged bore removed nothing");
        assert!(
            ragged < clean,
            "jaggedness must only ever bite inward: clean removed {clean}, ragged removed {ragged}"
        );
    }

    /// **Flare widens the exit and leaves the entry exactly where it was.**
    ///
    /// `flare` tilts each barrel plane rather than adding any, so the claim is about one plane's
    /// distance from the axis measured at two heights: at the entry it must be the apothem
    /// `radius · cos(π/n)`, and at the exit that times `1 + flare`. Prediction: both to within float
    /// noise, and the plane count unchanged, because a tilt is not a new face.
    ///
    /// This is the assertion behind `docs/holes.gif`'s third claim — that the exit side seen during
    /// the orbit is the wider one. Measured here rather than counted in pixels, where the oblique
    /// view of a face confounds the comparison.
    #[test]
    fn flare_widens_the_exit_and_leaves_the_entry_where_it_was() {
        let (radius, sides) = (0.1f32, 8u32);
        let apothem = radius * (std::f32::consts::PI / sides as f32).cos();
        let straight = prism(&through_y(radius, sides, 0.0, 0.0)).expect("a clean bore");
        let flared = prism(&through_y(radius, sides, 0.0, 0.6)).expect("a flared bore");
        assert_eq!(straight.len(), flared.len(), "flare must not change the plane count");

        // **The channel's radius along a facet's own outward direction, at height `h` up the axis.**
        // The plane's *perpendicular* distance to the axis is this times the cosine of the tilt,
        // which is not what `radius` names — so solve for where the plane crosses the ray
        // `axis(h) + dir·t` instead. `dir` is read back off the plane's own normal (its horizontal
        // part) rather than recomputed from `plane_basis`, so the test does not restate an internal
        // choice it is not testing.
        let radial = |plane: &Plane, h: f32| -> f32 {
            let dir = Vec3::new(plane.normal.x, 0.0, plane.normal.z).normalize();
            let base = Vec3::new(0.0, -1.5 + h, 0.0);
            -signed_dist(base, plane) / dir.dot(plane.normal)
        };
        // `through_y` runs the axis from y = -1.5 to y = +1.5, so the exit is 3.0 up it.
        for (i, (s, f)) in straight.iter().zip(&flared).take(sides as usize).enumerate() {
            for (label, plane) in [("straight", s), ("flared", f)] {
                assert!(
                    (radial(plane, 0.0) - apothem).abs() < 1.0e-6,
                    "{label} barrel plane {i} opens at {} on entry, not the apothem {apothem}",
                    radial(plane, 0.0)
                );
            }
            assert!(
                (radial(s, 3.0) - apothem).abs() < 1.0e-6,
                "an unflared barrel plane {i} must stay parallel: {} at the exit",
                radial(s, 3.0)
            );
            assert!(
                (radial(f, 3.0) - apothem * 1.6).abs() < 1.0e-6,
                "flared barrel plane {i} reaches {} at the exit, expected {} (1.6 × the apothem)",
                radial(f, 3.0),
                apothem * 1.6
            );
        }
    }

    /// **A bored subject is still one island** — the claim the whole bond argument rests on.
    ///
    /// Shard *k* and shard *k+1* share plane *k*'s face region bit-for-bit, because `clip` hands both
    /// halves the same ring. So the shards form a connected chain and `islands` must find exactly
    /// one, with no fracture cut needed to hold them together.
    #[test]
    fn a_bored_cell_is_still_one_island() {
        let (cube, proxy) = cube_parts();
        let baked = bake(&cube, &proxy, 1, vec![through_y(0.1, 8, 0.0, 0.0)]);
        let ids = baked.tree.leaves();
        assert!(ids.len() > 2, "the bore should have made several shards, got {}", ids.len());
        let members: Vec<(crate::FragmentId, &ProxyCell)> = ids
            .iter()
            .filter_map(|id| baked.fragments.get(id.index()).map(|f| (*id, &f.cell)))
            .collect();
        let graph = BondGraph::of(&members, baked.tree.len());
        let found = graph.islands(&ids, &BondSet::new(&graph));
        assert_eq!(
            found.len(),
            1,
            "{} shards of one bored cell came back as {} islands, not one",
            ids.len(),
            found.len()
        );
    }

    /// **The skin opens exactly where the channel crosses it, and nowhere else.**
    ///
    /// Prediction: no surviving skin triangle's centroid lies inside the prism — `carve` clips rather
    /// than culls, so the new boundary is on the barrel planes — and the skin loses roughly twice the
    /// channel's cross-section, once at entry and once at exit.
    ///
    /// Measured at `soften = 0.0`, deliberately. The softening relaxes each fragment's skin
    /// independently and shrinks it, and 24 shards shrink 24 times: at the shipped `soften = 0.5`
    /// this fixture's skin came back 3.1 against the unbored 10.0, which measures the relaxation
    /// rather than the carve. Softening has its own tests.
    #[test]
    fn the_skin_opens_exactly_where_the_channel_crosses_it() {
        let (cube, proxy) = cube_parts();
        let bore = through_y(0.1, 24, 0.0, 0.0);
        let prism = prism(&bore).expect("a 0.1-radius 24-gon bore is a valid channel");

        let skin_area = |pieces: &[crate::FragmentGeometry]| -> f32 {
            pieces.iter().filter_map(|p| p.outer.as_ref()).map(mesh_area).sum()
        };
        let unsoftened = |bores: Vec<Bore>| -> Vec<crate::FragmentGeometry> {
            fracture_mesh(
                &[(&cube, Mat4::IDENTITY)],
                &proxy,
                &CutSettings { bores, soften: 0.0, ..CutSettings::new(1, 0.04, 0x5EED) },
            )
            .into_leaves()
        };
        let plain = unsoftened(Vec::new());
        let bored = unsoftened(vec![bore]);

        for (i, p) in bored.iter().enumerate() {
            let Some(mesh) = p.outer.as_ref() else { continue };
            for c in mesh_centroids(mesh, p.center_local) {
                assert!(
                    !inside(c, &prism),
                    "shard {i} kept a skin triangle at {c:?}, inside the channel"
                );
            }
        }

        let lost = skin_area(&plain) - skin_area(&bored);
        let section = 0.5 * 24.0 * 0.1 * 0.1 * (std::f32::consts::TAU / 24.0).sin();
        assert!(
            lost > section * 1.5 && lost < section * 2.5,
            "the skin lost {lost}; entry plus exit is about {} (2 × {section})",
            section * 2.0
        );
    }

    /// **A bore is a function of its own geometry, so two bakes agree bit for bit.**
    ///
    /// The jaggedness hash keys on the entry point's quantized position and the side index — no seed,
    /// no clock, no iteration order — so this must hold with the ragged dials at their widest.
    #[test]
    fn boring_is_bit_identical_across_runs() {
        let (cube, proxy) = cube_parts();
        let run = || {
            bake(&cube, &proxy, 6, vec![through_y(0.08, 8, 1.0, 0.5)])
                .into_leaves()
                .into_iter()
                .map(|f| {
                    let pts: Vec<u32> =
                        f.cell.points().iter().flat_map(|p| p.to_array()).map(f32::to_bits).collect();
                    (pts, f.center_local.to_array().map(f32::to_bits))
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run(), "two bakes of the same bore disagreed bit for bit");
    }

    /// **A bore narrower than the assignment nudge is refused at the door, not clamped.**
    ///
    /// `INWARD_NUDGE` pushes a triangle's centroid `1e-3` behind its own surface to decide which cell
    /// owns it, so a channel of that order loses the skin at its own rim. Prediction: `prism` returns
    /// `None`, and the bake is indistinguishable from an unbored one.
    #[test]
    fn a_bore_narrower_than_the_assignment_nudge_is_refused() {
        let (cube, proxy) = cube_parts();
        let too_thin = through_y(1.0e-3, 8, 0.0, 0.0);
        assert!(prism(&too_thin).is_none(), "a 1e-3 radius bore must be refused");

        let plain = bake(&cube, &proxy, 8, Vec::new()).into_leaves();
        let bored = bake(&cube, &proxy, 8, vec![too_thin]).into_leaves();
        assert_eq!(plain.len(), bored.len(), "a refused bore changed the fragment count");
        for (i, (a, b)) in plain.iter().zip(&bored).enumerate() {
            assert_eq!(
                a.cell.points().iter().map(|p| p.to_array().map(f32::to_bits)).collect::<Vec<_>>(),
                b.cell.points().iter().map(|p| p.to_array().map(f32::to_bits)).collect::<Vec<_>>(),
                "fragment {i} moved because of a bore that was refused"
            );
        }
    }

    /// **A bore that swallows a cell whole removes it**, loudly and without a sliver left behind.
    ///
    /// Two cells side by side; a channel wide enough to contain one of them. Prediction: the
    /// surviving root count drops by exactly one and the other cell is untouched.
    #[test]
    fn a_bore_that_swallows_a_cell_removes_it() {
        let cells = vec![
            ProxyCell::from_box(Vec3::new(-1.0, 0.0, 0.0), Vec3::splat(0.2)),
            ProxyCell::from_box(Vec3::new(1.0, 0.0, 0.0), Vec3::splat(0.2)),
        ];
        let swallow = Bore {
            from: Vec3::new(-1.0, -2.0, 0.0),
            to: Vec3::new(-1.0, 2.0, 0.0),
            // The 8-gon's apothem is `radius · cos(22.5°)`, so 0.5 reaches 0.462 — past the cell's
            // own 0.283 corner radius and nowhere near the neighbour 2.0 away.
            radius: 0.5,
            sides: 8,
            jaggedness: 0.0,
            flare: 0.0,
            shatter: 1,
        };
        let prism = prism(&swallow).expect("a 0.5-radius bore is a valid channel");
        let left = apply(&cells, &[swallow]).0;
        assert!(
            subtract(&cells[0], &prism).is_some_and(|c| c.shards.is_empty()),
            "the first cell should have been consumed whole"
        );
        assert_eq!(left.len(), 1, "one of the two cells should be gone, got {} left", left.len());
        assert_eq!(left[0], cells[1], "the cell the bore missed must come back untouched");
    }

    /// **The bore conserves volume now: shards plus plug is the cell, exactly.**
    ///
    /// This is the invariant keeping the plug bought. Before it, the bore was the one operation in the
    /// crate that destroyed solid — the channel's material simply vanished — and there was no way to
    /// state a conservation law about a bored subject. `subtract` is a partition of the cell into
    /// half-space intersections, so the sum is exact up to the weld, and the tolerance here is the
    /// same `1e-3` the fracture's own conservation test uses.
    ///
    /// Swept across the dials because slivers are the failure mode: a shard that collapses under
    /// `MIN_CROSS2` takes its volume with it, and this is what would notice.
    #[test]
    fn the_shards_and_the_plug_are_the_cell_exactly() {
        let cell = ProxyCell::from_box(Vec3::ZERO, Vec3::new(0.5, 1.0, 0.5));
        for radius in [0.02f32, 0.05, 0.12] {
            for sides in [3u32, 8, 24] {
                for jaggedness in [0.0f32, 0.35, 1.0] {
                    for flare in [0.0f32, 0.5] {
                        let bore = through_y(radius, sides, jaggedness, flare);
                        let p = prism(&bore).expect("a valid channel");
                        let Cut { shards, plug } =
                            subtract(&cell, &p).expect("the channel crosses the cell");
                        let what =
                            format!("radius {radius}, {sides} sides, jag {jaggedness}, flare {flare}");
                        let sum: f32 =
                            shards.iter().map(|s| s.volume()).sum::<f32>() + plug.volume();
                        assert!(
                            (sum - 2.0).abs() < 1.0e-3,
                            "{what}: {} shards plus the plug enclose {sum}, not the cell's 2.0",
                            shards.len()
                        );
                        assert!(plug.volume() > 0.0, "{what}: the plug enclosed nothing");
                    }
                }
            }
        }
    }

    /// **A plug is a closed convex solid, so it is a collider like any other chunk.**
    ///
    /// It is an intersection of the cell with half-spaces, exactly as a shard is, so this is the same
    /// theorem — and it must hold for the material that *left* as much as for the material that
    /// stayed. Audited through [`crate::audit_cell`], which exists so a plug does not need a
    /// `FragmentGeometry` it has no id for.
    #[test]
    fn every_plug_is_a_closed_convex_solid() {
        let (cube, proxy) = cube_parts();
        for radius in [0.02f32, 0.05, 0.12] {
            for sides in [3u32, 8, 24] {
                for jaggedness in [0.0f32, 1.0] {
                    for flare in [0.0f32, 0.5] {
                        let baked =
                            bake(&cube, &proxy, 6, vec![through_y(radius, sides, jaggedness, flare)]);
                        let what =
                            format!("radius {radius}, {sides} sides, jag {jaggedness}, flare {flare}");
                        assert_eq!(baked.ejecta.len(), 1, "{what}: expected exactly one plug");
                        for (i, e) in baked.ejecta.iter().enumerate() {
                            let a = crate::audit_cell(&e.cell)
                                .unwrap_or_else(|err| panic!("{what}: plug {i} unauditable: {err}"));
                            assert_eq!(a.boundary_edges, 0, "{what}: plug {i} is open: {a:?}");
                            assert!(a.is_manifold(), "{what}: plug {i} is not a manifold: {a:?}");
                            assert_eq!(
                                a.euler_characteristic, 2,
                                "{what}: plug {i} is not a topological sphere: {a:?}"
                            );
                            assert!(
                                a.supports_inside_outside,
                                "{what}: plug {i} is not solid enough for a collider: {a:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// **A plug is not a fragment, and cannot bond itself back into the hole it came from.**
    ///
    /// The whole reason [`Ejecta`](crate::Ejecta) is its own type. A plug's barrel faces are the same
    /// rings the shards got, so if it were a proxy cell the bond match would find it — correctly — and
    /// the channel would be filled by a piece welded across it. Prediction: the tree and the bond
    /// graph are *identical* to a bake of the same bore that ignores ejecta, the body is still one
    /// island, and no leaf's cell equals the plug's.
    #[test]
    fn a_plug_is_absent_from_the_tree_and_from_the_bonds() {
        let (cube, proxy) = cube_parts();
        let baked = bake(&cube, &proxy, 1, vec![through_y(0.1, 8, 0.0, 0.0)]);
        let ids = baked.tree.leaves();

        assert_eq!(baked.ejecta.len(), 1, "one bore through one cell is one plug");
        assert_eq!(
            baked.fragments.len(),
            ids.len(),
            "at target 1 every node is a leaf, so a plug must not have been added as one"
        );
        let plug = &baked.ejecta[0].cell;
        for (i, f) in baked.fragments.iter().enumerate() {
            assert_ne!(
                f.cell.points(),
                plug.points(),
                "fragment {i} IS the plug — it was added to the proxy instead of ejected"
            );
        }

        // And the shards it left behind are still one island, with the plug gone.
        let members: Vec<(crate::FragmentId, &ProxyCell)> = ids
            .iter()
            .filter_map(|id| baked.fragments.get(id.index()).map(|f| (*id, &f.cell)))
            .collect();
        let graph = BondGraph::of(&members, baked.tree.len());
        assert_eq!(
            graph.islands(&ids, &BondSet::new(&graph)).len(),
            1,
            "the {} shards around the channel must still be one island",
            ids.len()
        );
    }

    /// **A plug carries both materials: the channel wall and the skin the channel tore out.**
    ///
    /// That contrast is the whole read — a chunk of gore that is mostly raw interior with a patch of
    /// the subject's own surface at each end where the shot went in and came out. `cap` is the wall,
    /// `outer` is the patches, and a plug through a solid must have both.
    ///
    /// Measured with **both** softening dials at zero, for the reason
    /// `the_skin_opens_exactly_where_the_channel_crosses_it` already records: the relaxation subdivides
    /// and moves the drawn surface, and on a plug it *grows* the end discs rather than shrinking them —
    /// 0.244 against the carve's own 0.089 at `soften = 0.5`, because a disc welded to a barrel ring
    /// bulges outward when it relaxes.
    ///
    /// `ejecta_soften` has to be pinned here too, and this test is what found that out: it went red at
    /// 0.256 the moment that dial gained its shipped 0.55 default, which is exactly the evidence that
    /// the new dial reaches ejecta rather than being ignored.
    #[test]
    fn a_plug_carries_the_wall_and_the_skin_the_channel_tore_out() {
        let (cube, proxy) = cube_parts();
        let baked = fracture_mesh(
            &[(&cube, Mat4::IDENTITY)],
            &proxy,
            &CutSettings {
                bores: vec![through_y(0.12, 24, 0.0, 0.0)],
                soften: 0.0,
                ejecta_soften: 0.0,
                ..CutSettings::new(1, 0.04, 0x5EED)
            },
        );
        let e = &baked.ejecta[0];
        assert!(e.cap.is_some(), "the plug has no channel wall to give the interior material");
        assert!(
            e.outer.is_some(),
            "the plug has no skin patches — the entry and exit discs were dropped rather than carried"
        );
        // Both ends of a through-shot, so the patches are two discs, not one.
        let area = mesh_area(e.outer.as_ref().expect("skin"));
        let disc = 0.5 * 24.0 * 0.12 * 0.12 * (std::f32::consts::TAU / 24.0).sin();
        assert!(
            area > disc * 1.5 && area < disc * 2.5,
            "the plug's skin is {area}; entry plus exit is about {} (2 × {disc})",
            disc * 2.0
        );
    }

    /// **A plug knows where it left and which way it was going**, so a caller can throw it.
    ///
    /// Both are geometric facts about the [`Bore`] rather than physics: `exit` is its `to` and
    /// `direction` is its normalised axis. Pinned because the demo's gore flies along them, and a sign
    /// error would send every chunk back into the subject.
    #[test]
    fn a_plug_leaves_along_the_channel_and_exits_where_the_bore_did() {
        let (cube, proxy) = cube_parts();
        let bore = through_y(0.1, 8, 0.0, 0.0);
        let baked = bake(&cube, &proxy, 1, vec![bore]);
        let e = &baked.ejecta[0];
        assert_eq!(e.exit, bore.to, "the plug did not leave at the bore's own far end");
        assert!(
            (e.direction - Vec3::Y).length() < 1.0e-6,
            "a bore along +Y must eject along +Y, got {:?}",
            e.direction
        );
        // The plug's own centre sits on the channel axis, between entry and exit.
        assert!(
            e.center_local.x.abs() < 1.0e-3 && e.center_local.z.abs() < 1.0e-3,
            "the plug's centre {:?} is off the channel axis",
            e.center_local
        );
    }

    /// **A bore that never reached the solid ejects nothing**, whether it missed or was refused.
    ///
    /// The counterpart of the two existing refusal tests. Gore that appears for a shot which hit
    /// nothing is worse than no gore: it is the feature claiming a hit the geometry did not make.
    #[test]
    fn a_bore_that_reached_nothing_ejects_nothing() {
        let (cube, proxy) = cube_parts();
        let miss = Bore {
            from: Vec3::new(5.0, -1.5, 0.0),
            to: Vec3::new(5.0, 1.5, 0.0),
            ..Bore::new(Vec3::ZERO, Vec3::Y, 0.05)
        };
        assert!(bake(&cube, &proxy, 8, vec![miss]).ejecta.is_empty(), "a missed bore ejected a plug");
        let refused = through_y(1.0e-3, 8, 0.0, 0.0);
        assert!(
            bake(&cube, &proxy, 8, vec![refused]).ejecta.is_empty(),
            "a refused bore ejected a plug"
        );
        assert!(
            bake(&cube, &proxy, 8, Vec::new()).ejecta.is_empty(),
            "an unbored bake ejected a plug"
        );
    }

    /// **Shattering divides the plug and nothing else: the pieces tile it exactly.**
    ///
    /// The dial that answers "the plug looks like someone used an apple corer" — because a plug *is*
    /// a convex prism, and one prism cannot look like anything else. Prediction: the piece count
    /// tracks what was asked (a fat plug divides cleanly at every count the crate allows), the summed
    /// volume is the plug's own to within the fracture's usual `1e-3`, and out-of-range counts clamp
    /// rather than refusing the bore and losing the hole with it.
    #[test]
    fn shattering_divides_the_plug_and_conserves_it() {
        let (cube, proxy) = cube_parts();
        // The whole plug, unshattered, is the reference volume.
        let whole = bake(&cube, &proxy, 1, vec![shattered_y(0.12, 1)]);
        assert_eq!(whole.ejecta.len(), 1, "shatter 1 must leave the plug whole");
        let plug_volume = whole.ejecta[0].cell.volume();
        assert!(plug_volume > 0.0, "the reference plug enclosed nothing");

        for want in [1u32, 2, 3, 4, 6, 8, 12] {
            let baked = bake(&cube, &proxy, 1, vec![shattered_y(0.12, want)]);
            let n = baked.ejecta.len();
            assert_eq!(
                n, want as usize,
                "asked for {want} pieces of a 0.12 plug and got {n}"
            );
            let sum: f32 = baked.ejecta.iter().map(|e| e.cell.volume()).sum();
            assert!(
                (sum - plug_volume).abs() < 1.0e-3,
                "shatter {want}: the pieces enclose {sum}, but the plug is {plug_volume} — \
                 shattering must divide it, not resize it"
            );
        }

        // Clamped, not refused: losing the hole because a look dial was out of range would be the
        // worse failure by far.
        assert_eq!(
            bake(&cube, &proxy, 1, vec![shattered_y(0.12, 0)]).ejecta.len(),
            1,
            "shatter 0 must clamp to one whole plug"
        );
        assert_eq!(
            bake(&cube, &proxy, 1, vec![shattered_y(0.12, 999)]).ejecta.len(),
            MAX_SHATTER as usize,
            "shatter 999 must clamp to MAX_SHATTER"
        );
    }

    /// **Every shattered piece is still a closed convex solid, so it is still a collider.**
    ///
    /// The shatter is a run of half-space intersections over a cell that was already one, so this is
    /// the same theorem a third time — and it must survive the thin plugs a real calibre produces,
    /// which is where slivers live. Swept over calibre and count for that reason.
    #[test]
    fn every_shattered_piece_is_a_closed_convex_solid() {
        let (cube, proxy) = cube_parts();
        for radius in [0.02f32, 0.05, 0.12] {
            for want in [2u32, 4, 8, 12] {
                let baked = bake(&cube, &proxy, 6, vec![shattered_y(radius, want)]);
                let what = format!("radius {radius}, shatter {want}");
                assert!(!baked.ejecta.is_empty(), "{what}: nothing was ejected");
                for (i, e) in baked.ejecta.iter().enumerate() {
                    let a = crate::audit_cell(&e.cell)
                        .unwrap_or_else(|err| panic!("{what}: piece {i} unauditable: {err}"));
                    assert_eq!(a.boundary_edges, 0, "{what}: piece {i} is open: {a:?}");
                    assert!(a.is_manifold(), "{what}: piece {i} is not a manifold: {a:?}");
                    assert_eq!(
                        a.euler_characteristic, 2,
                        "{what}: piece {i} is not a topological sphere: {a:?}"
                    );
                    assert!(
                        a.supports_inside_outside,
                        "{what}: piece {i} is not solid enough for a collider: {a:?}"
                    );
                }
            }
        }
    }

    /// Summed triangle area of a mesh — the skin measurement the opening test compares.
    fn mesh_area(mesh: &Mesh) -> f32 {
        let Some(VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return 0.0;
        };
        let Some(idx) = mesh.indices() else { return 0.0 };
        let v: Vec<u32> = idx.iter().map(|i| i as u32).collect();
        let mut total = 0.0f32;
        let mut i = 0;
        while i + 2 < v.len() {
            let p = |k: usize| Vec3::from_array(pos[v[k] as usize]);
            total += 0.5 * (p(i + 1) - p(i)).cross(p(i + 2) - p(i)).length();
            i += 3;
        }
        total
    }

    /// Every triangle centroid of a mesh, back in subject-local space.
    fn mesh_centroids(mesh: &Mesh, recenter: Vec3) -> Vec<Vec3> {
        let Some(VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return Vec::new();
        };
        let Some(idx) = mesh.indices() else { return Vec::new() };
        let v: Vec<u32> = idx.iter().map(|i| i as u32).collect();
        let mut out = Vec::with_capacity(v.len() / 3);
        let mut i = 0;
        while i + 2 < v.len() {
            let p = |k: usize| Vec3::from_array(pos[v[k] as usize]) + recenter;
            out.push((p(i) + p(i + 1) + p(i + 2)) / 3.0);
            i += 3;
        }
        out
    }
}
