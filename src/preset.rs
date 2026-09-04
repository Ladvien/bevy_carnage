//! **The two-line inclusion.** `GorePlugin`, a `Gore` component on the meshes that can be hurt, and
//! a `GoreHit` message when one is — everything else in this family is wired behind it.
//!
//! Before 0.4.0 the flagship demo was a thousand lines that named eight crates and hand-ran their
//! ticks in the right order. That is the honest cost of "the caller owns the schedule", which each
//! leaf insists on and is right to — but a game wants a preset, and the preset is what this module
//! is: the one place in the family that *does* own a schedule, a tick and a spawner, and says so.
//!
//! ```ignore
//! app.add_plugins(bevy_carnage::preset::GorePlugin);
//! commands.entity(torso).insert(Gore::skin(Region::Torso));
//! commands.entity(floor).insert(Gore::floor());
//! // later, from a raycast:
//! hits.write(GoreHit::impact(torso, from, dir, 0.02, 40.0));
//! ```
//!
//! # What a hit does, in order
//!
//! 1. **Peels the skin** where it landed — `bevy_flaymap`, to the depth the hit asked for.
//! 2. **Throws blood** — the wound's `bloodstain::stain::stains` in closed form, painted onto every
//!    [`Gore::floor`] they land on, and a stain at the wound on the body's own wetmap.
//! 3. **Opens a bleed** — `bloodstain::bleed`, pulsing more blood onto the wetmap each systole until
//!    the clot, which the wetmap then runs under gravity and dries.
//! 4. **A slash gapes** — [`GoreHit::slash`] lays a `bevy_laceration` along the cut instead of a
//!    crater, and its bed is banded by `bevy_cross_section`.
//! 5. **Guts spill** — a torso hit that reached muscle spawns `bevy_viscera` strands tethered to the
//!    wound, when [`GoreDials::strands`] is nonzero.
//! 6. **Bone hands off** — the first hit to show cortex forwards `BoneExposed`, and if the body is a
//!    baked [`crate::FractureSubject`] the preset breaks it: the bake's leaves are thrown from the
//!    hit with skin on the outside and a [`crate::flesh::FleshMaterial`] cap in [`FleshMode::Cap`],
//!    integrated ballistically to [`GoreDials::floor_y`]. A game with its own physics sets
//!    [`GoreDials::spawn_fragments`] to `false` and reads [`GoreFractured`] instead.
//!
//! Every surface a [`Gore`] dresses gets the flesh material, so all of the above is drawn with
//! subsurface wrap, a wet clear coat and, on cloth, a film composited over the weave.
//!
//! # The clock
//!
//! [`GoreClock`] is one `u32` advanced once per `FixedUpdate`. That is the tick every leaf asks the
//! caller for, and this preset *is* the caller. Nothing here reads `Time`; a fixed step is what makes
//! the canvases' digests reproducible, and a game that wants the gore on its own step configures
//! [`GoreSystems`] and drives the clock itself.

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy_cross_section::{CrossSectionPlugin, CrossSectionSettings, Layers, Region};
use bevy_flaymap::{BoneExposed, FlayCanvas, FlaySettings, FlaymapPlugin, Layer};
use bevy_laceration::{Gape, Laceration, LacerationClock, LacerationPlugin, Tension};
use bevy_viscera::{Mesentery, Strand, ViscSettings, VisceraPlugin, VisceraSystems, spill, tube_mesh};
use bevy_wetmap::{StainShape, WetCanvas, WetSettings, WetmapPlugin};
use bloodstain::{BloodSettings, Wound as BloodWound, WoundKind, wound_seed};

use crate::flesh::{FleshMaterial, FleshMode, FleshParams, FleshPlugin, FleshTables};
use crate::{CarnagePlugin, FractureCache, FractureSubject, Wounded};

/// **What a surface is, to the preset.**
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Gore {
    /// Which kind of surface, and so which canvases it gets.
    pub kind: GoreKind,
    /// Which way blood runs on this mesh's atlas, in UV space. `(0, 1)` is down on most atlases.
    pub gravity_uv: Vec2,
    /// **How many UV units one metre of this surface covers.** The canvases take radii in UV space
    /// and assume one UV unit per metre; a mesh whose atlas spans a 40 cm limb from 0 to 1 packs
    /// 2.5 UV per metre, and a 3 cm wound on it is `0.075` UV wide, not `0.03`. Set it from the
    /// asset: `1 / (metres the atlas spans)`.
    pub uv_per_metre: f32,
}

/// The three surfaces the preset knows how to dress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GoreKind {
    /// A body part: a flaymap and a wetmap over skin, banded by `region`'s thickness row.
    Skin(Region),
    /// Clothing: a wetmap whose film is composited over the fabric on the GPU.
    Cloth,
    /// Ground: a wetmap that catches what a wound throws.
    Floor,
}

impl Gore {
    /// A body part of `region`.
    pub fn skin(region: Region) -> Self {
        Self { kind: GoreKind::Skin(region), gravity_uv: Vec2::new(0.0, 1.0), uv_per_metre: 1.0 }
    }
    /// A fabric surface.
    pub fn cloth() -> Self {
        Self { kind: GoreKind::Cloth, gravity_uv: Vec2::new(0.0, 1.0), uv_per_metre: 1.0 }
    }
    /// A floor. Blood pools rather than runs, so its UV gravity is zero.
    pub fn floor() -> Self {
        Self { kind: GoreKind::Floor, gravity_uv: Vec2::ZERO, uv_per_metre: 1.0 }
    }
    /// The same surface with its atlas density stated — see [`Gore::uv_per_metre`].
    pub fn with_uv_per_metre(mut self, uv_per_metre: f32) -> Self {
        self.uv_per_metre = if uv_per_metre.is_finite() && uv_per_metre > 0.0 { uv_per_metre } else { 1.0 };
        self
    }
}

/// The wetmap settings with the preset's edge supersampling dialled in.
fn edge_settings(dials: &GoreDials, settings: Option<&WetSettings>) -> WetSettings {
    WetSettings { edge_samples: dials.edge_samples, ..settings.cloned().unwrap_or_default() }
}

/// The UV a world ray lands on, by the same Möller–Trumbore walk the canvases use — the preset needs
/// it for the injuries that live in texture space (a bruise, a burn) rather than on a ray.
fn ray_hit(mesh: &Mesh, xf: &GlobalTransform, from: Vec3, dir: Vec3) -> Option<(Vec2, Vec3, Vec3)> {
    let inv = xf.affine().inverse();
    let origin = inv.transform_point3(from);
    let d = inv.transform_vector3(dir);
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(p)) => p,
        _ => return None,
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(u)) => u,
        _ => return None,
    };
    let tri = |i: usize| -> Option<[usize; 3]> {
        match mesh.indices() {
            Some(bevy::mesh::Indices::U32(ix)) => Some([*ix.get(i)? as usize, *ix.get(i + 1)? as usize, *ix.get(i + 2)? as usize]),
            Some(bevy::mesh::Indices::U16(ix)) => Some([*ix.get(i)? as usize, *ix.get(i + 1)? as usize, *ix.get(i + 2)? as usize]),
            None => Some([i, i + 1, i + 2]),
        }
    };
    let count = mesh.indices().map(|ix| ix.len()).unwrap_or(positions.len());
    let mut best: Option<(f32, Vec2, Vec3, Vec3)> = None;
    let mut i = 0;
    while i + 2 < count {
        let Some([a, b, c]) = tri(i) else { break };
        i += 3;
        let (Some(pa), Some(pb), Some(pc)) = (positions.get(a), positions.get(b), positions.get(c)) else { continue };
        let (pa, pb, pc) = (Vec3::from(*pa), Vec3::from(*pb), Vec3::from(*pc));
        let e1 = pb - pa;
        let e2 = pc - pa;
        let h = d.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1.0e-9 {
            continue;
        }
        let inv_det = 1.0 / det;
        let s_vec = origin - pa;
        let u = inv_det * s_vec.dot(h);
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s_vec.cross(e1);
        let v = inv_det * d.dot(q);
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let t = inv_det * e2.dot(q);
        if t <= 0.0 || best.is_some_and(|(bt, ..)| bt <= t) {
            continue;
        }
        let (Some(ua), Some(ub), Some(uc)) = (uvs.get(a), uvs.get(b), uvs.get(c)) else { continue };
        let uv = Vec2::from(*ua) * (1.0 - u - v) + Vec2::from(*ub) * u + Vec2::from(*uc) * v;
        best = Some((t, uv, origin + d * t, e1.cross(e2).normalize_or_zero()));
    }
    best.map(|(_, uv, p, n)| (uv, p, n))
}

