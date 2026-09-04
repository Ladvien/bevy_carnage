#![doc = include_str!("../../docs/cross_section.md")]

mod depth;
mod layers;
mod strip;

pub use depth::{Scale, SkinPlane, annotate_cap, depth_below_skin, uv1_at, uv1_at_core};
pub use layers::{Layer, Layers, Region};
pub use strip::{Band, Strip, strip, texel_at};


use bevy::app::{App, Plugin, Startup};
use bevy::asset::{Assets, Handle, RenderAssetUsages};
use bevy::ecs::resource::Resource;
use bevy::ecs::schedule::{IntoScheduleConfigs, SystemSet};
use bevy::ecs::system::{Res, ResMut};
use bevy::image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::log::warn;
use bevy::mesh::UvChannel;
use bevy::pbr::StandardMaterial;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// **Everything this crate registers, in one set.** One system: the strip bake, on `Startup`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CrossSectionSystems;

/// **The bake dials.** Which thickness row each region uses, how big the strips are, and the seed.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct CrossSectionSettings {
    /// Per-region layer thicknesses, in [`Region::ALL`] order. Defaults to [`Layers::for_region`].
    pub layers: [Layers; 3],
    /// Strip columns — the depth axis. `512`.
    pub width: u32,
    /// Strip rows — the along axis, which tiles over `scale.tile_units`. `512`, which over the
    /// default 50 mm tile is 10 rows per millimetre against the depth axis' 12 columns: near enough
    /// square that a 2 mm lobule is round. Square texels want `width · tile_mm / span_mm`.
    pub height: u32,
    /// Noise seed. Two apps with one seed bake one strip.
    pub seed: u32,
    /// How cap positions map to the strip. See [`Scale`].
    pub scale: Scale,
}

impl Default for CrossSectionSettings {
    fn default() -> Self {
        Self {
            layers: [
                Layers::for_region(Region::Limb),
                Layers::for_region(Region::Torso),
                Layers::for_region(Region::Head),
            ],
            width: 512,
            height: 512,
            seed: 0xC0FF_EE00,
            scale: Scale::default(),
        }
    }
}

impl CrossSectionSettings {
    /// The thickness row for `region`.
    pub fn layers(&self, region: Region) -> &Layers {
        match region {
            Region::Limb => &self.layers[0],
            Region::Torso => &self.layers[1],
            Region::Head => &self.layers[2],
        }
    }
}

/// One region's baked strip, as assets.
#[derive(Clone, Debug)]
pub struct RegionStrip {
    /// The `Rgba8UnormSrgb` albedo strip.
    pub albedo: Handle<Image>,
    /// The `Rgba8Unorm` metallic-roughness strip (`G` roughness, `B` metallic).
    pub rough: Handle<Image>,
    /// A `StandardMaterial` sampling both through `UvChannel::Uv1`. `None` when the app has no
    /// `Assets<StandardMaterial>` (a headless bake); the images are still there.
    pub material: Option<Handle<StandardMaterial>>,
    /// FNV-1a over the strip's pixels — the same value [`Strip::digest`] reports.
    pub digest: u64,
}

/// **The baked strips, one per region**, filled on `Startup` by [`CrossSectionPlugin`].
#[derive(Resource, Clone, Debug, Default)]
pub struct CrossSectionAtlas {
    strips: Vec<(Region, RegionStrip)>,
}

impl CrossSectionAtlas {
    /// The strip for `region`, once baked.
    pub fn get(&self, region: Region) -> Option<&RegionStrip> {
        self.strips.iter().find(|(r, _)| *r == region).map(|(_, s)| s)
    }

    /// The cut-face material for `region` — the one to put on a cap this crate has annotated.
    pub fn material(&self, region: Region) -> Option<Handle<StandardMaterial>> {
        self.get(region).and_then(|s| s.material.clone())
    }

    /// Every baked region.
    pub fn regions(&self) -> impl Iterator<Item = Region> + '_ {
        self.strips.iter().map(|(r, _)| *r)
    }
}

/// **Bakes one strip per region on `Startup` and hands back materials that read them.**
///
/// Adds [`CrossSectionSettings`] (if absent) and [`CrossSectionAtlas`]. A caller that wants its own
/// thickness rows inserts the settings *before* the plugin; `init_resource` then no-ops.
pub struct CrossSectionPlugin;

impl Plugin for CrossSectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CrossSectionSettings>()
            .init_resource::<CrossSectionAtlas>()
            .add_systems(Startup, bake_strips.in_set(CrossSectionSystems));
    }
}

/// Build the strip images and materials. Both asset stores are optional: a headless app without a
/// PBR plugin still gets the images and the digests, and one without `ImagePlugin` gets a warning
/// and an empty atlas rather than a crash — a missing `Res<T>` panics a system in Bevy 0.19 rather
/// than skipping it, so "optional" has to be spelled `Option`. The digests are one `strip` call
/// away regardless.
fn bake_strips(
    settings: Res<CrossSectionSettings>,
    mut atlas: ResMut<CrossSectionAtlas>,
    images: Option<ResMut<Assets<Image>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let mut images = images;
    let mut materials = materials;
    let tile_mm = settings.scale.tile_units * settings.scale.mm_per_unit;
    atlas.strips.clear();
    let Some(images) = images.as_mut() else {
        warn!("cross_section: no `Assets<Image>` to bake strips into; add `ImagePlugin` or call `strip` directly");
        return;
    };
    for region in Region::ALL {
        let s = strip(settings.layers(region), settings.width, settings.height, tile_mm, settings.seed);
        let digest = s.digest();
        let albedo = images.add(image_of(s.width, s.height, s.albedo, TextureFormat::Rgba8UnormSrgb));
        let rough = images.add(image_of(s.width, s.height, s.rough, TextureFormat::Rgba8Unorm));
        let material = materials.as_mut().map(|m| {
            m.add(StandardMaterial {
                base_color_texture: Some(albedo.clone()),
                base_color_channel: UvChannel::Uv1,
                metallic_roughness_texture: Some(rough.clone()),
                metallic_roughness_channel: UvChannel::Uv1,
                perceptual_roughness: 1.0,
                metallic: 0.0,
                ..Default::default()
            })
        });
        atlas.strips.push((region, RegionStrip { albedo, rough, material, digest }));
    }
}

/// A strip image: clamped along depth, repeating along the tile, linearly filtered.
fn image_of(width: u32, height: u32, data: Vec<u8>, format: TextureFormat) -> Image {
    let mut image = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..Default::default()
    });
    image
}
