//! **The canvas: one `Vec<(u8, u16)>` in row-major order, and four passes over it.**
//!
//! Everything a wetmap decides lives in this file, on the CPU, in integers. The two `Image`s exist so
//! a renderer has something to sample; nothing in this crate ever reads one back.

use bevy::asset::{Assets, Handle, RenderAssetUsages};
use bevy::image::Image;
use bevy::log::warn;
use bevy::math::{Vec2, Vec3};
use bevy::mesh::Mesh;
use bevy::prelude::Component;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::transform::components::GlobalTransform;
use crate::bloodstain::dry::{DRY_REF_AREA_M2, DRY_REF_TICKS, appearance_with_fresh};
use crate::bloodstain::spectral::Film;
use crate::bloodstain::stain::{StainShape, rasterise};

/// Thickness levels the fresh-colour table holds per tick. Sixteen: blood's colour moves fastest
/// in the first quarter-millimetre and is flat past a couple, so the levels are spent where the
/// substrate still shows through.
const FILM_LEVELS: usize = 16;

use crate::wetmap::digest::Fnv1a;
use crate::wetmap::settings::WetSettings;
use crate::wetmap::uv::{Pick, mesh_key, ray_uv};

/// **How much world one UV unit is taken to be**, metres.
///
/// A `Mesh`'s UV parameterisation carries no scale — nothing in a texture coordinate says whether the
/// unit square covers a thumbnail or a torso — so the bridge from `StainShape::major` (metres) and
/// [`WetCanvas::wetted_area`] (m²) to texels has to be *stated somewhere*. It is stated here, once, as
/// a constant: **one UV unit is one metre.**
///
/// A caller whose atlas covers a two-metre actor therefore has a canvas at half the texel density this
/// constant implies, and either authors `StainShape::major` at that scale or accepts the resolution.
/// The alternative — a per-canvas scale parameter — would be a dial every call site had to agree on to
/// get comparable stains, and disagreement would be invisible until two actors' blood looked like two
/// different fluids.
pub const UV_SPAN_M: f32 = 1.0;

/// Meshes remembered for the once-per-mesh warning before the memo stops growing.
///
/// A cap rather than an unbounded set: the memo exists to stop log spam, and an actor is built from a
/// handful of meshes. Past the cap the oldest key is dropped, so a long session that keeps meeting new
/// broken meshes may warn about one twice — the right way round for a memo whose other failure mode
/// would be unbounded memory in a shipped game.
const WARN_MEMO: usize = 16;

/// **Texture-space blood accumulation for one actor.** Two images out, and the authority stays here.
///
/// # The state is one buffer, and its order is the canonical order
///
/// `amount: u8` (normalised coverage) and `age: u16` (ticks since the youngest blood in this texel
/// landed), as a single `Vec<(u8, u16)>` in **row-major** order. That order *is* the order
/// [`tick`](Self::tick) walks, so nothing here needs a sort and nothing here may add one — a sort would
/// be a second answer to a question the layout already answers, and the digest would then depend on
/// which answer ran.
///
/// # What the caller owns
///
/// The schedule. [`tick`](Self::tick) takes the tick number because this crate has no clock and will
/// not guess one; the plugin registers only the upload budget, which is the one part with no gameplay
/// opinion in it.
#[derive(Component, Debug)]
pub struct WetCanvas {
    /// Edge length in texels. Always ≥ 1.
    size: u32,
    /// `(amount, age)` per texel, row-major. **The authority.**
    wet: Vec<(u8, u16)>,
    /// Snapshot of `wet` taken at the top of the drip and spread passes.
    ///
    /// Both passes read *this* and write disjoint slots of `wet`, which is what makes their result
    /// independent of traversal order — and therefore what makes [`digest`](Self::digest) mean
    /// anything at all. (The same argument `bevy_stigmergy` makes for its diffusion stencil.)
    prev: Vec<(u8, u16)>,
    /// Scratch for `crate::bloodstain::stain::rasterise`, kept so a stamp does not allocate.
    ///
    /// Two buffers because the rasterisation happens at `edge_samples` times the canvas resolution
    /// and is then box-filtered down: `fine` is what `rasterise` writes, `mask` is the per-texel
    /// coverage the stamp adds. At one sample per texel the filter is the identity and the two hold
    /// the same bytes, which is why the dial cannot move a digest.
    mask: Vec<u8>,
    fine: Vec<u8>,
    albedo_handle: Handle<Image>,
    rough_handle: Handle<Image>,
    /// RGBA8 sRGB bytes, base colour composited with blood by coverage.
    albedo_px: Vec<u8>,
    /// RGBA8 **linear** bytes: G is roughness, B is metallic. See [`roughness`](Self::roughness).
    rough_px: Vec<u8>,
    /// The dry surface under the blood, as bytes, so the composite does not re-encode it per texel.
    base_rgba: [u8; 4],
    /// The dry surface's roughness byte.
    base_rough: u8,
    /// Earliest tick at which the CPU state diverged from what was last uploaded, or `None` when the
    /// two agree. **Oldest-dirty-first** ordering in the plugin's budget reads this.
    dirty_since: Option<u32>,
    /// Meshes already refused, so a UV-less mesh warns exactly once instead of once per shot.
    warned: Vec<u64>,
    /// The fresh-colour table `shade` reads, and the bit patterns of the three inputs it is a
    /// function of. Sixteen `spectral::srgb` calls are ~1300 transcendentals; paying them once per
    /// tick per canvas, on a dry canvas, was the difference between "a wetmap costs nothing when
    /// nothing is wet" being true and false.
    film_lut: [[f32; 3]; FILM_LEVELS],
    film_key: Option<(u32, u32, u32)>,
    /// **The settings this canvas went still under, or `None` while anything on it can still move.**
    ///
    /// A canvas whose every bloody texel has dried — and an untouched one, which has no blood at
    /// all — is a fixed point of [`tick`](Self::tick): drip and spread move nothing through a dry
    /// crust, `advance` holds a dried age at the span, and `shade` writes the bytes already there.
    /// So `tick` returns at its top while the settings it is handed equal these, and a paint, or any
    /// change to the settings, wakes it. On a body dressed by the preset that is most canvases on
    /// most ticks: a part nobody has shot, or one whose blood has set.
    quiet: Option<WetSettings>,
    /// **The smallest rectangle holding every texel with blood in it**, or `None` for a canvas with
    /// none. Every texel outside it is `(0, 0)`. One tick moves blood at most [`TICK_REACH`] texels:
    /// drip translates a parcel one texel, and spread, which runs after it in the same tick, can
    /// carry that parcel's share one more. So the passes run over this rectangle grown by that much
    /// and leave the rest of the canvas untouched. That is exact, not an approximation: outside the
    /// grown rectangle every pass would read zeros and write the zeros already there. Re-measured
    /// after every tick, so it shrinks as the substrate takes the blood.
    live: Option<Live>,
    /// **Image pixels per texel along each edge**, `1..=MAX_DETAIL`. See
    /// [`new_detailed`](Self::new_detailed).
    detail: u32,
    /// The two images' bytes at `detail > 1`: the shaded texels redrawn `detail` times finer through
    /// a cubic B-spline, with the stain's edge drawn crisp. Empty at `detail == 1`, where the images
    /// are `albedo_px` and `rough_px` themselves.
    detail_albedo: Vec<u8>,
    detail_rough: Vec<u8>,
    /// One flag per [`STALE_TILE`]-square tile, row-major: its texels' shaded bytes changed since
    /// the detail images were last redrawn. Empty at `detail == 1`, where there is nothing to redraw.
    stale: Vec<bool>,
    /// Scratch for the redraw: each texel's six coverage-weighted values, the pass along the rows'
    /// sums, and the smoothed coverage per pixel.
    texels: Vec<f32>,
    rows: Vec<[f32; 6]>,
    cover: Vec<f32>,
}

impl WetCanvas {
    /// **A blank canvas and its two images.**
    ///
    /// `size` is the edge length in texels; **128 is the shipped default and 256 is the practical
    /// ceiling.** The arithmetic is the reason, not taste: a 128×128 canvas at `Rgba8UnormSrgb` is
    /// `128 · 128 · 4 = 65 536` bytes — 64 KB per upload — and this crate owns **two** images per
    /// actor, so one canvas costs 128 KB of `Assets<Image>` writes every time it is flushed. At the
    /// shipped budget of four canvases per frame that is 512 KB/frame; at 256 it is 2 MB/frame; at 512
    /// it is 8 MB/frame, which is a bandwidth budget rather than a texture. Blood reads fine at 128,
    /// because a stain is a blob and not text.
    ///
    /// `base_srgb` and `base_roughness` are the **dry surface under the blood**. They are composited
    /// into the canvas on the CPU, which is why this crate ships no shader and no asset: there is
    /// nothing left to blend in WGSL.
    ///
    /// A `size` of 0 is corrected to 1 and warns — a zero-extent texture is not a canvas.
    pub fn new(
        images: &mut Assets<Image>,
        size: u32,
        base_srgb: [f32; 3],
        base_roughness: f32,
    ) -> Self {
        Self::new_detailed(images, size, 1, base_srgb, base_roughness)
    }

    /// **[`new`](Self::new), with images `detail` times finer than the canvas**, so a stain's edge
    /// is a smooth line on a plain `StandardMaterial` instead of the texels' staircase.
    ///
    /// The canvas, its passes and its [`digest`](Self::digest) are exactly [`new`](Self::new)'s at
    /// `size`: only the two images change. On [`flush`](Self::flush) the texels that changed are
    /// redrawn at `detail × detail` pixels each, read as a cubic B-spline (Sigg & Hadwiger, *GPU
    /// Gems 2*, ch. 20) — the same reading `flesh.wgsl` makes of the map in its shader, done here
    /// once per upload instead — and the blood's edge is drawn where the smoothed coverage crosses
    /// [`DETAIL_EDGE`], anti-aliased over one pixel. Inside the edge the colour is the film's own,
    /// weighted by how much blood each texel holds, so a stain keeps its colour up to its rim rather
    /// than fading into the surface it lies on. R and A of the roughness image are the smoothed
    /// coverage and wetness under that edge.
    ///
    /// **The cost is `detail²`.** Both images and their CPU copies grow by it, so at 128 a `detail`
    /// of 2 uploads what a 256 canvas would and 4 what a 512 one would, and a flush redraws the
    /// changed texels' pixels, about 50 float multiply-adds each. `1` is [`new`](Self::new), where the
    /// images are the texels themselves; it is clamped to `1..=`[`MAX_DETAIL`], warning when it is
    /// outside. A material that reads the canvas in its own shader, as the `flesh` feature's does,
    /// gains nothing from it.
    pub fn new_detailed(
        images: &mut Assets<Image>,
        size: u32,
        detail: u32,
        base_srgb: [f32; 3],
        base_roughness: f32,
    ) -> Self {
        let size = if size == 0 {
            warn!("wetmap: a canvas of size 0 has no texels; using 1");
            1
        } else {
            size
        };
        let detail = if (1..=MAX_DETAIL).contains(&detail) {
            detail
        } else {
            warn!("wetmap: an image detail of {detail} is outside 1..={MAX_DETAIL}; clamping");
            detail.clamp(1, MAX_DETAIL)
        };
        let base_rgba = [enc(base_srgb[0]), enc(base_srgb[1]), enc(base_srgb[2]), 255];
        let base_rough = enc(base_roughness);
        // **The four channels of the metallic-roughness image, and only two of them are Bevy's.**
        // G carries roughness and B carries metallic — blood is a dielectric, so B stays 0 — and R
        // and A are free for data a caller's own shader may want: R is the coverage byte (the film
        // depth) and A is wetness. An untouched texel has no blood on it, so both are 0.
        let base_rough_rgba = [0, base_rough, 0, 0];

        let edge = size.saturating_mul(detail);
        let extent = Extent3d { width: edge, height: edge, depth_or_array_layers: 1 };
        // `MAIN_WORLD | RENDER_WORLD` because `flush` rewrites the pixels every time they change: with
        // `RENDER_WORLD` alone the main-world copy is dropped after the first upload and every
        // subsequent write would land in nothing.
        let usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        let albedo = Image::new_fill(
            extent,
            TextureDimension::D2,
            &base_rgba,
            // sRGB, because this image is sampled as colour by `StandardMaterial::base_color_texture`.
            TextureFormat::Rgba8UnormSrgb,
            usage,
        );
        // **Linear, not sRGB.** Roughness and metallic are material data, not colour; glTF's own
        // metallic-roughness textures are linear for the same reason, and sampling this one through an
        // sRGB decode would bend the gloss curve.
        let rough = Image::new_fill(
            extent,
            TextureDimension::D2,
            &base_rough_rgba,
            TextureFormat::Rgba8Unorm,
            usage,
        );

        let texels = (size as usize) * (size as usize);
        let pixels = if detail > 1 { (edge as usize) * (edge as usize) } else { 0 };
        Self {
            size,
            wet: vec![(0, 0); texels],
            prev: vec![(0, 0); texels],
            mask: Vec::new(),
            fine: Vec::new(),
            albedo_handle: images.add(albedo),
            rough_handle: images.add(rough),
            albedo_px: base_rgba.iter().copied().cycle().take(texels * 4).collect(),
            rough_px: base_rough_rgba.iter().copied().cycle().take(texels * 4).collect(),
            base_rgba,
            base_rough,
            dirty_since: None,
            warned: Vec::new(),
            film_lut: [[0.0; 3]; FILM_LEVELS],
            film_key: None,
            quiet: None,
            live: None,
            detail,
            detail_albedo: base_rgba.iter().copied().cycle().take(pixels * 4).collect(),
            detail_rough: base_rough_rgba.iter().copied().cycle().take(pixels * 4).collect(),
            stale: if detail > 1 { vec![false; (size.div_ceil(STALE_TILE as u32) as usize).pow(2)] } else { Vec::new() },
            texels: Vec::new(),
            rows: Vec::new(),
            cover: Vec::new(),
        }
    }

