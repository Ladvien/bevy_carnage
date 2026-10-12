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
//! 1. **Peels the skin** where it landed — `flaymap`, to the depth the hit asked for.
//! 2. **Throws blood** — the wound's `crate::bloodstain::stain::stains` in closed form, painted onto every
//!    [`Gore::floor`] they land on, and a stain at the wound on the body's own wetmap.
//! 3. **Opens a bleed** — `crate::bloodstain::bleed`, pulsing more blood onto the wetmap each systole until
//!    the clot, which the wetmap then runs under gravity and dries.
//! 4. **A slash gapes** — [`GoreHit::slash`] lays a `laceration` along the cut instead of a
//!    crater, and its bed is banded by `cross_section`.
//! 5. **Guts spill** — a torso hit that reached muscle spawns `viscera` strands tethered to the
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

use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use crate::cross_section::{CrossSectionPlugin, CrossSectionSettings, Layers, Region};
use crate::flaymap::{BoneExposed, FlayCanvas, FlaySettings, FlaymapPlugin, Layer};
use crate::laceration::{Gape, Laceration, LacerationClock, LacerationPlugin, Tension};
use crate::viscera::{Mesentery, Strand, ViscSettings, VisceraPlugin, VisceraSystems, spill, tube_mesh};
use crate::wetmap::{StainShape, WetCanvas, WetSettings, WetmapPlugin};
use crate::bloodstain::{BloodSettings, Wound as BloodWound, WoundKind, wound_seed};

use crate::flesh::{FleshMaterial, FleshMode, FleshParams, FleshPlugin, FleshTables};
use crate::skin::{self, Pose, Rig};
use crate::{CarnagePlugin, FractureCache, FractureSubject, Wounded};

/// **What a surface is, to the preset.**
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Gore {
    /// Which kind of surface, and so which canvases it gets.
    pub kind: GoreKind,
    /// **Which way blood runs on this mesh's atlas, in UV space.** `(0, 1)`, the default, is down on
    /// a glTF atlas whose islands stand upright, because glTF puts `v = 0` at the top of the image.
    ///
    /// **Bevy's own primitives do not all agree with that, nor with each other.** A `Cuboid`'s front
    /// (+Z) face grows `v` *up* the face, so down there is `(0, −1)`, while its back face grows `v`
    /// down. A `Sphere::mesh().uv(..)` puts its poles on the mesh's Z axis, so it wants turning
    /// pole-up before `(0, 1)` is down, and a `Plane3d` facing +Z grows `v` down the sheet. One
    /// direction per canvas cannot be right on every face of a mesh whose islands disagree; set the
    /// one for the faces that get hit, with [`Gore::with_gravity_uv`]. A wrong sign is easy to miss
    /// in a still and obvious in motion: the drips climb.
    pub gravity_uv: Vec2,
    /// **How many UV units one metre of this surface covers.** The canvases take radii in UV space,
    /// so a mesh whose atlas spans a 40 cm limb from 0 to 1 packs 2.5 UV per metre, and a 3 cm wound
    /// on it is `0.075` UV wide, not `0.03`.
    ///
    /// **`None`, the default, has the preset measure it** when it dresses the mesh. The measure is the
    /// square root of the mesh's total UV area over its total world area, the average over every
    /// triangle, at the entity's world scale. The result is stored here, so a game can read what was
    /// measured. State it with [`Gore::with_uv_per_metre`] when the average is wrong for where the
    /// wounds land, as on an atlas that packs the face far denser than the body. Before 0.5.18 the
    /// default was `1.0`, which on a typical character atlas shrank every wound to a dot.
    pub uv_per_metre: Option<f32>,
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
        Self { kind: GoreKind::Skin(region), gravity_uv: Vec2::new(0.0, 1.0), uv_per_metre: None }
    }
    /// A fabric surface.
    pub fn cloth() -> Self {
        Self { kind: GoreKind::Cloth, gravity_uv: Vec2::new(0.0, 1.0), uv_per_metre: None }
    }
    /// A floor. Blood pools rather than runs, so its UV gravity is zero.
    pub fn floor() -> Self {
        Self { kind: GoreKind::Floor, gravity_uv: Vec2::ZERO, uv_per_metre: None }
    }
    /// The same surface with its atlas density stated — see [`Gore::uv_per_metre`]. A value that is
    /// not a positive finite number leaves it to be measured.
    pub fn with_uv_per_metre(mut self, uv_per_metre: f32) -> Self {
        self.uv_per_metre = (uv_per_metre.is_finite() && uv_per_metre > 0.0).then_some(uv_per_metre);
        self
    }
    /// The atlas density the canvases use: as stated or measured, and `1.0` only on a surface not
    /// dressed yet or one whose mesh could not be measured.
    fn density(&self) -> f32 {
        self.uv_per_metre.unwrap_or(1.0)
    }
    /// The same surface with the way blood runs on its atlas stated — see [`Gore::gravity_uv`].
    pub fn with_gravity_uv(mut self, gravity_uv: Vec2) -> Self {
        self.gravity_uv = if gravity_uv.is_finite() { gravity_uv } else { Vec2::ZERO };
        self
    }
}

/// The wetmap settings with the preset's edge supersampling dialled in.
fn edge_settings(dials: &GoreDials, settings: Option<&WetSettings>) -> WetSettings {
    WetSettings { edge_samples: dials.edge_samples, ..settings.cloned().unwrap_or_default() }
}

/// **Where a world ray lands on a mesh**, mesh-local: the UV, the point and the triangle's winding
/// normal, and *which* triangle and where in it. The triangle is what lets a hit on a posed skin be
/// carried back to the bind pose it was cut from, and be found again once the body has moved.
#[derive(Clone, Copy, Debug)]
struct Landing {
    uv: Vec2,
    point: Vec3,
    normal: Vec3,
    /// The three vertex indices of the triangle that was hit.
    tri: [u32; 3],
    /// Barycentric weights of `tri`'s vertices at the hit: `point = Σ bary[i] · position[tri[i]]`.
    bary: Vec3,
}

impl Landing {
    /// **The world matrix at the hit**, for a skinned mesh: its three vertices' blended joint matrices,
    /// weighted by where in the triangle the ray landed. What a length on the bind pose is multiplied
    /// by on the way to where the body is. `None` when the mesh has no rig or a vertex cannot be posed.
    fn matrix(&self, bind: &Mesh, pose: &Pose) -> Option<Mat4> {
        let rig = Rig::of(bind)?;
        let mut sum = Mat4::ZERO;
        for (vertex, weight) in self.tri.into_iter().zip(self.bary.to_array()) {
            sum += rig.matrix(pose, vertex as usize)? * weight;
        }
        Some(sum)
    }
}

/// **A landing on a posed skin, carried back to the bind pose**: the point and the winding normal of
/// the same triangle in the asset, which is what a wound's bleeding and a laceration's path are laid
/// out on. `None` when a vertex is missing.
fn bind_landing(bind: &Mesh, landing: &Landing) -> Option<(Vec3, Vec3)> {
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = bind.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let [a, b, c] = landing.tri.map(|vertex| positions.get(vertex as usize).copied().map(Vec3::from));
    let (a, b, c) = (a?, b?, c?);
    let point = a * landing.bary.x + b * landing.bary.y + c * landing.bary.z;
    Some((point, (b - a).cross(c - a).normalize_or_zero()))
}

/// The UV a world ray lands on, by the same Möller–Trumbore walk the canvases use — the preset needs
/// it for the injuries that live in texture space (a bruise, a burn) rather than on a ray.
fn ray_hit(mesh: &Mesh, xf: &GlobalTransform, from: Vec3, dir: Vec3) -> Option<Landing> {
    let inv = xf.affine().inverse();
    let origin = inv.transform_point3(from);
    let d = inv.transform_vector3(dir);
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let Ok(Some(VertexAttributeValues::Float32x2(uvs))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) else {
        return None;
    };
    let indices = mesh.try_indices_option().ok()?;
    let tri = |i: usize| -> Option<[usize; 3]> {
        match indices {
            Some(Indices::U32(ix)) => Some([*ix.get(i)? as usize, *ix.get(i + 1)? as usize, *ix.get(i + 2)? as usize]),
            Some(Indices::U16(ix)) => Some([*ix.get(i)? as usize, *ix.get(i + 1)? as usize, *ix.get(i + 2)? as usize]),
            None => Some([i, i + 1, i + 2]),
        }
    };
    let count = indices.map(|ix| ix.len()).unwrap_or(positions.len());
    let mut best: Option<(f32, Landing)> = None;
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
        if t <= 0.0 || best.is_some_and(|(bt, _)| bt <= t) {
            continue;
        }
        let (Some(ua), Some(ub), Some(uc)) = (uvs.get(a), uvs.get(b), uvs.get(c)) else { continue };
        let uv = Vec2::from(*ua) * (1.0 - u - v) + Vec2::from(*ub) * u + Vec2::from(*uc) * v;
        best = Some((
            t,
            Landing {
                uv,
                point: origin + d * t,
                normal: e1.cross(e2).normalize_or_zero(),
                tri: [a as u32, b as u32, c as u32],
                bary: Vec3::new(1.0 - u - v, u, v),
            },
        ));
    }
    best.map(|(_, landing)| landing)
}

/// **A mesh's atlas density, UV units per metre**: the square root of its total UV area over its
/// total world area, at `xf` and `mm_per_unit` millimetres per world unit. That is the average
/// density over every triangle, so an atlas packed evenly measures exactly. `None` for a mesh with no
/// `Float32x2` `UV_0`, no `Float32x3` positions, or no area to divide by.
fn atlas_density(mesh: &Mesh, xf: &GlobalTransform, mm_per_unit: f32) -> Option<f32> {
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let Ok(Some(VertexAttributeValues::Float32x2(uvs))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) else {
        return None;
    };
    let affine = xf.affine();
    let (mut uv_area, mut world_area) = (0.0f64, 0.0f64);
    let mut triangle = |[a, b, c]: [usize; 3]| {
        let (Some(pa), Some(pb), Some(pc), Some(ua), Some(ub), Some(uc)) =
            (positions.get(a), positions.get(b), positions.get(c), uvs.get(a), uvs.get(b), uvs.get(c))
        else {
            return;
        };
        let [pa, pb, pc] = [pa, pb, pc].map(|p| affine.transform_point3(Vec3::from(*p)));
        world_area += 0.5 * f64::from((pb - pa).cross(pc - pa).length());
        let (ua, ub, uc) = (Vec2::from(*ua), Vec2::from(*ub), Vec2::from(*uc));
        uv_area += 0.5 * f64::from((ub - ua).perp_dot(uc - ua).abs());
    };
    match mesh.try_indices_option() {
        Ok(Some(Indices::U32(ix))) => ix.as_chunks::<3>().0.iter().for_each(|t| triangle(t.map(|i| i as usize))),
        Ok(Some(Indices::U16(ix))) => ix.as_chunks::<3>().0.iter().for_each(|t| triangle(t.map(usize::from))),
        Ok(None) => (0..positions.len() / 3).for_each(|t| triangle([3 * t, 3 * t + 1, 3 * t + 2])),
        Err(_) => return None,
    }
    let metres_per_unit = f64::from(mm_per_unit) / 1000.0;
    let world_m2 = world_area * metres_per_unit * metres_per_unit;
    let density = (uv_area / world_m2).sqrt();
    (world_m2 > 0.0 && uv_area > 0.0 && density.is_finite()).then_some(density as f32)
}

