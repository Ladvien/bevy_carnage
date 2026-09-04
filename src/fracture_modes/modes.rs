//! **Fracture modes on a cell graph** — Sellán, Ni, Stein, Jacobson & ..., *"Breaking Good: Fracture
//! Modes for Realtime Destruction"*, ACM TOG 42(1) (2022), `doi:10.1145/3549540`, reduced to the
//! one place a convex decomposition already has a null space of rigid motions: its cells.
//!
//! # The model, as the paper states it
//!
//! A fracture mode is a displacement field `u` over the shape that minimises
//!
//! ```text
//! E(u) = E_Ψ(u) + ω · E_D(u),      E_D(u) = ‖D(u, S)‖₂,₁ = Σ_i √( ∫_{S_i} ‖D(u, x)‖² dx )
//! ```
//!
//! subject to mass-orthonormality `Uᵀ M U = I` (their Eq. 15). `E_Ψ` is a strain energy whose only
//! job is to fix a null space — the paper finds *"the precise choice Q is irrelevant and only its
//! null space matters"* (§3.7) and takes `Q = I_d ⊗ L̃`, the Laplacian, whose null space is
//! translations. `E_D` is a group-ℓ1 over the fault patches `S_i`: a mode pays for the *number* of
//! faces it opens rather than for how far, so its minimisers are piecewise constant with jumps on a
//! sparse set of faces — a fracture pattern.
//!
//! # The reduction
//!
//! On a convex decomposition every cell already moves as one piece (that is what the paper's
//! zero-strain regime finds: *"each fracture fragment undergoes its own zero-strain energy
//! transformation"*), so the unknowns are one translation per cell, the fault patches are the
//! shared faces, and the discontinuity on a face is the difference of its two cells' translations.
//! A translation field is a scalar field times a direction, and `E_D` cannot tell directions apart,
//! so the modes are **scalar** functions on the cell graph:
//!
//! ```text
//! E_D(φ) = Σ_faces √area_f · |φ_a − φ_b|,      φᵀ M φ = 1,   φ ⊥_M {1, φ_1, …, φ_{i−1}}
//! ```
//!
//! with `M = diag(cell masses)` and `L` the area-weighted graph Laplacian standing in for `Q`. The
//! direction comes back at impact time and factors out of the gluing rule, which is why a cell
//! partition needs only the scalar.
//!
//! # The solver
//!
//! The paper uses ICCM (Brandt & Hildebrandt 2017) with a conic solver, initialised from the
//! eigenvectors of `Q` because random starts *"introduce non-determinism"*. This crate keeps the
//! eigenvector initialisation — a fixed-sweep Jacobi decomposition of the generalised problem
//! `L φ = λ M φ` — and replaces the conic sub-problem with a fixed number of ADMM steps on the same
//! objective, splitting the face jumps `z = Dφ` so the group-ℓ1 becomes a soft threshold, and
//! re-imposing orthonormality by mass-weighted Gram–Schmidt after every step. Same objective, same
//! initialisation, same sequential mode-by-mode structure; a fixed schedule so two machines agree.

use crate::fracture_modes::graph::{CellGraph, GraphError};
use crate::fracture_modes::linalg::{Cholesky, SymMat, jacobi_eigen};

/// **The bake dials.** Defaults are the paper's regime: strain weight small enough that the
/// discontinuity term dominates (Fig. 12), tolerance `σ = 10⁻³` at impact.
#[derive(bevy::ecs::resource::Resource, Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ModeSettings {
    /// Modes to compute. The paper uses 10–30; a body with a few dozen cells wants ~8.
    pub k: usize,
    /// Discontinuity weight `ω`. Only its ratio to `strain` matters.
    pub omega: f32,
    /// Strain (Laplacian) weight. Small: the paper's zero-deformation fracture realm.
    pub strain: f32,
    /// ADMM penalty `ρ`.
    pub rho: f32,
    /// Weight of the linearised sphere and orthogonality constraints inside the ADMM step.
    pub constraint: f32,
    /// Outer ICCM rounds per mode — re-linearisations of the sphere constraint. Fixed.
    pub outer: u32,
    /// Inner ADMM steps per round. Fixed, so the bake is a schedule rather than a convergence test.
    pub iterations: u32,
    /// Jacobi sweeps for the eigenvector initialisation.
    pub eigen_sweeps: u32,
    /// Timestep `τ` of the impact filter `g = (M + τL)⁻¹ M δ_p` — how far an impact blurs along
    /// the graph before it is projected.
    pub tau: f32,
}