    /// The base-colour image. Feed it to `StandardMaterial::base_color_texture`
    /// (`bevy_pbr-0.19.0/src/pbr_material.rs:57`).
    pub fn albedo(&self) -> Handle<Image> {
        self.albedo_handle.clone()
    }

    /// The metallic-roughness image. Feed it to `StandardMaterial::metallic_roughness_texture`
    /// (`bevy_pbr-0.19.0/src/pbr_material.rs:170`).
    ///
    /// **Roughness is the green channel and metallic is the blue one** — stated in that file at
    /// `:153-154`: *"The blue channel contains metallic values, and the green channel contains the
    /// roughness values."* B stays 0, so blood stays a dielectric.
    ///
    /// **R and A are the wetmap's own data channels**, because Bevy reads only G and B from this
    /// image and a second texture per actor would be a second upload. Per texel:
    ///
    /// | channel | what | range |
    /// |---|---|---|
    /// | R | coverage — the film depth byte the buffer holds, `amount` | `0..=255` |
    /// | G | perceptual roughness, wet blood over the dry surface | `0..=255` |
    /// | B | metallic — always `0` | `0` |
    /// | A | wetness: `round(255 · (1 − age / dry_ticks))`, `0` where there is no blood | `0..=255` |
    ///
    /// So `R · film_depth_mm / 255` is the millimetres of blood a texel holds and `A` is how far it
    /// is from set — the two quantities a caller's own shader would otherwise have to guess from the
    /// colour. Nothing in this crate reads them back: they are output, like every other byte here.
    ///
    /// **The material must set `perceptual_roughness: 1.0` and `metallic: 1.0`**, because Bevy
    /// *multiplies* the scalars by the texture (`:157-163`), and the shipped scalars would scale the
    /// map away. Wetness is the strongest disgust cue and it is specular rather than a colour (Oum,
    /// Lieberman & Aylward, `doi:10.1080/02699931.2010.496997`), so getting this wrong loses the
    /// channel that carries the effect.
    pub fn roughness(&self) -> Handle<Image> {
        self.rough_handle.clone()
    }

    /// Edge length in texels.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Image pixels per texel along each edge: `1` from [`new`](Self::new), and what
    /// [`new_detailed`](Self::new_detailed) was given, clamped. The images are `size() · detail()`
    /// pixels square.
    pub fn detail(&self) -> u32 {
        self.detail
    }

    /// **The bytes this canvas holds in the main world**, apart from its two images: the `(amount,
    /// age)` buffer and its snapshot, the stamp scratch, and the CPU copy of each image (at a
    /// [`detail`](Self::detail) above 1, both the texels' and the finer images', and the redraw's
    /// scratch). At 128 and detail 1 it is 16 bytes a texel, 256 KiB. Nothing here is read back from
    /// `Assets<Image>`; see [`image_bytes`](Self::image_bytes) for what the images add.
    pub fn heap_bytes(&self) -> usize {
        let cell = std::mem::size_of::<(u8, u16)>();
        (self.wet.capacity() + self.prev.capacity()) * cell
            + self.mask.capacity()
            + self.fine.capacity()
            + self.albedo_px.capacity()
            + self.rough_px.capacity()
            + self.detail_albedo.capacity()
            + self.detail_rough.capacity()
            + self.texels.capacity() * std::mem::size_of::<f32>()
            + self.rows.capacity() * std::mem::size_of::<[f32; 6]>()
            + self.cover.capacity() * std::mem::size_of::<f32>()
    }

    /// **The size of the two images this canvas uploads** (albedo and metallic-roughness), once. The
    /// main world keeps a copy of each and the GPU holds another, so the memory they cost is twice this,
    /// and `Assets<Image>` is where the first copy is. Eight bytes a pixel: 128 KiB at 128 and detail
    /// 1, `detail²` times that above it.
    pub fn image_bytes(&self) -> usize {
        let (albedo, rough) = self.images_px();
        albedo.len() + rough.len()
    }

    /// The bytes the two images hold: the texels' own at detail 1, the redrawn ones above it.
    fn images_px(&self) -> (&[u8], &[u8]) {
        if self.detail > 1 { (&self.detail_albedo, &self.detail_rough) } else { (&self.albedo_px, &self.rough_px) }
    }

    /// Normalised coverage at a texel, `0` outside the canvas — a texel that does not exist holds no
    /// blood.
    pub fn amount_at(&self, x: u32, y: u32) -> u8 {
        self.at(x, y).map(|c| c.0).unwrap_or(0)
    }

    /// Ticks since the youngest blood in this texel landed, `0` outside the canvas.
    pub fn age_at(&self, x: u32, y: u32) -> u16 {
        self.at(x, y).map(|c| c.1).unwrap_or(0)
    }

    /// Whether the CPU state has diverged from the uploaded images.
    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    /// **Wipe every drop of blood off, keeping the canvas and its images.** The buffer, both images'
    /// bytes and every pass's bookkeeping go back to what [`new_detailed`](Self::new_detailed) made,
    /// so [`digest`](Self::digest) is a fresh canvas's again; the images are re-uploaded on the next
    /// [`flush`](Self::flush), as of `tick`. Nothing is reallocated: a pooled character is cleaned for
    /// the cost of overwriting its bytes, where despawning and redressing it builds every canvas and
    /// image again.
    pub fn clear(&mut self, tick: u32) {
        let (base, rough) = (self.base_rgba, [0, self.base_rough, 0, 0]);
        self.wet.fill((0, 0));
        self.prev.fill((0, 0));
        self.albedo_px.as_chunks_mut::<4>().0.fill(base);
        self.rough_px.as_chunks_mut::<4>().0.fill(rough);
        self.detail_albedo.as_chunks_mut::<4>().0.fill(base);
        self.detail_rough.as_chunks_mut::<4>().0.fill(rough);
        self.stale.fill(false);
        self.quiet = None;
        self.live = None;
        self.dirty_since = Some(self.dirty_since.map_or(tick, |since| since.min(tick)));
    }

    /// The tick at which this canvas first diverged from its images, or `None` when they agree.
    ///
    /// The plugin's upload budget sorts on this — oldest dirty first — so a canvas that has been
    /// waiting cannot be starved by one that keeps being repainted.
    pub fn dirty_since(&self) -> Option<u32> {
        self.dirty_since
    }

    fn at(&self, x: u32, y: u32) -> Option<(u8, u16)> {
        if x >= self.size || y >= self.size {
            return None;
        }
        self.wet.get((y as usize) * (self.size as usize) + x as usize).copied()
    }

    /// **Stamp a stain at a UV, at one sample per texel.** The coverage mask comes from
    /// `crate::bloodstain::stain::rasterise`, so a stain's silhouette is derived from its own impact rather
    /// than picked from a texture set.
    ///
    /// The mask's edge length in texels is `shape.major` scaled by [`UV_SPAN_M`] — at least one texel,
    /// at most the whole canvas. Coverage **accumulates** (saturating at full), because accumulation
    /// is the thing a wetmap is for.
    ///
    /// Arriving blood is age 0, so a repainted texel is young again. That is the same rule the drip and
    /// spread passes use: **the youngest blood present decides**, because it is the youngest blood that
    /// decides whether the texel is still wet enough to move.
    ///
    /// UVs outside `[0, 1]` are clamped to the edge, matching Bevy's default `ClampToEdge` sampler: a
    /// tiling atlas would otherwise paint an arbitrary texel and look like a bug somewhere else.
    ///
    /// A caller holding [`WetSettings`] should use [`paint_uv_with`](Self::paint_uv_with), which is
    /// the same stamp at the dialled [`WetSettings::edge_samples`]; this entry point is that stamp at
    /// one sample, which is the shipped value.
    pub fn paint_uv(&mut self, uv: Vec2, shape: &StainShape, tick: u32) {
        self.stamp(uv, shape, tick, 1);
    }

    /// **[`paint_uv`](Self::paint_uv) at the caller's [`WetSettings::edge_samples`].**
    ///
    /// The only dial a stamp reads, and it reads it here rather than from a copy on the canvas, for
    /// the same reason [`tick`](Self::tick) takes the settings: one authority, held by the caller.
    pub fn paint_uv_with(&mut self, uv: Vec2, shape: &StainShape, tick: u32, s: &WetSettings) {
        self.stamp(uv, shape, tick, s.edge_span());
    }

