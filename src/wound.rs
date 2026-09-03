//! **The wound.** Where a subject came open, how wide, how badly, and which way it faces.
//!
//! The crate could already say where a shared face was ([`Bond`]), how hard a blow reached it
//! ([`Reach`]), and where a channel left the subject ([`EjectaChunk`]). What it could not say is
//! "there is a wound here" — a single value with a point, a direction, an area and a severity, which
//! is what every downstream carnage decision actually needs. Blood, spatter, stains, pulse and hit
//! feel all read a [`Wound`] and nothing else, so none of them has to know whether the subject was
//! cut in half or shot through.
//!
//! # Two sources, one type
//!
//! A wound comes from exactly two places, and they are geometrically different things that read the
//! same:
//!
//! - **A severance.** Two fragments stopped sharing a face. The bond *is* the wound surface: its
//!   centroid, its normal and its area are the wound's, with no derivation at all.
//! - **A channel.** A bore removed material, so the subject has an interior wall that is open to the
//!   air. That wall is a set of cut faces on a convex cell, which [`cap_faces`] extracts.
//!
//! [`crate::proxy::ProxyCell::face_is_cut`] answers `true` for a fracture cut face **and** a bore's
//! channel wall — see [`FaceKind`](crate::proxy::FaceKind), where that decision is recorded — so cut-
//! face extraction picks up bullet channels for free. That is the whole reason a bullet hole bleeds in
//! this crate without a second code path: the geometry already said the interior was exposed.
//!
//! # Determinism
//!
//! Everything here is a pure function of baked geometry. No clock, no `Entity`, no handle, no
//! accumulator. [`wounds_from_reach`] and [`wounds_from_bonds`] return their wounds sorted by
//! [`BondId`] through [`crate::order::sort_total_by_key_at`], so the order is a function of the graph
//! rather than of how the caller happened to iterate — and `BondId` is unique per bond, so the key is
//! total and the check cannot fire on well-formed input.

use bevy::log::warn;
use bevy::math::Vec3;

use crate::bake::EjectaChunk;
use crate::bond::{Bond, BondGraph, BondId};
use crate::order::sort_total_by_key_at;
use crate::proxy::ProxyCell;
use crate::severance::Reach;
use crate::soup::MIN_CROSS2;

/// **`WoundKind` lives in `bloodstain` now**, because its discriminant is mixed into every blood
/// seed and the seed function moved with the blood model. One enum, one home: a copy here would be a
/// second numbering of the same fact, and the two would disagree the first time either gained a
/// variant. Re-exported from `lib.rs` under the name it always had.
pub use bloodstain::WoundKind;

/// A wound surface in subject-local space. Deterministic: derived only from baked geometry.
///
/// **A value, not an entity.** It has no lifetime, no handle and no id, so a caller can compute one,
/// hash it, send it, store it in a replay, or throw it away. That is what lets the same type serve a
/// headless simulation and a particle burst.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Wound {
    /// Where it is, subject-local — the centre of the opened surface.
    pub at: Vec3,
    /// Which way it faces, unit. Blood leaves along this, and the shake kicks along it.
    pub normal: Vec3,
    /// How much surface came open, subject-local units squared. Drives droplet count.
    pub area: f32,
    /// How badly, in `[0, 1]`. `1.0` is fully open; a pulse's taper scales this and nothing else.
    pub severity: f32,
    /// Which of the two things happened — see the module docs.
    pub kind: WoundKind,
}


/// One cut face of a convex cell — the wound surface that travels with a fragment or a plug.
///
/// Separate from [`Wound`] because a cell has several of these and a wound is one: a caller that
/// wants "the wound on this chunk" takes [`largest_cap`], and one that wants "how much of this chunk
/// is raw" sums [`cap_faces`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapFace {
    /// Area-weighted centre of the face, subject-local.
    pub centroid: Vec3,
    /// Outward unit normal, from the face's own winding.
    pub normal: Vec3,
    /// Face area, subject-local units squared.
    pub area: f32,
}

/// The wounds a blow opened: every bond it reached at or above `threshold`.
///
/// `threshold` is the caller's "this gives way" line — the same number it would pass to
/// [`Reach::above`], and deliberately the caller's rather than a dial here, because how much damage
/// severs a limb is a game rule.
///
/// Sorted by [`BondId`], so the result is a function of the graph.
pub fn wounds_from_reach(graph: &BondGraph, reach: &Reach, threshold: f32) -> Vec<Wound> {
    let mut out: Vec<(BondId, Wound)> = reach
        .iter()
        .filter(|(_, severity)| *severity >= threshold)
        .filter_map(|(id, severity)| {
            let bond = graph.bond(id)?;
            Some((id, wound_of(bond, severity.clamp(0.0, 1.0))))
        })
        .collect();
    sort_total_by_key_at("wound::wounds_from_reach", &mut out, |(id, _)| *id);
    out.into_iter().map(|(_, w)| w).collect()
}

