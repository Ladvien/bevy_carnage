//! **How far below the skin a point on a cut face is**, and the vertex attribute that carries it.
//!
//! A cut face belongs to a convex cell whose *supplied* faces are the subject's own surface — the
//! skin. For a point inside a convex polytope the distance to its boundary is exactly the minimum
//! over its faces of the signed distance to that face's plane, so depth-below-skin is a handful of
//! dot products and needs no mesh query at all. That is the whole reason this crate can run at bake
//! time on the CPU and hash its output.
//!
//! The depth lands in `ATTRIBUTE_UV_1` as `(depth / span, along / tile)`: a material that samples the
//! region's strip texture through `UvChannel::Uv1` then paints the bands, and `UV_0` — the planar
//! cross-section coordinates every cap already carries — is untouched, so nothing that hashed a cap
//! before this crate existed moves.

use bevy::math::{Vec2, Vec3};
use bevy::mesh::{Mesh, VertexAttributeValues};

use crate::cross_section::layers::Layers;

/// A skin plane: a point on the surface and the **outward** unit normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinPlane {
    /// Any point on the plane.
    pub point: Vec3,
    /// Outward unit normal.
    pub normal: Vec3,
}

/// **Depth of `p` below the nearest skin plane**, in the planes' own units. Negative outside.
///
/// Exact for a point inside a convex cell whose supplied faces are `planes`; no planes at all means
/// no skin to measure from, and the answer is zero rather than infinity so a cell with no supplied
/// face (an interior fragment) reads as skin rather than as marrow.
pub fn depth_below_skin(p: Vec3, planes: &[SkinPlane]) -> f32 {
    let mut best = f32::INFINITY;
    for pl in planes {
        let d = (pl.point - p).dot(pl.normal);
        if d < best {
            best = d;
        }
    }
    if best.is_finite() { best } else { 0.0 }
}

/// How the depth attribute is scaled into the strip's UV space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    /// Millimetres per mesh unit. `1000.0` for a mesh authored in metres.
    pub mm_per_unit: f32,
    /// Mesh units one repeat of the strip's along-axis covers. `0.05` (5 cm) at metres.
    pub tile_units: f32,
}

impl Default for Scale {
    fn default() -> Self {
        Self { mm_per_unit: 1000.0, tile_units: 0.05 }
    }
}

/// **Write `ATTRIBUTE_UV_1` on a cap mesh** from its positions and the cell's skin planes.
///
/// `offset` is added to every position before measuring — a cap mesh recentred on its fragment's
/// own origin hands in that centre here, so the planes and the positions share a frame. Along the
/// strip the coordinate is `UV_0.x` when the mesh has it (the planar cross-section coordinate, which
/// is continuous over the face) and otherwise the position's own `x`.
///
/// **The bone is a core, not a floor.** This function measures the cap's own deepest point —
/// `d_core`, the maximum of the depth at every vertex *and* at the cap's centroid, because a cut
/// through the middle of a limb has its deepest point in the interior rather than on the boundary —
/// and hands it to [`uv1_at_core`], which lays the muscle band over the meat and the cortex and
/// marrow over the innermost `bone_mm / 2`. On a region with `bone_mm = 0`, or a cap too shallow to
/// reach the cortex, that is exactly [`uv1_at`].
///
/// Returns the number of vertices written; zero, and nothing inserted, when the mesh has no
/// `Float32x3` positions.
pub fn annotate_cap(mesh: &mut Mesh, planes: &[SkinPlane], offset: Vec3, layers: &Layers, scale: &Scale) -> usize {
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return 0;
    };
    let along: Vec<f32> = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(uv)) if uv.len() == pos.len() => uv.iter().map(|u| u[0]).collect(),
        _ => pos.iter().map(|p| p[0]).collect(),
    };
    let d_core = core_depth_mm(pos, planes, offset, scale);
    let uv1: Vec<[f32; 2]> = pos
        .iter()
        .zip(along.iter())
        .map(|(p, a)| {
            let world = Vec3::from_array(*p) + offset;
            uv1_at_core(world, *a, planes, layers, scale, d_core).to_array()
        })
        .collect();
    let n = uv1.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    n
}

