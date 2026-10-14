//! **Flesh, as a material.** The pixel half of the stack: everything the CPU kernels compute is a
//! byte in a canvas or a coordinate on a cap, and until 0.4.0 all of it was drawn by a plain
//! `StandardMaterial` — flat red caps, matte stains, fat that reads as painted plastic. This module
//! is one `ExtendedMaterial` over `StandardMaterial` that reads those same bytes and renders them the
//! way tissue and blood actually scatter and reflect light, on the forward path, on WebGL2.
//!
//! # What it does, and where each piece comes from
//!
//! **Subsurface scattering, pre-integrated.** Skin, fat and muscle are not opaque: light enters,
//! diffuses a few millimetres and leaves elsewhere, which is why a terminator on flesh is soft and
//! reddish and why fat glows. Jensen, Marschner, Levoy & Hanrahan, *"A practical model for
//! subsurface light transport"*, SIGGRAPH 2001, `doi:10.1145/383259.383319`, give the dipole
//! diffusion profile `R_d(r)` for a homogeneous slab from its reduced scattering and absorption
//! coefficients, and measure skin (`skin1`: `σ_s' = (0.74, 0.88, 1.01) mm⁻¹`, `σ_a = (0.032, 0.17,
//! 0.48) mm⁻¹`, `η = 1.3`). Penner's *Pre-Integrated Skin Shading* (SIGGRAPH 2011 *Advances in
//! Real-Time Rendering*, restated in the 2012 course) folds that profile into a two-dimensional
//! lookup over `(N·L, 1/r)`: integrate the profile around a ring on a sphere of radius `r` and the
//! result is what a Lambert term becomes once the light has diffused. [`sss_lut`] bakes that table
//! on the CPU, one row-block per [`Layer`], and the shader reads it with the surface curvature from
//! screen-space derivatives. Nothing screen-space, nothing that needs a prepass: one texture fetch.
//!
//! **Fat and muscle profiles are this crate's own.** Jensen measured skin; the corpus this was
//! written from tabulates no adipose or muscle coefficients. Fat is skin's scattering scaled up and
//! its absorption scaled down with the blue held — a translucent yellow. Muscle's absorption is
//! **derived, not authored**: whole-blood absorption from Bosschaart et al. 2014
//! (`doi:10.1007/s10103-013-1446-7`, the table in `crate::bloodstain::spectral`) at venous saturation,
//! band-averaged over the R, G and B thirds of the visible range, times a blood volume fraction of
//! 5 % (own), plus a small non-haem baseline (own). Cortical bone gets no wrap at all — the table
//! row is Lambert — and marrow is fat.
//!
//! **A wet clear coat.** Fresh blood is a liquid film, and a film has a specular layer of its own
//! above the diffuse colour beneath it. The wetness byte the wetmap writes (its `A` channel: fresh
//! `255` → dry `0`) drives `StandardMaterial`'s clearcoat strength per pixel, with a roughness a
//! caller dials. The film's *colour* is not recomputed here — `wetmap` already composites it on
//! the CPU from the same spectral model, and that CPU byte is what a golden hashes.
//!
//! **A film has a surface** (0.5.47). The film's depth, read from the wet map as a cubic B-spline,
//! is a height above what it lies on, drawn `FILM_RELIEF` (6) times its depth because a meniscus is
//! far narrower than a canvas texel, and that height tilts the shading normal and the coat's, by
//! screen-space derivatives so no tangents are needed (Mikkelsen 2010), never past 25°. A pool's edge
//! curves down to the floor and catches the light, a run stands up off the skin, and a pool at a
//! light's mirror angle shows a glint where it was one white sticker. Blood in a peeled crater lies
//! level in the hollow and a weave holds its blood between the yarns, so neither is lifted; the
//! relief fades out where a canvas texel is under a pixel. Wet films are at roughness `0.3`.
//!
//! **Blood on cloth.** Where the surface under the blood is a fabric rather than skin, the film
//! colour is composited on the GPU instead: [`blood_lut`] tabulates the Kubelka–Munk film
//! reflectance from `crate::bloodstain::spectral` over a black and over a white substrate, and the shader
//! interpolates by the fabric's own albedo per channel. That keeps the fabric's hue, which the CPU
//! composite (grey substrate) cannot. The *spread* of blood into cloth is not here either: it is
//! `crate::bloodstain::wick`'s Lucas–Washburn front driving the wetmap's spread rate, on the CPU.
//!
//! **A film's edge is a line.** The wet map is read as a cubic B-spline in four bilinear taps (Sigg &
//! Hadwiger, *GPU Gems 2*, ch. 20), not one bilinear tap, and its rim is drawn crisp at 4 % of the
//! canvas's film depth (`WET_EDGE` in `flesh.wgsl`, own number), sharpened over one pixel's change
//! in the smoothed amount. A bilinear read of a canvas whose texels are wet or not drew every rim as
//! the texels' staircase, soft and fringed; the spline's level set is a curve, so a run ends in a
//! round drop and a pool in a smooth line however far the canvas is magnified. Since 0.5.36.
//!
//! **Exposed tissue is wet with its own blood** ([`FleshParams::with_cut_film`]). In
//! [`FleshMode::Cap`] the tissue the strip draws, and in [`FleshMode::Canvas`] the tissue a flaymap
//! has bared, gets a film of blood over it, its full depth where the vessels are, in the muscle, and
//! a quarter of it over the fat at the rim, graded by the tissue's own depth below the skin, with the
//! film's clear coat. Off by default; the preset's broken pieces wear it since 0.5.38, where a bare fat
//! band read as cheese, and its skins' craters since 0.5.42, where a crater was a ring of bare fat.
//! A crater's graze wears 4 % of the film (`GRAZE_FILM` in the shader, since 0.5.53), ramped in over
//! the first 6 % of the layer span, about the skin's own depth: the ooze of a fresh abrasion, which is
//! raw and moist from the first moment. Bare, the graze was pale pink mottled with the fat's cream,
//! and round every crater it read as a pale halo.
//!
//! **A crater is lit through its mouth** (0.5.52). In [`FleshMode::Canvas`], a texel below the skin
//! looks out over the flaymap's own depth in eight directions, to 1, 3, 7 and 15 texels, for the
//! highest wall it sees each way, and every light term on it — diffuse, coat, subsurface — is
//! multiplied by `1 − mean(sin h)`: the cosine-weighted share of its view that clears the walls, the
//! same crossed-strings result a laceration's bed is lit by. The walls are measured in millimetres,
//! the depth from the canvas's `R` channel over [`FleshParams::cut`]`.y` (the layer span) and the
//! canvas's size on the body from screen-space derivatives of the position and the UV. A deep
//! crater's floor darkens and its rim keeps its red; drawn as open skin, a crater pooled with blood
//! was the same red to its middle whatever its depth. It leaves out light the walls throw back.
//!
//! # Determinism
//!
//! Both lookup tables are pure functions of their inputs, computed with `libm` so two machines bake
//! the same bytes, and [`lut_digest`] is frozen by a test. The shader is cosmetic and reads only
//! canvases the CPU owns; nothing here can reach a hash.
//!
//! # WebGL2
//!
//! **Verified, not assumed**: `examples/flesh.rs` built for `wasm32-unknown-unknown` with `bevy`'s
//! `webgl2` feature in place of the site's `webgpu`, loaded in headless Chromium over SwiftShader,
//! rendered the limb, the sphere and the sheet with the blood composited and no shader diagnostic
//! (2026-09-04, 0.4.0). Again after the flaymap moved to the extension and the authored skin was
//! kept (2026-10-12, 0.5.15): `flesh`, and `carnage_web`, which is the preset over a textured body
//! with a skin's base colour, metallic-roughness and normal textures all bound. Both ran in headless
//! Chromium on ANGLE over Metal, at WebGL2's sixteen texture units, and drew their wounds with no
//! shader or pipeline diagnostic. Again after the wet sheen was retuned (2026-10-12, 0.5.24, which
//! carries `AG-096`): `carnage_web`, built from the mirror's tip with `webgl2`, in Chrome on ANGLE over
//! Metal, took a shot, a slash, an arterial spurt and three shots to bone. It drew the wounds, the pools
//! and the broken body, and the console held nothing but Bevy's notices that its optional plugins need
//! storage buffers WebGL2 lacks. Again after the wet map became a B-spline with a crisp rim
//! (2026-10-13, 0.5.36, `AG-109`): `flesh` with `webgl2` for `webgpu`, in Firefox 157 on an Xvfb
//! display, whose WebGL2 is Mesa's llvmpipe (headless Firefox offered none): WebGL 2.0, GLSL ES
//! 3.00, the `Gl` backend; the limb, the sphere's crater and its run and the sheet's streaks drew,
//! and the console held only Bevy's notices about the software driver and the plugins that need what
//! WebGL2 lacks. Again after a film was given a surface (2026-10-13, 0.5.47, `AG-120`), whose shader
//! takes screen-space derivatives of the world position and the UV and samples the wet map five
//! times: the same build, browser and driver drew the sphere's run standing and shining, the limb and
//! the sheet, with the same notices and nothing else. The demo site itself stays WebGPU, like every
//! other module on it.
//!
//! The uniform block is sixteen-byte aligned, there are no storage buffers, and the extension adds
//! four textures and one sampler to `StandardMaterial`'s: since 0.5.54 the two tables are one image
//! ([`luts`]), because with them apart the material used fifteen of the sixteen sampled textures a
//! stage may have on WebGL2 and under WebGPU's default limits, and a camera with an
//! `EnvironmentMapLight`, two cubemaps more, failed to build its pipeline (measured: "count was 17").
//! The shader reads the metallic-roughness
//! texture through `StandardMaterial`'s own binding and sampler, so reading it again adds no
//! binding. The wrap term runs over the view's directional lights; point and spot lights get the
//! ordinary PBR response, and
//! ambient light gets none on purpose — under uniform ambient light a convex surface's subsurface
//! response integrates to its total diffuse reflectance, which is the base colour the ambient term
//! already multiplies. That is the scope the 0.4.0 plan chose, and a screen-space pass (Jimenez
//! 2012) is what a WebGPU-only build would reach for.

