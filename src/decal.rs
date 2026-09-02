//! **Stains, as entities.** The cosmetic half of blood on the floor.
//!
//! Entirely behind the `vfx` feature, and it is the *only* half that is optional: **where blood lands
//! is core.** [`crate::spatter::stains`] is deterministic and available headless, because on the
//! consuming side a blood pool's position feeds simulation — a mycelium colony reads pools as
//! chemoattractant sources. So this module turns a [`Stain`] into something visible and does nothing
//! else. It never decides where one is.
//!
//! # Two Bevy facts this module is shaped around, both verified in 0.19.1's source
//!
//! **Do not register the decal material or plugin.** `ForwardDecalPlugin` is added *inside*
//! `PbrPlugin` (`bevy_pbr-0.19.1/src/lib.rs:243`) and registers
//! `MaterialPlugin::<ForwardDecalMaterial<StandardMaterial>>` itself
//! (`decal/forward.rs:43`). Adding either again panics on a duplicate plugin.
//!
//! **The camera must carry `DepthPrepass`.** `ForwardDecal`'s own usage notes say so
//! (`decal/forward.rs:58`): a forward decal reconstructs the surface it lies on from the depth buffer,
//! so without a prepass there is nothing to blend against and the decal renders as an opaque quad or
//! not at all. That is the first thing to check if stains look wrong, and it is a property of the
//! *camera*, not of this module — which is why [`spawn_stain`] cannot fix it for you.
//!
//! The decal mesh is inserted for you by a component hook (`decal/forward.rs:154-167`), which replaces
//! a **defaulted** `Mesh3d` handle. So spawn `ForwardDecal` with no mesh and let the hook run; passing
//! one would suppress it.
//!
//! # Four splats, generated, not shipped
//!
//! The crate ships **no asset files**, and neither do its examples — a consumer should not have to
//! copy a texture out of a dependency to get blood. The splat textures are generated from
//! [`crate::soup::hash_f32`] on first use and cached in [`SplatTextures`].
//!
//! Four variants rather than one per stain: a material is an asset, and a distinct material per stain
//! specialises a render pipeline per stain. Four is enough that a floor does not read as tiled, and it
//! is four pipelines rather than hundreds.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::pbr::decal::{ForwardDecal, ForwardDecalMaterial, ForwardDecalMaterialExt};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::pool::Pool;
use crate::soup::hash_f32;
use crate::spatter::Stain;

/// How many distinct splat textures are generated. See the module docs for why it is not one per
/// stain and not one overall.
pub const SPLAT_VARIANTS: usize = 4;

/// Edge length of a generated splat texture, in pixels.
///
/// 64 is enough for a stain that reads at a metre and is 16 KB of RGBA — small enough that generating
/// four at startup is free, and large enough that the lobes are not visibly stepped.
const SPLAT_SIZE: u32 = 64;

/// The generated splat textures and the four materials built over them.
///
/// **Materials, not just images.** A stain entity needs a material handle, and building one per stain
/// would specialise a pipeline per stain; these four are built once and shared.
#[derive(Resource, Debug, Clone)]
pub struct SplatTextures {
    /// The four textures, in variant order.
    pub images: Vec<Handle<Image>>,
    /// The four decal materials over those textures, in the same order.
    pub materials: Vec<Handle<ForwardDecalMaterial<StandardMaterial>>>,
}

impl SplatTextures {
    /// The material a stain should use, chosen from its own seed.
    ///
    /// Deterministic, because the seed is: two runs stamp the same stain with the same splat. Not
    /// required for correctness — nothing reads a texture back — but a recorded demo that differed
    /// frame to frame would be a false negative on the digest check the recorder exists for.
    pub fn material_for(&self, stain: &Stain) -> Handle<ForwardDecalMaterial<StandardMaterial>> {
        let i = (stain.seed as usize) % self.materials.len().max(1);
        self.materials[i].clone()
    }
}

