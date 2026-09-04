//! **The render mesh: an `n`-gon swept along the node polyline, rebuilt on the CPU.**
//!
//! Rendering a strand is the caller's job. This hands back a [`Mesh`] and **never spawns** — no
//! entity, no material, no asset handle. An eight-sided tube over the 25 nodes [`crate::viscera::spill`]
//! produces is 384 side triangles plus 16 of cap, which is cheap enough to regenerate every tick and
//! is why there is no GPU skinning path to keep in step with the solver.

use core::f32::consts::TAU;

use bevy::asset::RenderAssetUsages;
use bevy::math::Vec3;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

use crate::viscera::frame::perpendicular_basis;
use crate::viscera::strand::Strand;

/// The fewest sides a tube can have and still enclose anything.
pub const MIN_SIDES: u32 = 3;
/// The most sides [`tube_mesh`] will build. Past this the silhouette stops changing.
pub const MAX_SIDES: u32 = 32;

/// **Build the tube for one strand.**
///
/// `sides` is clamped into `MIN_SIDES..=MAX_SIDES`. The result is a `TriangleList` carrying
/// `ATTRIBUTE_POSITION`, `ATTRIBUTE_NORMAL` and `ATTRIBUTE_UV_0` with `U32` indices, at
/// `RenderAssetUsages::default()` — main world *and* render world, because a caller regenerating the
/// mesh every tick has to be able to read back the one it just built.
///
/// Counts, exactly: `(nodes + 2) × (sides + 1) + 2` vertices — a duplicated seam vertex per ring so
/// the `u` coordinate runs 0→1 without wrapping, plus a fan centre and a rim copy for each end cap —
/// and `untorn_segments × sides × 2 + 2 × sides` triangles. Every emitted normal is unit length: the
/// side walls carry the radial, the caps carry the end tangent.
///
/// **A parted segment emits no side quads.** The tube is severed where the strand is severed, which
/// leaves the two new ends open: the caller is looking at a tear, and a tear is open.
///
/// The cross-section frame is parallel-transported. The first ring's reference is the world axis
/// *least aligned* with the first tangent (see [`crate::viscera::frame`]), and every ring after it re-projects
/// the previous ring's normal onto the new tangent's plane. Picking the reference from the tangent's
/// smallest component rather than always taking `Y` is what stops the frame flipping when a strand
/// happens to lie along an axis, and it makes the sweep a total function of the node positions — the
/// same strand gives the same mesh on every run.
pub fn tube_mesh(strand: &Strand, sides: u32) -> Mesh {
    let sides = sides.clamp(MIN_SIDES, MAX_SIDES) as usize;
    let ring = sides + 1; // the seam vertex is duplicated so UVs do not wrap
    let nodes = strand.nodes();
    let radius = strand.radius();
    let node_count = nodes.len();

    let vertex_cap = (node_count + 2) * ring;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(vertex_cap);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(vertex_cap);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(vertex_cap);
    let mut indices: Vec<u32> = Vec::with_capacity(node_count * sides * 6 + sides * 6);

    // `Strand::new` clamps to at least one segment, so a shorter polyline can only arrive from a
    // future edit. It yields an empty mesh rather than a panic.
    if node_count < 2 {
        return finish(positions, normals, uvs, indices);
    }

    // --- the swept rings ------------------------------------------------------------------------
    let mut tangent = tangent_at(nodes, 0);
    let (mut normal, mut binormal) = perpendicular_basis(tangent);
    let span = (node_count - 1) as f32;

    for (i, centre) in nodes.iter().enumerate() {
        if i > 0 {
            let next = tangent_at(nodes, i);
            // Parallel transport by projection: keep whatever of the previous normal is already
            // perpendicular to the new tangent. A 180° reversal between adjacent nodes is the only
            // case with no answer, and there the previous frame is kept rather than guessed at.
            let projected = normal - next * normal.dot(next);
            normal = projected.try_normalize().unwrap_or(normal);
            binormal = next.cross(normal);
            tangent = next;
        }
        let v = i as f32 / span;
        for j in 0..ring {
            let (sin_t, cos_t) = (TAU * (j as f32) / (sides as f32)).sin_cos();
            let radial = normal * cos_t + binormal * sin_t;
            positions.push((*centre + radial * radius).to_array());
            normals.push(radial.to_array());
            uvs.push([j as f32 / sides as f32, v]);
        }
    }
    // `tangent` now holds the last node's tangent; the loop above walked every node.
    let last_tangent = tangent;

    for seg in 0..node_count - 1 {
        if strand.segment_torn(seg) {
            continue;
        }
        let base = (seg * ring) as u32;
        let next = base + ring as u32;
        for j in 0..sides as u32 {
            let (a, b, c, d) = (base + j, base + j + 1, next + j, next + j + 1);
            // Counter-clockwise seen from outside, which is wgpu's front face.
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }

    // --- the two end caps -----------------------------------------------------------------------
    // Snapshot the rim positions before appending, so the cap can carry its own flat normals without
    // aliasing the side wall's vertices.
    let first_rim: Vec<[f32; 3]> = positions.iter().take(ring).copied().collect();
    let last_rim: Vec<[f32; 3]> =
        positions.iter().skip((node_count - 1) * ring).take(ring).copied().collect();
    let first_tangent = tangent_at(nodes, 0);

    if let Some(&start) = nodes.first() {
        // The rings wind counter-clockwise about `+tangent`, so the cap facing `-tangent` reverses.
        push_cap(
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut indices,
            start,
            -first_tangent,
            &first_rim,
            true,
        );
    }
    if let Some(&end) = nodes.last() {
        push_cap(
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut indices,
            end,
            last_tangent,
            &last_rim,
            false,
        );
    }

    finish(positions, normals, uvs, indices)
}

/// The tangent at node `i`: a central difference in the interior, one-sided at the ends.
///
/// A coincident pair has no direction; `Vec3::Y` stands in so the frame stays orthonormal, which is
/// the only property the sweep needs from it.
fn tangent_at(nodes: &[Vec3], i: usize) -> Vec3 {
    let n = nodes.len();
    let (a, b) = if i == 0 {
        (nodes.first().copied(), nodes.get(1).copied())
    } else if i + 1 >= n {
        (nodes.get(n.saturating_sub(2)).copied(), nodes.last().copied())
    } else {
        (nodes.get(i - 1).copied(), nodes.get(i + 1).copied())
    };
    match (a, b) {
        (Some(a), Some(b)) => (b - a).try_normalize().unwrap_or(Vec3::Y),
        _ => Vec3::Y,
    }
}

/// A flat fan closing one end of the tube: a centre vertex plus its own copy of the rim, so the cap
/// carries the end tangent as its normal rather than the side wall's radials.
fn push_cap(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    centre: Vec3,
    outward: Vec3,
    rim: &[[f32; 3]],
    reverse: bool,
) {
    let ring = rim.len();
    let sides = ring.saturating_sub(1);
    if sides < MIN_SIDES as usize {
        return;
    }
    let base = positions.len() as u32;
    let normal = outward.to_array();

    positions.push(centre.to_array());
    normals.push(normal);
    uvs.push([0.5, 0.5]);
    for (j, p) in rim.iter().enumerate() {
        let (sin_t, cos_t) = (TAU * (j as f32) / (sides as f32)).sin_cos();
        positions.push(*p);
        normals.push(normal);
        uvs.push([0.5 + 0.5 * cos_t, 0.5 + 0.5 * sin_t]);
    }

    for j in 0..sides as u32 {
        let a = base + 1 + j;
        let b = base + 2 + j; // the duplicated seam vertex closes the fan without a modulo
        if reverse {
            indices.extend_from_slice(&[base, b, a]);
        } else {
            indices.extend_from_slice(&[base, a, b]);
        }
    }
}

/// Assemble the attribute arrays into a mesh.
fn finish(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}