use bevy::asset::{AssetPath, Assets, Handle, RenderAssetUsages, embedded_asset};
use bevy::image::{Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Vec4;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin, StandardMaterial};
use bevy::prelude::*;
use bevy::reflect::Reflect;
use bevy::render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use crate::cross_section::{Layer, Layers};
use crate::flaymap::FlayCanvas;
use crate::bloodstain::spectral::{Film, SO2_ARTERIAL, SO2_VENOUS, TABLE, srgb};

/// **The material a canvas or a cap wears.** `StandardMaterial` underneath, [`FleshExtension`] on top.
pub type FleshMaterial = ExtendedMaterial<StandardMaterial, FleshExtension>;

/// Columns in the subsurface table: `N·L` from `−1` (left) to `+1` (right).
pub const SSS_COLS: u32 = 64;
/// Rows per tissue in the subsurface table: curvature `1/r`, flat (top) to the tightest radius.
pub const SSS_ROWS: u32 = 16;
/// The smallest sphere radius the table covers, mm. Tighter curvature clamps to this row.
pub const SSS_MIN_RADIUS_MM: f32 = 0.5;
/// The largest radius before the table is Lambert to the byte, mm.
pub const SSS_MAX_RADIUS_MM: f32 = 64.0;
/// Columns in the blood table: film depth from `0` to [`BLOOD_LUT_MAX_MM`].
pub const BLOOD_COLS: u32 = 256;
/// The film depth the last blood column stands for, mm. Whole blood is opaque to the byte by
/// roughly 0.6 mm (`crate::bloodstain::spectral`'s Kubelka–Munk saturates there), so a longer axis would
/// spend its columns on identical bytes; the shader clamps deeper films to the last column.
pub const BLOOD_LUT_MAX_MM: f32 = 1.0;