/// **How deep the deepest part of a cap is**, millimetres — the `d_core` [`uv1_at_core`] wants.
///
/// The maximum over the cap's vertices *and* its centroid. The centroid is in there because a cap is
/// a *boundary* and the deepest point of a convex cross-section is in its interior: on a limb the
/// vertices sit on the skin and a vertex-only maximum would report a bone that is not there. One
/// extra dot product per cap, which is cheaper than every other way of asking.
fn core_depth_mm(pos: &[[f32; 3]], planes: &[SkinPlane], offset: Vec3, scale: &Scale) -> f32 {
    let mm = if scale.mm_per_unit.is_finite() { scale.mm_per_unit } else { 0.0 };
    let mut deepest = 0.0f32;
    let mut sum = Vec3::ZERO;
    for p in pos {
        let world = Vec3::from_array(*p) + offset;
        sum += world;
        deepest = deepest.max(depth_below_skin(world, planes).max(0.0) * mm);
    }
    if !pos.is_empty() {
        let centroid = sum / pos.len() as f32;
        if centroid.is_finite() {
            deepest = deepest.max(depth_below_skin(centroid, planes).max(0.0) * mm);
        }
    }
    deepest
}

/// The `UV_1` a single point would receive — the per-vertex rule of [`annotate_cap`], exposed so a
/// caller building its own geometry (a wound bed, a flayed patch) can parameterise it the same way.
///
/// **Depth-only**: the bands lie where the table says, so a point 30 mm below the skin of a limb is
/// in the cortex whatever the piece it belongs to looks like. That is the right rule for a wound bed
/// cut into a surface, and the wrong one for a cut *through* a limb — for that, see
/// [`uv1_at_core`].
pub fn uv1_at(p: Vec3, along: f32, planes: &[SkinPlane], layers: &Layers, scale: &Scale) -> Vec2 {
    let span = layers.span_mm().max(1.0e-3);
    let tile = if scale.tile_units.is_finite() && scale.tile_units > 0.0 { scale.tile_units } else { 1.0 };
    let depth_mm = depth_below_skin(p, planes).max(0.0) * scale.mm_per_unit;
    Vec2::new((depth_mm / span).clamp(0.0, 1.0), along / tile)
}

/// **[`uv1_at`] with the bone as a core**, given the cap's own deepest depth `d_core` in millimetres.
///
/// The depth-only model puts cortical bone at every point past `starts_mm()[3]`, which on a limb is
/// 27.8 mm. That is right for a shallow wound and wrong for a severed thigh: it makes the whole
/// middle of the cut face bone, however wide the limb is, and the muscle band ends up a rim. A long
/// bone is a **core** — `Layers::bone_mm` across, in the middle of the meat — so the bands are laid
/// out from the deepest point of *this* cap instead:
///
/// - skin and fat keep their measured depths, because they are measured from the surface;
/// - `[fat_end, d_core − bone_mm/2)` maps linearly onto the **muscle** band, so the meat stretches
///   or compresses to fill whatever is between the fat and the bone;
/// - `[d_core − bone_mm/2, d_core]` maps onto **cortex + marrow**, so the bone is exactly
///   `bone_mm` wide across the deepest point and nowhere else.
///
/// The rule engages only when `bone_mm > 0` **and** the cap actually reaches the cortex
/// (`d_core > starts_mm()[3]`); otherwise this is [`uv1_at`] to the bit, which is why a torso, a head
/// and a shallow limb cut are untouched by it.
///
/// What is written is still `depth_equivalent / span` along x, so the strip texture, its digest and
/// every material sampling it are unchanged — the remapping happens on the way in, not in the image.
pub fn uv1_at_core(
    p: Vec3,
    along: f32,
    planes: &[SkinPlane],
    layers: &Layers,
    scale: &Scale,
    d_core: f32,
) -> Vec2 {
    let span = layers.span_mm().max(1.0e-3);
    let tile = if scale.tile_units.is_finite() && scale.tile_units > 0.0 { scale.tile_units } else { 1.0 };
    let depth_mm = depth_below_skin(p, planes).max(0.0) * scale.mm_per_unit;
    let equivalent = core_equivalent_mm(depth_mm, d_core, layers);
    Vec2::new((equivalent / span).clamp(0.0, 1.0), along / tile)
}