impl Default for ModeSettings {
    fn default() -> Self {
        Self { k: 8, omega: 1.0, strain: 1.0e-3, rho: 1.0, constraint: 1.0e3, outer: 8, iterations: 60, eigen_sweeps: 10, tau: 0.5 }
    }
}

/// One fracture mode: a scalar per cell, its discontinuity energy, and the precomputed impact
/// row `A_i` (the paper's Eq. 22 — `A_i = U_iᵀ M (M + τL)⁻¹ M`, so an impact is one dot product).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Mode {
    /// The mode, one value per cell, mass-normalised.
    pub phi: Vec<f32>,
    /// `E_D(φ)`: the area-weighted sum of jumps. Ascending across the set.
    pub energy: f32,
    /// `A_i`, one value per cell: the response of this mode to a unit impulse at that cell.
    pub impact_row: Vec<f32>,
}

/// **A baked mode set** for one cell graph.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ModeSet {
    /// Cells in the graph this was baked on.
    pub cells: usize,
    /// The modes, energy ascending.
    pub modes: Vec<Mode>,
    /// Per-face `(a, b)` as baked, so an impact can be partitioned without the graph.
    pub faces: Vec<(usize, usize)>,
}

/// Why a bake was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeError {
    /// The graph did not validate.
    Graph(GraphError),
    /// The system could not be factored — a degenerate graph (all masses equal and no faces is
    /// fine; a non-finite value is not).
    Singular,
}

impl From<GraphError> for BakeError {
    fn from(e: GraphError) -> Self {
        BakeError::Graph(e)
    }
}