/// A stain silhouette in this surface's UV units, from one in metres — and never under a texel and
/// a half across. A 2 mm droplet on a floor at 12 mm per texel is real and invisible; the floor of
/// the stamp is a resolution fact, so a distant spatter reads as the dot it is rather than as nothing.
fn shape_on(gore: &Gore, shape: &StainShape, texels: u32) -> StainShape {
    let floor_uv = 1.5 / texels.max(1) as f32;
    StainShape {
        major: (shape.major * gore.uv_per_metre).max(floor_uv),
        minor: (shape.minor * gore.uv_per_metre).max(floor_uv),
        ..*shape
    }
}

/// Marks an entity the preset has already dressed, and keeps the material it wore before — the
/// skin a thrown fragment's outer mesh goes back to.
#[derive(Component, Clone, Debug, Default)]
pub struct Dressed {
    /// The `StandardMaterial` the entity had before the flesh material replaced it.
    pub original: Option<Handle<StandardMaterial>>,
}

/// A wound the preset is bleeding, on this entity's wetmap.
#[derive(Component, Clone, Copy, Debug)]
pub struct WoundSite {
    /// The ray that made the wound, world; every pulse is painted through it again.
    pub from: Vec3,
    /// Its direction.
    pub dir: Vec3,
    /// The wound in world space, for the pulse schedule and the stains it throws.
    pub wound: BloodWound,
    /// The schedule.
    pub bleed: bloodstain::Bleed,
}

/// **A body built at runtime that the preset may break.** For a body that is a loaded scene, put
/// [`crate::FractureSubject`] on its root and the preset reads the cache; a body assembled in code
/// has no asset path to seed a cached bake from, so it bakes once with [`crate::fracture_mesh`] and
/// hands the leaves here. The pieces are thrown from the entity's own transform on the bone handoff
/// of any descendant [`Gore`].
#[derive(Component)]
pub struct GoreBreakable {
    /// The finest frontier of a [`crate::Fracture`], caps annotated with `UV_1`.
    pub leaves: Vec<crate::FragmentGeometry>,
    /// Which leaves touch which, so a blow parts islands rather than everything.
    pub bonds: crate::BondGraph,
    /// The fracture modes over those bonds (`bevy_fracture_modes` through [`crate::modal`]), when
    /// the bake could produce them. Without them every leaf flies.
    pub modes: Option<crate::modal::ModalSet>,
    /// The thickness row the caps are banded by.
    pub region: Region,
    /// How hard the bone handoff strikes, as a multiple of the smallest blow that parts two pieces.
    pub blow: f32,
}

impl GoreBreakable {
    /// Take a bake, annotate every cap for `region` at `scale` so the flesh material can band it,
    /// and bake the fracture modes over its bonds.
    pub fn from_fracture(fracture: crate::Fracture, region: Region, scale: &bevy_cross_section::Scale) -> Self {
        let layers = Layers::for_region(region);
        let modes = crate::modal::bake_modes(
            &fracture.bonds,
            |id| fracture.solids().get(id.index()).map(|s| &s.cell),
            &bevy_fracture_modes::ModeSettings::default(),
        )
        .ok();
        let bonds = fracture.bonds.clone();
        let mut leaves = fracture.into_leaves();
        for leaf in &mut leaves {
            leaf.annotate_cap(&layers, scale);
        }
        Self { leaves, bonds, modes, region, blow: 2.5 }
    }

    /// Which leaves a blow at `at_local` throws: the islands the modes part, all but the largest.
    /// Without modes, every leaf.
    fn thrown_by(&self, at_local: Vec3) -> Vec<crate::FragmentId> {
        let Some(modes) = &self.modes else {
            return self.leaves.iter().map(|l| l.id).collect();
        };
        let Some(struck) = self
            .leaves
            .iter()
            .min_by(|a, b| {
                let da = a.cell.center().distance_squared(at_local);
                let db = b.cell.center().distance_squared(at_local);
                da.partial_cmp(&db).unwrap_or(core::cmp::Ordering::Equal).then(a.id.cmp(&b.id))
            })
            .map(|l| l.id)
        else {
            return Vec::new();
        };
        let magnitude = modes.impulse_for(struck, 2).unwrap_or(0.1) * self.blow;
        let broken = modes.break_at(struck, magnitude);
        let mut severed = crate::BondSet::new(&self.bonds);
        severed.sever_all(&broken);
        let islands = self.bonds.islands(self.bonds.members(), &severed);
        let keep = islands
            .iter()
            .enumerate()
            .max_by_key(|(i, island)| (island.len(), core::cmp::Reverse(*i)))
            .map(|(i, _)| i);
        islands
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != keep)
            .flat_map(|(_, island)| island.iter().copied())
            .collect()
    }
}

/// **What lies under intact skin**: bruises and burns, as an image the flesh material blends under
/// the epidermis. CPU-authoritative like the canvases, and uploaded by the preset when it changes.
///
/// One RGBA8 image, linear: `rgb` is the **ratio** the skin's colour is multiplied by where the
/// dermis shows through — a bruise's colour over the same skin with nothing in it, so a skin tone
/// stays the caller's — and `a` is how much of it shows. A peeled texel (the flaymap's `A`) hides
/// it: the skin that carried it is gone.
#[derive(Component, Debug)]
pub struct DermisCanvas {
    size: u32,
    px: Vec<u8>,
    image: Handle<Image>,
    dirty: bool,
}

impl DermisCanvas {
    fn new(images: &mut Assets<Image>, size: u32) -> Self {
        let size = size.max(1);
        let extent = bevy::render::render_resource::Extent3d { width: size, height: size, depth_or_array_layers: 1 };
        let image = Image::new_fill(
            extent,
            bevy::render::render_resource::TextureDimension::D2,
            &[255, 255, 255, 0],
            bevy::render::render_resource::TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
        );
        Self { size, px: [255, 255, 255, 0].repeat((size as usize) * (size as usize)), image: images.add(image), dirty: false }
    }

    /// The image the flesh material reads.
    pub fn image(&self) -> Handle<Image> {
        self.image.clone()
    }

    /// Edge length, texels.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Paint a radial field: `colour(r_mm)` returns the linear ratio the skin is multiplied by and an
    /// alpha, for the texel `r_mm` from `uv`, out to `radius_uv`. Alpha is taken as the max of what
    /// was there, so two injuries overlap rather than erase.
    pub fn paint_radial(&mut self, uv: Vec2, radius_uv: f32, mm_per_uv: f32, mut colour: impl FnMut(f32) -> ([f32; 3], f32)) {
        if !uv.is_finite() || !radius_uv.is_finite() || radius_uv <= 0.0 {
            return;
        }
        let n = self.size as i64;
        // Saturated casts, then clamped: a huge radius or an off-atlas UV from a caller's mesh must
        // not overflow the bounds arithmetic below.
        let r_px = ((radius_uv * self.size as f32).ceil() as i64).clamp(0, n);
        let cx = ((uv.x * self.size as f32) as i64).clamp(-n, 2 * n);
        let cy = ((uv.y * self.size as f32) as i64).clamp(-n, 2 * n);
        for y in (cy - r_px).max(0)..(cy + r_px + 1).min(n) {
            for x in (cx - r_px).max(0)..(cx + r_px + 1).min(n) {
                let du = (x as f32 + 0.5) / self.size as f32 - uv.x;
                let dv = (y as f32 + 0.5) / self.size as f32 - uv.y;
                let r_uv = (du * du + dv * dv).sqrt();
                if r_uv > radius_uv {
                    continue;
                }
                let (c, a) = colour(r_uv * mm_per_uv);
                let a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 };
                if a <= 0.0 {
                    continue;
                }
                let i = ((y * n + x) * 4) as usize;
                if let Some(px) = self.px.get_mut(i..i + 4) {
                    let old_a = px[3] as f32 / 255.0;
                    let w = if a + old_a > 0.0 { a / (a + old_a) } else { 0.0 };
                    for ch in 0..3 {
                        let new = c[ch].clamp(0.0, 1.0);
                        let old = px[ch] as f32 / 255.0;
                        px[ch] = ((old + (new - old) * w) * 255.0).round() as u8;
                    }
                    px[3] = (a.max(old_a) * 255.0).round() as u8;
                    self.dirty = true;
                }
            }
        }
    }

    /// Upload if anything changed.
    pub fn flush(&mut self, images: &mut Assets<Image>) {
        if !self.dirty {
            return;
        }
        if let Some(mut image) = images.get_mut(&self.image)
            && let Some(data) = image.data.as_mut()
            && data.len() == self.px.len()
        {
            data.copy_from_slice(&self.px);
            self.dirty = false;
        }
    }
}