/// What the shader is looking at, so it knows where the tissue index comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum FleshMode {
    /// A cut face: the tissue is `UV_1.x` (depth over span) from `cross_section`'s annotation.
    Cap,
    /// A body canvas: tissue from the flaymap's depth byte, wetness from the wetmap's.
    Canvas,
    /// A fabric canvas: no subsurface; the wetmap's amount byte composites blood over the base. Blood
    /// in a weave is held between the yarns, so a wet patch stays matte and flat where a film on a
    /// floor shines and stands: its roughness goes to `0.7` rather than `0.3`, its clear coat is a
    /// third, and it is not lifted.
    Cloth,
    /// A floor, a wall or a slab: as [`Cloth`](Self::Cloth), but blood stands on it as a film and
    /// pools shine.
    Floor,
}

impl FleshMode {
    fn code(self) -> f32 {
        match self {
            FleshMode::Cap => 0.0,
            FleshMode::Canvas => 1.0,
            FleshMode::Cloth => 2.0,
            FleshMode::Floor => 3.0,
        }
    }
}

/// **The dials the shader reads, packed sixteen-byte aligned for WebGL2.**
#[derive(ShaderType, Clone, Copy, Debug, Reflect)]
pub struct FleshParams {
    /// Where the fat, muscle, cortex and marrow bands start, as fractions of the layer span. Skin is
    /// everything before `x`.
    pub bands: Vec4,
    /// `x` clearcoat strength at full wetness, `y` clearcoat perceptual roughness, `z` film depth in
    /// mm at amount byte 255 (the wetmap's `film_depth_mm`), `w` the blood's oxygen saturation `[0, 1]`.
    /// **Shipped as `(0.3, 0.3, 2.0, venous)` since 0.5.23**, where it was `(1.0, 0.08, ..)`: a coat
    /// that sharp and that strong clips to white wherever a flat pool crosses a light's mirror angle.
    pub wet: Vec4,
    /// `x` subsurface strength, `y` millimetres per mesh unit (curvature is measured in mesh units
    /// and the table is in mm), `z` the [`FleshMode`] code, `w` the canvas flags — which of `wet`,
    /// `flay` and `dermis` are bound. **Maintained by [`FleshExtension::set_wet`],
    /// [`FleshTables::material`] and [`FleshExtension::set_dermis`]**; an unbound texture samples
    /// Bevy's fallback image, which would otherwise read as a fully wet, fully peeled surface.
    pub sss: Vec4,
    /// `x` the thickest film of blood exposed tissue wears, mm: a cut face in [`FleshMode::Cap`], and
    /// a crater's walls where the skin is peeled in [`FleshMode::Canvas`]; `0` (as
    /// [`FleshParams::for_layers`] ships) draws the tissue bare. The film is `x` from half the layer
    /// span below the skin down, and a quarter of it at the skin, so the rim's fat shows through. See
    /// [`FleshParams::with_cut_film`]. `y` the layer span, mm — the depth a flaymap's `R` channel reads
    /// `1` at — which a crater's walls are measured in when the shader works out how much light
    /// reaches its floor. `z` and `w` are unused.
    pub cut: Vec4,
}