    /// **Blood wicking through a weave**: wet every texel inside `shape`'s silhouette, frayed by the
    /// threads, to a share of full coverage that falls from the centre to the front — and never dry a
    /// texel, since a front that has passed leaves the blood behind it.
    ///
    /// A stamp ([`paint_uv`](Self::paint_uv)) is a droplet's silhouette and adds its coverage; repeated
    /// to grow a soak, it adds up to a flat, full patch with an edge as clean as a cut-out, which is not
    /// what blood in cloth looks like. Blood in cloth travels along the yarns, so its front is ragged at
    /// the scale of the weave and reaches further along the threads than across them. Here the front
    /// is the ellipse `shape` describes, measured with a `p = 1.85` norm on the canvas's axes, which
    /// stretches it along `u` and `v` — **the warp and the weft, on a garment whose UVs follow the
    /// grain** — and moved in and out by a two-octave value noise over the texel lattice keyed by
    /// `shape.seed`. Coverage is [`WICK_CORE`] at the centre and a little over half that toward the
    /// front, feathered to nothing over its outer third, and mottled ±28 % per texel by a hash of the
    /// texel alone — the yarn it falls on, which every landing agrees about. A texel takes the larger
    /// of what it held and what the front brings, so the call can be repeated as the front grows; a
    /// texel it raises is fresh blood again, age 0. The norm, the noise and the coverages are this
    /// crate's own.
    pub fn wick_uv(&mut self, uv: Vec2, shape: &StainShape, tick: u32) {
        if !uv.is_finite() || !shape.major.is_finite() || !shape.minor.is_finite() || shape.major <= 0.0 || shape.minor <= 0.0 {
            return;
        }
        let size = self.size as f32;
        let n = self.size as i64;
        // Semi-axes in texels, and the reach the noise can push the front out to.
        let (a, b) = (0.5 * shape.major / UV_SPAN_M * size, 0.5 * shape.minor / UV_SPAN_M * size);
        let reach = (a.max(b) * 1.4).ceil() as i64 + 1;
        let dir = Vec2::new(shape.direction[0], shape.direction[1]).normalize_or(Vec2::Y);
        let (cx, cy) = (uv.x.clamp(0.0, 1.0) * size, uv.y.clamp(0.0, 1.0) * size);
        let (ix, iy) = (cx as i64, cy as i64);
        let seed = shape.seed;
        let mut touched = false;
        for y in (iy - reach).max(0)..(iy + reach + 1).min(n) {
            for x in (ix - reach).max(0)..(ix + reach + 1).min(n) {
                let d = Vec2::new(x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                // Into the silhouette's frame: `along` the major axis, `across` it, both over their axis.
                let along = d.dot(dir) / a.max(1.0e-3);
                let across = d.perp_dot(dir) / b.max(1.0e-3);
                let p = 1.85;
                let r = libm::powf(libm::powf(along.abs(), p) + libm::powf(across.abs(), p), 1.0 / p);
                let front = 0.66 + 0.68 * weave(x, y, seed);
                let s = r / front;
                if s >= 1.0 {
                    continue;
                }
                // Feathered over the front's outer third, and mottled by the yarn a texel falls on —
                // a fixed property of the cloth, keyed by the texel alone, so every landing on it
                // agrees which yarns take more.
                let feather = 1.0 - smoothstep01((s - 0.66) / 0.34);
                let yarn = 0.72 + 0.56 * crate::bloodstain::hash_f32((x as u32).wrapping_mul(0x27D4_EB2F) ^ (y as u32).wrapping_mul(0x1656_67B1));
                let target = (WICK_CORE * (1.0 - 0.45 * s * s) * feather * yarn).clamp(0.0, 1.0);
                let target = (target * 255.0).round() as u8;
                let Some(cell) = self.wet.get_mut((y * n + x) as usize) else {
                    continue;
                };
                if target <= cell.0 {
                    continue;
                }
                *cell = (target, 0);
                touched = true;
                self.live = Some(self.live.map_or(Live::texel(x, y), |l| l.with(x, y)));
            }
        }
        if touched {
            self.quiet = None;
            if self.dirty_since.is_none() {
                self.dirty_since = Some(tick);
            }
        }
    }

    /// **A sole put down in blood picks some of it up**: take `share` of the wet blood under `print`'s
    /// outline at `uv`, and return how much was there, as the mean coverage under the outline from `0`
    /// to `1`. Dry blood is neither taken nor counted — **dry paint does not move** — so a sole crossing
    /// an old, dried pool comes away clean. A texel left with nothing is age 0 again.
    ///
    /// Each texel gives up `share` of what it holds, rounded, so the pool keeps the sole's shape as a
    /// lighter dent until its own spread fills it. Call it before [`print_uv`](Self::print_uv) for the
    /// same step, or a sole would pick its own print back up and never walk it off.
    pub fn lift_uv(&mut self, uv: Vec2, print: &Print, share: f32, tick: u32, s: &WetSettings) -> f32 {
        let share = if share.is_finite() { share.clamp(0.0, 1.0) } else { 0.0 };
        let span = s.dry_span();
        let n = self.size as i64;
        let (mut held, mut under) = (0u64, 0u64);
        let mut touched = false;
        let wet = &mut self.wet;
        each_sole_texel(self.size, uv, print, |x, y, _, _, _| {
            under += 1;
            let Some(cell) = wet.get_mut((y * n + x) as usize) else {
                return;
            };
            if cell.0 == 0 || !is_wet(cell.1, span) {
                return;
            }
            held += u64::from(cell.0);
            let take = byte(f32::from(cell.0) * share);
            if take > 0 {
                cell.0 = cell.0.saturating_sub(take);
                if cell.0 == 0 {
                    cell.1 = 0;
                }
                touched = true;
            }
        });
        if touched {
            self.quiet = None;
            if self.dirty_since.is_none() {
                self.dirty_since = Some(tick);
            }
        }
        if under == 0 { 0.0 } else { held as f32 / (255.0 * under as f32) }
    }

    /// **A bloody sole put down**: ink `print`'s outline at `uv` with `load` of a full print — `1` for a
    /// sole that has just left a pool, falling toward `0` as it walks the blood off.
    ///
    /// What inks is what the sole pressed hardest and what its tread holds blood in: the outline's
    /// rim and the arch press less than the heel and the ball, and [`tread`] — a value noise fixed to the sole's own frame
    /// and [`Print::seed`] — decides which patches give up blood first, so the share of the sole that
    /// inks follows the load: a full load prints the whole sole save a few gaps at its rim, a half load
    /// about half of it in patches, and a light one a scatter of flecks, and one shoe leaves the same
    /// gaps every step. Where it inks, a texel holds [`PRINT_CORE`] of full coverage at a full load and
    /// six tenths of that at none, so a fading print is paler as well as sparser, feathered over the
    /// outline's last tenth. A texel takes the larger of what it held and what the print brings, so a
    /// print on a pool adds nothing to it, and a texel it raises is fresh blood, age 0. The outline,
    /// the pressure and the noise are this crate's own.
    pub fn print_uv(&mut self, uv: Vec2, print: &Print, load: f32, tick: u32) {
        let load = if load.is_finite() { load.clamp(0.0, 1.0) } else { 0.0 };
        if load <= 0.0 {
            return;
        }
        let n = self.size as i64;
        let mut touched = false;
        let (wet, live) = (&mut self.wet, &mut self.live);
        each_sole_texel(self.size, uv, print, |x, y, reach, along, across| {
            let pressed = load * (1.2 - 0.4 * smoothstep01((reach - 0.5) / 0.5)) * Print::arch(along);
            let ink = smoothstep01((pressed - tread(along, across, print.seed)) * 6.0 + 0.5);
            let feather = 1.0 - smoothstep01((reach - 0.9) / 0.1);
            let target = byte(PRINT_CORE * (0.6 + 0.4 * load) * ink * feather * 255.0);
            let Some(cell) = wet.get_mut((y * n + x) as usize) else {
                return;
            };
            if target <= cell.0 {
                return;
            }
            *cell = (target, 0);
            touched = true;
            *live = Some(live.map_or(Live::texel(x, y), |l| l.with(x, y)));
        });
        if touched {
            self.quiet = None;
            if self.dirty_since.is_none() {
                self.dirty_since = Some(tick);
            }
        }
    }

    /// The one stamp. `span` subsamples per texel axis; `1` is the shipped rasterisation to the byte.
    ///
    /// The mask is rasterised at `span` times the canvas resolution and box-filtered down, so a texel
    /// the silhouette only clips receives the share of it that is actually inside. At `span = 1` the
    /// filter is the identity — one sample, divisor one, rounding term zero — which is what keeps
    /// every frozen digest in this crate exactly where it was.
    ///
    /// `span` is capped so the scratch cannot exceed a 2048-texel edge: at the shipped 128-texel
    /// canvas that is never reached, and on a canvas large enough to reach it a subtexel edge is not
    /// what is limiting the look.
    fn stamp(&mut self, uv: Vec2, shape: &StainShape, tick: u32, span: u32) {
        if !uv.is_finite() {
            return;
        }
        let px = self.mask_px(shape);
        let span = span.clamp(1, 8).min((2048 / px.max(1)).max(1));
        let fine_px = px.saturating_mul(span);
        let fine_need = (fine_px as usize) * (fine_px as usize);
        if self.fine.len() != fine_need {
            self.fine.resize(fine_need, 0);
        }
        if !rasterise(shape, fine_px, &mut self.fine) {
            return;
        }
        let need = (px as usize) * (px as usize);
        if self.mask.len() != need {
            self.mask.resize(need, 0);
        }
        let taps = (span * span).max(1);
        let round = taps / 2;
        let (px_u, span_u, fine_u) = (px as usize, span as usize, fine_px as usize);
        for my in 0..px_u {
            for mx in 0..px_u {
                let mut sum = 0u32;
                for sy in 0..span_u {
                    for sx in 0..span_u {
                        let i = (my * span_u + sy) * fine_u + mx * span_u + sx;
                        // In bounds by construction; `get` rather than `[]` so the crate holds no
                        // panicking index at all.
                        if let Some(&v) = self.fine.get(i) {
                            sum += v as u32;
                        }
                    }
                }
                if let Some(slot) = self.mask.get_mut(my * px_u + mx) {
                    *slot = ((sum + round) / taps).min(255) as u8;
                }
            }
        }

        let n = self.size as i64;
        let cx = ((uv.x.clamp(0.0, 1.0) * self.size as f32) as i64).clamp(0, n - 1);
        let cy = ((uv.y.clamp(0.0, 1.0) * self.size as f32) as i64).clamp(0, n - 1);
        let px_i = px as i64;
        let half = px_i / 2;
        let mut touched = false;
        for my in 0..px_i {
            let y = cy + my - half;
            if y < 0 || y >= n {
                continue;
            }
            for mx in 0..px_i {
                let x = cx + mx - half;
                if x < 0 || x >= n {
                    continue;
                }
                let Some(&cov) = self.mask.get((my * px_i + mx) as usize) else {
                    continue;
                };
                if cov == 0 {
                    continue;
                }
                // In bounds by the two guards above; `get_mut` rather than `[]` so the crate holds no
                // panicking index at all.
                let Some(cell) = self.wet.get_mut((y * n + x) as usize) else {
                    continue;
                };
                cell.0 = cell.0.saturating_add(cov);
                cell.1 = 0;
                touched = true;
                self.live = Some(self.live.map_or(Live::texel(x, y), |l| l.with(x, y)));
            }
        }
        if touched {
            self.quiet = None;
            if self.dirty_since.is_none() {
                self.dirty_since = Some(tick);
            }
        }
    }

    /// **Stamp a stain where a world-space ray hits this actor.**
    ///
    /// Möller–Trumbore over the mesh's `ATTRIBUTE_POSITION` and index buffer, then a barycentric read
    /// of `ATTRIBUTE_UV_0`. The ray is moved into mesh space by one inverse transform rather than the
    /// geometry being moved into world space.
    ///
    /// **Returns the UV it painted at**, so a caller can paint the same spot again by
    /// [`paint_uv_with`](Self::paint_uv_with) after the actor has moved and the ray no longer lands
    /// there. `None` when the ray misses, and `None` with **one warning per mesh** when the mesh
    /// cannot carry a wetmap at all — no `Float32x2` UV0, no `Float32x3` positions, or not a triangle
    /// list. That refusal is the point: a mesh without UVs makes every drop of blood a caller paints
    /// land nowhere, and reporting a spot after painting nothing would hide it.
    pub fn paint_world(
        &mut self,
        mesh: &Mesh,
        xf: &GlobalTransform,
        from: Vec3,
        dir: Vec3,
        shape: &StainShape,
        tick: u32,
    ) -> Option<Vec2> {
        self.cast(mesh, xf, from, dir, shape, tick, 1)
    }

    /// **[`paint_world`](Self::paint_world) at the caller's [`WetSettings::edge_samples`].**
    ///
    /// Same ray, same stamp; the dial is the caller's, for the reason
    /// [`paint_uv_with`](Self::paint_uv_with) gives.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_world_with(
        &mut self,
        mesh: &Mesh,
        xf: &GlobalTransform,
        from: Vec3,
        dir: Vec3,
        shape: &StainShape,
        tick: u32,
        s: &WetSettings,
    ) -> Option<Vec2> {
        self.cast(mesh, xf, from, dir, shape, tick, s.edge_span())
    }

    /// The one ray. See [`paint_world`](Self::paint_world) for the contract.
    #[allow(clippy::too_many_arguments)]
    fn cast(
        &mut self,
        mesh: &Mesh,
        xf: &GlobalTransform,
        from: Vec3,
        dir: Vec3,
        shape: &StainShape,
        tick: u32,
        span: u32,
    ) -> Option<Vec2> {
        let inv = xf.affine().inverse();
        let origin = inv.transform_point3(from);
        let local_dir = inv.transform_vector3(dir);
        match ray_uv(mesh, origin, local_dir) {
            Pick::At(uv) => {
                self.stamp(uv, shape, tick, span);
                Some(uv)
            }
            Pick::Miss => None,
            Pick::Unusable => {
                let key = mesh_key(mesh);
                if !self.warned.contains(&key) {
                    if self.warned.len() >= WARN_MEMO {
                        self.warned.remove(0);
                    }
                    self.warned.push(key);
                    warn!(
                        "wetmap: this mesh has no Float32x2 ATTRIBUTE_UV_0 (or is not a triangle \
                         list), so it cannot carry a wetmap; nothing was painted"
                    );
                }
                None
            }
        }
    }