/// **A blood splat, generated.** Radial falloff broken by hashed lobes, so it reads as a splash rather
/// than as an airbrushed circle.
///
/// The alpha channel carries the shape and the colour channels are left white, because the stain's
/// colour comes from the material's `base_color` — one texture then serves any blood colour a consumer
/// wants, and a paler or darker blood needs no new asset.
///
/// `variant` selects a different set of lobes. Pure: the same variant is the same bytes every run.
pub fn splat_image(variant: u32) -> Image {
    // Eight lobes, each a bump in the falloff at its own angle with its own reach and width. Drawn
    // from the crate's only random source, keyed by the variant, so the four are different and each
    // is reproducible.
    const LOBES: usize = 8;
    let mut lobe_angle = [0.0f32; LOBES];
    let mut lobe_reach = [0.0f32; LOBES];
    let mut lobe_width = [0.0f32; LOBES];
    for k in 0..LOBES {
        let key = variant
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add((k as u32).wrapping_mul(0x85EB_CA6B));
        lobe_angle[k] = hash_f32(key) * std::f32::consts::TAU;
        // Reach in [0.55, 1.15] of the radius: some lobes fall short of the rim, some overshoot it,
        // which is what breaks the circle.
        lobe_reach[k] = 0.55 + hash_f32(key ^ 0xC2B2_AE35) * 0.60;
        // Width in [0.08, 0.38] radians: a mix of narrow spikes and broad bulges.
        lobe_width[k] = 0.08 + hash_f32(key ^ 0x27D4_EB2F) * 0.30;
    }

    let n = SPLAT_SIZE as usize;
    let mut data = vec![0u8; n * n * 4];
    let half = SPLAT_SIZE as f32 * 0.5;
    for y in 0..n {
        for x in 0..n {
            // Pixel centres, so the splat is symmetric about the texture centre rather than half a
            // pixel off it.
            let dx = (x as f32 + 0.5 - half) / half;
            let dy = (y as f32 + 0.5 - half) / half;
            let r = (dx * dx + dy * dy).sqrt();
            let theta = dy.atan2(dx);

            // The rim: a base radius pushed outward by whichever lobes point this way.
            let mut rim = 0.52f32;
            for k in 0..LOBES {
                let mut d = (theta - lobe_angle[k]).abs();
                if d > std::f32::consts::PI {
                    d = std::f32::consts::TAU - d;
                }
                let falloff = (-(d * d) / (2.0 * lobe_width[k] * lobe_width[k])).exp();
                rim += lobe_reach[k] * 0.42 * falloff;
            }

            // Soft edge inside the rim rather than a hard cut, so the decal's own alpha blend has
            // something to work with and the stain does not show a polygon edge.
            let alpha = if r >= rim {
                0.0
            } else {
                let t = (1.0 - r / rim).clamp(0.0, 1.0);
                // Squared, so the centre stays solid and only the last of the radius fades.
                (t * t * 3.2).min(1.0)
            };

            let i = (y * n + x) * 4;
            data[i] = 255;
            data[i + 1] = 255;
            data[i + 2] = 255;
            data[i + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    Image::new(
        Extent3d { width: SPLAT_SIZE, height: SPLAT_SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        // sRGB, because the colour channels are sampled as colour. The alpha channel is linear in
        // either format, and it is the alpha that carries the shape.
        TextureFormat::Rgba8UnormSrgb,
        // The GPU samples it and nothing on the CPU reads it back, but `MAIN_WORLD` is kept so a
        // consumer can inspect or replace the image; dropping it would free the pixels after upload
        // and turn any such attempt into a silent blank.
        RenderAssetUsages::default(),
    )
}

/// Build the four splat textures and their materials.
///
/// **Not a `MaterialPlugin` registration** — see the module docs. This only adds assets.
pub fn build_splats(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ForwardDecalMaterial<StandardMaterial>>>,
) {
    let mut image_handles = Vec::with_capacity(SPLAT_VARIANTS);
    let mut material_handles = Vec::with_capacity(SPLAT_VARIANTS);
    for variant in 0..SPLAT_VARIANTS as u32 {
        let image = images.add(splat_image(variant));
        material_handles.push(materials.add(ForwardDecalMaterial {
            base: StandardMaterial {
                base_color_texture: Some(image.clone()),
                // Dried arterial blood, dark enough to read against a lit floor.
                base_color: Color::srgb(0.30, 0.02, 0.02),
                perceptual_roughness: 0.30,
                // The extension forces `AlphaMode::Blend` for the decal pass; setting it here as well
                // keeps the base material honest about what it is if a consumer reuses it.
                alpha_mode: AlphaMode::Blend,
                ..default()
            },
            extension: ForwardDecalMaterialExt { depth_fade_factor: 1.0 },
        }));
        image_handles.push(image);
    }
    commands.insert_resource(SplatTextures { images: image_handles, materials: material_handles });
}

/// Spawn one stain as a forward decal.
///
/// **No `Mesh3d`** — `ForwardDecal`'s `on_add` hook inserts the 1×1 rectangle for us, but only if the
/// handle is still defaulted, so passing one here would suppress it.
///
/// Lifted `2 mm` off the plane it stained: the decal projects along local `−Y` and a coplanar quad
/// z-fights with the floor. Scaled to the stain's diameter, because the mesh is 1×1 and `radius` is a
/// radius.
pub fn spawn_stain(
    commands: &mut Commands,
    splats: &SplatTextures,
    stain: &Stain,
) -> Entity {
    commands
        .spawn((
            ForwardDecal,
            MeshMaterial3d(splats.material_for(stain)),
            Transform::from_translation(stain.at + Vec3::Y * 0.002)
                .with_scale(Vec3::splat(stain.radius * 2.0)),
        ))
        .id()
}

/// Marks a pool decal and remembers which pool it draws — an index into the caller's pool list.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolDecal(pub usize);

/// Spawn one [`Pool`] as a forward decal.
///
/// Differs from [`spawn_stain`] in exactly two ways, and both are deliberate: it lifts by `1.5 mm`
/// rather than `2 mm`, so a slick sits *under* the finer stains that seeded it instead of z-fighting
/// them; and it carries [`PoolDecal`] so [`update_pool_decals`] can keep its scale in step as the
/// pool grows. A stain never changes size, so it needs no tag.
///
/// Everything else is [`spawn_stain`]'s path verbatim — including **no `Mesh3d`**, because
/// `ForwardDecal`'s `on_add` hook supplies the 1×1 rectangle only while the handle is still
/// defaulted. The camera must carry `DepthPrepass`; see the module docs for why this cannot fix that.
pub fn spawn_pool(commands: &mut Commands, splats: &SplatTextures, index: usize, pool: &Pool) -> Entity {
    // `material_for` keys on a stain's seed; a pool carries the seed of the stain that formed it, so
    // the two agree about which splat variant this patch of floor wears.
    let as_stain = Stain { at: pool.at, radius: pool.radius, seed: pool.seed };
    commands
        .spawn((
            ForwardDecal,
            MeshMaterial3d(splats.material_for(&as_stain)),
            Transform::from_translation(pool.at + Vec3::Y * 0.0015)
                .with_scale(Vec3::splat(pool.radius * 2.0)),
            PoolDecal(index),
        ))
        .id()
}

/// Refresh every pool decal's scale from its pool's current radius — **diameter**, because the hook's
/// mesh is 1×1.
///
/// Takes an iterator rather than a `Query` so it serves both a normal system (`q.iter_mut()`) and an
/// exclusive one (`world.query::<…>().iter_mut(world)`); the examples drive their whole frame from an
/// exclusive system and could not call it otherwise.
///
/// An index past the end of `pools` is skipped rather than panicking, which is this crate's standing
/// rule for a resolved id: a decal left over from a cleared pool list must not take the process down.
pub fn update_pool_decals<'a>(
    pools: &[Pool],
    decals: impl Iterator<Item = (&'a PoolDecal, Mut<'a, Transform>)>,
) {
    for (tag, mut transform) in decals {
        let Some(pool) = pools.get(tag.0) else { continue };
        transform.scale = Vec3::splat(pool.radius * 2.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generated texture must be the right shape, and its alpha must actually carry a splat —
    /// solid in the middle, empty in the corners. A uniformly transparent image would render as
    /// nothing and look like a decal-setup problem.
    #[test]
    fn a_splat_is_solid_in_the_middle_and_empty_at_the_corners() {
        let img = splat_image(0);
        let n = SPLAT_SIZE as usize;
        assert_eq!(img.width(), SPLAT_SIZE);
        assert_eq!(img.height(), SPLAT_SIZE);
        let data = img.data.as_ref().expect("a generated image must carry its pixels");
        assert_eq!(data.len(), n * n * 4, "RGBA8 of {SPLAT_SIZE} squared");

        let alpha_at = |x: usize, y: usize| data[(y * n + x) * 4 + 3];
        assert_eq!(alpha_at(n / 2, n / 2), 255, "the centre of a splat must be opaque");
        for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
            assert_eq!(alpha_at(x, y), 0, "the corner at {x},{y} must be fully transparent");
        }
        let opaque = data.chunks(4).filter(|p| p[3] > 200).count();
        let clear = data.chunks(4).filter(|p| p[3] == 0).count();
        assert!(opaque > n * n / 8, "only {opaque} solid pixels — the splat is barely there");
        assert!(clear > n * n / 8, "only {clear} clear pixels — the splat fills the whole texture");
    }

    /// **The lobes must actually break the circle.** A splat that was radially symmetric would be an
    /// airbrushed dot, which is the thing the hashed lobes exist to avoid — so the rim's radius must
    /// vary with angle by a real margin.
    #[test]
    fn a_splat_is_not_a_circle() {
        let img = splat_image(1);
        let n = SPLAT_SIZE as usize;
        let data = img.data.as_ref().expect("pixels");
        let half = SPLAT_SIZE as f32 * 0.5;

        // Walk out along several rays and record where the alpha dies.
        let mut reach = Vec::new();
        for step in 0..16 {
            let theta = step as f32 / 16.0 * std::f32::consts::TAU;
            let (c, sn) = (theta.cos(), theta.sin());
            let mut last = 0.0f32;
            for r in 1..(n / 2) {
                let x = (half + c * r as f32) as usize;
                let y = (half + sn * r as f32) as usize;
                if x >= n || y >= n {
                    break;
                }
                if data[(y * n + x) * 4 + 3] > 8 {
                    last = r as f32;
                }
            }
            reach.push(last);
        }
        let (lo, hi) = reach.iter().fold((f32::MAX, 0.0f32), |(a, b), r| (a.min(*r), b.max(*r)));
        assert!(hi > 0.0, "no ray found any splat at all");
        assert!(
            hi - lo > 3.0,
            "the rim varied by only {:.1} pixels between angles ({lo} to {hi}) — that is a circle, \
             not a splat",
            hi - lo
        );
    }

    /// The four variants must differ, or there is no point generating four.
    #[test]
    fn the_variants_are_different_splats() {
        let images: Vec<Image> = (0..SPLAT_VARIANTS as u32).map(splat_image).collect();
        for i in 0..images.len() {
            for j in (i + 1)..images.len() {
                assert_ne!(
                    images[i].data, images[j].data,
                    "splat variants {i} and {j} are byte-identical"
                );
            }
        }
    }

    /// A generated splat must be the same bytes every run — the crate's determinism rule reaching even
    /// into the cosmetic half, so a recorded demo is frame-identical.
    #[test]
    fn a_splat_is_the_same_bytes_every_time() {
        for variant in 0..SPLAT_VARIANTS as u32 {
            assert_eq!(
                splat_image(variant).data,
                splat_image(variant).data,
                "variant {variant} generated differently the second time"
            );
        }
    }
}