/// The wounds a set of already-broken bonds represents, at full severity.
///
/// For the caller that has severed bonds itself — through a [`BondSet`](crate::BondSet) — and wants
/// the wounds those severances left. A bond that is not in this graph is skipped loudly: it means two
/// frontiers' ids were mixed, which would otherwise resolve to a *different* bond and put a wound in
/// the wrong place.
pub fn wounds_from_bonds(graph: &BondGraph, broken: &[BondId]) -> Vec<Wound> {
    let mut out: Vec<(BondId, Wound)> = broken
        .iter()
        .filter_map(|id| match graph.bond(*id) {
            Some(bond) => Some((*id, wound_of(bond, 1.0))),
            None => {
                warn!("carnage: bond {id:?} is not in this graph — mixing graphs from two frontiers");
                None
            }
        })
        .collect();
    sort_total_by_key_at("wound::wounds_from_bonds", &mut out, |(id, _)| *id);
    out.into_iter().map(|(_, w)| w).collect()
}

/// The bond, as a wound. One place, so the two entry points cannot disagree about the mapping.
fn wound_of(bond: &Bond, severity: f32) -> Wound {
    Wound {
        at: bond.centroid,
        normal: bond.normal,
        area: bond.area,
        severity,
        kind: WoundKind::Severance,
    }
}

/// Every raw-interior face of a convex cell: fracture cut faces **and** bore channel walls.
///
/// Rings are converted with the Newell formula over the closed ring, which is exact for a planar
/// convex polygon and needs no triangulation choice — and the faces here are guaranteed planar and
/// convex, because a plane through a convex polyhedron produces exactly that.
///
/// **A grazed corner is not an error.** A ring shorter than a triangle, an index outside the cell's
/// points, or a face whose Newell cross product is below the crate's own zero-area floor is skipped
/// silently: those arise from a cut that clipped a vertex, they are geometrically correct, and this
/// runs per fragment per blow. Warning on them would print a page per gib.
pub fn cap_faces(cell: &ProxyCell) -> Vec<CapFace> {
    let points = cell.points();
    let mut out = Vec::new();
    for (fi, ring) in cell.faces().enumerate() {
        if !cell.face_is_cut(fi) || ring.len() < 3 {
            continue;
        }
        if ring.iter().any(|&i| i as usize >= points.len()) {
            continue;
        }
        // The ring's own vertex mean, as the pivot for both the Newell sum and the area-weighted
        // centroid. Any interior point works for Newell; using the mean keeps the fan triangles
        // well-conditioned, which matters for the weighting rather than for the normal.
        let pivot =
            ring.iter().map(|&i| points[i as usize]).sum::<Vec3>() / ring.len() as f32;

        let mut n = Vec3::ZERO;
        let mut weighted = Vec3::ZERO;
        let mut area2 = 0.0f32;
        for k in 0..ring.len() {
            let a = points[ring[k] as usize];
            let b = points[ring[(k + 1) % ring.len()] as usize];
            let cross = (a - pivot).cross(b - pivot);
            n += cross;
            // The fan triangle's own doubled area, as the weight for its own centroid. Summing
            // `centroid * weight` and dividing by the total is the area-weighted centre; a plain
            // vertex mean would drift toward whichever side of the face has more vertices.
            let w = cross.length();
            weighted += (pivot + a + b) / 3.0 * w;
            area2 += w;
        }
        let n2 = n.length_squared();
        if n2 < MIN_CROSS2 || area2 <= 0.0 {
            continue;
        }
        let len = n2.sqrt();
        out.push(CapFace {
            centroid: weighted / area2,
            normal: n / len,
            area: 0.5 * len,
        });
    }
    out
}

/// The widest raw-interior face of a cell — "the wound on this chunk".
///
/// `None` for a cell with none, which is the honest answer for an untouched proxy cell: it has no
/// exposed interior, so it has no wound. Ties keep the first face in ring order, which is a function
/// of the cell's own construction rather than of iteration.
pub fn largest_cap(cell: &ProxyCell) -> Option<CapFace> {
    cap_faces(cell)
        .into_iter()
        // Strictly greater, scanned in face order, so a tie keeps the first — the same rule
        // `Reach::strongest` and the weak-axis scan use, and for the same reason.
        .reduce(|best, f| if f.area > best.area { f } else { best })
}