    /// **The state machine, in exactly four passes and exactly this order.**
    ///
    /// 1. **Drip.** A wet texel holding more than `drip_rate` sheds the *excess* one texel along
    ///    `gravity_uv` and keeps the rest: one texel down gravity's dominant axis, and on a share of
    ///    the lines it falls through one across as well, for gravity's lean (`DripStep::across`).
    ///    What leaves one texel arrives in exactly one other, or is lost at the border — nothing
    ///    else, because every texel of a line steps alike, so each line moves as a translation and
    ///    at most one texel can ever step into another. **Conserved exactly wherever the destination
    ///    is also wet:** a
    ///    wet destination sheds down to the threshold, so its residue is at most `threshold` and the
    ///    parcel is at most `255 − threshold`, which sums to exactly 255. The one lossy case is blood
    ///    running onto a *dry crust that is already saturated*, and there the loss is the model rather
    ///    than a rounding error: `amount` is normalised coverage, a texel at 255 is fully covered, and
    ///    more blood on it is not representable because it is not visible.
    /// 2. **Spread.** `spread_rate` diffuses into the 4-neighbourhood, written as an antisymmetric flux
    ///    on each edge's coverage difference: `round(spread_rate/4 · (aᵢ − aⱼ))`. `f32::round` is odd,
    ///    so the flux one way is exactly minus the flux the other, and mass is conserved to the byte.
    /// 3. **Time.** `age` increments and the substrate takes its cut — see
    ///    [`WetSettings::absorbency`]. Age stops at `dry_ticks`, which makes a fully dried canvas a
    ///    **fixed point** of this function: nothing moves, no byte changes, and no upload is asked for.
    /// 4. **Shade.** `crate::bloodstain::dry::appearance` writes the sRGB and roughness bytes.
    ///
    /// **Dry paint does not move.** Wetness gates every change to `amount` in passes 1–3, which is
    /// what makes a run stop where it stopped rather than creeping forever, and what stops a dried
    /// crust from soaking away.
    ///
    /// Both moving passes read a snapshot taken at their own top and write disjoint slots, so the
    /// result cannot depend on traversal order. That property is the whole reason
    /// [`digest`](Self::digest) is worth taking.
    ///
    /// `gravity_uv` is in **UV space**, not world space: which way is down on a texture is a property
    /// of the atlas, and only the caller knows it.
    pub fn tick(&mut self, tick: u32, gravity_uv: Vec2, s: &WetSettings) {
        // A fixed point under these settings: the four passes would change nothing. See `quiet`.
        if self.quiet.as_ref() == Some(s) {
            return;
        }
        let n = self.size as i64;
        // Where blood can be after this tick: the live rectangle grown by the distance one tick can
        // move it. A canvas with no blood has nothing to move or shade. See `live`.
        let region = self.live.map(|l| l.grown(TICK_REACH, n));
        if let Some(region) = region {
            self.drip(region, gravity_uv, s);
            self.spread(region, s);
            self.advance(region, s);
            self.shade(region, tick, s);
        }
        let span = s.dry_span();
        let mut moving = false;
        let mut live: Option<Live> = None;
        if let Some(region) = region {
            for y in region.y0..=region.y1 {
                for x in region.x0..=region.x1 {
                    let Some(&(amount, age)) = self.wet.get((y * n + x) as usize) else { continue };
                    if amount > 0 {
                        moving |= is_wet(age, span);
                        live = Some(live.map_or(Live::texel(x, y), |l| l.with(x, y)));
                    }
                }
            }
        }
        self.live = live;
        self.quiet = (!moving).then(|| s.clone());
    }

    /// Whether the last [`tick`](Self::tick) found this canvas still — every bloody texel dried, or no
    /// blood at all — so the next tick under the same settings will cost nothing. A paint wakes it.
    pub fn is_still(&self) -> bool {
        self.quiet.is_some()
    }

    /// **Upload, and the only place `Assets<Image>` is touched.**
    ///
    /// Returns `false` and writes nothing when the canvas is clean, so a still scene costs no
    /// bandwidth. The per-frame budget *across* canvases is the plugin's — see
    /// [`WetSettings::max_canvas_updates_per_tick`].
    ///
    /// Also returns `false` without clearing the dirty flag if either image has gone or has been
    /// resized under it, so the next flush retries rather than the canvas quietly diverging forever.
    pub fn flush(&mut self, images: &mut Assets<Image>) -> bool {
        if self.dirty_since.is_none() {
            return false;
        }
        self.redraw();
        let (albedo, rough) = self.images_px();
        let ok = upload(images, &self.albedo_handle, albedo) && upload(images, &self.rough_handle, rough);
        if ok {
            self.dirty_since = None;
        }
        ok
    }

    /// **Redraw the detail images over the tiles whose texels changed, and forget them.** Nothing at
    /// detail 1, where the images are the texels and no tile is ever marked. See
    /// [`new_detailed`](Self::new_detailed).
    ///
    /// A run of marked tiles along a tile row is redrawn as one rectangle, so a stain costs what its
    /// tiles cover rather than what the box around every stain on the canvas would.
    fn redraw(&mut self) {
        if self.detail <= 1 {
            return;
        }
        let n = self.size as i64;
        let tiles = (n + STALE_TILE - 1) / STALE_TILE;
        for ty in 0..tiles {
            let mut tx = 0;
            while tx < tiles {
                let marked = |s: &mut Vec<bool>, tx: i64| s.get_mut((ty * tiles + tx) as usize).map(std::mem::take).unwrap_or(false);
                if !marked(&mut self.stale, tx) {
                    tx += 1;
                    continue;
                }
                let start = tx;
                tx += 1;
                while tx < tiles && marked(&mut self.stale, tx) {
                    tx += 1;
                }
                let last = n - 1;
                self.redraw_rect(Live {
                    x0: start * STALE_TILE,
                    y0: ty * STALE_TILE,
                    x1: (tx * STALE_TILE - 1).min(last),
                    y1: ((ty + 1) * STALE_TILE - 1).min(last),
                });
            }
        }
    }