/// **A bruise ageing under this entity's skin**, at one spot. The kernel is `bloodstain::bruise`; the
/// preset steps it on the fixed tick and repaints the dermis canvas when its colour moves.
#[derive(Component, Debug)]
pub struct Bruising {
    /// Where on the canvas.
    pub uv: Vec2,
    /// The kernel.
    pub bruise: bloodstain::Bruise,
    /// Fractional model steps carried between ticks.
    pub carry: f32,
}

/// **Blood soaking into this cloth** from one landing: the Lucas–Washburn front (`bloodstain::wick`)
/// widens the stain over the seconds after it lands.
#[derive(Component, Debug, Default)]
pub struct Soaking {
    /// `(ray origin, ray direction, silhouette in metres, tick it landed)` per landing still spreading.
    pub landings: Vec<(Vec3, Vec3, StainShape, u32)>,
}

/// A thrown fragment the preset integrates.
#[derive(Component, Clone, Copy, Debug)]
pub struct Flying(pub Vec3);

/// A piece of a broken body that stayed where it stood — the island the modes did not part. Marked
/// so a caller resetting a scene can find it; the body it came from is gone.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Standing;

/// A rendered gut tube the preset keeps in step with its strand.
#[derive(Component, Clone, Copy, Debug)]
pub struct Gut;

/// **The preset's dials.** Not `deny_unknown_fields` and not serialised: a game that authors these
/// from data does it through its own settings type.
#[derive(Resource, Clone, Debug)]
pub struct GoreDials {
    /// Canvas edge in texels for every surface the preset dresses. 128 ships; 256 is the ceiling.
    pub canvas: u32,
    /// The scale of the game's meshes, millimetres per unit. `1000` for metres.
    pub mm_per_unit: f32,
    /// The blood model.
    pub blood: BloodSettings,
    /// Fixed ticks per second, for the bleed schedule.
    pub hz: u32,
    /// Strands a torso hit spills once it reaches muscle. `0` disables viscera.
    pub strands: u32,
    /// Whether a bone handoff on a baked subject spawns and throws its fragments here.
    pub spawn_fragments: bool,
    /// How many pieces a thrown body breaks into — the frontier the bake is read back at.
    pub pieces: usize,
    /// The plane thrown pieces stop on and stains land on, world `y`.
    pub floor_y: f32,
    /// Gravity for thrown pieces, world units per second squared.
    pub gravity: f32,
    /// Supersamples along a stain's edge on the wetmap — `bevy_wetmap`'s `edge_samples`.
    pub edge_samples: u32,
    /// Canvas edge for a [`Gore::floor`], texels. Floors are metres across where a limb is
    /// centimetres, so they get the ceiling by default.
    pub floor_canvas: u32,
    /// **How fast a bruise ages**: hours of `bloodstain::bruise` time per fixed tick. The model's
    /// own step is 0.1 h; at the shipped `0.02` a bruise runs its ten-day course in about four
    /// minutes of play, which is time-lapse and says so. `0` freezes every bruise.
    pub bruise_hours_per_tick: f32,
    /// **How long a burn's heat keeps working after contact**, seconds of `bloodstain::burn` time
    /// integrated at hit time under still air — damage accrues after the heat is removed, and this
    /// is how much of that the hit accounts for at once.
    pub burn_cooling_s: f32,
}

impl Default for GoreDials {
    fn default() -> Self {
        Self {
            canvas: 128,
            mm_per_unit: 1000.0,
            blood: BloodSettings::default(),
            hz: 60,
            strands: 5,
            spawn_fragments: true,
            pieces: 12,
            floor_y: 0.0,
            gravity: 9.81,
            edge_samples: 4,
            floor_canvas: 256,
            bruise_hours_per_tick: 0.02,
            burn_cooling_s: 5.0,
        }
    }
}

/// The fixed tick every canvas, bleed and laceration is measured against.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GoreClock(pub u32);

/// **What kind of hit** — the four injuries the family models.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HitKind {
    /// Something went in: a crater `depth_mm` deep that bleeds. Past the cortex start it is a bone
    /// handoff.
    Impact { depth_mm: f32 },
    /// Something was drawn across: a `bevy_laceration` along the blade's travel that gapes onto a
    /// bed `depth_mm` deep.
    Slash { along: Vec3, depth_mm: f32 },
    /// Something hit without opening the skin: a `bloodstain::bruise` that pools, spreads and turns
    /// from red through to yellow under intact skin, over the hours [`GoreDials::bruise_hours_per_tick`]
    /// sets.
    Blunt,
    /// Something hot touched the skin for `seconds` at `temp_c`: a `bloodstain::burn` whose damage
    /// integral decides the degree — reddened, blistered, or charred. Nothing is removed: an eschar
    /// is dead tissue that stays where it is.
    Burn { temp_c: f32, seconds: f32 },
}

/// **A hit, addressed to a dressed entity.**
#[derive(Message, Clone, Copy, Debug)]
pub struct GoreHit {
    /// The entity to hurt. It needs a [`Gore`] and a `Mesh3d`.
    pub entity: Entity,
    /// Ray origin, world.
    pub from: Vec3,
    /// Ray direction, world. Need not be unit.
    pub dir: Vec3,
    /// Radius of the injury on the surface, metres.
    pub radius_m: f32,
    /// Which injury.
    pub kind: HitKind,
    /// The blood's saturation: arterial for a breached vessel, venous otherwise.
    pub so2: f32,
}

impl GoreHit {
    /// An impact: a crater `radius_m` wide, `depth_mm` deep.
    pub fn impact(entity: Entity, from: Vec3, dir: Vec3, radius_m: f32, depth_mm: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Impact { depth_mm }, so2: bloodstain::spectral::SO2_VENOUS }
    }
    /// A slash `along` the blade's travel, `radius_m` half its length.
    pub fn slash(entity: Entity, from: Vec3, dir: Vec3, along: Vec3, radius_m: f32, depth_mm: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Slash { along, depth_mm }, so2: bloodstain::spectral::SO2_VENOUS }
    }
    /// A blunt blow: a bruise `radius_m` across under intact skin.
    pub fn blunt(entity: Entity, from: Vec3, dir: Vec3, radius_m: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Blunt, so2: bloodstain::spectral::SO2_VENOUS }
    }
    /// A contact burn `radius_m` across: `temp_c` for `seconds`.
    pub fn burn(entity: Entity, from: Vec3, dir: Vec3, radius_m: f32, temp_c: f32, seconds: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Burn { temp_c, seconds }, so2: bloodstain::spectral::SO2_VENOUS }
    }
    /// The same hit, bleeding arterial blood.
    pub fn arterial(mut self) -> Self {
        self.so2 = bloodstain::spectral::SO2_ARTERIAL;
        self
    }
    /// How much tissue this hit removes, millimetres; zero for a blow or a burn (a burn's depth is
    /// the model's to decide).
    pub fn depth_mm(&self) -> f32 {
        match self.kind {
            HitKind::Impact { depth_mm } | HitKind::Slash { depth_mm, .. } => depth_mm,
            HitKind::Blunt | HitKind::Burn { .. } => 0.0,
        }
    }
}

/// **A baked subject came apart under the preset.** Written on the bone handoff whether or not the
/// preset spawned the pieces, so a game with physics can spawn its own from
/// [`FractureCache::leaves`].
#[derive(Message, Clone, Copy, Debug)]
pub struct GoreFractured {
    /// The subject root.
    pub subject: Entity,
    /// Where the blow landed, world.
    pub at: Vec3,
}

/// Marks a dressed [`Gore::floor`], so the bleed can find the floors without a second `Gore` read.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct FloorTag;

/// How many of a wound's droplets are cast at each other surface. The floor gets the whole spray in
/// closed form; this bounds the per-surface ray casts.
const SPRAY_CAST: usize = 24;

/// Everything the preset runs, on `FixedUpdate`, in this order.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GoreSystems;

/// **Adds every plugin the family has, and the preset's systems.** Each leaf plugin is added only
/// if the app does not already have it, so a game that configured one keeps its configuration.
pub struct GorePlugin;