impl ModeSet {
    /// **Bake the modes.** Pure: the same graph and settings give the same bits.
    pub fn bake(graph: &CellGraph, s: &ModeSettings) -> Result<ModeSet, BakeError> {
        graph.validate()?;
        let n = graph.len();
        let k = s.k.min(n.saturating_sub(1));
        let masses: Vec<f64> = graph.masses.iter().map(|m| *m as f64).collect();
        let faces: Vec<(usize, usize, f64)> = graph.faces.iter().map(|f| (f.a, f.b, f.area as f64)).collect();

        // Area-weighted Laplacian L and the unweighted incidence Laplacian DᵀD.
        let mut lap = SymMat::zeros(n);
        let mut inc = SymMat::zeros(n);
        for &(a, b, area) in &faces {
            lap.add(a, a, area);
            lap.add(b, b, area);
            lap.add(a, b, -area);
            lap.add(b, a, -area);
            inc.add(a, a, 1.0);
            inc.add(b, b, 1.0);
            inc.add(a, b, -1.0);
            inc.add(b, a, -1.0);
        }

        // Initialisation: generalised eigenvectors of L φ = λ M φ via M^{-1/2} L M^{-1/2}.
        let mut b = SymMat::zeros(n);
        for i in 0..n {
            for j in 0..n {
                let v = lap.get(i, j) / (masses[i] * masses[j]).sqrt();
                b.add(i, j, v);
            }
        }
        let (_vals, vecs) = jacobi_eigen(&b, s.eigen_sweeps as usize);
        let init = |col: usize| -> Vec<f64> { (0..n).map(|r| vecs[r * n + col] / masses[r].sqrt()).collect() };

        // **Sellán's Algorithm 1, with the conic sub-problem solved by ADMM.** For each mode the
        // sphere constraint `φᵀMφ = 1` is linearised about the current iterate `c` (their Eq. 16:
        // `cᵀMφ = 1`), the orthogonality to earlier modes is kept as `U_jᵀMφ = 0`, and the resulting
        // convex problem is minimised; then `c ← φ/√(φᵀMφ)` and the linearisation is repeated. The
        // linear constraints enter the ADMM φ-update as a quadratic penalty of weight `constraint`,
        // so the system stays SPD and one Cholesky serves every inner step of an outer round.
        let (strain, rho, omega) = (s.strain.max(0.0) as f64, s.rho.max(1.0e-6) as f64, s.omega.max(0.0) as f64);
        let lambda = s.constraint.max(1.0) as f64;

        // Impact filter (M + τL), factored once.
        let mut heat = SymMat::zeros(n);
        for i in 0..n {
            for j in 0..n {
                heat.add(i, j, s.tau.max(0.0) as f64 * lap.get(i, j));
            }
            heat.add(i, i, masses[i]);
        }
        let hfac = Cholesky::new(&heat).ok_or(BakeError::Singular)?;

        let ones = vec![1.0f64; n];
        let mut basis: Vec<Vec<f64>> = Vec::with_capacity(k + 1);
        basis.push(m_normalise(&ones, &masses));

        let mut modes: Vec<(Vec<f64>, f64)> = Vec::with_capacity(k);
        for i in 0..k {
            let mut c = init(i + 1);
            project(&mut c, &basis, &masses);
            let mut phi = c.clone();
            let mut z: Vec<f64> = faces.iter().map(|&(a, b, _)| phi[a] - phi[b]).collect();
            let mut y = vec![0.0f64; faces.len()];
            let mut rhs = vec![0.0f64; n];
            for _ in 0..s.outer {
                // K = strain·L + ρ·DᵀD + λ·M(ccᵀ + Σ_j u_j u_jᵀ)M, for this linearisation.
                let mut kmat = SymMat::zeros(n);
                for r in 0..n {
                    for q in 0..n {
                        let mut v = strain * lap.get(r, q) + rho * inc.get(r, q);
                        let mut outer = c[r] * c[q];
                        for u in &basis {
                            outer += u[r] * u[q];
                        }
                        v += lambda * masses[r] * masses[q] * outer;
                        kmat.add(r, q, v);
                    }
                }
                let Some(kfac) = Cholesky::new(&kmat) else { return Err(BakeError::Singular) };
                for _ in 0..s.iterations {
                    // φ-update: K φ = ρ Dᵀ(z − y) + λ M c   (the `1` on the right of cᵀMφ = 1).
                    for (r, (cc, m)) in rhs.iter_mut().zip(c.iter().zip(masses.iter())) {
                        *r = lambda * m * cc;
                    }
                    for (f, &(a, b, _)) in faces.iter().enumerate() {
                        let v = rho * (z[f] - y[f]);
                        rhs[a] += v;
                        rhs[b] -= v;
                    }
                    kfac.solve(&mut rhs);
                    phi.copy_from_slice(&rhs);
                    // z-update: soft threshold of the face jumps at ω√area/ρ; then the dual.
                    for (f, &(a, b, area)) in faces.iter().enumerate() {
                        let d = phi[a] - phi[b] + y[f];
                        let t = omega * area.sqrt() / rho;
                        z[f] = if d > t {
                            d - t
                        } else if d < -t {
                            d + t
                        } else {
                            0.0
                        };
                        y[f] += phi[a] - phi[b] - z[f];
                    }
                }
                // c ← φ / √(φᵀMφ), exactly on the sphere and exactly orthogonal to the earlier modes.
                c.copy_from_slice(&phi);
                project(&mut c, &basis, &masses);
            }
            let phi = c;
            let energy: f64 = faces.iter().map(|&(a, b, area)| area.sqrt() * (phi[a] - phi[b]).abs()).sum();
            basis.push(phi.clone());
            modes.push((phi, energy));
        }

        // Energy ascending, index as the tiebreak — a total order.
        // SORT-OK: `(energy, index)` over a Vec the bake built in a fixed order; the index breaks
        // every tie, so the order is total and no query is involved.
        let mut order: Vec<usize> = (0..modes.len()).collect();
        order.sort_by(|&a, &b| {
            modes[a].1.partial_cmp(&modes[b].1).unwrap_or(core::cmp::Ordering::Equal).then(a.cmp(&b))
        });

        let mut out = Vec::with_capacity(modes.len());
        for &ix in &order {
            let (phi, energy) = &modes[ix];
            // r = (M + τL)⁻¹ M φ ; A[p] = r[p] · mass[p].
            let mut r: Vec<f64> = phi.iter().zip(masses.iter()).map(|(p, m)| p * m).collect();
            hfac.solve(&mut r);
            let impact_row: Vec<f32> = r.iter().zip(masses.iter()).map(|(r, m)| (r * m) as f32).collect();
            out.push(Mode { phi: phi.iter().map(|v| *v as f32).collect(), energy: *energy as f32, impact_row });
        }
        Ok(ModeSet { cells: n, modes: out, faces: faces.iter().map(|&(a, b, _)| (a, b)).collect() })
    }

    /// The largest jump of mode `i` across any face, and that face's index — where the mode breaks.
    pub fn strongest_fault(&self, i: usize) -> Option<(usize, f32)> {
        let m = self.modes.get(i)?;
        let mut best: Option<(usize, f32)> = None;
        for (f, &(a, b)) in self.faces.iter().enumerate() {
            let jump = (m.phi.get(a)? - m.phi.get(b)?).abs();
            match best {
                Some((_, j)) if j >= jump => {}
                _ => best = Some((f, jump)),
            }
        }
        best
    }
}