    /// **Redraw the detail images' pixels that read any texel of `stale`.**
    ///
    /// Separable, in three passes over scratch: each texel's six coverage-weighted values once; along
    /// each texel row, every pixel column's four-texel sum; down the columns, every pixel's sum of
    /// four of those, coverage first — over the pixels to write and one more on every side, because
    /// the edge is anti-aliased by the coverage's change from one pixel to the next — and the other
    /// five only where there is blood to draw. The scratch rectangles reach two texels past the
    /// canvas on every side where they meet it, filled with the edge texel's values the way a
    /// `ClampToEdge` sampler would, so no tap is ever out of range.
    fn redraw_rect(&mut self, stale: Live) {
        let n = self.size as i64;
        let k = self.detail as i64;
        let nf = n * k;
        // A pixel reads the two texels on either side of it, so a changed texel moves pixels up to
        // two texels away.
        let r = stale.grown(2, n);
        let (px0, py0, px1, py1) = (r.x0 * k, r.y0 * k, r.x1 * k + k - 1, r.y1 * k + k - 1);
        let (gx0, gy0, gx1, gy1) = ((px0 - 1).max(0), (py0 - 1).max(0), (px1 + 1).min(nf - 1), (py1 + 1).min(nf - 1));
        let w = (gx1 - gx0 + 1) as usize;
        // The texels those pixels read, unclamped: `tx0..=tx1` by `ty0..=ty1`.
        let (tx0, ty0, tx1, ty1) = (gx0 / k - 2, gy0 / k - 2, gx1 / k + 2, gy1 / k + 2);
        let tw = (tx1 - tx0 + 1) as usize;

        // Each of the `k` positions a pixel can have in its texel: the first of its four texels,
        // relative to its own, and their weights.
        let mut taps = [(0i64, [0.0f32; 4]); MAX_DETAIL as usize];
        for (j, tap) in taps.iter_mut().enumerate().take(k as usize) {
            let u = (j as f32 + 0.5) / k as f32 - 0.5;
            let i0 = u.floor();
            *tap = (i0 as i64 - 1, bspline(u - i0));
        }
        let tap = |p: i64| taps.get(p.rem_euclid(k) as usize).copied().unwrap_or((0, [0.0; 4]));

        // Each texel's coverage, and its colour, roughness and wetness weighted by its coverage.
        self.texels.clear();
        let pre = &mut self.texels;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let i = (ty.clamp(0, n - 1) * n + tx.clamp(0, n - 1)) as usize;
                let amount = self.wet.get(i).map_or(0, |c| c.0);
                let (Some(c), Some(g)) = (self.albedo_px.get(i * 4..i * 4 + 4), self.rough_px.get(i * 4..i * 4 + 4)) else {
                    pre.extend([0.0; 6]);
                    continue;
                };
                let a = amount as f32;
                pre.extend([a, a * c[0] as f32, a * c[1] as f32, a * c[2] as f32, a * g[1] as f32, a * g[3] as f32]);
            }
        }
        // Along the rows: per texel row and pixel column, the four texels' weighted sums.
        self.rows.clear();
        for ty in 0..(ty1 - ty0 + 1) as usize {
            let row = self.texels.get(ty * tw * 6..(ty + 1) * tw * 6).unwrap_or(&[]);
            for x in gx0..=gx1 {
                let (o, wt) = tap(x);
                let first = ((x / k + o - tx0) * 6) as usize;
                let mut acc = [0.0f32; 6];
                if let Some(texels) = row.get(first..first + 24) {
                    for (texel, wt) in texels.as_chunks::<6>().0.iter().zip(wt) {
                        for (slot, v) in acc.iter_mut().zip(texel) {
                            *slot += wt * v;
                        }
                    }
                }
                self.rows.push(acc);
            }
        }
        // The four rows of sums a pixel row reads, and their weights.
        let rows = &self.rows;
        let four = |y: i64| -> ([&[[f32; 6]]; 4], [f32; 4]) {
            let (o, wt) = tap(y);
            let row = |t: i64| {
                let first = ((y / k + o + t - ty0) as usize) * w;
                rows.get(first..first + w).unwrap_or(&[])
            };
            ([row(0), row(1), row(2), row(3)], wt)
        };
        // Down the columns: the coverage at every pixel the edge's gradient reads.
        self.cover.clear();
        for y in gy0..=gy1 {
            let ([a, b, c, d], wt) = four(y);
            for (((a, b), c), d) in a.iter().zip(b).zip(c).zip(d) {
                self.cover.push(wt[0] * a[0] + wt[1] * b[0] + wt[2] * c[0] + wt[3] * d[0]);
            }
        }

        let cover = &self.cover;
        let at = |x: i64, y: i64| cover.get(((y.clamp(gy0, gy1) - gy0) as usize) * w + (x.clamp(gx0, gx1) - gx0) as usize).copied().unwrap_or(0.0);
        let edge = DETAIL_EDGE * 255.0;
        let base = self.base_rgba;
        let base_rough = [0, self.base_rough, 0, 0];
        for y in py0..=py1 {
            let ([ra, rb, rc, rd], wt) = four(y);
            for x in px0..=px1 {
                let c = at(x, y);
                let m = if c > 0.0 {
                    // Anti-aliased over one pixel: how far the coverage is past the edge, in units
                    // of how much it changes from one pixel to the next.
                    let (dx, dy) = (0.5 * (at(x + 1, y) - at(x - 1, y)), 0.5 * (at(x, y + 1) - at(x, y - 1)));
                    ((c - edge) / (dx * dx + dy * dy).sqrt().max(1.0e-3) + 0.5).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (albedo, rough) = if m > 0.0 {
                    let col = (x - gx0) as usize;
                    let mut s = [0.0f32; 6];
                    for (row, wt) in [ra, rb, rc, rd].iter().zip(wt) {
                        if let Some(v) = row.get(col) {
                            for (slot, v) in s.iter_mut().zip(v) {
                                *slot += wt * v;
                            }
                        }
                    }
                    let mean = |v: f32| v / s[0].max(1.0e-6);
                    let mix = |from: u8, to: f32| byte(from as f32 + (to - from as f32) * m);
                    (
                        [mix(base[0], mean(s[1])), mix(base[1], mean(s[2])), mix(base[2], mean(s[3])), base[3]],
                        [byte(c * m), mix(self.base_rough, mean(s[4])), 0, byte(mean(s[5]) * m)],
                    )
                } else {
                    (base, base_rough)
                };
                let i = ((y * nf + x) * 4) as usize;
                if let Some(px) = self.detail_albedo.get_mut(i..i + 4) {
                    px.copy_from_slice(&albedo);
                }
                if let Some(px) = self.detail_rough.get_mut(i..i + 4) {
                    px.copy_from_slice(&rough);
                }
            }
        }
    }

    /// **FNV-1a over the wet buffer** — `amount` then `age` little-endian, row-major.
    ///
    /// Over the *buffer*, not the images: see [`crate::wetmap::digest`] for why the CPU state is the authority
    /// and the uploaded pixels are not.
    pub fn digest(&self) -> u64 {
        let mut f = Fnv1a::new();
        for &(amount, age) in &self.wet {
            f.byte(amount);
            f.u16(age);
        }
        f.finish()
    }

    /// Wetted area, m² under [`UV_SPAN_M`].
    ///
    /// The coverage *sum* rather than a texel count, so a faint mist and a saturated pool with the same
    /// footprint do not read the same. Rises with paint and falls as the substrate absorbs.
    pub fn wetted_area(&self) -> f32 {
        let mut sum: u64 = 0;
        for &(amount, _) in &self.wet {
            sum += amount as u64;
        }
        let side = self.size as f32;
        (sum as f32 / 255.0) * (UV_SPAN_M * UV_SPAN_M / (side * side))
    }

    /// Mask edge length in texels for a stain of this size.
    fn mask_px(&self, shape: &StainShape) -> u32 {
        let major = if shape.major.is_finite() { shape.major.max(0.0) } else { 0.0 };
        let px = (major / UV_SPAN_M * self.size as f32).round();
        (px as i64).clamp(1, self.size as i64) as u32
    }

    /// Pass 1, over `region`. See [`tick`](Self::tick).
    fn drip(&mut self, region: Live, gravity_uv: Vec2, s: &WetSettings) {
        let Some(step) = drip_step(gravity_uv) else {
            return;
        };
        let thr = s.drip_threshold();
        let span = s.dry_span();
        self.prev.copy_from_slice(&self.wet);
        let n = self.size as i64;
        let ((sx, sy), (mx, my)) = (step.major, step.minor);
        for y in region.y0..=region.y1 {
            for x in region.x0..=region.x1 {
                let Some(&(amount, age)) = self.prev.get((y * n + x) as usize) else {
                    continue;
                };
                let leaves = if is_wet(age, span) { amount.saturating_sub(thr) } else { 0 };
                let residue = amount - leaves;
                // Exactly one texel can step into this one: every texel of a line across the fall
                // steps alike, so each line moves as a translation. So there is no accumulation here
                // and no order in which two arrivals could race.
                let line = if sx != 0 { x - sx } else { y - sy };
                let across = step.across(line);
                let (ax, ay) = (x - sx - across * mx, y - sy - across * my);
                let arrival_in_bounds = ax >= 0 && ax < n && ay >= 0 && ay < n;
                let (arriving, arriving_age) = match self.prev.get((ay * n + ax) as usize) {
                    Some(&(aj, agej)) if arrival_in_bounds && is_wet(agej, span) => {
                        (aj.saturating_sub(thr), agej)
                    }
                    // Off the border, or a dry crust upstream: nothing arrives. A parcel that steps
                    // off the canvas is LOST, which is the border rule stated in `tick`.
                    _ => (0, 0),
                };
                let new_amount = (residue as u32 + arriving as u32).min(255) as u8;
                let new_age = if new_amount == 0 {
                    0
                } else if residue == 0 {
                    arriving_age
                } else if arriving == 0 {
                    age
                } else {
                    age.min(arriving_age)
                };
                if let Some(cell) = self.wet.get_mut((y * n + x) as usize) {
                    *cell = (new_amount, new_age);
                }
            }
        }
    }

    /// Pass 2, over `region`. See [`tick`](Self::tick).
    fn spread(&mut self, region: Live, s: &WetSettings) {
        let k = s.spread_rate.clamp(0.0, 1.0) * 0.25;
        if !k.is_finite() || k <= 0.0 {
            return;
        }
        let span = s.dry_span();
        self.prev.copy_from_slice(&self.wet);
        let n = self.size as i64;
        // Fixed E/W/S/N order. Nothing here reduces across texels, so the order cannot change the
        // answer — it is fixed anyway, because a neighbour order that varied would be one more thing
        // to have to reason about later.
        const NEIGHBOURS: [(i64, i64); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        for y in region.y0..=region.y1 {
            for x in region.x0..=region.x1 {
                let Some(&(amount, age)) = self.prev.get((y * n + x) as usize) else {
                    continue;
                };
                // A dry crust neither gives nor receives: nothing flows through set blood.
                if !is_wet(age, span) {
                    continue;
                }
                let mut net: i32 = 0;
                let mut youngest = age;
                for (dx, dy) in NEIGHBOURS {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || nx >= n || ny < 0 || ny >= n {
                        // No-flux boundary: the share that would have left stays put rather than
                        // draining off the edge of the atlas.
                        continue;
                    }
                    let Some(&(aj, agej)) = self.prev.get((ny * n + nx) as usize) else {
                        continue;
                    };
                    if !is_wet(agej, span) {
                        continue;
                    }
                    // Odd in its argument, and the wet/bounds gate above is symmetric in (i, j), so
                    // `flux(i→j) == −flux(j→i)` exactly. That is the whole conservation argument.
                    let flux = (k * (amount as f32 - aj as f32)).round() as i32;
                    net -= flux;
                    if flux < 0 {
                        youngest = youngest.min(agej);
                    }
                }
                // Unreachable at the shipped `spread_rate`; reachable near `spread_rate = 1`, where
                // four saturated neighbours can offer more than a texel can hold.
                let new_amount = (amount as i32 + net).clamp(0, 255) as u8;
                if let Some(cell) = self.wet.get_mut((y * n + x) as usize) {
                    *cell = (new_amount, if new_amount == 0 { 0 } else { youngest });
                }
            }
        }
    }

    /// Pass 3, over `region`. See [`tick`](Self::tick).
    fn advance(&mut self, region: Live, s: &WetSettings) {
        let span = s.dry_span();
        let ceiling = span as u16;
        let n = self.size as i64;
        for y in region.y0..=region.y1 {
            for x in region.x0..=region.x1 {
                let Some(cell) = self.wet.get_mut((y * n + x) as usize) else { continue };
                let (amount, age) = *cell;
                if amount == 0 {
                    *cell = (0, 0);
                    continue;
                }
                let new_age = age.saturating_add(1).min(ceiling);
                if !is_wet(age, span) {
                    // Set. Frozen: it does not move, does not soak, and its age has stopped — which
                    // is what makes a dried canvas a fixed point.
                    *cell = (amount, new_age);
                    continue;
                }
                let taken = s.absorbed_by(new_age as u32).saturating_sub(s.absorbed_by(age as u32));
                let new_amount = amount.saturating_sub(taken.min(255) as u8);
                *cell = (new_amount, if new_amount == 0 { 0 } else { new_age });
            }
        }
    }

    /// Pass 4, over `region`. See [`tick`](Self::tick).
    fn shade(&mut self, region: Live, tick: u32, s: &WetSettings) {
        let blood = s.blood();
        let span = s.dry_span();
        let base = self.base_rgba;
        let base_rough = self.base_rough;

        // **The fresh colour is a function of thickness, tabulated once per tick.** The coverage byte
        // is a film depth (see `WetSettings::film_depth_mm`), and `crate::bloodstain::spectral` turns a depth
        // into a colour through 81 wavelengths of Kubelka–Munk — far too much per texel, and
        // pointless: sixteen depth levels are indistinguishable from 255 at blood's own contrast.
        // The substrate the film lies on is this canvas's base albedo, as a grey.
        let substrate = luminance(base);
        let key = (s.film_depth_mm.to_bits(), s.so2.to_bits(), substrate.to_bits());
        if self.film_key != Some(key) {
            self.film_lut = core::array::from_fn(|i| {
                let thickness_mm = s.film_depth_mm.max(0.0) * (i as f32 + 0.5) / FILM_LEVELS as f32;
                crate::bloodstain::spectral::srgb(&Film { thickness_mm, so2: s.so2, substrate })
            });
            self.film_key = Some(key);
        }
        let fresh = self.film_lut;

        // One-slot memo on `(level, age)`. A stamp lands with one age across a contiguous blob and
        // its interior is one level, so a row-major walk hits this for almost every texel. Exact,
        // not quantised: the memo either matches or is recomputed.
        let mut memo = (usize::MAX, u32::MAX);
        let mut memo_srgb = [0u8; 3];
        let mut memo_rough = 0u8;
        let mut changed = false;

        let n = self.size as i64;
        for y in region.y0..=region.y1 {
            for x in region.x0..=region.x1 {
                let i = (y * n + x) as usize;
                let Some(&(amount, age)) = self.wet.get(i) else { continue };
                let (rgb, rough_byte) = if amount == 0 {
                    ([base[0], base[1], base[2]], base_rough)
                } else {
                    let level = (amount as usize * FILM_LEVELS) / 256;
                    if (level, age as u32) != memo {
                        memo = (level, age as u32);
                        // **`s.dry_ticks` is the single authority for the timeline.** `appearance`
                        // normalises the age it is given against `dry_ticks(area, hz)`, so the texel's
                        // age is rescaled onto `bloodstain`'s own reference span and the reference
                        // inputs are passed. Feeding the raw age with an unrelated area would leave two
                        // dials deciding one curve.
                        let scaled = ((age as u64 * DRY_REF_TICKS as u64) / span as u64) as u32;
                        let ap = appearance_with_fresh(scaled, 60, DRY_REF_AREA_M2, &blood, fresh[level]);
                        memo_srgb = [enc(ap.srgb[0]), enc(ap.srgb[1]), enc(ap.srgb[2])];
                        memo_rough = enc(ap.roughness);
                    }
                    // The albedo is the film's own colour: the substrate is already inside it,
                    // because a thin film transmits what it lies on — compositing it over the base a
                    // second time would count the surface twice. Roughness still blends, in the
                    // encoded space: the honest version is a `powf` each way per texel per tick and at
                    // these values the difference is under a code value. `rim`, `halo` and
                    // `craquelure` are deliberately unread — they need a shader, and this crate ships
                    // none.
                    (memo_srgb, over(base_rough, memo_rough, amount))
                };

                let mut touched = false;
                if let Some([r, g, b, _]) = self.albedo_px.get_mut(i * 4..i * 4 + 4)
                    && (*r != rgb[0] || *g != rgb[1] || *b != rgb[2])
                {
                    *r = rgb[0];
                    *g = rgb[1];
                    *b = rgb[2];
                    touched = true;
                }
                // **All four channels of the metallic-roughness image**, per
                // [`roughness`](Self::roughness): R the coverage byte, G the roughness, B the
                // metallic zero, A the wetness. R and A are the buffer's own two numbers written
                // where a shader can read them, so nothing has to be inferred from the albedo — and
                // they are still *output*: no pass that moves blood reads a pixel back, and the
                // detail redraw reads these bytes, never the images.
                let want = [amount, rough_byte, 0, wetness(amount, age, span)];
                if let Some(r_px) = self.rough_px.get_mut(i * 4..i * 4 + 4) {
                    for (slot, want) in r_px.iter_mut().zip(want) {
                        if *slot != want {
                            *slot = want;
                            touched = true;
                        }
                    }
                }
                if touched {
                    changed = true;
                    let tiles = (n + STALE_TILE - 1) / STALE_TILE;
                    if let Some(tile) = self.stale.get_mut(((y / STALE_TILE) * tiles + x / STALE_TILE) as usize) {
                        *tile = true;
                    }
                }
            }
        }

        if changed && self.dirty_since.is_none() {
            self.dirty_since = Some(tick);
        }
    }
}

/// An inclusive rectangle of texels, `x0..=x1` by `y0..=y1`. See [`WetCanvas::live`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Live {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Live {
    /// The one texel `(x, y)`.
    fn texel(x: i64, y: i64) -> Self {
        Self { x0: x, y0: y, x1: x, y1: y }
    }

    /// This rectangle widened to take in `(x, y)`.
    fn with(self, x: i64, y: i64) -> Self {
        Self { x0: self.x0.min(x), y0: self.y0.min(y), x1: self.x1.max(x), y1: self.y1.max(y) }
    }

    /// Grown by `by` texels on every side, clamped to a canvas `n` texels square.
    fn grown(self, by: i64, n: i64) -> Self {
        let last = (n - 1).max(0);
        Self {
            x0: (self.x0 - by).clamp(0, last),
            y0: (self.y0 - by).clamp(0, last),
            x1: (self.x1 + by).clamp(0, last),
            y1: (self.y1 + by).clamp(0, last),
        }
    }
}