/// The `sss.w` bit that says the wetmap image is bound.
pub const FLAG_WET: u32 = 1;
/// The `sss.w` bit that says the flaymap image is bound.
pub const FLAG_FLAY: u32 = 2;
/// The `sss.w` bit that says the dermis image — bruises and burns under intact skin — is bound.
pub const FLAG_DERMIS: u32 = 4;

impl Default for FleshParams {
    fn default() -> Self {
        Self::for_layers(&Layers::for_region(crate::cross_section::Region::Limb), FleshMode::Canvas, 1000.0)
    }
}

impl FleshParams {
    /// The dials for a tissue row, a mode and a scale (`mm_per_unit`, `1000` for metres).
    pub fn for_layers(layers: &Layers, mode: FleshMode, mm_per_unit: f32) -> Self {
        let span = layers.span_mm().max(1.0e-3);
        let s = layers.starts_mm();
        Self {
            bands: Vec4::new(s[1] / span, s[2] / span, s[3] / span, s[4] / span),
            wet: Vec4::new(0.3, 0.3, 2.0, SO2_VENOUS),
            sss: Vec4::new(1.0, mm_per_unit, mode.code(), 0.0),
            cut: Vec4::new(0.0, span, 0.0, 0.0),
        }
    }

    /// **The same dials, with a fresh wound's blood on the tissue it bares**, `film_mm` where it lies
    /// thickest. Tissue cut or torn open is wet with its own blood: drawn bare, a fat band reads as
    /// cheese and a crater as a ring of fat.
    pub fn with_cut_film(mut self, film_mm: f32) -> Self {
        self.cut.x = if film_mm.is_finite() { film_mm.max(0.0) } else { 0.0 };
        self
    }

    fn set_flag(&mut self, flag: u32, on: bool) {
        let flags = self.sss.w.max(0.0) as u32;
        let flags = if on { flags | flag } else { flags & !flag };
        self.sss.w = flags as f32;
    }
}

/// **The extension**: the dials, the baked tables, and the optional canvas images.
///
/// Bindings start at 100 so they cannot collide with `StandardMaterial`'s. `wet` is the wetmap's
/// metallic-roughness image, read for the `R`/`A` channels that crate writes and Bevy ignores.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct FleshExtension {
    #[uniform(100)]
    pub params: FleshParams,
    /// [`luts`], from [`FleshTables`]: the subsurface and the blood tables in one image.
    ///
    /// **One image, and one sampler for all four textures** (binding 102), because every texture the
    /// extension adds counts against the sixteen a stage may sample on WebGL2 and under WebGPU's
    /// default limits, with `StandardMaterial`'s and the view's already counted. With the two tables
    /// apart the material used fifteen, and a camera with an `EnvironmentMapLight` — two cubemaps
    /// more — failed to build its pipeline on the web. Packed, it is sixteen with one.
    #[texture(101)]
    #[sampler(102)]
    pub luts: Handle<Image>,
    /// `wetmap`'s roughness image: `R` amount, `A` wetness. Unbound means dry.
    #[texture(104)]
    pub wet: Option<Handle<Image>>,
    /// `flaymap`'s albedo image: the peeled tissue, and the flat colour the canvas was dressed
    /// with wherever the skin is intact. The shader reads it with the same canvas's roughness
    /// image, which [`FleshTables::material`] puts in the base's metallic-roughness slot: `R` the
    /// depth over the span, `A` whether the texel is peeled. Unbound means intact skin.
    #[texture(105)]
    pub flay: Option<Handle<Image>>,
    /// The preset's dermis image: `rgb` the ratio the skin is multiplied by, `a` how much. Unbound
    /// means nothing under the skin. **On a [`FleshMode::Cloth`] material the same slot holds the
    /// holes shot through the cloth** (`ClothHoles` in the preset): `rgb` the ratio of the ring around
    /// a hole, shown by twice `a`, and the cloth cut away wherever `a` is over one half. The cut is an
    /// alpha, so it takes effect when the base's `alpha_mode` is a mask.
    #[texture(106)]
    pub dermis: Option<Handle<Image>>,
}

impl FleshExtension {
    /// Bind (or unbind) the wetmap's roughness image, keeping the flag the shader gates on in step.
    pub fn set_wet(&mut self, image: Option<Handle<Image>>) {
        self.params.set_flag(FLAG_WET, image.is_some());
        self.wet = image;
    }

    /// Bind (or unbind) the flaymap's albedo image, keeping the flag the shader gates on in step.
    /// Private: the shader also reads the canvas's roughness image out of the base material, and
    /// [`FleshTables::material`] is the one place that sets both.
    fn set_flay(&mut self, image: Option<Handle<Image>>) {
        self.params.set_flag(FLAG_FLAY, image.is_some());
        self.flay = image;
    }

