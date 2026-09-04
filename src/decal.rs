//! **Stains, as entities.** The cosmetic half of blood on the floor.
//!
//! Entirely behind the `vfx` feature, and it is the *only* half that is optional: **where blood lands
//! is core.** `crate::bloodstain::stain::stains` is deterministic and available headless, because on the
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
//! A mask is now rasterised from the stain's own [`StainShape`] (`crate::bloodstain::stain::rasterise`):
//! aspect from the impact angle, spines from the Weber number, satellites past the splash threshold.
//! Two stains look the same only when two impacts *were* the same. The crate still ships **no asset
//! files** — a consumer should not have to copy a texture out of a dependency to get blood.
//!
//! # The cache, and why it has a ceiling
//!
//! A distinct material per stain specialises a render pipeline per stain, which is the cost the old
//! four-variant scheme existed to avoid. So masks are cached by a **quantised shape key**: stains
//! whose silhouettes are indistinguishable share one material, and the cache is capped. At the cap,
//! `crate::bloodstain::bag::pick` chooses among the masks already built — a shuffle bag whose minimum gap
//! between repeats is `n − 1` draws, rather than the fresh uniform draw that made `seed % 4` tile.

use std::sync::LazyLock;

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::pbr::decal::{ForwardDecal, ForwardDecalMaterial, ForwardDecalMaterialExt};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::bloodstain::spectral::Film;
use crate::bloodstain::stain::{StainShape, rasterise};
use crate::bloodstain::{Pool, Stain};

/// **The film a stain on a floor is**, and therefore its colour.
///
/// 1.5 mm of venous blood over a mid-grey substrate. **No blood colour is authored here**: the
/// thickness and the oxygen saturation go into `crate::bloodstain::spectral`, which puts Bosschaart's
/// whole-blood absorption and scattering tables (`doi:10.1007/s10103-013-1446-7`) through
/// Kubelka–Munk at 81 wavelengths and the CIE observer — the same optics that colour a wetmap texel
/// and a cross-section's muscle band, so blood on a floor and blood on a body cannot drift apart.
///
/// **Venous and 1.5 mm because that is what a floor stain is.** A pooled stain has stopped being a
/// droplet: it is deoxygenated by the time it settles, and `crate::bloodstain::stain` describes a stain's
/// *silhouette* rather than its depth — there is no thickness in the morphology to read, so the
/// number is stated here. At 1.5 mm the film is close enough to blood's own semi-infinite
/// reflectance that the substrate barely reaches the answer, which is why an unknown floor is
/// mid-grey rather than a dial.
const FLOOR_FILM: Film = Film { thickness_mm: 1.5, so2: crate::bloodstain::SO2_VENOUS, substrate: 0.5 };

/// The stain colour, computed once — 81 wavelengths of Kubelka–Munk per material would be paid per
/// silhouette, and the answer is a function of a constant.
static FLOOR_SRGB: LazyLock<[f32; 3]> = LazyLock::new(|| crate::bloodstain::spectral::srgb(&FLOOR_FILM));

/// The colour every stain and pool material carries. See [`FLOOR_FILM`].
fn floor_colour() -> Color {
    let [r, g, b] = *FLOOR_SRGB;
    Color::srgb(r, g, b)
}

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
    /// Draw ordinal for [`crate::bloodstain::pick`], advanced once per reuse.
    ///
    /// **The one piece of mutable state here, and it is a counter rather than a cursor into a
    /// shuffled list** — `pick` derives its whole permutation from the ordinal, so a replay
    /// reproduces the choice by construction and ECS iteration order cannot perturb which mask a
    /// stain wears.
    epoch: u32,
}

