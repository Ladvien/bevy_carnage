//! **An impact, projected onto the modes, and the partition it leaves.**
//!
//! Sellán et al. (`doi:10.1145/3549540`, §3.4): at runtime an impact at point `p` with direction
//! `n⃗` is blurred one implicit step along the shape, `g = (M + τL)⁻¹ M δ_p`, projected onto the
//! precomputed modes, `w* = Σ_i U_i A_i δ_p n⃗`, and the exploded mesh is **glued back together**
//! wherever two coincident vertices' deformations agree: `‖w*_af − w*_cg‖ < σ`. What is left apart
//! is the fracture pattern. Because the projection is linear, *"scaling σ and scaling the magnitude
//! of the impact are equivalent"*, so `σ` is fixed at their `10⁻³` and the impulse carries the size
//! of the blow.
//!
//! On the cell graph the modes are scalar (see [`crate::fracture_modes::modes`]), the direction factors out of the
//! norm, and the gluing rule on a shared face reads `|s_a − s_b| · |impulse| < σ` with
//! `s = Σ_i φ_i · A_i[p]`. Cells joined by an unbroken face are one fragment.

use crate::fracture_modes::modes::ModeSet;

/// **A blow.** Which cell it lands in, and how hard.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Impact {
    /// The cell the impact lands in.
    pub cell: usize,
    /// Impulse magnitude, in the units the gluing tolerance is read in. Direction has already
    /// factored out; only how hard remains.
    pub magnitude: f32,
}

/// The gluing tolerance `σ`, the paper's own value. An impulse of `1.0` at the shape's own scale
/// opens every fault the modes prefer; scale the impulse, not this.
pub const SIGMA: f32 = 1.0e-3;

/// **The pieces an impact leaves.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partition {
    /// For each cell, which group it belongs to.
    pub group_of: Vec<usize>,
    /// The groups, each a sorted list of cells, ordered by their smallest cell.
    pub groups: Vec<Vec<usize>>,
    /// Indices into the baked face list of every face that broke.
    pub broken: Vec<usize>,
}

impl Partition {
    /// How many fragments the impact produced.
    pub fn fragment_count(&self) -> usize {
        self.groups.len()
    }
}

impl ModeSet {
    /// **The projected response of every cell** to `impact`:
    /// `s_j = Σ_i φ_i[j] · A_i[p] · magnitude / E_D(φ_i)`.
    ///
    /// **One departure from the paper, stated.** Sellán et al. project with every mode weighted
    /// equally. This crate divides each mode's contribution by its discontinuity energy — the
    /// area-weighted sum of the jumps it opens, which is the work its fault set costs — so a weak
    /// fault opens under a small blow and a strong one needs a large one. Without it, on a graph
    /// small enough that a few modes span most of it, an impulse at a cell excites whichever modes
    /// are large *there* and a thin neck elsewhere never opens first; with it, the neck is the first
    /// face to give, which is the property this crate exists for and its tests pin.
    ///
    /// Empty when the impact names a cell the set does not have.
    pub fn response(&self, impact: &Impact) -> Vec<f32> {
        if impact.cell >= self.cells || !impact.magnitude.is_finite() {
            return Vec::new();
        }
        let mut s = vec![0.0f32; self.cells];
        for m in &self.modes {
            let Some(row) = m.impact_row.get(impact.cell) else { continue };
            let energy = if m.energy.is_finite() && m.energy > 1.0e-6 { m.energy } else { 1.0e-6 };
            let coef = row * impact.magnitude / energy;
            for (out, phi) in s.iter_mut().zip(m.phi.iter()) {
                *out += coef * phi;
            }
        }
        s
    }

    /// **Partition the cells** under `impact`: a face stays glued when its two cells' responses
    /// differ by less than [`SIGMA`], and connected cells are one fragment.
    ///
    /// An impact the set cannot answer (bad cell, non-finite impulse) leaves everything in one
    /// piece — refusing to fracture is the safe side.
    pub fn partition(&self, impact: &Impact) -> Partition {
        self.partition_at(impact, SIGMA)
    }