/// **How far one tick can move blood, in texels: two.** Drip moves a parcel one texel, or one down and
/// one across when it leans, which is still one along each axis; spread,
/// in the same tick and after it, reaches the 4-neighbourhood of where the parcel landed. A margin of
/// one was tried first and lost the second step's share: `the_live_rectangle_changes_nothing_but_the_cost`
/// caught it on a spined stain at tick 0.
const TICK_REACH: i64 = 2;

/// **The tile a detailed canvas tracks changes in, texels square: 32.** A redraw reads two texels
/// past whatever it redraws, so a smaller tile spends more of its work on that apron (at 32 it is an
/// eighth more along each edge) and a larger one redraws more untouched surface around a stain.
/// This crate's own number.
const STALE_TILE: i64 = 32;

/// **The finest a canvas's images may be drawn: four pixels per texel along each edge**, sixteen to
/// a texel. At 128 that is the bytes of a 512 canvas, which [`WetCanvas::new`] already calls a
/// bandwidth budget rather than a texture; past it the redraw costs more than the look gains.
pub const MAX_DETAIL: u32 = 4;

/// **Where a detailed image draws a stain's edge**: at `0.04` of full coverage in the smoothed map.
/// The same fraction `flesh.wgsl` draws its rim at (`WET_EDGE`), so a canvas looks the same through
/// either. This crate's own number: low enough to keep a thin smear, high enough that the spline's
/// faint tail past a stain is not drawn as a halo.
pub const DETAIL_EDGE: f32 = 0.04;

/// The cubic B-spline weights of the four texels around a sample at fraction `f` between the middle
/// two. They sum to one and none is negative, so a smoothed coverage never overshoots.
fn bspline(f: f32) -> [f32; 4] {
    let g = 1.0 - f;
    let (f2, f3) = (f * f, f * f * f);
    [g * g * g / 6.0, (3.0 * f3 - 6.0 * f2 + 4.0) / 6.0, (-3.0 * f3 + 3.0 * f2 + 3.0 * f + 1.0) / 6.0, f3 / 6.0]
}

/// A `0..=255` value as a byte, rounded.
#[inline]
fn byte(v: f32) -> u8 {
    if v.is_finite() { v.round().clamp(0.0, 255.0) as u8 } else { 0 }
}

/// **The share of full coverage blood wicked into a weave holds at the centre of its front**: `0.7`.
/// Cloth holds blood in its yarns rather than as a film on top, so a soak is never a pool. This
/// crate's own number, read off how dark a soaked patch should be against a stamped drop.
pub const WICK_CORE: f32 = 0.7;

/// **The weave's irregularity at a texel**, in `[0, 1]`: value noise at six texels and at two,
/// weighted two to one, each lattice corner from [`crate::bloodstain::hash_f32`] of its coordinates
/// and `seed`. Smoothstepped between corners, so the front frays into lobes rather than speckle.
fn weave(x: i64, y: i64, seed: u32) -> f32 {
    let octave = |cell: i64, salt: u32| -> f32 {
        let (gx, gy) = (x.div_euclid(cell), y.div_euclid(cell));
        let (fx, fy) = ((x.rem_euclid(cell) as f32 + 0.5) / cell as f32, (y.rem_euclid(cell) as f32 + 0.5) / cell as f32);
        let corner = |i: i64, j: i64| {
            let key = (i as u32).wrapping_mul(0x9E37_79B9) ^ (j as u32).wrapping_mul(0x85EB_CA6B) ^ seed.wrapping_add(salt);
            crate::bloodstain::hash_f32(key)
        };
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let top = corner(gx, gy) + (corner(gx + 1, gy) - corner(gx, gy)) * sx;
        let bottom = corner(gx, gy + 1) + (corner(gx + 1, gy + 1) - corner(gx, gy + 1)) * sx;
        top + (bottom - top) * sy
    };
    (2.0 * octave(6, 0) + octave(2, 0x68E3_1DA4)) / 3.0
}

/// **A sole's outline on a canvas**, for [`WetCanvas::lift_uv`] and [`WetCanvas::print_uv`]: how long
/// and how wide it is, in UV units at [`UV_SPAN_M`] as a [`StainShape`]'s axes are, and which way its
/// toe points on the canvas.
///
/// The outline is a boot's: a squared heel block, a narrow waist under the arch, and a forefoot widest
/// at the ball that narrows toward the arch and rounds at the toe. Symmetric about its long axis, so a
/// left and a right print the same shape. The proportions are this crate's own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Print {
    /// Heel to toe, UV units.
    pub length: f32,
    /// Across the forefoot, UV units.
    pub width: f32,
    /// Heel to toe on the canvas; normalised where it is read, and `+v` if it is zero.
    pub heading: [f32; 2],
    /// The sole's tread: which of its patches give up blood first. Fixed to the sole, so every print
    /// of one shoe leaves the same gaps.
    pub seed: u32,
}

impl Print {
    /// Where `(along, across)` — fractions of the length and of the width from the print's centre —
    /// lies against the outline: under `1` inside, `1` on its edge.
    fn reach(along: f32, across: f32) -> f32 {
        // The heel: the back 27 % of the length, 72 % of the width, a superellipse of power three, so a
        // block with rounded corners rather than an oval.
        let (a, c) = (((along + 0.365) / 0.135).abs(), (across / 0.36).abs());
        let heel = libm::cbrtf(a * a * a + c * c * c);
        // The forefoot: the front 56 %, from 0.06 behind the middle to the toe, full width at the ball
        // and three quarters of it where it meets the arch.
        let toward_arch = ((0.14 - along) / 0.2).clamp(0.0, 1.0);
        let (a, c) = ((along - 0.22) / 0.28, across / (0.5 * (1.0 - 0.25 * toward_arch)));
        let forefoot = (a * a + c * c).sqrt();
        // The waist joining them under the arch, 40 % of the width: a superellipse of power four,
        // nearly a bar. Without it a print read as two drops, a heel and a ball.
        let (a, c) = ((along + 0.145) / 0.11, across / 0.2);
        let waist = ((a * a) * (a * a) + (c * c) * (c * c)).sqrt().sqrt();
        heel.min(forefoot).min(waist)
    }

    /// **How hard the sole presses at `along`**, as a share of the heel's and the ball's: the arch
    /// least, at `0.8`, rising to full over 4 % of the length either side of it — so the waist prints
    /// with the rest from a full sole, and is the first to go as the sole runs dry.
    fn arch(along: f32) -> f32 {
        let into = smoothstep01((along + 0.27) / 0.04) * (1.0 - smoothstep01((along + 0.06) / 0.04));
        1.0 - 0.2 * into
    }
}

/// **The share of full coverage a sole's print holds where it inks**: `0.6`. A print is the film a
/// sole carried, wiped off, never a pool, so it is lighter than the blood it walked out of. This
/// crate's own number.
pub const PRINT_CORE: f32 = 0.6;

/// **Visit every texel `print`'s outline centred at `uv` covers** on a `size`² canvas, with how far
/// out in the outline it lies ([`Print::reach`]) and where in the sole's frame: `along` toward the toe
/// and `across`, both as fractions of the sole's length and width.
fn each_sole_texel(size: u32, uv: Vec2, print: &Print, mut visit: impl FnMut(i64, i64, f32, f32, f32)) {
    let sane = |v: f32| v.is_finite() && v > 0.0;
    if !uv.is_finite() || !sane(print.length) || !sane(print.width) {
        return;
    }
    let edge = size as f32;
    let n = size as i64;
    // In texels, and no larger than the canvas: a sole bigger than that is a caller's unit error.
    let (len, wid) = ((print.length / UV_SPAN_M * edge).min(edge), (print.width / UV_SPAN_M * edge).min(edge));
    let toe = Vec2::new(print.heading[0], print.heading[1]).normalize_or(Vec2::Y);
    let reach = (0.5 * len.max(wid)).ceil() as i64 + 1;
    let (cx, cy) = (uv.x.clamp(0.0, 1.0) * edge, uv.y.clamp(0.0, 1.0) * edge);
    let (ix, iy) = (cx as i64, cy as i64);
    for y in (iy - reach).max(0)..(iy + reach + 1).min(n) {
        for x in (ix - reach).max(0)..(ix + reach + 1).min(n) {
            let d = Vec2::new(x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let along = d.dot(toe) / len.max(1.0e-3);
            let across = d.perp_dot(toe) / wid.max(1.0e-3);
            let r = Print::reach(along, across);
            if r < 1.0 {
                visit(x, y, r, along, across);
            }
        }
    }
}

/// **Which patches of a sole give up blood first**, in `[0, 1]`, low first: value noise in the sole's
/// own frame, eighteen cells heel to toe and six across — about a lug of a boot's tread each — with a
/// finer octave at a third of the weight, each lattice corner from [`crate::bloodstain::hash_f32`] of
/// its cell and `seed`. In the sole's frame rather than the canvas's, so it moves and turns with the
/// shoe. Fine, so that a fading print keeps its outline and goes pale and speckled: at eight cells
/// heel to toe whole patches of a half-loaded sole went missing and the rest read as a stray blot.
fn tread(along: f32, across: f32, seed: u32) -> f32 {
    let octave = |cells_along: f32, cells_across: f32, salt: u32| -> f32 {
        let (u, v) = ((along + 0.5) * cells_along, (across + 0.5) * cells_across);
        let (gu, gv) = (u.floor(), v.floor());
        let (fu, fv) = (u - gu, v - gv);
        let corner = |i: f32, j: f32| {
            let key = (i as i32 as u32).wrapping_mul(0x9E37_79B9) ^ (j as i32 as u32).wrapping_mul(0x85EB_CA6B) ^ seed.wrapping_add(salt);
            crate::bloodstain::hash_f32(key)
        };
        let (su, sv) = (fu * fu * (3.0 - 2.0 * fu), fv * fv * (3.0 - 2.0 * fv));
        let top = corner(gu, gv) + (corner(gu + 1.0, gv) - corner(gu, gv)) * su;
        let bottom = corner(gu, gv + 1.0) + (corner(gu + 1.0, gv + 1.0) - corner(gu, gv + 1.0)) * su;
        top + (bottom - top) * sv
    };
    // Two octaves of value noise bunch round a half; spread over 0.2 to 0.8, the share of a sole under
    // any level is close to that level, so the share that inks follows the load.
    smoothstep01(((2.0 * octave(18.0, 6.0, 0x51ED_270B) + octave(41.0, 13.0, 0x2C1B_3C6D)) / 3.0 - 0.2) / 0.6)
}

/// The cubic smoothstep over `[0, 1]`, clamped.
fn smoothstep01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A texel is wet until its age reaches the drying span. A texel with no blood in it has age 0 and is
/// trivially wet, which costs nothing: it has nothing to shed.
#[inline]
fn is_wet(age: u16, span: u32) -> bool {
    (age as u32) < span
}

/// **The wetness byte a texel reports**: `round(255 · (1 − age / span))`, and `0` where there is no
/// blood to be wet.
///
/// Integer, and over the *same* `dry_span` the wet/dry gate and the appearance rescale use, so a
/// texel that reads 0 here is exactly a texel [`is_wet`] calls dry. A shader multiplying a specular
/// boost by this gets the drying timeline for free.
#[inline]
fn wetness(amount: u8, age: u16, span: u32) -> u8 {
    if amount == 0 {
        return 0;
    }
    let left = span.saturating_sub(age as u32) as u64;
    (((left * 255 + (span as u64) / 2) / span.max(1) as u64).min(255)) as u8
}

/// **One drip step**: the texel a parcel falls along gravity's dominant axis (`major`), the texel
/// across it toward gravity's lean (`minor`), and how far it leans, `|minor| / |major|` of gravity
/// itself, in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct DripStep {
    major: (i64, i64),
    minor: (i64, i64),
    slant: f64,
}

impl DripStep {
    /// **Whether a parcel leaving `line` steps across as well as down**: `1` or `0`, in units of
    /// `minor`. `line` is the index of the line across the fall the parcel leaves — its row when it
    /// falls along `y`, its column when along `x`.
    ///
    /// Bresenham's rule: across on exactly the lines where `line · slant` passes a whole number, so
    /// a lean of a half steps on every other line and a lean of a third on every third, at a steady
    /// rhythm, and a run's sideways travel over any stretch of lines is the slant times their count
    /// to within one texel. Integer-exact for the slants a canvas meets: `f64` holds `line · slant`
    /// to far below a texel for any canvas edge a `u32` can name. A function of the line alone, so
    /// **every texel of a line steps alike** and the line moves as a translation: no two parcels can
    /// land on one texel, which is what keeps the drip's conservation argument the one it was.
    fn across(self, line: i64) -> i64 {
        if self.slant <= 0.0 {
            return 0;
        }
        let k = line as f64;
        i64::from(((k + 1.0) * self.slant).floor() > (k * self.slant).floor())
    }
}

