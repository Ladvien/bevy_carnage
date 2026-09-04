//! **Fracture modes over a bond graph** — `bevy_fracture_modes` composed onto this crate's own
//! decomposition.
//!
//! A [`BondGraph`] is already a cell graph: fragments with a volume and a centre, joined by shared
//! faces with an area. Sellán et al. (`doi:10.1145/3549540`) compute, once, the sparse modes such
//! a graph *wants* to come apart along, and at impact project the blow onto them — so a body struck
//! on the shoulder gives at the neck and the wrist, where it is thin, rather than in a Voronoi
//! pattern around the impact. This module is the two conversions: the graph out, the broken bonds
//! back in. The mathematics lives in the leaf crate and is not repeated here.
//!
//! The result is a [`BondSet`]'s worth of ids, which is exactly what [`crate::severance`]'s region
//! queries produce, so a modal break and a blade sweep feed the same `islands` and spawn the same
//! way. Morphology — *how* each opened face is cut — stays with [`crate::FaultPolicy`]: the modes
//! decide the partition, the loading mode decides the faces.

use crate::fracture_modes::{BakeError, CellGraph, Impact, ModeSet, ModeSettings};

use crate::bond::{BondGraph, BondId};
use crate::proxy::ProxyCell;
use crate::tree::FragmentId;

/// **A baked mode set, with the fragment ids it was baked over.**
///
/// The leaf crate indexes cells `0..n`; this remembers which fragment each index is, and which
/// bond each face is, so a partition comes back as [`BondId`]s.
#[derive(Clone, Debug, PartialEq)]
pub struct ModalSet {
    /// The modes.
    pub set: ModeSet,
    /// Cell index → fragment id, ascending.
    pub members: Vec<FragmentId>,
    /// Face index → bond id, in the graph's bond order.
    pub bonds: Vec<BondId>,
}

/// **The cell graph of a frontier**: one node per member with its cell's volume and centre, one
/// face per bond with its area. `cell_of` resolves a fragment id to its solid; a member it cannot
/// resolve is left out, and a bond touching one is left out with it.
pub fn cell_graph<'a>(
    graph: &BondGraph,
    cell_of: impl Fn(FragmentId) -> Option<&'a ProxyCell>,
) -> (CellGraph, Vec<FragmentId>, Vec<BondId>) {
    let mut members: Vec<FragmentId> = Vec::with_capacity(graph.members().len());
    let mut masses = Vec::with_capacity(graph.members().len());
    let mut centers = Vec::with_capacity(graph.members().len());
    for &id in graph.members() {
        let Some(cell) = cell_of(id) else { continue };
        let volume = cell.volume();
        // A degenerate cell would make the mass matrix singular; it is dropped rather than patched.
        if !(volume.is_finite() && volume > 0.0) {
            continue;
        }
        members.push(id);
        masses.push(volume);
        centers.push(cell.center());
    }
    let mut g = CellGraph::new(masses, centers);
    let mut bond_ids = Vec::with_capacity(graph.bonds().len());
    for (i, bond) in graph.bonds().iter().enumerate() {
        let (Ok(a), Ok(b)) = (members.binary_search(&bond.a), members.binary_search(&bond.b)) else {
            continue;
        };
        g.bond(a, b, bond.area, bond.centroid, bond.normal);
        bond_ids.push(BondId(i as u32));
    }
    (g, members, bond_ids)
}

/// **Bake the modes of a frontier.** Pure, and the error is the leaf crate's.
pub fn bake_modes<'a>(
    graph: &BondGraph,
    cell_of: impl Fn(FragmentId) -> Option<&'a ProxyCell>,
    settings: &ModeSettings,
) -> Result<ModalSet, BakeError> {
    let (g, members, bonds) = cell_graph(graph, cell_of);
    let set = ModeSet::bake(&g, settings)?;
    Ok(ModalSet { set, members, bonds })
}