    /// [`partition`](Self::partition) with an explicit tolerance.
    pub fn partition_at(&self, impact: &Impact, sigma: f32) -> Partition {
        let n = self.cells;
        let s = self.response(impact);
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let mut broken = Vec::new();
        for (f, &(a, b)) in self.faces.iter().enumerate() {
            if a >= n || b >= n {
                continue;
            }
            let jump = match (s.get(a), s.get(b)) {
                (Some(x), Some(y)) => (x - y).abs(),
                _ => 0.0,
            };
            if jump >= sigma {
                broken.push(f);
            } else {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    // Smaller root wins, so the union is order-independent.
                    let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                    parent[hi] = lo;
                }
            }
        }
        let mut root_of: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
        // Groups ordered by their smallest member: roots are the smallest member by construction.
        // SORT-OK: cell indices from a Vec, deduplicated after — a total order, no ECS query near it.
        let mut roots: Vec<usize> = root_of.clone();
        roots.sort_unstable();
        roots.dedup();
        let mut group_of = vec![0usize; n];
        let mut groups: Vec<Vec<usize>> = vec![Vec::new(); roots.len()];
        for (cell, root) in root_of.iter_mut().enumerate() {
            let g = roots.binary_search(root).unwrap_or(0);
            group_of[cell] = g;
            groups[g].push(cell);
        }
        Partition { group_of, groups, broken }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fracture_modes::graph::CellGraph;
    use crate::fracture_modes::modes::ModeSettings;

    fn bar() -> ModeSet {
        ModeSet::bake(&CellGraph::bar(10, 4, 0.05), &ModeSettings { k: 3, ..Default::default() }).expect("bar bakes")
    }

    /// **The first thing to give is the neck.** As a blow at either end of the necked bar grows,
    /// the first face to open is the thin one — and it opens alone. That is the property geometric
    /// prefracture cannot have and this method exists for.
    #[test]
    fn the_neck_is_the_first_face_to_break() {
        let set = bar();
        for cell in [0usize, 9] {
            let mut first = None;
            for i in 0..400 {
                let magnitude = 1.0e-4 * 1.03f32.powi(i);
                let p = set.partition(&Impact { cell, magnitude });
                if p.fragment_count() > 1 {
                    first = Some((magnitude, p));
                    break;
                }
            }
            let (magnitude, p) = first.expect("some impulse breaks the bar");
            assert_eq!(p.broken, vec![4], "blow at {cell}, impulse {magnitude}: {p:?}");
            assert_eq!(p.groups, vec![vec![0, 1, 2, 3, 4], vec![5, 6, 7, 8, 9]]);
        }
    }

    /// A feeble blow leaves the bar whole; a harder one produces at least as many pieces.
    #[test]
    fn fragment_count_is_monotone_in_impulse() {
        let set = bar();
        let mut last = 0usize;
        for i in 0..=8 {
            let magnitude = 10f32.powi(i - 6);
            let n = set.partition(&Impact { cell: 9, magnitude }).fragment_count();
            assert!(n >= last, "pieces fell from {last} to {n} at impulse {magnitude}");
            last = n;
        }
        assert_eq!(set.partition(&Impact { cell: 9, magnitude: 0.0 }).fragment_count(), 1);
    }

    /// A bad impact is refused by leaving everything whole.
    #[test]
    fn a_bad_impact_leaves_one_piece() {
        let set = bar();
        assert_eq!(set.partition(&Impact { cell: 99, magnitude: 1.0 }).fragment_count(), 1);
        assert_eq!(set.partition(&Impact { cell: 0, magnitude: f32::NAN }).fragment_count(), 1);
    }

    /// The projection is deterministic to the bit across two bakes.
    #[test]
    fn projection_is_deterministic() {
        let a = bar().response(&Impact { cell: 2, magnitude: 0.7 });
        let b = bar().response(&Impact { cell: 2, magnitude: 0.7 });
        assert_eq!(a, b);
    }
}
