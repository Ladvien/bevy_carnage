//! **A studio to light the preset scenes in.**
//!
//! Wet blood is a liquid film, and what makes a film read as wet is what it reflects. Under Bevy's
//! `AmbientLight` alone there is nothing to reflect: the ambient is a uniform sky of one brightness from
//! every direction, so a film's clear coat mirrors a flat grey, and at a grazing angle, where the
//! Fresnel term goes to one, a fresh pool on the floor reads as a pale veil rather than a dark red.
//! A game's scene has a room. This is a small one, made in code: charcoal walls the colour of the
//! clips' backdrop, a grey floor, a ceiling with a soft panel overhead, a large softbox just above and
//! behind the camera, where a portrait key hangs so that every wet surface turned toward the viewer
//! carries its broad reflection, and a tall cool strip behind on the right where the fill is.
//!
//! **Prefiltered on the CPU, so it needs no compute.** Bevy can filter a source cubemap on the GPU
//! (`GeneratedEnvironmentMapLight`), but that needs compute shaders, which WebGL2 does not have, and
//! the flesh material's clips must hold there. So the two cubemaps `EnvironmentMapLight` takes are
//! made here: the specular map's mips each the radiance seen through a GGX lobe of the perceptual
//! roughness Bevy samples that mip at (`roughness · (levels − 1)`), and the diffuse map the
//! cosine-weighted mean radiance over the hemisphere, which Bevy uses exactly as it uses an ambient's
//! brightness. Both by a fixed Hammersley set, so two runs bake the same bytes.

use bevy::asset::RenderAssetUsages;
use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};

/// Edge of the specular map's top mip, texels. Seven mips, 64 down to 1.
const SPECULAR_EDGE: u32 = 64;
/// Edge of the diffuse map, texels: irradiance has no detail to keep.
const DIFFUSE_EDGE: u32 = 16;
/// Samples per texel for every filtered mip and for the diffuse map.
const SAMPLES: u32 = 256;

/// Where the key softbox hangs, from the subject: just above and behind the clips' cameras, which
/// stand about `(0.25, 1.25, 2.9)` and look at the body's middle.
const KEY: Vec3 = Vec3::new(0.1, 0.45, 1.0);
/// Where the fill strip stands: toward the preset scene's fill, at `(1.5, 2.5, -3)`.
const FILL: Vec3 = Vec3::new(1.5, 1.7, -3.0);

/// **The studio's radiance toward world direction `w`** (unit), relative: the intensity scales it.
fn radiance(w: Vec3) -> Vec3 {
    let walls = Vec3::new(0.08, 0.08, 0.09);
    let ceiling = Vec3::new(0.40, 0.40, 0.41);
    let floor = Vec3::new(0.25, 0.24, 0.23);
    let mut l = if w.y < 0.0 { floor.lerp(walls, smoothstep(-0.05, 0.0, w.y)) } else { walls.lerp(ceiling, smoothstep(0.15, 0.9, w.y)) };
    l += Vec3::new(1.0, 0.97, 0.92) * 6.0 * panel(w, KEY.normalize(), 0.55, 0.32);
    l += Vec3::new(1.0, 1.0, 1.0) * 2.0 * panel(w, Vec3::Y, 0.5, 0.5);
    l += Vec3::new(0.85, 0.90, 1.0) * 3.0 * panel(w, FILL.normalize(), 0.08, 0.40);
    l
}