    /// Bind (or unbind) the dermis image, keeping the flag the shader gates on in step.
    pub fn set_dermis(&mut self, image: Option<Handle<Image>>) {
        self.params.set_flag(FLAG_DERMIS, image.is_some());
        self.dermis = image;
    }
}

impl MaterialExtension for FleshExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://bevy_carnage/flesh.wgsl"))
    }

    /// Bevy's own prepass plus a cloth's holes, so a hole is cut from the depth prepass and the
    /// shadow maps as well as from the colour.
    fn prepass_fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::parse("embedded://bevy_carnage/flesh_prepass.wgsl"))
    }
}

/// **The two tables, baked once on `Startup`**, into one image: [`luts`].
#[derive(Resource, Clone, Debug, Default)]
pub struct FleshTables {
    /// [`luts`]: the pre-integrated subsurface table in its top-left corner and the blood film
    /// table in its last four rows, `Rgba8Unorm`.
    pub luts: Handle<Image>,
}

impl FleshTables {
    /// A [`FleshExtension`] over these tables with no canvases bound.
    pub fn extension(&self, mut params: FleshParams) -> FleshExtension {
        params.set_flag(FLAG_WET, false);
        params.set_flag(FLAG_FLAY, false);
        params.set_flag(FLAG_DERMIS, false);
        FleshExtension { params, luts: self.luts.clone(), wet: None, flay: None, dermis: None }
    }

    /// **A whole [`FleshMaterial`]** over `base`, with the canvases bound and the flags set. The base's
    /// `clearcoat` is forced to `1.0` — that is what turns Bevy's clearcoat lobe on at all; the shader
    /// then scales it per pixel by wetness, so a dry surface has none.
    ///
    /// **A skin over a flaymap keeps the skin it was authored with.** `base`'s colour, its
    /// `base_color_texture` and the mesh's vertex colours show wherever the skin is intact, and the
    /// canvas's tissue wherever it is peeled. Build the [`FlayCanvas`] with `base`'s colour factor
    /// and roughness, as the preset does: the shader swaps the canvas's flat intact colour for the
    /// authored skin by exactly the intact share of each texel. Two slots of `base` are the
    /// canvas's. The metallic-roughness texture becomes the canvas's roughness image, so a skin's
    /// own roughness map is replaced, and `perceptual_roughness` becomes `1.0` because Bevy
    /// multiplies it by that image. The crater's normal map is used only when `base` carries none of
    /// its own: a game's normal map is kept, and the crater then shades without its slope. Neither
    /// gets a slot of its own because every texture the extension adds counts against WebGL2's
    /// sixteen per stage, which `StandardMaterial` and the view already share.
    pub fn material(
        &self,
        mut base: StandardMaterial,
        params: FleshParams,
        wet: Option<Handle<Image>>,
        flay: Option<&FlayCanvas>,
    ) -> FleshMaterial {
        base.clearcoat = 1.0;
        base.clearcoat_perceptual_roughness = params.wet.y;
        let mut extension = self.extension(params);
        extension.set_wet(wet);
        if let Some(flay) = flay {
            base.metallic_roughness_texture = Some(flay.roughness());
            base.perceptual_roughness = 1.0;
            base.normal_map_texture = base.normal_map_texture.or_else(|| Some(flay.normal()));
            extension.set_flay(Some(flay.albedo()));
        }
        FleshMaterial { base, extension }
    }
}

/// **Registers [`FleshMaterial`], embeds its shader and bakes [`FleshTables`] on `Startup`.**
///
/// Nothing runs per frame. The tables are baked in [`FleshSystems`] so a caller that builds
/// materials on `Startup` can order `.after(FleshSystems)`.
pub struct FleshPlugin;

/// The one-shot table bake on `Startup`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FleshSystems;

impl Plugin for FleshPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "flesh.wgsl");
        embedded_asset!(app, "flesh_prepass.wgsl");
        app.init_resource::<FleshTables>()
            .add_plugins(MaterialPlugin::<FleshMaterial>::default())
            .add_systems(Startup, bake_tables.in_set(FleshSystems));
    }
}

/// Bake both tables into `Assets<Image>`. `Option`, because a headless app without `ImagePlugin`
/// still builds and should get an empty resource rather than a panic.
fn bake_tables(mut tables: ResMut<FleshTables>, images: Option<ResMut<Assets<Image>>>) {
    let Some(mut images) = images else {
        warn!("bevy_carnage::flesh: no Assets<Image>; the flesh tables are not baked");
        return;
    };
    let mut image = Image::new(
        Extent3d { width: LUTS_WIDTH, height: LUTS_HEIGHT, depth_or_array_layers: 1 },
        TextureDimension::D2,
        luts(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = clamped_linear();
    tables.luts = images.add(image);
}

fn clamped_linear() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    })
}

// ---------------------------------------------------------------------------------------------
// The dipole, and the tissue rows.
// ---------------------------------------------------------------------------------------------

/// Reduced scattering and absorption per channel, mm⁻¹, and the relative index of refraction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    /// `σ_s'` per R, G, B.
    pub sigma_s: [f32; 3],
    /// `σ_a` per R, G, B.
    pub sigma_a: [f32; 3],
    /// `η`.
    pub eta: f32,
}

