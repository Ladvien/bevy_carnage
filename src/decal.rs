//! **Stains, as entities.** The cosmetic half of blood on the floor.
//!
//! Entirely behind the `vfx` feature, and it is the *only* half that is optional: **where blood lands
//! is core.** `bloodstain::stain::stains` is deterministic and available headless, because on the
//! consuming side a blood pool's position feeds simulation — a mycelium colony reads pools as
//! chemoattractant sources. So this module turns a [`Stain`] into something visible and does nothing
//! else. It never decides where one is.
//!
//! # Two Bevy facts this module is shaped around, both verified in 0.19's source
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
//! # Masks derived from the impact, not four baked variants
//!
//! **This replaced `SPLAT_VARIANTS = 4` and `splat_image(variant)`, and the arithmetic is why.** With
//! four textures chosen by `seed % 4`, the probability that four consecutive stains contain a repeat
//! is `1 − 4!/4⁴ = 90.6 %` — so the *expected* first visible repeat is the fourth stain, and a floor
//! reads as tiled almost immediately. Authoring more textures moves the number without changing the
//! shape of the problem.
//!
//! A mask is now rasterised from the stain's own [`StainShape`] (`bloodstain::stain::rasterise`):
//! aspect from the impact angle, spines from the Weber number, satellites past the splash threshold.
//! Two stains look the same only when two impacts *were* the same. The crate still ships **no asset
//! files** — a consumer should not have to copy a texture out of a dependency to get blood.
//!
//! # The cache, and why it has a ceiling
//!
//! A distinct material per stain specialises a render pipeline per stain, which is the cost the old
//! four-variant scheme existed to avoid. So masks are cached by a **quantised shape key**: stains
//! whose silhouettes are indistinguishable share one material, and the cache is capped. At the cap,
//! `bloodstain::bag::pick` chooses among the masks already built — a shuffle bag whose minimum gap
//! between repeats is `n − 1` draws, rather than the fresh uniform draw that made `seed % 4` tile.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::pbr::decal::{ForwardDecal, ForwardDecalMaterial, ForwardDecalMaterialExt};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use bloodstain::stain::{StainShape, rasterise};
use bloodstain::{Pool, Stain};

/// Edge length of a generated mask, in pixels.
///
/// 64 is enough for a stain that reads at a metre and is 16 KB of RGBA — small enough that generating
/// one per distinct silhouette is free, and large enough that the spines are not visibly stepped.
const MASK_SIZE: u32 = 64;

/// **Live masks, and therefore live render pipelines, before the cache starts reusing.**
///
/// Each entry is one material, and a material is one pipeline specialisation. 32 distinct silhouettes
/// is far past the point where a floor stops reading as tiled — the old scheme managed with four —
/// while staying a number a frame budget can carry.
pub const MAX_MASKS: usize = 32;

/// One cached mask: the silhouette it was rasterised from, its image, and its material.
#[derive(Debug, Clone)]
pub struct StainMask {
    /// The quantised key this mask answers for. Two stains with the same key share it.
    pub key: u32,
    /// The rasterised coverage texture.
    pub image: Handle<Image>,
    /// The decal material over it.
    pub material: Handle<ForwardDecalMaterial<StandardMaterial>>,
}

/// **The mask cache.** Built on demand, capped, and shared.
///
/// A `Resource` rather than a local, because a material handle has to outlive the system that made it
/// and every stain in a level wants the same handles.
#[derive(Resource, Debug, Default)]
pub struct StainMasks {
    masks: Vec<StainMask>,
    /// Draw ordinal for [`bloodstain::pick`], advanced once per reuse.
    ///
    /// **The one piece of mutable state here, and it is a counter rather than a cursor into a
    /// shuffled list** — `pick` derives its whole permutation from the ordinal, so a replay
    /// reproduces the choice by construction and ECS iteration order cannot perturb which mask a
    /// stain wears.
    epoch: u32,
}

/// Quantise a silhouette into a cache key.
///
/// Aspect ratio to 32 steps, spine count exactly, satellites exactly, direction to 16 sectors. Two
/// stains that agree on all four are indistinguishable at 64 pixels, so they share a mask; anything
/// coarser starts merging shapes a player can tell apart, and anything finer builds pipelines for
/// differences nobody can see.
pub fn mask_key(shape: &StainShape) -> u32 {
    let aspect = if shape.major > 0.0 { (shape.minor / shape.major).clamp(0.0, 1.0) } else { 1.0 };
    let a = (aspect * 31.0).round() as u32;
    let sector = {
        let (x, y) = (shape.direction[0], shape.direction[1]);
        let ang = y.atan2(x);
        let t = (ang + std::f32::consts::PI) / std::f32::consts::TAU;
        (t.clamp(0.0, 0.999) * 16.0) as u32
    };
    a | ((shape.spines as u32) << 5) | ((shape.satellites as u32) << 10) | (sector << 15)
}