/// Quantise a silhouette into a cache key.
///
/// Aspect ratio to 32 steps, spine count exactly, direction to 16 sectors. Two stains that agree on
/// all three are indistinguishable at 64 pixels, so they share a mask; anything coarser starts
/// merging shapes a player can tell apart, and anything finer builds pipelines for differences
/// nobody can see.
///
/// **Satellites are deliberately not in the key**, because they are not in the mask — see
/// [`mask_image`] for why a decal draws the deposit rather than the whole spray. Keying on a field
/// the image is not a function of would fill the cache's 32 slots with identical textures and make a
/// floor start repeating for no visible reason.
pub fn mask_key(shape: &StainShape) -> u32 {
    let aspect = if shape.major > 0.0 { (shape.minor / shape.major).clamp(0.0, 1.0) } else { 1.0 };
    let a = (aspect * 31.0).round() as u32;
    let sector = {
        let (x, y) = (shape.direction[0], shape.direction[1]);
        let ang = y.atan2(x);
        let t = (ang + std::f32::consts::PI) / std::f32::consts::TAU;
        (t.clamp(0.0, 0.999) * 16.0) as u32
    };
    a | ((shape.spines as u32) << 5) | (sector << 10)
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
    /// [`crate::bloodstain::pick`] from the draw ordinal. Two runs that stain the same floor wear the same
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
                    // Fresh blood, coloured by [`FLOOR_FILM`] rather than authored. A consumer
                    // walking the drying timeline sets `base_color` and `perceptual_roughness` from
                    // `crate::bloodstain::dry::appearance` instead — the mask carries shape only, so one
                    // texture serves every age.
                    base_color: floor_colour(),
                    perceptual_roughness: crate::bloodstain::BloodSettings::default().wet_roughness,
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
        let index = crate::bloodstain::pick(self.epoch, key, n, 2) as usize;
        self.epoch = self.epoch.wrapping_add(1);
        match self.masks.get(index) {
            Some(m) => m.material.clone(),
            // Unreachable while `pick` honours its `n`, and still not a panic: a resolved index that
            // went out of range must not take the process down. This crate's standing rule.
            None => self.masks[0].material.clone(),
        }
    }
}

