//! **A bounding-volume hierarchy over a mesh's triangles**, for casting rays at it.
//!
//! A cast that walks every triangle tests all of them for every ray, and a dressed crowd casts a
//! shotgun's spray at hundreds of meshes: at forty characters that was 368,000 triangle tests a tick.
//! The hierarchy tests the few triangles whose boxes the ray enters, nearest first.
//!
//! **It answers exactly what walking every triangle answers.** A walk keeps the nearest crossing,
//! and of two at the same distance the one earlier in the index buffer; the hierarchy keeps the
//! least `(t, ordinal)` over every triangle it tests, and it tests every triangle the ray can cross at
//! or before the best so far, because a node's box is padded past its triangles and is entered up to
//! and including that distance. The crossing itself is the caller's, the same function the walk
//! uses, so the arithmetic is the same to the bit.

use bevy::math::Vec3;

/// A triangle the hierarchy holds: its corners' vertex indices, and its place in the index buffer,
/// which breaks a tie between two crossings at one distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tri {
    pub corners: [u32; 3],
    pub ordinal: u32,
}

/// One node: a padded box, and either `count` triangles from `start` (a leaf) or, when `count` is
/// zero, a left child at the next node and a right child at `start`.
#[derive(Clone, Copy, Debug)]
struct Node {
    lo: Vec3,
    hi: Vec3,
    start: u32,
    count: u32,
}

/// **The hierarchy**: nodes in depth-first order, and the triangles reordered so every leaf's are
/// contiguous.
#[derive(Debug)]
pub(crate) struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<Tri>,
}

/// Triangles in a leaf, at most.
const LEAF: usize = 4;

/// The deepest a traversal goes. A median split halves a node's triangles, so a mesh would need
/// `LEAF · 2⁶²` triangles to reach it.
const DEPTH: usize = 64;

impl Bvh {
    /// **Build the hierarchy over `tris`**, whose corners index `positions`. Each node splits its
    /// triangles at the median of their centroids along the longest side of the centroids' box, ties
    /// broken by ordinal, so the same mesh builds the same tree.
    pub(crate) fn build(positions: &[[f32; 3]], tris: Vec<Tri>) -> Self {
        let at = |i: u32| positions.get(i as usize).map_or(Vec3::ZERO, |p| Vec3::from(*p));
        // Each triangle's box and centroid once, rather than from its corners at every level.
        let mut items: Vec<Item> = tris
            .into_iter()
            .map(|tri| {
                let [a, b, c] = tri.corners.map(at);
                Item { tri, lo: a.min(b).min(c), hi: a.max(b).max(c), centroid: (a + b + c) / 3.0 }
            })
            .collect();
        let mut nodes = Vec::with_capacity(2 * items.len().div_ceil(LEAF).max(1));
        let len = items.len();
        if len > 0 {
            split(&mut nodes, &mut items, 0, len);
        }
        Self { nodes, tris: items.into_iter().map(|item| item.tri).collect() }
    }

    /// **The nearest crossing of the ray from `origin` along `d`**, as `cross` measures it, and the
    /// triangle it is on: the least `(t, ordinal)` over every triangle, which is what walking them in
    /// index order and keeping the strictly nearer finds.
    pub(crate) fn nearest(
        &self,
        positions: &[[f32; 3]],
        origin: Vec3,
        d: Vec3,
        cross: impl Fn(Vec3, Vec3, [Vec3; 3]) -> Option<(f32, f32, f32)>,
    ) -> Option<(f32, f32, f32, Tri)> {
        let corner = |i: u32| positions.get(i as usize).map(|p| Vec3::from(*p));
        let mut best: Option<(f32, f32, f32, Tri)> = None;
        let mut stack = [0u32; DEPTH];
        let mut top = if self.nodes.is_empty() { 0 } else { 1 };
        while top > 0 {
            top -= 1;
            let Some(node) = stack.get(top).and_then(|&k| self.nodes.get(k as usize).map(|n| (k, *n))) else { break };
            let (k, node) = node;
            if !enters(&node, origin, d, best.map_or(f32::INFINITY, |b| b.0)) {
                continue;
            }
            if node.count > 0 {
                let (from, to) = (node.start as usize, node.start as usize + node.count as usize);
                for tri in self.tris.get(from..to).unwrap_or(&[]) {
                    let [a, b, c] = tri.corners;
                    let (Some(pa), Some(pb), Some(pc)) = (corner(a), corner(b), corner(c)) else { continue };
                    let Some((t, u, v)) = cross(origin, d, [pa, pb, pc]) else { continue };
                    if best.is_none_or(|(bt, _, _, bk)| t < bt || (t == bt && tri.ordinal < bk.ordinal)) {
                        best = Some((t, u, v, *tri));
                    }
                }
            } else if top + 2 <= DEPTH {
                // The right child first onto the stack, so the left is visited first.
                if let Some(slot) = stack.get_mut(top) {
                    *slot = node.start;
                }
                if let Some(slot) = stack.get_mut(top + 1) {
                    *slot = k + 1;
                }
                top += 2;
            }
        }
        best
    }
}

/// A triangle while the tree is built: it, its box and its centroid.
struct Item {
    tri: Tri,
    lo: Vec3,
    hi: Vec3,
    centroid: Vec3,
}

/// Build the node for `items[from..to]` at the end of `nodes`, and its subtree after it.
fn split(nodes: &mut Vec<Node>, items: &mut [Item], from: usize, to: usize) {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let (mut clo, mut chi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for item in items.get(from..to).unwrap_or(&[]) {
        lo = lo.min(item.lo);
        hi = hi.max(item.hi);
        clo = clo.min(item.centroid);
        chi = chi.max(item.centroid);
    }
    // **Padded past its triangles**, by their extent and by their distance from the origin, both far
    // beyond a float's rounding, so no crossing a walk would find lies outside its box.
    let pad = (hi - lo).max_element() * 1.0e-5 + lo.abs().max(hi.abs()).max_element() * 1.0e-6 + 1.0e-12;
    let (lo, hi) = (lo - Vec3::splat(pad), hi + Vec3::splat(pad));
    let here = nodes.len();
    nodes.push(Node { lo, hi, start: from as u32, count: (to - from) as u32 });
    if to - from <= LEAF {
        return;
    }
    let extent = chi - clo;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let mid = from + (to - from) / 2;
    if let Some(span) = items.get_mut(from..to) {
        span.select_nth_unstable_by(mid - from, |a, b| a.centroid[axis].total_cmp(&b.centroid[axis]).then(a.tri.ordinal.cmp(&b.tri.ordinal)));
    }
    split(nodes, items, from, mid);
    let right = nodes.len() as u32;
    split(nodes, items, mid, to);
    if let Some(node) = nodes.get_mut(here) {
        node.start = right;
        node.count = 0;
    }
}

/// **Whether the ray from `origin` along `d` is inside `node`'s box somewhere in `[0, reach]`**: the
/// slab test, with `reach` itself included, so a crossing tied with the best so far is still tested.
fn enters(node: &Node, origin: Vec3, d: Vec3, reach: f32) -> bool {
    let (mut near, mut far) = (0.0f32, reach);
    for axis in 0..3 {
        let (o, dir, lo, hi) = (origin[axis], d[axis], node.lo[axis], node.hi[axis]);
        if dir == 0.0 {
            if o < lo || o > hi {
                return false;
            }
            continue;
        }
        let (a, b) = ((lo - o) / dir, (hi - o) / dir);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
        if near > far {
            return false;
        }
    }
    true
}