/// The drip step gravity implies, or `None` for no gravity.
///
/// **A parcel still moves one texel a tick**, down the dominant axis, and leans by stepping one
/// across as well on a share of the lines it falls through — see [`DripStep::across`]. A tie in `|x|`
/// against `|y|` goes to the x axis, with a slant of one, so a diagonal gravity is reproducible rather
/// than dependent on how the caller spelled it.
fn drip_step(g: Vec2) -> Option<DripStep> {
    if !g.is_finite() || (g.x == 0.0 && g.y == 0.0) {
        return None;
    }
    let sign = |v: f32| if v > 0.0 { 1 } else { -1 };
    let across = |v: f32| if v < 0.0 { -1 } else { 1 };
    let lean = |minor: f32, major: f32| (f64::from(minor.abs()) / f64::from(major.abs())).clamp(0.0, 1.0);
    if g.x.abs() >= g.y.abs() {
        Some(DripStep { major: (sign(g.x), 0), minor: (0, across(g.y)), slant: lean(g.y, g.x) })
    } else {
        Some(DripStep { major: (0, sign(g.y)), minor: (across(g.x), 0), slant: lean(g.x, g.y) })
    }
}

/// Relative luminance of an encoded-sRGB base colour, `[0, 1]` — the grey substrate the film lies on.
fn luminance(base: [u8; 4]) -> f32 {
    let lin = |b: u8| {
        let c = b as f32 / 255.0;
        if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(base[0]) + 0.7152 * lin(base[1]) + 0.0722 * lin(base[2])
}

/// `base` with `blood` composited over it at `cov/255` coverage, rounded.
#[inline]
fn over(base: u8, blood: u8, cov: u8) -> u8 {
    let c = cov as u32;
    (((base as u32) * (255 - c) + (blood as u32) * c + 127) / 255) as u8
}

/// A `[0, 1]` channel as a byte.
#[inline]
fn enc(v: f32) -> u8 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    (v * 255.0).round() as u8
}