/// A stain silhouette in this surface's UV units, from one in metres — and never under a texel and
/// a half across. A 2 mm droplet on a floor at 12 mm per texel is real and invisible; the floor of
/// the stamp is a resolution fact, so a distant spatter reads as the dot it is rather than as nothing.
fn shape_on(gore: &Gore, shape: &StainShape, texels: u32) -> StainShape {
    let floor_uv = 1.5 / texels.max(1) as f32;
    StainShape {
        major: (shape.major * gore.density()).max(floor_uv),
        minor: (shape.minor * gore.density()).max(floor_uv),
        ..*shape
    }
}

/// **A skinned mesh's pose**, from the inverse bind poses asset and a way to ask for each joint's
/// world matrix. `None` until both exist — a scene still streaming in has joints no transform
/// reaches yet, and that is "not yet", never "unskinned".
fn pose_of(
    skin: &SkinnedMesh,
    bindposes: Option<&Assets<SkinnedMeshInverseBindposes>>,
    world: impl FnMut(Entity) -> Option<Mat4>,
) -> Option<Pose> {
    Pose::of(skin, bindposes?.get(&skin.inverse_bindposes)?, world)
}

/// What the hit systems read to pose a skinned mesh: the inverse bind poses and the joints' world
/// transforms as the last `PostUpdate` propagated them — the matrices the GPU is drawing this frame
/// with. `Option` for the arena, because a missing `Res<T>` panics its system in Bevy 0.19.
#[derive(bevy::ecs::system::SystemParam)]
struct Rigging<'w, 's> {
    bindposes: Option<Res<'w, Assets<SkinnedMeshInverseBindposes>>>,
    joints: Query<'w, 's, &'static GlobalTransform>,
}

impl Rigging<'_, '_> {
    /// The pose `skin` is drawn in now.
    fn pose(&self, skin: &SkinnedMesh) -> Option<Pose> {
        pose_of(skin, self.bindposes.as_deref(), |joint| self.joints.get(joint).ok().map(|g| g.to_matrix()))
    }
}

/// **What a ray is cast at for one mesh entity.**
enum Drawn {
    /// The asset, placed by the entity's own `GlobalTransform`.
    Rigid,
    /// A skinned mesh as its pose draws it: world-space positions, cast with the identity transform
    /// because the mesh entity's own transform plays no part in drawing a skin. The pose rides along
    /// for what is read off the bind pose afterwards.
    Posed(Box<Mesh>, Pose),
    /// Skinned, but its joints or inverse bind poses have not arrived, or a vertex names a joint
    /// that does not exist. Cast at nothing: the bind pose is not where the body is.
    Unposed,
}

impl Drawn {
    /// A mesh is skinned when its entity carries a [`SkinnedMesh`] *and* the mesh has joint indices
    /// and weights — the same two conditions the renderer uses to take the skinned path.
    fn of(mesh: &Mesh, skin: Option<&SkinnedMesh>, rigging: &Rigging) -> Self {
        match skin {
            Some(skin) if Rig::of(mesh).is_some() => match rigging.pose(skin) {
                Some(pose) => match skin::posed_mesh(mesh, &pose) {
                    Some(posed) => Self::Posed(Box::new(posed), pose),
                    None => Self::Unposed,
                },
                None => Self::Unposed,
            },
            _ => Self::Rigid,
        }
    }
}

/// **One dressed surface, as a wound's spray sees it**: what to cast at, and what places it.
struct Surface {
    entity: Entity,
    mesh: Handle<Mesh>,
    xf: GlobalTransform,
    gore: Gore,
    /// Present on a skinned surface, whose spray is cast at its pose.
    skin: Option<SkinnedMesh>,
}

/// What [`dress`] visits: every [`Gore`] entity that has not been dressed yet.
type Undressed<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static mut Gore, &'static Mesh3d, Option<&'static MeshMaterial3d<StandardMaterial>>, Option<&'static SkinnedMesh>),
    Without<Dressed>,
>;

/// The dressed bodies a hit can reach, with the canvases it paints.
type Bodies<'w, 's> = Query<
    'w,
    's,
    (
        &'static Gore,
        &'static Mesh3d,
        &'static GlobalTransform,
        Option<&'static SkinnedMesh>,
        Option<&'static mut FlayCanvas>,
        Option<&'static mut WetCanvas>,
    ),
    With<Dressed>,
>;

/// Every dressed surface, for what a wound throws onto the others.
type Surfaces<'w, 's> =
    Query<'w, 's, (Entity, &'static Gore, &'static Mesh3d, &'static GlobalTransform, Option<&'static SkinnedMesh>), With<Dressed>>;

/// The parts [`bleed`] pulses.
type Bleeders<'w, 's> = Query<
    'w,
    's,
    (
        &'static Gore,
        &'static mut Wounds,
        &'static GlobalTransform,
        &'static Mesh3d,
        Option<&'static SkinnedMesh>,
        &'static mut WetCanvas,
    ),
    Without<FloorTag>,
>;

/// The canvases [`tick_canvases`] runs.
type Canvases<'w, 's> = Query<
    'w,
    's,
    (&'static Gore, &'static mut WetCanvas, &'static Mesh3d, Option<&'static Wounds>, Option<&'static SkinnedMesh>, Option<&'static mut RunDirection>),
>;

/// Marks an entity the preset has already dressed, and keeps the material it wore before — the
/// skin a thrown fragment's outer mesh goes back to.
#[derive(Component, Clone, Debug, Default)]
pub struct Dressed {
    /// The `StandardMaterial` the entity had before the flesh material replaced it.
    pub original: Option<Handle<StandardMaterial>>,
}

/// **One wound the preset is bleeding**, found once, where the hit landed.
#[derive(Clone, Copy, Debug)]
pub struct WoundSite {
    /// Where the wound is on this entity's wetmap. Every pulse is painted here. The spot belongs to
    /// the skin, so a body that walks, turns or falls after the hit keeps bleeding from its wound.
    /// Until 0.5.16 each pulse re-cast the hit's world ray, which on a moved body landed elsewhere
    /// or nowhere.
    pub uv: Vec2,
    /// Where the wound is on the mesh asset, mesh-local — the bind pose, for a skinned mesh — so what
    /// runs off falls from where the wound is now. A rigid mesh places it with the entity's transform;
    /// a skinned one with [`WoundSite::skin`].
    pub at_local: Vec3,
    /// **The triangle of a skinned mesh the wound is on**, so each pulse can ask where that spot is
    /// under the pose the body has *now*. `None` on a mesh that is not skinned.
    pub skin: Option<SkinAnchor>,
    /// The wound as it opened, world space: the pulse schedule's seed and severity.
    pub wound: BloodWound,
    /// The schedule.
    pub bleed: crate::bloodstain::Bleed,
}

/// **A spot on a skinned mesh, kept as a triangle and a place in it** rather than as a point. A point
/// stays where it was cut while the body walks away from it; a triangle is carried by the joints that
/// move its three vertices, so [`SkinAnchor`] is the spot wherever the pose puts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinAnchor {
    /// The three vertex indices of the triangle.
    pub tri: [u32; 3],
    /// Barycentric weights of those vertices: the spot is `Σ bary[i] · posed vertex[tri[i]]`.
    pub bary: Vec3,
}

impl SkinAnchor {
    /// **The spot under `pose`, world space.** `None` when the mesh has no rig or a vertex of the
    /// triangle cannot be posed.
    pub fn posed(&self, mesh: &Mesh, pose: &Pose) -> Option<Vec3> {
        let mut at = Vec3::ZERO;
        for (vertex, weight) in self.tri.into_iter().zip(self.bary.to_array()) {
            at += skin::posed_vertex(mesh, pose, vertex as usize)? * weight;
        }
        Some(at)
    }
}

/// **Every wound this entity is bleeding**, oldest first. A hit away from the open wounds opens a
/// new one beside them; until 0.5.16 it replaced the last, and that wound stopped bleeding
/// mid-pulse. A hit inside an open wound reopens it: its bleed starts again from the spurt, rather
/// than a second schedule pulsing the same hole. A wound leaves the list once its perfusion envelope
/// reaches zero, after which it can never pulse again, and past [`GoreDials::open_wounds`] the
/// oldest stops bleeding.
#[derive(Component, Clone, Debug, Default)]
pub struct Wounds(pub Vec<WoundSite>);

impl Wounds {
    /// Open `site`: it replaces any open wound within `reach_uv` of it, and the oldest past `cap`
    /// stop.
    fn open(&mut self, site: WoundSite, reach_uv: f32, cap: usize) {
        self.0.retain(|s| s.uv.distance(site.uv) > reach_uv);
        self.0.push(site);
        let excess = self.0.len().saturating_sub(cap);
        self.0.drain(..excess);
    }
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
    /// The fracture modes over those bonds (`fracture_modes` through [`crate::modal`]), when
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
    pub fn from_fracture(fracture: crate::Fracture, region: Region, scale: &crate::cross_section::Scale) -> Self {
        let layers = Layers::for_region(region);
        let modes = crate::modal::bake_modes(
            &fracture.bonds,
            |id| fracture.solids().get(id.index()).map(|s| &s.cell),
            &crate::fracture_modes::ModeSettings::default(),
        )
        .ok();
        let bonds = fracture.bonds.clone();
        let mut leaves = fracture.into_leaves();
        for leaf in &mut leaves {
            leaf.annotate_cap(&layers, scale);
        }
        Self { leaves, bonds, modes, region, blow: 2.5 }
    }