impl Profile {
    /// Jensen et al. 2001, Table (their `skin1`).
    pub const SKIN: Profile = Profile { sigma_s: [0.74, 0.88, 1.01], sigma_a: [0.032, 0.17, 0.48], eta: 1.3 };
    /// **This crate's own**: skin's scattering scaled 1.5× and its absorption quartered in red and
    /// green with the blue held, which is a translucent yellow. No adipose profile was in the corpus.
    pub const FAT: Profile = Profile { sigma_s: [1.11, 1.32, 1.52], sigma_a: [0.008, 0.043, 0.40], eta: 1.4 };

    /// **Derived**: skin's scattering, absorption = 5 % blood volume (own) × Bosschaart's venous
    /// whole-blood `μa` band-averaged over the R (620–700), G (500–580) and B (440–500) thirds, plus a
    /// `0.02 mm⁻¹` non-haem baseline (own).
    pub fn muscle() -> Profile {
        const FRACTION: f32 = 0.05;
        const BASELINE: f32 = 0.02;
        let band = |lo: u16, hi: u16| -> f32 {
            let mut sum = 0.0;
            let mut n = 0.0;
            for s in TABLE.iter() {
                if s.nm >= lo && s.nm <= hi {
                    let mua = SO2_VENOUS * s.mua_oxy + (1.0 - SO2_VENOUS) * s.mua_deoxy;
                    sum += mua;
                    n += 1.0;
                }
            }
            if n > 0.0 { sum / n } else { 0.0 }
        };
        let r = band(620, 700);
        let g = band(500, 580);
        let b = band(440, 500);
        Profile {
            sigma_s: [0.6, 0.8, 1.0],
            sigma_a: [FRACTION * r + BASELINE, FRACTION * g + BASELINE, FRACTION * b + BASELINE],
            eta: 1.37,
        }
    }

    /// The profile a layer scatters by; `None` for cortex, which the table leaves Lambert.
    pub fn for_layer(layer: Layer) -> Option<Profile> {
        match layer {
            Layer::Skin => Some(Profile::SKIN),
            Layer::Fat | Layer::Marrow => Some(Profile::FAT),
            Layer::Muscle => Some(Profile::muscle()),
            Layer::Cortex => None,
        }
    }
}

/// **Jensen's dipole diffuse reflectance `R_d(r)`** for one channel at distance `r` mm.
///
/// `σ_t' = σ_a + σ_s'`, `α' = σ_s'/σ_t'`, `σ_tr = √(3 σ_a σ_t')`, the real source one mean free path
/// down (`z_r = 1/σ_t'`) and the virtual one at `z_v = z_r (1 + 4A/3)` with `A` from the diffuse
/// Fresnel reflectance `F_dr(η) = −1.440/η² + 0.710/η + 0.668 + 0.0636 η`.
pub fn dipole(r_mm: f32, sigma_s: f32, sigma_a: f32, eta: f32) -> f32 {
    let sigma_t = sigma_a + sigma_s;
    if sigma_t <= 0.0 {
        return 0.0;
    }
    let alpha = sigma_s / sigma_t;
    let sigma_tr = libm::sqrtf(3.0 * sigma_a * sigma_t);
    let fdr = -1.440 / (eta * eta) + 0.710 / eta + 0.668 + 0.0636 * eta;
    let a = (1.0 + fdr) / (1.0 - fdr);
    let z_r = 1.0 / sigma_t;
    let z_v = z_r * (1.0 + 4.0 * a / 3.0);
    let d_r = libm::sqrtf(r_mm * r_mm + z_r * z_r);
    let d_v = libm::sqrtf(r_mm * r_mm + z_v * z_v);
    let term = |z: f32, d: f32| z * (sigma_tr * d + 1.0) * libm::expf(-sigma_tr * d) / (sigma_t * d * d * d);
    alpha / (4.0 * core::f32::consts::PI) * (term(z_r, d_r) + term(z_v, d_v))
}

/// The radius the table row `row` stands for, mm — log-spaced from flat to [`SSS_MIN_RADIUS_MM`].
fn row_radius_mm(row: u32) -> f32 {
    if row == 0 {
        return f32::INFINITY;
    }
    let t = row as f32 / (SSS_ROWS - 1) as f32;
    let lo = libm::logf(SSS_MAX_RADIUS_MM);
    let hi = libm::logf(SSS_MIN_RADIUS_MM);
    libm::expf(lo + (hi - lo) * t)
}