/// `v / √(vᵀ M v)`.
fn m_normalise(v: &[f64], masses: &[f64]) -> Vec<f64> {
    let norm: f64 = v.iter().zip(masses).map(|(x, m)| x * x * m).sum::<f64>().sqrt();
    if norm > 0.0 && norm.is_finite() { v.iter().map(|x| x / norm).collect() } else { v.to_vec() }
}

/// Remove the `M`-components of `phi` along every vector in `basis`, then `M`-normalise.
fn project(phi: &mut [f64], basis: &[Vec<f64>], masses: &[f64]) {
    for b in basis {
        let dot: f64 = phi.iter().zip(b).zip(masses).map(|((p, q), m)| p * q * m).sum();
        for (p, q) in phi.iter_mut().zip(b) {
            *p -= dot * q;
        }
    }
    let n = m_normalise(phi, masses);
    phi.copy_from_slice(&n);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The first mode breaks at the neck.** A bar whose one thin face is the obvious weakness
    /// must have a first mode that jumps there and is flat everywhere else — the property geometric
    /// prefracture is blind to and this whole method exists to find.
    #[test]
    fn the_first_mode_opens_the_neck() {
        let g = CellGraph::bar(10, 4, 0.05);
        let set = ModeSet::bake(&g, &ModeSettings::default()).expect("bar bakes");
        let (face, jump) = set.strongest_fault(0).expect("a fault");
        assert_eq!(face, 4, "the strongest fault of mode 0 is not the neck: {:?}", set.modes[0].phi);
        // Every other face is nearly continuous.
        let phi = &set.modes[0].phi;
        for f in 0..9 {
            if f == 4 {
                continue;
            }
            let j = (phi[f] - phi[f + 1]).abs();
            assert!(j < jump * 0.05, "face {f} jumps {j} against the neck's {jump}");
        }
    }

    /// Energies come out ascending, which is what "the k lowest-energy modes" means.
    #[test]
    fn energies_are_ascending_and_modes_orthonormal() {
        let g = CellGraph::bar(12, 3, 0.2);
        let set = ModeSet::bake(&g, &ModeSettings { k: 5, ..Default::default() }).expect("bakes");
        assert_eq!(set.modes.len(), 5);
        for w in set.modes.windows(2) {
            assert!(w[0].energy <= w[1].energy, "energies not ascending");
        }
        for i in 0..5 {
            for j in 0..5 {
                let dot: f32 = (0..12).map(|c| set.modes[i].phi[c] * set.modes[j].phi[c] * g.masses[c]).sum();
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1.0e-3, "modes {i},{j} M-dot {dot}");
            }
        }
    }

    /// **Bit-identical across bakes.** Two bakes of one graph agree to the bit, and a graph with
    /// the neck moved bakes to something else.
    #[test]
    fn a_bake_is_a_pure_function_of_its_inputs() {
        let g = CellGraph::bar(9, 2, 0.1);
        let a = ModeSet::bake(&g, &ModeSettings::default()).expect("bakes");
        let b = ModeSet::bake(&g, &ModeSettings::default()).expect("bakes");
        assert_eq!(a, b);
        let c = ModeSet::bake(&CellGraph::bar(9, 5, 0.1), &ModeSettings::default()).expect("bakes");
        assert_ne!(a, c);
    }

    /// **Frozen.** The necked bar's mode set, as bits. A moved value here is a moved solver, and
    /// that is a deliberate re-bless with the reason in the same commit.
    #[test]
    fn the_bar_modes_are_frozen() {
        let set = ModeSet::bake(&CellGraph::bar(10, 4, 0.05), &ModeSettings { k: 3, ..Default::default() }).expect("bakes");
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |x: f32| {
            for byte in x.to_bits().to_le_bytes() {
                h ^= byte as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        for m in &set.modes {
            m.phi.iter().for_each(|v| eat(*v));
            m.impact_row.iter().for_each(|v| eat(*v));
            eat(m.energy);
        }
        println!("{h:016x}");
        assert_eq!(h, 0xe3b9_cbc5_515d_2aea);
    }

    #[test]
    fn a_bad_graph_is_refused() {
        let mut g = CellGraph::bar(4, 1, 1.0);
        g.masses[2] = 0.0;
        assert_eq!(ModeSet::bake(&g, &ModeSettings::default()), Err(BakeError::Graph(GraphError::BadMass(2))));
        assert_eq!(
            ModeSet::bake(&CellGraph::default(), &ModeSettings::default()),
            Err(BakeError::Graph(GraphError::Empty))
        );
    }
}
