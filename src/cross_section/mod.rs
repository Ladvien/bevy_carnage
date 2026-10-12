#![doc = include_str!("../../docs/cross_section.md")]

mod depth;
mod layers;
mod strip;

pub use depth::{Scale, SkinPlane, annotate_cap, depth_below_skin, uv1_at, uv1_at_core};
pub use layers::{Layer, Layers, Region};
pub use strip::{Band, Strip, strip, texel_at, texel_mean};
pub(crate) use strip::value_noise;
use strip::{linear_to_srgb, srgb_to_linear};


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
        let albedo = images.add(image_of(s.width, s.height, s.albedo, TextureFormat::Rgba8UnormSrgb, true));
        let rough = images.add(image_of(s.width, s.height, s.rough, TextureFormat::Rgba8Unorm, false));
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

/// A strip image: clamped along depth, repeating along the tile, trilinearly filtered over a full
/// mip chain.
///
/// **The mip chain is what keeps a cut face from sparkling.** A strip draws its grain at physical
/// size — fascicles 0.7 mm across, Haversian canals 0.35 mm apart — so on a cap a few hundred pixels
/// wide one screen pixel covers several features. Sampled from level 0 alone, each pixel lands on a
/// different one, and the cap reads as salt and pepper that crawls as the piece turns. The levels are
/// box-filtered on the CPU, colour in linear light, so the bands keep their mean colour as the grain
/// drops below a pixel. The digest is over level 0, the strip itself, and does not see them.
fn image_of(width: u32, height: u32, data: Vec<u8>, format: TextureFormat, srgb: bool) -> Image {
    let (data, levels) = mip_chain(width, height, data, srgb);
    // `new_uninit` and then the bytes, because `Image::new` debug-asserts that the data is exactly one
    // level's worth, and these carry the whole chain.
    let mut image = Image::new_uninit(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        format,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..Default::default()
    });
    image
}

/// **Level 0 followed by every smaller level down to 1×1**, RGBA8, in the layer-major order Bevy
/// uploads. Returns the bytes and the level count. Each level averages the 2×2 block above it (the
/// last row or column repeated when a size is odd); `srgb` averages the colour channels in linear
/// light, so a red-and-ivory grain fades to its mean rather than a darker colour.
fn mip_chain(width: u32, height: u32, base: Vec<u8>, srgb: bool) -> (Vec<u8>, u32) {
    let decode: [f32; 256] = std::array::from_fn(|b| if srgb { srgb_to_linear(b as f32 / 255.0) } else { b as f32 / 255.0 });
    let (mut w, mut h) = (width.max(1) as usize, height.max(1) as usize);
    if base.len() != w * h * 4 {
        return (base, 1);
    }
    let mut out = base;
    let mut start = 0;
    let mut levels = 1;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = Vec::with_capacity(nw * nh * 4);
        for y in 0..nh {
            for x in 0..nw {
                let (x0, y0) = ((2 * x).min(w - 1), (2 * y).min(h - 1));
                let (x1, y1) = ((2 * x + 1).min(w - 1), (2 * y + 1).min(h - 1));
                for c in 0..4 {
                    let at = |px: usize, py: usize| out.get(start + (py * w + px) * 4 + c).copied().unwrap_or(0);
                    let quad = [at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1)];
                    next.push(if c < 3 && srgb {
                        let l = quad.iter().map(|&b| decode[b as usize]).sum::<f32>() * 0.25;
                        (linear_to_srgb(l) * 255.0).round().clamp(0.0, 255.0) as u8
                    } else {
                        ((quad.iter().map(|&b| b as u32).sum::<u32>() + 2) / 4) as u8
                    });
                }
            }
        }
        start = out.len();
        out.extend_from_slice(&next);
        (w, h) = (nw, nh);
        levels += 1;
    }
    (out, levels)
}
