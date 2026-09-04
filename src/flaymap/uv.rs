//! **World-space hit → texture-space UV.** Möller–Trumbore, then a barycentric UV read.
//!
//! Möller & Trumbore, *"Fast, minimum storage ray-triangle intersection"*, Journal of Graphics Tools
//! 2(1), 1997, `doi:10.1080/10867651.1997.10487468`. Reimplemented here rather than imported: it is
//! thirty lines, and the two crates that already own a copy are both on the wrong side of this one.
//! `bevy_carnage` composes this crate, so depending on it would invert the layering; `bevy_wetmap` is
//! a **sibling**, not a base, and taking a dependency on it would mean an actor that only ever gets
//! flayed still resolves a blood-drip model it never calls.
//!
//! # Why a miss and an unusable mesh are different answers
//!
//! A ray that misses is ordinary — a shot went past the actor — so it is silent. A mesh with no
//! `ATTRIBUTE_UV_0` is a **content bug**: it cannot carry a flaymap at all, and every hit a caller
//! thinks it is peeling will never appear anywhere. Silently peeling nothing there is the failure
//! this module returns a distinct variant for, so the canvas can say so once and then shut up.

use bevy::math::{Vec2, Vec3};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};

use crate::flaymap::digest::Fnv1a;

/// Rays closer than this to parallel with a triangle's plane are treated as missing it. Below this the
/// barycentric division is dominated by its own rounding.
const PARALLEL_EPSILON: f32 = 1.0e-8;

/// What a ray found.
pub(crate) enum Pick {
    /// The nearest forward hit, in the **mesh's own space**.
    At {
        /// The barycentric `ATTRIBUTE_UV_0` at the hit.
        uv: Vec2,
        /// Where the ray met the triangle: `origin + dir · t`.
        point: Vec3,
        /// The triangle's **geometric** normal — `e1 × e2` from its winding, normalised, and not the
        /// interpolated shading normal, because a consumer fracturing at this point wants the plane
        /// the surface actually lies in. Zero for a degenerate triangle.
        ///
        /// **Not flipped toward the ray.** The intersection is two-sided (a hit can land on the
        /// inside of an opened torso), and a caller that needs the side it was struck from has it in
        /// one dot product, whereas a normal this module had already turned around could not be
        /// recovered.
        normal: Vec3,
    },
    /// The mesh is fine; the ray went past it.
    Miss,
    /// **The mesh cannot carry a flaymap.** No `Float32x3` positions, no `Float32x2` UV0, or not a
    /// triangle list. Worth exactly one warning per mesh.
    Unusable,
}