impl Plugin for GorePlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<CarnagePlugin>() {
            app.add_plugins(CarnagePlugin);
        }
        if !app.is_plugin_added::<CrossSectionPlugin>() {
            app.add_plugins(CrossSectionPlugin);
        }
        if !app.is_plugin_added::<FlaymapPlugin>() {
            app.add_plugins(FlaymapPlugin);
        }
        if !app.is_plugin_added::<WetmapPlugin>() {
            app.add_plugins(WetmapPlugin);
        }
        if !app.is_plugin_added::<LacerationPlugin>() {
            app.add_plugins(LacerationPlugin);
        }
        if !app.is_plugin_added::<VisceraPlugin>() {
            app.add_plugins(VisceraPlugin);
        }
        if !app.is_plugin_added::<FleshPlugin>() {
            app.add_plugins(FleshPlugin);
        }
        app.init_resource::<GoreDials>()
            .init_resource::<GoreClock>()
            .add_message::<GoreHit>()
            .add_message::<GoreFractured>()
            .configure_sets(FixedUpdate, GoreSystems.before(VisceraSystems))
            .add_systems(
                FixedUpdate,
                (advance_clock, dress, take_hits, throw_bodies, bleed, age_bruises, soak_cloth, tick_canvases, flush_dermis, fly)
                    .chain()
                    .in_set(GoreSystems),
            )
            .add_systems(FixedUpdate, retube.after(VisceraSystems));
    }
}

/// One tick per fixed step, mirrored into the laceration clock so both agree.
fn advance_clock(mut clock: ResMut<GoreClock>, lac: Option<ResMut<LacerationClock>>) {
    clock.0 = clock.0.saturating_add(1);
    if let Some(mut lac) = lac {
        lac.0 = clock.0;
    }
}

/// **Dress every new [`Gore`] entity**: canvases, and the flesh material over its own material.
#[allow(clippy::too_many_arguments)]
fn dress(
    mut commands: Commands,
    dials: Res<GoreDials>,
    tables: Res<FleshTables>,
    settings: Option<Res<CrossSectionSettings>>,
    wet_settings: Option<Res<WetSettings>>,
    images: Option<ResMut<Assets<Image>>>,
    standard: Option<Res<Assets<StandardMaterial>>>,
    flesh: Option<ResMut<Assets<FleshMaterial>>>,
    undressed: Query<(Entity, &Gore, Option<&MeshMaterial3d<StandardMaterial>>), (With<Mesh3d>, Without<Dressed>)>,
) {
    let (Some(mut images), Some(standard), Some(mut flesh)) = (images, standard, flesh) else {
        return;
    };
    if tables.sss == Handle::default() {
        // The tables are baked on `Startup`; a `FixedUpdate` before that would dress with nothing.
        return;
    }
    let scale = settings.as_ref().map(|s| s.scale).unwrap_or_default();
    for (entity, gore, material) in &undressed {
        let base = material.and_then(|m| standard.get(&m.0).cloned()).unwrap_or_default();
        let base_srgb = base.base_color.to_srgba();
        let base_srgb = [base_srgb.red, base_srgb.green, base_srgb.blue];
        let rough = base.perceptual_roughness;
        let mut entity_commands = commands.entity(entity);
        entity_commands
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(Dressed { original: material.map(|m| m.0.clone()) });
        match gore.kind {
            GoreKind::Skin(region) => {
                let layers = layers_for(region, settings.as_deref());
                let flay = FlayCanvas::new(&mut images, dials.canvas, region, layers, base_srgb, rough);
                let wet = WetCanvas::new(&mut images, dials.canvas, base_srgb, rough);
                let dressed = StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: Some(flay.albedo()),
                    metallic_roughness_texture: Some(flay.roughness()),
                    perceptual_roughness: 1.0,
                    ..base
                };
                let mut params = FleshParams::for_layers(&layers, FleshMode::Canvas, scale.mm_per_unit);
                params.wet.z = wet_settings.as_ref().map(|w| w.film_depth_mm).unwrap_or(2.0);
                let dermis = DermisCanvas::new(&mut images, dials.canvas);
                let mut material = tables.material(dressed, params, Some(wet.roughness()), Some(flay.roughness()));
                material.extension.set_dermis(Some(dermis.image()));
                entity_commands.insert((MeshMaterial3d(flesh.add(material)), flay, wet, dermis));
            }
            GoreKind::Cloth => {
                let wet = WetCanvas::new(&mut images, dials.canvas, base_srgb, rough);
                let layers = Layers::for_region(Region::Torso);
                let mut params = FleshParams::for_layers(&layers, FleshMode::Cloth, scale.mm_per_unit);
                params.wet.z = wet_settings.as_ref().map(|w| w.film_depth_mm).unwrap_or(2.0);
                let material = tables.material(base, params, Some(wet.roughness()), None);
                entity_commands.insert((MeshMaterial3d(flesh.add(material)), wet, Soaking::default()));
            }
            GoreKind::Floor => {
                let wet = WetCanvas::new(&mut images, dials.floor_canvas, base_srgb, rough);
                let layers = Layers::for_region(Region::Torso);
                let mut params = FleshParams::for_layers(&layers, FleshMode::Cloth, scale.mm_per_unit);
                params.wet.z = wet_settings.as_ref().map(|w| w.film_depth_mm).unwrap_or(2.0);
                params.sss.x = 0.0;
                let material = tables.material(base, params, Some(wet.roughness()), None);
                entity_commands.insert((MeshMaterial3d(flesh.add(material)), wet, FloorTag));
            }
        }
    }
}

fn layers_for(region: Region, settings: Option<&CrossSectionSettings>) -> Layers {
    settings.map(|s| *s.layers(region)).unwrap_or_else(|| Layers::for_region(region))
}

/// The settings, caches and outputs [`take_hits`] reads — grouped so the system stays under Bevy's
/// parameter limit and the hit loop reads as the module docs list it.
#[derive(bevy::ecs::system::SystemParam)]
struct HitContext<'w> {
    clock: Res<'w, GoreClock>,
    dials: Res<'w, GoreDials>,
    flay_settings: Option<Res<'w, FlaySettings>>,
    wet_settings: Option<Res<'w, WetSettings>>,
    visc: Option<Res<'w, ViscSettings>>,
    cache: Res<'w, FractureCache>,
    tables: Res<'w, FleshTables>,
    atlas: Option<Res<'w, bevy_cross_section::CrossSectionAtlas>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    standard: ResMut<'w, Assets<StandardMaterial>>,
    flesh: ResMut<'w, Assets<FleshMaterial>>,
    bone: MessageWriter<'w, BoneExposed>,
    wounded: MessageWriter<'w, Wounded>,
    fractured: MessageWriter<'w, GoreFractured>,
}