    /// What a blow at `at_local` does to the leaves: which the modes part off and throw, and which
    /// belong to a body that lost its footing and comes down. Without modes, every leaf is thrown.
    ///
    /// The largest island stays standing **only while it still holds a leaf the body stood on** — one
    /// reaching the subject's own lowest extent, in its local frame. This crate knows no floor, so
    /// "what it stood on" is the only footing it can see. Keeping the largest island unconditionally
    /// left a torso and head hanging at standing height once a blow took the legs. The torso stayed
    /// motionless for the rest of the clip, its shadow on the floor well below it.
    fn thrown_by(&self, at_local: Vec3) -> Parted {
        let Some(modes) = &self.modes else {
            return Parted { thrown: self.leaves.iter().map(|l| l.id).collect(), fell: Vec::new() };
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
            return Parted::default();
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
        let lowest = |id: &crate::FragmentId| {
            self.leaves
                .iter()
                .find(|l| l.id == *id)
                .map_or(f32::INFINITY, |l| l.cell.points().iter().map(|p| p.y).fold(f32::INFINITY, f32::min))
        };
        let floor = self.leaves.iter().map(|l| lowest(&l.id)).fold(f32::INFINITY, f32::min);
        let stands = keep
            .and_then(|k| islands.get(k))
            .is_some_and(|island| island.iter().any(|id| lowest(id) <= floor + FOOTING));
        let mut parted = Parted::default();
        for (i, island) in islands.iter().enumerate() {
            match (Some(i) == keep, stands) {
                (true, true) => {}
                (true, false) => parted.fell.extend(island.iter().copied()),
                (false, _) => parted.thrown.extend(island.iter().copied()),
            }
        }
        parted
    }
}

/// How close to the subject's lowest extent a leaf must reach to be one it stands on. A leaf cut
/// from the bottom keeps that face exactly, so this is a lattice-sized tolerance, not a tuned one.
const FOOTING: f32 = 1.0e-3;

/// What one blow does to a breakable body's leaves.
#[derive(Default)]
struct Parted {
    /// Parted off by the modes, and thrown by the blow.
    thrown: Vec<crate::FragmentId>,
    /// The largest island, when nothing it stood on is left: not hit, so not thrown — it comes down.
    fell: Vec<crate::FragmentId>,
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

/// **A bruise ageing under this entity's skin**, at one spot. The kernel is `crate::bloodstain::bruise`; the
/// preset steps it on the fixed tick and repaints the dermis canvas when its colour moves.
#[derive(Component, Debug)]
pub struct Bruising {
    /// Where on the canvas.
    pub uv: Vec2,
    /// The kernel.
    pub bruise: crate::bloodstain::Bruise,
    /// Fractional model steps carried between ticks.
    pub carry: f32,
}

/// **Blood soaking into this cloth** from one landing: the Lucas–Washburn front (`crate::bloodstain::wick`)
/// widens the stain over the seconds after it lands.
#[derive(Component, Debug, Default)]
pub struct Soaking {
    /// `(UV it landed at, silhouette in metres, tick it landed)` per landing still spreading.
    pub landings: Vec<(Vec2, StainShape, u32)>,
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
    /// Supersamples along a stain's edge on the wetmap — `wetmap`'s `edge_samples`.
    pub edge_samples: u32,
    /// Canvas edge for a [`Gore::floor`], texels. Floors are metres across where a limb is
    /// centimetres, so they get the ceiling by default.
    pub floor_canvas: u32,
    /// **How fast a bruise ages**: hours of `crate::bloodstain::bruise` time per fixed tick. The model's
    /// own step is 0.1 h; at the shipped `0.02` a bruise runs its ten-day course in about four
    /// minutes of play, which is time-lapse and says so. `0` freezes every bruise.
    pub bruise_hours_per_tick: f32,
    /// **How long a burn's heat keeps working after contact**, seconds of `crate::bloodstain::burn` time
    /// integrated at hit time under still air — damage accrues after the heat is removed, and this
    /// is how much of that the hit accounts for at once.
    pub burn_cooling_s: f32,
    /// **How many wounds one surface bleeds from at once.** A hit past this stops the oldest
    /// bleeding; a wound also stops once its bleed has clotted. Each pulse costs one stamp on the
    /// surface and one drip on each floor, so this bounds what a burst of fire into one part costs.
    pub open_wounds: usize,
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
            open_wounds: 16,
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
    /// Something was drawn across: a `laceration` along the blade's travel that gapes onto a
    /// bed `depth_mm` deep.
    Slash { along: Vec3, depth_mm: f32 },
    /// Something hit without opening the skin: a `crate::bloodstain::bruise` that pools, spreads and turns
    /// from red through to yellow under intact skin, over the hours [`GoreDials::bruise_hours_per_tick`]
    /// sets.
    Blunt,
    /// Something hot touched the skin for `seconds` at `temp_c`: a `crate::bloodstain::burn` whose damage
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
        Self { entity, from, dir, radius_m, kind: HitKind::Impact { depth_mm }, so2: crate::bloodstain::spectral::SO2_VENOUS }
    }
    /// A slash `along` the blade's travel, `radius_m` half its length.
    pub fn slash(entity: Entity, from: Vec3, dir: Vec3, along: Vec3, radius_m: f32, depth_mm: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Slash { along, depth_mm }, so2: crate::bloodstain::spectral::SO2_VENOUS }
    }
    /// A blunt blow: a bruise `radius_m` across under intact skin.
    pub fn blunt(entity: Entity, from: Vec3, dir: Vec3, radius_m: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Blunt, so2: crate::bloodstain::spectral::SO2_VENOUS }
    }
    /// A contact burn `radius_m` across: `temp_c` for `seconds`.
    pub fn burn(entity: Entity, from: Vec3, dir: Vec3, radius_m: f32, temp_c: f32, seconds: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Burn { temp_c, seconds }, so2: crate::bloodstain::spectral::SO2_VENOUS }
    }
    /// The same hit, bleeding arterial blood.
    pub fn arterial(mut self) -> Self {
        self.so2 = crate::bloodstain::spectral::SO2_ARTERIAL;
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

/// **A body came apart under the preset**, with everything needed to build its pieces. Written on
/// the bone handoff whether or not the preset spawns them: a game with physics sets
/// [`GoreDials::spawn_fragments`] to `false`, gives each [`GorePiece`] a rigid body, and despawns or
/// hides the subject itself. A body breaks once.
#[derive(Message, Clone, Debug)]
pub struct GoreFractured {
    /// The subject root.
    pub subject: Entity,
    /// Where the blow landed, world.
    pub at: Vec3,
    /// Every piece the body broke into.
    pub pieces: Vec<GorePiece>,
    /// What a piece's skin side wears: the hit part's own material, as authored.
    pub skin: Handle<StandardMaterial>,
    /// What a piece's cut faces wear: the region's cross-section strip, banded through `UV_1`.
    pub cap: Handle<FleshMaterial>,
}

/// **One piece of a broken body**, where it is the moment the body broke and how the preset would
/// launch it.
#[derive(Clone, Debug)]
pub struct GorePiece {
    /// The skin side, if the piece has any. Wear [`GoreFractured::skin`].
    pub outer: Option<Handle<Mesh>>,
    /// The cut faces, if any. Wear [`GoreFractured::cap`].
    pub cap: Option<Handle<Mesh>>,
    /// Where both meshes are drawn, world.
    pub transform: Transform,
    /// Half the piece's extent along each of its local axes, for sizing a collider.
    pub half_extents: Vec3,
    /// The velocity the preset launches it at, world units per second. Zero for a piece that stood.
    pub velocity: Vec3,
    /// What became of it.
    pub fate: PieceFate,
}

/// **What became of a piece** when its body broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PieceFate {
    /// The blow parted it off and threw it.
    Thrown,
    /// Its body lost its footing: it slumps outward from the body's centre and falls.
    Fell,
    /// It kept its footing and stays where it stood.
    Stood,
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
///
/// **A skin keeps the skin it was authored with.** Its colour, its `base_color_texture`, its
/// occlusion and emissive maps and its own normal map are kept; only its metallic-roughness
/// texture is replaced, by the flaymap's (see [`FleshTables::material`]). Before 0.5.15 the skin's
/// base texture was replaced by the flaymap's flat colour, so a textured glTF character, whose
/// colour factor is usually white, was dressed white.
///
/// **A skin gets tangents if its mesh has none.** The flaymap's normal image is a tangent-space map,
/// and Bevy leaves a normal map out entirely on a mesh without `ATTRIBUTE_TANGENT` — so a skin dressed
/// without them would shade its craters flat, silently. They are generated once, on the mesh asset
/// itself: tangents are a function of the mesh alone, so every instance sharing it wants the same
/// ones. A mesh that cannot have them (no normals, no UV0, not indexed) warns once and is dressed
/// anyway, flat. **A skin whose mesh has not loaded yet is left undressed until it has** — still
/// streaming, not absent — rather than dressed without the tangents it will need.
///
/// **A surface whose [`Gore::uv_per_metre`] is `None` gets it measured here**, from its mesh at its
/// world scale, and waits for its mesh the same way. The scale comes from the `Transform` chain
/// rather than `GlobalTransform`: a scene spawned this frame has not been propagated yet, and a
/// glTF exported in centimetres carries its 0.01 in exactly that chain.
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
    meshes: Option<ResMut<Assets<Mesh>>>,
    mut undressed: Undressed,
    bindposes: Option<Res<Assets<SkinnedMeshInverseBindposes>>>,
    transforms: TransformHelper,
) {
    let (Some(mut images), Some(standard), Some(mut flesh)) = (images, standard, flesh) else {
        return;
    };
    if tables.sss == Handle::default() {
        // The tables are baked on `Startup`; a `FixedUpdate` before that would dress with nothing.
        return;
    }
    let mut meshes = meshes;
    let scale = settings.as_ref().map(|s| s.scale).unwrap_or_default();
    for (entity, mut gore, mesh3d, material, skin) in &mut undressed {
        if let (GoreKind::Skin(_), Some(meshes)) = (gore.kind, meshes.as_mut()) {
            match meshes.get(&mesh3d.0).map(|m| m.contains_attribute(Mesh::ATTRIBUTE_TANGENT)) {
                None => continue,
                Some(false) => {
                    if let Some(mut mesh) = meshes.get_mut(&mesh3d.0)
                        && let Err(e) = mesh.generate_tangents()
                    {
                        bevy::log::warn_once!(
                            "gore: a skin mesh cannot have tangents ({e}), so its wounds will shade flat; \
                             give it normals, a Float32x2 UV_0 and indices"
                        );
                    }
                }
                Some(true) => {}
            }
        }
        if gore.uv_per_metre.is_none() {
            let Some(mesh) = meshes.as_ref().and_then(|m| m.get(&mesh3d.0)) else { continue };
            // A skinned mesh is measured as its pose draws it: the scale of a rigged character lives
            // in its joints (a glTF in centimetres carries its 0.01 on the armature), and the mesh
            // entity's own transform plays no part in drawing it. Its joints are asked through the
            // `Transform` chain for the reason below, and a pose that cannot be had yet is waited for.
            let measured = match skin.filter(|_| Rig::of(mesh).is_some()) {
                Some(skin) => {
                    let Some(pose) = pose_of(skin, bindposes.as_deref(), |joint| {
                        transforms.compute_global_transform(joint).ok().map(|g| g.to_matrix())
                    }) else {
                        continue;
                    };
                    skin::posed_mesh(mesh, &pose).and_then(|posed| atlas_density(&posed, &GlobalTransform::IDENTITY, dials.mm_per_unit))
                }
                None => {
                    let xf = transforms.compute_global_transform(entity).unwrap_or_default();
                    atlas_density(mesh, &xf, dials.mm_per_unit)
                }
            };
            match measured {
                Some(density) => gore.uv_per_metre = Some(density),
                None => bevy::log::warn_once!(
                    "gore: a dressed mesh has no UV area or no world area to measure its atlas density by, \
                     so its wounds are sized at one UV per metre; state it with Gore::with_uv_per_metre"
                ),
            }
        }
        let base = material.and_then(|m| standard.get(&m.0).cloned()).unwrap_or_default();
        let base_srgb = base.base_color.to_srgba();
        let base_srgb = [base_srgb.red, base_srgb.green, base_srgb.blue];
        let rough = base.perceptual_roughness;
        let mut entity_commands = commands.entity(entity);
        entity_commands
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(Dressed { original: material.map(|m| m.0.clone()) });
        if skin.is_some() {
            // Which way is down on the atlas is a fact about the pose, once the part moves.
            entity_commands.insert(RunDirection(gore.gravity_uv));
        }
        match gore.kind {
            GoreKind::Skin(region) => {
                let layers = layers_for(region, settings.as_deref());
                // The canvas is filled with the material's colour factor; the flesh material swaps
                // that for the authored skin, texture included, wherever the skin is intact. The
                // tangents the crater's normal map needs were generated above if the mesh had none.
                let flay = FlayCanvas::new(&mut images, dials.canvas, region, layers, base_srgb, rough);
                let wet = WetCanvas::new(&mut images, dials.canvas, base_srgb, rough);
                let mut params = FleshParams::for_layers(&layers, FleshMode::Canvas, scale.mm_per_unit);
                params.wet.z = wet_settings.as_ref().map(|w| w.film_depth_mm).unwrap_or(2.0);
                let dermis = DermisCanvas::new(&mut images, dials.canvas);
                let mut material = tables.material(base, params, Some(wet.roughness()), Some(&flay));
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
    atlas: Option<Res<'w, crate::cross_section::CrossSectionAtlas>>,
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
    subjects: Query<'w, 's, (&'static FractureSubject, &'static GlobalTransform), Without<Broken>>,
    breakables: Query<'w, 's, &'static GlobalTransform, With<GoreBreakable>>,
    dressed: Query<'w, 's, &'static Dressed>,
}

/// **Apply every hit.** See the module docs for the order.
fn take_hits(
    mut commands: Commands,
    mut hits: MessageReader<GoreHit>,
    mut cx: HitContext,
    lookup: SubjectLookup,
    rigging: Rigging,
    mut bodies: Bodies,
    bodies_index: Surfaces,
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
    let surfaces: Vec<Surface> = bodies_index
        .iter()
        .map(|(entity, gore, mesh, xf, skin)| Surface { entity, mesh: mesh.0.clone(), xf: *xf, gore: *gore, skin: skin.cloned() })
        .collect();
    // A skinned surface is posed once per batch, the first time a droplet is cast at it: a pose is a
    // pass over every vertex, and a batch of shots at one body would otherwise pose it once per shot.
    let mut posed: std::collections::HashMap<Entity, Option<Mesh>> = std::collections::HashMap::new();
    let identity = GlobalTransform::IDENTITY;
    for hit in hits {
        let Ok((gore, mesh3d, xf, skin, flay, wet)) = bodies.get_mut(hit.entity) else { continue };
        let gore = *gore;
        let gore = &gore;
        let xf = *xf;
        let xf = &xf;
        let skin: Option<SkinnedMesh> = skin.cloned();
        let Some(bind) = meshes.get(&mesh3d.0).cloned() else { continue };
        // **Cast at the body as it is drawn.** A skinned mesh's asset is its bind pose, and the entity's
        // own transform plays no part in drawing it; the pose is what the player is aiming at. A rigid
        // mesh is the asset placed by its transform.
        let drawn = Drawn::of(&bind, skin.as_ref(), &rigging);
        let (mesh, xf): (&Mesh, &GlobalTransform) = match &drawn {
            Drawn::Rigid => (&bind, xf),
            Drawn::Posed(posed, _) => (posed.as_ref(), &identity),
            Drawn::Unposed => {
                bevy::log::warn_once!(
                    "gore: a hit on a skinned mesh was dropped because its joints or inverse bind poses are not \
                     there yet, or a vertex names a joint that does not exist; a hit is never cast at the bind pose"
                );
                continue;
            }
        };
        let radius_uv = (hit.radius_m * gore.density() / crate::flaymap::UV_SPAN_M).max(0.0);
        let region = match gore.kind {
            GoreKind::Skin(r) => r,
            _ => Region::Torso,
        };

        // 1. Peel. A slash opens geometry instead; a blow and a burn go through the dermis below.
        let mut handoff = None;
        let mut deepest = Layer::Skin;
        let mut burn_degree = crate::bloodstain::burn::Degree::None;
        if let HitKind::Burn { temp_c, seconds } = hit.kind {
            // The whole exposure, then the cooling the dial allows, integrated at once: what the
            // model says the contact did. A burn removes nothing — an eschar is dead tissue that
            // stays on the surface — so it never peels; the degree decides what the dermis shows.
            let mut burn = crate::bloodstain::Burn::new();
            burn.expose(temp_c, seconds);
            burn.expose(crate::bloodstain::burn::SURFACE_REST_C, dials.burn_cooling_s);
            burn_degree = burn.degree();
        }
        if let Some(mut flay) = flay
            && let HitKind::Impact { depth_mm: peel_depth } = hit.kind
        {
            handoff = flay.paint_world(mesh, xf, hit.from, hit.dir, radius_uv, peel_depth, tick);
            if let Some(s) = flay_settings.as_deref() {
                flay.shade(s);
            }
            if let Some(h) = handoff {
                deepest = h.deepest_layer;
            }
        }
        // Where the ray lands, for every kind of hit — a slash and a blow never peel, so they have no
        // handoff to read a point from.
        let landed = ray_hit(mesh, xf, hit.from, hit.dir);
        let site_uv = landed.map(|l| l.uv);

        // A blow: a bruise starts under the skin, pooled to the blow's radius.
        if let HitKind::Blunt = hit.kind
            && let Some(uv) = site_uv
        {
            let params = crate::bloodstain::bruise::Params {
                pool_diameter_mm: (hit.radius_m * 2.0 * 1000.0).clamp(2.0, 100.0),
                ..crate::bloodstain::bruise::Params::default()
            };
            commands.entity(hit.entity).try_insert(Bruising { uv, bruise: crate::bloodstain::Bruise::new(params), carry: 0.0 });
        }

        // A burn: the degree colours the dermis — reddened, blistered, charred. **These three colours
        // are this crate's own**; the degree that picks one and the depth that is peeled are the model's.
        if let HitKind::Burn { .. } = hit.kind
            && let Some(uv) = site_uv
            && let Ok(mut entity_ref) = commands.get_entity(hit.entity)
        {
            let (tint, alpha): ([f32; 3], f32) = match burn_degree {
                crate::bloodstain::burn::Degree::None => ([1.0; 3], 0.0),
                // Erythema: the dermis flushes, so red is kept and green and blue are taken.
                crate::bloodstain::burn::Degree::First => ([1.0, 0.45, 0.40], 0.7),
                // A blister: the epidermis lifts white over a pale bed.
                crate::bloodstain::burn::Degree::Second => ([1.0, 0.92, 0.80], 0.6),
                // Eschar: charred, near black.
                crate::bloodstain::burn::Degree::Third => ([0.10, 0.06, 0.04], 1.0),
            };
            let radius_uv_burn = radius_uv;
            let mm_per_uv = 1000.0 / gore.density().max(1.0e-6);
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
        let at_local = handoff.and_then(|h| h.at).or(landed.map(|l| l.point));
        let normal_local = handoff.and_then(|h| h.normal).or(landed.map(|l| l.normal));
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
        let mut painted = None;
        if let Some(mut wet) = wet {
            let texels = wet.size();
            painted = wet.paint_world_with(mesh, xf, hit.from, hit.dir, &shape_on(gore, &shape, texels), tick, &edges);
        }
        // What the wound throws lands on everything else: the closed-form stains on every floor, and
        // the spray's own droplets cast straight at every other dressed surface for what a floor's
        // plane cannot catch — a sheet, a wall, the next body part. Straight rather than ballistic
        // for those: a droplet at 8–40 m/s falls under a centimetre in the metre it travels.
        let droplets = crate::bloodstain::patterns::impact_spatter(&wound, &dials.blood);
        for surface in &surfaces {
            let Surface { entity: other, mesh: other_handle, xf: other_xf, gore: other_gore, skin: other_skin } = surface;
            if *other == hit.entity {
                continue;
            }
            let Ok((_, _, _, _, _, Some(mut other_wet))) = bodies.get_mut(*other) else { continue };
            let Some(other_asset) = meshes.get(other_handle) else { continue };
            // A skinned surface is cast as its pose draws it, posed once per batch.
            let (other_mesh, other_xf): (&Mesh, &GlobalTransform) = match other_skin.as_ref().filter(|_| Rig::of(other_asset).is_some()) {
                None => (other_asset, other_xf),
                Some(skin) => {
                    let entry = posed
                        .entry(*other)
                        .or_insert_with(|| rigging.pose(skin).and_then(|pose| skin::posed_mesh(other_asset, &pose)));
                    let Some(body) = entry.as_ref() else { continue };
                    (body, &identity)
                }
            };
            match other_gore.kind {
                GoreKind::Floor => {
                    let plane_y = other_xf.translation().y;
                    for stain in crate::bloodstain::stain::stains(&wound, &dials.blood, plane_y) {
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
                        let radius = crate::bloodstain::stain::stain_radius(d, d.speed, &dials.blood);
                        let shape = StainShape {
                            major: radius * 2.4,
                            minor: radius * 1.6,
                            spines: 2,
                            satellites: 1,
                            direction: [0.0, 1.0],
                            seed: seed.wrapping_add(i as u32),
                        };
                        let texels = other_wet.size();
                        if let Some(uv) =
                            other_wet.paint_world_with(other_mesh, other_xf, at_world, dir, &shape_on(other_gore, &shape, texels), tick, &edges)
                            && other_gore.kind == GoreKind::Cloth
                        {
                            landed.push((uv, shape, tick));
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
        // The wound bleeds from where it opened on the skin: beside the ones it already has, or in
        // place of one it landed inside. Silenced, as `try_insert` is: an earlier hit this tick may
        // have broken the body and despawned the part.
        // What the wound is on the mesh asset: for a skinned mesh the bind-pose point of the triangle
        // it landed on, and that triangle as an anchor to be posed again on every pulse.
        let (site_at, site_skin) = match (&drawn, &landed) {
            (Drawn::Posed(..), Some(l)) => match bind_landing(&bind, l) {
                Some((point, _)) => (Some(point), Some(SkinAnchor { tri: l.tri, bary: l.bary })),
                None => (None, None),
            },
            _ => (at_local, None),
        };
        if let (Some(uv), Some(at_local)) = (painted, site_at)
            && let Ok(mut e) = commands.get_entity(hit.entity)
        {
            let site = WoundSite { uv, at_local, skin: site_skin, wound, bleed: crate::bloodstain::Bleed::new(tick, &wound) };
            let (reach, cap) = (hit.radius_m * gore.density(), dials.open_wounds);
            e.queue_silenced(move |mut e: EntityWorldMut| match e.get_mut::<Wounds>() {
                Some(mut wounds) => wounds.open(site, reach, cap),
                None => {
                    let mut wounds = Wounds::default();
                    wounds.open(site, reach, cap);
                    e.insert(wounds);
                }
            });
        }
        wounded.write(Wounded {
            at: at_world,
            normal: normal_world,
            area: wound.area,
            severity: 1.0,
            kind: wound.kind,
            class: if hit.so2 >= crate::bloodstain::spectral::SO2_ARTERIAL {
                crate::bloodstain::PatternClass::ArterialSpurt
            } else {
                crate::bloodstain::PatternClass::Impact
            },
        });

        // 4. A slash gapes instead of cratering.
        if let HitKind::Slash { along, depth_mm } = hit.kind {
            // The cut is laid out on the mesh asset. For a skinned body that is the bind pose, so the
            // point, the face normal, the direction of travel and the length are carried back to it
            // through the pose at the triangle that was hit: the same triangle, the same stretch.
            let layout = match (&drawn, &landed) {
                (Drawn::Posed(_, pose), Some(l)) => bind_landing(&bind, l).and_then(|(p, n)| {
                    let m = l.matrix(&bind, pose)?;
                    let along_local = m.inverse().transform_vector3(along).normalize_or_zero();
                    Some((p, n, along_local, hit.radius_m / skin::max_scale(&m).max(1.0e-6)))
                }),
                _ => at_local.map(|p| {
                    let inv = xf.affine().inverse();
                    let along_local = inv.transform_vector3(along).normalize_or_zero();
                    (p, normal_local.unwrap_or(Vec3::Y), along_local, hit.radius_m / xf.scale().max_element().max(1.0e-6))
                }),
            };
            if let Some((p, n_local, along_local, half)) = layout {
                let source = meshes.add(bind.clone());
                // **The wound is torn into a mesh this entity owns.** The laceration rewrites the
                // entity's drawn mesh in place, and a loaded scene shares one mesh handle between every
                // instance of a character: torn through the shared handle, one slash opened the same
                // gape on every enemy wearing that mesh. The copy keeps whatever the mesh had, tangents
                // and joint weights included.
                let torn = meshes.add(bind.clone());
                commands.entity(hit.entity).try_insert(Mesh3d(torn));
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
            // **The pieces wear the material of the part that was hit**, as it was before the
            // preset dressed it. That part is under the root by construction, at whatever depth: a
            // glTF scene puts its meshes under node entities, so searching the root's own children
            // found nothing there and threw the pieces in Bevy's default white.
            let skin = dressed.get(hit.entity).ok().and_then(|d| d.original.clone());
            let skin = skin.unwrap_or_else(|| standard.add(StandardMaterial::default()));
            if let Some((source, root_xf)) = subject
                && cache.is_baked(source.0.id())
            {
                let region = cache.region(source.0.id()).unwrap_or(region);
                let leaves: Vec<Piece> = cache
                    .frontier_of(source.0.id(), dials.pieces)
                    .into_iter()
                    .map(|f| Piece {
                        outer: f.outer_mesh.clone(),
                        cap: f.cap_mesh.clone(),
                        center_local: f.center_local,
                        half_extents: f.half_extents,
                        fate: PieceFate::Thrown,
                    })
                    .collect();
                let cap = cap_material(region, atlas.as_deref(), standard, tables, flesh, dials);
                let pieces = place(leaves, root_xf, at_world);
                if dials.spawn_fragments {
                    spawn_pieces(&mut commands, &pieces, &skin, &cap);
                    commands.entity(root).despawn();
                } else {
                    commands.entity(root).try_insert(Broken);
                }
                fractured.write(GoreFractured { subject: root, at: at_world, pieces, skin, cap });
            } else if let Some(root_xf) = breakable {
                // The message goes out from `throw_bodies`, next in the chain this tick, once the
                // leaves are meshes.
                let cap = cap_material(region, atlas.as_deref(), standard, tables, flesh, dials);
                let at_local = root_xf.affine().inverse().transform_point3(at_world);
                let spawn = dials.spawn_fragments;
                commands.entity(root).queue_silenced(move |mut e: EntityWorldMut| {
                    if let Some(body) = e.take::<GoreBreakable>() {
                        e.insert(ThrowNow { body, at: at_world, at_local, skin, cap, spawn });
                    }
                });
            }
        }
    }
}

/// One piece of a body, in its local frame, before it is placed.
struct Piece {
    outer: Option<Handle<Mesh>>,
    cap: Option<Handle<Mesh>>,
    center_local: Vec3,
    half_extents: Vec3,
    fate: PieceFate,
}

/// Marks a baked subject the preset broke without spawning its pieces, so it breaks once.
#[derive(Component)]
struct Broken;

/// A runtime body whose pieces are made on the next tick — the meshes need `Assets<Mesh>`, which
/// the hit loop does not hold.
#[derive(Component)]
struct ThrowNow {
    body: GoreBreakable,
    at: Vec3,
    at_local: Vec3,
    skin: Handle<StandardMaterial>,
    cap: Handle<FleshMaterial>,
    /// Whether the preset spawns the pieces ([`GoreDials::spawn_fragments`]).
    spawn: bool,
}

/// The cap material for a region: the cross-section strip the atlas baked, worn by the flesh
/// material in [`FleshMode::Cap`] so `UV_1` picks the tissue. Without an atlas, a plain red.
fn cap_material(
    region: Region,
    atlas: Option<&crate::cross_section::CrossSectionAtlas>,
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

/// **Place a broken body's pieces in the world**, each with the velocity the preset launches it at.
///
/// A thrown piece flies away from the blow and upward, slower the bigger it is. A piece whose body
/// lost its footing was not hit, so nothing throws it: it slumps outward from the fallen pieces'
/// centre, slowly and horizontally, so the heap spreads instead of stacking in one column, and
/// gravity in [`fly`] brings it down. A piece that stood has no velocity.
fn place(leaves: Vec<Piece>, root_xf: &GlobalTransform, at: Vec3) -> Vec<GorePiece> {
    let centre_of = |p: &Piece| root_xf.transform_point(p.center_local);
    let fell: Vec<Vec3> = leaves.iter().filter(|p| p.fate == PieceFate::Fell).map(centre_of).collect();
    let middle = if fell.is_empty() { Vec3::ZERO } else { fell.iter().sum::<Vec3>() / fell.len() as f32 };
    leaves
        .into_iter()
        .map(|p| {
            let centre = centre_of(&p);
            let size = p.half_extents.length();
            let velocity = match p.fate {
                PieceFate::Thrown => ((centre - at).normalize_or_zero() + Vec3::new(0.0, 0.6, 0.0)) * (1.5 / (1.0 + size)),
                PieceFate::Fell => Vec3::new(centre.x - middle.x, 0.0, centre.z - middle.z).normalize_or_zero() * (0.5 / (1.0 + size)),
                PieceFate::Stood => Vec3::ZERO,
            };
            GorePiece {
                outer: p.outer,
                cap: p.cap,
                transform: Transform::from_translation(centre).with_rotation(root_xf.rotation()).with_scale(root_xf.scale()),
                half_extents: p.half_extents,
                velocity,
                fate: p.fate,
            }
        })
        .collect()
}

/// Spawn placed pieces: skin outside, a banded cap inside. The ones that stood stay [`Standing`];
/// the rest are [`Flying`] at their velocity.
fn spawn_pieces(commands: &mut Commands, pieces: &[GorePiece], skin: &Handle<StandardMaterial>, cap: &Handle<FleshMaterial>) {
    for fate in [PieceFate::Stood, PieceFate::Thrown, PieceFate::Fell] {
        for piece in pieces.iter().filter(|p| p.fate == fate) {
            let meshes = [(piece.outer.clone(), None), (piece.cap.clone(), Some(cap.clone()))];
            for (mesh, cap) in meshes {
                let Some(mesh) = mesh else { continue };
                let mut e = match cap {
                    Some(cap) => commands.spawn((Mesh3d(mesh), MeshMaterial3d(cap), piece.transform)),
                    None => commands.spawn((Mesh3d(mesh), MeshMaterial3d(skin.clone()), piece.transform)),
                };
                if fate == PieceFate::Stood {
                    e.insert(Standing);
                } else {
                    e.insert(Flying(piece.velocity));
                }
            }
        }
    }
}

/// **Break a runtime body**: upload its leaves' meshes, part them by the modes, write
/// [`GoreFractured`], and — when the preset owns the pieces — spawn them and despawn the body.
fn throw_bodies(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut fractured: MessageWriter<GoreFractured>,
    mut bodies: Query<(Entity, &GlobalTransform, &mut ThrowNow)>,
) {
    for (entity, xf, mut throw) in &mut bodies {
        let parted = throw.body.thrown_by(throw.at_local);
        let leaves: Vec<Piece> = throw
            .body
            .leaves
            .drain(..)
            .map(|leaf| Piece {
                fate: if parted.thrown.contains(&leaf.id) {
                    PieceFate::Thrown
                } else if parted.fell.contains(&leaf.id) {
                    PieceFate::Fell
                } else {
                    PieceFate::Stood
                },
                outer: leaf.outer.map(|m| meshes.add(m)),
                cap: leaf.cap.map(|m| meshes.add(m)),
                center_local: leaf.center_local,
                half_extents: leaf.half_extents,
            })
            .collect();
        let pieces = place(leaves, xf, throw.at);
        if throw.spawn {
            spawn_pieces(&mut commands, &pieces, &throw.skin, &throw.cap);
            commands.entity(entity).despawn();
        } else {
            commands.entity(entity).remove::<ThrowNow>();
        }
        fractured.write(GoreFractured { subject: entity, at: throw.at, pieces, skin: throw.skin.clone(), cap: throw.cap.clone() });
    }
}

/// **Pulse every open wound onto its wetmap** until the clot.
fn bleed(
    clock: Res<GoreClock>,
    dials: Res<GoreDials>,
    wet_settings: Option<Res<WetSettings>>,
    meshes: Res<Assets<Mesh>>,
    rigging: Rigging,
    mut sites: Bleeders,
    mut floors: Query<(&Gore, &Mesh3d, &GlobalTransform, &mut WetCanvas), With<FloorTag>>,
) {
    let tick = clock.0;
    let edges = edge_settings(&dials, wet_settings.as_deref());
    for (gore, mut wounds, xf, mesh3d, skin, mut wet) in &mut sites {
        // The pose a skinned body is drawn in now, read once for every wound it is bleeding and only
        // if one of them needs it.
        let mut pose: Option<Option<Pose>> = None;
        // A wound whose perfusion envelope has reached zero can never pulse again: the driving
        // stress is zero from then on, and no yield stress is below it.
        let spent = |s: &WoundSite| crate::bloodstain::bleed::envelope(s.bleed.age(tick), &dials.blood) <= 0.0;
        if wounds.0.iter().any(spent) {
            wounds.0.retain(|s| !spent(s));
        }
        for site in &wounds.0 {
            let Some(pulse) = crate::bloodstain::bleed::pulse_wound(&site.bleed, &site.wound, tick, dials.hz, &dials.blood) else {
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
            wet.paint_uv_with(site.uv, &shape_on(gore, &shape, texels), tick, &edges);
            // What runs off falls straight down from where the wound is now: one drip per systole,
            // sized the way `crate::bloodstain::patterns::drip_trail` sizes a drip, onto every floor —
            // so a pool grows under a wound that keeps bleeding, and the wetmap's own spread does the
            // pooling.
            let drop = crate::bloodstain::Droplet {
                dir: [0.0, -1.0, 0.0],
                speed: crate::bloodstain::patterns::DRIP_FALL_SPEED,
                diameter: crate::bloodstain::patterns::DRIP_DIAMETER_M,
            };
            let radius = crate::bloodstain::stain::stain_radius(&drop, crate::bloodstain::patterns::DRIP_FALL_SPEED, &dials.blood);
            let wobble = |k: u32| (crate::bloodstain::hash_f32(site.bleed.seed ^ tick.wrapping_mul(0x9E37_79B9) ^ k) - 0.5) * 0.04;
            // Where the wound is now. A rigid mesh carries it with the entity; a skinned one with its
            // joints, so the anchor is posed again — and if the pose cannot be had this tick the drip
            // waits rather than falling from where the wound used to be.
            let now = match (site.skin, skin) {
                (Some(anchor), Some(skin)) => {
                    let pose = pose.get_or_insert_with(|| rigging.pose(skin));
                    match (pose.as_ref(), meshes.get(&mesh3d.0)) {
                        (Some(pose), Some(mesh)) => match anchor.posed(mesh, pose) {
                            Some(world) => world,
                            None => continue,
                        },
                        _ => continue,
                    }
                }
                _ => xf.transform_point(site.at_local),
            };
            let at = Vec3::new(now.x + wobble(1), now.y, now.z + wobble(2));
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
}

/// **Age every bruise** by the dial's hours per tick and repaint its dermis when a model step lands.
fn age_bruises(
    dials: Res<GoreDials>,
    mut bruised: Query<(&Gore, &mut Bruising, &mut DermisCanvas)>,
) {
    let per_tick = dials.bruise_hours_per_tick.max(0.0) / crate::bloodstain::bruise::STEP_H;
    for (gore, mut bruising, mut dermis) in &mut bruised {
        bruising.carry += per_tick;
        let steps = bruising.carry.floor() as u32;
        if steps == 0 {
            continue;
        }
        bruising.carry -= steps as f32;
        bruising.bruise.advance(steps);
        let mm_per_uv = 1000.0 / gore.density().max(1.0e-6);
        let radius_uv = crate::bloodstain::bruise::RADIUS_MM / mm_per_uv;
        let uv = bruising.uv;
        let bruise = &bruising.bruise;
        // The ratio of the bruised skin to the same skin with nothing in it, in linear light, so the
        // caller's skin tone is what the chromophores modulate. Far out, where the model holds no
        // chromophore, the ratio is one and the alpha is zero.
        let blank = linear(bruise.srgb_through_at(crate::bloodstain::bruise::RADIUS_MM));
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

/// **Widen every landing on cloth** along the Lucas–Washburn front until it stops moving, at the UV
/// it landed on, so a sheet that moves keeps soaking where the blood is.
fn soak_cloth(
    clock: Res<GoreClock>,
    dials: Res<GoreDials>,
    wet_settings: Option<Res<WetSettings>>,
    mut cloth: Query<(&Gore, &mut Soaking, &mut WetCanvas)>,
) {
    let tick = clock.0;
    let sheet = crate::bloodstain::wick::Sheet::default();
    let edges = edge_settings(&dials, wet_settings.as_deref());
    for (gore, mut soaking, mut wet) in &mut cloth {
        if soaking.landings.is_empty() {
            continue;
        }
        let hz = dials.hz.max(1) as f32;
        let mut keep = Vec::with_capacity(soaking.landings.len());
        for (uv, shape, landed) in soaking.landings.drain(..) {
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
                wet.paint_uv_with(uv, &shape_on(gore, &grown, texels), tick, &edges);
            }
            if t_s < SOAK_SECONDS {
                keep.push((uv, shape, landed));
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

/// **Which way blood runs on a skinned part, in UV space, right now.** Starts as
/// [`Gore::gravity_uv`] and is rewritten each tick while the part has an open wound: on a body that
/// moves, which way is down on the atlas is a fact about the pose, and a forearm held above the head
/// has blood running along it toward the elbow, not toward the hand, however its atlas is laid out.
/// Present on every dressed skinned mesh; a rigid mesh keeps the one direction it was given.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct RunDirection(pub Vec2);

/// The shallowest slope blood runs on: a surface within about seven degrees of level holds it, and
/// it pools instead.
const RUN_SLOPE: f32 = 0.12;

/// **Downhill in UV space at one triangle of a posed skin**: the UV direction whose image on the
/// surface points straight down in the world. `Some(ZERO)` where the surface is too level to run on,
/// `None` when the triangle has no UV area or a vertex cannot be posed.
fn downhill(mesh: &Mesh, pose: &Pose, tri: [u32; 3]) -> Option<Vec2> {
    let Ok(Some(VertexAttributeValues::Float32x2(uvs))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) else {
        return None;
    };
    let [p0, p1, p2] = tri.map(|vertex| skin::posed_vertex(mesh, pose, vertex as usize));
    let (p0, p1, p2) = (p0?, p1?, p2?);
    let [u0, u1, u2] = tri.map(|vertex| uvs.get(vertex as usize).copied().map(Vec2::from));
    let (u0, u1, u2) = (u0?, u1?, u2?);
    let (e1, e2, a, b) = (p1 - p0, p2 - p0, u1 - u0, u2 - u0);
    let det = a.x * b.y - a.y * b.x;
    if det.abs() < 1.0e-12 {
        return None;
    }
    // Where one UV unit along each axis goes on the surface, from `P(u, v) = p0 + e1 s + e2 t` with
    // `(s, t)` the solution of `[a b](s, t) = (u, v)`.
    let dp_du = (e1 * b.y - e2 * a.y) / det;
    let dp_dv = (e2 * a.x - e1 * b.x) / det;
    let normal = e1.cross(e2).normalize_or_zero();
    let down = Vec3::NEG_Y - normal * normal.dot(Vec3::NEG_Y);
    if down.length() < RUN_SLOPE {
        return Some(Vec2::ZERO);
    }
    // The least-squares UV direction whose image is `down`, on the surface's own two-axis basis.
    let (g00, g01, g11) = (dp_du.dot(dp_du), dp_du.dot(dp_dv), dp_dv.dot(dp_dv));
    let (r0, r1) = (dp_du.dot(down), dp_dv.dot(down));
    let gram = g00 * g11 - g01 * g01;
    if gram.abs() < 1.0e-12 {
        return None;
    }
    Some(Vec2::new((g11 * r0 - g01 * r1) / gram, (g00 * r1 - g01 * r0) / gram).normalize_or_zero())
}

/// **Downhill, averaged over the triangles a skinned part's open wounds are on.** `None` when none
/// of them can say, so the direction already in force stays.
fn runoff(mesh: &Mesh, pose: &Pose, wounds: &[WoundSite]) -> Option<Vec2> {
    let (mut sum, mut counted) = (Vec2::ZERO, 0u32);
    for anchor in wounds.iter().filter_map(|w| w.skin) {
        if let Some(d) = downhill(mesh, pose, anchor.tri) {
            sum += d;
            counted += 1;
        }
    }
    (counted > 0).then(|| sum.normalize_or_zero())
}

/// Run and dry every wetmap on the fixed tick.
fn tick_canvases(
    clock: Res<GoreClock>,
    settings: Option<Res<WetSettings>>,
    meshes: Option<Res<Assets<Mesh>>>,
    rigging: Rigging,
    mut canvases: Canvases,
) {
    let Some(settings) = settings else { return };
    for (gore, mut wet, mesh3d, wounds, skin, run) in &mut canvases {
        let gravity = match run {
            Some(mut run) => {
                if let (Some(skin), Some(wounds), Some(mesh)) = (skin, wounds, meshes.as_deref().and_then(|m| m.get(&mesh3d.0)))
                    && !wounds.0.is_empty()
                    && let Some(pose) = rigging.pose(skin)
                    && let Some(downhill) = runoff(mesh, &pose, &wounds.0)
                {
                    run.0 = downhill;
                }
                run.0
            }
            None => gore.gravity_uv,
        };
        wet.tick(clock.0, gravity, &settings);
    }
}

/// Thrown pieces fall to the floor and stop. **A piece at rest on the floor is not touched**: it
/// would land exactly where it lies every tick, and writing its `Transform` anyway made Bevy
/// re-propagate every settled piece of every broken body, every tick, for the rest of the game.
fn fly(dials: Res<GoreDials>, time: Res<Time<Fixed>>, mut pieces: Query<(&mut Flying, &mut Transform)>) {
    let dt = time.delta_secs();
    for (mut v, mut tf) in &mut pieces {
        if v.0 == Vec3::ZERO && tf.translation.y <= dials.floor_y {
            continue;
        }
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

    /// A headless app running the preset's chain, with the baked tables faked: any handle other than
    /// the default passes the gate. The material plugin needs a render app; the preset must still
    /// build without one.
    fn headless() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<FleshMaterial>()
            .init_asset::<SkinnedMeshInverseBindposes>();
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
        let images = app.world_mut().resource_mut::<Assets<Image>>().reserve_handle();
        app.world_mut().resource_mut::<FleshTables>().sss = images;
        app
    }

    /// The preset builds on a headless app, dresses a `Gore` mesh with its canvases and the flesh
    /// material, and takes a hit without panicking — the two-line contract, exercised.
    #[test]
    fn a_gore_entity_is_dressed_and_can_be_hit() {
        let mut app = headless();
        let sphere = Sphere::new(0.3).mesh().uv(16, 8);
        assert!(!sphere.contains_attribute(Mesh::ATTRIBUTE_TANGENT), "the premise: Bevy's sphere ships without tangents");
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(sphere);
        let material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let body = app
            .world_mut()
            .spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb)))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<Dressed>(body).is_some(), "dressed on the first tick");
        assert!(app.world().get::<FlayCanvas>(body).is_some() && app.world().get::<WetCanvas>(body).is_some());
        assert!(app.world().get::<MeshMaterial3d<FleshMaterial>>(body).is_some(), "wears the flesh material");
        let tangents = app.world().resource::<Assets<Mesh>>().get(&mesh).is_some_and(|m| m.contains_attribute(Mesh::ATTRIBUTE_TANGENT));
        assert!(tangents, "a dressed skin's mesh has the tangents its crater normal map needs");
        app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, 0.03, 5.0));
        app.world_mut().run_schedule(FixedUpdate);
        let site = app.world().get::<Wounds>(body).and_then(|w| w.0.first().copied());
        assert!(site.is_some(), "a hit opens a wound");
        if let Some(site) = site {
            // The ray came down −Z onto a sphere at the origin: the wound sits on its +Z face and faces +Z.
            assert!(site.wound.at[2] > 0.25 && site.wound.at[2] < 0.35, "wound at {:?}", site.wound.at);
            assert!(site.wound.normal[2] > 0.9, "wound normal {:?}", site.wound.normal);
        }
        let wet = app.world().get::<WetCanvas>(body).map(|w| w.wetted_area()).unwrap_or(0.0);
        assert!(wet > 0.0, "the hit painted blood");
    }

    /// **A second hit on a part opens a second wound; it does not close the first.** Both stay open
    /// and each sits where its own ray landed. A third hit inside the first wound reopens that one
    /// instead of stacking a second schedule on the same hole.
    #[test]
    fn a_second_wound_does_not_close_the_first() {
        let mut app = headless();
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.6, 0.6, 0.2).mesh().build());
        let body = app.world_mut().spawn((Mesh3d(mesh), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Torso))).id();
        app.world_mut().run_schedule(FixedUpdate);
        let (left, right) = (Vec3::new(-0.15, 0.1, 2.0), Vec3::new(0.15, -0.1, 2.0));
        for from in [left, right] {
            app.world_mut().write_message(GoreHit::impact(body, from, Vec3::NEG_Z, 0.02, 5.0));
            app.world_mut().run_schedule(FixedUpdate);
        }
        let open: Vec<WoundSite> = app.world().get::<Wounds>(body).map(|w| w.0.clone()).unwrap_or_default();
        assert_eq!(open.len(), 2, "the second hit closed the first wound");
        assert!(open[0].at_local.x < -0.1 && open[1].at_local.x > 0.1, "each wound sits where its own ray landed: {:?}", open.iter().map(|s| s.at_local).collect::<Vec<_>>());
        assert!(open[0].uv.distance(open[1].uv) > 0.1, "the two wounds share a spot on the canvas");
        app.world_mut().write_message(GoreHit::impact(body, left + Vec3::new(0.005, 0.0, 0.0), Vec3::NEG_Z, 0.02, 5.0));
        app.world_mut().run_schedule(FixedUpdate);
        let open: Vec<WoundSite> = app.world().get::<Wounds>(body).map(|w| w.0.clone()).unwrap_or_default();
        assert_eq!(open.len(), 2, "a hit inside an open wound stacked a second schedule on it");
        assert!(open[1].at_local.x < -0.1 && open[1].bleed.opened_at > open[0].bleed.opened_at, "the left wound was not the one reopened");
    }

    /// **An atlas density nobody stated is measured from the mesh, at its world scale.** A 3 m
    /// floor whose UVs run 0 to 1 packs a third of a UV per metre, and under a parent scaled by 2 it
    /// packs a sixth. This test never propagates a `GlobalTransform`, as a scene spawned this frame
    /// has none yet, so the scale has to come from the `Transform` chain. A stated density is kept.
    #[test]
    fn an_unstated_atlas_density_is_measured_at_world_scale() {
        let mut app = headless();
        let plane = app.world_mut().resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Y, Vec2::splat(1.5)).mesh().build());
        let world = app.world_mut();
        let alone = world.spawn((Mesh3d(plane.clone()), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::floor())).id();
        let parent = world.spawn((Transform::from_scale(Vec3::splat(2.0)), GlobalTransform::IDENTITY)).id();
        let scaled = world.spawn((Mesh3d(plane.clone()), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::floor(), ChildOf(parent))).id();
        let stated = world.spawn((Mesh3d(plane), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::floor().with_uv_per_metre(0.5))).id();
        app.world_mut().run_schedule(FixedUpdate);
        let density = |e: Entity| app.world().get::<Gore>(e).and_then(|g| g.uv_per_metre);
        let near = |got: Option<f32>, want: f32| got.is_some_and(|g| (g - want).abs() < 1.0e-4);
        assert!(near(density(alone), 1.0 / 3.0), "a 3 m floor measured {:?}", density(alone));
        assert!(near(density(scaled), 1.0 / 6.0), "the floor under a parent scaled by 2 measured {:?}", density(scaled));
        assert_eq!(density(stated), Some(0.5), "a stated density was overwritten");
    }

    /// **A body that moves after the hit bleeds exactly as one that stood still.** The wound is a
    /// spot on the skin, so where the body went does not change what its wetmap receives. Before
    /// 0.5.16 every pulse re-cast the hit's world ray, which here misses the moved body entirely.
    #[test]
    fn a_body_that_moves_keeps_bleeding_from_its_wound() {
        let run = |moved: bool| {
            let mut app = headless();
            let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.6, 0.6, 0.2).mesh().build());
            let body = app.world_mut().spawn((Mesh3d(mesh), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Torso))).id();
            app.world_mut().run_schedule(FixedUpdate);
            app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, 0.03, 5.0));
            app.world_mut().run_schedule(FixedUpdate);
            if moved {
                let away = GlobalTransform::from(Transform::from_xyz(5.0, 0.0, 0.0).with_rotation(Quat::from_rotation_y(1.2)));
                app.world_mut().entity_mut(body).insert(away);
            }
            for _ in 0..240 {
                app.world_mut().run_schedule(FixedUpdate);
            }
            app.world().get::<WetCanvas>(body).map(|w| w.digest())
        };
        let still = run(false);
        assert!(still.is_some(), "the body has no wetmap");
        assert_eq!(run(true), still, "the moved body's wound bled differently from the one that stood still");
    }

    /// **A tube along +X from 0 to 2**, five rings of eight sides, skinned to a joint at its root and
    /// one at x = 1: the rings either side of the elbow follow one joint each and the ring on it
    /// follows both. Its UVs cover the unit square once, so it can wear a canvas.
    fn arm() -> Mesh {
        let (rings, sides, radius) = (5usize, 8usize, 0.2f32);
        let (mut positions, mut normals, mut uvs, mut joints, mut weights) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for ring in 0..rings {
            let (joint, weight): ([u16; 4], [f32; 4]) = match ring {
                0 | 1 => ([0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]),
                2 => ([0, 1, 0, 0], [0.5, 0.5, 0.0, 0.0]),
                _ => ([1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]),
            };
            for side in 0..=sides {
                let (sin, cos) = (side as f32 / sides as f32 * core::f32::consts::TAU).sin_cos();
                positions.push([ring as f32 * 0.5, radius * cos, radius * sin]);
                normals.push([0.0, cos, sin]);
                uvs.push([ring as f32 / (rings - 1) as f32, side as f32 / sides as f32]);
                joints.push(joint);
                weights.push(weight);
            }
        }
        let mut indices = Vec::new();
        for ring in 0..rings - 1 {
            for side in 0..sides {
                let a = (ring * (sides + 1) + side) as u32;
                let (b, c) = (a + 1, a + (sides + 1) as u32);
                indices.extend([a, c, b, b, c, c + 1]);
            }
        }
        let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
        mesh.insert_indices(Indices::U32(indices));
        mesh
    }

    /// The elbow's transform with the forearm bent `bend` radians about Z. Its parent is the root, at
    /// the origin, so local and world agree.
    fn elbow(bend: f32) -> Transform {
        Transform::from_xyz(1.0, 0.0, 0.0).with_rotation(Quat::from_rotation_z(bend))
    }

    /// An arm on a limb `Gore`, and a floor a metre under it, in a headless app, the elbow bent `bend`
    /// radians. **The mesh entity sits far from the arm on purpose**: drawing a skin ignores its own
    /// transform, so the hit must too.
    fn rigged_arm(bend: f32) -> (App, Entity, Entity, Entity) {
        let mut app = headless();
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(arm());
        let floor_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Y, Vec2::splat(3.0)).mesh().build());
        let inverse_bindposes = app
            .world_mut()
            .resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
            .add(SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY, Mat4::from_translation(Vec3::new(-1.0, 0.0, 0.0))]));
        let world = app.world_mut();
        let root = world.spawn((Transform::IDENTITY, GlobalTransform::IDENTITY)).id();
        let elbow = world.spawn((elbow(bend), GlobalTransform::from(elbow(bend)), ChildOf(root))).id();
        let away = Transform::from_xyz(5.0, 5.0, 5.0);
        let body = world
            .spawn((
                Mesh3d(mesh),
                SkinnedMesh { inverse_bindposes, joints: vec![root, elbow] },
                away,
                GlobalTransform::from(away),
                Gore::skin(Region::Limb),
            ))
            .id();
        let floor = world
            .spawn((Mesh3d(floor_mesh), Transform::from_xyz(0.0, -1.0, 0.0), GlobalTransform::from(Transform::from_xyz(0.0, -1.0, 0.0)), Gore::floor()))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        (app, body, elbow, floor)
    }

    /// Bend the elbow of a [`rigged_arm`] without a propagation pass, which a headless app has none of.
    fn bend_elbow(app: &mut App, elbow_entity: Entity, bend: f32) {
        app.world_mut().entity_mut(elbow_entity).insert((elbow(bend), GlobalTransform::from(elbow(bend))));
    }

    /// A shot straight down −Z at the forearm, `along` metres out of the elbow in the pose it is now in
    /// when bent a quarter-turn (the forearm points up from x = 1).
    fn shoot_forearm(app: &mut App, body: Entity, along: f32) {
        app.world_mut().write_message(GoreHit::impact(body, Vec3::new(1.0, along, 2.0), Vec3::NEG_Z, 0.04, 5.0));
        app.world_mut().run_schedule(FixedUpdate);
    }

    /// **A shot lands on the limb where it is drawn, not where its asset lies.** With the elbow bent a
    /// quarter-turn the forearm stands up from x = 1 and the ray at (1, 0.7) meets it. In the bind pose
    /// that spot is empty air: the same ray at a straight arm opens nothing. The wound is recorded
    /// against the bind pose too — the point 0.7 m out the forearm, at x = 1.7 — so a laceration or a
    /// later pose can find it again.
    #[test]
    fn a_shot_lands_on_the_limb_where_it_is_drawn() {
        let (mut app, body, ..) = rigged_arm(core::f32::consts::FRAC_PI_2);
        assert!(app.world().get::<Dressed>(body).is_some(), "a skinned limb is dressed once its pose can be had");
        shoot_forearm(&mut app, body, 0.7);
        let site = app.world().get::<Wounds>(body).and_then(|w| w.0.first().copied());
        let Some(site) = site else { panic!("a shot at the bent forearm opened no wound") };
        let at = Vec3::from(site.wound.at);
        assert!(at.distance(Vec3::new(1.0, 0.7, 0.2)) < 0.03, "the wound opened at {at:?}, where the forearm is drawn");
        assert!(site.skin.is_some(), "a wound on a skinned mesh keeps its triangle");
        assert!(site.at_local.distance(Vec3::new(1.7, 0.0, 0.2)) < 0.03, "the bind-pose point was {:?}", site.at_local);

        let (mut straight, body, ..) = rigged_arm(0.0);
        shoot_forearm(&mut straight, body, 0.7);
        assert!(
            straight.world().get::<Wounds>(body).is_none_or(|w| w.0.is_empty()),
            "the same ray opened a wound on an arm that is straight: it was cast somewhere else"
        );
    }

    /// **The hit is cast at nothing while a skin has no pose, never at its bind pose.** A scene still
    /// streaming in has joints no transform reaches, and the bind pose is not where that body is.
    #[test]
    fn a_hit_on_a_skin_with_no_pose_is_dropped() {
        let (mut app, body, elbow_entity, _) = rigged_arm(core::f32::consts::FRAC_PI_2);
        app.world_mut().entity_mut(elbow_entity).remove::<GlobalTransform>();
        shoot_forearm(&mut app, body, 0.7);
        assert!(app.world().get::<Wounds>(body).is_none_or(|w| w.0.is_empty()), "a wound opened on a body whose pose was missing");
    }

    /// **A wound on a skin bleeds from where its limb is now.** Two arms take the same shot while bent.
    /// One stays bent; the other straightens straight after. The drips of the first land under x = 1,
    /// where the forearm stood when it was hit; the second's land under x = 1.7, where the same spot of
    /// skin has been carried to. The shot's spray lands elsewhere on the floor (metres out along z),
    /// so only the drips are in the band.
    #[test]
    fn a_wound_on_a_skin_bleeds_from_where_its_limb_is_now() {
        let drips = |straighten: bool| {
            let (mut app, body, elbow_entity, floor) = rigged_arm(core::f32::consts::FRAC_PI_2);
            shoot_forearm(&mut app, body, 0.7);
            if straighten {
                bend_elbow(&mut app, elbow_entity, 0.0);
            }
            for _ in 0..90 {
                app.world_mut().run_schedule(FixedUpdate);
            }
            let Some(wet) = app.world().get::<WetCanvas>(floor) else { panic!("the floor has no wetmap") };
            let n = wet.size();
            // World x of texel column `tx` on a 6 m plane centred on the origin; z likewise from the row.
            let mass = |x: core::ops::Range<f32>| -> u32 {
                let mut sum = 0;
                for ty in 0..n {
                    for tx in 0..n {
                        let u = (tx as f32 + 0.5) / n as f32;
                        let v = (ty as f32 + 0.5) / n as f32;
                        let (wx, wz) = ((u - 0.5) * 6.0, (v - 0.5) * 6.0);
                        if x.contains(&wx) && (0.0..0.4).contains(&wz) {
                            sum += u32::from(wet.amount_at(tx, ty));
                        }
                    }
                }
                sum
            };
            (mass(0.85..1.15), mass(1.55..1.85))
        };
        let (bent_under_elbow, bent_further_out) = drips(false);
        let (straight_under_elbow, straight_further_out) = drips(true);
        assert!(bent_under_elbow > 0 && bent_further_out == 0, "a bent arm dripped ({bent_under_elbow}, {bent_further_out})");
        assert!(
            straight_further_out > 0 && straight_under_elbow == 0,
            "an arm straightened after the hit dripped ({straight_under_elbow}, {straight_further_out}), from where the wound was"
        );
    }

    /// **A slash on a skinned limb is torn into the bind pose, at the triangle that was hit.** The cut
    /// is laid out on the asset, which is the bind pose; a slash across the bent forearm that was laid
    /// out in world space would cut the straight forearm somewhere it never was.
    #[test]
    fn a_slash_on_a_skinned_limb_is_laid_out_in_the_bind_pose() {
        let (mut app, body, ..) = rigged_arm(core::f32::consts::FRAC_PI_2);
        // Along the forearm's own length: up the pose, which is along +X in the bind pose.
        app.world_mut().write_message(GoreHit::slash(body, Vec3::new(1.0, 0.7, 2.0), Vec3::NEG_Z, Vec3::Y, 0.1, 4.0));
        app.world_mut().run_schedule(FixedUpdate);
        let Some(cut) = app.world().get::<Laceration>(body) else { panic!("a slash laid no laceration") };
        let [from, at, to] = [cut.path[0], cut.path[1], cut.path[2]];
        assert!(at.distance(Vec3::new(1.7, 0.0, 0.2)) < 0.03, "the cut's middle is at {at:?} in the bind pose");
        let along = (to - from).normalize_or_zero();
        assert!(along.dot(Vec3::X) > 0.99, "the cut runs {along:?}; up the bent forearm is along +X in the bind pose");
        assert!(((to - from).length() - 0.2).abs() < 0.02, "the cut is {} m long, the slash's 0.2", (to - from).length());
    }

    /// **Blood runs downhill in the world on a skinned limb, whichever way its atlas is laid out.** The
    /// arm's `u` runs along its length and `v` around it. Straight and level, a wound on its side
    /// runs around the tube (`+v`, down the world on that face); with the forearm held straight up it
    /// runs along the tube toward the elbow (`-u`). A rigid mesh keeps the direction it was given, and
    /// a skinned one starts with it.
    #[test]
    fn blood_runs_downhill_in_the_world_on_a_skinned_limb() {
        let direction = |bend: f32, along: f32| {
            let (mut app, body, ..) = rigged_arm(bend);
            assert_eq!(app.world().get::<RunDirection>(body).map(|r| r.0), Some(Vec2::new(0.0, 1.0)), "a skinned part starts with its stated direction");
            shoot_forearm(&mut app, body, along);
            app.world_mut().run_schedule(FixedUpdate);
            app.world().get::<RunDirection>(body).map(|r| r.0)
        };
        let up = direction(core::f32::consts::FRAC_PI_2, 0.7).unwrap_or(Vec2::ZERO);
        assert!(up.x < -0.9 && up.y.abs() < 0.2, "a forearm held up ran blood {up:?}, toward its hand");
        let level = direction(0.0, 0.0).unwrap_or(Vec2::ZERO);
        assert!(level.y > 0.9 && level.x.abs() < 0.2, "a level arm ran blood {level:?}, not around it");
    }

    /// **A skin whose mesh is still streaming is dressed once it lands**, not before — so it is never
    /// dressed without the tangents it needs.
    #[test]
    fn a_skin_waits_for_its_mesh() {
        let mut app = headless();
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().reserve_handle();
        let body = app.world_mut().spawn((Mesh3d(mesh.clone()), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Torso))).id();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<Dressed>(body).is_none(), "nothing to dress yet");
        let inserted = app.world_mut().resource_mut::<Assets<Mesh>>().insert(mesh.id(), Cuboid::new(0.4, 0.6, 0.2).mesh().build());
        assert!(inserted.is_ok());
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<Dressed>(body).is_some(), "dressed on the tick after it landed");
        let tangents = app.world().resource::<Assets<Mesh>>().get(&mesh).is_some_and(|m| m.contains_attribute(Mesh::ATTRIBUTE_TANGENT));
        assert!(tangents);
    }

    /// **A slash tears only the entity it hit.** Two dressed bodies draw one shared mesh handle, as
    /// every instance of a loaded scene does. The slashed one gets a mesh of its own to tear; the
    /// other still draws the shared handle, so no wound opens on it.
    #[test]
    fn a_slash_tears_only_the_entity_it_hit() {
        let mut app = headless();
        let shared = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.6, 0.2).mesh().build());
        let spawn = |app: &mut App, x: f32| {
            app.world_mut()
                .spawn((Mesh3d(shared.clone()), Transform::from_xyz(x, 0.0, 0.0), GlobalTransform::from_xyz(x, 0.0, 0.0), Gore::skin(Region::Limb)))
                .id()
        };
        let (hit, other) = (spawn(&mut app, 0.0), spawn(&mut app, 2.0));
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().write_message(GoreHit::slash(hit, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, Vec3::X, 0.08, 6.0));
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<Laceration>(hit).is_some(), "the slash laid no laceration");
        let drawn = |e: Entity| app.world().get::<Mesh3d>(e).map(|m| m.0.id());
        assert_ne!(drawn(hit), Some(shared.id()), "the slashed body still draws the shared mesh, so its tear lands on every instance");
        assert_eq!(drawn(other), Some(shared.id()), "the body nobody hit stopped drawing its own mesh");
    }

    /// **A textured skin is dressed over its own texture.** The material keeps its texture and
    /// colour factor, and the flaymap is filled with that factor, which is what lets the shader swap
    /// the canvas's flat intact colour for the authored skin exactly.
    #[test]
    fn a_textured_skin_keeps_its_texture() {
        let mut app = headless();
        let texture = app.world_mut().resource_mut::<Assets<Image>>().add(Image::default());
        let factor = Color::srgb(0.3, 0.5, 0.7);
        let authored = StandardMaterial { base_color: factor, base_color_texture: Some(texture.clone()), ..default() };
        let material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(authored);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.6, 0.2).mesh().build());
        let body = app
            .world_mut()
            .spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb)))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        let dressed = app.world().get::<MeshMaterial3d<FleshMaterial>>(body).and_then(|m| app.world().resource::<Assets<FleshMaterial>>().get(&m.0)).cloned();
        let Some(dressed) = dressed else { panic!("the skin was not dressed") };
        assert_eq!(dressed.base.base_color_texture.map(|h| h.id()), Some(texture.id()), "the skin's own texture was replaced");
        assert_eq!(dressed.base.base_color, factor, "the skin's colour factor was replaced");
        let intact = app.world().get::<FlayCanvas>(body).and_then(|f| app.world().resource::<Assets<Image>>().get(&f.albedo())).and_then(|i| i.data.as_ref()).map(|d| [d[0], d[1], d[2]]);
        let want = factor.to_srgba();
        let want = [want.red, want.green, want.blue].map(|c| c * 255.0);
        let Some(intact) = intact else { panic!("the flaymap has no albedo bytes") };
        for (got, want) in intact.iter().zip(want) {
            assert!((*got as f32 - want).abs() <= 1.0, "the canvas's intact skin {intact:?} is not the factor {want:?}");
        }
    }

    /// **Thrown pieces wear the hit part's own material, however deep the part sits.** A glTF scene
    /// puts its meshes under node entities, so here the part is the body root's grandchild.
    #[test]
    fn thrown_pieces_wear_the_skin_of_the_part_that_was_hit() {
        let mut app = headless();
        let cube = Cuboid::new(0.4, 0.4, 0.4).mesh().build();
        let proxy = [crate::ProxyCell::from_box(Vec3::ZERO, Vec3::splat(0.2))];
        let fracture = crate::fracture_mesh(&[(&cube, Mat4::IDENTITY)], &proxy, &crate::CutSettings::new(4, 0.05, 3));
        let breakable = GoreBreakable::from_fracture(fracture, Region::Limb, &crate::cross_section::Scale::default());
        let skin = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial { base_color: Color::srgb(0.3, 0.5, 0.7), ..default() });
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(cube);
        let world = app.world_mut();
        let root = world.spawn((Transform::IDENTITY, GlobalTransform::IDENTITY, breakable)).id();
        let node = world.spawn((Transform::IDENTITY, GlobalTransform::IDENTITY, ChildOf(root))).id();
        let part = world
            .spawn((Mesh3d(mesh), MeshMaterial3d(skin.clone()), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb), ChildOf(node)))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().write_message(GoreHit::impact(part, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, 0.03, 60.0));
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().run_schedule(FixedUpdate);
        let worn: Vec<_> = app
            .world_mut()
            .query_filtered::<&MeshMaterial3d<StandardMaterial>, Or<(With<Flying>, With<Standing>)>>()
            .iter(app.world())
            .map(|m| m.0.id())
            .collect();
        assert!(!worn.is_empty(), "the body did not break");
        assert!(worn.iter().all(|id| *id == skin.id()), "a piece was thrown in a material the part never wore");
    }

    /// **A game with its own physics gets every piece, and the body breaks once.** With
    /// `spawn_fragments` off, the bone handoff writes one [`GoreFractured`] whose pieces each have a
    /// mesh, a place and a size, and the preset spawns none of them and leaves the body to the game.
    /// A second part of the same body hit to bone does not break it again.
    #[test]
    fn a_game_with_physics_gets_the_pieces() {
        let mut app = headless();
        app.world_mut().resource_mut::<GoreDials>().spawn_fragments = false;
        let cube = Cuboid::new(0.4, 0.4, 0.4).mesh().build();
        let proxy = [crate::ProxyCell::from_box(Vec3::ZERO, Vec3::splat(0.2))];
        let fracture = crate::fracture_mesh(&[(&cube, Mat4::IDENTITY)], &proxy, &crate::CutSettings::new(4, 0.05, 3));
        let breakable = GoreBreakable::from_fracture(fracture, Region::Limb, &crate::cross_section::Scale::default());
        let skin = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(cube);
        let world = app.world_mut();
        let root = world.spawn((Transform::from_xyz(0.0, 1.0, 0.0), GlobalTransform::from_xyz(0.0, 1.0, 0.0), breakable)).id();
        let parts: Vec<Entity> = [0.0, 1.0]
            .map(|x| {
                world
                    .spawn((Mesh3d(mesh.clone()), MeshMaterial3d(skin.clone()), Transform::IDENTITY, GlobalTransform::from_xyz(x, 1.0, 0.0), Gore::skin(Region::Limb), ChildOf(root)))
                    .id()
            })
            .into();
        app.world_mut().run_schedule(FixedUpdate);
        for (part, x) in parts.into_iter().zip([0.0, 1.0]) {
            app.world_mut().write_message(GoreHit::impact(part, Vec3::new(x, 1.0, 2.0), Vec3::NEG_Z, 0.03, 60.0));
            app.world_mut().run_schedule(FixedUpdate);
        }
        let messages = app.world().resource::<Messages<GoreFractured>>();
        let broke: Vec<GoreFractured> = messages.get_cursor().read(messages).cloned().collect();
        assert_eq!(broke.len(), 1, "the body broke {} times", broke.len());
        let broke = &broke[0];
        assert_eq!(broke.subject, root);
        assert_eq!(broke.skin.id(), skin.id(), "the pieces' skin is not the part's material");
        assert!(!broke.pieces.is_empty(), "the message carried no pieces");
        let meshes = app.world().resource::<Assets<Mesh>>();
        for piece in &broke.pieces {
            let drawn = [&piece.outer, &piece.cap].into_iter().flatten().collect::<Vec<_>>();
            assert!(!drawn.is_empty() && drawn.iter().all(|h| meshes.contains(*h)), "a piece has no mesh to build from");
            assert!(piece.half_extents.min_element() > 0.0, "a piece has no size to give a collider");
            assert!((piece.transform.translation.y - 1.0).abs() < 0.25, "a piece is not where the body was: {:?}", piece.transform.translation);
        }
        let spawned = app.world_mut().query_filtered::<(), Or<(With<Flying>, With<Standing>)>>().iter(app.world()).count();
        assert_eq!(spawned, 0, "the preset spawned pieces a game with physics owns");
        assert!(app.world().get_entity(root).is_ok(), "the preset despawned a body the game owns");
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
        assert!(app.world().get::<Wounds>(bruised).is_none() && app.world().get::<Wounds>(burnt).is_none(), "neither bleeds");
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