/// A stable-enough fingerprint of a mesh, for the once-per-mesh warning.
///
/// Vertex and index counts plus the first and last position, which is what actually distinguishes the
/// meshes an actor is built from. It is a memo key, not an identity: a collision costs one suppressed
/// warning about a second broken mesh, which is why it does not need to be a hash of every vertex —
/// that would make the refusal cost a full pass over the geometry it is refusing.
pub(crate) fn mesh_key(mesh: &Mesh) -> u64 {
    let mut f = Fnv1a::new();
    let verts = mesh.count_vertices() as u64;
    for k in 0..8 {
        f.byte((verts >> (k * 8)) as u8);
    }
    let indices = match mesh.indices() {
        Some(Indices::U16(v)) => v.len() as u64,
        Some(Indices::U32(v)) => v.len() as u64,
        None => 0,
    };
    for k in 0..8 {
        f.byte((indices >> (k * 8)) as u8);
    }
    if let Some(VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        && let (Some(first), Some(last)) = (p.first(), p.last())
    {
        for v in [first, last] {
            for c in v {
                for b in c.to_le_bytes() {
                    f.byte(b);
                }
            }
        }
    }
    f.finish()
}

/// **The UV a ray lands on**, in mesh-local space.
///
/// `origin` and `dir` are already in the mesh's own space — the caller inverts the actor's transform
/// once per hit rather than transforming every vertex, which is the whole reason the ray is moved
/// instead of the geometry.
///
/// Triangles are walked in index order and the **nearest forward hit wins**, ties going to the earlier
/// triangle. That order is total by construction, so no sort is needed and none may be added: a sort
/// here would be a second answer to a question the index buffer already answers.
pub(crate) fn ray_uv(mesh: &Mesh, origin: Vec3, dir: Vec3) -> Pick {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return Pick::Unusable;
    }
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return Pick::Unusable;
    };
    let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
        return Pick::Unusable;
    };
    if uv.len() != pos.len() || pos.len() < 3 {
        return Pick::Unusable;
    }
    if !origin.is_finite() || !dir.is_finite() || dir.length_squared() <= 0.0 {
        return Pick::Miss;
    }

    let mut best_t = f32::INFINITY;
    let mut best: Option<(Vec2, Vec3, Vec3)> = None;

    // One closure over both index encodings and the non-indexed case. `U16` and `U32` are two
    // spellings of one triangle list, and a non-indexed list is the third; refusing two of the three
    // would refuse meshes that are not broken.
    let mut consider = |a: usize, b: usize, c: usize| {
        let (Some(&pa), Some(&pb), Some(&pc)) = (pos.get(a), pos.get(b), pos.get(c)) else {
            return;
        };
        let (p0, p1, p2) = (Vec3::from_array(pa), Vec3::from_array(pb), Vec3::from_array(pc));
        // Möller–Trumbore. Two-sided, because a hit can land on the inside of an opened torso too.
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let h = dir.cross(e2);
        let det = e1.dot(h);
        // Every rejection below is written as a negated acceptance, so a NaN — from a non-finite
        // vertex — fails it rather than slipping past a `<`. A `best_t` of NaN would otherwise make
        // every later `t >= best_t` false and hand the hit to whichever triangle came next.
        if !(det.abs() >= PARALLEL_EPSILON) {
            return;
        }
        let inv = 1.0 / det;
        let s = origin - p0;
        let u = s.dot(h) * inv;
        if !(0.0..=1.0).contains(&u) {
            return;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        if !(v >= 0.0 && u + v <= 1.0) {
            return;
        }
        let t = e2.dot(q) * inv;
        // Strictly forward, and strictly nearer: a tie keeps the earlier triangle.
        if !(t > PARALLEL_EPSILON && t < best_t) {
            return;
        }
        let (Some(&ta), Some(&tb), Some(&tc)) = (uv.get(a), uv.get(b), uv.get(c)) else {
            return;
        };
        best_t = t;
        let (t0, t1, t2) = (Vec2::from_array(ta), Vec2::from_array(tb), Vec2::from_array(tc));
        best = Some((
            t0 * (1.0 - u - v) + t1 * u + t2 * v,
            origin + dir * t,
            e1.cross(e2).normalize_or_zero(),
        ));
    };

    match mesh.indices() {
        Some(Indices::U16(idx)) => {
            for tri in idx.chunks_exact(3) {
                let (Some(&a), Some(&b), Some(&c)) = (tri.first(), tri.get(1), tri.get(2)) else {
                    continue;
                };
                consider(a as usize, b as usize, c as usize);
            }
        }
        Some(Indices::U32(idx)) => {
            for tri in idx.chunks_exact(3) {
                let (Some(&a), Some(&b), Some(&c)) = (tri.first(), tri.get(1), tri.get(2)) else {
                    continue;
                };
                consider(a as usize, b as usize, c as usize);
            }
        }
        None => {
            for t in 0..pos.len() / 3 {
                consider(t * 3, t * 3 + 1, t * 3 + 2);
            }
        }
    }

    match best {
        Some((uv, point, normal)) if uv.is_finite() && point.is_finite() => {
            Pick::At { uv, point, normal }
        }
        _ => Pick::Miss,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;

    /// One triangle in the z = 0 plane with a UV per corner, so a barycentric read has a known answer.
    fn corner_tri(with_uv: bool) -> Mesh {
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD);
        m.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        if with_uv {
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        }
        m.insert_indices(Indices::U32(vec![0, 1, 2]));
        m
    }

    #[test]
    fn a_ray_down_a_corner_reads_that_corners_uv_its_point_and_its_normal() {
        let mesh = corner_tri(true);
        let hit = ray_uv(&mesh, Vec3::new(0.9, 0.05, 1.0), Vec3::new(0.0, 0.0, -1.0));
        match hit {
            Pick::At { uv, point, normal } => {
                assert!((uv.x - 0.9).abs() < 1.0e-4, "u was {}", uv.x);
                assert!((uv.y - 0.05).abs() < 1.0e-4, "v was {}", uv.y);
                // The triangle is the z = 0 plane, so the hit is on it and the winding normal is +Z.
                assert!(point.distance(Vec3::new(0.9, 0.05, 0.0)) < 1.0e-4, "point was {point}");
                assert!(normal.distance(Vec3::Z) < 1.0e-4, "normal was {normal}");
            }
            _ => panic!("a ray straight through the triangle must hit it"),
        }
    }

    #[test]
    fn a_mesh_without_uvs_is_unusable_and_a_ray_past_it_only_misses() {
        assert!(matches!(
            ray_uv(&corner_tri(false), Vec3::new(0.2, 0.2, 1.0), Vec3::new(0.0, 0.0, -1.0)),
            Pick::Unusable
        ));
        assert!(matches!(
            ray_uv(&corner_tri(true), Vec3::new(5.0, 5.0, 1.0), Vec3::new(0.0, 0.0, -1.0)),
            Pick::Miss
        ));
    }
}