/// The depth the strip should be sampled at for a point `depth_mm` below the skin of a cap whose
/// deepest point is `d_core`. See [`uv1_at_core`] for the rule and why it exists.
fn core_equivalent_mm(depth_mm: f32, d_core: f32, layers: &Layers) -> f32 {
    let starts = layers.starts_mm();
    let (fat_end, cortex_start) = (starts[2], starts[3]);
    let bone = layers.bone_mm;
    // **No bone at all**: the trunk. Past the muscle there is a cavity this five-layer table cannot
    // name, and the nearest honest tissue is the muscle wall itself — so the meat fills the cut to
    // its deepest point and no cortex or marrow is ever drawn. (Sampled just under the cortex start,
    // where the strip is still muscle.)
    if bone == 0.0 {
        return depth_mm.min((cortex_start - 1.0e-3).max(fat_end));
    }
    // **Bone without end** — the skull: the shell begins at its measured depth and everything inside
    // is the depth-only rule, which is what a cranium cut through reads as.
    if !bone.is_finite() || !(bone > 0.0) || !d_core.is_finite() || d_core <= cortex_start {
        return depth_mm;
    }
    let span = layers.span_mm();
    // The bone's outer surface at the deepest point. Clamped to the fat's floor so a cap narrower
    // than the bone has no muscle band rather than an inverted one.
    let bone_top = (d_core - bone * 0.5).max(fat_end);
    if depth_mm <= fat_end {
        // Measured from the surface, so it keeps its measured depth.
        return depth_mm;
    }
    if depth_mm < bone_top {
        let t = (depth_mm - fat_end) / (bone_top - fat_end).max(1.0e-6);
        return fat_end + (cortex_start - fat_end) * t.clamp(0.0, 1.0);
    }
    let t = ((depth_mm - bone_top) / (d_core - bone_top).max(1.0e-6)).clamp(0.0, 1.0);
    cortex_start + (span - cortex_start) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_section::layers::Region;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    fn slab() -> Vec<SkinPlane> {
        // A slab 0.1 units thick between y = ±0.05, skin on both faces.
        vec![
            SkinPlane { point: Vec3::new(0.0, 0.05, 0.0), normal: Vec3::Y },
            SkinPlane { point: Vec3::new(0.0, -0.05, 0.0), normal: -Vec3::Y },
        ]
    }

    #[test]
    fn depth_is_the_nearest_skin() {
        let s = slab();
        assert!((depth_below_skin(Vec3::ZERO, &s) - 0.05).abs() < 1.0e-6);
        assert!((depth_below_skin(Vec3::new(0.0, 0.04, 0.0), &s) - 0.01).abs() < 1.0e-6);
        assert!(depth_below_skin(Vec3::new(0.0, 0.06, 0.0), &s) < 0.0, "outside is negative");
        assert_eq!(depth_below_skin(Vec3::ONE, &[]), 0.0, "no skin, no depth");
    }

    #[test]
    fn annotate_writes_uv1_and_leaves_uv0_alone() {
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        m.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.05, 0.0], [0.0, 0.0, 0.0], [0.02, -0.05, 0.0]],
        );
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0], [0.5, 0.0], [1.0, 0.0]]);
        let layers = Layers::for_region(Region::Limb);
        let n = annotate_cap(&mut m, &slab(), Vec3::ZERO, &layers, &Scale::default());
        assert_eq!(n, 3);
        let Some(VertexAttributeValues::Float32x2(uv1)) = m.attribute(Mesh::ATTRIBUTE_UV_1) else {
            panic!("UV_1 missing");
        };
        assert_eq!(uv1[0][0], 0.0, "a vertex on the skin is at depth zero");
        // 50 mm below the skin, over a 40.8 mm limb span: clamped to the end of the strip.
        assert_eq!(uv1[1][0], 1.0);
        assert_eq!(uv1[2][0], 0.0);
        // Along the strip is UV_0.x over the tile length.
        assert!((uv1[1][1] - 0.5 / 0.05).abs() < 1.0e-5);
        let Some(VertexAttributeValues::Float32x2(uv0)) = m.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!("UV_0 lost");
        };
        assert_eq!(uv0[1], [0.5, 0.0]);
    }

    /// **The bone is `bone_mm` wide at the deepest point, and the muscle fills the rest.**
    ///
    /// A cap whose deepest point is 60 mm down on the limb row: the 27 mm bone means its outer
    /// surface is at `60 − 13.5 = 46.5` mm, so a vertex at 45 mm is still meat and one at 59 mm is
    /// nearly through the cortex into the marrow. Under the depth-only rule both would be marrow,
    /// because both are past the table's 27.8 mm cortex start — which is the thing this rule exists
    /// to stop.
    #[test]
    fn a_deep_cap_puts_the_bone_in_the_middle() {
        let layers = Layers::for_region(Region::Limb);
        let starts = layers.starts_mm();
        let (muscle_start, cortex_start) = (starts[2], starts[3]);
        let span = layers.span_mm();
        let scale = Scale::default();
        // One skin plane at the origin, so a point `d` units below it is `d · 1000` mm down.
        let skin = [SkinPlane { point: Vec3::ZERO, normal: Vec3::Y }];
        let at = |depth_mm: f32| {
            uv1_at_core(
                Vec3::new(0.0, -depth_mm / scale.mm_per_unit, 0.0),
                0.0,
                &skin,
                &layers,
                &scale,
                60.0,
            )
            .x * span
        };

        let meat = at(45.0);
        assert!(
            meat > muscle_start && meat < cortex_start,
            "45 mm into a 60 mm cap read as {meat} mm, not muscle ({muscle_start}..{cortex_start})"
        );
        let deep = at(59.0);
        assert!(deep > starts[4], "59 mm into a 60 mm cap read as {deep} mm, which is not marrow");
        // Skin and fat are measured from the surface, so they are where the table put them.
        assert!((at(1.0) - 1.0).abs() < 1.0e-3, "the skin moved");
        assert!((at(5.0) - 5.0).abs() < 1.0e-3, "the fat moved");
        // The bone's outer surface lands exactly on the cortex start, which is what makes the core
        // `bone_mm` wide rather than approximately so.
        let rim = at(60.0 - layers.bone_mm * 0.5);
        assert!((rim - cortex_start).abs() < 1.0e-3, "the bone's rim is at {rim} mm, not {cortex_start}");
        // Monotone: a deeper vertex never reads as shallower tissue.
        let mut last = -1.0;
        let mut d = 0.0;
        while d <= 60.0 {
            let now = at(d);
            assert!(now >= last - 1.0e-4, "the mapping went backwards at {d} mm");
            last = now;
            d += 0.25;
        }
    }

    /// **The skull is the depth-only rule, bit for bit, and the trunk never shows bone.** The head
    /// carries `bone_mm = ∞`, so `annotate_cap` cannot have moved anything on it and neither can a
    /// limb cut too shallow to reach the cortex; the torso carries `bone_mm = 0`, so its cut is skin,
    /// fat and then muscle all the way down.
    #[test]
    fn a_skull_is_the_plain_rule_and_a_trunk_never_shows_bone() {
        let scale = Scale::default();
        let skin = [SkinPlane { point: Vec3::ZERO, normal: Vec3::Y }];
        for region in [Region::Head, Region::Limb] {
            let layers = Layers::for_region(region);
            for step in 0..200u32 {
                let depth_mm = step as f32 * 0.5;
                let p = Vec3::new(0.0, -depth_mm / scale.mm_per_unit, 0.0);
                let plain = uv1_at(p, 0.25, &skin, &layers, &scale);
                // `d_core` at the cortex start exactly: the rule's own gate is `>`, so it is off.
                let core = uv1_at_core(p, 0.25, &skin, &layers, &scale, layers.starts_mm()[3]);
                assert_eq!(core, plain, "{region:?} at {depth_mm} mm moved");
                if !layers.bone_mm.is_finite() {
                    let deep = uv1_at_core(p, 0.25, &skin, &layers, &scale, 90.0);
                    assert_eq!(deep, plain, "{region:?} is bone without end but its mapping moved");
                }
            }
        }
        let torso = Layers::for_region(Region::Torso);
        let cortex = torso.starts_mm()[3];
        for depth_mm in [0.0, 1.0, 10.0, 20.0, cortex, 40.0, 120.0] {
            let p = Vec3::new(0.0, -depth_mm / scale.mm_per_unit, 0.0);
            let uv = uv1_at_core(p, 0.25, &skin, &torso, &scale, 120.0);
            let (layer, _) = torso.at(uv.x * torso.span_mm());
            assert_ne!(layer, crate::cross_section::Layer::Cortex, "the trunk showed cortex at {depth_mm} mm");
            assert_ne!(layer, crate::cross_section::Layer::Marrow, "the trunk showed marrow at {depth_mm} mm");
            if depth_mm < cortex {
                let plain = uv1_at(p, 0.25, &skin, &torso, &scale);
                assert_eq!(uv, plain, "the trunk's soft tissue moved at {depth_mm} mm");
            }
        }
    }

    #[test]
    fn a_mesh_without_positions_is_refused() {
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        assert_eq!(annotate_cap(&mut m, &slab(), Vec3::ZERO, &Layers::for_region(Region::Head), &Scale::default()), 0);
        assert!(m.attribute(Mesh::ATTRIBUTE_UV_1).is_none());
    }
}