/// **A blood mask, rasterised from a silhouette — the deposit, filled, with a soft edge.**
///
/// The alpha channel carries the shape and the colour channels are left white, because the stain's
/// colour comes from the material's `base_color` — one texture then serves any blood colour a consumer
/// wants, and a paler, darker or older blood needs no new asset.
///
/// # Why the satellites are dropped here, and why they were the "donut"
///
/// **Measured, on the mask this function used to build.** `crate::bloodstain::stain::rasterise` draws the
/// whole forensic silhouette — the elliptical deposit, its rim spines, *and* the ring of detached
/// satellite droplets — and to fit the furthest satellite inside the texture it shrinks the deposit
/// by `1 + max_spine_reach + 0.47`, which is about `2.15×`. At the impact energies a wound's droplets
/// actually arrive with, `stain_shape` saturates **both** `spines` and `satellites` at
/// `SPINE_MAX = 24` (a 4 mm droplet at 6 m/s is `We ≈ 2500`, `K ≈ 290` against a splash threshold of
/// 57.7), so *every* floor decal came out as a small central deposit inside a detached ring of
/// satellites with a near-transparent moat between them. Measured on the 90°/6 m/s fixture, as the
/// mean alpha over sixteen radial bins:
/// `[255, 255, 255, 254, 229, 167, 105, 57, 38, 62, 115, 181, 174, 95, 28, 2]` — a trough of **38**
/// at half the radius and a bright ring of **181** at 0.69 of it. That is a donut, and it is what
/// `capture_carnage` showed on its floor.
///
/// The deposit is what a decal is *for* — [`spawn_stain`] scales the quad to `Stain::radius`, which
/// is the deposit's own radius from `crate::bloodstain::stain::stain_radius`, so a mask whose body filled
/// less than half its width drew the stain at less than half the size it was placed at and then put
/// a ring of specks where the rim should have been. So the mask is rasterised from the silhouette
/// with `satellites: 0`: the body then fills the texture (`extent = (1 + max_spine) · 1.04`), the
/// spines still break the circle, and the coverage falls monotonically from an opaque centre to zero
/// at the rim — a filled disc with a soft edge.
///
/// **The silhouette is not authored away.** Aspect still comes from the impact angle and the spines
/// from the Weber number, so two stains still look the same only when two impacts were the same; and
/// the satellites are still in the model for anything that draws the *spray* — a caller wanting them
/// on the floor spawns them as their own stains, at their own radii, which is what they are.
///
/// Pure: the same silhouette is the same bytes every run.
pub fn mask_image(shape: &StainShape) -> Image {
    let n = MASK_SIZE as usize;
    let mut coverage = vec![0u8; n * n];
    let deposit = StainShape { satellites: 0, ..*shape };
    // `rasterise` refuses a wrong-sized buffer rather than half-filling it; the buffer above is built
    // from the same constant, so the refusal is unreachable — and if it ever fires, an all-clear mask
    // is the honest result of "no coverage was computed" rather than garbage uploaded to the GPU.
    let _ = rasterise(&deposit, MASK_SIZE, &mut coverage);

    let mut data = vec![0u8; n * n * 4];
    for (i, &c) in coverage.iter().enumerate() {
        // `get_mut` rather than an index: `data` is four times `coverage` by construction, and a
        // panicking index in library code is this crate's one standing prohibition.
        if let Some(px) = data.get_mut(i * 4..i * 4 + 4) {
            px.copy_from_slice(&[255, 255, 255, c]);
        }
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
/// `crate::bloodstain::stain::impact_at_plane` then `stain_shape` — and keeping them separate is what let the
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
    use crate::bloodstain::stain::{Impact, stain_shape};
    use crate::bloodstain::BloodSettings;

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

    /// **A floor decal is a filled disc, not a donut — and this is the test that would have caught
    /// it.**
    ///
    /// The mask used to be the whole forensic silhouette, satellites included, and `rasterise`
    /// shrinks the deposit to make room for the furthest one. At the impact energies a wound's
    /// droplets arrive with, `stain_shape` saturates both `spines` and `satellites` at 24, so every
    /// decal came out as a small deposit inside a detached ring. Checked against the pre-fix mask,
    /// this fixture's mean alpha per bin was
    /// `[255, 255, 255, 254, 229, 167, 105, 57, 38, 62, 115, 181, 174, 95, 28, 2]`: a trough of 38
    /// at half the radius, then a ring of 181, rising by **66** in one step. A profile that rises
    /// outward *is* a ring, so the claim is that it never rises.
    #[test]
    fn a_mask_is_a_filled_disc_and_not_a_ring() {
        let shape = shape_at(90.0, 6.0, 5);
        assert!(shape.satellites > 0, "the fixture must be an impact that throws satellites");
        let img = mask_image(&shape);
        let n = MASK_SIZE as usize;
        let data = img.data.as_ref().expect("pixels");
        let half = MASK_SIZE as f32 * 0.5;

        // Mean alpha per radial bin, sixteen bins over the texture's half-width.
        const BINS: usize = 16;
        let mut sum = [0u32; BINS];
        let mut count = [0u32; BINS];
        for y in 0..n {
            for x in 0..n {
                let (dx, dy) = (x as f32 + 0.5 - half, y as f32 + 0.5 - half);
                let r = (dx * dx + dy * dy).sqrt() / half;
                let bin = ((r * BINS as f32) as usize).min(BINS - 1);
                let a = data.get((y * n + x) * 4 + 3).copied().unwrap_or(0);
                sum[bin] += a as u32;
                count[bin] += 1;
            }
        }
        let mean: Vec<u32> = (0..BINS).map(|i| sum[i] / count[i].max(1)).collect();

        assert_eq!(mean.first().copied(), Some(255), "the middle of a stain is not opaque: {mean:?}");
        // Non-increasing outward, with four code values of slack for the spines' own bumps. The old
        // mask broke this by 66 at bin 11.
        for i in 1..BINS {
            assert!(
                mean[i] <= mean[i - 1] + 4,
                "the alpha rose outward at bin {i} ({} -> {}) — that is a ring: {mean:?}",
                mean[i - 1],
                mean[i]
            );
        }
        // And it is a *disc*: the deposit fills the texture rather than sitting in the middle of it,
        // because the quad is scaled to the stain's own radius.
        assert!(mean[7] > 32, "the disc covers less than half its own radius: {mean:?}");
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