/// **Penner's ring integral** for one channel: the diffuse response at `cos θ = ndotl` on a sphere of
/// radius `r`, normalised so a flat surface is exactly Lambert.
///
/// `D(θ, r) = ∫ max(0, cos(θ + x)) · R_d(2 r sin(x/2)) dx / ∫ R_d(2 r sin(x/2)) dx`, `x ∈ [−π, π]`.
pub fn pre_integrate(ndotl: f32, r_mm: f32, sigma_s: f32, sigma_a: f32, eta: f32) -> f32 {
    if !r_mm.is_finite() {
        return ndotl.max(0.0);
    }
    const N: usize = 256;
    let theta = libm::acosf(ndotl.clamp(-1.0, 1.0));
    let mut num = 0.0f32;
    let mut den = 0.0f32;
    for i in 0..N {
        let x = -core::f32::consts::PI + (i as f32 + 0.5) * (2.0 * core::f32::consts::PI / N as f32);
        let d = 2.0 * r_mm * libm::sinf(0.5 * x).abs();
        let w = dipole(d, sigma_s, sigma_a, eta);
        num += libm::cosf(theta + x).max(0.0) * w;
        den += w;
    }
    if den > 0.0 { (num / den).clamp(0.0, 1.0) } else { ndotl.max(0.0) }
}

/// **The subsurface table's bytes**, [`SSS_COLS`] × ([`SSS_ROWS`] · 5), RGBA8 linear, row-major.
///
/// Row block `i` is `Layer::ALL[i]`; within a block, row 0 is flat and the last row is
/// [`SSS_MIN_RADIUS_MM`]; column `c` is `N·L = 2c/(SSS_COLS−1) − 1`. Alpha is 255.
pub fn sss_lut() -> Vec<u8> {
    let rows = SSS_ROWS as usize * Layer::ALL.len();
    let mut out = Vec::with_capacity(rows * SSS_COLS as usize * 4);
    for layer in Layer::ALL {
        let profile = Profile::for_layer(layer);
        for row in 0..SSS_ROWS {
            let r = row_radius_mm(row);
            for col in 0..SSS_COLS {
                let ndotl = 2.0 * col as f32 / (SSS_COLS - 1) as f32 - 1.0;
                for ch in 0..3 {
                    let v = match profile {
                        Some(p) => pre_integrate(ndotl, r, p.sigma_s[ch], p.sigma_a[ch], p.eta),
                        None => ndotl.max(0.0),
                    };
                    out.push(byte(v));
                }
                out.push(255);
            }
        }
    }
    out
}

/// **The blood table's bytes**, [`BLOOD_COLS`] × 4, RGBA8 sRGB. Rows: arterial over black, arterial
/// over white, venous over black, venous over white. Column `c` is a film `c/255 ·`
/// [`BLOOD_LUT_MAX_MM`] thick. Alpha is 255.
pub fn blood_lut() -> Vec<u8> {
    let mut out = Vec::with_capacity(BLOOD_COLS as usize * 4 * 4);
    for (so2, substrate) in [(SO2_ARTERIAL, 0.0), (SO2_ARTERIAL, 1.0), (SO2_VENOUS, 0.0), (SO2_VENOUS, 1.0)] {
        for col in 0..BLOOD_COLS {
            let thickness_mm = col as f32 / (BLOOD_COLS - 1) as f32 * BLOOD_LUT_MAX_MM;
            let c = srgb(&Film { thickness_mm, so2, substrate });
            out.extend_from_slice(&[byte(c[0]), byte(c[1]), byte(c[2]), 255]);
        }
    }
    out
}

/// The width of [`luts`]: the blood table's.
pub const LUTS_WIDTH: u32 = BLOOD_COLS;
/// The height of [`luts`]: the subsurface table's rows, then the blood table's four.
pub const LUTS_HEIGHT: u32 = SSS_ROWS * Layer::ALL.len() as u32 + 4;

/// **Both tables in one image's bytes**, [`LUTS_WIDTH`] × [`LUTS_HEIGHT`], RGBA8, row-major: the
/// subsurface table ([`sss_lut`]) in the top-left [`SSS_COLS`] columns of its first rows, the rest of
/// those rows zero, and the blood table ([`blood_lut`]) in the last four rows, byte for byte. The
/// image is `Rgba8Unorm`, so the blood rows, which are sRGB-encoded, are decoded in the shader, and
/// the shader holds each lookup to its own table's texel centres, so neither filters into the other
/// or into the padding.
pub fn luts() -> Vec<u8> {
    let (sss, blood) = (sss_lut(), blood_lut());
    let row = LUTS_WIDTH as usize * 4;
    let mut out = vec![0; row * LUTS_HEIGHT as usize];
    for (y, src) in sss.chunks(SSS_COLS as usize * 4).enumerate() {
        if let Some(dst) = out.get_mut(y * row..y * row + src.len()) {
            dst.copy_from_slice(src);
        }
    }
    let at = row * (SSS_ROWS as usize * Layer::ALL.len());
    if let Some(dst) = out.get_mut(at..at + blood.len()) {
        dst.copy_from_slice(&blood);
    }
    out
}

