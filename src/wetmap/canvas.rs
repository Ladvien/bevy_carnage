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
        let size = if size == 0 {
            warn!("wetmap: a canvas of size 0 has no texels; using 1");
            1
        } else {
            size
        };
        let base_rgba = [enc(base_srgb[0]), enc(base_srgb[1]), enc(base_srgb[2]), 255];
        let base_rough = enc(base_roughness);
        // **The four channels of the metallic-roughness image, and only two of them are Bevy's.**
        // G carries roughness and B carries metallic — blood is a dielectric, so B stays 0 — and R
        // and A are free for data a caller's own shader may want: R is the coverage byte (the film
        // depth) and A is wetness. An untouched texel has no blood on it, so both are 0.
        let base_rough_rgba = [0, base_rough, 0, 0];

        let extent = Extent3d { width: size, height: size, depth_or_array_layers: 1 };
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
            }
        }
        if touched && self.dirty_since.is_none() {
            self.dirty_since = Some(tick);
        }
    }

    /// **Stamp a stain where a world-space ray hits this actor.**
    ///
    /// Möller–Trumbore over the mesh's `ATTRIBUTE_POSITION` and index buffer, then a barycentric read
    /// of `ATTRIBUTE_UV_0`. The ray is moved into mesh space by one inverse transform rather than the
    /// geometry being moved into world space.
    ///
    /// Returns `false` when the ray misses, and `false` with **one warning per mesh** when the mesh
    /// cannot carry a wetmap at all — no `Float32x2` UV0, no `Float32x3` positions, or not a triangle
    /// list. That refusal is the point: a mesh without UVs makes every drop of blood a caller paints
    /// land nowhere, and returning `true` after painting nothing would hide it.
    pub fn paint_world(
        &mut self,
        mesh: &Mesh,
        xf: &GlobalTransform,
        from: Vec3,
        dir: Vec3,
        shape: &StainShape,
        tick: u32,
    ) -> bool {
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
    ) -> bool {
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
    ) -> bool {
        let inv = xf.affine().inverse();
        let origin = inv.transform_point3(from);
        let local_dir = inv.transform_vector3(dir);
        match ray_uv(mesh, origin, local_dir) {
            Pick::At(uv) => {
                self.stamp(uv, shape, tick, span);
                true
            }
            Pick::Miss => false,
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
                false
            }
        }
    }

    /// **The state machine, in exactly four passes and exactly this order.**
    ///
    /// 1. **Drip.** A wet texel holding more than `drip_rate` sheds the *excess* one texel along
    ///    `gravity_uv` and keeps the rest. What leaves one texel arrives in exactly one other, or is
    ///    lost at the border — nothing else, because the step is a translation, so at most one texel
    ///    can ever step into another. **Conserved exactly wherever the destination is also wet:** a
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
        self.drip(gravity_uv, s);
        self.spread(s);
        self.advance(s);
        self.shade(tick, s);
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
        let ok = upload(images, &self.albedo_handle, &self.albedo_px)
            && upload(images, &self.rough_handle, &self.rough_px);
        if ok {
            self.dirty_since = None;
        }
        ok
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

    /// Pass 1. See [`tick`](Self::tick).
    fn drip(&mut self, gravity_uv: Vec2, s: &WetSettings) {
        let Some((sx, sy)) = dominant_step(gravity_uv) else {
            return;
        };
        let thr = s.drip_threshold();
        let span = s.dry_span();
        self.prev.copy_from_slice(&self.wet);
        let n = self.size as i64;
        for y in 0..n {
            for x in 0..n {
                let Some(&(amount, age)) = self.prev.get((y * n + x) as usize) else {
                    continue;
                };
                let leaves = if is_wet(age, span) { amount.saturating_sub(thr) } else { 0 };
                let residue = amount - leaves;
                // Exactly one texel can step into this one, because the step is a translation. So
                // there is no accumulation here and no order in which two arrivals could race.
                let (ax, ay) = (x - sx, y - sy);
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

    /// Pass 2. See [`tick`](Self::tick).
    fn spread(&mut self, s: &WetSettings) {
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
        for y in 0..n {
            for x in 0..n {
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

    /// Pass 3. See [`tick`](Self::tick).
    fn advance(&mut self, s: &WetSettings) {
        let span = s.dry_span();
        let ceiling = span as u16;
        for cell in &mut self.wet {
            let (amount, age) = *cell;
            if amount == 0 {
                *cell = (0, 0);
                continue;
            }
            let new_age = age.saturating_add(1).min(ceiling);
            if !is_wet(age, span) {
                // Set. Frozen: it does not move, does not soak, and its age has stopped — which is
                // what makes a dried canvas a fixed point.
                *cell = (amount, new_age);
                continue;
            }
            let taken = s.absorbed_by(new_age as u32).saturating_sub(s.absorbed_by(age as u32));
            let new_amount = amount.saturating_sub(taken.min(255) as u8);
            *cell = (new_amount, if new_amount == 0 { 0 } else { new_age });
        }
    }

    /// Pass 4. See [`tick`](Self::tick).
    fn shade(&mut self, tick: u32, s: &WetSettings) {
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

        let cells = self.wet.iter();
        let albedo = self.albedo_px.chunks_exact_mut(4);
        let rough = self.rough_px.chunks_exact_mut(4);
        for (&(amount, age), (a_px, r_px)) in cells.zip(albedo.zip(rough)) {
            let (rgb, rough_byte) = if amount == 0 {
                ([base[0], base[1], base[2]], base_rough)
            } else {
                let level = (amount as usize * FILM_LEVELS) / 256;
                if (level, age as u32) != memo {
                    memo = (level, age as u32);
                    // **`s.dry_ticks` is the single authority for the timeline.** `appearance`
                    // normalises the age it is given against `dry_ticks(area, hz)`, so the texel's age
                    // is rescaled onto `bloodstain`'s own reference span and the reference inputs are
                    // passed. Feeding the raw age with an unrelated area would leave two dials
                    // deciding one curve.
                    let scaled = ((age as u64 * DRY_REF_TICKS as u64) / span as u64) as u32;
                    let ap = appearance_with_fresh(scaled, 60, DRY_REF_AREA_M2, &blood, fresh[level]);
                    memo_srgb = [enc(ap.srgb[0]), enc(ap.srgb[1]), enc(ap.srgb[2])];
                    memo_rough = enc(ap.roughness);
                }
                // The albedo is the film's own colour: the substrate is already inside it, because
                // a thin film transmits what it lies on — compositing it over the base a second time
                // would count the surface twice. Roughness still blends, in the encoded space: the
                // honest version is a `powf` each way per texel per tick and at these values the
                // difference is under a code value. `rim`, `halo` and `craquelure` are deliberately
                // unread — they need a shader, and this crate ships none.
                (memo_srgb, over(base_rough, memo_rough, amount))
            };

            if let [r, g, b, _] = a_px
                && (*r != rgb[0] || *g != rgb[1] || *b != rgb[2])
            {
                *r = rgb[0];
                *g = rgb[1];
                *b = rgb[2];
                changed = true;
            }
            // **All four channels of the metallic-roughness image**, per
            // [`roughness`](Self::roughness): R the coverage byte, G the roughness, B the metallic
            // zero, A the wetness. R and A are the buffer's own two numbers written where a shader
            // can read them, so nothing has to be inferred from the albedo — and they are still
            // *output*: no pass in this file reads a pixel back.
            let want = [amount, rough_byte, 0, wetness(amount, age, span)];
            for (slot, want) in r_px.iter_mut().zip(want) {
                if *slot != want {
                    *slot = want;
                    changed = true;
                }
            }
        }

        if changed && self.dirty_since.is_none() {
            self.dirty_since = Some(tick);
        }
    }
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

/// The single-texel step gravity implies.
///
/// **Quantised to the dominant axis on purpose.** A fractional step would need interpolation to split a
/// parcel between two texels, and interpolation is a second movement model with its own conservation
/// argument to make. One texel per tick along one axis has exactly one, and it is three lines.
///
/// A tie in `|x|` against `|y|` goes to the x axis — arbitrary but fixed, so a diagonal gravity is
/// reproducible rather than dependent on how the caller spelled it.
fn dominant_step(g: Vec2) -> Option<(i64, i64)> {
    if !g.is_finite() || (g.x == 0.0 && g.y == 0.0) {
        return None;
    }
    if g.x.abs() >= g.y.abs() {
        Some((if g.x > 0.0 { 1 } else { -1 }, 0))
    } else {
        Some((0, if g.y > 0.0 { 1 } else { -1 }))
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
            assert!(!c.paint_world(
                &m,
                &xf,
                Vec3::new(0.2, 0.2, 1.0),
                Vec3::new(0.0, 0.0, -1.0),
                &blob(0.2),
                0
            ));
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
        c.shade(0, &s);
        let lum = |px: &[u8]| 0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32;
        let thin = c.albedo_px[0..3].to_vec();
        let pool = c.albedo_px[12..15].to_vec();
        assert!(lum(&thin) > lum(&pool), "a smear {thin:?} was not lighter than a pool {pool:?}");
        assert!(pool[0] > pool[1] && pool[0] > pool[2], "a pool is not red: {pool:?}");

        let venous_red_share = pool[0] as f32 / (pool[0] as u32 + pool[1] as u32 + pool[2] as u32) as f32;
        s.so2 = crate::bloodstain::SO2_ARTERIAL;
        c.shade(0, &s);
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
        c.drip(Vec2::new(0.0, 1.0), &s);
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
        c.drip(Vec2::new(0.0, 1.0), &s);
        assert_eq!(c.amount_at(3, 2), s.drip_threshold(), "the source kept the wrong residue");
        assert_eq!(c.amount_at(3, 3), 255 - s.drip_threshold(), "the parcel did not land next door");
        assert_eq!(c.amount_at(3, 4), 0, "the parcel cascaded — the pass read its own writes");
    }

    #[test]
    fn the_spread_pass_conserves_every_byte() {
        let s = WetSettings::default();
        let (_images, mut c) = canvas(8);
        c.wet[8 * 4 + 4] = (255, 0);
        c.wet[8 * 4 + 5] = (90, 0);
        let before: u32 = c.wet.iter().map(|t| t.0 as u32).sum();
        for _ in 0..20 {
            c.spread(&s);
        }
        let after: u32 = c.wet.iter().map(|t| t.0 as u32).sum();
        assert_eq!(before, after, "the antisymmetric flux is not antisymmetric");
    }

    #[test]
    fn the_spread_pass_reaches_only_the_four_neighbourhood_in_one_call() {
        let s = WetSettings::default();
        let (_images, mut c) = canvas(8);
        c.wet[8 * 4 + 4] = (255, 0);
        c.spread(&s);
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
}
