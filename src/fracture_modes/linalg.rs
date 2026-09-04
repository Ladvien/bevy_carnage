//! **Dense symmetric linear algebra, in `f64`, with fixed iteration counts.**
//!
//! A cell graph has tens to a few hundred nodes, so dense is the right size class and nothing here
//! needs a library. Every routine runs a fixed schedule — a Cholesky is a fixed sweep, the Jacobi
//! eigensolver runs a fixed number of sweeps — so the same input produces the same bits on every
//! machine, which is what lets a mode set be a golden.

/// A dense symmetric `n × n` matrix, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct SymMat {
    pub n: usize,
    pub a: Vec<f64>,
}

impl SymMat {
    pub fn zeros(n: usize) -> Self {
        Self { n, a: vec![0.0; n * n] }
    }

    #[inline]
    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.a[i * self.n + j]
    }

    #[inline]
    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        self.a[i * self.n + j] += v;
    }

    /// `y = A x`.
    #[cfg(test)]
    pub fn mul(&self, x: &[f64], y: &mut [f64]) {
        for i in 0..self.n {
            let row = &self.a[i * self.n..(i + 1) * self.n];
            let mut s = 0.0;
            for (a, b) in row.iter().zip(x.iter()) {
                s += a * b;
            }
            y[i] = s;
        }
    }
}

/// A Cholesky factor `L` with `A = L Lᵀ`, lower triangle stored row-major.
#[derive(Clone, Debug)]
pub struct Cholesky {
    n: usize,
    l: Vec<f64>,
}

impl Cholesky {
    /// Factor an SPD matrix. `None` if a pivot is not positive — the matrix was not SPD, which for
    /// this crate means a graph with a zero-mass cell or a non-finite face area.
    pub fn new(a: &SymMat) -> Option<Self> {
        let n = a.n;
        let mut l = vec![0.0f64; n * n];
        for j in 0..n {
            let mut d = a.get(j, j);
            for k in 0..j {
                d -= l[j * n + k] * l[j * n + k];
            }
            if !(d > 0.0) || !d.is_finite() {
                return None;
            }
            let dj = d.sqrt();
            l[j * n + j] = dj;
            for i in (j + 1)..n {
                let mut s = a.get(i, j);
                for k in 0..j {
                    s -= l[i * n + k] * l[j * n + k];
                }
                l[i * n + j] = s / dj;
            }
        }
        Some(Self { n, l })
    }

    /// Solve `A x = b` in place.
    pub fn solve(&self, b: &mut [f64]) {
        let n = self.n;
        // Forward: L y = b.
        for i in 0..n {
            let mut s = b[i];
            for k in 0..i {
                s -= self.l[i * n + k] * b[k];
            }
            b[i] = s / self.l[i * n + i];
        }
        // Back: Lᵀ x = y.
        for i in (0..n).rev() {
            let mut s = b[i];
            for k in (i + 1)..n {
                s -= self.l[k * n + i] * b[k];
            }
            b[i] = s / self.l[i * n + i];
        }
    }
}

/// **Eigen-decomposition of a symmetric matrix by cyclic Jacobi rotations**, a fixed number of
/// sweeps. Returns `(eigenvalues, eigenvectors as columns)` sorted ascending.
///
/// Jacobi is chosen over anything faster because it is the one dense symmetric solver whose every
/// operation is a fixed, data-independent schedule of rotations — no pivoting, no convergence test
/// that could take a different branch on a different machine. Ten sweeps are past machine precision
/// for the sizes this crate meets.
pub fn jacobi_eigen(a: &SymMat, sweeps: usize) -> (Vec<f64>, Vec<f64>) {
    let n = a.n;
    let mut m = a.a.clone();
    let mut v = vec![0.0f64; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _ in 0..sweeps {
        for p in 0..n {
            for q in (p + 1)..n {
                let apq = m[p * n + q];
                if apq.abs() < 1.0e-300 {
                    continue;
                }
                let app = m[p * n + p];
                let aqq = m[q * n + q];
                let theta = (aqq - app) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let mkp = m[k * n + p];
                    let mkq = m[k * n + q];
                    m[k * n + p] = c * mkp - s * mkq;
                    m[k * n + q] = s * mkp + c * mkq;
                }
                for k in 0..n {
                    let mpk = m[p * n + k];
                    let mqk = m[q * n + k];
                    m[p * n + k] = c * mpk - s * mqk;
                    m[q * n + k] = s * mpk + c * mqk;
                }
                for k in 0..n {
                    let vkp = v[k * n + p];
                    let vkq = v[k * n + q];
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    // A total order: eigenvalue, then index — two equal eigenvalues keep column order.
    order.sort_by(|&i, &j| {
        m[i * n + i].partial_cmp(&m[j * n + j]).unwrap_or(core::cmp::Ordering::Equal).then(i.cmp(&j))
    });
    let vals: Vec<f64> = order.iter().map(|&i| m[i * n + i]).collect();
    let mut vecs = vec![0.0f64; n * n];
    for (col, &src) in order.iter().enumerate() {
        for k in 0..n {
            vecs[k * n + col] = v[k * n + src];
        }
    }
    (vals, vecs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cholesky_solves_a_small_spd_system() {
        let mut a = SymMat::zeros(3);
        let vals = [[4.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 2.0]];
        for i in 0..3 {
            for j in 0..3 {
                a.add(i, j, vals[i][j]);
            }
        }
        let ch = Cholesky::new(&a).expect("spd");
        let mut b = vec![1.0, 2.0, 3.0];
        ch.solve(&mut b);
        let mut y = vec![0.0; 3];
        a.mul(&b, &mut y);
        for (got, want) in y.iter().zip([1.0, 2.0, 3.0]) {
            assert!((got - want).abs() < 1.0e-9, "{y:?}");
        }
        let mut bad = SymMat::zeros(2);
        bad.add(0, 0, 1.0);
        bad.add(1, 1, -1.0);
        assert!(Cholesky::new(&bad).is_none());
    }

    #[test]
    fn jacobi_finds_the_spectrum_of_a_path_laplacian() {
        // Path graph on 4 nodes: eigenvalues 0, 2−√2, 2, 2+√2.
        let mut a = SymMat::zeros(4);
        for i in 0..3 {
            a.add(i, i, 1.0);
            a.add(i + 1, i + 1, 1.0);
            a.add(i, i + 1, -1.0);
            a.add(i + 1, i, -1.0);
        }
        let (vals, vecs) = jacobi_eigen(&a, 10);
        let want = [0.0, 2.0 - 2f64.sqrt(), 2.0, 2.0 + 2f64.sqrt()];
        for (g, w) in vals.iter().zip(want) {
            assert!((g - w).abs() < 1.0e-9, "{vals:?}");
        }
        // Columns are orthonormal.
        for i in 0..4 {
            for j in 0..4 {
                let dot: f64 = (0..4).map(|k| vecs[k * 4 + i] * vecs[k * 4 + j]).sum();
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1.0e-9);
            }
        }
    }
}