impl ModalSet {
    /// **The bonds a blow of `magnitude` at fragment `at` breaks.** Sorted, so two calls with one
    /// input sever in one order. Empty when `at` is not on this frontier.
    pub fn break_at(&self, at: FragmentId, magnitude: f32) -> Vec<BondId> {
        let Ok(cell) = self.members.binary_search(&at) else {
            return Vec::new();
        };
        let p = self.set.partition(&Impact { cell, magnitude });
        let mut out: Vec<BondId> = p.broken.iter().filter_map(|&f| self.bonds.get(f).copied()).collect();
        out.sort_unstable();
        out
    }

    /// **The smallest blow at `at` that leaves at least `pieces` pieces**, or `None` if none does
    /// within eight decades of impulse. A geometric sweep, so it is a pure function of the modes:
    /// a demo that wants "the arm comes off" asks for two pieces rather than tuning a constant
    /// against a body it cannot see.
    pub fn impulse_for(&self, at: FragmentId, pieces: usize) -> Option<f32> {
        let Ok(cell) = self.members.binary_search(&at) else {
            return None;
        };
        (0..400).map(|i| 1.0e-4 * 1.05f32.powi(i)).find(|&magnitude| {
            self.set.partition(&Impact { cell, magnitude }).fragment_count() >= pieces
        })
    }

    /// How many pieces a blow at `at` would leave, without severing anything.
    pub fn pieces_after(&self, at: FragmentId, magnitude: f32) -> usize {
        match self.members.binary_search(&at) {
            Ok(cell) => self.set.partition(&Impact { cell, magnitude }).fragment_count(),
            Err(_) => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bond::BondSet;
    use bevy::math::Vec3;

    /// A row of five boxes, the middle joint narrow: the modes find the narrow joint, and the ids
    /// that come back are the graph's own.
    fn necked_row() -> (BondGraph, Vec<ProxyCell>) {
        let cells: Vec<ProxyCell> = (0..5)
            .map(|i| {
                // Cells 0–1 and 3–4 are full width; cell 2 is a thin neck between them.
                let half = if i == 2 { Vec3::new(0.5, 0.15, 0.15) } else { Vec3::new(0.5, 0.5, 0.5) };
                ProxyCell::from_box(Vec3::new(i as f32, 0.0, 0.0), half)
            })
            .collect();
        let members: Vec<(FragmentId, &ProxyCell)> =
            cells.iter().enumerate().map(|(i, c)| (FragmentId(i as u32), c)).collect();
        let graph = BondGraph::of(&members, 5);
        (graph, cells)
    }

    #[test]
    fn a_blow_on_the_row_gives_at_the_neck() {
        let (graph, cells) = necked_row();
        assert_eq!(graph.len(), 4, "four joints in a row of five");
        let modal = bake_modes(&graph, |id| cells.get(id.index()), &ModeSettings { k: 2, ..Default::default() })
            .expect("bakes");
        assert_eq!(modal.members.len(), 5);
        let mut first = None;
        for i in 0..400 {
            let magnitude = 1.0e-4 * 1.05f32.powi(i);
            let broken = modal.break_at(FragmentId(0), magnitude);
            if !broken.is_empty() {
                first = Some(broken);
                break;
            }
        }
        let broken = first.expect("some blow breaks the row");
        // The narrow joints are the two bonds touching cell 2; whichever gives first is one of them.
        for id in &broken {
            let bond = graph.bond(*id).expect("a real bond");
            assert!(bond.a == FragmentId(2) || bond.b == FragmentId(2), "gave at {bond:?}, not at the neck");
        }
        let mut set = BondSet::new(&graph);
        assert_eq!(set.sever_all(&broken), broken.len());
        assert!(graph.islands(graph.members(), &set).len() >= 2);
        assert!(modal.break_at(FragmentId(99), 1.0).is_empty(), "an id off the frontier breaks nothing");
    }
}