/// The scene graph [`take_hits`] walks to find a hit's baked subject and its skin material.
#[derive(bevy::ecs::system::SystemParam)]
struct SubjectLookup<'w, 's> {
    parents: Query<'w, 's, &'static ChildOf>,
    subjects: Query<'w, 's, (&'static FractureSubject, &'static GlobalTransform, Option<&'static Children>)>,
    breakables: Query<'w, 's, (&'static GlobalTransform, Option<&'static Children>), With<GoreBreakable>>,
    dressed: Query<'w, 's, &'static Dressed>,
}

/// **Apply every hit.** See the module docs for the order.
fn take_hits(
    mut commands: Commands,
    mut hits: MessageReader<GoreHit>,
    mut cx: HitContext,
    lookup: SubjectLookup,
    mut bodies: Query<(&Gore, &Mesh3d, &GlobalTransform, Option<&mut FlayCanvas>, Option<&mut WetCanvas>), With<Dressed>>,
    bodies_index: Query<(Entity, &Gore, &Mesh3d, &GlobalTransform), With<Dressed>>,
) {
    let HitContext {
        clock,
        dials,
        flay_settings,
        wet_settings,
        visc,
        cache,
        tables,
        atlas,
        meshes,
        standard,
        flesh,
        bone,
        wounded,
        fractured,
    } = &mut cx;
    let SubjectLookup { parents, subjects, breakables, dressed } = &lookup;
    let tick = clock.0;
    // Every hit of the tick, in one pass. A hit that reaches bone queues the subject's despawn, and a
    // later hit in the same batch on a sibling part still passes the query — so every insert below
    // is a `try_insert`, which tolerates a despawn queued ahead of it where `insert` panics.
    let hits: Vec<GoreHit> = hits.read().copied().collect();
    if hits.is_empty() {
        return;
    }
    let edges = edge_settings(dials, wet_settings.as_deref());
    // Every dressed surface, for what a wound throws onto the others.
    let surfaces: Vec<(Entity, Handle<Mesh>, GlobalTransform, Gore)> = bodies_index
        .iter()
        .map(|(e, g, m, x)| (e, m.0.clone(), *x, *g))
        .collect();
    for hit in hits {
        let Ok((gore, mesh3d, xf, flay, wet)) = bodies.get_mut(hit.entity) else { continue };
        let gore = *gore;
        let gore = &gore;
        let xf = *xf;
        let xf = &xf;
        let Some(mesh) = meshes.get(&mesh3d.0).cloned() else { continue };
        let radius_uv = (hit.radius_m * gore.uv_per_metre / bevy_flaymap::UV_SPAN_M).max(0.0);
        let region = match gore.kind {
            GoreKind::Skin(r) => r,
            _ => Region::Torso,
        };

        // 1. Peel. A slash opens geometry instead; a blow and a burn go through the dermis below.
        let mut handoff = None;
        let mut deepest = Layer::Skin;
        let mut burn_degree = bloodstain::burn::Degree::None;
        if let HitKind::Burn { temp_c, seconds } = hit.kind {
            // The whole exposure, then the cooling the dial allows, integrated at once: what the
            // model says the contact did. A burn removes nothing — an eschar is dead tissue that
            // stays on the surface — so it never peels; the degree decides what the dermis shows.
            let mut burn = bloodstain::Burn::new();
            burn.expose(temp_c, seconds);
            burn.expose(bloodstain::burn::SURFACE_REST_C, dials.burn_cooling_s);
            burn_degree = burn.degree();
        }
        if let Some(mut flay) = flay
            && let HitKind::Impact { depth_mm: peel_depth } = hit.kind
        {
            handoff = flay.paint_world(&mesh, xf, hit.from, hit.dir, radius_uv, peel_depth, tick);
            if let Some(s) = flay_settings.as_deref() {
                flay.shade(s);
            }
            if let Some(h) = handoff {
                deepest = h.deepest_layer;
            }
        }
        // Where the ray lands, for every kind of hit — a slash and a blow never peel, so they have no
        // handoff to read a point from.
        let landed = ray_hit(&mesh, xf, hit.from, hit.dir);
        let site_uv = landed.map(|(uv, ..)| uv);

        // A blow: a bruise starts under the skin, pooled to the blow's radius.
        if let HitKind::Blunt = hit.kind
            && let Some(uv) = site_uv
        {
            let params = bloodstain::bruise::Params {
                pool_diameter_mm: (hit.radius_m * 2.0 * 1000.0).clamp(2.0, 100.0),
                ..bloodstain::bruise::Params::default()
            };
            commands.entity(hit.entity).try_insert(Bruising { uv, bruise: bloodstain::Bruise::new(params), carry: 0.0 });
        }

        // A burn: the degree colours the dermis — reddened, blistered, charred. **These three colours
        // are this crate's own**; the degree that picks one and the depth that is peeled are the model's.
        if let HitKind::Burn { .. } = hit.kind
            && let Some(uv) = site_uv
            && let Ok(mut entity_ref) = commands.get_entity(hit.entity)
        {
            let (tint, alpha): ([f32; 3], f32) = match burn_degree {
                bloodstain::burn::Degree::None => ([1.0; 3], 0.0),
                // Erythema: the dermis flushes, so red is kept and green and blue are taken.
                bloodstain::burn::Degree::First => ([1.0, 0.45, 0.40], 0.7),
                // A blister: the epidermis lifts white over a pale bed.
                bloodstain::burn::Degree::Second => ([1.0, 0.92, 0.80], 0.6),
                // Eschar: charred, near black.
                bloodstain::burn::Degree::Third => ([0.10, 0.06, 0.04], 1.0),
            };
            let radius_uv_burn = radius_uv;
            let mm_per_uv = 1000.0 / gore.uv_per_metre.max(1.0e-6);
            entity_ref.queue(move |mut e: EntityWorldMut| {
                if let Some(mut dermis) = e.get_mut::<DermisCanvas>() {
                    dermis.paint_radial(uv, radius_uv_burn, mm_per_uv, |r_mm| {
                        let edge = 1.0 - (r_mm / (radius_uv_burn * mm_per_uv)).clamp(0.0, 1.0);
                        (tint, alpha * edge.sqrt())
                    });
                }
            });
        }
        if matches!(hit.kind, HitKind::Blunt | HitKind::Burn { .. }) {
            // No open wound: nothing bleeds, nothing is thrown, nothing spills.
            continue;
        }
        let at_local = handoff.and_then(|h| h.at).or(landed.map(|(_, p, _)| p));
        let normal_local = handoff.and_then(|h| h.normal).or(landed.map(|(_, _, n)| n));
        let (at_world, normal_world) = match (at_local, normal_local) {
            (Some(p), Some(n)) => (xf.transform_point(p), xf.affine().transform_vector3(n).normalize_or_zero()),
            _ => (hit.from, -hit.dir.normalize_or_zero()),
        };
        let normal_world = if normal_world.dot(-hit.dir) < 0.0 { -normal_world } else { normal_world };

        // 2. Blood at the wound and on the floors.
        let wound = BloodWound {
            at: [at_world.x, at_world.y, at_world.z],
            normal: [normal_world.x, normal_world.y, normal_world.z],
            area: core::f32::consts::PI * hit.radius_m * hit.radius_m,
            severity: 1.0,
            kind: if hit.depth_mm() > 0.0 { WoundKind::Channel } else { WoundKind::Severance },
        };
        let seed = wound_seed(&wound);
        let shape = StainShape {
            major: hit.radius_m * 2.6,
            minor: hit.radius_m * 2.0,
            spines: 4,
            satellites: 2,
            direction: [0.0, 1.0],
            seed,
        };
        if let Some(mut wet) = wet {
            let texels = wet.size();
            let _ = wet.paint_world_with(&mesh, xf, hit.from, hit.dir, &shape_on(gore, &shape, texels), tick, &edges);
        }
        // What the wound throws lands on everything else: the closed-form stains on every floor, and
        // the spray's own droplets cast straight at every other dressed surface for what a floor's
        // plane cannot catch — a sheet, a wall, the next body part. Straight rather than ballistic
        // for those: a droplet at 8–40 m/s falls under a centimetre in the metre it travels.
        let droplets = bloodstain::patterns::impact_spatter(&wound, &dials.blood);
        for (other, other_mesh, other_xf, other_gore) in &surfaces {
            if *other == hit.entity {
                continue;
            }
            let Ok((_, _, _, _, Some(mut other_wet))) = bodies.get_mut(*other) else { continue };
            let Some(other_mesh) = meshes.get(other_mesh) else { continue };
            match other_gore.kind {
                GoreKind::Floor => {
                    let plane_y = other_xf.translation().y;
                    for stain in bloodstain::stain::stains(&wound, &dials.blood, plane_y) {
                        let at = Vec3::new(stain.at[0], stain.at[1] + 0.5, stain.at[2]);
                        let shape = StainShape {
                            major: stain.radius * 2.2,
                            minor: stain.radius * 2.0,
                            spines: 3,
                            satellites: 1,
                            direction: [0.0, 1.0],
                            seed: stain.seed,
                        };
                        let texels = other_wet.size();
                        let _ = other_wet.paint_world_with(other_mesh, other_xf, at, Vec3::NEG_Y, &shape_on(other_gore, &shape, texels), tick, &edges);
                    }
                }
                _ => {
                    let mut landed = Vec::new();
                    for (i, d) in droplets.iter().enumerate().take(SPRAY_CAST) {
                        let dir = Vec3::new(d.dir[0], d.dir[1], d.dir[2]);
                        let radius = bloodstain::stain::stain_radius(d, d.speed, &dials.blood);
                        let shape = StainShape {
                            major: radius * 2.4,
                            minor: radius * 1.6,
                            spines: 2,
                            satellites: 1,
                            direction: [0.0, 1.0],
                            seed: seed.wrapping_add(i as u32),
                        };
                        let texels = other_wet.size();
                        if other_wet.paint_world_with(other_mesh, other_xf, at_world, dir, &shape_on(other_gore, &shape, texels), tick, &edges)
                            && other_gore.kind == GoreKind::Cloth
                        {
                            landed.push((at_world, dir, shape, tick));
                        }
                    }
                    if !landed.is_empty()
                        && let Ok(mut e) = commands.get_entity(*other)
                    {
                        e.queue(move |mut e: EntityWorldMut| {
                            if let Some(mut soaking) = e.get_mut::<Soaking>() {
                                soaking.landings.extend(landed);
                            }
                        });
                    }
                }
            }
        }
        commands.entity(hit.entity).try_insert(WoundSite {
            from: hit.from,
            dir: hit.dir,
            wound,
            bleed: bloodstain::Bleed::new(tick, &wound),
        });
        wounded.write(Wounded {
            at: at_world,
            normal: normal_world,
            area: wound.area,
            severity: 1.0,
            kind: wound.kind,
            class: if hit.so2 >= bloodstain::spectral::SO2_ARTERIAL {
                bloodstain::PatternClass::ArterialSpurt
            } else {
                bloodstain::PatternClass::Impact
            },
        });

        // 4. A slash gapes instead of cratering.
        if let HitKind::Slash { along, depth_mm } = hit.kind
            && let Some(p) = at_local
        {
            let inv = xf.affine().inverse();
            let along_local = inv.transform_vector3(along).normalize_or_zero();
            let n_local = normal_local.unwrap_or(Vec3::Y);
            let half = hit.radius_m / xf.scale().max_element().max(1.0e-6);
            let source = meshes.add(mesh.clone());
            commands.entity(hit.entity).try_insert(Laceration {
                path: vec![p - along_local * half, p, p + along_local * half],
                normal: n_local,
                gape: Gape::default(),
                tension: Tension::default(),
                bed_depth_mm: depth_mm.max(1.0),
                region,
                opened_at: tick,
                source,
                ..Default::default()
            });
        }

        // 5. Guts, from a torso that has been opened to the muscle.
        if region == Region::Torso
            && dials.strands > 0
            && matches!(deepest, Layer::Muscle | Layer::Cortex | Layer::Marrow)
            && let Some(visc) = visc.as_deref()
        {
            let strands = spill(at_world, normal_world + Vec3::new(0.0, -0.4, 0.0), dials.strands, seed, visc);
            let mut muscle_params = FleshParams::for_layers(&Layers::for_region(region), FleshMode::Cap, dials.mm_per_unit);
            // Every texel is muscle: the strands carry no `UV_1`, so the shader's depth is `0`, and
            // `layer_of` walks past every band start it is not below. Muscle is index 2, so fat and
            // muscle start at `0` and cortex and marrow start past any depth. (`Vec4::ZERO` selected
            // marrow, whose profile is fat's — caught in review.)
            muscle_params.bands = Vec4::new(0.0, 0.0, 2.0, 2.0);
            let gut_material = flesh.add(tables.material(
                StandardMaterial { base_color: Color::srgb(0.55, 0.20, 0.22), perceptual_roughness: 0.25, ..default() },
                muscle_params,
                None,
                None,
            ));
            for strand in strands {
                let anchors = (0..strand.nodes().len() as u32).step_by(4).map(|i| (i, at_world)).collect();
                let tube = meshes.add(tube_mesh(&strand, 8));
                commands.spawn((
                    Mesh3d(tube),
                    MeshMaterial3d(gut_material.clone()),
                    Transform::IDENTITY,
                    strand,
                    Mesentery { anchors, ..Default::default() },
                    Gut,
                ));
            }
        }

        // 6. Bone.
        if let Some(h) = handoff
            && let Some(msg) = BoneExposed::from_handoff(hit.entity, &h)
        {
            bone.write(msg);
            // Walk up to a baked subject or a runtime body and break it.
            let mut root = hit.entity;
            let mut subject = subjects.get(root).ok();
            let mut breakable = breakables.get(root).ok();
            while subject.is_none() && breakable.is_none() {
                let Ok(parent) = parents.get(root) else { break };
                root = parent.parent();
                subject = subjects.get(root).ok();
                breakable = breakables.get(root).ok();
            }
            let skin_of = |children: Option<&Children>| {
                children
                    .into_iter()
                    .flatten()
                    .find_map(|c| dressed.get(*c).ok().and_then(|d| d.original.clone()))
            };
            if let Some((source, root_xf, children)) = subject
                && cache.is_baked(source.0.id())
            {
                fractured.write(GoreFractured { subject: root, at: at_world });
                if dials.spawn_fragments {
                    let skin = skin_of(children).unwrap_or_else(|| standard.add(StandardMaterial::default()));
                    let region = cache.region(source.0.id()).unwrap_or(region);
                    let pieces: Vec<Piece> = cache
                        .frontier_of(source.0.id(), dials.pieces)
                        .into_iter()
                        .map(|f| Piece {
                            outer: f.outer_mesh.clone(),
                            cap: f.cap_mesh.clone(),
                            center_local: f.center_local,
                            size: f.half_extents.length(),
                        })
                        .collect();
                    let cap = cap_material(region, atlas.as_deref(), standard, tables, flesh, dials);
                    throw_pieces(&mut commands, &pieces, root_xf, at_world, cap, skin);
                    commands.entity(root).despawn();
                }
            } else if let Some((root_xf, children)) = breakable {
                fractured.write(GoreFractured { subject: root, at: at_world });
                if dials.spawn_fragments {
                    let skin = skin_of(children).unwrap_or_else(|| standard.add(StandardMaterial::default()));
                    let cap = cap_material(region, atlas.as_deref(), standard, tables, flesh, dials);
                    let at_local = root_xf.affine().inverse().transform_point3(at_world);
                    commands.entity(root).queue(move |mut e: EntityWorldMut| {
                        if let Some(body) = e.take::<GoreBreakable>() {
                            e.insert(ThrowNow { body, at: at_world, at_local, skin, cap });
                        }
                    });
                }
            }
        }
    }
}

/// One piece to throw, in subject-local space.
struct Piece {
    outer: Option<Handle<Mesh>>,
    cap: Option<Handle<Mesh>>,
    center_local: Vec3,
    size: f32,
}

/// A runtime body whose pieces are thrown on the next tick — the meshes need `Assets<Mesh>`, which
/// the hit loop does not hold.
#[derive(Component)]
struct ThrowNow {
    body: GoreBreakable,
    at: Vec3,
    at_local: Vec3,
    skin: Handle<StandardMaterial>,
    cap: Handle<FleshMaterial>,
}

/// The cap material for a region: the cross-section strip the atlas baked, worn by the flesh
/// material in [`FleshMode::Cap`] so `UV_1` picks the tissue. Without an atlas, a plain red.
fn cap_material(
    region: Region,
    atlas: Option<&bevy_cross_section::CrossSectionAtlas>,
    standard: &Assets<StandardMaterial>,
    tables: &FleshTables,
    flesh: &mut Assets<FleshMaterial>,
    dials: &GoreDials,
) -> Handle<FleshMaterial> {
    let layers = Layers::for_region(region);
    let base = atlas
        .and_then(|a| a.material(region))
        .and_then(|h| standard.get(&h).cloned())
        .unwrap_or_else(|| StandardMaterial { base_color: Color::srgb(0.55, 0.12, 0.12), perceptual_roughness: 0.4, ..default() });
    flesh.add(tables.material(base, FleshParams::for_layers(&layers, FleshMode::Cap, dials.mm_per_unit), None, None))
}

/// Spawn a frontier as flying pieces: skin outside, a banded cap inside.
fn throw_pieces(
    commands: &mut Commands,
    pieces: &[Piece],
    root_xf: &GlobalTransform,
    at: Vec3,
    cap: Handle<FleshMaterial>,
    skin: Handle<StandardMaterial>,
) {
    for piece in pieces {
        let centre = root_xf.transform_point(piece.center_local);
        let away = (centre - at).normalize_or_zero();
        let velocity = (away + Vec3::new(0.0, 0.6, 0.0)) * (1.5 / (1.0 + piece.size));
        let at_tf = Transform::from_translation(centre).with_rotation(root_xf.rotation()).with_scale(root_xf.scale());
        if let Some(outer) = piece.outer.clone() {
            commands.spawn((Mesh3d(outer), MeshMaterial3d(skin.clone()), at_tf, Flying(velocity)));
        }
        if let Some(cap_mesh) = piece.cap.clone() {
            commands.spawn((Mesh3d(cap_mesh), MeshMaterial3d(cap.clone()), at_tf, Flying(velocity)));
        }
    }
}

/// Throw a runtime body's leaves: upload the meshes, spawn the pieces, despawn the body.
fn throw_bodies(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut bodies: Query<(Entity, &GlobalTransform, &mut ThrowNow)>,
) {
    for (entity, xf, mut throw) in &mut bodies {
        let flying = throw.body.thrown_by(throw.at_local);
        let skin = throw.skin.clone();
        let cap = throw.cap.clone();
        let mut thrown = Vec::new();
        // The island that stays standing is drawn where it stood: every leaf is a piece now, the
        // body's own meshes are gone with the body, and a standing piece is one with no velocity.
        for leaf in throw.body.leaves.drain(..) {
            let piece = Piece {
                outer: leaf.outer.map(|m| meshes.add(m)),
                cap: leaf.cap.map(|m| meshes.add(m)),
                center_local: leaf.center_local,
                size: leaf.half_extents.length(),
            };
            if flying.contains(&leaf.id) {
                thrown.push(piece);
            } else {
                let centre = xf.transform_point(piece.center_local);
                let at_tf = Transform::from_translation(centre).with_rotation(xf.rotation()).with_scale(xf.scale());
                if let Some(outer) = piece.outer {
                    commands.spawn((Mesh3d(outer), MeshMaterial3d(skin.clone()), at_tf, Standing));
                }
                if let Some(cap_mesh) = piece.cap {
                    commands.spawn((Mesh3d(cap_mesh), MeshMaterial3d(cap.clone()), at_tf, Standing));
                }
            }
        }
        throw_pieces(&mut commands, &thrown, xf, throw.at, cap, skin);
        commands.entity(entity).despawn();
    }
}

/// **Pulse every open wound onto its wetmap** until the clot.
fn bleed(
    clock: Res<GoreClock>,
    dials: Res<GoreDials>,
    wet_settings: Option<Res<WetSettings>>,
    meshes: Res<Assets<Mesh>>,
    mut sites: Query<(&Gore, &WoundSite, &Mesh3d, &GlobalTransform, &mut WetCanvas), Without<FloorTag>>,
    mut floors: Query<(&Gore, &Mesh3d, &GlobalTransform, &mut WetCanvas), With<FloorTag>>,
) {
    let tick = clock.0;
    let edges = edge_settings(&dials, wet_settings.as_deref());
    for (gore, site, mesh3d, xf, mut wet) in &mut sites {
        let Some(mesh) = meshes.get(&mesh3d.0) else { continue };
        let Some(pulse) = bloodstain::bleed::pulse_wound(&site.bleed, &site.wound, tick, dials.hz, &dials.blood) else {
            continue;
        };
        let r = (site.wound.area / core::f32::consts::PI).sqrt().max(0.002) * (0.6 + pulse.severity);
        let shape = StainShape {
            major: r * 1.6,
            minor: r,
            spines: 0,
            satellites: 0,
            direction: [0.0, 1.0],
            seed: site.bleed.seed ^ tick,
        };
        let texels = wet.size();
        let _ = wet.paint_world_with(mesh, xf, site.from, site.dir, &shape_on(gore, &shape, texels), tick, &edges);
        // What runs off falls straight down: one drip per systole under the wound, sized the way
        // `bloodstain::patterns::drip_trail` sizes a drip, onto every floor — so a pool grows under a
        // wound that keeps bleeding, and the wetmap's own spread does the pooling.
        let drop = bloodstain::Droplet {
            dir: [0.0, -1.0, 0.0],
            speed: bloodstain::patterns::DRIP_FALL_SPEED,
            diameter: bloodstain::patterns::DRIP_DIAMETER_M,
        };
        let radius = bloodstain::stain::stain_radius(&drop, bloodstain::patterns::DRIP_FALL_SPEED, &dials.blood);
        let wobble = |k: u32| (bloodstain::hash_f32(site.bleed.seed ^ tick.wrapping_mul(0x9E37_79B9) ^ k) - 0.5) * 0.04;
        let at = Vec3::new(site.wound.at[0] + wobble(1), site.wound.at[1], site.wound.at[2] + wobble(2));
        for (floor_gore, floor_mesh, floor_xf, mut floor_wet) in &mut floors {
            let Some(floor_mesh) = meshes.get(&floor_mesh.0) else { continue };
            let drip = StainShape {
                major: radius * 2.0,
                minor: radius * 2.0,
                spines: 0,
                satellites: 0,
                direction: [0.0, 1.0],
                seed: site.bleed.seed ^ tick,
            };
            let texels = floor_wet.size();
            let _ = floor_wet.paint_world_with(floor_mesh, floor_xf, at, Vec3::NEG_Y, &shape_on(floor_gore, &drip, texels), tick, &edges);
        }
    }
}

/// **Age every bruise** by the dial's hours per tick and repaint its dermis when a model step lands.
fn age_bruises(
    dials: Res<GoreDials>,
    mut bruised: Query<(&Gore, &mut Bruising, &mut DermisCanvas)>,
) {
    let per_tick = dials.bruise_hours_per_tick.max(0.0) / bloodstain::bruise::STEP_H;
    for (gore, mut bruising, mut dermis) in &mut bruised {
        bruising.carry += per_tick;
        let steps = bruising.carry.floor() as u32;
        if steps == 0 {
            continue;
        }
        bruising.carry -= steps as f32;
        bruising.bruise.advance(steps);
        let mm_per_uv = 1000.0 / gore.uv_per_metre.max(1.0e-6);
        let radius_uv = bloodstain::bruise::RADIUS_MM / mm_per_uv;
        let uv = bruising.uv;
        let bruise = &bruising.bruise;
        // The ratio of the bruised skin to the same skin with nothing in it, in linear light, so the
        // caller's skin tone is what the chromophores modulate. Far out, where the model holds no
        // chromophore, the ratio is one and the alpha is zero.
        let blank = linear(bruise.srgb_through_at(bloodstain::bruise::RADIUS_MM));
        dermis.paint_radial(uv, radius_uv, mm_per_uv, |r_mm| {
            let here = linear(bruise.srgb_through_at(r_mm));
            let ratio = [
                (here[0] / blank[0].max(1.0e-4)).clamp(0.0, 1.0),
                (here[1] / blank[1].max(1.0e-4)).clamp(0.0, 1.0),
                (here[2] / blank[2].max(1.0e-4)).clamp(0.0, 1.0),
            ];
            let change = (1.0 - ratio[0]).max(1.0 - ratio[1]).max(1.0 - ratio[2]);
            (ratio, (change * 6.0).clamp(0.0, 1.0))
        });
    }
}

/// Decode encoded sRGB to linear.
fn linear(c: [f32; 3]) -> [f32; 3] {
    let d = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    [d(c[0]), d(c[1]), d(c[2])]
}

/// **Widen every landing on cloth** along the Lucas–Washburn front until it stops moving.
fn soak_cloth(
    clock: Res<GoreClock>,
    dials: Res<GoreDials>,
    wet_settings: Option<Res<WetSettings>>,
    meshes: Res<Assets<Mesh>>,
    mut cloth: Query<(&Gore, &Mesh3d, &GlobalTransform, &mut Soaking, &mut WetCanvas)>,
) {
    let tick = clock.0;
    let sheet = bloodstain::wick::Sheet::default();
    let edges = edge_settings(&dials, wet_settings.as_deref());
    for (gore, mesh3d, xf, mut soaking, mut wet) in &mut cloth {
        if soaking.landings.is_empty() {
            continue;
        }
        let Some(mesh) = meshes.get(&mesh3d.0) else { continue };
        let hz = dials.hz.max(1) as f32;
        let mut keep = Vec::with_capacity(soaking.landings.len());
        for (from, dir, shape, landed) in soaking.landings.drain(..) {
            let t_s = tick.wrapping_sub(landed) as f32 / hz;
            // Every fourth tick, to keep the stamping cheap; the front is √t so it barely moves late.
            if tick.wrapping_sub(landed) % 4 == 0 {
                let front_m = sheet.front_mm(t_s, &dials.blood) * 1.0e-3;
                let grown = StainShape {
                    major: shape.major + 2.0 * front_m,
                    minor: shape.minor + 2.0 * front_m,
                    spines: 0,
                    satellites: 0,
                    ..shape
                };
                let texels = wet.size();
                // Stamped with the current tick: the tick is the upload budget's sort key, and a
                // landing-tick stamp would let a sheet jump the queue by up to `SOAK_SECONDS`.
                let _ = wet.paint_world_with(mesh, xf, from, dir, &shape_on(gore, &grown, texels), tick, &edges);
            }
            if t_s < SOAK_SECONDS {
                keep.push((from, dir, shape, landed));
            }
        }
        soaking.landings = keep;
    }
}

/// How long a landing keeps soaking, seconds: past this the √t front is inside a texel per tick.
const SOAK_SECONDS: f32 = 6.0;

/// Upload every dermis canvas that changed.
fn flush_dermis(images: Option<ResMut<Assets<Image>>>, mut canvases: Query<&mut DermisCanvas>) {
    let Some(mut images) = images else { return };
    for mut c in &mut canvases {
        c.flush(&mut images);
    }
}

/// Run and dry every wetmap on the fixed tick.
fn tick_canvases(clock: Res<GoreClock>, settings: Option<Res<WetSettings>>, mut canvases: Query<(&Gore, &mut WetCanvas)>) {
    let Some(settings) = settings else { return };
    for (gore, mut wet) in &mut canvases {
        wet.tick(clock.0, gore.gravity_uv, &settings);
    }
}

/// Thrown pieces fall to the floor and stop.
fn fly(dials: Res<GoreDials>, time: Res<Time<Fixed>>, mut pieces: Query<(&mut Flying, &mut Transform)>) {
    let dt = time.delta_secs();
    for (mut v, mut tf) in &mut pieces {
        v.0.y -= dials.gravity * dt;
        tf.translation += v.0 * dt;
        if tf.translation.y < dials.floor_y {
            tf.translation.y = dials.floor_y;
            v.0 = Vec3::ZERO;
        }
    }
}

/// Rebuild each gut's tube from its strand after the solver moved it.
fn retube(mut meshes: ResMut<Assets<Mesh>>, guts: Query<(&Mesh3d, &Strand), With<Gut>>) {
    for (mesh, strand) in &guts {
        if let Some(mut m) = meshes.get_mut(&mesh.0) {
            let fresh = tube_mesh(strand, 8);
            if let Some(VertexAttributeValues::Float32x3(pos)) = fresh.attribute(Mesh::ATTRIBUTE_POSITION) {
                m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos.clone());
            }
            if let Some(VertexAttributeValues::Float32x3(n)) = fresh.attribute(Mesh::ATTRIBUTE_NORMAL) {
                m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, n.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preset builds on a headless app, dresses a `Gore` mesh with its canvases and the flesh
    /// material, and takes a hit without panicking — the two-line contract, exercised.
    #[test]
    fn a_gore_entity_is_dressed_and_can_be_hit() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<FleshMaterial>();
        // The material plugin needs a render app; the preset must still build without one.
        app.init_resource::<FleshTables>()
            .init_resource::<GoreDials>()
            .init_resource::<GoreClock>()
            .add_message::<GoreHit>()
            .add_message::<GoreFractured>()
            .add_message::<BoneExposed>()
            .add_message::<Wounded>()
            .init_resource::<FractureCache>()
            .init_resource::<WetSettings>()
            .init_resource::<FlaySettings>()
            .add_systems(FixedUpdate, (advance_clock, dress, take_hits, throw_bodies, bleed, age_bruises, soak_cloth, tick_canvases, flush_dermis, fly).chain());
        // Fake the baked tables: any handle other than the default passes the gate.
        let images = app.world_mut().resource_mut::<Assets<Image>>().reserve_handle();
        app.world_mut().resource_mut::<FleshTables>().sss = images;
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(0.3).mesh().uv(16, 8));
        let material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let body = app
            .world_mut()
            .spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb)))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<Dressed>(body).is_some(), "dressed on the first tick");
        assert!(app.world().get::<FlayCanvas>(body).is_some() && app.world().get::<WetCanvas>(body).is_some());
        assert!(app.world().get::<MeshMaterial3d<FleshMaterial>>(body).is_some(), "wears the flesh material");
        app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, 0.03, 5.0));
        app.world_mut().run_schedule(FixedUpdate);
        let site = app.world().get::<WoundSite>(body).copied();
        assert!(site.is_some(), "a hit opens a wound");
        if let Some(site) = site {
            // The ray came down −Z onto a sphere at the origin: the wound sits on its +Z face and faces +Z.
            assert!(site.wound.at[2] > 0.25 && site.wound.at[2] < 0.35, "wound at {:?}", site.wound.at);
            assert!(site.wound.normal[2] > 0.9, "wound normal {:?}", site.wound.normal);
        }
        let wet = app.world().get::<WetCanvas>(body).map(|w| w.wetted_area()).unwrap_or(0.0);
        assert!(wet > 0.0, "the hit painted blood");
    }

    /// A blow bruises without bleeding, and the bruise warms the dermis under the blow within the
    /// first hours; a contact burn hot enough for a third-degree eschar blackens the dermis without
    /// peeling anything, and neither opens a wound.
    #[test]
    fn a_blow_bruises_and_a_burn_chars_without_bleeding() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<FleshMaterial>()
            .init_resource::<FleshTables>()
            .insert_resource(GoreDials { bruise_hours_per_tick: 1.0, ..default() })
            .init_resource::<GoreClock>()
            .add_message::<GoreHit>()
            .add_message::<GoreFractured>()
            .add_message::<BoneExposed>()
            .add_message::<Wounded>()
            .init_resource::<FractureCache>()
            .init_resource::<WetSettings>()
            .init_resource::<FlaySettings>()
            .add_systems(FixedUpdate, (advance_clock, dress, take_hits, throw_bodies, bleed, age_bruises, soak_cloth, tick_canvases, flush_dermis, fly).chain());
        let images = app.world_mut().resource_mut::<Assets<Image>>().reserve_handle();
        app.world_mut().resource_mut::<FleshTables>().sss = images;
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.4, 0.2).mesh().build());
        let material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let spawn = |app: &mut App| {
            app.world_mut()
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::IDENTITY,
                    GlobalTransform::IDENTITY,
                    Gore::skin(Region::Limb).with_uv_per_metre(2.5),
                ))
                .id()
        };
        let bruised = spawn(&mut app);
        let burnt = spawn(&mut app);
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().write_message(GoreHit::blunt(bruised, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, 0.02));
        app.world_mut().write_message(GoreHit::burn(burnt, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, 0.03, 150.0, 1.0));
        for _ in 0..8 {
            app.world_mut().run_schedule(FixedUpdate);
        }
        assert!(app.world().get::<WoundSite>(bruised).is_none() && app.world().get::<WoundSite>(burnt).is_none(), "neither bleeds");
        let hours = app.world().get::<Bruising>(bruised).map(|b| b.bruise.hours()).unwrap_or(0.0);
        assert!(hours >= 6.0, "the bruise aged {hours} h");
        let dermis = app.world().get::<DermisCanvas>(bruised).map(|d| d.px.clone()).unwrap_or_default();
        let (mut warmth, mut n) = (0.0f32, 0u32);
        for px in dermis.chunks_exact(4).filter(|p| p[3] > 0) {
            warmth += (px[0] as f32 - px[2] as f32) / 255.0;
            n += 1;
        }
        assert!(n > 0, "the bruise painted nothing");
        // Stam's model has no day-zero red (the pool reaches the visible dermis by transport), so the
        // first hours are a warming rather than a crimson: red over blue by a clear margin.
        assert!(warmth / n as f32 > 0.05, "a young bruise warms: mean R−B {}", warmth / n as f32);

        let peeled = app.world().get::<FlayCanvas>(burnt).map(|f| f.digest()).unwrap_or(0);
        let fresh = FlayCanvas::new(&mut app.world_mut().resource_mut::<Assets<Image>>(), 128, Region::Limb, Layers::for_region(Region::Limb), [0.5; 3], 0.5).digest();
        assert_eq!(peeled, fresh, "a burn removes nothing: the eschar stays on the surface");
        let dermis = app.world().get::<DermisCanvas>(burnt).map(|d| d.px.clone()).unwrap_or_default();
        let charred = dermis.chunks_exact(4).filter(|p| p[3] > 200 && p[0] < 40).count();
        assert!(charred > 0, "a third-degree burn chars the dermis");
    }

    /// What a wound throws lands on the floor under it: the closed-form stains reach a dressed floor
    /// three metres across at the preset's floor resolution.
    #[test]
    fn a_wound_stains_the_floor_beneath_it() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<FleshMaterial>()
            .init_resource::<FleshTables>()
            .init_resource::<GoreDials>()
            .init_resource::<GoreClock>()
            .add_message::<GoreHit>()
            .add_message::<GoreFractured>()
            .add_message::<BoneExposed>()
            .add_message::<Wounded>()
            .init_resource::<FractureCache>()
            .init_resource::<WetSettings>()
            .init_resource::<FlaySettings>()
            .add_systems(FixedUpdate, (advance_clock, dress, take_hits, throw_bodies, bleed, age_bruises, soak_cloth, tick_canvases, flush_dermis, fly).chain());
        let images = app.world_mut().resource_mut::<Assets<Image>>().reserve_handle();
        app.world_mut().resource_mut::<FleshTables>().sss = images;
        let sphere = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(0.3).mesh().uv(16, 8));
        let plane = app.world_mut().resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Y, Vec2::splat(1.5)).mesh().build());
        let material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let body = app
            .world_mut()
            .spawn((
                Mesh3d(sphere),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(0.0, 0.9, 0.0),
                GlobalTransform::from_xyz(0.0, 0.9, 0.0),
                Gore::skin(Region::Limb),
            ))
            .id();
        let floor = app
            .world_mut()
            .spawn((Mesh3d(plane), MeshMaterial3d(material), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::floor().with_uv_per_metre(1.0 / 3.0)))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 0.9, 2.0), Vec3::NEG_Z, 0.03, 5.0));
        app.world_mut().run_schedule(FixedUpdate);
        let floor_wet = app.world().get::<WetCanvas>(floor).map(|w| w.wetted_area()).unwrap_or(0.0);
        assert!(floor_wet > 0.0, "nothing landed on the floor");
    }
}