/// Copy a pixel buffer into an image, refusing a size mismatch rather than half-writing it.
fn upload(images: &mut Assets<Image>, handle: &Handle<Image>, px: &[u8]) -> bool {
    let Some(mut image) = images.get_mut(handle) else {
        return false;
    };
    let Some(data) = image.data.as_mut() else {
        return false;
    };
    if data.len() != px.len() {
        return false;
    }
    data.copy_from_slice(px);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::PrimitiveTopology;

    fn canvas(size: u32) -> (Assets<Image>, WetCanvas) {
        let mut images = Assets::<Image>::default();
        let c = WetCanvas::new(&mut images, size, [0.8, 0.7, 0.65], 0.6);
        (images, c)
    }

    fn blob(major: f32) -> StainShape {
        StainShape { major, minor: major, spines: 0, satellites: 0, direction: [1.0, 0.0], seed: 7 }
    }

    #[test]
    fn a_uv_less_mesh_warns_exactly_once() {
        let (_images, mut c) = canvas(16);
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD);
        m.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        let xf = GlobalTransform::default();
        for _ in 0..5 {
            assert!(c
                .paint_world(
                    &m,
                    &xf,
                    Vec3::new(0.2, 0.2, 1.0),
                    Vec3::new(0.0, 0.0, -1.0),
                    &blob(0.2),
                    0
                )
                .is_none());
        }
        // One memo entry, so one warning was emitted for five refusals.
        assert_eq!(c.warned.len(), 1);
    }

    /// **A thin film is lighter than a pool, and arterial is redder than venous** — the two claims
    /// `crate::bloodstain::spectral` makes, checked where a renderer would read them: the uploaded bytes.
    #[test]
    fn the_albedo_darkens_with_thickness_and_brightens_with_oxygen() {
        let mut s = WetSettings::default();
        let (_images, mut c) = canvas(4);
        // Fresh, so the age shift is zero and only the film decides the colour. Column 0 is a thin
        // smear, column 3 a full-depth pool; the drip pass cannot move either because the gravity
        // handed to `tick` is zero.
        c.wet[0] = (1, 0);
        c.wet[3] = (255, 0);
        c.shade(whole(&c), 0, &s);
        let lum = |px: &[u8]| 0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32;
        let thin = c.albedo_px[0..3].to_vec();
        let pool = c.albedo_px[12..15].to_vec();
        assert!(lum(&thin) > lum(&pool), "a smear {thin:?} was not lighter than a pool {pool:?}");
        assert!(pool[0] > pool[1] && pool[0] > pool[2], "a pool is not red: {pool:?}");

        let venous_red_share = pool[0] as f32 / (pool[0] as u32 + pool[1] as u32 + pool[2] as u32) as f32;
        s.so2 = crate::bloodstain::SO2_ARTERIAL;
        c.shade(whole(&c), 0, &s);
        let art = c.albedo_px[12..15].to_vec();
        let arterial_red_share = art[0] as f32 / (art[0] as u32 + art[1] as u32 + art[2] as u32) as f32;
        assert!(
            arterial_red_share > venous_red_share,
            "arterial {art:?} was not redder than venous {pool:?}"
        );
    }

    #[test]
    fn the_drip_pass_conserves_every_byte_it_does_not_lose_at_the_border() {
        let s = WetSettings::default();
        let (_images, mut c) = canvas(8);
        // Fill the top two rows by hand, above the drip threshold, so the pass has work to do and
        // nothing reaches the far border in one step.
        for x in 0..8 {
            c.wet[x] = (200, 0);
            c.wet[8 + x] = (150, 0);
        }
        let before: u32 = c.wet.iter().map(|t| t.0 as u32).sum();
        c.drip(whole(&c), Vec2::new(0.0, 1.0), &s);
        let after: u32 = c.wet.iter().map(|t| t.0 as u32).sum();
        assert_eq!(before, after, "the drip pass lost or invented coverage");
    }

    #[test]
    fn the_drip_pass_moves_a_parcel_exactly_one_texel() {
        // A pass that read its own writes would cascade a parcel down several rows in one call. This
        // is what "reads the PREVIOUS buffer" means, checked rather than asserted in a comment.
        let s = WetSettings::default();
        let (_images, mut c) = canvas(8);
        c.wet[8 * 2 + 3] = (255, 0);
        c.drip(whole(&c), Vec2::new(0.0, 1.0), &s);
        assert_eq!(c.amount_at(3, 2), s.drip_threshold(), "the source kept the wrong residue");
        assert_eq!(c.amount_at(3, 3), 255 - s.drip_threshold(), "the parcel did not land next door");
        assert_eq!(c.amount_at(3, 4), 0, "the parcel cascaded — the pass read its own writes");
    }

    /// A block of blood, four rows by `w` texels from `(x0, y0)`, every texel well over the threshold
    /// and no two rows alike, so every texel sheds and the parcels differ.
    fn reservoir(c: &mut WetCanvas, x0: usize, y0: usize, w: usize) {
        let n = c.size as usize;
        for y in 0..4 {
            for x in 0..w {
                c.wet[(y0 + y) * n + x0 + x] = ([255, 230, 200, 170][y], 0);
            }
        }
    }

    /// **A drip that leans conserves every byte.** A leaning parcel must not land where another
    /// already lands: every texel of a line stepping alike is what keeps each destination to one
    /// source, and a collision would drop one of the two parcels. A wide block under four leaning
    /// gravities, along both axes and both ways, for twelve drips.
    #[test]
    fn a_leaning_drip_conserves_every_byte() {
        let s = WetSettings::default();
        for gravity in [Vec2::new(0.6, 1.0), Vec2::new(-1.0, 0.3), Vec2::new(1.0, -1.0), Vec2::new(0.25, -1.0)] {
            let (_images, mut c) = canvas(64);
            reservoir(&mut c, 24, 28, 14);
            let total: u32 = c.wet.iter().map(|t| t.0 as u32).sum();
            // The same block in step, under gravity's dominant axis alone: no lean at all.
            let axis = if gravity.x.abs() >= gravity.y.abs() { Vec2::new(gravity.x.signum(), 0.0) } else { Vec2::new(0.0, gravity.y.signum()) };
            let (_images, mut d) = canvas(64);
            reservoir(&mut d, 24, 28, 14);
            for i in 0..12 {
                c.drip(whole(&c), gravity, &s);
                d.drip(whole(&d), axis, &s);
                let now: u32 = c.wet.iter().map(|t| t.0 as u32).sum();
                assert_eq!(now, total, "{gravity}, drip {i}: a leaning parcel was lost or doubled");
            }
            assert_ne!(c.wet, d.wet, "{gravity}: nothing leaned, so nothing here was tested");
        }
    }

    /// **A run leans the way gravity leans, by as much.** The blood's mean position after thirty
    /// drips from a block six texels wide: under a gravity leaning one across for two down it has
    /// moved across half as far as down, toward the lean and away from it when the lean is turned;
    /// along the other axis the same; and under straight-down gravity it has not moved across and
    /// has not left its own six columns.
    #[test]
    fn a_run_leans_the_way_gravity_leans() {
        let run = |gravity: Vec2| -> (f32, f32, usize) {
            let s = WetSettings { dry_ticks: 60_000, ..Default::default() };
            let (_images, mut c) = canvas(96);
            reservoir(&mut c, 45, 40, 6);
            let mean = |c: &WetCanvas| {
                let (mut sx, mut sy, mut m) = (0.0f64, 0.0f64, 0.0f64);
                for (i, t) in c.wet.iter().enumerate() {
                    let a = t.0 as f64;
                    sx += a * (i % 96) as f64;
                    sy += a * (i / 96) as f64;
                    m += a;
                }
                (sx / m, sy / m)
            };
            let before = mean(&c);
            for _ in 0..30 {
                c.drip(whole(&c), gravity, &s);
            }
            let after = mean(&c);
            let columns = (0..96).filter(|x| (0..96).any(|y| c.wet[y * 96 + x].0 > 0)).count();
            ((after.0 - before.0) as f32, (after.1 - before.1) as f32, columns)
        };
        let (dx, dy, _) = run(Vec2::new(0.5, 1.0));
        assert!(dy > 2.0, "the blood hardly fell ({dy} texels), so the lean was not exercised");
        assert!((0.4..0.6).contains(&(dx / dy)), "the blood moved {dx} across for {dy} down under a lean of 0.5");
        let (dx, dy, _) = run(Vec2::new(-0.5, 1.0));
        assert!((-0.6..-0.4).contains(&(dx / dy)), "the blood moved {dx} across for {dy} down under a lean of -0.5");
        let (dx, dy, _) = run(Vec2::new(-1.0, -0.25));
        assert!(dx < -2.0 && (0.15..0.35).contains(&(dy / dx)), "the blood moved {dy} across for {dx} along under a lean of 0.25");
        let (dx, _, straight) = run(Vec2::new(0.0, 1.0));
        assert_eq!((dx, straight), (0.0, 6), "with no lean the run left its own six columns");
    }

    /// **A wick wets inside its front, never past it, and never takes blood away.** A pooled texel
    /// inside the front keeps its pool; the wetted texels lie within the reach the noise allows; the
    /// front is ragged, so the wetted set is not the plain ellipse the shape describes; and a second
    /// call with the same shape is a no-op, so growing a soak by calling it again is safe.
    #[test]
    fn a_wick_wets_inside_a_ragged_front_and_never_dries_a_texel() {
        let (_images, mut c) = canvas(64);
        let pooled = 32 * 64 + 34;
        c.wet[pooled] = (255, 0);
        let shape = StainShape { major: 0.5, minor: 0.5, spines: 0, satellites: 0, direction: [0.0, 1.0], seed: 7 };
        c.wick_uv(Vec2::splat(0.5), &shape, 3);
        assert_eq!(c.wet[pooled].0, 255, "the wick lowered a pooled texel");
        let radius_px = 0.25 * 64.0;
        let (mut wet, mut ellipse_misses) = (0, 0);
        for y in 0..64usize {
            for x in 0..64usize {
                let d = Vec2::new(x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0).length() / radius_px;
                let amount = c.wet[y * 64 + x].0;
                if amount > 0 {
                    wet += 1;
                    assert!(d < 1.4, "a texel at {d:.2} radii was wetted past the reach");
                }
                // Inside the ellipse but dry, or outside it but wet: where the front frays.
                if (d < 0.95) != (amount > 0) {
                    ellipse_misses += 1;
                }
            }
        }
        assert!(wet > 300, "the wick wetted only {wet} texels");
        assert!(ellipse_misses > 40, "the front is the plain ellipse ({ellipse_misses} texels off it): no weave");
        let before = c.wet.clone();
        c.wick_uv(Vec2::splat(0.5), &shape, 4);
        assert_eq!(c.wet, before, "a second wick of the same front changed the canvas");
    }

    #[test]
    fn the_spread_pass_conserves_every_byte() {
        let s = WetSettings::default();
        let (_images, mut c) = canvas(8);
        c.wet[8 * 4 + 4] = (255, 0);
        c.wet[8 * 4 + 5] = (90, 0);
        let before: u32 = c.wet.iter().map(|t| t.0 as u32).sum();
        for _ in 0..20 {
            c.spread(whole(&c), &s);
        }
        let after: u32 = c.wet.iter().map(|t| t.0 as u32).sum();
        assert_eq!(before, after, "the antisymmetric flux is not antisymmetric");
    }

    #[test]
    fn the_spread_pass_reaches_only_the_four_neighbourhood_in_one_call() {
        let s = WetSettings::default();
        let (_images, mut c) = canvas(8);
        c.wet[8 * 4 + 4] = (255, 0);
        c.spread(whole(&c), &s);
        assert!(c.amount_at(5, 4) > 0, "nothing diffused east");
        assert_eq!(c.amount_at(6, 4), 0, "diffusion jumped two texels in one pass");
        assert_eq!(c.amount_at(5, 5), 0, "diffusion reached a diagonal");
    }

    #[test]
    fn a_dried_canvas_is_a_fixed_point() {
        let s = WetSettings { dry_ticks: 12, ..Default::default() };
        let (mut images, mut c) = canvas(64);
        c.paint_uv(Vec2::new(0.5, 0.2), &blob(0.2), 0);
        for t in 0..40 {
            c.tick(t, Vec2::new(0.0, 1.0), &s);
        }
        assert!(c.flush(&mut images), "the run never asked to be uploaded");
        let settled = c.digest();
        for t in 40..60 {
            c.tick(t, Vec2::new(0.0, 1.0), &s);
        }
        assert_eq!(c.digest(), settled, "a dried canvas still moved");
        assert!(!c.is_dirty(), "a dried canvas asked to be uploaded again");
    }

    /// **A still canvas skips its ticks, and anything that could move it wakes it.** An untouched
    /// canvas is still after one tick; a paint wakes it; it is still again once the blood has set, and
    /// a later tick changes nothing; lengthening the drying span makes that set blood wet again, so the
    /// skip must not hide it.
    #[test]
    fn a_still_canvas_sleeps_until_something_can_move_it() {
        let s = WetSettings { dry_ticks: 12, ..Default::default() };
        let (_images, mut c) = canvas(64);
        c.tick(0, Vec2::new(0.0, 1.0), &s);
        assert!(c.is_still(), "an untouched canvas has nothing to move");
        c.paint_uv(Vec2::new(0.5, 0.2), &blob(0.2), 1);
        assert!(!c.is_still(), "a paint must wake the canvas");
        let mut woke_at = None;
        for t in 1..60 {
            c.tick(t, Vec2::new(0.0, 1.0), &s);
            if c.is_still() && woke_at.is_none() {
                woke_at = Some(t);
            }
        }
        assert!(woke_at.is_some_and(|t| t >= 12), "still at {woke_at:?}, before the blood could set");
        // What the skip stands in for: the four passes, run directly on the still canvas, change
        // nothing — the buffer, both images, or the dirty flag.
        let settled = (c.digest(), c.albedo_px.clone(), c.rough_px.clone(), c.is_dirty());
        let all = whole(&c);
        c.drip(all, Vec2::new(0.0, 1.0), &s);
        c.spread(all, &s);
        c.advance(all, &s);
        c.shade(all, 60, &s);
        assert_eq!((c.digest(), c.albedo_px.clone(), c.rough_px.clone(), c.is_dirty()), settled, "the skipped passes would have moved something");
        let longer = WetSettings { dry_ticks: 600, ..s.clone() };
        c.tick(61, Vec2::new(0.0, 1.0), &longer);
        assert!(!c.is_still(), "a longer drying span re-wets set blood, and the skip hid it");
    }

    /// Every texel of the canvas, as a region — what the passes covered before `live` existed.
    fn whole(c: &WetCanvas) -> Live {
        let last = c.size as i64 - 1;
        Live { x0: 0, y0: 0, x1: last, y1: last }
    }

    /// **Ticking over the live rectangle is ticking over the whole canvas, to the bit.** Two canvases
    /// take the same three stains, at different ticks and both edges, then 400 ticks of drip, spread,
    /// soak and drying. One is ticked by `tick`, which runs its passes over the rectangle the blood
    /// can reach; the other runs every pass over every texel. Their buffers and both images must
    /// agree on every tick.
    #[test]
    fn the_live_rectangle_changes_nothing_but_the_cost() {
        // Blobs at both edges under a fast spread and a short drying span, gravity turned mid-run.
        let s = WetSettings { dry_ticks: 300, spread_rate: 0.3, ..Default::default() };
        let stains = [(0, Vec2::new(0.5, 0.2), blob(0.3)), (40, Vec2::new(0.02, 0.5), blob(0.15)), (90, Vec2::new(0.9, 0.97), blob(0.2))];
        agree(48, &s, &stains, 400);
        // The shipped dials on a shipped-size canvas, and stains with spines and satellites. Seed 1
        // is the one that caught a one-texel margin losing spread's second step at tick 0.
        for seed in 0..4 {
            let spiky = StainShape { major: 0.09, minor: 0.06, spines: 5, satellites: 3, direction: [0.0, 1.0], seed };
            agree(128, &WetSettings::default(), &[(0, Vec2::new(0.5, 0.3), spiky)], 300);
        }
    }

    /// Tick one canvas with `tick` and another with every pass over every texel, and require them
    /// equal on every tick.
    fn agree(size: u32, s: &WetSettings, stains: &[(u32, Vec2, StainShape)], ticks: u32) {
        let (_ia, mut fast) = canvas(size);
        let (_ib, mut full) = canvas(size);
        for t in 0..ticks {
            for (at, uv, shape) in stains {
                if t == *at {
                    fast.paint_uv(*uv, shape, t);
                    full.paint_uv(*uv, shape, t);
                }
            }
            let gravity = if t < ticks / 2 { Vec2::new(0.0, 1.0) } else { Vec2::new(1.0, 0.2) };
            fast.tick(t, gravity, s);
            let all = whole(&full);
            full.drip(all, gravity, s);
            full.spread(all, s);
            full.advance(all, s);
            full.shade(all, t, s);
            assert_eq!(fast.wet, full.wet, "size {size}, tick {t}: the buffers parted");
            assert_eq!((&fast.albedo_px, &fast.rough_px), (&full.albedo_px, &full.rough_px), "size {size}, tick {t}: the images parted");
        }
    }

    /// **A detailed image draws a pool's edge crisp, with its corners softened, and in the pool's
    /// colour up to it.** A pool of full texels whose edge slants at 0.3 across a 32 canvas, drawn at
    /// detail 1 and at detail 4 and read back from the uploaded images the way a sampler reads them,
    /// bilinearly, down each column. At detail 1 the colour takes most of a texel to go from the
    /// pool's to the surface's, and the line where it is half way is the texels' staircase, turning
    /// a corner at every step. At detail 4 it must take under a third of that, and the line must bend
    /// less from one half-texel to the next. (It does not come out straight: the canvas holds a
    /// staircase, and a spline four texels wide softens a step three long rather than erasing it.)
    /// And the pixels one in from the detailed edge must be the pool's colour, not a paler mix with
    /// the surface.
    #[test]
    fn a_detailed_image_draws_an_edge_crisp_and_red_to_its_rim() {
        const N: i64 = 32;
        let s = WetSettings::default();
        let mut images = Assets::<Image>::default();
        // The green channel of the albedo a canvas at detail `k` uploads, and that image's edge in pixels.
        let mut draw = |k: u32| -> (Vec<i32>, i64) {
            let mut c = WetCanvas::new_detailed(&mut images, N as u32, k, [0.8, 0.7, 0.65], 0.6);
            for y in 0..N {
                for x in 0..N {
                    if (y as f32 + 0.5) < 0.3 * (x as f32 + 0.5) + 10.0 {
                        c.wet[(y * N + x) as usize] = (255, 0);
                    }
                }
            }
            c.shade(whole(&c), 0, &s);
            assert!(c.flush(&mut images));
            let px = images.get(&c.albedo()).and_then(|i| i.data.clone()).unwrap_or_default();
            let side = N * k as i64;
            assert_eq!(px.len() as i64, side * side * 4, "the image is not detail times the canvas");
            (px.chunks(4).map(|p| p[1] as i32).collect(), side)
        };
        let (texels, _) = draw(1);
        let (pixels, nf) = draw(4);
        let (pool, base) = (pixels[0], pixels[pixels.len() - 1]);
        assert_eq!((texels[0], texels[texels.len() - 1]), (pool, base), "the two images disagree on the colours");

        // Down each column, in texels, the first place the image is `frac` of the way from the pool
        // to the surface.
        let edge = |g: &[i32], side: i64, frac: f32| -> Vec<f32> {
            let unit = side as f32 / N as f32;
            let at = |x: i64, y: i64| g[(y.clamp(0, side - 1) * side + x.clamp(0, side - 1)) as usize] as f32;
            let bilinear = |tx: f32, ty: f32| {
                let (px, py) = (tx * unit - 0.5, ty * unit - 0.5);
                let (x0, y0) = (px.floor(), py.floor());
                let (fx, fy) = (px - x0, py - y0);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * fx;
                let bottom = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * fx;
                top + (bottom - top) * fy
            };
            let ys: Vec<f32> = (0..400)
                .filter_map(|i| {
                    let x = 6.0 + i as f32 * 0.05;
                    (0..6000).map(|j| 4.0 + j as f32 * 0.005).find(|&y| (bilinear(x, y) - pool as f32).abs() > frac * (base - pool).abs() as f32)
                })
                .collect();
            assert_eq!(ys.len(), 400, "a column found no edge");
            ys
        };
        // The widest the colour's change from 10 % to 90 % is, and the most the half-way line bends
        // from one half-texel to the next, as a second difference.
        let measure = |g: &[i32], side: i64| -> (f32, f32) {
            let (near, far, half) = (edge(g, side, 0.1), edge(g, side, 0.9), edge(g, side, 0.5));
            let width = near.iter().zip(&far).map(|(a, b)| b - a).fold(0.0, f32::max);
            let bend = (10..half.len() - 10).map(|i| (half[i + 10] - 2.0 * half[i] + half[i - 10]).abs()).fold(0.0, f32::max);
            (width, bend)
        };
        let (stairs_width, stairs_bend) = measure(&texels, N);
        let (width, bend) = measure(&pixels, nf);
        assert!(width < stairs_width / 3.0, "the detailed edge takes {width} texels to change, the texels' own {stairs_width}");
        assert!(bend < 0.8 * stairs_bend, "the detailed edge bends {bend} texels in a half-texel, the texels' own {stairs_bend}");

        // One pixel in from the detailed edge, every pixel is the pool's colour within two code values.
        let bloody = |x: i64, y: i64| {
            let g = pixels[(y.clamp(0, nf - 1) * nf + x.clamp(0, nf - 1)) as usize];
            (g - base).abs() * 2 > (pool - base).abs()
        };
        for y in 0..nf {
            for x in 0..nf {
                let ring = (-1..=1).all(|j| (-1..=1).all(|i| bloody(x + i, y + j))) && !(-2..=2).all(|j| (-2..=2).all(|i| bloody(x + i, y + j)));
                if ring {
                    let g = pixels[(y * nf + x) as usize];
                    assert!((g - pool).abs() <= 2, "({x}, {y}) one in from the rim has green {g}, the pool {pool}");
                }
            }
        }
    }
}