/// **The wound a channel is**, from the plug's own geometry.
///
/// `at` is where the channel crossed the skin and `normal` is the channel's axis, so blood leaves the
/// hole along the direction the shot was travelling. The area is the plug's whole **channel wall** —
/// the sum of its raw-interior faces — because that is the surface the subject now has open. Not the
/// entry disc, which is only the hole a decal would cover.
///
/// **This takes the three geometric facts rather than a struct**, because the crate hands plugs back
/// in two shapes: [`crate::Ejecta`] from the pure [`crate::fracture_mesh`] path carries owned meshes,
/// and [`EjectaChunk`] from the ECS bake carries handles. They differ in nothing this function reads.
/// One implementation, so the pure path and the ECS path cannot disagree about how wide a bullet hole
/// bleeds — and so a caller holding either shape has a real answer instead of an approximation.
pub fn wound_of_channel(cell: &ProxyCell, exit: Vec3, direction: Vec3) -> Wound {
    Wound {
        at: exit,
        normal: direction,
        area: cap_faces(cell).iter().map(|f| f.area).sum(),
        severity: 1.0,
        kind: WoundKind::Channel,
    }
}

/// The wound a baked plug leaves behind — [`wound_of_channel`] for an [`EjectaChunk`].
pub fn wound_from_ejecta(chunk: &EjectaChunk) -> Wound {
    wound_of_channel(&chunk.cell, chunk.exit, chunk.direction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bond::BondGraph;
    use crate::soup::fracture;
    use crate::{Bore, CutSettings};

    fn unit_cube_cells() -> Vec<ProxyCell> {
        vec![ProxyCell::from_box(Vec3::ZERO, Vec3::splat(0.5))]
    }

    /// Fracture the unit cube and build the finest frontier's bond graph — the same three lines every
    /// bond test in this crate writes, so they are written once here.
    fn baked_graph(cut: CutSettings) -> BondGraph {
        let (pieces, tree, _, _) = fracture(crate::soup::Soup::default(), &unit_cube_cells(), &cut);
        let leaves = tree.leaves();
        let members: Vec<_> =
            leaves.iter().filter_map(|&id| pieces.get(id.index()).map(|p| (id, &p.cell))).collect();
        BondGraph::of(&members, tree.len())
    }

    /// **An untouched proxy cell has no wound, and that is not a degenerate case to paper over.**
    ///
    /// Every face of `from_box` is `Supplied` — the caller's own hull — so nothing about it is open
    /// to the air. A `cap_faces` that returned the hull faces here would have every intact body part
    /// bleeding from its skin.
    #[test]
    fn an_uncut_cell_has_no_wound() {
        let cell = ProxyCell::from_box(Vec3::ZERO, Vec3::splat(0.5));
        assert!(cap_faces(&cell).is_empty(), "a supplied hull face is not an open wound");
        assert!(largest_cap(&cell).is_none(), "and there is no largest one");
    }

    /// A bond maps to a wound with **no derivation at all** — the four fields are the bond's own. If
    /// this ever needs arithmetic, the wound has stopped being the shared face.
    #[test]
    fn a_bond_is_its_wound_exactly() {
        let graph = baked_graph(CutSettings::new(2, 0.1, 0x00C0_FFEE));
        assert!(graph.len() >= 1, "a two-piece cut must leave the halves bonded");

        let id = BondId(0);
        let bond = graph.bond(id).expect("bond 0 exists");
        let wounds = wounds_from_bonds(&graph, &[id]);
        assert_eq!(wounds.len(), 1, "one broken bond is one wound");
        let w = wounds[0];
        assert_eq!(w.at, bond.centroid, "the wound is where the shared face was");
        assert_eq!(w.normal, bond.normal, "and faces the way that face did");
        assert_eq!(w.area, bond.area, "and is as wide as that face was");
        assert_eq!(w.severity, 1.0, "an already-broken bond is fully open");
        assert_eq!(w.kind, WoundKind::Severance);
    }

    /// A bond id from another frontier must be **skipped, not resolved**. Resolving it would silently
    /// return a different bond's geometry and put the wound somewhere the blow never landed.
    #[test]
    fn a_bond_from_another_graph_is_skipped() {
        let graph = baked_graph(CutSettings::new(2, 0.1, 0x00C0_FFEE));
        let beyond = BondId(graph.len() as u32 + 7);
        assert!(
            wounds_from_bonds(&graph, &[beyond]).is_empty(),
            "an id this graph does not hold must produce no wound at all"
        );
    }

    /// Wounds come back in `BondId` order regardless of the order the caller asked in — which is what
    /// makes a fold over them reproducible.
    #[test]
    fn wounds_are_returned_in_bond_id_order() {
        let graph = baked_graph(CutSettings::new(8, 0.05, 0xD00D));
        assert!(graph.len() >= 3, "need a few bonds to have an order at all");

        let ids: Vec<BondId> = (0..graph.len() as u32).map(BondId).collect();
        let forward = wounds_from_bonds(&graph, &ids);
        let mut reversed = ids.clone();
        reversed.reverse();
        let backward = wounds_from_bonds(&graph, &reversed);
        assert_eq!(forward, backward, "the caller's iteration order must not reach the output");
    }

    /// A real cut face: unit normal, positive area, and it exists. The tolerances are the crate's own
    /// — `1e-4` on a normal that was built by normalising a float cross product.
    #[test]
    fn a_cut_cell_has_a_unit_normal_cap_of_real_area() {
        let (pieces, _, _, _) =
            fracture(crate::soup::Soup::default(), &unit_cube_cells(), &CutSettings::new(2, 0.1, 0x00C0_FFEE));
        let with_caps: Vec<_> =
            pieces.iter().filter(|p| !cap_faces(&p.cell).is_empty()).collect();
        assert!(!with_caps.is_empty(), "a cut must leave at least one open face");
        for p in with_caps {
            for f in cap_faces(&p.cell) {
                assert!(
                    (f.normal.length() - 1.0).abs() < 1.0e-4,
                    "cap normal length {} is not unit",
                    f.normal.length()
                );
                assert!(f.area > 0.0, "a cap face with no area should have been skipped");
            }
        }
    }

    /// **The one that proves a bullet hole bleeds.** A bore's barrel wall is `FaceKind::Bore`, which
    /// `face_is_cut` reports as open — so it arrives as a cap face, and a barrel plane contains the
    /// channel axis, so its normal is perpendicular to that axis.
    ///
    /// **`flare: 0.0` is the point of the test, not a simplification.** Flare tilts every barrel
    /// plane rather than adding any, so the shipped `Bore::new` flare of 0.25 over this channel's
    /// length puts the wall about 0.01 rad off perpendicular — measured, and the reason a 1e-3
    /// tolerance against a *flared* bore fails. Jaggedness is left at its shipped 0.35 deliberately:
    /// it slides each plane inward along its own normal, so a ragged channel's walls are still
    /// exactly parallel to the axis, and this asserts that.
    #[test]
    fn a_bored_cell_exposes_its_channel_wall_perpendicular_to_the_axis() {
        let axis = Vec3::X;
        let bore = Bore {
            flare: 0.0,
            ..Bore::new(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 0.08)
        };
        let cut = CutSettings { bores: vec![bore], ..CutSettings::new(2, 0.1, 0x00C0_FFEE) };
        let (pieces, _, _, _) = fracture(crate::soup::Soup::default(), &unit_cube_cells(), &cut);

        let walls: Vec<CapFace> = pieces
            .iter()
            .flat_map(|p| cap_faces(&p.cell))
            .filter(|f| f.normal.dot(axis).abs() < 1.0e-3)
            .collect();
        assert!(
            !walls.is_empty(),
            "no cap face was perpendicular to the bore axis — the channel wall is not being \
             reported as open, so a bullet hole would not bleed"
        );
        assert!(
            walls.iter().all(|f| f.area > 0.0),
            "a channel wall with no area would give a bullet hole no blood to throw"
        );

        /// The same channel, flared, still exposes walls — just tilted by the flare. Asserted here
        /// rather than in its own test because the two are one fact about the same geometry.
        const FLARE_TILT: f32 = 0.02;
        let flared = Bore::new(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 0.08);
        let cut = CutSettings { bores: vec![flared], ..CutSettings::new(2, 0.1, 0x00C0_FFEE) };
        let (pieces, _, _, _) = fracture(crate::soup::Soup::default(), &unit_cube_cells(), &cut);
        assert!(
            pieces
                .iter()
                .flat_map(|p| cap_faces(&p.cell))
                .any(|f| f.normal.dot(axis).abs() < FLARE_TILT),
            "a flared channel's wall should still be within the flare's own tilt of perpendicular"
        );
    }

    /// `largest_cap` picks the widest face, not the first — checked against the set it came from.
    #[test]
    fn the_largest_cap_is_the_widest_one() {
        let (pieces, _, _, _) =
            fracture(crate::soup::Soup::default(), &unit_cube_cells(), &CutSettings::new(8, 0.05, 0xD00D));
        for p in &pieces {
            let all = cap_faces(&p.cell);
            match largest_cap(&p.cell) {
                None => assert!(all.is_empty(), "None must mean there were none"),
                Some(best) => {
                    assert!(
                        all.iter().all(|f| f.area <= best.area),
                        "a wider cap face than the chosen one exists"
                    );
                }
            }
        }
    }
}