impl StainMasks {
    /// How many masks are live. One material, and therefore one pipeline specialisation, each.
    pub fn len(&self) -> usize {
        self.masks.len()
    }

    /// Is the cache empty? (Clippy asks for this beside `len`.)
    pub fn is_empty(&self) -> bool {
        self.masks.is_empty()
    }

    /// **The material for one stain silhouette**, building it if the cache has room and reusing one
    /// if it does not.
    ///
    /// Deterministic in both branches: a hit is keyed by the shape, and a miss at the cap is chosen by
    /// [`bloodstain::pick`] from the draw ordinal. Two runs that stain the same floor wear the same
    /// masks in the same order — which is what makes a recorded demo frame-identical.
    ///
    /// **This is not a fallback path.** There is one function that answers "which material", and the
    /// cap is an input to it rather than a second implementation behind a condition.
    pub fn material_for(
        &mut self,
        shape: &StainShape,
        images: &mut Assets<Image>,
        materials: &mut Assets<ForwardDecalMaterial<StandardMaterial>>,
    ) -> Handle<ForwardDecalMaterial<StandardMaterial>> {
        let key = mask_key(shape);
        if let Some(hit) = self.masks.iter().find(|m| m.key == key) {
            return hit.material.clone();
        }
        if self.masks.len() < MAX_MASKS {
            let image = images.add(mask_image(shape));
            let material = materials.add(ForwardDecalMaterial {
                base: StandardMaterial {
                    base_color_texture: Some(image.clone()),
                    // Fresh blood. A consumer walking the drying timeline sets `base_color` and
                    // `perceptual_roughness` from `bloodstain::dry::appearance` instead — the mask
                    // carries shape only, so one texture serves every age.
                    base_color: Color::srgb(0.30, 0.02, 0.02),
                    perceptual_roughness: bloodstain::BloodSettings::default().wet_roughness,
                    // The extension forces `AlphaMode::Blend` for the decal pass; setting it here as
                    // well keeps the base material honest about what it is if a consumer reuses it.
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                },
                extension: ForwardDecalMaterialExt { depth_fade_factor: 1.0 },
            });
            self.masks.push(StainMask { key, image, material: material.clone() });
            return material;
        }
        // At the cap: reuse, chosen by the shuffle bag rather than by a modulo of the seed.
        let n = self.masks.len() as u32;
        let index = bloodstain::pick(self.epoch, key, n, 2) as usize;
        self.epoch = self.epoch.wrapping_add(1);
        match self.masks.get(index) {
            Some(m) => m.material.clone(),
            // Unreachable while `pick` honours its `n`, and still not a panic: a resolved index that
            // went out of range must not take the process down. This crate's standing rule.
            None => self.masks[0].material.clone(),
        }
    }
}