/// FNV-1a over both tables' bytes — the golden `the_flesh_tables_are_frozen` locks.
pub fn lut_digest() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in sss_lut().iter().chain(blood_lut().iter()) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn byte(v: f32) -> u8 {
    if !v.is_finite() {
        return 0;
    }
    libm::roundf(v.clamp(0.0, 1.0) * 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dipole is positive, falls monotonically with distance, and the flat row of the table is
    /// Lambert to the byte for every tissue.
    #[test]
    fn the_dipole_falls_and_the_flat_row_is_lambert() {
        let p = Profile::SKIN;
        let mut last = f32::INFINITY;
        for i in 1..40 {
            let r = i as f32 * 0.25;
            let v = dipole(r, p.sigma_s[0], p.sigma_a[0], p.eta);
            assert!(v > 0.0 && v < last, "R_d({r}) = {v} not below {last}");
            last = v;
        }
        let lut = sss_lut();
        let stride = SSS_COLS as usize * 4;
        for (i, _) in Layer::ALL.iter().enumerate() {
            let row = &lut[i * SSS_ROWS as usize * stride..][..stride];
            for col in 0..SSS_COLS as usize {
                let ndotl = 2.0 * col as f32 / (SSS_COLS - 1) as f32 - 1.0;
                assert_eq!(row[col * 4], byte(ndotl.max(0.0)), "layer {i} col {col}");
            }
        }
    }

    /// Curvature wraps light past the terminator: at `N·L = 0` every scattering tissue's tightest row
    /// is brighter than its flat row in red, and skin wraps red further than blue (its red absorption
    /// is fifteen times lower).
    #[test]
    fn curvature_wraps_light_past_the_terminator_and_red_wraps_furthest() {
        let lut = sss_lut();
        let stride = SSS_COLS as usize * 4;
        let mid = (SSS_COLS / 2) as usize * 4;
        for (i, layer) in Layer::ALL.iter().enumerate() {
            let flat = lut[i * SSS_ROWS as usize * stride + mid];
            let tight = lut[(i * SSS_ROWS as usize + SSS_ROWS as usize - 1) * stride + mid];
            if *layer == Layer::Cortex {
                assert_eq!(flat, tight, "cortex stays Lambert");
            } else {
                assert!(tight > flat, "{layer:?}: tight {tight} ≤ flat {flat}");
            }
        }
        let skin_tight = &lut[(SSS_ROWS as usize - 1) * stride + mid..][..3];
        assert!(skin_tight[0] > skin_tight[2], "skin red {} ≤ blue {}", skin_tight[0], skin_tight[2]);
    }

    /// The blood table darkens with depth and arterial is redder than venous at equal depth.
    #[test]
    fn the_blood_table_darkens_with_depth_and_arterial_is_redder() {
        let lut = blood_lut();
        let row = |r: usize, c: usize| &lut[(r * BLOOD_COLS as usize + c) * 4..][..3];
        // Linear luminance from the decoded bytes. The *encoded* channels are not individually
        // monotone — a thin film is out of the sRGB gamut and its green clamps to zero before the
        // thicker, desaturated film brings it back — but the CIE Y underneath is.
        let decode = |b: u8| {
            let c = b as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let luma = |p: &[u8]| 0.2126 * decode(p[0]) + 0.7152 * decode(p[1]) + 0.0722 * decode(p[2]);
        let mut last = f32::INFINITY;
        for c in (0..BLOOD_COLS as usize).step_by(16) {
            let l = luma(row(1, c));
            assert!(l <= last + 0.002, "arterial over white brightens at column {c}: {l} > {last}");
            last = l;
        }
        let c = 64;
        let art = row(0, c);
        let ven = row(2, c);
        let redness = |p: &[u8]| p[0] as f32 / (p[1] as f32 + p[2] as f32 + 1.0);
        assert!(redness(art) > redness(ven), "arterial {art:?} not redder than venous {ven:?}");
    }

    /// The bytes both tables bake to. Two machines must agree, so `libm` does the transcendental
    /// arithmetic. **A lock, not a target**: if this moves, a profile or the integrator changed.
    #[test]
    fn the_flesh_tables_are_frozen() {
        assert_eq!(lut_digest(), 17_594_585_363_927_039_033);
    }

    /// **The packed image holds both tables where the shader reads them**: the subsurface table's rows
    /// in the first [`SSS_COLS`] columns of the top rows, the blood table's four rows last, and zero
    /// between. The shader's offsets are written against exactly this; a table shifted by one row or
    /// column would shade every surface with its neighbour's values.
    #[test]
    fn the_packed_image_holds_both_tables_where_the_shader_reads_them() {
        let (packed, sss, blood) = (luts(), sss_lut(), blood_lut());
        let row = LUTS_WIDTH as usize * 4;
        let sss_row = SSS_COLS as usize * 4;
        let sss_rows = SSS_ROWS as usize * Layer::ALL.len();
        assert_eq!(packed.len(), row * LUTS_HEIGHT as usize);
        for y in 0..sss_rows {
            assert_eq!(&packed[y * row..y * row + sss_row], &sss[y * sss_row..(y + 1) * sss_row], "subsurface row {y}");
            assert!(packed[y * row + sss_row..(y + 1) * row].iter().all(|b| *b == 0), "row {y} is not padded with zero");
        }
        assert_eq!(&packed[sss_rows * row..], &blood[..], "the blood table is not the last four rows");
    }
}