/// A soft-edged rectangle facing the origin along `axis`, `half_w` wide and `half_h` tall in the plane
/// one unit out, `1` inside and `0` outside over a tenth of its size.
fn panel(w: Vec3, axis: Vec3, half_w: f32, half_h: f32) -> f32 {
    let along = w.dot(axis);
    if along <= 0.0 {
        return 0.0;
    }
    let reference = if axis.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let right = axis.cross(reference).normalize_or_zero();
    let up = right.cross(axis);
    let (u, v) = (w.dot(right) / along, w.dot(up) / along);
    let edge = |half: f32, t: f32| 1.0 - smoothstep(half * 0.9, half * 1.1, t.abs());
    edge(half_w, u) * edge(half_h, v)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The world direction a cube texel shows: wgpu's face order and orientation, then Bevy's flip of
/// `z` (its environment maps are sampled left-handed, `environment_map.wgsl`).
fn direction(face: u32, x: u32, y: u32, edge: u32) -> Vec3 {
    let u = 2.0 * (x as f32 + 0.5) / edge as f32 - 1.0;
    let v = 2.0 * (y as f32 + 0.5) / edge as f32 - 1.0;
    let c = match face {
        0 => Vec3::new(1.0, -v, -u),
        1 => Vec3::new(-1.0, -v, u),
        2 => Vec3::new(u, 1.0, v),
        3 => Vec3::new(u, -1.0, -v),
        4 => Vec3::new(u, -v, 1.0),
        _ => Vec3::new(-u, -v, -1.0),
    };
    Vec3::new(c.x, c.y, -c.z).normalize()
}

/// The `i`th of `n` points of the Hammersley set.
fn hammersley(i: u32, n: u32) -> (f32, f32) {
    (i as f32 / n as f32, i.reverse_bits() as f32 / 4_294_967_296.0)
}

/// Two unit vectors that complete `n` to an orthonormal frame.
fn frame(n: Vec3) -> (Vec3, Vec3) {
    let t = if n.y.abs() < 0.99 { Vec3::Y.cross(n).normalize() } else { Vec3::X.cross(n).normalize() };
    (t, n.cross(t))
}

/// The radiance toward `n` seen through a GGX lobe of `perceptual_roughness`, with the view along the
/// normal (the split-sum approximation Bevy's environment lighting assumes).
fn specular(n: Vec3, perceptual_roughness: f32) -> Vec3 {
    if perceptual_roughness <= 0.0 {
        return radiance(n);
    }
    let a = perceptual_roughness * perceptual_roughness;
    let (t, b) = frame(n);
    let (mut sum, mut weight) = (Vec3::ZERO, 0.0);
    for i in 0..SAMPLES {
        let (e1, e2) = hammersley(i, SAMPLES);
        let phi = 2.0 * std::f32::consts::PI * e1;
        let cos_t = ((1.0 - e2) / (1.0 + (a * a - 1.0) * e2)).sqrt();
        let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
        let h = t * (sin_t * phi.cos()) + b * (sin_t * phi.sin()) + n * cos_t;
        let l = 2.0 * n.dot(h) * h - n;
        let nl = n.dot(l);
        if nl > 0.0 {
            sum += radiance(l) * nl;
            weight += nl;
        }
    }
    if weight > 0.0 { sum / weight } else { radiance(n) }
}

/// The cosine-weighted mean radiance over the hemisphere around `n`.
fn diffuse(n: Vec3) -> Vec3 {
    let (t, b) = frame(n);
    let mut sum = Vec3::ZERO;
    for i in 0..SAMPLES {
        let (e1, e2) = hammersley(i, SAMPLES);
        let phi = 2.0 * std::f32::consts::PI * e1;
        let (r, z) = (e2.sqrt(), (1.0 - e2).max(0.0).sqrt());
        sum += radiance(t * (r * phi.cos()) + b * (r * phi.sin()) + n * z);
    }
    sum / SAMPLES as f32
}

/// An `f32` as IEEE half-precision bits, rounded to nearest; finite inputs in the range this file
/// writes, which is far inside a half's.
fn half(x: f32) -> [u8; 2] {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if exp <= 0 {
        return sign.to_le_bytes();
    }
    let mant = bits & 0x7f_ffff;
    let mut h = (u32::from(sign) | ((exp as u32) << 10) | (mant >> 13)) + ((mant >> 12) & 1);
    if exp >= 31 {
        h = u32::from(sign) | 0x7c00;
    }
    (h as u16).to_le_bytes()
}

/// A cubemap of `levels` mips from `edge`, each texel `texel(direction, mip)`, laid out layer-major
/// as wgpu reads it.
fn cubemap(edge: u32, levels: u32, texel: impl Fn(Vec3, u32) -> Vec3) -> Image {
    let mut data = Vec::new();
    for face in 0..6 {
        for mip in 0..levels {
            let e = (edge >> mip).max(1);
            for y in 0..e {
                for x in 0..e {
                    let l = texel(direction(face, x, y, e), mip);
                    for c in [l.x, l.y, l.z, 1.0] {
                        data.extend_from_slice(&half(c));
                    }
                }
            }
        }
    }
    let mut image = Image::default();
    image.data = Some(data);
    image.texture_descriptor.size = Extent3d { width: edge, height: edge, depth_or_array_layers: 6 };
    image.texture_descriptor.mip_level_count = levels;
    image.texture_descriptor.dimension = TextureDimension::D2;
    image.texture_descriptor.format = TextureFormat::Rgba16Float;
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image
}

/// **The studio, as an `EnvironmentMapLight`** at an intensity that makes its mean radiance over every
/// direction `mean`, in the units an `AmbientLight`'s brightness is: swap it for an ambient of that
/// brightness and the scene is lit as much overall, from a room instead of from nowhere.
pub fn environment(images: &mut Assets<Image>, mean: f32) -> EnvironmentMapLight {
    let levels = SPECULAR_EDGE.ilog2() + 1;
    let specular_map = cubemap(SPECULAR_EDGE, levels, |n, mip| specular(n, mip as f32 / (levels - 1) as f32));
    let diffuse_map = cubemap(DIFFUSE_EDGE, 1, |n, _| diffuse(n));
    // The room's own mean radiance, over a sphere of Hammersley directions.
    let n = 4096;
    let total: f32 = (0..n)
        .map(|i| {
            let (e1, e2) = hammersley(i, n);
            let (z, phi) = (1.0 - 2.0 * e2, 2.0 * std::f32::consts::PI * e1);
            let r = (1.0 - z * z).max(0.0).sqrt();
            let l = radiance(Vec3::new(r * phi.cos(), z, r * phi.sin()));
            (l.x + l.y + l.z) / 3.0
        })
        .sum();
    EnvironmentMapLight {
        diffuse_map: images.add(diffuse_map),
        specular_map: images.add(specular_map),
        intensity: mean / (total / n as f32),
        ..default()
    }
}