/// **A blood mask, rasterised from a silhouette.**
///
/// The alpha channel carries the shape and the colour channels are left white, because the stain's
/// colour comes from the material's `base_color` — one texture then serves any blood colour a consumer
/// wants, and a paler, darker or older blood needs no new asset.
///
/// Pure: the same silhouette is the same bytes every run.
pub fn mask_image(shape: &StainShape) -> Image {
    let n = MASK_SIZE as usize;
    let mut coverage = vec![0u8; n * n];
    // `rasterise` refuses a wrong-sized buffer rather than half-filling it; the buffer above is built
    // from the same constant, so the refusal is unreachable — and if it ever fires, an all-clear mask
    // is the honest result of "no coverage was computed" rather than garbage uploaded to the GPU.
    let _ = rasterise(shape, MASK_SIZE, &mut coverage);

    let mut data = vec![0u8; n * n * 4];
    for (i, &c) in coverage.iter().enumerate() {
        data[i * 4] = 255;
        data[i * 4 + 1] = 255;
        data[i * 4 + 2] = 255;
        data[i * 4 + 3] = c;
    }

    Image::new(
        Extent3d { width: MASK_SIZE, height: MASK_SIZE, depth_or_array_layers: 1 },
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

/// Spawn one stain as a forward decal.
///
/// **No `Mesh3d`** — `ForwardDecal`'s `on_add` hook inserts the 1×1 rectangle for us, but only if the
/// handle is still defaulted, so passing one here would suppress it.
///
/// Lifted `2 mm` off the plane it stained: the decal projects along local `−Y` and a coplanar quad
/// z-fights with the floor. Scaled to the stain's diameter, because the mesh is 1×1 and `radius` is a
/// radius.
///
/// **Takes the silhouette as well as the stain**, and that is the `0.2.0` signature change: a stain
/// says *where* and *how wide*, a [`StainShape`] says *what shape*. The two come from one droplet —
/// `bloodstain::stain::impact_at_plane` then `stain_shape` — and keeping them separate is what let the
/// placement stay frozen while the morphology became derived.
pub fn spawn_stain(
    commands: &mut Commands,
    masks: &mut StainMasks,
    images: &mut Assets<Image>,
    materials: &mut Assets<ForwardDecalMaterial<StandardMaterial>>,
    stain: &Stain,
    shape: &StainShape,
) -> Entity {
    let material = masks.material_for(shape, images, materials);
    let at = Vec3::new(stain.at[0], stain.at[1], stain.at[2]);
    commands
        .spawn((
            ForwardDecal,
            MeshMaterial3d(material),
            Transform::from_translation(at + Vec3::Y * 0.002)
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
pub fn spawn_pool(
    commands: &mut Commands,
    masks: &mut StainMasks,
    images: &mut Assets<Image>,
    materials: &mut Assets<ForwardDecalMaterial<StandardMaterial>>,
    index: usize,
    pool: &Pool,
    shape: &StainShape,
) -> Entity {
    let material = masks.material_for(shape, images, materials);
    let at = Vec3::new(pool.at[0], pool.at[1], pool.at[2]);
    commands
        .spawn((
            ForwardDecal,
            MeshMaterial3d(material),
            Transform::from_translation(at + Vec3::Y * 0.0015)
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
    use bloodstain::stain::{Impact, stain_shape};
    use bloodstain::BloodSettings;

    fn shape_at(deg: f32, speed: f32, seed: u32) -> StainShape {
        let s = BloodSettings::default();
        stain_shape(
            &Impact {
                speed,
                diameter: 0.004,
                angle_rad: deg.to_radians(),
                roughness: s.substrate_roughness,
                travel: [1.0, 0.0],
            },
            &s,
            seed,
        )
    }

    /// The generated texture must be the right shape, and its alpha must actually carry a splat —
    /// solid in the middle, empty in the corners. A uniformly transparent image would render as
    /// nothing and look like a decal-setup problem.
    #[test]
    fn a_mask_is_solid_in_the_middle_and_empty_at_the_corners() {
        let img = mask_image(&shape_at(90.0, 6.0, 0));
        let n = MASK_SIZE as usize;
        assert_eq!(img.width(), MASK_SIZE);
        assert_eq!(img.height(), MASK_SIZE);
        let data = img.data.as_ref().expect("a generated image must carry its pixels");
        assert_eq!(data.len(), n * n * 4, "RGBA8 of {MASK_SIZE} squared");

        let alpha_at = |x: usize, y: usize| data[(y * n + x) * 4 + 3];
        assert_eq!(alpha_at(n / 2, n / 2), 255, "the centre of a stain must be opaque");
        for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
            assert_eq!(alpha_at(x, y), 0, "the corner at {x},{y} must be fully transparent");
        }
        let opaque = data.chunks(4).filter(|p| p[3] > 200).count();
        let clear = data.chunks(4).filter(|p| p[3] == 0).count();
        assert!(opaque > n * n / 16, "only {opaque} solid pixels — the stain is barely there");
        assert!(clear > n * n / 8, "only {clear} clear pixels — the stain fills the whole texture");
    }

    /// **The spines must actually break the circle.** A mask that was radially symmetric would be an
    /// airbrushed dot, which is the thing the derived morphology exists to avoid.
    #[test]
    fn a_mask_is_not_a_circle() {
        let img = mask_image(&shape_at(90.0, 6.0, 1));
        let n = MASK_SIZE as usize;
        let data = img.data.as_ref().expect("pixels");
        let half = MASK_SIZE as f32 * 0.5;

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
        assert!(hi > 0.0, "no ray found any stain at all");
        assert!(
            hi - lo > 2.0,
            "the rim varied by only {:.1} pixels between angles ({lo} to {hi}) — that is a circle, \
             not a stain",
            hi - lo
        );
    }

    /// **Different impacts are different masks, and the same impact is the same mask.** That pair is
    /// the whole replacement for `seed % 4`: variety comes from the physics, and reproducibility comes
    /// from the physics being a pure function.
    #[test]
    fn masks_differ_by_impact_and_repeat_only_when_the_impact_does() {
        let shallow = mask_image(&shape_at(15.0, 6.0, 7));
        let steep = mask_image(&shape_at(90.0, 6.0, 7));
        assert_ne!(shallow.data, steep.data, "a 15° and a 90° impact must not share a mask");

        assert_eq!(
            mask_image(&shape_at(40.0, 6.0, 3)).data,
            mask_image(&shape_at(40.0, 6.0, 3)).data,
            "the same silhouette generated differently the second time"
        );
    }

    /// The cache key merges what a player cannot tell apart and separates what they can.
    #[test]
    fn the_cache_key_separates_visibly_different_stains() {
        assert_ne!(
            mask_key(&shape_at(15.0, 6.0, 1)),
            mask_key(&shape_at(90.0, 6.0, 1)),
            "impact angle must reach the key — it is the aspect ratio"
        );
        assert_eq!(
            mask_key(&shape_at(45.0, 6.0, 1)),
            mask_key(&shape_at(45.0, 6.0, 1)),
            "the key must be a pure function of the silhouette"
        );
    }
}
