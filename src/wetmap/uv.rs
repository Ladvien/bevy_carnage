//! **World-space hit → texture-space UV.** Möller–Trumbore, then a barycentric UV read.
//!
//! Möller & Trumbore, *"Fast, minimum storage ray-triangle intersection"*, Journal of Graphics Tools
//! 2(1), 1997, `doi:10.1080/10867651.1997.10487468`. Reimplemented here rather than taken from
//! `bevy_carnage`: it is thirty lines, and depending on the gore crate for it would invert the
//! layering — `bevy_carnage` composes this crate, not the other way round.
//!
//! # Why a miss and an unusable mesh are different answers
//!
//! A ray that misses is ordinary — a shot went past the actor — so it is silent. A mesh with no
//! `ATTRIBUTE_UV_0` is a **content bug**: it cannot carry a wetmap at all, and the blood a caller
//! thinks it is painting will never appear anywhere. Silently painting nothing there is the failure
//! this module returns a distinct variant for, so the canvas can say so once and then shut up.

use bevy::math::{Vec2, Vec3};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};

use crate::wetmap::digest::Fnv1a;

/// Rays closer than this to parallel with a triangle's plane are treated as missing it. Below this the
/// barycentric division is dominated by its own rounding.
const PARALLEL_EPSILON: f32 = 1.0e-8;

/// What a ray found.
pub(crate) enum Pick {
    /// The UV of the nearest forward hit.
    At(Vec2),
    /// The mesh is fine; the ray went past it.
    Miss,
    /// **The mesh cannot carry a wetmap.** No `Float32x3` positions, no `Float32x2` UV0, or not a
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
    let mut best: Option<Vec2> = None;

    // One closure over both index encodings and the non-indexed case. `U16` and `U32` are two
    // spellings of one triangle list, and a non-indexed list is the third; refusing two of the three
    // would refuse meshes that are not broken.
    let mut consider = |a: usize, b: usize, c: usize| {
        if a >= pos.len() || b >= pos.len() || c >= pos.len() {
            return;
        }
        let (p0, p1, p2) =
            (Vec3::from_array(pos[a]), Vec3::from_array(pos[b]), Vec3::from_array(pos[c]));
        // Möller–Trumbore. Two-sided, because blood lands on the inside of an opened torso too.
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let h = dir.cross(e2);
        let det = e1.dot(h);
        if det.abs() < PARALLEL_EPSILON {
            return;
        }
        let inv = 1.0 / det;
        let s = origin - p0;
        let u = s.dot(h) * inv;
        if u < 0.0 || u > 1.0 {
            return;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return;
        }
        let t = e2.dot(q) * inv;
        // Strictly forward, and strictly nearer: a tie keeps the earlier triangle.
        if t <= PARALLEL_EPSILON || t >= best_t {
            return;
        }
        best_t = t;
        let (t0, t1, t2) =
            (Vec2::from_array(uv[a]), Vec2::from_array(uv[b]), Vec2::from_array(uv[c]));
        best = Some(t0 * (1.0 - u - v) + t1 * u + t2 * v);
    };

    match mesh.indices() {
        Some(Indices::U16(idx)) => {
            for tri in idx.chunks_exact(3) {
                consider(tri[0] as usize, tri[1] as usize, tri[2] as usize);
            }
        }
        Some(Indices::U32(idx)) => {
            for tri in idx.chunks_exact(3) {
                consider(tri[0] as usize, tri[1] as usize, tri[2] as usize);
            }
        }
        None => {
            for t in 0..pos.len() / 3 {
                consider(t * 3, t * 3 + 1, t * 3 + 2);
            }
        }
    }

    match best {
        Some(found) if found.is_finite() => Pick::At(found),
        _ => Pick::Miss,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;

    /// One triangle in the z = 0 plane with a UV per corner, so a barycentric read has a known answer.
    fn quad_tri(with_uv: bool) -> Mesh {
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
    fn a_hit_reads_the_barycentric_uv() {
        let m = quad_tri(true);
        let pick = ray_uv(&m, Vec3::new(0.25, 0.25, 1.0), Vec3::new(0.0, 0.0, -1.0));
        match pick {
            Pick::At(uv) => {
                assert!((uv.x - 0.25).abs() < 1.0e-5, "u was {}", uv.x);
                assert!((uv.y - 0.25).abs() < 1.0e-5, "v was {}", uv.y);
            }
            _ => panic!("expected a hit"),
        }
    }

    #[test]
    fn a_ray_past_the_triangle_misses_rather_than_refusing() {
        let m = quad_tri(true);
        assert!(matches!(
            ray_uv(&m, Vec3::new(5.0, 5.0, 1.0), Vec3::new(0.0, 0.0, -1.0)),
            Pick::Miss
        ));
        // Behind the origin is a miss too: a wetmap is painted by what a ray travels into.
        assert!(matches!(
            ray_uv(&m, Vec3::new(0.25, 0.25, 1.0), Vec3::new(0.0, 0.0, 1.0)),
            Pick::Miss
        ));
    }

    #[test]
    fn a_mesh_with_no_uvs_is_unusable_not_a_miss() {
        let m = quad_tri(false);
        assert!(matches!(
            ray_uv(&m, Vec3::new(0.25, 0.25, 1.0), Vec3::new(0.0, 0.0, -1.0)),
            Pick::Unusable
        ));
    }

    #[test]
    fn the_nearest_forward_hit_wins() {
        // Two parallel triangles; the near one carries UV (1, 1) and the far one (0, 0).
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD);
        m.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0, 0.0, -1.0],
                [1.0, 0.0, -1.0],
                [0.0, 1.0, -1.0],
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        );
        m.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [1.0, 1.0], [1.0, 1.0], [1.0, 1.0]],
        );
        // The far triangle is listed FIRST, so "nearest wins" is doing the work rather than "first".
        m.insert_indices(Indices::U32(vec![0, 1, 2, 3, 4, 5]));
        match ray_uv(&m, Vec3::new(0.2, 0.2, 2.0), Vec3::new(0.0, 0.0, -1.0)) {
            Pick::At(uv) => assert_eq!(uv, Vec2::new(1.0, 1.0)),
            _ => panic!("expected a hit"),
        }
    }

    #[test]
    fn the_mesh_key_separates_two_different_meshes() {
        assert_ne!(mesh_key(&quad_tri(true)), {
            let mut m = quad_tri(true);
            m.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            );
            mesh_key(&m)
        });
    }
}
