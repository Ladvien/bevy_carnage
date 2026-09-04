//! **The cell graph a mode set is computed on.**
//!
//! A convex decomposition is a set of cells and the faces they share. For fracture modes that is
//! all that matters: each cell is a node with a mass, each shared face an edge with an area, and
//! the discontinuity energy of a mode is a sum over edges of the face area times the jump across
//! it. The cells' shapes never enter — which is what makes this crate composable with any proxy.

use bevy::math::Vec3;

/// A shared face between two cells: the fault patch a fracture can open along.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    /// First cell.
    pub a: usize,
    /// Second cell.
    pub b: usize,
    /// Face area, in the caller's units squared. The weight of the fault.
    pub area: f32,
    /// Face centroid, for a consumer placing a wound.
    pub centroid: Vec3,
    /// Face normal, pointing from `a` into `b`.
    pub normal: Vec3,
}

/// **Cells and the faces between them.**
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellGraph {
    /// Per-cell mass (volume). Must be positive and finite.
    pub masses: Vec<f32>,
    /// Per-cell centre, for the impact filter and for a consumer.
    pub centers: Vec<Vec3>,
    /// Every shared face.
    pub faces: Vec<Face>,
}

/// Why a graph was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// No cells at all.
    Empty,
    /// `masses` and `centers` disagree in length.
    Mismatched,
    /// A cell has a mass that is not positive and finite.
    BadMass(usize),
    /// A face names a cell that does not exist, or names the same cell twice.
    BadFace(usize),
    /// A face has an area that is not positive and finite.
    BadArea(usize),
}

impl CellGraph {
    /// A graph with cells and no faces yet.
    pub fn new(masses: Vec<f32>, centers: Vec<Vec3>) -> Self {
        Self { masses, centers, faces: Vec::new() }
    }

    /// Add a shared face.
    pub fn bond(&mut self, a: usize, b: usize, area: f32, centroid: Vec3, normal: Vec3) {
        self.faces.push(Face { a, b, area, centroid, normal });
    }

    /// Number of cells.
    pub fn len(&self) -> usize {
        self.masses.len()
    }

    /// True when there are no cells.
    pub fn is_empty(&self) -> bool {
        self.masses.is_empty()
    }

    /// **Refuse anything the solver cannot factor**, naming what was wrong.
    pub fn validate(&self) -> Result<(), GraphError> {
        let n = self.masses.len();
        if n == 0 {
            return Err(GraphError::Empty);
        }
        if self.centers.len() != n {
            return Err(GraphError::Mismatched);
        }
        for (i, m) in self.masses.iter().enumerate() {
            if !(m.is_finite() && *m > 0.0) {
                return Err(GraphError::BadMass(i));
            }
        }
        for (i, f) in self.faces.iter().enumerate() {
            if f.a >= n || f.b >= n || f.a == f.b {
                return Err(GraphError::BadFace(i));
            }
            if !(f.area.is_finite() && f.area > 0.0) {
                return Err(GraphError::BadArea(i));
            }
        }
        Ok(())
    }

    /// **A bar of `n` unit cells in a row**, every shared face of area `1.0` except the one after
    /// cell `neck`, which gets `neck_area`. The graph every test and the terminal example reason
    /// about: a shape with exactly one obvious weakness.
    pub fn bar(n: usize, neck: usize, neck_area: f32) -> Self {
        let masses = vec![1.0; n];
        let centers = (0..n).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
        let mut g = Self::new(masses, centers);
        for i in 0..n.saturating_sub(1) {
            let area = if i == neck { neck_area } else { 1.0 };
            g.bond(i, i + 1, area, Vec3::new(i as f32 + 0.5, 0.0, 0.0), Vec3::X);
        }
        g
    }
}
