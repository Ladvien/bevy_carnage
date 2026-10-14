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
//! # The player's settings
//!
//! [`crate::GorePolicy`] is read at every hit, so a settings menu changes the next one and takes
//! nothing back. **Reduction is substitution, never deletion:** at [`crate::GoreTier::Stylised`] a hit
//! that would have opened a wound is still found and announced as [`crate::Wounded`], at the same
//! point with the same normal and area, and nothing is painted, torn, spilled or thrown, so the
//! game puts its own effect where the blood would have been. `blood_decals` is the world's share of
//! the blood (the spray, and what drips off the wound), `viscera` the guts and `dismemberment` a body
//! coming apart, each its own switch; `GoreTier::Blood` is the tier with the last two off.
//!
//! # The clock
//!
//! [`GoreClock`] is one `u32` advanced once per `FixedUpdate`. That is the tick every leaf asks the
//! caller for, and this preset *is* the caller. Nothing here reads `Time`; a fixed step is what makes
//! the canvases' digests reproducible, and a game that wants the gore on its own step configures
//! [`GoreSystems`] and drives the clock itself.

use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, UvChannel, VertexAttributeValues};
use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use crate::cross_section::{CrossSectionPlugin, CrossSectionSettings, Layers, Region};
use crate::flaymap::{BoneExposed, FlayCanvas, FlaySettings, FlaymapPlugin, Layer};
use crate::laceration::{Gape, Laceration, LacerationBed, LacerationClock, LacerationPlugin, Tension};
use crate::viscera::{GUT_ROUGHNESS, Mesentery, Strand, ViscSettings, VisceraPlugin, VisceraSystems, gut_image, spill, tube_mesh};
use crate::wetmap::{Print, StainShape, WetCanvas, WetSettings, WetmapPlugin};
use crate::bloodstain::{BloodSettings, Wound as BloodWound, WoundKind, wound_seed};

use crate::flesh::{FleshMaterial, FleshMode, FleshParams, FleshPlugin, FleshTables};
use crate::skin::{self, Pose, Rig};
use crate::{CarnagePlugin, FractureCache, FractureSubject, GorePolicy, Wounded};

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
    /// **The edge, in texels, of this surface's canvases**, or `None` for the preset's own:
    /// [`GoreDials::canvas`] for skin and cloth, [`GoreDials::floor_canvas`] for a floor or a wall. A dressed
    /// skin owns three square canvases and each is a few bytes a texel, so the edge is the memory
    /// bill: a head, a hand or a distant character does not need the 128 a torso's wounds read at, and
    /// 64 costs a quarter. See [`Gore::with_canvas`] and *What it costs* in `docs/preset.md`.
    pub canvas: Option<u32>,
    /// **Lay a floor's or a wall's canvas over it from its broadest side**, rather than by its own
    /// `UV_0` — see [`Gore::projected`]. `false` by default.
    pub projected: bool,
}

/// The four surfaces the preset knows how to dress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GoreKind {
    /// A body part: a flaymap and a wetmap over skin, banded by `region`'s thickness row.
    Skin(Region),
    /// Clothing: a wetmap whose film is composited over the fabric on the GPU.
    Cloth,
    /// Ground: a wetmap that catches what a wound throws.
    Floor,
    /// **An upright hard surface** — a wall, a door, the side of a crate: a wetmap the spray's
    /// droplets are cast at, as at cloth, but blood stands on it as a film and runs down it, where a
    /// floor takes the closed-form stains of a horizontal plane and cloth soaks it up.
    Wall,
}

impl Gore {
    /// A body part of `region`.
    pub fn skin(region: Region) -> Self {
        Self { kind: GoreKind::Skin(region), gravity_uv: Vec2::new(0.0, 1.0), uv_per_metre: None, canvas: None, projected: false }
    }
    /// A fabric surface.
    pub fn cloth() -> Self {
        Self { kind: GoreKind::Cloth, gravity_uv: Vec2::new(0.0, 1.0), uv_per_metre: None, canvas: None, projected: false }
    }
    /// A floor. Blood pools rather than runs, so its UV gravity is zero.
    pub fn floor() -> Self {
        Self { kind: GoreKind::Floor, gravity_uv: Vec2::ZERO, uv_per_metre: None, canvas: None, projected: false }
    }
    /// A wall or any upright hard surface — see [`GoreKind::Wall`]. Blood runs down its atlas's `+v`,
    /// as on cloth; a wall whose UVs run the other way states it with [`Gore::with_gravity_uv`].
    pub fn wall() -> Self {
        Self { kind: GoreKind::Wall, gravity_uv: Vec2::new(0.0, 1.0), uv_per_metre: None, canvas: None, projected: false }
    }
    /// The same surface with its canvases `texels` on an edge instead of the preset's dial — see
    /// [`Gore::canvas`]. Read when the mesh is dressed; a surface already dressed keeps its canvases.
    /// Zero leaves the preset's own.
    pub fn with_canvas(mut self, texels: u32) -> Self {
        self.canvas = (texels > 0).then_some(texels);
        self
    }
    /// **The same floor or wall with its canvas laid over it from its broadest side**, for a mesh
    /// whose `UV_0` tiles a texture — a level's floor, whose UVs run from 0 to 20 so a 15 cm tile
    /// repeats — and so cannot hold blood: the wetmap clamps a UV outside `[0, 1]` to the canvas's edge.
    ///
    /// When it is dressed the preset gives the entity a copy of its mesh whose `UV_0` is the mesh seen
    /// square-on from the side it mostly faces — from above for a floor, from the front for a wall,
    /// `+v` down a wall — scaled to fit `[0, 1]` without stretching, and whose `UV_1` is the `UV_0` it
    /// had; the surface's own textures read `UV_1`, so its tiles stay as they were, and every canvas
    /// reads the projection. The density is the projection's, so [`Gore::uv_per_metre`] is set from it.
    /// The entity's `Mesh3d` is that copy from then on; a mesh another entity shares is not touched.
    ///
    /// Refused, with one warning, on a mesh that already has a `UV_1` — it may be a lightmap's, and
    /// the projection would overwrite it — and on a skin or a cloth, whose canvases must follow the
    /// body's own atlas.
    pub fn projected(mut self) -> Self {
        self.projected = true;
        self
    }
    /// The canvas edge to dress this surface with.
    fn canvas_texels(&self, dials: &GoreDials) -> u32 {
        self.canvas.unwrap_or(match self.kind {
            GoreKind::Floor | GoreKind::Wall => dials.floor_canvas,
            _ => dials.canvas,
        })
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

/// **Dress a whole loaded scene.** Put this on the entity a glTF scene is spawned under and every mesh
/// that appears below it, now or when the scene finishes streaming in, is given a [`Gore`] by name:
/// the one system a game would otherwise write to walk the spawned hierarchy.
///
/// A mesh is matched on its own `Name` and then on the name of each node above it, nearest first, so
/// a mesh called `Mesh.001` under a node called `Head` is the head. **A name is read as words**, split
/// at punctuation, at a capital that follows a lower-case letter and at digits, so `ArmL`, `Arm_L` and
/// `arm.L` are the words `arm` `l`. A rule matches when its words are a run of the name's, ignoring
/// case: `"arm"` matches all three and not `Armature`, which a Blender export puts above its meshes,
/// nor `Armor`. Substring matching dressed a whole character as one limb on exactly that name. **The
/// last rule that matches wins**, so a rule added to [`GoreScene::humanoid`] overrides its own. What no
/// rule matches gets the default, or is left alone if there is none. A mesh that already has a
/// [`Gore`] is never touched, so one surface can be dressed by hand and the rest by name. The wound
/// beds the preset itself spawns under a slashed skin ([`LacerationBed`]) are never dressed, though
/// they sit below a node with its name.
///
/// ```ignore
/// commands.spawn((
///     WorldAssetRoot(assets.load("enemy.glb#Scene0")),
///     GoreScene::skin(Region::Torso)
///         .with("head", Gore::skin(Region::Head))
///         .with("arm", Gore::skin(Region::Limb))
///         .with("shirt", Gore::cloth())
///         .without("weapon"),
/// ));
/// // Later, at whatever your physics found: the preset finds the mesh.
/// hits.write(GoreHit::impact(enemy, from, dir, 0.03, 12.0));
/// ```
#[derive(Component, Clone, Debug)]
pub struct GoreScene {
    rules: Vec<(Vec<String>, Option<Gore>)>,
    default: Option<Gore>,
}

/// A name as its lower-case words: split at anything that is not a letter or a digit, at a capital
/// that follows a lower-case letter or a digit, where a run of capitals ends in a word (`XMLFile` is
/// `xml` `file`), and between a letter and a digit.
fn words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            continue;
        }
        let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied().filter(|p| p.is_alphanumeric());
        let next = chars.get(i + 1).copied();
        let breaks = prev.is_some_and(|p| {
            (c.is_uppercase() && (p.is_lowercase() || p.is_ascii_digit()))
                || (c.is_uppercase() && p.is_uppercase() && next.is_some_and(char::is_lowercase))
                || (c.is_ascii_digit() != p.is_ascii_digit())
        });
        if breaks && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

impl GoreScene {
    /// Every mesh below is dressed as `default`, unless a rule says otherwise.
    pub fn new(default: Gore) -> Self {
        Self { rules: Vec::new(), default: Some(default) }
    }

    /// Every mesh below is skin of `region`, unless a rule says otherwise.
    pub fn skin(region: Region) -> Self {
        Self::new(Gore::skin(region))
    }

    /// A mesh named for `name`, or below a node that is, is dressed as `gore`. See [`GoreScene`] for
    /// how a name is matched; a later rule beats an earlier one.
    pub fn with(mut self, name: &str, gore: Gore) -> Self {
        self.rules.push((words(name), Some(gore)));
        self
    }

    /// A mesh named for `name`, or below a node that is, is left alone: a held weapon, a prop.
    pub fn without(mut self, name: &str) -> Self {
        self.rules.push((words(name), None));
        self
    }

    /// A mesh no rule matches is left alone instead of dressed as a default.
    pub fn otherwise_nothing(mut self) -> Self {
        self.default = None;
        self
    }

    /// **A starting point for a humanoid**, from the names a character kit carries, checked against
    /// the names in six characters and a creature from the game this crate is developed in. A real
    /// character is a body and, over it, a wardrobe and some gear, and which is which is the whole job:
    ///
    /// - **skin**: `body`, `skin` (the material a kit names its body's after), `torso`, `chest`; a
    ///   `head` or `face`; an `arm`, `leg`, `hand`, `foot`, `thigh`, `shin` and the like. These are
    ///   what a hit hurts.
    /// - **cloth**: `bodysuit`, `jumpsuit`, `labcoat`, `vest`, `shirt`, `boots`, `gloves`, `base` and
    ///   the like. These soak the blood and are never the thing a hit peels.
    /// - **left alone**: `belt`, `holster`, `pouch`, `backpack`, `chestrig`, `kneepads`, `eyewear`,
    ///   `facewear`, `laces`, `buttons`, `hair`, and every weapon. A rule for these wins over the others,
    ///   so `thigh_holster` is a holster and `body.hair_cap` is hair.
    ///
    /// **Anything it does not recognise is left alone.** On a real character that is nearly always gear,
    /// and the first version of this dressed it as torso skin, which put a belt, a backpack and a chest
    /// rig in line to be flayed and wounded in front of the body they hang on. The cost is that a
    /// character whose only mesh has a name it does not know is not dressed at all: name it with
    /// [`with`](Self::with). Rules you add after it win, so name what your rig calls things.
    pub fn humanoid() -> Self {
        let body = ["body", "skin", "torso", "chest", "abdomen", "pelvis", "neck"];
        let limbs = [
            "arm", "arms", "forearm", "forearms", "upperarm", "lowerarm", "hand", "hands", "leg", "legs", "upperleg", "lowerleg", "foot", "feet", "thigh", "thighs",
            "calf", "calves", "shin", "finger", "fingers", "toe", "toes",
        ];
        let heads = ["head", "skull", "face"];
        let cloth = [
            "base", "bodysuit", "jumpsuit", "hazmatsuit", "suit", "labcoat", "coat", "jacket", "vest", "shirt", "tshirt", "sweater", "hoodie", "pants", "trousers", "jeans",
            "shorts", "dress", "skirt", "cloak", "cape", "scarf", "hat", "cloth", "clothes", "fabric", "uniform", "sleeve", "sleeves", "boots", "boot", "shoes", "shoe",
            "gloves", "glove", "socks",
        ];
        let gear = [
            "hair", "belt", "strap", "straps", "buckle", "hook", "holster", "pouch", "backpack", "pack", "rig", "chestrig", "kneepads", "pads", "pad", "plates", "plate",
            "armor", "armour", "helmet", "mask", "facewear", "eyewear", "glasses", "goggles", "visor", "laces", "lace", "buttons", "button", "trim", "radio", "keycard",
            "clipboard", "flashlight", "weapon", "muzzle", "pivot", "gun", "rifle", "pistol", "sword", "knife", "axe", "bow",
        ];
        let mut scene = Self::skin(Region::Torso).otherwise_nothing();
        for word in body {
            scene = scene.with(word, Gore::skin(Region::Torso));
        }
        for word in limbs {
            scene = scene.with(word, Gore::skin(Region::Limb));
        }
        for word in heads {
            scene = scene.with(word, Gore::skin(Region::Head));
        }
        for word in cloth {
            scene = scene.with(word, Gore::cloth());
        }
        for word in gear {
            scene = scene.without(word);
        }
        scene
    }

    /// What a mesh is dressed as, from its own name and the names above it, nearest first.
    fn gore_for(&self, names: &[Vec<String>]) -> Option<Gore> {
        for name in names {
            let matches = |rule: &Vec<String>| !rule.is_empty() && name.windows(rule.len()).any(|run| run == rule.as_slice());
            if let Some((_, gore)) = self.rules.iter().rev().find(|(rule, _)| matches(rule)) {
                return *gore;
            }
        }
        self.default
    }
}

/// A mesh no [`Gore`] has claimed, and that is not the preset's own wound bed.
type Unclaimed = (With<Mesh3d>, Without<Gore>, Without<LacerationBed>);

/// **Dress every mesh that appears under a [`GoreScene`].** Event-shaped rather than a poll: a mesh is
/// looked at when it is added, and a scene is walked when it is added, so a scene that streams in over
/// many ticks and one whose component was added after its meshes both end up dressed. A
/// [`LacerationBed`] is a mesh the preset spawned *under* a dressed skin, so it would match that skin's
/// name and be dressed as a second skin; it is skipped.
#[allow(clippy::too_many_arguments)]
fn scene_gore(
    mut commands: Commands,
    added_meshes: Query<Entity, (Added<Mesh3d>, Unclaimed)>,
    added_scenes: Query<Entity, Added<GoreScene>>,
    children: Query<&Children>,
    meshes: Query<(), Unclaimed>,
    scenes: Query<&GoreScene>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
) {
    let mut candidates: Vec<Entity> = added_meshes.iter().collect();
    for root in &added_scenes {
        candidates.extend(children.iter_descendants(root).filter(|e| meshes.contains(*e)));
        if meshes.contains(root) {
            candidates.push(root);
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    for mesh in candidates {
        // The nearest scene at or above the mesh, and the words of every name between them. The
        // scene's own name is the game's label for the whole character, not a part of it.
        let mut chain: Vec<Vec<String>> = Vec::new();
        let mut at = mesh;
        loop {
            let scene = scenes.get(at).ok();
            if (scene.is_none() || at == mesh)
                && let Ok(name) = names.get(at)
            {
                chain.push(words(name.as_str()));
            }
            if let Some(scene) = scene {
                if let Some(gore) = scene.gore_for(&chain) {
                    commands.entity(mesh).try_insert(gore);
                }
                break;
            }
            match parents.get(at) {
                Ok(parent) => at = parent.parent(),
                Err(_) => break,
            }
        }
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
    /// How far along the ray it landed, in multiples of `dir`. The map into a mesh's own space is
    /// affine, so this is the same number in world space: two surfaces' landings compare directly.
    t: f32,
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

/// **The stain one droplet leaves where it struck a hard surface**: `crate::bloodstain::stain_shape`
/// from the droplet's own size and speed, the angle its path makes with the face it struck, and the
/// direction it was travelling along that face, carried into the atlas through the triangle's UVs —
/// so a drop striking a wall at a slant is an ellipse drawn out the way it was going, with its spines
/// downrange. `None` for a triangle with no area or no UV extent, or a path along the face.
fn hard_stain(mesh: &Mesh, xf: &GlobalTransform, l: &Landing, d: &crate::bloodstain::Droplet, blood: &BloodSettings, seed: u32) -> Option<StainShape> {
    // The path in the mesh's own space, where the triangle is.
    let path = xf.affine().inverse().transform_vector3(Vec3::from_array(d.dir)).normalize_or_zero();
    let (travel, normal) = face_uv(mesh, l.tri, path)?;
    let sin_theta = path.dot(normal).abs();
    if path == Vec3::ZERO || sin_theta < 1.0e-3 {
        return None;
    }
    let impact = crate::bloodstain::Impact {
        speed: d.speed,
        diameter: d.diameter,
        angle_rad: libm::asinf(sin_theta.clamp(0.0, 1.0)),
        roughness: blood.substrate_roughness,
        travel: if travel.is_finite() { travel.to_array() } else { [0.0, 0.0] },
    };
    Some(crate::bloodstain::stain_shape(&impact, blood, seed))
}

/// **A mesh-space vector, laid along triangle `tri` and read in UV**: its part along the face, as the
/// change of `UV_0` it makes there — `w = a·e1 + b·e2` by the triangle's Gram matrix, then
/// `a·Δuv1 + b·Δuv2` — and the face's unit normal. `None` for a triangle with no area or no UVs.
fn face_uv(mesh: &Mesh, tri: [u32; 3], v: Vec3) -> Option<(Vec2, Vec3)> {
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let Ok(Some(VertexAttributeValues::Float32x2(uvs))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) else {
        return None;
    };
    let [p0, p1, p2] = tri.map(|i| positions.get(i as usize).copied().map(Vec3::from));
    let [t0, t1, t2] = tri.map(|i| uvs.get(i as usize).copied().map(Vec2::from));
    let (p0, p1, p2, t0, t1, t2) = (p0?, p1?, p2?, t0?, t1?, t2?);
    let (e1, e2) = (p1 - p0, p2 - p0);
    let normal = e1.cross(e2).normalize_or_zero();
    let (g11, g12, g22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
    let det = g11 * g22 - g12 * g12;
    if normal == Vec3::ZERO || det.abs() < 1.0e-12 {
        return None;
    }
    let along = v - normal * v.dot(normal);
    let (we1, we2) = (along.dot(e1), along.dot(e2));
    let (a, b) = ((we1 * g22 - we2 * g12) / det, (we2 * g11 - we1 * g12) / det);
    Some(((t1 - t0) * a + (t2 - t0) * b, normal))
}

/// **Where a ray crosses a triangle**: `(t, u, v)`, its distance along `d` in lengths of `d` and the
/// barycentric weights of the second and third corners, by Möller & Trumbore (1997). `None` for a
/// miss, a ray along the face, or a crossing at or behind the origin.
fn crossing(origin: Vec3, d: Vec3, [pa, pb, pc]: [Vec3; 3]) -> Option<(f32, f32, f32)> {
    let e1 = pb - pa;
    let e2 = pc - pa;
    let h = d.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1.0e-9 {
        return None;
    }
    let inv_det = 1.0 / det;
    let s_vec = origin - pa;
    let u = inv_det * s_vec.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s_vec.cross(e1);
    let v = inv_det * d.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = inv_det * e2.dot(q);
    (t > 0.0).then_some((t, u, v))
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
        let Some((t, u, v)) = crossing(origin, d, [pa, pb, pc]) else { continue };
        if best.is_some_and(|(bt, _)| bt <= t) {
            continue;
        }
        let Some(landing) = land(origin, d, (t, u, v), [a, b, c], positions, uvs) else { continue };
        best = Some((t, landing));
    }
    best.map(|(_, landing)| landing)
}

/// The landing a crossing `(t, u, v)` of triangle `[a, b, c]` makes, mesh-local. `None` when a
/// corner has no position or no UV, which is a triangle a cast passes over.
fn land(origin: Vec3, d: Vec3, (t, u, v): (f32, f32, f32), [a, b, c]: [usize; 3], positions: &[[f32; 3]], uvs: &[[f32; 2]]) -> Option<Landing> {
    let (pa, pb, pc) = (Vec3::from(*positions.get(a)?), Vec3::from(*positions.get(b)?), Vec3::from(*positions.get(c)?));
    let (ua, ub, uc) = (Vec2::from(*uvs.get(a)?), Vec2::from(*uvs.get(b)?), Vec2::from(*uvs.get(c)?));
    Some(Landing {
        uv: ua * (1.0 - u - v) + ub * u + uc * v,
        point: origin + d * t,
        normal: (pb - pa).cross(pc - pa).normalize_or_zero(),
        tri: [a as u32, b as u32, c as u32],
        bary: Vec3::new(1.0 - u - v, u, v),
        t,
    })
}

/// **[`ray_hit`], through `index` when there is one**: the same landing, to the bit, from the few
/// triangles whose boxes the ray enters instead of all of them. See [`crate::bvh`].
fn ray_cast(mesh: &Mesh, index: Option<crate::bvh::Fitted<'_>>, xf: &GlobalTransform, from: Vec3, dir: Vec3) -> Option<Landing> {
    let Some(index) = index else { return ray_hit(mesh, xf, from, dir) };
    let inv = xf.affine().inverse();
    let origin = inv.transform_point3(from);
    let d = inv.transform_vector3(dir);
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let Ok(Some(VertexAttributeValues::Float32x2(uvs))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) else {
        return None;
    };
    let (t, u, v, tri) = index.nearest(positions, origin, d, crossing)?;
    land(origin, d, (t, u, v), tri.corners.map(|c| c as usize), positions, uvs)
}

/// **The triangles a cast at `mesh` considers**, in index order with their ordinals — every triangle
/// whose corners all have a position and a UV, which are the ones [`ray_hit`] does not pass over —
/// and the positions they index. `None` for a mesh with no `Float32x3` positions, no `Float32x2`
/// `UV_0`, or indices that cannot be read.
fn castable(mesh: &Mesh) -> Option<(&[[f32; 3]], Vec<crate::bvh::Tri>)> {
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let Ok(Some(VertexAttributeValues::Float32x2(uvs))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) else {
        return None;
    };
    let ok = |c: [u32; 3]| c.iter().all(|&i| (i as usize) < positions.len() && (i as usize) < uvs.len());
    let tri = |k: usize, corners: [u32; 3]| ok(corners).then_some(crate::bvh::Tri { corners, ordinal: k as u32 });
    let tris = match mesh.try_indices_option().ok()? {
        Some(Indices::U32(ix)) => ix.as_chunks::<3>().0.iter().enumerate().filter_map(|(k, t)| tri(k, *t)).collect(),
        Some(Indices::U16(ix)) => ix.as_chunks::<3>().0.iter().enumerate().filter_map(|(k, t)| tri(k, t.map(u32::from))).collect(),
        None => (0..positions.len() / 3).filter_map(|k| tri(k, [3 * k as u32, 3 * k as u32 + 1, 3 * k as u32 + 2])).collect(),
    };
    Some((positions, tris))
}

/// **What a mesh's hierarchy was built from**: its vertex count, its index count, and a hash of the
/// bits of up to 32 of its positions spread through the buffer — so a mesh rewritten under the same
/// asset, which Bevy reports as modified anyway, is indexed again even where the report is missed.
type MeshStamp = (usize, usize, u64);

fn mesh_stamp(mesh: &Mesh) -> Option<MeshStamp> {
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let indices = mesh.try_indices_option().ok()?.map_or(0, |ix| ix.len());
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let step = (positions.len() / 32).max(1);
    for p in positions.iter().step_by(step) {
        for c in p {
            for b in c.to_bits().to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    Some((positions.len(), indices, h))
}

/// **The rigid meshes hits are cast at, indexed** ([`crate::bvh`]), kept across ticks by asset and
/// built again when its [`MeshStamp`] moves or Bevy reports the asset modified. A skinned surface is
/// cast at its pose, a new mesh each batch, and walks its triangles.
#[derive(Default)]
struct MeshIndex {
    built: std::collections::HashMap<AssetId<Mesh>, (MeshStamp, crate::bvh::Bvh)>,
    /// For each skinned mesh, the boxes its joints carry, or `None` for a rig they cannot box.
    joints: std::collections::HashMap<AssetId<Mesh>, (MeshStamp, Option<JointBoxes>)>,
}

impl MeshIndex {
    /// Index `mesh` under `id`, unless it already is as it is now. A mesh that cannot be cast at gets
    /// an empty index, which lands nothing, as walking it does, and is not built again each tick.
    fn ensure(&mut self, id: AssetId<Mesh>, mesh: &Mesh) {
        let stamp = mesh_stamp(mesh).unwrap_or((0, 0, 0));
        if self.built.get(&id).is_some_and(|(s, _)| *s == stamp) {
            return;
        }
        let index = match castable(mesh) {
            Some((positions, tris)) => crate::bvh::Bvh::build(positions, tris),
            None => crate::bvh::Bvh::build(&[], Vec::new()),
        };
        self.built.insert(id, (stamp, index));
    }

    fn get(&self, id: AssetId<Mesh>) -> Option<&crate::bvh::Bvh> {
        self.built.get(&id).map(|(_, b)| b)
    }

    /// Box the joints of skinned `mesh` under `id`, unless they already are as it is now.
    fn ensure_joints(&mut self, id: AssetId<Mesh>, mesh: &Mesh) {
        let stamp = mesh_stamp(mesh).unwrap_or((0, 0, 0));
        if self.joints.get(&id).is_some_and(|(s, _)| *s == stamp) {
            return;
        }
        self.joints.insert(id, (stamp, JointBoxes::of(mesh)));
    }

    fn joint_boxes(&self, id: AssetId<Mesh>) -> Option<&JointBoxes> {
        self.joints.get(&id).and_then(|(_, j)| j.as_ref())
    }

    /// Forget everything kept for `id`.
    fn forget(&mut self, id: &AssetId<Mesh>) {
        self.built.remove(id);
        self.joints.remove(id);
    }
}

/// **The bind-space box of the vertices each joint moves**, for boxing a skinned mesh's pose without
/// posing it. A posed vertex is `Σ wᵢ·Mᵢ·v` over its joints, the GPU's sum: where the weights add to
/// one it is a blend of the points `Mᵢ·v`, each inside joint `i`'s box carried by `Mᵢ`, so it lies in
/// the box round every carried box; where they add to `s` it is `s` times such a blend, within
/// `|s − 1|` of its size of it — `slack`, the largest such — and where they add to nothing it is the
/// origin. Posing a mesh is a blend of up to four matrices per vertex; this is eight corners per joint.
#[derive(Debug)]
struct JointBoxes {
    /// `(joint, min, max)`, in joint order.
    boxes: Vec<(u16, Vec3, Vec3)>,
    slack: f32,
    /// Whether some vertex has no weight at all, and so is posed to the origin.
    unweighted: bool,
}

impl JointBoxes {
    /// `None` for a mesh with no rig, a position that is not finite, or a weight that is negative or
    /// not finite: no box can be had from its joints, so the pose itself is boxed.
    fn of(mesh: &Mesh) -> Option<Self> {
        let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
            return None;
        };
        let Ok(Some(VertexAttributeValues::Uint16x4(joints))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_JOINT_INDEX) else {
            return None;
        };
        let Ok(Some(VertexAttributeValues::Float32x4(weights))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_JOINT_WEIGHT) else {
            return None;
        };
        if joints.len() != positions.len() || weights.len() != positions.len() {
            return None;
        }
        let mut boxes: std::collections::BTreeMap<u16, (Vec3, Vec3)> = std::collections::BTreeMap::new();
        let (mut slack, mut unweighted) = (0.0f32, false);
        for ((p, j), w) in positions.iter().zip(joints).zip(weights) {
            let p = Vec3::from(*p);
            if !p.is_finite() {
                return None;
            }
            let mut sum = 0.0f32;
            for (joint, weight) in j.iter().zip(w) {
                if !weight.is_finite() || *weight < 0.0 {
                    return None;
                }
                if *weight == 0.0 {
                    continue;
                }
                sum += weight;
                let span = boxes.entry(*joint).or_insert((p, p));
                *span = (span.0.min(p), span.1.max(p));
            }
            // A vertex with no weight is posed to the origin, which `unweighted` covers; its sum's distance
            // from one is not a slack, and counted as one it padded the box by its whole size.
            if sum == 0.0 {
                unweighted = true;
            } else {
                slack = slack.max((sum - 1.0).abs());
            }
        }
        Some(Self { boxes: boxes.into_iter().map(|(j, (lo, hi))| (j, lo, hi)).collect(), slack, unweighted })
    }

    /// **The world box the mesh is inside under `pose`**, padded for the weights' slack and for
    /// rounding. `None` when a joint is past the end of the pose, which posing would refuse too.
    fn bounds(&self, pose: &Pose) -> Option<Bounds> {
        let (mut lo, mut hi) = if self.unweighted { (Vec3::ZERO, Vec3::ZERO) } else { (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)) };
        for (joint, min, max) in &self.boxes {
            let m = pose.joint(*joint)?;
            for k in 0..8 {
                let corner = Vec3::new(if k & 1 == 0 { min.x } else { max.x }, if k & 2 == 0 { min.y } else { max.y }, if k & 4 == 0 { min.z } else { max.z });
                let w = m.transform_point3(corner);
                lo = lo.min(w);
                hi = hi.max(w);
            }
        }
        let size = lo.abs().max(hi.abs()).max_element();
        let pad = Vec3::splat(1.0e-4 + size * (self.slack + 1.0e-5)) + (hi - lo) * 1.0e-4;
        let (lo, hi) = (lo - pad, hi + pad);
        (lo.is_finite() && hi.is_finite()).then_some(Bounds { min: lo, max: hi, to_local: bevy::math::Affine3A::IDENTITY, world_min: lo, world_max: hi })
    }
}

/// **Where a surface can be reached**, for the broad phase of a batch's casts.
#[derive(Clone, Copy)]
enum Reach {
    /// Nowhere: a skin whose pose cannot be had, which is never cast at its bind pose.
    Never,
    /// Anywhere: no box could be had, so every ray is cast at it and its refusal is still heard.
    Anywhere,
    /// Inside this box.
    Within(Bounds),
}

impl Reach {
    /// The box, when there is one.
    fn bounds(self) -> Option<Bounds> {
        match self {
            Reach::Within(b) => Some(b),
            _ => None,
        }
    }
}

/// **Where `surface` can be reached this batch, without posing it**: a rigid surface's box from its
/// asset; a skinned one's from the boxes its joints carry under its pose. Only a rig its joints cannot
/// box is posed for it, and the pose is kept in `posed` for the casts that follow.
fn reach_of(
    surface: &Surface,
    asset: &Mesh,
    rigging: &Rigging,
    posed: &mut std::collections::HashMap<Entity, Option<Mesh>>,
    identity: &GlobalTransform,
    index: &MeshIndex,
) -> Reach {
    let within = |b: Option<Bounds>| b.map_or(Reach::Anywhere, Reach::Within);
    match surface.skin.as_ref().filter(|_| Rig::of(asset).is_some()) {
        None => within(Bounds::of(asset, &surface.xf)),
        Some(skin) => {
            let Some(pose) = rigging.pose(skin) else { return Reach::Never };
            if let Some(b) = index.joint_boxes(surface.mesh.id()).and_then(|j| j.bounds(&pose)) {
                return Reach::Within(b);
            }
            match cast_at(surface, asset, rigging, posed, identity) {
                Some((m, xf)) => within(Bounds::of(m, xf)),
                None => Reach::Never,
            }
        }
    }
}

/// **The index to cast at `surface` through, drawn now as `mesh`**: its mesh's own for a rigid one;
/// for a skinned one, the tree built over its bind pose refitted to `mesh`, its pose — once a batch,
/// kept in `refits` — since posing moves the vertices and not the triangles.
fn cast_index<'a>(
    index: &'a MeshIndex,
    refits: &'a mut std::collections::HashMap<Entity, crate::bvh::Refit>,
    surface: &Surface,
    asset: &Mesh,
    mesh: &Mesh,
) -> Option<crate::bvh::Fitted<'a>> {
    let tree = index.get(surface.mesh.id())?;
    match surface.skin.as_ref().filter(|_| Rig::of(asset).is_some()) {
        None => Some(tree.fitted()),
        Some(_) => {
            let refit = match refits.entry(surface.entity) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(tree.refit(positions_of(mesh)?)),
            };
            Some(tree.with(refit))
        }
    }
}

/// A mesh's `Float32x3` positions, or `None`.
fn positions_of(mesh: &Mesh) -> Option<&[[f32; 3]]> {
    match mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) {
        Ok(Some(VertexAttributeValues::Float32x3(positions))) => Some(positions),
        _ => None,
    }
}

/// **How many steps a slash's path takes each way from the hit**, each that fraction of the blade's
/// half-length. On the skinned mannequin's thigh, 8 cm in radius, a 7 cm half-blade steps 9 mm and the
/// skin falls half a millimetre below each step, which the next step's ray takes back up.
const SLASH_STEPS: u32 = 8;

/// **A slash's path, laid on the skin it cuts**, in the mesh's own space, from the blade's middle at
/// `at` on a face whose normal is `normal`, `half` either way along `along`.
///
/// A blade drawn across a limb follows the limb. A straight line through the hit in the plane of the
/// skin there leaves it, and the wound bed [`tear`](crate::laceration::tear) builds along the path
/// leaves with it: the mannequin's thigh slash runs 5.6 cm of its 7 cm half-length round a thigh 8 cm
/// in radius, so its ends stood `√(r² + t²) − r`, near two centimetres, off the skin, and the bed was
/// a pale plank laid across the leg. So the path is walked out from the hit both ways in
/// [`SLASH_STEPS`] steps, each along the skin at the last point and brought back onto it by a ray down
/// that point's normal, landing only on a face that faces out the way the skin there does. A step
/// whose ray finds no such face within a step of the tangent ends the walk on that side: the blade
/// has run off what it can reach.
fn slash_path(mesh: &Mesh, at: Vec3, normal: Vec3, along: Vec3, half: f32) -> Vec<Vec3> {
    let step = half / SLASH_STEPS as f32;
    let near = triangles_near(mesh, at, half + 2.0 * step);
    let walk = |sign: f32| {
        let mut points = Vec::with_capacity(SLASH_STEPS as usize);
        let (mut p, mut n, mut heading) = (at, normal, along * sign);
        for _ in 0..SLASH_STEPS {
            let ahead = (heading - n * heading.dot(n)).normalize_or_zero();
            if ahead == Vec3::ZERO {
                break;
            }
            let from = p + ahead * step + n * step;
            let mut best: Option<(f32, Vec3)> = None;
            for tri in &near {
                let face = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
                if face.dot(n) > 0.0
                    && let Some((t, _, _)) = crossing(from, -n, *tri)
                    && t <= 2.0 * step
                    && best.is_none_or(|(bt, _)| t < bt)
                {
                    best = Some((t, face));
                }
            }
            let Some((t, face)) = best else { break };
            p = from - n * t;
            n = face;
            heading = ahead;
            points.push(p);
        }
        points
    };
    let (behind, ahead) = (walk(-1.0), walk(1.0));
    behind.into_iter().rev().chain(core::iter::once(at)).chain(ahead).collect()
}

/// The triangles of `mesh` whose bounding boxes come within `reach` of `p` on every axis, as corners.
fn triangles_near(mesh: &Mesh, p: Vec3, reach: f32) -> Vec<[Vec3; 3]> {
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return Vec::new();
    };
    let (lo, hi) = (p - Vec3::splat(reach), p + Vec3::splat(reach));
    let corner = |i: usize| positions.get(i).copied().map(Vec3::from);
    let mut near = Vec::new();
    let mut take = |[a, b, c]: [usize; 3]| {
        let (Some(a), Some(b), Some(c)) = (corner(a), corner(b), corner(c)) else { return };
        if a.min(b).min(c).cmple(hi).all() && a.max(b).max(c).cmpge(lo).all() {
            near.push([a, b, c]);
        }
    };
    match mesh.try_indices_option() {
        Ok(Some(Indices::U32(ix))) => ix.as_chunks::<3>().0.iter().for_each(|t| take(t.map(|i| i as usize))),
        Ok(Some(Indices::U16(ix))) => ix.as_chunks::<3>().0.iter().for_each(|t| take(t.map(usize::from))),
        Ok(None) => (0..positions.len() / 3).for_each(|t| take([3 * t, 3 * t + 1, 3 * t + 2])),
        Err(_) => {}
    }
    near
}

/// **A floor's or a wall's mesh with its canvas laid over it from its broadest side** (see
/// [`Gore::projected`]): a copy whose `UV_0` is the projection and whose `UV_1` is the `UV_0` it had,
/// and the two mesh-space vectors one canvas edge long along the projection's `u` and `v`. The side it
/// faces most is the axis its triangles' areas project onto most; `u` and `v` are `x` and `z` seen from
/// `y`, and the horizontal axis and `−y` seen from `x` or `z`, so `+v` runs down a wall.
fn projected(mesh: &Mesh) -> Result<(Mesh, [Vec3; 2]), &'static str> {
    if mesh.contains_attribute(Mesh::ATTRIBUTE_UV_1) {
        return Err("it already has a UV_1, which may be a lightmap's, and the projection would overwrite it");
    }
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return Err("it has no Float32x3 positions");
    };
    let mut facing = Vec3::ZERO;
    let mut triangle = |[a, b, c]: [usize; 3]| {
        if let (Some(pa), Some(pb), Some(pc)) = (positions.get(a), positions.get(b), positions.get(c)) {
            let (pa, pb, pc) = (Vec3::from(*pa), Vec3::from(*pb), Vec3::from(*pc));
            facing += (pb - pa).cross(pc - pa).abs();
        }
    };
    match mesh.try_indices_option() {
        Ok(Some(Indices::U32(ix))) => ix.as_chunks::<3>().0.iter().for_each(|t| triangle(t.map(|i| i as usize))),
        Ok(Some(Indices::U16(ix))) => ix.as_chunks::<3>().0.iter().for_each(|t| triangle(t.map(usize::from))),
        Ok(None) => (0..positions.len() / 3).for_each(|t| triangle([3 * t, 3 * t + 1, 3 * t + 2])),
        Err(_) => return Err("its indices cannot be read"),
    }
    let (u, v) = if facing.y >= facing.x && facing.y >= facing.z {
        (Vec3::X, Vec3::Z)
    } else if facing.z >= facing.x {
        (Vec3::X, Vec3::NEG_Y)
    } else {
        (Vec3::Z, Vec3::NEG_Y)
    };
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for p in positions {
        let at = Vec2::new(Vec3::from(*p).dot(u), Vec3::from(*p).dot(v));
        lo = lo.min(at);
        hi = hi.max(at);
    }
    let edge = (hi - lo).max_element();
    if !(edge.is_finite() && edge > 0.0) {
        return Err("it has no extent to lay a canvas over");
    }
    let uvs: Vec<[f32; 2]> = positions.iter().map(|p| ((Vec2::new(Vec3::from(*p).dot(u), Vec3::from(*p).dot(v)) - lo) / edge).to_array()).collect();
    let mut copy = mesh.clone();
    if let Ok(Some(own)) = mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) {
        copy.insert_attribute(Mesh::ATTRIBUTE_UV_1, own.clone());
    }
    copy.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    Ok((copy, [u * edge, v * edge]))
}

/// Whether any of a mesh's `UV_0` lies outside `[0, 1]` by more than a rounding error: a UV layout
/// that tiles a texture rather than covering the mesh once.
fn tiles(mesh: &Mesh) -> bool {
    match mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) {
        Ok(Some(VertexAttributeValues::Float32x2(uvs))) => uvs.iter().flatten().any(|c| !(-1.0e-3..=1.0 + 1.0e-3).contains(c)),
        _ => false,
    }
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

/// **The message a hit that opened a wound announces**: where, which way it faces, how wide, and what
/// kind of opening. One function for the wound that bleeds and the one a tier draws no blood for, so
/// the second is the first with nothing painted, never a near copy of it.
fn announce(hit: &GoreHit, at: Vec3, normal: Vec3) -> Wounded {
    Wounded {
        at,
        normal,
        area: opened_area(hit),
        severity: 1.0,
        kind: if hit.depth_mm() > 0.0 { WoundKind::Channel } else { WoundKind::Severance },
        class: if hit.so2 >= crate::bloodstain::spectral::SO2_ARTERIAL {
            crate::bloodstain::PatternClass::ArterialSpurt
        } else {
            crate::bloodstain::PatternClass::Impact
        },
    }
}

/// **How much skin a hit opened**, m²: a disc its radius across, or for a slash a spindle as long as
/// the blade and [`SLASH_OPEN_M`] wide at the middle, `π/4 · length · width`, the ellipse it is near.
/// Until 0.5.34 a slash was a disc its own length across, 23 times the open skin of a 14 cm cut, and it
/// bled and announced that much.
fn opened_area(hit: &GoreHit) -> f32 {
    match hit.kind {
        HitKind::Slash { .. } => core::f32::consts::FRAC_PI_4 * 2.0 * hit.radius_m * SLASH_OPEN_M,
        _ => core::f32::consts::PI * hit.radius_m * hit.radius_m,
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

/// **A surface's bounds, for the spray's broad phase**: the axis-aligned box of its vertices in the
/// mesh's own space, and the map into it from world space. A ray is moved into the box's space, not
/// the box into the ray's, the way every cast in this crate moves the ray and not the geometry.
#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
    to_local: bevy::math::Affine3A,
    /// The padded box as world space holds it, axis-aligned there: a slab test against it needs no
    /// transform, and a ray that misses it misses the box.
    world_min: Vec3,
    world_max: Vec3,
}

impl Bounds {
    /// The box of `mesh`'s vertices as `xf` places it. `None` for a mesh with no positions, or a
    /// transform that cannot be inverted: nothing is culled against those, so a cast still happens
    /// and still says why it cannot work.
    fn of(mesh: &Mesh, xf: &GlobalTransform) -> Option<Self> {
        let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
            return None;
        };
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for p in positions {
            let p = Vec3::from(*p);
            if p.is_finite() {
                min = min.min(p);
                max = max.max(p);
            }
        }
        // A box must never be a hair smaller than the triangles in it, or a cull loses blood.
        let pad = Vec3::splat(1.0e-4) + (max - min) * 1.0e-4;
        let to_local = xf.affine().inverse();
        let (min, max) = (min - pad, max + pad);
        // The world box round the placed one's corners, padded again for the corners' own rounding.
        let (mut world_min, mut world_max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for k in 0..8 {
            let corner = Vec3::new(if k & 1 == 0 { min.x } else { max.x }, if k & 2 == 0 { min.y } else { max.y }, if k & 4 == 0 { min.z } else { max.z });
            let w = xf.affine().transform_point3(corner);
            world_min = world_min.min(w);
            world_max = world_max.max(w);
        }
        let world_pad = Vec3::splat(1.0e-4) + (world_max - world_min) * 1.0e-4 + world_min.abs().max(world_max.abs()) * 1.0e-6;
        let (world_min, world_max) = (world_min - world_pad, world_max + world_pad);
        (min.is_finite() && max.is_finite() && to_local.is_finite() && world_min.is_finite() && world_max.is_finite())
            .then_some(Self { min, max, to_local, world_min, world_max })
    }

    /// Whether the ray from `from` along `dir` can reach the world box ahead of its origin. `false` is
    /// exact — the placed box lies inside it — and `true` is "test the placed box". A ray that cannot
    /// be tested passes.
    fn may_reach(&self, from: Vec3, dir: Vec3) -> bool {
        let (mut near, mut far) = (0.0f32, f32::INFINITY);
        for axis in 0..3 {
            let (o, d, lo, hi) = (from[axis], dir[axis], self.world_min[axis], self.world_max[axis]);
            if !(o.is_finite() && d.is_finite()) {
                return true;
            }
            if d == 0.0 {
                if o < lo || o > hi {
                    return false;
                }
                continue;
            }
            let (a, b) = ((lo - o) / d, (hi - o) / d);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return false;
            }
        }
        true
    }

    /// **Whether the ray from `from` along `dir` can enter the box ahead of its origin**: the slab
    /// test. Conservative: `true` is "cast at it", never "it lands", and a ray that cannot be tested
    /// (not finite) is passed to the cast, which refuses it.
    fn reached_by(&self, from: Vec3, dir: Vec3) -> bool {
        let origin = self.to_local.transform_point3(from);
        let d = self.to_local.transform_vector3(dir);
        if !(origin.is_finite() && d.is_finite()) {
            return true;
        }
        self.span(from, dir).is_some()
    }

    /// **How far along the ray from `from` along `dir` it enters the box**, in multiples of `dir`,
    /// `0` from inside: a landing on the mesh inside is never nearer, since the box is padded past its
    /// triangles. `Some(0.0)` for a ray that cannot be tested, which is cast at as [`reached_by`]
    /// casts it; `None` when the ray never enters.
    ///
    /// [`reached_by`]: Self::reached_by
    fn entry(&self, from: Vec3, dir: Vec3) -> Option<f32> {
        if !self.may_reach(from, dir) {
            return None;
        }
        let origin = self.to_local.transform_point3(from);
        let d = self.to_local.transform_vector3(dir);
        if !(origin.is_finite() && d.is_finite()) {
            return Some(0.0);
        }
        self.local_span(origin, d).map(|(enters, _)| enters)
    }

    /// **Where the ray from `from` along `dir` is inside the box**, as `(enters, leaves)` in multiples
    /// of `dir`, the entry clamped to the origin. The map into the box's space is affine, so these are
    /// the world ray's own parameters. `None` when it never is, or cannot be tested.
    fn span(&self, from: Vec3, dir: Vec3) -> Option<(f32, f32)> {
        let origin = self.to_local.transform_point3(from);
        let d = self.to_local.transform_vector3(dir);
        if !(origin.is_finite() && d.is_finite()) {
            return None;
        }
        self.local_span(origin, d)
    }

    /// [`span`](Self::span) for a ray already in the box's space.
    fn local_span(&self, origin: Vec3, d: Vec3) -> Option<(f32, f32)> {
        let (mut near, mut far) = (0.0f32, f32::INFINITY);
        for axis in 0..3 {
            if d[axis].abs() < 1.0e-12 {
                if origin[axis] < self.min[axis] || origin[axis] > self.max[axis] {
                    return None;
                }
            } else {
                let (a, b) = ((self.min[axis] - origin[axis]) / d[axis], (self.max[axis] - origin[axis]) / d[axis]);
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if near > far {
                    return None;
                }
            }
        }
        Some((near, far))
    }
}

/// **Where a ray leaves a mesh**: its last crossing, found as the first crossing of the same line
/// cast back from just past the mesh's box. `t` is in the forward ray's measure, so it compares with
/// a [`ray_hit`] on the same ray. `None` when the ray never reaches the mesh.
fn ray_exit(mesh: &Mesh, index: Option<crate::bvh::Fitted<'_>>, xf: &GlobalTransform, from: Vec3, dir: Vec3) -> Option<Landing> {
    let (_, leaves) = Bounds::of(mesh, xf)?.span(from, dir)?;
    let beyond = leaves + 1.0e-3 * (1.0 + leaves.abs());
    if !beyond.is_finite() {
        return None;
    }
    let back = ray_cast(mesh, index, xf, from + dir * beyond, -dir)?;
    Some(Landing { t: beyond - back.t, ..back })
}

/// **What to cast at for one dressed surface**: its asset placed by its own transform, or, for a
/// skinned one, the pose it is drawn in, cast with the identity. A pose is built once per batch and
/// kept in `posed`. `None` when the pose cannot be had: a skin is never cast at its bind pose.
fn cast_at<'a>(
    surface: &'a Surface,
    asset: &'a Mesh,
    rigging: &Rigging,
    posed: &'a mut std::collections::HashMap<Entity, Option<Mesh>>,
    identity: &'a GlobalTransform,
) -> Option<(&'a Mesh, &'a GlobalTransform)> {
    match surface.skin.as_ref().filter(|_| Rig::of(asset).is_some()) {
        None => Some((asset, &surface.xf)),
        Some(skin) => posed
            .entry(surface.entity)
            .or_insert_with(|| rigging.pose(skin).and_then(|pose| skin::posed_mesh(asset, &pose)))
            .as_mut()
            .map(|body| (&*body, identity)),
    }
}

/// Whether `entity` is `root` or one of its descendants.
fn is_under(root: Entity, entity: Entity, parents: &Query<&ChildOf>) -> bool {
    let mut at = entity;
    loop {
        if at == root {
            return true;
        }
        match parents.get(at) {
            Ok(parent) => at = parent.parent(),
            Err(_) => return false,
        }
    }
}

/// **The skin a ray reaches first under a character**: the nearest landing among the [`GoreKind::Skin`]
/// surfaces below `hit.entity`, as each is drawn now. Clothing is not a candidate, because it is
/// not what a wound opens on: the spray soaks it from the wound. Ties go to the lower entity, so the
/// answer does not depend on the order a query happened to yield them. `None` when the ray reaches
/// none of them.
#[allow(clippy::too_many_arguments)]
fn nearest_skin(
    hit: &GoreHit,
    surfaces: &[Surface],
    parents: &Query<&ChildOf>,
    meshes: &Assets<Mesh>,
    rigging: &Rigging,
    posed: &mut std::collections::HashMap<Entity, Option<Mesh>>,
    bounds: &mut std::collections::HashMap<Entity, Reach>,
    identity: &GlobalTransform,
    index: &MeshIndex,
    refits: &mut std::collections::HashMap<Entity, crate::bvh::Refit>,
) -> Option<Entity> {
    let mut best: Option<(f32, Entity)> = None;
    for surface in surfaces {
        if !matches!(surface.gore.kind, GoreKind::Skin(_)) || !is_under(hit.entity, surface.entity, parents) {
            continue;
        }
        let Some(asset) = meshes.get(&surface.mesh) else { continue };
        let reach = *bounds.entry(surface.entity).or_insert_with(|| reach_of(surface, asset, rigging, posed, identity, index));
        match reach {
            Reach::Never => continue,
            Reach::Within(b) if !b.reached_by(hit.from, hit.dir) => continue,
            _ => {}
        }
        // Posed only now that the ray can reach it.
        let Some((mesh, xf)) = cast_at(surface, asset, rigging, posed, identity) else { continue };
        let Some(landing) = ray_cast(mesh, cast_index(index, refits, surface, asset, mesh), xf, hit.from, hit.dir) else { continue };
        if best.is_none_or(|(t, entity)| landing.t < t || (landing.t == t && surface.entity < entity)) {
            best = Some((landing.t, surface.entity));
        }
    }
    best.map(|(_, entity)| entity)
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
        Option<&'static RunDirection>,
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
    /// **A slash's line on the wetmap**, so each pulse wells over the cut's lips along its length;
    /// `None` for a wound that is a hole.
    pub cut: Option<CutLine>,
}

impl WoundSite {
    /// **Where well `k` of this wound beads, from its centre on the wetmap, and which way is out**:
    /// along a cut's lower lip ([`CutLine::well`]), or on a hole's rim, the wells fanned over the arc
    /// that faces `downhill`, where the blood that fills a crater spills over. A level hole spills all
    /// round. The rim is the wound's own radius at `density` UV per metre.
    fn well(&self, k: u32, downhill: Vec2, density: f32) -> (Vec2, Vec2) {
        if let Some(cut) = self.cut {
            return cut.well(self.bleed.seed, k, downhill);
        }
        let h = |salt: u32| crate::bloodstain::hash_f32(self.bleed.seed ^ k.wrapping_mul(0x85EB_CA6B) ^ salt);
        let n = BLEED_WELLS.max(1) as f32;
        let rim = (self.wound.area / core::f32::consts::PI).max(0.0).sqrt() * density;
        let down = downhill.normalize_or_zero();
        let (axis, fan) = if down == Vec2::ZERO { (Vec2::Y, core::f32::consts::PI) } else { (down, RIM_FAN) };
        // Stratified over the fan, each a little off its share.
        let angle = ((k as f32 + 0.25 + 0.5 * h(0x51)) / n * 2.0 - 1.0) * fan;
        let (s, c) = (libm::sinf(angle), libm::cosf(angle));
        let out = Vec2::new(c * axis.x - s * axis.y, s * axis.x + c * axis.y);
        (out * rim, out)
    }
}

/// **How far either side of downhill a hole's wells fan**, radians: 50°. Blood that fills a crater
/// spills over the lower part of its rim, not the sides. This crate's own number.
const RIM_FAN: f32 = 0.87;

/// **How wide a slash opens, in metres on the body**: the lips stand this far apart at the middle of
/// the cut once it has finished opening. 6 mm, which is what `Gape::default()` under
/// `Tension::default()` opened at metre scale before 0.5.34 and still does; this crate's own number, as
/// `Gape::default` is. Stated in metres now so the wound's area and its gape agree on any mesh scale.
pub const SLASH_OPEN_M: f32 = 0.006;

/// **How many places each pulse of a wound wells at**: along a cut's lower lip, or round the downhill
/// rim of a hole. The pulse's blood is shared between them, so a wound bleeds as a few runs down the
/// skin rather than one column its own width. This crate's own number.
pub const BLEED_WELLS: u32 = 3;

/// **A slash's line on a wetmap**: where along the cut, and how far out its lips are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutLine {
    /// Half the cut, from its middle to one tip, as a change of `UV_0`.
    pub half_uv: Vec2,
    /// Half the open width at the middle, in UV: how far either lip stands from the line there. It
    /// closes toward both tips as the laceration does, by `laceration::GAPE_TAPER`.
    pub lip_uv: f32,
}

impl CutLine {
    /// **Where well `k` of a cut beads, and which way is out from the cut there**: a place along the
    /// cut, the wells spread from end to end and each a little off its share by `seed`, on the lip
    /// `downhill` (UV) points across. A level cut, with no downhill, beads on both lips by turns.
    ///
    /// **The wells stay put for the wound's life.** The wetmap leaves its drip threshold behind in
    /// every texel a run crosses, so a run is only long when it is fed over the track the last pulse
    /// left wet; a bead somewhere new each pulse spends itself wetting new skin.
    fn well(&self, seed: u32, k: u32, downhill: Vec2) -> (Vec2, Vec2) {
        let h = |salt: u32| crate::bloodstain::hash_f32(seed ^ k.wrapping_mul(0x85EB_CA6B) ^ salt);
        let n = BLEED_WELLS.max(1) as f32;
        // Stratified over `[-0.75, 0.75]`, kept off the tips, where the cut has closed.
        let along = ((k as f32 + 0.25 + 0.5 * h(0x51)) / n * 2.0 - 1.0) * 0.75;
        let across = self.half_uv.normalize_or_zero().perp();
        let lean = across.dot(downhill);
        let side = if lean > 0.0 {
            1.0
        } else if lean < 0.0 || k % 2 == 1 {
            -1.0
        } else {
            1.0
        };
        let open = libm::powf(libm::cosf(core::f32::consts::FRAC_PI_2 * along).max(0.0), crate::laceration::GAPE_TAPER);
        (self.half_uv * along + across * (self.lip_uv * open * side), across * side)
    }
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
///
/// **Bruises are redrawn, not repainted over themselves.** A bruise changes every model step, and
/// several can overlap, so the canvas keeps what it holds apart from its bruises (`under`, made when the
/// first is drawn) and [`DermisCanvas::draw_bruises`] rebuilds the bruises over it each time. Painted
/// one over another, a bruise's faint margin was blended into its neighbour's core at every step and
/// bleached a pale seam between them.
#[derive(Component, Debug)]
pub struct DermisCanvas {
    size: u32,
    px: Vec<u8>,
    /// The canvas without its bruises — burns and anything else painted once — from the first bruise
    /// on; `None` until then.
    under: Option<Vec<u8>>,
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
        Self { size, px: [255, 255, 255, 0].repeat((size as usize) * (size as usize)), under: None, image: images.add(image), dirty: false }
    }

    /// The image the flesh material reads.
    pub fn image(&self) -> Handle<Image> {
        self.image.clone()
    }

    /// Edge length, texels.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// The bytes this canvas holds in the main world apart from its image: the CPU copy of it, and of
    /// it without its bruises once it has any.
    pub fn heap_bytes(&self) -> usize {
        self.px.capacity() + self.under.as_ref().map_or(0, Vec::capacity)
    }

    /// The size of the image it uploads, once; the main world and the GPU each hold a copy.
    pub fn image_bytes(&self) -> usize {
        self.px.len()
    }

    /// Paint a radial field: `colour(r_mm, dir)` returns the linear ratio the skin is multiplied by and
    /// an alpha, for the texel `r_mm` from `uv` in the unit direction `dir` (in UV, zero at the centre),
    /// out to `radius_uv`. Alpha is taken as the max of what was there, so two injuries overlap rather
    /// than erase.
    pub fn paint_radial(&mut self, uv: Vec2, radius_uv: f32, mm_per_uv: f32, mut colour: impl FnMut(f32, Vec2) -> ([f32; 3], f32)) {
        let size = self.size;
        let mut blend = |px: &mut [u8], c: [f32; 3], a: f32| {
            let old_a = px[3] as f32 / 255.0;
            let w = if a + old_a > 0.0 { a / (a + old_a) } else { 0.0 };
            for ch in 0..3 {
                let new = c[ch].clamp(0.0, 1.0);
                let old = px[ch] as f32 / 255.0;
                px[ch] = ((old + (new - old) * w) * 255.0).round() as u8;
            }
            px[3] = (a.max(old_a) * 255.0).round() as u8;
        };
        // A mark painted once belongs under the bruises too, or the next redraw would take it away.
        if let Some(under) = self.under.as_mut() {
            radial(under, size, uv, radius_uv, mm_per_uv, &mut colour, &mut blend);
        }
        self.dirty |= radial(&mut self.px, size, uv, radius_uv, mm_per_uv, &mut colour, &mut blend);
    }

    /// **Redraw every bruise** over what the canvas holds without them: bruise `i` centred at
    /// `centres[i]`, out to `radius_uv`, its field `colour(i, r_mm, dir)` as
    /// [`paint_radial`](Self::paint_radial) takes one, composed by multiplying the ratios they lay over
    /// the skin, `1 − a + a·c` per channel, so two that overlap darken it as their chromophores would
    /// together and neither bleaches the other. Alpha is their union.
    pub fn draw_bruises(&mut self, centres: &[Vec2], radius_uv: f32, mm_per_uv: f32, mut colour: impl FnMut(usize, f32, Vec2) -> ([f32; 3], f32)) {
        let under = self.under.get_or_insert_with(|| self.px.clone());
        self.px.copy_from_slice(under);
        let mut multiply = |px: &mut [u8], c: [f32; 3], a: f32| {
            let old_a = px[3] as f32 / 255.0;
            let out_a = 1.0 - (1.0 - old_a) * (1.0 - a);
            for ch in 0..3 {
                let old_m = 1.0 - old_a + old_a * px[ch] as f32 / 255.0;
                let new_m = 1.0 - a + a * c[ch].clamp(0.0, 1.0);
                let rgb = if out_a > 0.0 { 1.0 - (1.0 - old_m * new_m) / out_a } else { 1.0 };
                px[ch] = (rgb.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            px[3] = (out_a * 255.0).round() as u8;
        };
        for (i, uv) in centres.iter().enumerate() {
            radial(&mut self.px, self.size, *uv, radius_uv, mm_per_uv, |r_mm, dir| colour(i, r_mm, dir), &mut multiply);
        }
        self.dirty = true;
    }

    /// **Fade every bruise and burn**, keeping the canvas and its image: the bytes go back to a
    /// fresh canvas's, re-uploaded on the next flush.
    pub fn clear(&mut self) {
        self.px.as_chunks_mut::<4>().0.fill([255, 255, 255, 0]);
        self.under = None;
        self.dirty = true;
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

/// **Visit every texel of a `size`² RGBA8 buffer within `radius_uv` of `uv`** with the field
/// `colour(r_mm, dir)` gives there, handing `put` the texel and the colour and alpha, when the alpha is
/// above zero. Whether anything was put.
#[allow(clippy::too_many_arguments)]
fn radial(
    px: &mut [u8],
    size: u32,
    uv: Vec2,
    radius_uv: f32,
    mm_per_uv: f32,
    mut colour: impl FnMut(f32, Vec2) -> ([f32; 3], f32),
    mut put: impl FnMut(&mut [u8], [f32; 3], f32),
) -> bool {
    if !uv.is_finite() || !radius_uv.is_finite() || radius_uv <= 0.0 {
        return false;
    }
    let n = size as i64;
    // Saturated casts, then clamped: a huge radius or an off-atlas UV from a caller's mesh must not
    // overflow the bounds arithmetic below.
    let r_px = ((radius_uv * size as f32).ceil() as i64).clamp(0, n);
    let cx = ((uv.x * size as f32) as i64).clamp(-n, 2 * n);
    let cy = ((uv.y * size as f32) as i64).clamp(-n, 2 * n);
    let mut any = false;
    for y in (cy - r_px).max(0)..(cy + r_px + 1).min(n) {
        for x in (cx - r_px).max(0)..(cx + r_px + 1).min(n) {
            let du = (x as f32 + 0.5) / size as f32 - uv.x;
            let dv = (y as f32 + 0.5) / size as f32 - uv.y;
            let r_uv = (du * du + dv * dv).sqrt();
            if r_uv > radius_uv {
                continue;
            }
            let (c, a) = colour(r_uv * mm_per_uv, Vec2::new(du, dv).normalize_or_zero());
            let a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 };
            if a <= 0.0 {
                continue;
            }
            let i = ((y * n + x) * 4) as usize;
            if let Some(texel) = px.get_mut(i..i + 4) {
                put(texel, c, a);
                any = true;
            }
        }
    }
    any
}

/// **The bruises ageing under this entity's skin**, each at its own spot, oldest first. The kernel is
/// `crate::bloodstain::bruise`; the preset steps every one on the fixed tick and repaints the dermis
/// canvas where its colour moves. Until 0.5.55 an entity held one, and a second blow started a new
/// bruise while the first stopped ageing where it was, so a limb hit twice kept one bruise frozen
/// beside one that darkened.
#[derive(Component, Debug, Default)]
pub struct Bruising {
    /// Every bruise still ageing, oldest first, at most [`MAX_BRUISES`].
    pub sites: Vec<BruiseSite>,
}

/// **One bruise**: where on the canvas, the kernel, and the fractional model steps carried between
/// ticks.
#[derive(Debug)]
pub struct BruiseSite {
    /// Where on the canvas.
    pub uv: Vec2,
    /// The kernel.
    pub bruise: crate::bloodstain::Bruise,
    /// Fractional model steps carried between ticks.
    pub carry: f32,
}

/// **How many bruises one entity ages at once**: 8. A blow past that retires the oldest, which keeps
/// the colour it had reached. A bound, not a model: each bruise is a kernel stepped and a patch of the
/// canvas repainted every model step, and a limb beaten more than eight times has few texels left
/// that are not already bruised.
pub const MAX_BRUISES: usize = 8;

/// **Blood soaking into this cloth** from one landing: the Lucas–Washburn front (`crate::bloodstain::wick`)
/// widens the stain over the seconds after it lands.
#[derive(Component, Debug, Default)]
pub struct Soaking {
    /// `(UV it landed at, silhouette in metres, tick it landed)` per landing still spreading.
    pub landings: Vec<(Vec2, StainShape, u32)>,
}

/// **The share of a film's depth that blood soaked into cloth stands for**: `0.1`. A weave holds blood
/// inside it, so full coverage on a shirt is a far thinner layer over the fabric than on skin or a
/// floor. Whole blood is opaque by about 0.6 mm (`crate::flesh::BLOOD_LUT_MAX_MM`'s note), so at the
/// shared 2 mm a soak was near-black to its edge; at a tenth it is red where it is heavy and pink at
/// its front. This crate's own number.
pub const CLOTH_FILM_SHARE: f32 = 0.1;

/// **The holes shot through this cloth**, so the wound under a shirt can be seen through it. An
/// impact whose ray crosses a cloth before it reaches the skin it wounds opens a hole where it
/// crossed: the cloth is cut away out to about [`HOLE_OPEN`] of the blow's radius, along a ragged,
/// seeded outline, and darkened in a ring around that out to [`HOLE_RIM`] of it — what forensic
/// pathology calls the *bullet wipe*, the grey ring a projectile leaves on the fabric it pierces. The
/// ring's colour and both reaches are this crate's own.
///
/// The pixels are a [`DermisCanvas`] in the flesh material's dermis slot, read the other way round on
/// cloth: `rgb` the ratio the fabric is multiplied by, `a` above one half where the cloth is gone.
/// The material is switched to [`AlphaMode::Mask`] by the first hole and not before, so an unholed
/// cloth keeps an opaque pipeline's early depth test; an authored `Blend` cloth is left as it was.
/// A hole is never closed.
#[derive(Component, Debug)]
pub struct ClothHoles {
    canvas: DermisCanvas,
    /// Whether the material has been switched to a mask yet.
    masked: bool,
}

/// How far a hole is open, as a share of the blow's radius: enough to see the crater, not the whole
/// of its falloff. This crate's own number.
pub const HOLE_OPEN: f32 = 0.85;
/// How far the darkened ring around a hole reaches, as a share of the blow's radius. Own number.
pub const HOLE_RIM: f32 = 1.5;
/// The ratio the fabric is multiplied by at the hole's edge. Own number.
const HOLE_WIPE: [f32; 3] = [0.32, 0.29, 0.27];
/// **How wide a blade's slit in cloth opens at its middle**, as a share of its half-length: `0.3`. A
/// cut weave is held under tension by the garment and parts; this crate's own number, set so the slit
/// shows the laceration under it. At `0.18` it was a hairline a pixel or two wide at the demo's framing.
pub const SLIT_WIDTH: f32 = 0.3;
/// The ratio the fabric is multiplied by along a slit's frayed edge: its own shadow, no soot. Own number.
const SLIT_FRAY: [f32; 3] = [0.62, 0.60, 0.58];

impl ClothHoles {
    fn new(images: &mut Assets<Image>, size: u32) -> Self {
        Self { canvas: DermisCanvas::new(images, size), masked: false }
    }

    /// The canvas the holes are drawn in.
    pub fn canvas(&self) -> &DermisCanvas {
        &self.canvas
    }

    /// FNV-1a over the canvas's bytes: two runs that opened the same holes print the same number.
    pub fn digest(&self) -> u64 {
        self.canvas.px.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
    }

    /// **Mend every hole and slit**, keeping the canvas and its image. The material stays as the
    /// first hole left it, a mask with nothing cut, which draws the cloth whole.
    pub fn clear(&mut self) {
        self.canvas.clear();
    }

    /// Whether any texel is open.
    pub fn is_holed(&self) -> bool {
        self.canvas.px.iter().skip(3).step_by(4).any(|&a| a > 127)
    }

    /// Whether the texel under `uv` is open.
    pub fn is_open_at(&self, uv: Vec2) -> bool {
        let n = self.canvas.size as f32;
        if !uv.is_finite() || uv.x < 0.0 || uv.y < 0.0 || uv.x >= 1.0 || uv.y >= 1.0 {
            return false;
        }
        let (x, y) = ((uv.x * n) as usize, (uv.y * n) as usize);
        let i = (y * self.canvas.size as usize + x) * 4 + 3;
        self.canvas.px.get(i).is_some_and(|&a| a > 127)
    }

    /// **Open a hole** of `radius_uv` (the blow's radius, in this canvas's UV) at `uv`. `seed` turns
    /// the outline: three and seven lobes at seeded phases, and a seeded jitter per texel, so no two
    /// holes are the same circle and the edge reads as torn fibre rather than a punched disc.
    pub fn open(&mut self, uv: Vec2, radius_uv: f32, seed: u32) {
        if !uv.is_finite() || !radius_uv.is_finite() || radius_uv <= 0.0 {
            return;
        }
        let size = self.canvas.size;
        let n = size as i64;
        let reach = radius_uv * HOLE_RIM;
        let r_px = ((reach * size as f32).ceil() as i64).clamp(0, n);
        let cx = ((uv.x * size as f32) as i64).clamp(-n, 2 * n);
        let cy = ((uv.y * size as f32) as i64).clamp(-n, 2 * n);
        let tau = core::f32::consts::TAU;
        let p3 = crate::bloodstain::hash_f32(seed) * tau;
        let p7 = crate::bloodstain::hash_f32(seed ^ 0x51ED_270B) * tau;
        for y in (cy - r_px).max(0)..(cy + r_px + 1).min(n) {
            for x in (cx - r_px).max(0)..(cx + r_px + 1).min(n) {
                let du = (x as f32 + 0.5) / size as f32 - uv.x;
                let dv = (y as f32 + 0.5) / size as f32 - uv.y;
                let r = libm::sqrtf(du * du + dv * dv);
                if r > reach {
                    continue;
                }
                let theta = libm::atan2f(dv, du);
                let jitter = crate::bloodstain::hash_f32(seed ^ ((y as u32).wrapping_mul(0x9E37_79B9) ^ (x as u32).wrapping_mul(0x85EB_CA6B)));
                let lobes = 1.0 + 0.16 * libm::sinf(3.0 * theta + p3) + 0.09 * libm::sinf(7.0 * theta + p7);
                let edge = radius_uv * HOLE_OPEN * lobes * (0.9 + 0.2 * jitter);
                // Open inside the edge; outside it a ring that fades to nothing at the reach, at
                // most a little under one half so the cut is decided by the edge alone.
                let a = if r <= edge {
                    1.0
                } else {
                    let s = ((r - edge) / (reach - edge).max(1.0e-6)).clamp(0.0, 1.0);
                    0.45 * (1.0 - s) * (1.0 - s)
                };
                if a <= 0.0 {
                    continue;
                }
                self.mark(x, y, a, &HOLE_WIPE);
            }
        }
    }

    /// **Cut a slit**, a blade's, centred on `uv` and running `half_uv` either way from it, in this
    /// canvas's UV. It is a lens: [`SLIT_WIDTH`] of its half-length wide at the middle and closing to
    /// nothing at the ends, its edges wavering along the cut and frayed per texel by `seed`. A blade
    /// leaves no wipe, so the ring around it is the fabric's own fraying, paler than a bullet's.
    pub fn slit(&mut self, uv: Vec2, half_uv: Vec2, seed: u32) {
        let len = half_uv.length();
        if !uv.is_finite() || !half_uv.is_finite() || len <= 0.0 {
            return;
        }
        let dir = half_uv / len;
        let width = len * SLIT_WIDTH;
        let size = self.canvas.size;
        let n = size as i64;
        let pad = width * 2.0;
        let (lo, hi) = (uv - half_uv.abs() - Vec2::splat(pad), uv + half_uv.abs() + Vec2::splat(pad));
        let (x0, y0) = (((lo.x * size as f32).floor() as i64).clamp(0, n), ((lo.y * size as f32).floor() as i64).clamp(0, n));
        let (x1, y1) = (((hi.x * size as f32).ceil() as i64).clamp(0, n), ((hi.y * size as f32).ceil() as i64).clamp(0, n));
        let phase = crate::bloodstain::hash_f32(seed ^ 0x2C1B_3C6D) * core::f32::consts::TAU;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = Vec2::new((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32) - uv;
                let t = (p.dot(dir) / len).clamp(-1.0, 1.0);
                let off = (p - dir * (t * len)).length();
                let jitter = crate::bloodstain::hash_f32(seed ^ ((y as u32).wrapping_mul(0x9E37_79B9) ^ (x as u32).wrapping_mul(0x85EB_CA6B)));
                let waver = 1.0 + 0.2 * libm::sinf(5.0 * core::f32::consts::PI * t + phase);
                let edge = width * libm::sqrtf(1.0 - t * t) * waver * (0.8 + 0.4 * jitter);
                let fray = edge + width * 0.8;
                let a = if off <= edge {
                    1.0
                } else if off < fray {
                    let s = (off - edge) / (fray - edge);
                    0.35 * (1.0 - s) * (1.0 - s)
                } else {
                    0.0
                };
                self.mark(x, y, a, &SLIT_FRAY);
            }
        }
    }

    /// Raise texel `(x, y)` to opening `a`, darkening it toward `tint`; a texel already as open is
    /// left alone, so cuts overlap rather than heal one another.
    fn mark(&mut self, x: i64, y: i64, a: f32, tint: &[f32; 3]) {
        if a <= 0.0 {
            return;
        }
        let n = self.canvas.size as i64;
        let i = ((y * n + x) * 4) as usize;
        if let Some(px) = self.canvas.px.get_mut(i..i + 4) {
            let old_a = px[3] as f32 / 255.0;
            if a <= old_a {
                return;
            }
            for (ch, t) in tint.iter().enumerate() {
                px[ch] = px[ch].min((t * 255.0).round() as u8);
            }
            px[3] = (a * 255.0).round() as u8;
            self.canvas.dirty = true;
        }
    }
}

/// A thrown fragment the preset integrates.
#[derive(Component, Clone, Copy, Debug)]
pub struct Flying(pub Vec3);

/// A piece of a broken body that stayed where it stood — the island the modes did not part. Marked
/// so a caller resetting a scene can find it; the body it came from is gone.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Standing;

/// **A broken body's cut face, still bleeding.** The preset spawns every piece with a cut face with
/// one: its wound is the face — its area, its centre, the way it faces — opened on the tick the body
/// broke, and each pulse of it pours onto the floor beneath it until the perfusion runs out, so a heap
/// of pieces lies in its own pool. Before 0.5.37 a body that burst left its floor as clean as it was.
///
/// A game that spawns the pieces itself (`GoreDials::spawn_fragments` off) keeps the pours by
/// inserting [`CutFaceBleed::of`] on each piece's cut face, with the piece's `Transform`.
#[derive(Component, Clone, Copy, Debug)]
pub struct CutFaceBleed {
    /// The face's centre, in the piece's own space, so the pour follows the piece as it falls.
    pub centre_local: Vec3,
    /// The face as a wound, where it was when the body broke: the pulse schedule's seed and area.
    pub wound: BloodWound,
    /// The schedule.
    pub bleed: crate::bloodstain::Bleed,
}

impl CutFaceBleed {
    /// **A cut face that opened on `tick`**, from its mesh and where the piece is. `None` for a face
    /// with no area.
    pub fn of(cap: &Mesh, place: &Transform, tick: u32) -> Option<Self> {
        let (area, centre_local, normal) = face_measure(cap)?;
        let scale = place.scale.abs().max_element();
        let wound = BloodWound {
            at: place.transform_point(centre_local).to_array(),
            normal: (place.rotation * normal).to_array(),
            area: area * scale * scale,
            severity: 1.0,
            kind: WoundKind::Severance,
        };
        Some(Self { centre_local, wound, bleed: crate::bloodstain::Bleed::new(tick, &wound) })
    }
}

/// **How much of a cut face's pulse pours onto the floor**, as a share of the stain a wound of its
/// area paints a pulse at. A face is all wound and nothing holds it shut, and a body that burst lies in
/// most of its blood: at `0.5` each pour was a stain under a fifth of a metre across, hidden under the
/// piece it came from. This crate's own number.
pub const CUT_FACE_POUR: f32 = 1.5;

/// **A mesh's area, centre and facing**, in its own space: the triangles' total area, their
/// area-weighted centre and the unit sum of their normals. `None` for a mesh with no area.
fn face_measure(mesh: &Mesh) -> Option<(f32, Vec3, Vec3)> {
    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let (mut area, mut centre, mut normal) = (0.0f32, Vec3::ZERO, Vec3::ZERO);
    let mut add = |t: [usize; 3]| {
        let [Some(a), Some(b), Some(c)] = t.map(|i| positions.get(i).copied().map(Vec3::from)) else { return };
        let n = (b - a).cross(c - a);
        let s = 0.5 * n.length();
        if s.is_finite() {
            area += s;
            centre += (a + b + c) / 3.0 * s;
            normal += n;
        }
    };
    match mesh.try_indices_option() {
        Ok(Some(Indices::U32(ix))) => ix.as_chunks::<3>().0.iter().for_each(|t| add(t.map(|i| i as usize))),
        Ok(Some(Indices::U16(ix))) => ix.as_chunks::<3>().0.iter().for_each(|t| add(t.map(usize::from))),
        Ok(None) => (0..positions.len() / 3).for_each(|t| add([3 * t, 3 * t + 1, 3 * t + 2])),
        Err(_) => return None,
    }
    (area > 0.0 && area.is_finite()).then(|| (area, centre / area, normal.normalize_or_zero()))
}

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
    /// **How much blood a weave holds before it runs**, as a share of full coverage: the drip
    /// threshold a [`Gore::cloth`] canvas ticks with, in place of `WetSettings::drip_rate`
    /// when that is lower. Cloth soaks blood into its yarns; it runs down a shirt only once the
    /// fabric is saturated, where skin sheds it as a film. At the shared `0.35` a soaked shirt grew
    /// a straight, hard-edged column under every wound. `0.92`, this crate's own number.
    pub cloth_holds: f32,
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
            cloth_holds: 0.92,
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

/// How far outside a contact point [`GoreHit::at_surface`] starts its ray, metres: clear of a small
/// gap between a physics shape and the mesh, near enough that nothing else is likely to be between.
pub const SURFACE_STAND_OFF: f32 = 0.05;

/// **How much wider an exit wound is than the entry**, for [`GoreHit::through`]. That an exit is
/// larger and more irregular than the entrance — the projectile arrives tumbling or deformed, pushing
/// tissue ahead of it, with no abrasion collar — is a forensic commonplace (DiMaio, *Gunshot Wounds*);
/// the factor `1.6` is this crate's own.
pub const EXIT_SCALE: f32 = 1.6;

/// **A hit, addressed to a dressed entity or to a character.**
///
/// **Address it to whatever your physics found.** A game's raycast finds a collider, a capsule on the
/// character or a box on its head, rarely the mesh that was struck. Name any entity with dressed
/// skins below it and the preset casts the ray at each of them, as it is drawn now, and hurts the
/// nearest. Clothing is not a candidate, since the spray soaks it from the wound, and a ray that
/// reaches no skin is dropped. A mesh with its own [`Gore`] is hurt directly, as it always was.
#[derive(Message, Clone, Copy, Debug)]
pub struct GoreHit {
    /// The entity to hurt: a mesh with a [`Gore`] and a `Mesh3d`, or an ancestor of meshes that have
    /// one, in which case the nearest [`Gore::skin`] under it along the ray is hurt.
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
    /// **Whether the projectile leaves the body** — see [`GoreHit::through`]. Only an impact reads it.
    pub through: bool,
}

impl GoreHit {
    /// An impact: a crater `radius_m` wide, `depth_mm` deep.
    pub fn impact(entity: Entity, from: Vec3, dir: Vec3, radius_m: f32, depth_mm: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Impact { depth_mm }, so2: crate::bloodstain::spectral::SO2_VENOUS, through: false }
    }
    /// A slash `along` the blade's travel, `radius_m` half its length.
    pub fn slash(entity: Entity, from: Vec3, dir: Vec3, along: Vec3, radius_m: f32, depth_mm: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Slash { along, depth_mm }, so2: crate::bloodstain::spectral::SO2_VENOUS, through: false }
    }
    /// A blunt blow: a bruise `radius_m` across under intact skin.
    pub fn blunt(entity: Entity, from: Vec3, dir: Vec3, radius_m: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Blunt, so2: crate::bloodstain::spectral::SO2_VENOUS, through: false }
    }
    /// A contact burn `radius_m` across: `temp_c` for `seconds`.
    pub fn burn(entity: Entity, from: Vec3, dir: Vec3, radius_m: f32, temp_c: f32, seconds: f32) -> Self {
        Self { entity, from, dir, radius_m, kind: HitKind::Burn { temp_c, seconds }, so2: crate::bloodstain::spectral::SO2_VENOUS, through: false }
    }
    /// **An impact at a contact point**, for a game whose physics reports where something touched and
    /// the surface's normal there rather than a ray: the ray starts [`SURFACE_STAND_OFF`] outside `point`
    /// along `normal` and travels into it, so the preset's own cast lands on that spot. A zero or
    /// non-finite `normal` leaves the hit with no direction, which the preset drops.
    pub fn impact_at(entity: Entity, point: Vec3, normal: Vec3, radius_m: f32, depth_mm: f32) -> Self {
        Self::impact(entity, Vec3::ZERO, Vec3::ZERO, radius_m, depth_mm).at_surface(point, normal)
    }
    /// The same hit, moved to a contact point and the surface's normal there, as [`GoreHit::impact_at`]
    /// does: it keeps its kind, radius and depth, and takes its ray from the surface. A slash keeps
    /// its own `along`.
    pub fn at_surface(mut self, point: Vec3, normal: Vec3) -> Self {
        let outward = normal.normalize_or_zero();
        self.from = point + outward * SURFACE_STAND_OFF;
        self.dir = -outward;
        self
    }
    /// The same hit, bleeding arterial blood.
    pub fn arterial(mut self) -> Self {
        self.so2 = crate::bloodstain::spectral::SO2_ARTERIAL;
        self
    }
    /// **The same impact, going through.** Whether a projectile leaves a body is the game's ballistics,
    /// not this crate's — its calibre, its energy, the armour — so the game says so. The preset then
    /// finds where the ray leaves the part it entered, opens an exit wound there [`EXIT_SCALE`] times
    /// the entry's radius, as an exit is larger and more ragged than the hole it went in by, bleeds
    /// from both, and throws the exit's **forward spatter** along the shot onto whatever is behind: a
    /// wall, the floor, the next body part. Every cloth the ray crosses is holed, behind the body as
    /// well as in front. The exit is announced as a second [`Wounded`], after the entry's, at the exit
    /// point with its outward normal, so whatever a game hangs on a wound happens at both. The exit is
    /// on the part that was entered: a shot through an arm into the chest exits the arm.
    pub fn through(mut self) -> Self {
        self.through = true;
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

/// How many of a wound's droplets are cast at a [`GoreKind::Wall`]. More than at a body or a sheet,
/// because a wall is where spatter is read, drop by drop, and each drop there is its own physical
/// stain a few millimetres across rather than a blob standing for many; a wall is also, nearly
/// always, a handful of triangles.
const WALL_CAST: usize = 128;

/// **How much more blood the forward spatter of an exit throws than its area alone would**: `4`. The
/// projectile carries tissue and blood out ahead of it, and forward spatter is the larger and finer of
/// the two patterns a through-and-through shot leaves (the back spatter is the entry's). This
/// crate's own number.
pub const FORWARD_MIST: f32 = 4.0;

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
            .add_message::<GoreReset>()
            .add_message::<GoreStep>()
            .add_message::<GoreFractured>()
            .add_systems(FixedUpdate, scene_gore.before(dress).in_set(GoreSystems))
            .configure_sets(FixedUpdate, GoreSystems.before(VisceraSystems))
            .add_systems(
                FixedUpdate,
                (advance_clock, dress, reset_gore, take_hits, throw_bodies, bleed, pour_cut_faces, take_steps, age_bruises, soak_cloth, tick_canvases, flush_dermis, fly)
                    .chain()
                    .in_set(GoreSystems),
            )
            .add_systems(FixedUpdate, retube.after(VisceraSystems));
    }
}

/// One tick per fixed step, mirrored into the laceration clock so both agree. On the first tick it
/// checks the rate the app runs `FixedUpdate` at against [`GoreDials::hz`], since Bevy's own default is
/// 64 Hz and the dial's is 60, and a game that forgets the one line in `docs/preset.md` otherwise runs
/// every bleed, bruise and drying six per cent fast with nothing to say so.
fn advance_clock(
    mut clock: ResMut<GoreClock>,
    lac: Option<ResMut<LacerationClock>>,
    dials: Res<GoreDials>,
    fixed: Option<Res<Time<Fixed>>>,
) {
    clock.0 = clock.0.saturating_add(1);
    if clock.0 == 1
        && let Some(actual) = fixed.as_deref().and_then(|t| fixed_rate_mismatch(dials.hz, t.timestep()))
    {
        bevy::log::warn!(
            "gore: FixedUpdate runs at {actual} Hz and GoreDials::hz is {}, so every bleed, bruise and drying \
             runs at {:.0}% of the speed it was authored at; call app.insert_resource(Time::<Fixed>::from_hz({}.0)) \
             or set GoreDials::hz to {actual}",
            dials.hz,
            actual / dials.hz.max(1) as f64 * 100.0,
            dials.hz
        );
    }
    if let Some(mut lac) = lac {
        lac.0 = clock.0;
    }
}

/// The rate `FixedUpdate` runs at, when it is not the one the dials were authored for: within half a
/// hertz is the same rate. A zero step has no rate and is not reported.
fn fixed_rate_mismatch(hz: u32, step: std::time::Duration) -> Option<f64> {
    let actual = 1.0 / step.as_secs_f64().max(f64::MIN_POSITIVE);
    (!step.is_zero() && (actual - hz as f64).abs() > 0.5).then_some((actual * 100.0).round() / 100.0)
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
    if tables.luts == Handle::default() {
        // The tables are baked on `Startup`; a `FixedUpdate` before that would dress with nothing.
        return;
    }
    let mut meshes = meshes;
    let scale = settings.as_ref().map(|s| s.scale).unwrap_or_default();
    for (entity, mut gore, mesh3d, material, skin) in &mut undressed {
        // A floor or a wall laid over from its broadest side is dressed on a copy of its mesh: see
        // `Gore::projected`. The density is the projection's.
        let mut laid: Option<Handle<Mesh>> = None;
        if matches!(gore.kind, GoreKind::Floor | GoreKind::Wall)
            && let Some(meshes) = meshes.as_mut()
        {
            let Some(mesh) = meshes.get(&mesh3d.0) else { continue };
            if gore.projected {
                match projected(mesh) {
                    Ok((copy, [u, v])) => {
                        let xf = transforms.compute_global_transform(entity).unwrap_or_default();
                        let edge_m = 0.5 * (xf.affine().transform_vector3(u).length() + xf.affine().transform_vector3(v).length()) * dials.mm_per_unit / 1000.0;
                        if edge_m.is_finite() && edge_m > 0.0 {
                            gore.uv_per_metre = Some(1.0 / edge_m);
                        }
                        laid = Some(meshes.add(copy));
                    }
                    Err(why) => bevy::log::warn_once!("gore: a projected floor or wall is dressed by its own UV_0 instead, because {why}"),
                }
            } else if tiles(mesh) {
                bevy::log::warn_once!(
                    "gore: a floor's or a wall's UV_0 runs outside [0, 1], so it tiles its texture, and blood on it lands \
                     clamped at its canvas's edge; dress it with Gore::projected()"
                );
            }
        } else if gore.projected {
            bevy::log::warn_once!("gore: Gore::projected is for a floor or a wall; a skin or a cloth is dressed by its own UV_0");
        }
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
        let mut base = material.and_then(|m| standard.get(&m.0).cloned()).unwrap_or_default();
        if let Some(laid) = laid {
            // Its own textures keep the UVs they were authored on, now the copy's `UV_1`.
            base.base_color_channel = UvChannel::Uv1;
            base.emissive_channel = UvChannel::Uv1;
            base.metallic_roughness_channel = UvChannel::Uv1;
            base.normal_map_channel = UvChannel::Uv1;
            base.occlusion_channel = UvChannel::Uv1;
            commands.entity(entity).insert(Mesh3d(laid));
        }
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
                let flay = FlayCanvas::new(&mut images, gore.canvas_texels(&dials), region, layers, base_srgb, rough);
                let wet = WetCanvas::new(&mut images, gore.canvas_texels(&dials), base_srgb, rough);
                let mut params = FleshParams::for_layers(&layers, FleshMode::Canvas, scale.mm_per_unit).with_cut_film(CUT_FACE_FILM_MM);
                params.wet.z = wet_settings.as_ref().map(|w| w.film_depth_mm).unwrap_or(2.0);
                let dermis = DermisCanvas::new(&mut images, gore.canvas_texels(&dials));
                let mut material = tables.material(base, params, Some(wet.roughness()), Some(&flay));
                material.extension.set_dermis(Some(dermis.image()));
                entity_commands.insert((MeshMaterial3d(flesh.add(material)), flay, wet, dermis));
            }
            GoreKind::Cloth => {
                let wet = WetCanvas::new(&mut images, gore.canvas_texels(&dials), base_srgb, rough);
                let layers = Layers::for_region(Region::Torso);
                let mut params = FleshParams::for_layers(&layers, FleshMode::Cloth, scale.mm_per_unit);
                // Blood in a weave is held between the yarns, not stood on them as a film: see
                // `CLOTH_FILM_SHARE`. The shader keeps the wet weave matte (`FleshMode::Cloth`).
                params.wet.z = wet_settings.as_ref().map(|w| w.film_depth_mm).unwrap_or(2.0) * CLOTH_FILM_SHARE;
                let holes = ClothHoles::new(&mut images, gore.canvas_texels(&dials));
                let mut material = tables.material(base, params, Some(wet.roughness()), None);
                material.extension.set_dermis(Some(holes.canvas.image()));
                entity_commands.insert((MeshMaterial3d(flesh.add(material)), wet, Soaking::default(), holes));
            }
            GoreKind::Floor | GoreKind::Wall => {
                let wet = WetCanvas::new(&mut images, gore.canvas_texels(&dials), base_srgb, rough);
                let layers = Layers::for_region(Region::Torso);
                let mut params = FleshParams::for_layers(&layers, FleshMode::Floor, scale.mm_per_unit);
                params.wet.z = wet_settings.as_ref().map(|w| w.film_depth_mm).unwrap_or(2.0);
                params.sss.x = 0.0;
                let material = tables.material(base, params, Some(wet.roughness()), None);
                // One insert per kind, so a floor moves archetype once, as it always did.
                let material = MeshMaterial3d(flesh.add(material));
                if gore.kind == GoreKind::Floor {
                    entity_commands.insert((material, wet, FloorTag));
                } else {
                    entity_commands.insert((material, wet));
                }
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
struct HitContext<'w, 's> {
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
    images: Option<ResMut<'w, Assets<Image>>>,
    bone: MessageWriter<'w, BoneExposed>,
    wounded: MessageWriter<'w, Wounded>,
    fractured: MessageWriter<'w, GoreFractured>,
    policy: Option<Res<'w, GorePolicy>>,
    /// The material every spill's strands share, and the `mm_per_unit` bits it was made under.
    gut_look: Local<'s, Option<(u32, Handle<FleshMaterial>)>>,
    /// The rigid meshes hits are cast at, indexed across ticks, and what Bevy reports changing them.
    index: Local<'s, MeshIndex>,
    mesh_events: MessageReader<'w, 's, AssetEvent<Mesh>>,
}

/// What [`throw_spray`] needs from the batch it is in: the surfaces, the poses and boxes cached across
/// the batch's hits, and the holes a droplet can fly through — those already in a cloth and those this
/// batch opened, which are queued and not yet in it.
struct Thrown<'a, 'w, 's> {
    surfaces: &'a [Surface],
    posed: &'a mut std::collections::HashMap<Entity, Option<Mesh>>,
    bounds: &'a mut std::collections::HashMap<Entity, Reach>,
    identity: &'a GlobalTransform,
    edges: &'a WetSettings,
    tick: u32,
    holes: &'a Query<'w, 's, &'static ClothHoles>,
    /// `(cloth, UV, radius in UV)` of every hole opened earlier in this batch.
    opened: &'a [(Entity, Vec2, f32)],
    /// The meshes, indexed, and the skinned ones' indices refitted to their poses this batch.
    index: &'a MeshIndex,
    refits: &'a mut std::collections::HashMap<Entity, crate::bvh::Refit>,
}

/// **What a wound throws lands on what it reaches first.** Each droplet is cast straight at every
/// dressed surface that is not a floor — a sheet, a wall, the next body part, the shirt the wound is
/// under — and stains only the nearest, so a body part, a cloth or a wall in the way catches it and
/// nothing behind is stained through it. A cloth's hole lets a droplet through. Then the closed-form
/// stains of a horizontal plane go on every floor, for each droplet nothing caught. Straight rather
/// than ballistic for the cast: a droplet at 8–40 m/s falls under a centimetre in the metre it
/// travels. The spray leaves `wound.at` in a cone about `wound.normal`; `struck`, the part the wound
/// is on, is not cast at. The first [`SPRAY_CAST`] droplets are cast; up to [`WALL_CAST`] when a
/// wall's box is in a droplet's path.
#[allow(clippy::too_many_arguments)]
fn throw_spray(
    wound: &BloodWound,
    seed: u32,
    struck: Entity,
    thrown: &mut Thrown,
    bodies: &mut Bodies,
    meshes: &Assets<Mesh>,
    rigging: &Rigging,
    dials: &GoreDials,
    commands: &mut Commands,
) {
    let Thrown { surfaces, posed, bounds, identity, edges, tick, holes, opened, index, refits } = thrown;
    let index: &MeshIndex = index;
    let (edges, tick, identity) = (&**edges, *tick, &**identity);
    let at_world = Vec3::from_array(wound.at);
    let droplets = crate::bloodstain::patterns::impact_spatter(wound, &dials.blood);
    // **What a droplet can be cast at**, each boxed once for the batch and posed, if it is skinned, only
    // when a droplet's ray reaches its box: a crowd's pose is a pass over every vertex of every body.
    let candidates: Vec<&Surface> = surfaces.iter().filter(|s| s.entity != struck && s.gore.kind != GoreKind::Floor).collect();
    for s in &candidates {
        if !bounds.contains_key(&s.entity)
            && let Some(asset) = meshes.get(&s.mesh)
        {
            let reach = reach_of(s, asset, rigging, posed, identity, index);
            bounds.insert(s.entity, reach);
        }
    }
    let drawn: Vec<Option<Candidate>> = candidates
        .iter()
        .map(|s| match bounds.get(&s.entity) {
            Some(Reach::Never) | None => None,
            Some(reach) => Some((*s, reach.bounds())),
        })
        .collect();
    let walls = drawn.iter().flatten().any(|(s, ..)| s.gore.kind == GoreKind::Wall);
    let mut caught = vec![false; droplets.len()];
    let mut soak: Vec<Vec<(Vec2, StainShape, u32)>> = vec![Vec::new(); drawn.len()];
    // `(where the ray enters its box, surface)`, reused by every droplet.
    let mut order: Vec<(f32, usize)> = Vec::with_capacity(drawn.len());
    for (i, d) in droplets.iter().enumerate().take(if walls { WALL_CAST } else { SPRAY_CAST }) {
        let dir = Vec3::from_array(d.dir);
        // Past the ordinary cap a droplet is cast only toward a wall, which is what the wider cap is for.
        let toward_wall = || {
            drawn.iter().flatten().any(|(s, b)| s.gore.kind == GoreKind::Wall && b.as_ref().is_none_or(|b| b.reached_by(at_world, dir)))
        };
        if i >= SPRAY_CAST && !toward_wall() {
            continue;
        }
        let through_hole = |s: &&Surface, l: &Landing| {
            s.gore.kind == GoreKind::Cloth
                && (holes.get(s.entity).is_ok_and(|h| h.is_open_at(l.uv)) || opened.iter().any(|(e, uv, r)| *e == s.entity && uv.distance(l.uv) < *r))
        };
        let cast = |k: usize| -> Option<Landing> {
            let (s, _) = drawn.get(k)?.as_ref()?;
            let asset = meshes.get(&s.mesh)?;
            let (m, xf) = cast_at(s, asset, rigging, posed, identity)?;
            ray_cast(m, cast_index(index, refits, s, asset, m), xf, at_world, dir)
        };
        let Some((k, l)) = nearest_landing(&drawn, at_world, dir, &mut order, cast, through_hole) else { continue };
        if let Some(c) = caught.get_mut(i) {
            *c = true;
        }
        let Some(Some((s, _))) = drawn.get(k) else { continue };
        let Some(asset) = meshes.get(&s.mesh) else { continue };
        let Some((m, xf)) = drawn_now(s, asset, posed, identity) else { continue };
        let Ok((_, _, _, _, _, Some(mut wet))) = bodies.get_mut(s.entity) else { continue };
        let texels = wet.size();
        let shape = match s.gore.kind {
            // **On a wall each droplet is its own stain**: sized, stretched and pointed by how it
            // struck (`hard_stain`), not the blob a body part or a sheet takes for it.
            GoreKind::Wall => match hard_stain(m, xf, &l, d, &dials.blood, seed.wrapping_add(i as u32)) {
                Some(stain) => stain,
                None => continue,
            },
            _ => {
                let radius = crate::bloodstain::stain::stain_radius(d, d.speed, &dials.blood);
                StainShape { major: radius * 2.4, minor: radius * 1.6, spines: 2, satellites: 1, direction: [0.0, 1.0], seed: seed.wrapping_add(i as u32) }
            }
        };
        wet.paint_uv_with(l.uv, &shape_on(&s.gore, &shape, texels), tick, edges);
        if s.gore.kind == GoreKind::Cloth
            && let Some(landings) = soak.get_mut(k)
        {
            landings.push((l.uv, shape, tick));
        }
    }
    for (entry, landed) in drawn.iter().zip(soak) {
        if let (Some((s, ..)), false) = (entry, landed.is_empty())
            && let Ok(mut e) = commands.get_entity(s.entity)
        {
            e.queue(move |mut e: EntityWorldMut| {
                if let Some(mut soaking) = e.get_mut::<Soaking>() {
                    soaking.landings.extend(landed);
                }
            });
        }
    }
    // **The floors**, in closed form, for every droplet nothing above caught. A stain's seed is the
    // spray's own seed mixed with the droplet's ordinal, which is how a stain is matched to its droplet.
    let spray_seed = wound_seed(wound);
    for s in surfaces.iter().filter(|s| s.entity != struck && s.gore.kind == GoreKind::Floor) {
        let Some(asset) = meshes.get(&s.mesh) else { continue };
        let Some((floor_mesh, floor_xf)) = drawn_now(s, asset, posed, identity) else { continue };
        let Ok((_, _, _, _, _, Some(mut wet))) = bodies.get_mut(s.entity) else { continue };
        let plane_y = floor_xf.translation().y;
        for stain in crate::bloodstain::stain::stains(wound, &dials.blood, plane_y) {
            if caught.get((stain.seed ^ spray_seed) as usize).copied().unwrap_or(false) {
                continue;
            }
            let at = Vec3::new(stain.at[0], stain.at[1] + 0.5, stain.at[2]);
            let shape = StainShape {
                major: stain.radius * 2.2,
                minor: stain.radius * 2.0,
                spines: 3,
                satellites: 1,
                direction: [0.0, 1.0],
                seed: stain.seed,
            };
            let texels = wet.size();
            let _ = wet.paint_world_with(floor_mesh, floor_xf, at, Vec3::NEG_Y, &shape_on(&s.gore, &shape, texels), tick, edges);
        }
    }
}

/// **What a surface is drawn as now, without posing it**: its asset placed by its own transform, or
/// the pose [`cast_at`] already built and kept in `posed`. `None` for a skinned surface not posed yet.
fn drawn_now<'a>(
    surface: &'a Surface,
    asset: &'a Mesh,
    posed: &'a std::collections::HashMap<Entity, Option<Mesh>>,
    identity: &'a GlobalTransform,
) -> Option<(&'a Mesh, &'a GlobalTransform)> {
    match surface.skin.as_ref().filter(|_| Rig::of(asset).is_some()) {
        None => Some((asset, &surface.xf)),
        Some(_) => posed.get(&surface.entity)?.as_ref().map(|m| (m, identity)),
    }
}

/// **Open a bleeding wound on `entity`**: beside the ones it already has, or in place of one it
/// landed inside. Silenced, as `try_insert` is: an earlier hit this tick may have broken the body and
/// despawned the part.
fn open_site(commands: &mut Commands, entity: Entity, site: WoundSite, reach: f32, cap: usize) {
    if let Ok(mut e) = commands.get_entity(entity) {
        e.queue_silenced(move |mut e: EntityWorldMut| match e.get_mut::<Wounds>() {
            Some(mut wounds) => wounds.open(site, reach, cap),
            None => {
                let mut wounds = Wounds::default();
                wounds.open(site, reach, cap);
                e.insert(wounds);
            }
        });
    }
}

/// The scene graph [`take_hits`] walks to find a hit's baked subject and its skin material.
#[derive(bevy::ecs::system::SystemParam)]
struct SubjectLookup<'w, 's> {
    parents: Query<'w, 's, &'static ChildOf>,
    subjects: Query<'w, 's, (&'static FractureSubject, &'static GlobalTransform), Without<Broken>>,
    breakables: Query<'w, 's, &'static GlobalTransform, With<GoreBreakable>>,
    dressed: Query<'w, 's, &'static Dressed>,
    /// The holes in every cloth, for the spray to fly through.
    holes: Query<'w, 's, &'static ClothHoles>,
}

/// Something a ray can be cast at — `T` says which — and its box, if it has one.
type Castable<T> = (T, Option<Bounds>);

/// One surface a droplet can be cast at, and its box.
type Candidate<'a> = Castable<&'a Surface>;

/// **Where the ray from `from` along `dir` lands first among `drawn`**: the index of the surface and
/// the landing, nearest first and, of two at one distance, the surface listed first. A landing
/// `lets_through` passes (a droplet through a cloth's hole) is not a landing.
///
/// **The broad phase, nearest box first.** A ray that misses a surface's box cannot land on it; a
/// surface with no box is cast at from the origin, so its refusal is still heard. The rest are cast
/// in the order the ray enters their boxes, and once a landing is nearer than the next box's entry
/// nothing further can beat it, since a mesh lies inside its padded box. Before 0.5.45 every box the
/// ray entered was cast at, which in a crowd is most of them. `cast` casts at the surface it is given
/// the index of, and is called only for one a ray can reach — so a skinned surface is posed only
/// then. `order` is scratch, reused.
fn nearest_landing<T>(
    drawn: &[Option<Castable<T>>],
    from: Vec3,
    dir: Vec3,
    order: &mut Vec<(f32, usize)>,
    mut cast: impl FnMut(usize) -> Option<Landing>,
    mut lets_through: impl FnMut(&T, &Landing) -> bool,
) -> Option<(usize, Landing)> {
    order.clear();
    order.extend(drawn.iter().enumerate().filter_map(|(k, entry)| {
        let (_, b) = entry.as_ref()?;
        match b {
            Some(b) => b.entry(from, dir).map(|t| (t, k)),
            None => Some((0.0, k)),
        }
    }));
    order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut best: Option<(usize, Landing)> = None;
    for &(enters, k) in order.iter() {
        if best.as_ref().is_some_and(|(_, near)| near.t < enters) {
            break;
        }
        let Some(Some((s, _))) = drawn.get(k) else { continue };
        let Some(l) = cast(k) else { continue };
        if best.as_ref().is_some_and(|(bk, near)| near.t < l.t || (near.t == l.t && *bk < k)) {
            continue;
        }
        if !lets_through(s, &l) {
            best = Some((k, l));
        }
    }
    best
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
        images,
        bone,
        wounded,
        fractured,
        policy,
        gut_look,
        index,
        mesh_events,
    } = &mut cx;
    // A mesh Bevy reports changed is indexed again when it is next cast at; read every tick, hits or
    // none, so no report is missed.
    for event in mesh_events.read() {
        if let AssetEvent::Modified { id } | AssetEvent::Removed { id } | AssetEvent::Unused { id } = event {
            index.forget(id);
        }
    }
    let SubjectLookup { parents, subjects, breakables, dressed, holes: cloth_holes } = &lookup;
    let tick = clock.0;
    // What the player's settings allow, read once per batch. A tier that does not draw blood draws
    // none of the rest; each of the other three is its own switch, so a game can allow blood and
    // refuse the world's share of it, or the guts, or a body coming apart. Absent, everything is.
    let policy = policy.as_deref().cloned().unwrap_or_default();
    let draws_blood = policy.draws_blood();
    let (spray, guts, breaks) = (draws_blood && policy.blood_decals, draws_blood && policy.viscera, draws_blood && policy.dismemberment);
    // Every hit of the tick, in one pass. A hit that reaches bone queues the subject's despawn, and a
    // later hit in the same batch on a sibling part still passes the query — so every insert below
    // is a `try_insert`, which tolerates a despawn queued ahead of it where `insert` panics.
    // Each rigid surface's mesh indexed the first tick it is there, so the build is paid when it is
    // dressed rather than when it is first shot: 3 ms for sixteen thousand triangles on `big`.
    for (_, _, mesh, _, skin) in bodies_index.iter() {
        let id = mesh.0.id();
        if index.get(id).is_some() {
            continue;
        }
        if let Some(asset) = meshes.get(&mesh.0) {
            // A skinned mesh's tree is built over its bind pose and refitted to each pose it is cast at.
            index.ensure(id, asset);
            if skin.filter(|_| Rig::of(asset).is_some()).is_some() {
                index.ensure_joints(id, asset);
            }
        }
    }
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
    // Every rigid surface's mesh indexed before the batch casts at it: built once and kept while the
    // mesh stays as it was, so a crowd sharing one mesh builds one.
    for surface in &surfaces {
        if let Some(asset) = meshes.get(&surface.mesh) {
            index.ensure(surface.mesh.id(), asset);
            if surface.skin.as_ref().filter(|_| Rig::of(asset).is_some()).is_some() {
                index.ensure_joints(surface.mesh.id(), asset);
            }
        }
    }
    let index: &MeshIndex = index;
    // A skinned surface is posed once per batch, the first time a droplet is cast at it: a pose is a
    // pass over every vertex, and a batch of shots at one body would otherwise pose it once per shot.
    let mut posed: std::collections::HashMap<Entity, Option<Mesh>> = std::collections::HashMap::new();
    // And where each surface can be reached, once per batch, for the same reason.
    let mut bounds: std::collections::HashMap<Entity, Reach> = std::collections::HashMap::new();
    // And each posed surface's index, refitted to its pose, once per batch.
    let mut refits: std::collections::HashMap<Entity, crate::bvh::Refit> = std::collections::HashMap::new();
    let identity = GlobalTransform::IDENTITY;
    // Every hole this batch opened, for droplets to fly through before the queued cut lands.
    let mut opened: Vec<(Entity, Vec2, f32)> = Vec::new();
    for hit in hits {
        // **A hit at a character** names the entity the game's physics found, which is rarely the mesh
        // that was struck: the preset casts the ray at every skin under it and hurts the nearest.
        let hit = if bodies.contains(hit.entity) {
            hit
        } else {
            let Some(entity) = nearest_skin(&hit, &surfaces, parents, meshes, &rigging, &mut posed, &mut bounds, &identity, index, &mut refits) else {
                continue;
            };
            GoreHit { entity, ..hit }
        };
        let Ok((gore, mesh3d, xf, skin, flay, wet)) = bodies.get_mut(hit.entity) else { continue };
        let gore = *gore;
        let gore = &gore;
        let xf = *xf;
        let xf = &xf;
        let skin: Option<SkinnedMesh> = skin.cloned();
        let Some(bind) = meshes.get(&mesh3d.0).cloned() else { continue };
        let drawn_mesh = mesh3d.0.clone();
        // **Cast at the body as it is drawn.** A skinned mesh's asset is its bind pose, and the entity's
        // own transform plays no part in drawing it; the pose is what the player is aiming at. A rigid
        // mesh is the asset placed by its transform.
        let drawn = Drawn::of(&bind, skin.as_ref(), &rigging);
        // The struck mesh's index: its own when it is drawn rigidly, refitted to its pose when it is posed.
        let struck_refit = match &drawn {
            Drawn::Posed(posed, _) => index.get(drawn_mesh.id()).zip(positions_of(posed)).map(|(tree, at)| tree.refit(at)),
            _ => None,
        };
        let struck_index = index.get(drawn_mesh.id()).and_then(|tree| match (&drawn, &struck_refit) {
            (Drawn::Rigid, _) => Some(tree.fitted()),
            (Drawn::Posed(..), Some(refit)) => Some(tree.with(refit)),
            _ => None,
        });
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

        // **Reduction is substitution, never deletion** ([`GorePolicy`]). At a tier that does not draw
        // blood, a hit that would have opened a wound is still found, placed and announced: the same
        // `Wounded`, at the same point, with the same normal and area. Nothing is painted, torn or
        // thrown, and the game puts whatever it wants at that point in blood's place.
        if !draws_blood && matches!(hit.kind, HitKind::Impact { .. } | HitKind::Slash { .. }) {
            let (at_world, normal_world) = match ray_cast(mesh, struck_index, xf, hit.from, hit.dir) {
                Some(l) => (xf.transform_point(l.point), xf.affine().transform_vector3(l.normal).normalize_or_zero()),
                None => (hit.from, -hit.dir.normalize_or_zero()),
            };
            let normal_world = if normal_world.dot(-hit.dir) < 0.0 { -normal_world } else { normal_world };
            wounded.write(announce(&hit, at_world, normal_world));
            continue;
        }
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
        let landed = ray_cast(mesh, struck_index, xf, hit.from, hit.dir);
        let site_uv = landed.map(|l| l.uv);

        // A blow: a bruise starts under the skin, pooled to the blow's radius.
        if let HitKind::Blunt = hit.kind
            && let Some(uv) = site_uv
        {
            let params = crate::bloodstain::bruise::Params {
                pool_diameter_mm: (hit.radius_m * 2.0 * 1000.0).clamp(2.0, 100.0),
                ..crate::bloodstain::bruise::Params::default()
            };
            let site = BruiseSite { uv, bruise: crate::bloodstain::Bruise::new(params), carry: 0.0 };
            // **Added, not swapped**: every bruise already under the skin keeps ageing. In hit order,
            // so two blows in one tick land in the order they were sent.
            commands.entity(hit.entity).queue(move |mut e: EntityWorldMut| {
                if let Some(mut bruising) = e.get_mut::<Bruising>() {
                    if bruising.sites.len() >= MAX_BRUISES {
                        bruising.sites.remove(0);
                    }
                    bruising.sites.push(site);
                } else {
                    e.insert(Bruising { sites: vec![site] });
                }
            });
        }

        // A burn: Jackson's three zones round the contact, as far in as the degree reached — see
        // [`burn_zone`]. The degree is the model's; the colours are this crate's own.
        if let HitKind::Burn { .. } = hit.kind
            && let Some(uv) = site_uv
            && let Ok(mut entity_ref) = commands.get_entity(hit.entity)
        {
            let contact_mm = radius_uv * 1000.0 / gore.density().max(1.0e-6);
            let mm_per_uv = 1000.0 / gore.density().max(1.0e-6);
            let margin = Margin::at(uv, 0);
            entity_ref.queue(move |mut e: EntityWorldMut| {
                if let Some(mut dermis) = e.get_mut::<DermisCanvas>() {
                    dermis.paint_radial(uv, radius_uv * BURN_FLARE * (1.0 + BURN_WOBBLE), mm_per_uv, |r_mm, dir| {
                        burn_zone(burn_degree, r_mm / (contact_mm * margin.scale(dir, BURN_WOBBLE)).max(1.0e-6))
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

        // **A slash is a cut, not a disc.** Where it lies on the mesh asset — for a skinned body the
        // bind pose, carried back through the pose at the triangle that was hit: the same triangle, the
        // same stretch — its direction there, and the scale from the asset to the world.
        let layout = match hit.kind {
            HitKind::Slash { along, .. } => match (&drawn, &landed) {
                (Drawn::Posed(_, pose), Some(l)) => bind_landing(&bind, l).and_then(|(p, n)| {
                    let m = l.matrix(&bind, pose)?;
                    let along_local = m.inverse().transform_vector3(along).normalize_or_zero();
                    Some((p, n, along_local, skin::max_scale(&m).max(1.0e-6)))
                }),
                _ => at_local.map(|p| {
                    let along_local = xf.affine().inverse().transform_vector3(along).normalize_or_zero();
                    (p, normal_local.unwrap_or(Vec3::Y), along_local, xf.scale().max_element().max(1.0e-6))
                }),
            },
            _ => None,
        };
        // And its line on this wetmap: the blade's half-length read in `UV_0` at the triangle it
        // crossed, and how far out its lips stand once it has opened.
        let cut = match (hit.kind, landed) {
            (HitKind::Slash { along, .. }, Some(l)) => {
                let half = xf.affine().inverse().transform_vector3(along.normalize_or_zero() * hit.radius_m);
                face_uv(mesh, l.tri, half)
                    .map(|(half_uv, _)| half_uv)
                    .filter(|h| h.is_finite() && *h != Vec2::ZERO)
                    .map(|half_uv| CutLine { half_uv, lip_uv: 0.5 * SLASH_OPEN_M * gore.density() })
            }
            _ => None,
        };

        // 2. Blood at the wound and on the floors.
        let wound = BloodWound {
            at: [at_world.x, at_world.y, at_world.z],
            normal: [normal_world.x, normal_world.y, normal_world.z],
            area: opened_area(&hit),
            severity: 1.0,
            kind: if hit.depth_mm() > 0.0 { WoundKind::Channel } else { WoundKind::Severance },
        };
        let seed = wound_seed(&wound);
        let shape = match cut {
            // A cut leaves its blood along itself: a smear as long as the blade and a little wider than
            // the mouth, laid along the cut in the atlas. Until 0.5.34 it took a disc 2.6 times the
            // blade's half-length tall, and on a thigh that read as a sheet with a line drawn in it.
            Some(c) => StainShape {
                // The cut's own length on this atlas, in the metres `shape_on` reads, so the smear
                // ends where the cut does on an atlas stretched more one way than the other.
                major: 2.0 * c.half_uv.length() / gore.density().max(1.0e-6),
                minor: SLASH_OPEN_M * 2.0,
                spines: 0,
                satellites: 0,
                direction: c.half_uv.normalize_or_zero().to_array(),
                seed,
            },
            None => StainShape {
                major: hit.radius_m * 2.6,
                minor: hit.radius_m * 2.0,
                spines: 4,
                satellites: 2,
                direction: [0.0, 1.0],
                seed,
            },
        };
        let mut painted = None;
        if let Some(mut wet) = wet {
            let texels = wet.size();
            painted = wet.paint_world_with(mesh, xf, hit.from, hit.dir, &shape_on(gore, &shape, texels), tick, &edges);
        }
        // **A hole in what it went through.** An impact crosses every cloth between where it came from
        // and the skin it wounds — a shirt over the wound, a curtain in front of the body — and opens
        // a hole in each where it crossed, so the wound under clothing is seen. A cloth behind the
        // skin is not reached when the shot stopped in the body; one that goes through holes it too,
        // where it last crosses: the back of the shirt, a curtain behind. **A blade cuts a slit**
        // instead, in the cloth over the skin it opens, as long as the cut and along it.
        if let (HitKind::Impact { .. } | HitKind::Slash { .. }, Some(skin_landing)) = (hit.kind, landed) {
            for surface in surfaces.iter().filter(|s| s.gore.kind == GoreKind::Cloth) {
                let Some(asset) = meshes.get(&surface.mesh) else { continue };
                let reach = *bounds.entry(surface.entity).or_insert_with(|| reach_of(surface, asset, &rigging, &mut posed, &identity, index));
                match reach {
                    Reach::Never => continue,
                    Reach::Within(b) if !b.reached_by(hit.from, hit.dir) => continue,
                    _ => {}
                }
                // Posed only now that the shot can reach it.
                let Some((cloth_mesh, cloth_xf)) = cast_at(surface, asset, &rigging, &mut posed, &identity) else { continue };
                let front = ray_cast(cloth_mesh, cast_index(index, &mut refits, surface, asset, cloth_mesh), cloth_xf, hit.from, hit.dir).filter(|c| c.t <= skin_landing.t);
                let mut holes: Vec<(Vec2, f32)> = Vec::new();
                let mut slits: Vec<(Vec2, Vec2)> = Vec::new();
                match hit.kind {
                    HitKind::Slash { along, .. } => {
                        // The blade's half-length, carried into the cloth's own space and read in its UV
                        // at the triangle it crossed.
                        let half = cloth_xf.affine().inverse().transform_vector3(along.normalize_or_zero() * hit.radius_m);
                        if let Some(c) = front
                            && let Some((half_uv, _)) = face_uv(cloth_mesh, c.tri, half)
                        {
                            slits.push((c.uv, half_uv));
                        }
                    }
                    _ => {
                        let behind = if hit.through { ray_exit(cloth_mesh, cast_index(index, &mut refits, surface, asset, cloth_mesh), cloth_xf, hit.from, hit.dir).filter(|c| c.t > skin_landing.t) } else { None };
                        let radius_uv = hit.radius_m * surface.gore.density() / crate::flaymap::UV_SPAN_M;
                        holes.extend([front.map(|c| (c.uv, radius_uv)), behind.map(|c| (c.uv, radius_uv * EXIT_SCALE))].into_iter().flatten());
                        // What a droplet can fly through this batch, before the queued hole is cut: the
                        // hole's open core, inside the narrowest its ragged edge comes.
                        opened.extend(holes.iter().map(|(uv, r)| (surface.entity, *uv, r * HOLE_OPEN * 0.75)));
                    }
                }
                if !(holes.is_empty() && slits.is_empty())
                    && let Ok(mut e) = commands.get_entity(surface.entity)
                {
                    e.queue(move |mut e: EntityWorldMut| {
                        if let Some(mut cloth) = e.get_mut::<ClothHoles>() {
                            for (uv, radius_uv) in holes {
                                cloth.open(uv, radius_uv, seed);
                            }
                            for (uv, half_uv) in slits {
                                cloth.slit(uv, half_uv, seed);
                            }
                        }
                    });
                }
            }
        }
        if spray {
            let mut thrown = Thrown { surfaces: &surfaces, posed: &mut posed, bounds: &mut bounds, identity: &identity, edges: &edges, tick, holes: cloth_holes, opened: &opened, index, refits: &mut refits };
            throw_spray(&wound, seed, hit.entity, &mut thrown, &mut bodies, meshes, &rigging, dials, &mut commands);
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
        if let (Some(uv), Some(at_local)) = (painted, site_at) {
            let site = WoundSite { uv, at_local, skin: site_skin, wound, bleed: crate::bloodstain::Bleed::new(tick, &wound), cut };
            open_site(&mut commands, hit.entity, site, hit.radius_m * gore.density(), dials.open_wounds);
        }
        wounded.write(announce(&hit, at_world, normal_world));

        // 3. **An exit**, for an impact the game says went through: where the ray leaves the part it
        // entered, a crater wider than the entry's, cut from outside along the way the shot left, its
        // own bleeding wound, and the forward spatter thrown along the shot onto what is behind.
        if hit.through
            && let HitKind::Impact { depth_mm } = hit.kind
            && let Some(entry) = landed
            && let Some(exit) = ray_exit(mesh, struck_index, xf, hit.from, hit.dir).filter(|e| e.t > entry.t)
        {
            let out = hit.dir.normalize_or_zero();
            let exit_world = xf.transform_point(exit.point);
            let exit_normal = xf.affine().transform_vector3(exit.normal).normalize_or_zero();
            let exit_normal = if exit_normal.dot(out) < 0.0 { -exit_normal } else { exit_normal };
            let exit_radius = hit.radius_m * EXIT_SCALE;
            let exit_wound = BloodWound {
                at: exit_world.to_array(),
                normal: exit_normal.to_array(),
                area: core::f32::consts::PI * exit_radius * exit_radius,
                severity: 1.0,
                kind: WoundKind::Channel,
            };
            let exit_seed = wound_seed(&exit_wound);
            // Cut from just outside the exit, back along the shot, so the crater opens on that face.
            let outside = exit_world + out * SURFACE_STAND_OFF;
            let mut exit_uv = None;
            if let Ok((_, _, _, _, flay, wet)) = bodies.get_mut(hit.entity) {
                if let Some(mut flay) = flay {
                    let _ = flay.paint_world(mesh, xf, outside, -out, radius_uv * EXIT_SCALE, depth_mm, tick);
                    if let Some(s) = flay_settings.as_deref() {
                        flay.shade(s);
                    }
                }
                if let Some(mut wet) = wet {
                    let shape = StainShape {
                        major: exit_radius * 2.6,
                        minor: exit_radius * 2.0,
                        spines: 6,
                        satellites: 3,
                        direction: [0.0, 1.0],
                        seed: exit_seed,
                    };
                    let texels = wet.size();
                    exit_uv = wet.paint_world_with(mesh, xf, outside, -out, &shape_on(gore, &shape, texels), tick, &edges);
                }
            }
            if spray {
                // Forward spatter leaves along the shot, not off the skin's face: the cone's axis is
                // the projectile's own direction.
                let forward = BloodWound { normal: out.to_array(), area: exit_wound.area * FORWARD_MIST, ..exit_wound };
                let mut thrown = Thrown { surfaces: &surfaces, posed: &mut posed, bounds: &mut bounds, identity: &identity, edges: &edges, tick, holes: cloth_holes, opened: &opened, index, refits: &mut refits };
                throw_spray(&forward, exit_seed, hit.entity, &mut thrown, &mut bodies, meshes, &rigging, dials, &mut commands);
            }
            let exit_at = match &drawn {
                Drawn::Posed(..) => bind_landing(&bind, &exit).map(|(p, _)| (p, Some(SkinAnchor { tri: exit.tri, bary: exit.bary }))),
                _ => Some((exit.point, None)),
            };
            if let (Some(uv), Some((at_local, skin))) = (exit_uv, exit_at) {
                let site = WoundSite { uv, at_local, skin, wound: exit_wound, bleed: crate::bloodstain::Bleed::new(tick, &exit_wound), cut: None };
                open_site(&mut commands, hit.entity, site, exit_radius * gore.density(), dials.open_wounds);
            }
            wounded.write(Wounded { area: exit_wound.area, ..announce(&hit, exit_world, exit_normal) });
        }

        // 4. A slash gapes instead of cratering, along the layout above.
        if let HitKind::Slash { depth_mm, .. } = hit.kind
            && let Some((p, n_local, along_local, scale)) = layout
        {
            let half = hit.radius_m / scale;
            let tension = Tension::default();
            let source = meshes.add(bind.clone());
            // The mesh it wore before its first slash, which a reset puts back.
            commands.entity(hit.entity).queue(move |mut e: EntityWorldMut| {
                if !e.contains::<Untorn>() {
                    e.insert(Untorn(drawn_mesh));
                }
            });
            // **The wound is torn into a mesh this entity owns.** The laceration rewrites the entity's
            // drawn mesh in place, and a loaded scene shares one mesh handle between every instance of
            // a character: torn through the shared handle, one slash opened the same gape on every
            // enemy wearing that mesh. The copy keeps whatever the mesh had, tangents and joint weights
            // included.
            let torn = meshes.add(bind.clone());
            commands.entity(hit.entity).try_insert(Mesh3d(torn));
            commands.entity(hit.entity).try_insert(Laceration {
                path: slash_path(&bind, p, n_local, along_local, half),
                normal: n_local,
                // `SLASH_OPEN_M` in the world, in the asset's own units, under the tension it opens at.
                gape: Gape { width_max: SLASH_OPEN_M / (scale * tension.skin.max(1.0e-6)), ..Gape::default() },
                tension,
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
            && guts
            && matches!(deepest, Layer::Muscle | Layer::Cortex | Layer::Marrow)
            && let Some(visc) = visc.as_deref()
        {
            let strands = spill(at_world, normal_world + Vec3::new(0.0, -0.4, 0.0), dials.strands, seed, visc);
            // One material for every spill, made on the first: bowel's own texture over the muscle's
            // subsurface profile. Until 0.5.41 each spill made a new one, the red of the blood it fell
            // in.
            let key = dials.mm_per_unit.to_bits();
            let gut_material = match gut_look.as_ref() {
                Some((k, material)) if *k == key => material.clone(),
                _ => {
                    let mut muscle_params = FleshParams::for_layers(&Layers::for_region(region), FleshMode::Cap, dials.mm_per_unit);
                    // Every texel is muscle: the strands carry no `UV_1`, so the shader's depth is `0`, and
                    // `layer_of` walks past every band start it is not below. Muscle is index 2, so fat and
                    // muscle start at `0` and cortex and marrow start past any depth. (`Vec4::ZERO` selected
                    // marrow, whose profile is fat's — caught in review.)
                    muscle_params.bands = Vec4::new(0.0, 0.0, 2.0, 2.0);
                    // The texture goes where every image goes; with no `Assets<Image>` there is no
                    // renderer to draw the strands either.
                    let texture = images.as_deref_mut().map(|images| images.add(gut_image()));
                    let material = flesh.add(tables.material(
                        StandardMaterial { base_color: Color::WHITE, base_color_texture: texture, perceptual_roughness: GUT_ROUGHNESS, ..default() },
                        muscle_params,
                        None,
                        None,
                    ));
                    **gut_look = Some((key, material.clone()));
                    material
                }
            };
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
            // A tier without dismemberment forwards the bone and breaks nothing.
            if !breaks {
                continue;
            }
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
            // **A skinned body breaks where its skeleton is**, not where its asset was authored: the
            // pose it was drawn in, and the skinned meshes under the root that share this one's joints,
            // are what each piece is carried by. A rigid body has none of it.
            let posing = match (&drawn, &skin) {
                (Drawn::Posed(_, pose), Some(rig)) => Some(Posing {
                    pose: pose.clone(),
                    parts: surfaces
                        .iter()
                        .filter(|s| s.skin.as_ref().is_some_and(|k| k.joints == rig.joints) && is_under(root, s.entity, parents))
                        .map(|s| s.mesh.clone())
                        .collect(),
                }),
                _ => None,
            };
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
                let pieces = place(leaves, root_xf, at_world, posing.as_ref().map(|p| (p, &**meshes)));
                if dials.spawn_fragments {
                    spawn_pieces(&mut commands, meshes, tick, &pieces, &skin, &cap);
                    commands.entity(root).despawn();
                } else {
                    commands.entity(root).try_insert(Broken);
                }
                fractured.write(GoreFractured { subject: root, at: at_world, pieces, skin, cap });
            } else if let Some(root_xf) = breakable {
                // The message goes out from `throw_bodies`, next in the chain this tick, once the
                // leaves are meshes.
                let cap = cap_material(region, atlas.as_deref(), standard, tables, flesh, dials);
                // The blow in the subject's own space: for a skinned body that is where it landed on
                // the bind pose, since the fracture was baked there, not where the body is drawn.
                let at_local = match (&posing, site_at) {
                    (Some(_), Some(bind)) => bind,
                    _ => root_xf.affine().inverse().transform_point3(at_world),
                };
                let spawn = dials.spawn_fragments;
                commands.entity(root).queue_silenced(move |mut e: EntityWorldMut| {
                    if let Some(body) = e.take::<GoreBreakable>() {
                        e.insert(ThrowNow { body, at: at_world, at_local, skin, cap, spawn, posing });
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

/// The mesh a slashed skin wore before its first slash: the shared handle a reset puts back, where
/// the slash left the entity drawing a torn copy of its own.
#[derive(Component)]
struct Untorn(Handle<Mesh>);

/// **A foot came down**: the footstep a game already raises for its sound, told to the gore. A sole
/// put down in wet blood on a floor picks some of it up, and a sole carrying blood leaves a print,
/// fainter at every step until it has walked it off — see [`Tread`]. Read once per tick, after the
/// tick's bleeding, so a foot planted in the pool a wound is filling picks up this tick's blood.
///
/// The preset does not find feet or decide when one lands: a game's animation already knows, and a
/// guess from the mesh would print a sliding foot or miss a planted one. Send one per foot per step.
#[derive(Message, Clone, Copy, Debug)]
pub struct GoreStep {
    /// Whose sole it is. The blood it carries is kept on this entity as a [`Tread`], so name the same
    /// entity for every step of one foot — its bone, or its mesh.
    pub foot: Entity,
    /// Where the sole meets the ground, world space: the print's centre. The floor it lands on is the
    /// nearest straight below a point [`STEP_REACH_M`] above this, no further than that below it.
    pub at: Vec3,
    /// Heel to toe, world space; laid flat onto the floor.
    pub heading: Vec3,
    /// The sole's size and tread.
    pub sole: Sole,
}

impl GoreStep {
    /// A [`Sole::BOOT`] put down at `at`, toe toward `heading`.
    pub fn new(foot: Entity, at: Vec3, heading: Vec3) -> Self {
        Self { foot, at, heading, sole: Sole::BOOT }
    }
    /// The same step with another sole.
    pub fn with_sole(mut self, sole: Sole) -> Self {
        self.sole = sole;
        self
    }
}

/// **A sole**, for [`GoreStep`]: its size, and its tread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sole {
    /// Heel to toe, metres.
    pub length_m: f32,
    /// Across the forefoot, metres.
    pub width_m: f32,
    /// Which patches of it give up blood first: any number, the same for every step of one shoe, so
    /// its prints leave the same gaps. Two shoes of a pair may share it; a print is symmetric.
    pub tread: u32,
}

impl Sole {
    /// An adult's boot, 29 by 11 cm. This crate's own figure.
    pub const BOOT: Sole = Sole { length_m: 0.29, width_m: 0.11, tread: 0 };
}

/// **The blood on a foot's sole**, kept on the [`GoreStep::foot`] entity by the preset from its first
/// step. Read it to drive a game's own look — a sound, or blood drawn on the shoe — and a
/// [`GoreReset`] of the foot or anything above it wipes it.
///
/// `load` is how much of a full print the next step inks: a step in wet blood adds the mean coverage
/// it stood in, up to `1`, and every step sheds [`TREAD_SHED`] of it onto the floor first, so prints
/// out of a pool fade geometrically — the sixth is about a sixth of the first — and a sole that
/// crossed only dried blood carries none.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Tread {
    /// What the next print inks, `0` to `1`.
    pub load: f32,
}

/// **The share of a sole's blood each print leaves**: `0.3`, so the sixth print out of a pool inks
/// `0.7⁵ ≈ 0.17` of the first and the eighth is a scatter of flecks. This crate's own number: nothing
/// published counts the steps a bloodied sole prints on a hard floor.
pub const TREAD_SHED: f32 = 0.3;

/// **The share of the wet blood under a sole a step takes up**: `0.3`. What it leaves is a lighter
/// sole-shaped dent in the pool that the pool's own spread fills. This crate's own number.
pub const TREAD_LIFT: f32 = 0.3;

/// **How far above and below [`GoreStep::at`] a floor is looked for**: `0.25` m — a step lands on the
/// floor under the foot, never on one a storey down. This crate's own number.
pub const STEP_REACH_M: f32 = 0.25;

/// **Heal a character, or wipe a surface, back to how it was dressed.** `entity` is a dressed surface
/// or any ancestor of dressed surfaces, the way [`GoreHit::entity`] is: every surface at or below it
/// is reset. Its canvases are cleared in place — blood, peeled tissue, bruises, burns, holes in its
/// cloth — and the next flush uploads them clean; its bleeding, bruising and soaking stop, and a
/// foot's [`Tread`] is wiped; a slashed
/// skin draws its intact mesh again and its wound bed goes; a subject broken without spawning its
/// pieces can break again. Nothing is reallocated, so a pooled or respawned character costs the
/// bytes overwritten rather than a fresh dressing: every canvas and image built again.
///
/// **What it leaves.** What has left the body: thrown pieces, spilled guts, the stains it bled on a
/// floor or a wall — reset a floor or a wall by sending its own entity, which clears its canvas. A body
/// the preset broke into pieces has been despawned and cannot be reset. Hits read on the same tick
/// land after the reset.
#[derive(Message, Clone, Copy, Debug)]
pub struct GoreReset {
    /// The surface, or an ancestor of the surfaces, to reset.
    pub entity: Entity,
}

impl GoreReset {
    /// Reset `entity` and every dressed surface below it.
    pub fn new(entity: Entity) -> Self {
        Self { entity }
    }
}

/// The state [`reset_gore`] clears on one surface.
type Resettable<'a> = (
    Option<&'a mut WetCanvas>,
    Option<&'a mut FlayCanvas>,
    Option<&'a mut DermisCanvas>,
    Option<&'a mut ClothHoles>,
    Option<&'a mut Soaking>,
    Option<&'a Untorn>,
);

/// **Apply every [`GoreReset`]**, before the tick's hits.
fn reset_gore(
    mut commands: Commands,
    mut resets: MessageReader<GoreReset>,
    clock: Res<GoreClock>,
    children: Query<&Children>,
    beds: Query<(), With<LacerationBed>>,
    mut surfaces: Query<Resettable>,
) {
    let tick = clock.0;
    for reset in resets.read() {
        let mut todo = vec![reset.entity];
        while let Some(e) = todo.pop() {
            if let Ok(below) = children.get(e) {
                for child in below.iter() {
                    // A wound's bed is the slash's, and goes with it.
                    if beds.contains(child) {
                        commands.entity(child).try_despawn();
                    } else {
                        todo.push(child);
                    }
                }
            }
            let Ok((wet, flay, dermis, holes, soaking, untorn)) = surfaces.get_mut(e) else { continue };
            if let Some(mut wet) = wet {
                wet.clear(tick);
            }
            if let Some(mut flay) = flay {
                flay.clear(tick);
            }
            if let Some(mut dermis) = dermis {
                dermis.clear();
            }
            if let Some(mut holes) = holes {
                holes.clear();
            }
            if let Some(mut soaking) = soaking {
                soaking.landings.clear();
            }
            let mut entity = commands.entity(e);
            if let Some(untorn) = untorn {
                entity.try_insert(Mesh3d(untorn.0.clone()));
            }
            entity.try_remove::<(Wounds, Bruising, Laceration, Untorn, Broken, ThrowNow, CutFaceBleed, Tread)>();
        }
    }
}

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
    /// The skeleton's pose, when the body that broke was skinned.
    posing: Option<Posing>,
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
    let params = FleshParams::for_layers(&layers, FleshMode::Cap, dials.mm_per_unit).with_cut_film(CUT_FACE_FILM_MM);
    flesh.add(tables.material(base, params, None, None))
}

/// **How far past the contact a burn's reddening reaches**, as a multiple of the contact's radius:
/// `1.7`. The zone of hyperaemia is the inflamed skin round a burn, not under the thing that burnt
/// it, and it fades into the skin rather than stopping: drawn out to `1.4` it fell off over two
/// texels of a 128 canvas and read as a thin red outline. This crate's own number.
pub const BURN_FLARE: f32 = 1.7;

/// **What a burn of `degree` shows at `t` contact radii from its centre**: the linear ratio the skin is
/// multiplied by, and how much of it shows.
///
/// Jackson's three zones (Jackson 1953, `doi:10.1002/bjs.18004016413`): **coagulation** at the
/// centre, where the tissue is dead — a charred eschar, near black; **stasis** round it, pale and
/// blistered; and **hyperaemia** outermost, reddened, fading from the stasis ring out to
/// [`BURN_FLARE`]. A burn has the zones up to the degree it reached and no deeper: a first-degree
/// burn is all hyperaemia, a second a blister in a red ring, a third an eschar in a pale ring in a
/// red one. Until 0.5.43 a burn of each degree was one colour fading to its edge, and a third-degree
/// contact read as a grey smudge. The zones' widths and colours are this crate's own.
pub fn burn_zone(degree: crate::bloodstain::burn::Degree, t: f32) -> ([f32; 3], f32) {
    use crate::bloodstain::burn::Degree;
    const ESCHAR: [f32; 3] = [0.10, 0.06, 0.04];
    const STASIS: [f32; 3] = [1.0, 0.93, 0.84];
    const HYPERAEMIA: [f32; 3] = [1.0, 0.45, 0.40];
    let t = if t.is_finite() { t.max(0.0) } else { return ([1.0; 3], 0.0) };
    // The edge of a zone, blended over a tenth of the radius either side of it.
    let inside = |edge: f32| 1.0 - smooth01((t - edge + 0.1) / 0.2);
    let (coagulation, stasis) = match degree {
        Degree::None => return ([1.0; 3], 0.0),
        Degree::First => (0.0, 0.0),
        Degree::Second => (0.0, inside(0.7)),
        Degree::Third => (inside(0.55), inside(0.85)),
    };
    let stasis = (stasis - coagulation).max(0.0);
    // The flare fades from where the stasis ring of a third-degree burn ends, whatever the degree.
    let hyperaemia = (1.0 - coagulation - stasis).max(0.0) * (1.0 - smooth01((t - 0.85) / (BURN_FLARE - 0.85)));
    let mix = |c: usize| coagulation * ESCHAR[c] + stasis * STASIS[c] + (1.0 - coagulation - stasis) * HYPERAEMIA[c];
    ([mix(0), mix(1), mix(2)], coagulation + 0.65 * stasis + 0.75 * hyperaemia)
}

/// The cubic smoothstep over `[0, 1]`, clamped.
fn smooth01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// **How far a burn's zones wander off round**, as a share of their radius either way: `0.15`. A
/// contact burn's margin follows the skin's give and the object's edge, not a compass; drawn
/// perfectly round its three zones read as a target. This crate's own number.
pub const BURN_WOBBLE: f32 = 0.15;

/// **How far a bruise's outline wanders off round**, as a share of its radius either way: `0.25`.
/// The model is a radial reduction, so it hands back a disc; blood under the skin tracks along the
/// tissue planes it meets, and drawn as the disc a bruise read as a flat brown dot. Wider than a
/// burn's, whose margin is the shape of what touched the skin. This crate's own number.
pub const BRUISE_WOBBLE: f32 = 0.25;

/// **How patchy a bruise is**: its optical density varies by this share either way across it,
/// `0.4`, as a pool that much thicker or thinner would. An ecchymosis is mottled where the blood
/// pooled unevenly; the model's pool is uniform inside its diameter. This crate's own number.
pub const BRUISE_MOTTLE: f32 = 0.4;

/// The size of a bruise's mottle, mm: the larger of its two scales of noise. This crate's own.
const BRUISE_MOTTLE_MM: f32 = 9.0;

/// The shape of one injury's margin under intact skin: two lobed waves round it, with amplitudes
/// and phases from the place it was made, so an injury is the same shape every run and two are not.
#[derive(Clone, Copy, Debug)]
struct Margin {
    seed: u32,
    three: Vec2,
    five: Vec2,
}

impl Margin {
    /// The margin an injury at `uv` has; `salt` keeps a bruise's from a burn's at the same place.
    fn at(uv: Vec2, salt: u32) -> Self {
        let seed = uv.x.to_bits() ^ uv.y.to_bits().rotate_left(16) ^ salt;
        let h = |salt: u32| 2.0 * crate::bloodstain::hash_f32(seed ^ salt) - 1.0;
        Self { seed, three: Vec2::new(h(0x3A11), h(0x3A12)), five: Vec2::new(h(0x5B21), h(0x5B22)) }
    }

    /// **The injury's radius in direction `dir`**, as a multiple of the round one: within `wobble`
    /// of one. The waves are the third and fifth harmonics of the direction's angle, taken as powers
    /// of `dir` read as a complex number, so no angle is ever computed.
    fn scale(self, dir: Vec2, wobble: f32) -> f32 {
        let mul = |a: Vec2, b: Vec2| Vec2::new(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
        let d2 = mul(dir, dir);
        let d3 = mul(d2, dir);
        let d5 = mul(d3, d2);
        let wave = 0.6 * self.three.dot(d3) + 0.4 * self.five.dot(d5);
        1.0 + wobble * (wave / core::f32::consts::SQRT_2).clamp(-1.0, 1.0)
    }
}

/// Salts a bruise's [`Margin`] apart from a burn's at the same place.
const BRUISE_SALT: u32 = 0xB205_E000;

/// **A bruise's colour in each of the model's shells**, linear. The model is a step per shell, so
/// these are every colour it can show at its age, read once a repaint rather than once a texel.
fn bruise_shells(bruise: &crate::bloodstain::Bruise) -> [[f32; 3]; crate::bloodstain::bruise::SHELLS] {
    use crate::bloodstain::bruise::DR_MM;
    core::array::from_fn(|i| linear(bruise.srgb_through_at((i as f32 + 0.5) * DR_MM)))
}

/// **What a bruise shows `r_mm` from its centre in the unit direction `dir`**: the linear ratio the
/// skin is multiplied by, and how much of it shows. `shells` is [`bruise_shells`].
///
/// The colour is the model's — the bruise seen through the whole skin, over the same skin with
/// nothing in it (the outermost shell), so the caller's skin tone is what the chromophores modulate —
/// read at a radius stretched by the bruise's [`Margin`] and darkened more or less by its mottle. Far
/// out, where the model holds no chromophore, the ratio is one and nothing shows. Until 0.5.44 the
/// ratio was read at the true radius with no mottle and shown six times as strongly as it changed,
/// so a bruise was a disc of one colour with a hard rim.
fn bruise_texel(shells: &[[f32; 3]; crate::bloodstain::bruise::SHELLS], margin: Margin, r_mm: f32, dir: Vec2) -> ([f32; 3], f32) {
    use crate::bloodstain::bruise::DR_MM;
    let r = r_mm / margin.scale(dir, BRUISE_WOBBLE);
    // The shell `srgb_through_at` reads: the one the radius falls in, the outermost past the grid.
    let shell = if r.is_finite() { (r / DR_MM) as usize } else { 0 };
    let (Some(here), Some(blank)) = (shells.get(shell).or(shells.last()), shells.last()) else {
        return ([1.0; 3], 0.0);
    };
    let at = dir * (r_mm / BRUISE_MOTTLE_MM);
    let tile = 65_536.0;
    let coarse = crate::cross_section::value_noise(at.x, at.y, tile, margin.seed ^ 0x40B7);
    let fine = crate::cross_section::value_noise(at.x * 2.3, at.y * 2.3, tile, margin.seed ^ 0x9C2D);
    let mottle = 1.0 + BRUISE_MOTTLE * (2.0 * (0.65 * coarse + 0.35 * fine) - 1.0);
    // The mottle scales the optical density, as a pool that much thicker or thinner would: a power of
    // the ratio, which never clamps, so on average the bruise is as dark as the model says.
    let ratio = core::array::from_fn(|c| (here[c] / blank[c].max(1.0e-4)).clamp(0.0, 1.0).powf(mottle));
    let change = ratio.iter().fold(0.0f32, |m, &v| m.max(1.0 - v));
    // Shown wherever it changes the skin at all; how much it changes it is the ratio's to say.
    (ratio, (change * 50.0).clamp(0.0, 1.0))
}

/// **The thickest film of blood exposed tissue wears**, mm, on a broken body's cut face and, since
/// 0.5.42, on a crater's walls where a skin is peeled: `1.0`, the blood table's last column, where
/// whole blood is opaque and the tissue under it no longer shows. A quarter of it lies over the fat at
/// the rim. At `0.6` the faces read as candy red over the fat. Before 0.5.38 a cut face was drawn
/// bare, and a fat band read as cheese; before 0.5.42 a crater was ringed by bare yellow fat. This
/// crate's own number.
pub const CUT_FACE_FILM_MM: f32 = 1.0;

/// **Place a broken body's pieces in the world**, each with the velocity the preset launches it at.
///
/// A thrown piece flies away from the blow and upward, slower the bigger it is. A piece whose body
/// lost its footing was not hit, so nothing throws it: it slumps outward from the fallen pieces'
/// centre, slowly and horizontally, so the heap spreads instead of stacking in one column, and
/// gravity in [`fly`] brings it down. A piece that stood has no velocity.
fn place(leaves: Vec<Piece>, root_xf: &GlobalTransform, at: Vec3, posing: Option<(&Posing, &Assets<Mesh>)>) -> Vec<GorePiece> {
    // Where each piece is, and how it is turned: carried by the bone of the skeleton nearest it, for a
    // body that was posed when it broke, and placed by the root otherwise.
    let frames: Vec<(Vec3, Quat, Vec3)> = leaves
        .iter()
        .map(|p| match posing.and_then(|(posing, meshes)| posing.carry(meshes, p.center_local)) {
            Some(bone) => {
                let (scale, rotation, _) = bone.to_scale_rotation_translation();
                (bone.transform_point3(p.center_local), rotation, scale)
            }
            None => (root_xf.transform_point(p.center_local), root_xf.rotation(), root_xf.scale()),
        })
        .collect();
    let fell: Vec<Vec3> = leaves.iter().zip(&frames).filter(|(p, _)| p.fate == PieceFate::Fell).map(|(_, f)| f.0).collect();
    let middle = if fell.is_empty() { Vec3::ZERO } else { fell.iter().sum::<Vec3>() / fell.len() as f32 };
    leaves
        .into_iter()
        .zip(frames)
        .map(|(p, (centre, rotation, scale))| {
            let size = p.half_extents.length();
            let velocity = match p.fate {
                PieceFate::Thrown => ((centre - at).normalize_or_zero() + Vec3::new(0.0, 0.6, 0.0)) * (1.5 / (1.0 + size)),
                PieceFate::Fell => Vec3::new(centre.x - middle.x, 0.0, centre.z - middle.z).normalize_or_zero() * (0.5 / (1.0 + size)),
                PieceFate::Stood => Vec3::ZERO,
            };
            GorePiece {
                outer: p.outer,
                cap: p.cap,
                transform: Transform::from_translation(centre).with_rotation(rotation).with_scale(scale),
                half_extents: p.half_extents,
                velocity,
                fate: p.fate,
            }
        })
        .collect()
}

/// **A skinned body as it stood when it broke**: its skeleton's pose and the meshes whose vertices say
/// which bone carries a place on it. A fracture is baked from a mesh asset, which is the bind pose, so
/// without this its pieces appear there: a T-pose burst from a body that was running. With it each
/// piece is carried rigidly by the bone of the vertex nearest its centre, to where that bone is, turned
/// as that bone is turned.
///
/// **The contract on the game: bake the fracture in the skinned mesh's own space**, `Mat4::IDENTITY`
/// in `fracture_mesh`'s parts, since a skinned mesh is drawn from its joints and its entity's transform
/// plays no part. Every skinned mesh under the root that shares the hit part's joints is searched.
struct Posing {
    pose: Pose,
    parts: Vec<Handle<Mesh>>,
}

impl Posing {
    /// The matrix, bind space to world, of the bone that carries `place`: the dominant joint of the
    /// vertex nearest it over every part. Ties in distance go to the lower joint, so the answer does
    /// not depend on the order the parts were found in. `None` when no part has a rig or a vertex.
    fn carry(&self, meshes: &Assets<Mesh>, place: Vec3) -> Option<Mat4> {
        let mut best: Option<(f32, u16)> = None;
        for handle in &self.parts {
            let Some(mesh) = meshes.get(handle) else { continue };
            let Some(rig) = Rig::of(mesh) else { continue };
            let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION) else { continue };
            for (vertex, position) in positions.iter().enumerate() {
                let distance = Vec3::from_array(*position).distance_squared(place);
                let Some(joint) = rig.dominant_joint(vertex) else { continue };
                if best.is_none_or(|(d, j)| distance < d || (distance == d && joint < j)) {
                    best = Some((distance, joint));
                }
            }
        }
        let (_, joint) = best?;
        self.pose.blend([joint, 0, 0, 0], [1.0, 0.0, 0.0, 0.0])
    }
}

/// Spawn placed pieces: skin outside, a banded cap inside. The ones that stood stay [`Standing`];
/// the rest are [`Flying`] at their velocity. Every cut face bleeds from `tick` ([`CutFaceBleed`]).
fn spawn_pieces(commands: &mut Commands, meshes: &Assets<Mesh>, tick: u32, pieces: &[GorePiece], skin: &Handle<StandardMaterial>, cap: &Handle<FleshMaterial>) {
    for fate in [PieceFate::Stood, PieceFate::Thrown, PieceFate::Fell] {
        for piece in pieces.iter().filter(|p| p.fate == fate) {
            let parts = [(piece.outer.clone(), None), (piece.cap.clone(), Some(cap.clone()))];
            for (mesh, cap) in parts {
                let Some(mesh) = mesh else { continue };
                let pours = cap.as_ref().and_then(|_| meshes.get(&mesh)).and_then(|m| CutFaceBleed::of(m, &piece.transform, tick));
                let mut e = match cap {
                    Some(cap) => commands.spawn((Mesh3d(mesh), MeshMaterial3d(cap), piece.transform)),
                    None => commands.spawn((Mesh3d(mesh), MeshMaterial3d(skin.clone()), piece.transform)),
                };
                if fate == PieceFate::Stood {
                    e.insert(Standing);
                } else {
                    e.insert(Flying(piece.velocity));
                }
                if let Some(pours) = pours {
                    e.insert(pours);
                }
            }
        }
    }
}

/// **Break a runtime body**: upload its leaves' meshes, part them by the modes, write
/// [`GoreFractured`], and — when the preset owns the pieces — spawn them and despawn the body.
fn throw_bodies(
    mut commands: Commands,
    clock: Res<GoreClock>,
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
        let pieces = place(leaves, xf, throw.at, throw.posing.as_ref().map(|p| (p, &*meshes)));
        if throw.spawn {
            spawn_pieces(&mut commands, &meshes, clock.0, &pieces, &throw.skin, &throw.cap);
            commands.entity(entity).despawn();
        } else {
            commands.entity(entity).remove::<ThrowNow>();
        }
        fractured.write(GoreFractured { subject: entity, at: throw.at, pieces, skin: throw.skin.clone(), cap: throw.cap.clone() });
    }
}

/// **Pulse every open wound onto its wetmap** until the clot.
#[allow(clippy::too_many_arguments)]
fn bleed(
    clock: Res<GoreClock>,
    dials: Res<GoreDials>,
    wet_settings: Option<Res<WetSettings>>,
    meshes: Res<Assets<Mesh>>,
    rigging: Rigging,
    mut sites: Bleeders,
    mut floors: Query<(&Gore, &Mesh3d, &GlobalTransform, &mut WetCanvas), With<FloorTag>>,
    policy: Option<Res<GorePolicy>>,
) {
    let tick = clock.0;
    let drips = policy.as_deref().is_none_or(|p| p.draws_blood() && p.blood_decals);
    let edges = edge_settings(&dials, wet_settings.as_deref());
    for (gore, mut wounds, xf, mesh3d, skin, mut wet, run) in &mut sites {
        // Which way blood runs on this atlas now, for the lip a cut beads over.
        let downhill = run.map_or(gore.gravity_uv, |r| r.0);
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
            let texels = wet.size();
            // **A wound wells where its blood spills**: over a cut's lower lip, or the downhill rim of
            // a hole. The pulse is shared between `BLEED_WELLS` beads by area, each at its own fixed
            // place and drawn out down the slope, narrow: a run goes as far as its blood over its
            // width. Until 0.5.35 a hole pulsed one ellipse at its centre, and the blood ran from it as
            // one column the wound's own width.
            let bead = r / (BLEED_WELLS as f32).sqrt();
            let run = if downhill == Vec2::ZERO { [0.0, 1.0] } else { downhill.normalize_or_zero().to_array() };
            for k in 0..BLEED_WELLS {
                let shape = StainShape {
                    major: bead * 2.6,
                    minor: bead * 0.6,
                    spines: 0,
                    satellites: 0,
                    direction: run,
                    seed: site.bleed.seed ^ tick ^ k.wrapping_mul(0x27D4_EB2F),
                };
                // Centred half a bead out past the lip or the rim, so it sits on the skin.
                let (edge, out) = site.well(k, downhill, gore.density());
                let at = site.uv + edge + out * (0.5 * bead * gore.density());
                wet.paint_uv_with(at, &shape_on(gore, &shape, texels), tick, &edges);
            }
            // What runs off the body onto the world is the world's share of the blood.
            if !drips {
                continue;
            }
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

/// **Put down every [`GoreStep`]**: lift from the floor under the sole, then print what the sole
/// carried, then carry what it lifted. Lifting first is what lets a sole walk its blood off: printed
/// first, it would pick its own wet print back up and never fade.
#[allow(clippy::too_many_arguments)]
fn take_steps(
    mut commands: Commands,
    mut steps: MessageReader<GoreStep>,
    clock: Res<GoreClock>,
    dials: Res<GoreDials>,
    wet_settings: Option<Res<WetSettings>>,
    meshes: Res<Assets<Mesh>>,
    treads: Query<&Tread>,
    mut floors: Query<(Entity, &Gore, &Mesh3d, &GlobalTransform, &mut WetCanvas), With<FloorTag>>,
    policy: Option<Res<GorePolicy>>,
) {
    let tick = clock.0;
    let prints = policy.as_deref().is_none_or(|p| p.draws_blood() && p.blood_decals);
    let edges = edge_settings(&dials, wet_settings.as_deref());
    // Each foot's load as the tick's steps change it, in the order they were sent.
    let mut carried: Vec<(Entity, f32)> = Vec::new();
    for step in steps.read() {
        if !prints || !step.at.is_finite() || !step.heading.is_finite() {
            continue;
        }
        let held = carried
            .iter()
            .find(|(e, _)| *e == step.foot)
            .map(|c| c.1)
            .or_else(|| treads.get(step.foot).ok().map(|t| t.load))
            .unwrap_or(0.0);
        let toe = Vec3::new(step.heading.x, 0.0, step.heading.z).normalize_or_zero();
        let from = step.at + Vec3::Y * STEP_REACH_M;
        // The floor under the sole: the nearest a ray straight down meets within reach.
        let mut under: Option<(Entity, Landing)> = None;
        for (entity, _, mesh3d, xf, _) in &floors {
            let Some(mesh) = meshes.get(&mesh3d.0) else { continue };
            let Some(landing) = ray_hit(mesh, xf, from, Vec3::NEG_Y) else { continue };
            let drop = xf.transform_point(landing.point).distance(from);
            if drop <= 2.0 * STEP_REACH_M && under.as_ref().is_none_or(|(_, best)| drop < best.t) {
                under = Some((entity, Landing { t: drop, ..landing }));
            }
        }
        let mut load = held;
        if let Some((entity, landing)) = under
            && let Ok((_, gore, mesh3d, xf, mut wet)) = floors.get_mut(entity)
            && let Some(mesh) = meshes.get(&mesh3d.0)
        {
            // The toe's way on the canvas: where a ray a little toward it lands, or the opposite of
            // where one a little back does, at a floor's edge.
            let ahead = |sign: f32| ray_hit(mesh, xf, from + toe * (0.05 * sign), Vec3::NEG_Y).map(|l| (l.uv - landing.uv) * sign);
            let heading = ahead(1.0).or_else(|| ahead(-1.0)).map_or(Vec2::ZERO, |d| d.normalize_or_zero());
            let print = Print {
                length: step.sole.length_m * gore.density(),
                width: step.sole.width_m * gore.density(),
                heading: heading.to_array(),
                seed: step.sole.tread,
            };
            let picked = wet.lift_uv(landing.uv, &print, TREAD_LIFT, tick, &edges);
            wet.print_uv(landing.uv, &print, held, tick);
            load = (held * (1.0 - TREAD_SHED) + picked).clamp(0.0, 1.0);
        }
        match carried.iter_mut().find(|(e, _)| *e == step.foot) {
            Some(c) => c.1 = load,
            None => carried.push((step.foot, load)),
        }
    }
    for (foot, load) in carried {
        if let Ok(mut entity) = commands.get_entity(foot) {
            entity.try_insert(Tread { load });
        }
    }
}

/// **Pour every cut face of a broken body onto the floor beneath it**, a pulse at a time, until its
/// perfusion runs out ([`CutFaceBleed`]). Each pulse is a stain [`CUT_FACE_POUR`] of the radius a
/// wound of the face's area paints, cast straight down from above the face's centre as the piece is
/// now, so a piece in the air leaves a trail and one at rest grows a pool that the floor's wetmap
/// spreads. A tier that paints no blood pours none.
#[allow(clippy::too_many_arguments)]
fn pour_cut_faces(
    mut commands: Commands,
    clock: Res<GoreClock>,
    dials: Res<GoreDials>,
    wet_settings: Option<Res<WetSettings>>,
    meshes: Res<Assets<Mesh>>,
    faces: Query<(Entity, &CutFaceBleed, &Transform)>,
    mut floors: Query<(&Gore, &Mesh3d, &GlobalTransform, &mut WetCanvas), With<FloorTag>>,
    policy: Option<Res<GorePolicy>>,
) {
    let tick = clock.0;
    let drips = policy.as_deref().is_none_or(|p| p.draws_blood() && p.blood_decals);
    let edges = edge_settings(&dials, wet_settings.as_deref());
    for (entity, face, xf) in &faces {
        if crate::bloodstain::bleed::envelope(face.bleed.age(tick), &dials.blood) <= 0.0 {
            commands.entity(entity).remove::<CutFaceBleed>();
            continue;
        }
        if !drips {
            continue;
        }
        let Some(pulse) = crate::bloodstain::bleed::pulse_wound(&face.bleed, &face.wound, tick, dials.hz, &dials.blood) else {
            continue;
        };
        let reach = (face.wound.area / core::f32::consts::PI).sqrt().max(0.002);
        let r = reach * (0.6 + pulse.severity) * CUT_FACE_POUR;
        let shape = StainShape { major: r * 1.25, minor: r, spines: 0, satellites: 3, direction: [0.0, 1.0], seed: face.bleed.seed ^ tick };
        // **Where the blood leaves the face**: off its side by as far as the face reaches, so a face
        // turned sideways pours beside its piece and one turned up or down pours round its foot. From
        // above, so a piece lying on the floor, its centre at the floor's height, still casts onto it.
        // Pieces fly without turning, so the face's normal is still the one it broke with.
        let at = xf.transform_point(face.centre_local);
        let n = Vec3::from_array(face.wound.normal);
        let out = Vec3::new(n.x, 0.0, n.z) * reach;
        let from = Vec3::new(at.x + out.x, at.y.max(dials.floor_y) + 0.25, at.z + out.z);
        for (floor_gore, floor_mesh, floor_xf, mut floor_wet) in &mut floors {
            let Some(floor_mesh) = meshes.get(&floor_mesh.0) else { continue };
            let texels = floor_wet.size();
            let _ = floor_wet.paint_world_with(floor_mesh, floor_xf, from, Vec3::NEG_Y, &shape_on(floor_gore, &shape, texels), tick, &edges);
        }
    }
}

/// **Age every bruise** by the dial's hours per tick and repaint its dermis when a model step lands,
/// oldest first, so where two overlap the younger is painted last.
fn age_bruises(
    dials: Res<GoreDials>,
    mut bruised: Query<(&Gore, &mut Bruising, &mut DermisCanvas)>,
) {
    let per_tick = dials.bruise_hours_per_tick.max(0.0) / crate::bloodstain::bruise::STEP_H;
    for (gore, mut bruising, mut dermis) in &mut bruised {
        let mut stepped = false;
        for site in &mut bruising.sites {
            site.carry += per_tick;
            let steps = site.carry.floor() as u32;
            if steps > 0 {
                site.carry -= steps as f32;
                site.bruise.advance(steps);
                stepped = true;
            }
        }
        if !stepped {
            continue;
        }
        let mm_per_uv = 1000.0 / gore.density().max(1.0e-6);
        let radius_uv = crate::bloodstain::bruise::RADIUS_MM / mm_per_uv;
        let centres: Vec<Vec2> = bruising.sites.iter().map(|site| site.uv).collect();
        let looks: Vec<_> = bruising.sites.iter().map(|site| (bruise_shells(&site.bruise), Margin::at(site.uv, BRUISE_SALT))).collect();
        dermis.draw_bruises(&centres, radius_uv * (1.0 + BRUISE_WOBBLE), mm_per_uv, |i, r_mm, dir| {
            looks.get(i).map_or(([1.0; 3], 0.0), |(shells, margin)| bruise_texel(shells, *margin, r_mm, dir))
        });
    }
}

/// Decode encoded sRGB to linear.
fn linear(c: [f32; 3]) -> [f32; 3] {
    let d = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    [d(c[0]), d(c[1]), d(c[2])]
}

/// **Widen every landing on cloth** along the Lucas–Washburn front until it stops moving, at the UV
/// it landed on, so a sheet that moves keeps soaking where the blood is. The front is wicked
/// ([`WetCanvas::wick_uv`]): ragged at the weave and paler at its edge, where a stamp grown in place
/// left a flat patch with a cut-out's rim.
fn soak_cloth(clock: Res<GoreClock>, dials: Res<GoreDials>, mut cloth: Query<(&Gore, &mut Soaking, &mut WetCanvas)>) {
    let tick = clock.0;
    let sheet = crate::bloodstain::wick::Sheet::default();
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
                wet.wick_uv(uv, &shape_on(gore, &grown, texels), tick);
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

/// Upload every dermis canvas and every cloth's holes that changed. A cloth's first hole switches its
/// material from opaque to a mask, which is what makes Bevy run the flesh prepass shader that cuts the
/// hole out of the depth prepass and the shadow maps too.
fn flush_dermis(
    images: Option<ResMut<Assets<Image>>>,
    flesh: Option<ResMut<Assets<FleshMaterial>>>,
    mut canvases: Query<&mut DermisCanvas>,
    mut cloths: Query<(&mut ClothHoles, &MeshMaterial3d<FleshMaterial>)>,
) {
    let Some(mut images) = images else { return };
    let mut flesh = flesh;
    for (mut holes, material) in &mut cloths {
        if holes.canvas.dirty
            && !holes.masked
            && let Some(flesh) = flesh.as_mut()
            && let Some(mut material) = flesh.get_mut(&material.0)
        {
            if matches!(material.base.alpha_mode, AlphaMode::Opaque) {
                material.base.alpha_mode = AlphaMode::Mask(0.5);
            }
            holes.masked = true;
        }
        holes.canvas.flush(&mut images);
    }
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

/// Run and dry every wetmap on the fixed tick. A cloth runs only past what its weave holds
/// ([`GoreDials::cloth_holds`]).
///
/// **Every canvas at once.** A canvas's tick reads its own blood, its own mesh, pose and wounds, and
/// the settings, and writes only itself, so the canvases are ticked across the compute pool and the
/// bytes are the same in any order. Where Bevy's `multi_threaded` feature is off, as on the web, it is
/// the loop it was. Before 0.5.46 it was a loop everywhere, and in a crowd it was the most expensive
/// system on every tick, hit or not.
fn tick_canvases(
    clock: Res<GoreClock>,
    dials: Res<GoreDials>,
    settings: Option<Res<WetSettings>>,
    meshes: Option<Res<Assets<Mesh>>>,
    rigging: Rigging,
    mut canvases: Canvases,
) {
    let Some(settings) = settings else { return };
    // A weave moves blood by wicking (`soak_cloth`), not as a film creeping over it, so its creep is a
    // quarter of a film's: at the full rate the spread blurred every front the wick frayed.
    let cloth = WetSettings {
        drip_rate: settings.drip_rate.max(dials.cloth_holds),
        spread_rate: settings.spread_rate * 0.25,
        ..settings.clone()
    };
    let (tick, meshes, rigging, settings) = (clock.0, meshes.as_deref(), &rigging, &*settings);
    canvases.par_iter_mut().for_each(|(gore, mut wet, mesh3d, wounds, skin, run)| {
        let gravity = match run {
            Some(mut run) => {
                if let (Some(skin), Some(wounds), Some(mesh)) = (skin, wounds, meshes.and_then(|m| m.get(&mesh3d.0)))
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
        wet.tick(tick, gravity, if gore.kind == GoreKind::Cloth { &cloth } else { settings });
    });
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
            .add_message::<GoreReset>()
            .add_message::<GoreStep>()
            .add_message::<GoreFractured>()
            .add_message::<BoneExposed>()
            .add_message::<Wounded>()
            .init_resource::<FractureCache>()
            .init_resource::<WetSettings>()
            .init_resource::<FlaySettings>()
            .add_systems(FixedUpdate, scene_gore.before(dress))
            .add_systems(FixedUpdate, (advance_clock, dress, reset_gore, take_hits, throw_bodies, bleed, pour_cut_faces, take_steps, age_bruises, soak_cloth, tick_canvases, flush_dermis, fly).chain());
        let images = app.world_mut().resource_mut::<Assets<Image>>().reserve_handle();
        app.world_mut().resource_mut::<FleshTables>().luts = images;
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

    /// **A shot through a shirt holes the shirt where it crossed it, and nothing behind the body.** A
    /// skin box inside a slightly larger cloth box, a cloth sheet behind both, one impact from the front:
    /// the shirt is open where the ray crossed it and nowhere a blow's reach away, its material is
    /// switched to a mask so the hole is cut, and the sheet the shot never reached is whole and opaque.
    /// Two runs open the same texels.
    #[test]
    fn a_shot_through_a_shirt_holes_the_shirt_and_not_what_is_behind() {
        let run = || {
            let mut app = headless();
            let world = app.world_mut();
            let (body, shirt, sheet) = {
                let mut meshes = world.resource_mut::<Assets<Mesh>>();
                (
                    meshes.add(Cuboid::new(0.6, 0.6, 0.2).mesh().build()),
                    meshes.add(Cuboid::new(0.62, 0.62, 0.22).mesh().build()),
                    meshes.add(Plane3d::new(Vec3::Z, Vec2::splat(0.5)).mesh().build()),
                )
            };
            let body = world.spawn((Mesh3d(body), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Torso))).id();
            let shirt = world.spawn((Mesh3d(shirt), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::cloth())).id();
            let behind = Transform::from_xyz(0.0, 0.0, -0.5);
            let sheet = world.spawn((Mesh3d(sheet), behind, GlobalTransform::from(behind), Gore::cloth())).id();
            app.world_mut().run_schedule(FixedUpdate);
            app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, 0.03, 5.0));
            app.world_mut().run_schedule(FixedUpdate);
            let holes = |e: Entity| app.world().get::<ClothHoles>(e).map(|h| (h.is_holed(), h.is_open_at(Vec2::splat(0.5)), h.is_open_at(Vec2::new(0.5, 0.75)), h.canvas.px.clone()));
            let mask = |e: Entity| {
                let handle = app.world().get::<MeshMaterial3d<FleshMaterial>>(e).map(|m| m.0.clone());
                handle.and_then(|h| app.world().resource::<Assets<FleshMaterial>>().get(&h).map(|m| m.base.alpha_mode))
            };
            (holes(shirt), holes(sheet), mask(shirt), mask(sheet))
        };
        let (shirt, sheet, shirt_mode, sheet_mode) = run();
        let Some((holed, centre, aside, px)) = shirt else { panic!("the shirt was not dressed with holes") };
        assert!(holed && centre, "the shirt is not open where the shot crossed it");
        assert!(!aside, "the shirt is open a quarter of its face from where the shot crossed it");
        assert!(matches!(shirt_mode, Some(AlphaMode::Mask(_))), "a holed shirt still draws opaque: {shirt_mode:?}");
        assert!(sheet.is_some_and(|s| !s.0), "a sheet behind the body was holed by a shot the body stopped");
        assert!(matches!(sheet_mode, Some(AlphaMode::Opaque)), "an unholed cloth was switched off the opaque pipeline: {sheet_mode:?}");
        assert_eq!(run().0.map(|s| s.3), Some(px), "two runs opened different texels");
    }

    /// **A shot that goes through opens an exit and throws its spatter on the wall behind; one that
    /// stops does neither.** A skin box between the muzzle and a wall, a cloth curtain between the box
    /// and the wall. Through: two bleeding wounds, the second on the box's far face; the wall wetted;
    /// the curtain holed; two `Wounded`, the exit's facing the way the shot went. Stopped: one wound,
    /// the wall dry, the curtain whole, one `Wounded`.
    #[test]
    fn a_shot_that_goes_through_opens_an_exit_and_spatters_the_wall_behind() {
        let run = |through: bool| {
            let mut app = headless();
            let world = app.world_mut();
            let (body, panel) = {
                let mut meshes = world.resource_mut::<Assets<Mesh>>();
                (meshes.add(Cuboid::new(0.6, 0.6, 0.2).mesh().build()), meshes.add(Plane3d::new(Vec3::Z, Vec2::splat(1.0)).mesh().build()))
            };
            let body = world.spawn((Mesh3d(body), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Torso))).id();
            let (at_curtain, at_wall) = (Transform::from_xyz(0.0, 0.0, -0.4), Transform::from_xyz(0.0, 0.0, -1.0));
            let curtain = world.spawn((Mesh3d(panel.clone()), at_curtain, GlobalTransform::from(at_curtain), Gore::cloth())).id();
            let wall = world.spawn((Mesh3d(panel), at_wall, GlobalTransform::from(at_wall), Gore::wall())).id();
            app.world_mut().run_schedule(FixedUpdate);
            let hit = GoreHit::impact(body, Vec3::new(0.05, 0.02, 2.0), Vec3::NEG_Z, 0.03, 5.0);
            app.world_mut().write_message(if through { hit.through() } else { hit });
            app.world_mut().run_schedule(FixedUpdate);
            let w = app.world();
            let sites: Vec<Vec3> = w.get::<Wounds>(body).map(|o| o.0.iter().map(|s| s.at_local).collect()).unwrap_or_default();
            let wall_wet = w.get::<WetCanvas>(wall).map(|c| c.wetted_area()).unwrap_or(0.0);
            let holed = w.get::<ClothHoles>(curtain).is_some_and(ClothHoles::is_holed);
            let mut cursor = w.resource::<Messages<Wounded>>().get_cursor();
            let normals: Vec<Vec3> = cursor.read(w.resource::<Messages<Wounded>>()).map(|m| m.normal).collect();
            (sites, wall_wet, holed, normals)
        };
        let (sites, through_wet, holed, normals) = run(true);
        assert_eq!(sites.len(), 2, "a shot through opened {} wounds: {sites:?}", sites.len());
        assert!(sites[0].z > 0.09 && sites[1].z < -0.09, "the exit is not on the far face: {sites:?}");
        assert!(through_wet > 0.0, "the exit threw nothing on the wall behind");
        assert!(holed, "the curtain the shot went on through was not holed");
        assert_eq!(normals.len(), 2, "a shot through announced {} wounds", normals.len());
        assert!(normals[1].z < -0.9, "the exit's normal does not face the way the shot went: {:?}", normals[1]);
        assert_eq!(run(true).1.to_bits(), through_wet.to_bits(), "two runs spattered the wall differently");
        let (sites, wall_wet, holed, normals) = run(false);
        assert_eq!(sites.len(), 1, "a shot that stopped opened an exit");
        assert_eq!(wall_wet, 0.0, "a shot that stopped spattered the wall behind the body");
        assert!(!holed, "a shot that stopped holed the curtain behind the body");
        assert_eq!(normals.len(), 1);
    }

    /// **A blade cuts a slit in the cloth over the skin it opens, along the cut.** A sheet of cloth in
    /// front of a skin box, a slash across it along world `X`, which on this plane is the atlas's `u`.
    /// The cloth is open at the middle of the cut; the opening runs at least twice as far along `u` as
    /// across it, where a bullet's round hole runs as far either way; and its length is about the blade's.
    #[test]
    fn a_slash_cuts_a_slit_along_the_blade_in_the_cloth_over_the_wound() {
        let mut app = headless();
        let world = app.world_mut();
        let (body, sheet) = {
            let mut meshes = world.resource_mut::<Assets<Mesh>>();
            (meshes.add(Cuboid::new(0.6, 0.6, 0.2).mesh().build()), meshes.add(Plane3d::new(Vec3::Z, Vec2::splat(0.5)).mesh().build()))
        };
        let body = world.spawn((Mesh3d(body), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Torso))).id();
        let at = Transform::from_xyz(0.0, 0.0, 0.15);
        let cloth = world.spawn((Mesh3d(sheet), at, GlobalTransform::from(at), Gore::cloth().with_canvas(128))).id();
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().write_message(GoreHit::slash(body, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, Vec3::X, 0.1, 5.0));
        app.world_mut().run_schedule(FixedUpdate);
        let Some(holes) = app.world().get::<ClothHoles>(cloth) else { panic!("the cloth was not dressed") };
        assert!(holes.is_open_at(Vec2::splat(0.5)), "the cloth is not open at the middle of the cut");
        let n = 128;
        let (mut us, mut vs) = (Vec::new(), Vec::new());
        for y in 0..n {
            for x in 0..n {
                if holes.is_open_at(Vec2::new((x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32)) {
                    us.push(x);
                    vs.push(y);
                }
            }
        }
        let extent = |v: &[i32]| v.iter().max().zip(v.iter().min()).map_or(0, |(a, b)| a - b + 1);
        let (along, across) = (extent(&us), extent(&vs));
        assert!(along >= 2 * across, "the slit is {along} texels along the blade and {across} across it");
        // A 0.2 m blade on a 1 m sheet of 128 texels is 25.6 texels end to end; the lens closes before
        // its tips.
        assert!((14..=27).contains(&along), "the slit is {along} texels long for a 25.6-texel blade");
    }

    /// **A slash leaves its blood along the cut, not as a disc its own length across.** A slash along
    /// world `X` across the front of a skin box, which on that face is the atlas's `u`. The blood it
    /// leaves runs at least four times as far along `u` as down `v`; and the wound it announces is the
    /// cut's, a tenth of a disc the blade's half-length in radius or less.
    #[test]
    fn a_slash_bleeds_along_its_cut_not_from_a_disc() {
        let mut app = headless();
        let body = {
            let world = app.world_mut();
            let mesh = world.resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.6, 0.6, 0.2).mesh().build());
            world.spawn((Mesh3d(mesh), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Torso))).id()
        };
        app.world_mut().run_schedule(FixedUpdate);
        let radius = 0.1;
        app.world_mut().write_message(GoreHit::slash(body, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, Vec3::X, radius, 5.0));
        app.world_mut().run_schedule(FixedUpdate);
        let Some(wet) = app.world().get::<WetCanvas>(body) else { panic!("the body was not dressed") };
        let n = wet.size();
        let (mut us, mut vs) = (Vec::new(), Vec::new());
        for y in 0..n {
            for x in 0..n {
                if wet.amount_at(x, y) > 0 {
                    us.push(x as i64);
                    vs.push(y as i64);
                }
            }
        }
        let extent = |v: &[i64]| v.iter().max().zip(v.iter().min()).map_or(0, |(a, b)| a - b + 1);
        let (along, down) = (extent(&us), extent(&vs));
        assert!(along > 0, "the slash left no blood");
        assert!(along >= 4 * down, "the slash's blood runs {along} texels along the cut and {down} down it");
        let (wounded, _) = announced(&app);
        let Some(cut) = wounded.last() else { panic!("the slash announced nothing") };
        let disc = core::f32::consts::PI * radius * radius;
        assert!(cut.area > 0.0 && cut.area <= disc / 10.0, "the slash announced {} m², against {disc} for a disc its half-length across", cut.area);
    }

    /// **A hole bleeds over its downhill rim in runs, not as one column its own width.** A shot at a
    /// skin box, four seconds of pulses, and the canvas read across in rows from two and a half to
    /// three and a half radii below the wound, past what the hit's own stain covers: some row there
    /// holds two or more separate runs. One stream from the wound's centre is one run in every row.
    #[test]
    fn a_hole_bleeds_in_runs_from_its_lower_rim() {
        let mut app = headless();
        let body = {
            let world = app.world_mut();
            let mesh = world.resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.6, 0.6, 0.2).mesh().build());
            world.spawn((Mesh3d(mesh), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Torso).with_canvas(256))).id()
        };
        app.world_mut().run_schedule(FixedUpdate);
        let radius = 0.05;
        app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, radius, 4.0));
        for _ in 0..240 {
            app.world_mut().run_schedule(FixedUpdate);
        }
        let world = app.world();
        let (Some(wet), Some(gore), Some(wounds)) = (world.get::<WetCanvas>(body), world.get::<Gore>(body), world.get::<Wounds>(body)) else {
            panic!("the body was not dressed, or the shot opened no wound")
        };
        let Some(site) = wounds.0.first() else { panic!("the shot opened no wound") };
        let n = wet.size() as f32;
        let r = radius * gore.density() * n;
        let (cx, cy) = (site.uv.x * n, site.uv.y * n);
        let mut most = 0;
        for y in (cy + 2.5 * r) as u32..(cy + 3.5 * r) as u32 {
            let (mut runs, mut inside) = (0, false);
            for x in (cx - 2.0 * r).max(0.0) as u32..((cx + 2.0 * r) as u32).min(wet.size()) {
                let here = wet.amount_at(x, y) > 0;
                runs += usize::from(here && !inside);
                inside = here;
            }
            most = most.max(runs);
        }
        assert!(most >= 2, "below the wound the blood is at most {most} run(s) wide in any row");
    }

    /// **A droplet stops at the first thing it reaches, unless there is a hole in it.** A shot at a
    /// skin box throws its back spatter toward the muzzle, where a wall stands. With nothing between,
    /// the wall is wetted. With a second wall in front of it, the first catches the spray and the wall
    /// behind stays dry. With a cloth curtain in front instead, the shot holes the curtain on its way
    /// in and the spray reaches the wall behind only through that hole: some, and less than with
    /// nothing between. And a droplet a wall caught does not also land on the floor: with no wall at
    /// all, the floor far out takes more.
    #[test]
    fn a_droplet_stops_at_the_first_surface_it_reaches_unless_it_finds_a_hole() {
        // `between`: 0 nothing, 1 a wall, 2 a cloth curtain; `back`: whether the far wall stands.
        let run = |between: u8, back: bool| {
            let mut app = headless();
            let world = app.world_mut();
            let (body, panel, ground) = {
                let mut meshes = world.resource_mut::<Assets<Mesh>>();
                (
                    meshes.add(Cuboid::new(0.6, 0.6, 0.2).mesh().build()),
                    meshes.add(Plane3d::new(Vec3::Z, Vec2::splat(1.5)).mesh().build()),
                    // Wide enough for a droplet at 40 m/s to come down on it.
                    meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(30.0)).mesh().build()),
                )
            };
            let body = world.spawn((Mesh3d(body), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Torso))).id();
            let (near, far, low) = (Transform::from_xyz(0.0, 0.0, 0.5), Transform::from_xyz(0.0, 0.0, 1.2), Transform::from_xyz(0.0, -0.6, 0.0));
            let back = back.then(|| world.spawn((Mesh3d(panel.clone()), far, GlobalTransform::from(far), Gore::wall())).id());
            let floor = world.spawn((Mesh3d(ground), low, GlobalTransform::from(low), Gore::floor())).id();
            match between {
                1 => {
                    world.spawn((Mesh3d(panel), near, GlobalTransform::from(near), Gore::wall()));
                }
                2 => {
                    let close = Transform::from_xyz(0.0, 0.0, 0.2);
                    world.spawn((Mesh3d(panel), close, GlobalTransform::from(close), Gore::cloth()));
                }
                _ => {}
            }
            app.world_mut().run_schedule(FixedUpdate);
            app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 0.05, 2.0), Vec3::NEG_Z, 0.04, 5.0));
            app.world_mut().run_schedule(FixedUpdate);
            let area = |e: Entity| app.world().get::<WetCanvas>(e).map(|c| c.wetted_area()).unwrap_or(0.0);
            (back.map(area).unwrap_or(0.0), area(floor))
        };
        let (open_back, walled_floor) = run(0, true);
        let (walled_back, _) = run(1, true);
        let (curtained_back, _) = run(2, true);
        let (_, bare_floor) = run(0, false);
        assert!(open_back > 0.0, "with nothing between, the wall took no spray");
        assert_eq!(walled_back, 0.0, "a wall behind a wall was spattered through it");
        assert!(curtained_back > 0.0, "no droplet went through the hole the shot made in the curtain");
        assert!(curtained_back < open_back, "the curtain caught nothing: {curtained_back} against {open_back}");
        assert!(bare_floor > 0.0, "with no wall the spray never came down on the floor");
        assert!(walled_floor < bare_floor, "droplets a wall caught landed on the floor as well: {walled_floor} against {bare_floor}");
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
        let (Some(&from), Some(&at), Some(&to)) = (cut.path.first(), cut.path.get(cut.path.len() / 2), cut.path.last()) else {
            panic!("the cut has no path")
        };
        assert!(at.distance(Vec3::new(1.7, 0.0, 0.2)) < 0.03, "the cut's middle is at {at:?} in the bind pose");
        let along = (to - from).normalize_or_zero();
        assert!(along.dot(Vec3::X) > 0.99, "the cut runs {along:?}; up the bent forearm is along +X in the bind pose");
        assert!(((to - from).length() - 0.2).abs() < 0.02, "the cut is {} m long, the slash's 0.2", (to - from).length());
    }

    /// **A slash across a limb lies on its skin.** A blade drawn round a curved surface follows it, and
    /// the wound bed is built along the path. Laid as a straight line through the hit in the plane of
    /// the skin there, a 0.3 m slash across a sphere of radius 0.3 had its ends 35 mm off the sphere,
    /// and its bed with them. Walked on the skin, every point of the path is on the sphere's facets —
    /// a 24 × 12 UV sphere's sit up to about 5 mm inside it — and the path is as long as the blade.
    #[test]
    fn a_slash_across_a_limb_lies_on_its_skin() {
        let mut app = headless();
        let sphere = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(0.3).mesh().uv(24, 12));
        let body = app.world_mut().spawn((Mesh3d(sphere), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb))).id();
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().write_message(GoreHit::slash(body, Vec3::new(0.0, 0.05, 2.0), Vec3::NEG_Z, Vec3::X, 0.15, 8.0));
        app.world_mut().run_schedule(FixedUpdate);
        let Some(cut) = app.world().get::<Laceration>(body) else { panic!("a slash laid no laceration") };
        for p in &cut.path {
            let r = p.length();
            assert!((0.292..=0.3001).contains(&r), "a point of the cut is {} mm off the sphere: {p:?}", (r - 0.3) * 1000.0);
        }
        let length: f32 = cut.path.windows(2).map(|w| w[1].distance(w[0])).sum();
        assert!((length - 0.3).abs() < 0.015, "the cut is {length} m long on the skin, the blade's 0.3");
        let ends = cut.path.first().zip(cut.path.last()).map(|(a, b)| (a.x, b.x));
        // 0.15 m of arc either way on a 0.3 m sphere is half a radian: `0.3 · sin 0.5` along x.
        assert!(ends.is_some_and(|(a, b)| a < -0.13 && b > 0.13), "the cut does not run round the sphere along x: ends at {ends:?}");
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

    /// **A body that can break**: a root carrying the baked leaves of a cube and two dressed cube parts
    /// under it, a metre apart, already dressed by the first tick. Returns the root, the parts and the
    /// material the parts wore.
    fn breakable_body(app: &mut App) -> (Entity, [Entity; 2], Handle<StandardMaterial>) {
        let cube = Cuboid::new(0.4, 0.4, 0.4).mesh().build();
        let proxy = [crate::ProxyCell::from_box(Vec3::ZERO, Vec3::splat(0.2))];
        let fracture = crate::fracture_mesh(&[(&cube, Mat4::IDENTITY)], &proxy, &crate::CutSettings::new(4, 0.05, 3));
        let breakable = GoreBreakable::from_fracture(fracture, Region::Limb, &crate::cross_section::Scale::default());
        let skin = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(cube);
        let world = app.world_mut();
        let root = world.spawn((Transform::from_xyz(0.0, 1.0, 0.0), GlobalTransform::from_xyz(0.0, 1.0, 0.0), breakable)).id();
        let parts = [0.0, 1.0].map(|x| {
            world
                .spawn((Mesh3d(mesh.clone()), MeshMaterial3d(skin.clone()), Transform::IDENTITY, GlobalTransform::from_xyz(x, 1.0, 0.0), Gore::skin(Region::Limb), ChildOf(root)))
                .id()
        });
        app.world_mut().run_schedule(FixedUpdate);
        (root, parts, skin)
    }

    /// **A game with its own physics gets every piece, and the body breaks once.** With
    /// `spawn_fragments` off, the bone handoff writes one [`GoreFractured`] whose pieces each have a
    /// mesh, a place and a size, and the preset spawns none of them and leaves the body to the game.
    /// A second part of the same body hit to bone does not break it again.
    #[test]
    fn a_game_with_physics_gets_the_pieces() {
        let mut app = headless();
        app.world_mut().resource_mut::<GoreDials>().spawn_fragments = false;
        let (root, parts, skin) = breakable_body(&mut app);
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

    /// **A broken body's pieces pour onto the floor beneath them.** A body over a floor is hit to bone
    /// and breaks; from then on its pieces' cut faces are the only blood that can arrive on the floor,
    /// since the part that was hit is gone with its wound. Over the next five seconds the floor's blood
    /// grows by more than it held when the body broke; with no pours it can only shrink, as the floor
    /// soaks it up.
    #[test]
    fn a_broken_body_pours_onto_the_floor_beneath_its_pieces() {
        let mut app = headless();
        let floor = {
            let world = app.world_mut();
            let mesh = world.resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Y, Vec2::splat(2.0)).mesh().build());
            world.spawn((Mesh3d(mesh), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::floor())).id()
        };
        let (_, parts, _) = breakable_body(&mut app);
        app.world_mut().write_message(GoreHit::impact(parts[0], Vec3::new(0.0, 1.0, 2.0), Vec3::NEG_Z, 0.03, 60.0));
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().run_schedule(FixedUpdate);
        let pours = app.world_mut().query::<&CutFaceBleed>().iter(app.world()).count();
        assert!(pours > 0, "the body broke into pieces with no cut face bleeding");
        let mass = |app: &App| {
            let Some(wet) = app.world().get::<WetCanvas>(floor) else { panic!("the floor was not dressed") };
            (0..wet.size()).flat_map(|y| (0..wet.size()).map(move |x| (x, y))).map(|(x, y)| u64::from(wet.amount_at(x, y))).sum::<u64>()
        };
        let at_break = mass(&app);
        for _ in 0..300 {
            app.world_mut().run_schedule(FixedUpdate);
        }
        let later = mass(&app);
        assert!(later > 2 * at_break.max(1), "the floor held {at_break} when the body broke and {later} five seconds on");
    }

    /// **A burn shows the zones its degree reached, deepest at the centre.** A third-degree burn is
    /// an eschar at its centre, near black and wholly shown; a pale ring round it; a red ring past the
    /// contact; and nothing at [`BURN_FLARE`]. A second-degree burn is pale at its centre in a red
    /// ring, a first-degree one red all over, and a burn of no degree shows nothing.
    #[test]
    fn a_burn_shows_the_zones_its_degree_reached() {
        use crate::bloodstain::burn::Degree;
        let pale = |c: [f32; 3]| c.iter().all(|&v| v > 0.8);
        let red = |c: [f32; 3]| c[0] > c[1] + 0.3 && c[0] > c[2] + 0.3;
        let (c, a) = burn_zone(Degree::Third, 0.0);
        assert!(c.iter().all(|&v| v < 0.2) && a > 0.99, "a third-degree centre is {c:?} at {a}, not an eschar");
        let (c, a) = burn_zone(Degree::Third, 0.7);
        assert!(pale(c) && a > 0.3, "a third-degree burn's stasis ring is {c:?} at {a}");
        let (c, a) = burn_zone(Degree::Third, 1.1);
        assert!(red(c) && a > 0.3, "a third-degree burn's hyperaemia is {c:?} at {a}");
        assert_eq!(burn_zone(Degree::Third, BURN_FLARE).1, 0.0, "the reddening reaches past the flare");
        let (c, _) = burn_zone(Degree::Second, 0.0);
        assert!(pale(c), "a second-degree centre is {c:?}, not a blister");
        assert!(red(burn_zone(Degree::Second, 1.1).0), "a second-degree burn has no red ring");
        let (c, a) = burn_zone(Degree::First, 0.0);
        assert!(red(c) && a > 0.5, "a first-degree burn is {c:?} at {a}, not reddened");
        assert_eq!(burn_zone(Degree::None, 0.0).1, 0.0, "a burn of no degree shows");
    }

    /// **A projected wall's canvas runs down it.** A 3 m by 2.4 m wall facing +z is laid over square-on:
    /// `u` along it from its left edge, `v` from its top edge down, both over its longer side, so blood
    /// that runs down the canvas's `+v` (`Gore::wall`'s default) runs down the wall.
    #[test]
    fn a_projected_wall_runs_its_canvas_down_the_wall() {
        let wall = Plane3d::new(Vec3::Z, Vec2::new(1.5, 1.2)).mesh().build();
        let Ok((laid, [u, v])) = projected(&wall) else { panic!("a plain wall was refused") };
        assert!(u.distance(Vec3::X * 3.0) < 1.0e-5 && v.distance(Vec3::NEG_Y * 3.0) < 1.0e-5, "laid along {u} and {v}");
        let (Some(VertexAttributeValues::Float32x3(at)), Some(VertexAttributeValues::Float32x2(uv))) =
            (laid.attribute(Mesh::ATTRIBUTE_POSITION), laid.attribute(Mesh::ATTRIBUTE_UV_0))
        else {
            panic!("the laid wall lost its positions or its UVs")
        };
        for (p, t) in at.iter().zip(uv) {
            assert!((t[0] - (p[0] + 1.5) / 3.0).abs() < 1.0e-6 && (t[1] - (1.2 - p[1]) / 3.0).abs() < 1.0e-6, "{p:?} laid at {t:?}");
        }
    }

    /// **A floor whose UVs tile takes blood where it falls, once it is projected.** A 3 m floor whose
    /// `UV_0` repeats its texture four times, dressed with `Gore::projected`, and a bloody sole put
    /// down at x = −1 and x = +1 on z = 0: each print lands in its own patch of the canvas, a sixth of
    /// the way in from either side and halfway down, and nowhere else. Dressed by its own `UV_0`, the canvas would have clamped
    /// both. The entity draws a copy of its mesh whose `UV_1` is the tiling it had, its own textures
    /// read `UV_1`, and the density is the projection's: one canvas over three metres.
    #[test]
    fn a_projected_floor_whose_uvs_tile_takes_blood_where_it_falls() {
        let mut app = headless();
        let mut plane = Plane3d::new(Vec3::Y, Vec2::splat(1.5)).mesh().build();
        if let Some(VertexAttributeValues::Float32x2(uvs)) = plane.attribute_mut(Mesh::ATTRIBUTE_UV_0) {
            uvs.iter_mut().for_each(|uv| *uv = [uv[0] * 4.0, uv[1] * 4.0]);
        }
        let tiling = plane.attribute(Mesh::ATTRIBUTE_UV_0).cloned();
        let own = app.world_mut().resource_mut::<Assets<Mesh>>().add(plane);
        let material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial::default());
        let floor = app
            .world_mut()
            .spawn((Mesh3d(own.clone()), MeshMaterial3d(material), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::floor().projected()))
            .id();
        let foot = app.world_mut().spawn(Tread { load: 1.0 }).id();
        app.world_mut().run_schedule(FixedUpdate);
        let laid = app.world().get::<Mesh3d>(floor).map(|m| m.0.clone());
        assert!(laid.as_ref().is_some_and(|h| *h != own), "a projected floor drew its own mesh");
        let second = laid.and_then(|h| app.world().resource::<Assets<Mesh>>().get(&h).and_then(|m| m.attribute(Mesh::ATTRIBUTE_UV_1).cloned()));
        assert!(second.is_some() && second.map(|a| a.get_bytes().to_vec()) == tiling.map(|a| a.get_bytes().to_vec()), "the copy's UV_1 is not the tiling the floor had");
        let channel = app
            .world()
            .get::<MeshMaterial3d<FleshMaterial>>(floor)
            .and_then(|m| app.world().resource::<Assets<FleshMaterial>>().get(&m.0))
            .map(|m| m.base.base_color_channel.clone());
        assert_eq!(channel, Some(UvChannel::Uv1), "the floor's own texture no longer reads the tiling");
        let density = app.world().get::<Gore>(floor).and_then(|g| g.uv_per_metre).unwrap_or(0.0);
        assert!((density - 1.0 / 3.0).abs() < 1.0e-4, "the projected density is {density}, not a third of a UV per metre");
        for x in [-1.0, 1.0] {
            app.world_mut().write_message(GoreStep::new(foot, Vec3::new(x, 0.0, 0.0), Vec3::Z));
            app.world_mut().run_schedule(FixedUpdate);
        }
        let Some(wet) = app.world().get::<WetCanvas>(floor) else { panic!("the floor has no wetmap") };
        let n = wet.size();
        let (mut left, mut right, mut elsewhere) = (0u32, 0u32, 0u32);
        for ty in 0..n {
            for tx in 0..n {
                let (u, v, a) = ((tx as f32 + 0.5) / n as f32, (ty as f32 + 0.5) / n as f32, u32::from(wet.amount_at(tx, ty)));
                let mid = (0.4..0.6).contains(&v);
                match u {
                    u if mid && (0.1..0.24).contains(&u) => left += a,
                    u if mid && (0.76..0.9).contains(&u) => right += a,
                    _ => elsewhere += a,
                }
            }
        }
        assert!(left > 0 && right > 0 && elsewhere == 0, "the prints landed at ({left}, {right}) and {elsewhere} elsewhere");
    }

    /// **A sole walks the blood it stood in off in fading prints.** A 3 m floor holds a wet pool at
    /// x = −1; a boot steps into it, then five times along +x on clean floor, one step a tick apart.
    /// Every print holds blood and each holds less than the one before; the step into the pool took
    /// blood out of it. Out of a pool left to dry first, or at a tier that draws no blood, the same
    /// walk prints nothing.
    #[test]
    fn a_sole_walks_its_blood_off_in_fading_prints() {
        // The print masses at x = −0.5 to 1.3, 0.45 m apart, and the floor's whole mass after the step in.
        let walk = |into_pool: bool, dried: bool, tier: Option<crate::GoreTier>| -> (Vec<u32>, u32) {
            let mut app = headless();
            if let Some(tier) = tier {
                app.insert_resource(GorePolicy::for_tier(tier));
            }
            if dried {
                app.insert_resource(WetSettings { dry_ticks: 20, ..WetSettings::default() });
            }
            let plane = app.world_mut().resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Y, Vec2::splat(1.5)).mesh().build());
            let floor = app
                .world_mut()
                .spawn((Mesh3d(plane.clone()), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::floor().with_uv_per_metre(1.0 / 3.0)))
                .id();
            let foot = app.world_mut().spawn_empty().id();
            app.world_mut().run_schedule(FixedUpdate);
            let pool = StainShape { major: 0.5 / 3.0, minor: 0.5 / 3.0, spines: 0, satellites: 0, direction: [1.0, 0.0], seed: 7 };
            let world = app.world_mut();
            let mesh = world.resource::<Assets<Mesh>>().get(&plane).cloned();
            if let (Some(mesh), Some(mut wet)) = (mesh, world.get_mut::<WetCanvas>(floor)) {
                let painted = wet.paint_world(&mesh, &GlobalTransform::IDENTITY, Vec3::new(-1.0, 1.0, 0.0), Vec3::NEG_Y, &pool, 1);
                assert!(painted.is_some(), "the pool missed the floor");
            }
            for _ in 0..(if dried { 40 } else { 1 }) {
                app.world_mut().run_schedule(FixedUpdate);
            }
            let step = |app: &mut App, x: f32| {
                app.world_mut().write_message(GoreStep::new(foot, Vec3::new(x, 0.0, 0.0), Vec3::X));
                app.world_mut().run_schedule(FixedUpdate);
            };
            // Both runs tick alike, so only the step can tell their pools apart.
            if into_pool {
                step(&mut app, -1.0);
            } else {
                app.world_mut().run_schedule(FixedUpdate);
            }
            let Some(wet) = app.world().get::<WetCanvas>(floor) else { panic!("the floor has no wetmap") };
            let n = wet.size();
            let total: u32 = (0..n).flat_map(|ty| (0..n).map(move |tx| (tx, ty))).map(|(tx, ty)| u32::from(wet.amount_at(tx, ty))).sum();
            let mut prints = Vec::new();
            for x in [-0.5, -0.05, 0.4, 0.85, 1.3] {
                step(&mut app, x);
                let Some(wet) = app.world().get::<WetCanvas>(floor) else { panic!("the floor has no wetmap") };
                // World x and z of a texel on the 3 m plane centred on the origin.
                let mut mass = 0;
                for ty in 0..n {
                    for tx in 0..n {
                        let (wx, wz) = (((tx as f32 + 0.5) / n as f32 - 0.5) * 3.0, ((ty as f32 + 0.5) / n as f32 - 0.5) * 3.0);
                        if (x - 0.12..x + 0.12).contains(&wx) && wz.abs() < 0.1 {
                            mass += u32::from(wet.amount_at(tx, ty));
                        }
                    }
                }
                prints.push(mass);
            }
            (prints, total)
        };
        let (prints, stepped_in) = walk(true, false, None);
        assert!(prints.iter().all(|&m| m > 0), "a print held no blood: {prints:?}");
        assert!(prints.windows(2).all(|w| w[1] < w[0]), "the prints did not fade step by step: {prints:?}");
        let (_, stood_off) = walk(false, false, None);
        assert!(stepped_in < stood_off, "the step into the pool took no blood out of it ({stepped_in} against {stood_off})");
        let (dry, _) = walk(true, true, None);
        assert!(dry.iter().all(|&m| m == 0), "a sole that crossed dried blood printed: {dry:?}");
        let (clean, _) = walk(true, false, Some(crate::GoreTier::Stylised));
        assert!(clean.iter().all(|&m| m == 0), "a tier that draws no blood printed steps: {clean:?}");
    }

    /// **Every bruise on a limb ages, not only the newest.** Two blows on one limb at two spots,
    /// thirty ticks apart, then thirty ticks more: both bruises are still ageing, the first older than
    /// the second by the thirty ticks between them, to within one model step. Until 0.5.55 the second
    /// blow replaced the first, which stopped where it was. Past [`MAX_BRUISES`] the oldest retires.
    #[test]
    fn every_bruise_on_a_limb_ages() {
        let mut app = headless();
        let sphere = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(0.3).mesh().uv(24, 12));
        let body = app.world_mut().spawn((Mesh3d(sphere), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb))).id();
        app.world_mut().run_schedule(FixedUpdate);
        let blow = |app: &mut App, x: f32, y: f32| app.world_mut().write_message(GoreHit::blunt(body, Vec3::new(x, y, 2.0), Vec3::NEG_Z, 0.04));
        let ticks = |app: &mut App, n: u32| (0..n).for_each(|_| app.world_mut().run_schedule(FixedUpdate));
        blow(&mut app, -0.1, 0.0);
        ticks(&mut app, 30);
        blow(&mut app, 0.1, 0.05);
        ticks(&mut app, 30);
        let hours: Vec<f32> = app.world().get::<Bruising>(body).map(|b| b.sites.iter().map(|s| s.bruise.hours()).collect()).unwrap_or_default();
        assert_eq!(hours.len(), 2, "the limb holds {} bruises, not the two it took", hours.len());
        let between = 30.0 * app.world().resource::<GoreDials>().bruise_hours_per_tick;
        assert!(hours[1] > 0.0, "the second bruise did not age: {hours:?}");
        assert!(
            (hours[0] - hours[1] - between).abs() <= crate::bloodstain::bruise::STEP_H + 1.0e-4,
            "the first bruise is {} h older than the second, not the {between} h between the blows: {hours:?}",
            hours[0] - hours[1]
        );
        for i in 0..10 {
            blow(&mut app, -0.15 + 0.03 * i as f32, -0.1);
        }
        ticks(&mut app, 1);
        let held = app.world().get::<Bruising>(body).map(|b| b.sites.len()).unwrap_or(0);
        assert_eq!(held, MAX_BRUISES, "twelve blows left {held} bruises ageing");
    }

    /// **Overlapping bruises darken the skin together, redraw after redraw, and a burn survives them.**
    /// A solid bruise laying a ratio of 0.5 and a faint one laying 0.9 at a fifth's strength, drawn
    /// over each other twenty times as twenty model steps would: where both reach, the skin is
    /// multiplied by `0.5 · 0.98`, no lighter and no darker on the twentieth than the first. Painted
    /// one over another, the faint one's margin was blended into the solid one's core every step and
    /// bleached a seam. A burn painted between redraws is still there after the next.
    #[test]
    fn overlapping_bruises_darken_together_and_a_burn_survives_their_redraws() {
        let mut images = Assets::<Image>::default();
        let mut dermis = DermisCanvas::new(&mut images, 64);
        let centres = [Vec2::new(0.4, 0.5), Vec2::new(0.6, 0.5)];
        let field = |i: usize, _r_mm: f32, _dir: Vec2| if i == 0 { ([0.5; 3], 1.0) } else { ([0.9; 3], 0.2) };
        // The ratio a texel multiplies the skin's red by.
        let shown = |d: &DermisCanvas, x: usize, y: usize| {
            let i = (y * 64 + x) * 4;
            let a = d.px[i + 3] as f32 / 255.0;
            1.0 - a + a * d.px[i] as f32 / 255.0
        };
        for _ in 0..20 {
            dermis.draw_bruises(&centres, 0.15, 1.0, field);
        }
        let both = shown(&dermis, 32, 32);
        assert!((both - 0.5 * 0.98).abs() < 0.01, "where both bruises reach the skin is multiplied by {both}, not 0.49");
        assert!((shown(&dermis, 22, 32) - 0.5).abs() < 0.01, "where only the solid one reaches it is {}", shown(&dermis, 22, 32));
        dermis.paint_radial(Vec2::new(0.2, 0.2), 0.05, 1.0, |_, _| ([0.3; 3], 1.0));
        dermis.draw_bruises(&centres, 0.15, 1.0, field);
        assert!((shown(&dermis, 12, 12) - 0.3).abs() < 0.01, "the burn was redrawn away: {}", shown(&dermis, 12, 12));
    }

    /// **A bruise is a blotch, not a disc, and its colour is still the model's.** A day-old 40 mm
    /// bruise, read round its centre: at one radius inside its front some directions are a third
    /// denser than others, and how far it reaches differs by a fifth between directions; past where
    /// the wobble can carry its front nothing shows; and everywhere in its uniform core its density is
    /// the model's within the mottle, some texels denser and some fainter.
    #[test]
    fn a_bruise_is_a_blotch_whose_colour_is_the_models() {
        use crate::bloodstain::bruise::{Bruise, Chromophore, Params, STEP_H};
        let mut bruise = Bruise::new(Params { pool_diameter_mm: 40.0, ..Params::default() });
        bruise.advance((24.0 / STEP_H) as u32);
        let shells = bruise_shells(&bruise);
        let margin = Margin::at(Vec2::new(0.31, 0.62), BRUISE_SALT);
        // Optical density, summed over the channels: what the mottle scales.
        let dark = |r_mm: f32, dir: Vec2| {
            let (c, a) = bruise_texel(&shells, margin, r_mm, dir);
            a * c.iter().map(|&v| -v.max(1.0e-6).ln()).sum::<f32>()
        };
        let dirs: Vec<Vec2> = (0..48).map(|i| Vec2::from_angle(i as f32 * core::f32::consts::TAU / 48.0)).collect();

        let ring: Vec<f32> = dirs.iter().map(|&d| dark(8.0, d)).collect();
        let (lo, hi) = ring.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(lo > 0.0 && hi > lo * 1.33, "at 8 mm the bruise runs {lo} to {hi}: a flat disc, not a blotch");

        let reach: Vec<f32> = dirs.iter().map(|&d| (0..200).map(|k| k as f32 * 0.5).filter(|&r| dark(r, d) > 0.02).fold(0.0, f32::max)).collect();
        let (near, far) = reach.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(far > near * 1.2, "the bruise reaches {near} to {far} mm: a round outline");

        let front = bruise.front_mm(Chromophore::Bilirubin, 0.05).max(bruise.front_mm(Chromophore::Hemoglobin, 0.05));
        let beyond = front * (1.0 + BRUISE_WOBBLE) + 6.0;
        assert!(dirs.iter().all(|&d| dark(beyond, d) < 0.02), "the bruise shows at {beyond} mm, past its front at {front}");

        let model = {
            let (here, blank) = (shells[(8.0 / crate::bloodstain::bruise::DR_MM) as usize], shells[shells.len() - 1]);
            (0..3).map(|c| -(here[c] / blank[c]).ln()).sum::<f32>()
        };
        let core: Vec<f32> = (1..=24).flat_map(|k| dirs.iter().map(move |&d| (k as f32 * 0.5, d))).map(|(r, d)| dark(r, d) / model).collect();
        let (lo, hi) = core.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(
            lo >= 1.0 - BRUISE_MOTTLE - 1.0e-3 && hi <= 1.0 + BRUISE_MOTTLE + 1.0e-3 && lo < 0.95 && hi > 1.05,
            "inside 12 mm the bruise runs {lo} to {hi} of the model's density {model}"
        );
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
            .add_systems(FixedUpdate, (advance_clock, dress, take_hits, throw_bodies, bleed, pour_cut_faces, age_bruises, soak_cloth, tick_canvases, flush_dermis, fly).chain());
        let images = app.world_mut().resource_mut::<Assets<Image>>().reserve_handle();
        app.world_mut().resource_mut::<FleshTables>().luts = images;
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
        let hours = app.world().get::<Bruising>(bruised).and_then(|b| b.sites.first()).map(|b| b.bruise.hours()).unwrap_or(0.0);
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
            .add_systems(FixedUpdate, (advance_clock, dress, take_hits, throw_bodies, bleed, pour_cut_faces, age_bruises, soak_cloth, tick_canvases, flush_dermis, fly).chain());
        let images = app.world_mut().resource_mut::<Assets<Image>>().reserve_handle();
        app.world_mut().resource_mut::<FleshTables>().luts = images;
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

    /// **A culled droplet would have missed.** The spray's broad phase tests each droplet's ray against
    /// the surface's box before the cast, so a box a hair smaller than its mesh, or a slab test with an
    /// axis wrong, loses blood without a sound. Four thousand deterministic rays at a sphere under a
    /// scaled, turned and moved placement: every ray the mesh catches is one the box lets through, and
    /// both outcomes are present in numbers, so the test cannot pass by culling nothing.
    #[test]
    fn the_broad_phase_never_culls_a_ray_the_mesh_would_catch() {
        let mesh = Sphere::new(0.4).mesh().uv(16, 8);
        let placement = Transform::from_xyz(0.7, -0.2, 1.1)
            .with_rotation(Quat::from_euler(EulerRot::XYZ, 0.6, -1.1, 0.3))
            .with_scale(Vec3::new(1.5, 0.5, 2.0));
        let xf = GlobalTransform::from(placement);
        let Some(bounds) = Bounds::of(&mesh, &xf) else { panic!("a sphere has a box") };
        let (mut caught, mut turned_away) = (0, 0);
        for i in 0..4000u32 {
            let r = |k: u32| crate::hash_f32(i.wrapping_mul(7919).wrapping_add(k.wrapping_mul(104_729)));
            let from = Vec3::new(r(1), r(2), r(3)) * 8.0 - Vec3::splat(4.0);
            // Half the rays are aimed at somewhere near the mesh and half point anywhere, so some hit
            // and some are turned away by the box.
            let aimed = placement.translation + (Vec3::new(r(7), r(8), r(9)) * 2.0 - Vec3::ONE) * 0.7;
            let dir = if i % 2 == 0 { aimed - from } else { Vec3::new(r(4), r(5), r(6)) * 2.0 - Vec3::ONE };
            let reached = bounds.reached_by(from, dir);
            let hit = ray_hit(&mesh, &xf, from, dir).is_some();
            assert!(!hit || reached, "ray {i} from {from:?} along {dir:?} hits the mesh and the broad phase culled it");
            caught += usize::from(hit);
            turned_away += usize::from(!reached);
        }
        assert!(caught > 100 && turned_away > 1000, "{caught} rays hit and {turned_away} were culled: the test saw too little of one");
    }

    /// **Casting nearest box first lands every droplet where casting at everything would.** Eight
    /// overlapping surfaces — spheres and boxes, scaled, turned and moved, one with no box, two that
    /// let a droplet through half their atlas as a cloth's hole would — and four thousand
    /// deterministic rays: `nearest_landing` and a cast at every surface in list order agree on the
    /// surface and the landing for every ray. Some rays land behind a nearer surface's box, so the
    /// early stop is exercised, and some pass through the hole to what lies behind. Every other
    /// surface is cast at through its index, and the reference walks every triangle.
    #[test]
    fn nearest_box_first_lands_where_casting_at_everything_would() {
        let shapes = [
            Sphere::new(0.4).mesh().uv(16, 8),
            Cuboid::new(0.6, 0.3, 0.5).mesh().build(),
            Sphere::new(0.25).mesh().uv(12, 6),
            Cuboid::new(1.2, 0.1, 0.8).mesh().build(),
        ];
        let xfs: Vec<GlobalTransform> = (0..8u32)
            .map(|k| {
                let r = |s: u32| crate::hash_f32(k.wrapping_mul(31).wrapping_add(s.wrapping_mul(977)));
                GlobalTransform::from(
                    Transform::from_xyz(r(1) * 1.6 - 0.8, r(2) * 1.6 - 0.8, r(3) * 1.6 - 0.8)
                        .with_rotation(Quat::from_euler(EulerRot::XYZ, r(4) * 3.0, r(5) * 3.0, r(6) * 3.0))
                        .with_scale(Vec3::new(0.6 + r(7), 0.6 + r(8), 0.6 + r(9))),
                )
            })
            .collect();
        let indexed: Vec<crate::bvh::Bvh> = shapes.iter().filter_map(|m| castable(m).map(|(p, t)| crate::bvh::Bvh::build(p, t))).collect();
        let drawn: Vec<Option<Castable<usize>>> =
            xfs.iter().enumerate().map(|(k, xf)| Some((k, if k == 5 { None } else { Bounds::of(&shapes[k % shapes.len()], xf) }))).collect();
        let (shape_of, xf_of, index_of) = (&shapes, &xfs, &indexed);
        let cast = |from: Vec3, dir: Vec3| {
            move |k: usize| ray_cast(&shape_of[k % shape_of.len()], index_of.get(k % shape_of.len()).filter(|_| k % 2 == 0).map(|b| b.fitted()), &xf_of[k], from, dir)
        };
        let hole = |k: &usize, l: &Landing| *k % 4 == 3 && l.uv.x < 0.5;
        let mut order = Vec::new();
        let (mut behind, mut through) = (0, 0);
        for i in 0..4000u32 {
            let r = |k: u32| crate::hash_f32(i.wrapping_mul(7919).wrapping_add(k.wrapping_mul(104_729)));
            let from = Vec3::new(r(1), r(2), r(3)) * 6.0 - Vec3::splat(3.0);
            let aimed = Vec3::new(r(7), r(8), r(9)) * 1.6 - Vec3::splat(0.8);
            let dir = if i % 3 == 0 { Vec3::new(r(4), r(5), r(6)) * 2.0 - Vec3::ONE } else { aimed - from };

            let mut every: Option<(usize, Landing)> = None;
            let mut hits = 0;
            for (k, entry) in drawn.iter().enumerate() {
                let Some((tag, _)) = entry else { continue };
                let Some(l) = ray_hit(&shapes[k % shapes.len()], &xfs[k], from, dir) else { continue };
                hits += 1;
                if hole(tag, &l) {
                    through += 1;
                    continue;
                }
                if every.as_ref().is_none_or(|(_, near)| l.t < near.t) {
                    every = Some((k, l));
                }
            }
            behind += usize::from(hits > 1);
            let first = nearest_landing(&drawn, from, dir, &mut order, cast(from, dir), hole);
            assert_eq!(
                first.map(|(k, l)| (k, l.t.to_bits(), l.uv.to_array().map(f32::to_bits))),
                every.map(|(k, l)| (k, l.t.to_bits(), l.uv.to_array().map(f32::to_bits))),
                "ray {i} from {from:?} along {dir:?}"
            );
        }
        assert!(behind > 500 && through > 50, "{behind} rays crossed two surfaces and {through} went through the hole: too few to test");
    }

    /// **A skinned mesh's joints box its pose without posing it.** Three hundred vertices on four
    /// joints, their weights adding to one, to three per cent more or less, or to nothing, under forty
    /// random poses — turned, moved and stretched unevenly: every vertex `posed_mesh` places lies in
    /// the box its joints carry. A negative weight gives no box, and the pose itself is boxed instead.
    #[test]
    fn a_skinned_mesh_is_boxed_by_its_joints_without_being_posed() {
        let r = |k: u32| crate::hash_f32(k.wrapping_mul(0x2545_F491) ^ 0x9E37_79B9);
        let n = 300u32;
        let rig = |negative: bool| {
            let positions: Vec<[f32; 3]> = (0..n).map(|k| [r(3 * k) * 2.0 - 1.0, r(3 * k + 1) * 2.0 - 1.0, r(3 * k + 2) * 2.0 - 1.0]).collect();
            // Most vertices on one joint each, so a joint's box is tight round its own and a vertex
            // scaled off it by its weights' slack leaves it; every fifth blended over all four.
            let joints: Vec<[u16; 4]> = (0..n).map(|k| if k % 5 == 4 { [0, 1, 2, 3] } else { [(k % 4) as u16, 0, 0, 0] }).collect();
            let weights: Vec<[f32; 4]> = (0..n)
                .map(|k| {
                    let w: [f32; 4] = if k % 5 == 4 { core::array::from_fn(|s| r(5000 + 4 * k + s as u32)) } else { [1.0, 0.0, 0.0, 0.0] };
                    let sum: f32 = w.iter().sum();
                    match k % 10 {
                        0 => [0.0; 4],
                        1 => w.map(|x| x / sum * 1.03),
                        2 => w.map(|x| x / sum * 0.97),
                        3 if negative => [0.6, 0.6, -0.2, 0.0],
                        _ => w.map(|x| x / sum),
                    }
                })
                .collect();
            let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5f32, 0.5]; n as usize]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
            mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
            mesh.insert_indices(Indices::U32((0..n).collect()));
            mesh
        };
        let mesh = rig(false);
        let Some(boxes) = JointBoxes::of(&mesh) else { panic!("an ordinary rig was refused a box") };
        let mut checked = 0;
        for p in 0..40u32 {
            let q = |k: u32| r(90_000 + 64 * p + k);
            let pose = Pose::from_matrices(
                (0..4u32)
                    .map(|j| {
                        let scale = Vec3::new(0.5 + 1.5 * q(10 * j), 0.5 + 1.5 * q(10 * j + 1), 0.5 + 1.5 * q(10 * j + 2));
                        let turn = Quat::from_euler(EulerRot::XYZ, q(10 * j + 3) * 6.0, q(10 * j + 4) * 6.0, q(10 * j + 5) * 6.0);
                        // Away from the origin, on every axis, so a vertex with no weight — posed to the
                        // origin — is outside every joint's box.
                        let at = Vec3::new(q(10 * j + 6), q(10 * j + 7), q(10 * j + 8)) * 20.0 + Vec3::splat(4.0);
                        Mat4::from_scale_rotation_translation(scale, turn, at)
                    })
                    .collect(),
            );
            let Some(b) = boxes.bounds(&pose) else { panic!("pose {p} gave no box") };
            let Some(posed) = skin::posed_mesh(&mesh, &pose) else { panic!("pose {p} could not be posed") };
            let Some(VertexAttributeValues::Float32x3(at)) = posed.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("posed without positions") };
            for (v, w) in at.iter().enumerate() {
                let w = Vec3::from(*w);
                assert!(w.cmpge(b.min).all() && w.cmple(b.max).all(), "pose {p}: vertex {v} at {w} is outside {:?}..{:?}", b.min, b.max);
                checked += 1;
            }
        }
        assert_eq!(checked, 40 * n);
        assert!(JointBoxes::of(&rig(true)).is_none(), "a rig with a negative weight was boxed by its joints");
    }

    /// **An indexed cast lands where walking every triangle does, to the bit.** Spheres, an axis-aligned
    /// box, a torus, and a soup of random triangles with duplicates — two triangles on one place, so a
    /// crossing ties and the index buffer's order must decide — indexed and not, placed and turned,
    /// and thousands of rays at each: random, aimed at a vertex where triangles meet, along an axis,
    /// and from inside. `ray_cast` through the index and `ray_hit` walking every triangle return the
    /// same landing, every field bit for bit, or both none. Then every vertex is moved — twisted and
    /// jittered, as a pose moves a skin's — and the tree is refitted to it rather than built again: the
    /// same again, at the moved mesh.
    #[test]
    fn an_indexed_cast_lands_where_walking_every_triangle_does() {
        let r = |k: u32| crate::hash_f32(k.wrapping_mul(0x9E37_79B9).wrapping_add(0x85EB_CA6B));
        let soup = |indexed: bool| {
            let mut positions: Vec<[f32; 3]> = (0..300u32).map(|k| [r(3 * k) - 0.5, r(3 * k + 1) - 0.5, r(3 * k + 2) - 0.5]).collect();
            let mut uvs: Vec<[f32; 2]> = (0..300u32).map(|k| [r(1000 + k), r(2000 + k)]).collect();
            let mut indices: Vec<u32> = (0..300u32).map(|k| (r(5000 + k) * 299.0) as u32).collect();
            // Every tenth triangle again, later in the buffer: the same crossing at the same distance.
            let again: Vec<u32> = indices.chunks(3).step_by(10).flatten().copied().collect();
            indices.extend(again);
            let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
            if !indexed {
                positions = indices.iter().map(|&i| positions[i as usize]).collect();
                uvs = indices.iter().map(|&i| uvs[i as usize]).collect();
            }
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            if indexed {
                mesh.insert_indices(Indices::U32(indices));
            }
            mesh
        };
        let meshes = [Sphere::new(0.4).mesh().uv(32, 16), Cuboid::new(0.6, 0.3, 0.5).mesh().build(), Torus::new(0.2, 0.4).mesh().build(), soup(true), soup(false)];
        let (mut landed, mut tied) = (0, 0);
        for (m, mesh) in meshes.iter().enumerate() {
            let Some((positions, tris)) = castable(mesh) else { panic!("mesh {m} cannot be cast at") };
            let index = crate::bvh::Bvh::build(positions, tris);
            let corners: Vec<Vec3> = positions.iter().map(|p| Vec3::from(*p)).collect();
            for (x, xf) in [GlobalTransform::IDENTITY, GlobalTransform::from(Transform::from_xyz(0.3, -0.2, 0.1).with_rotation(Quat::from_rotation_y(0.7)).with_scale(Vec3::splat(1.7)))].iter().enumerate() {
                for i in 0..2500u32 {
                    let q = |k: u32| crate::hash_f32(i.wrapping_mul(7919).wrapping_add(k.wrapping_mul(104_729)).wrapping_add((m as u32) << 20).wrapping_add((x as u32) << 28));
                    let from = Vec3::new(q(1), q(2), q(3)) * 3.0 - Vec3::splat(1.5);
                    let at = |p: Vec3| xf.transform_point(p);
                    let dir = match i % 5 {
                        // At a vertex, where several triangles meet.
                        0 | 1 => at(corners[(q(4) * (corners.len() - 1) as f32) as usize]) - from,
                        // Along an axis.
                        2 => [Vec3::X, Vec3::NEG_Y, Vec3::Z][(i / 5 % 3) as usize],
                        // From inside, anywhere.
                        3 => Vec3::new(q(5), q(6), q(7)) * 2.0 - Vec3::ONE,
                        _ => at(Vec3::new(q(8), q(9), q(10)) * 0.8 - Vec3::splat(0.4)) - from,
                    };
                    let from = if i % 5 == 3 { at(Vec3::new(q(11), q(12), q(13)) * 0.4 - Vec3::splat(0.2)) } else { from };
                    let bits = |l: Landing| (l.t.to_bits(), l.uv.to_array().map(f32::to_bits), l.tri, l.bary.to_array().map(f32::to_bits), l.point.to_array().map(f32::to_bits), l.normal.to_array().map(f32::to_bits));
                    let walked = ray_hit(mesh, xf, from, dir).map(bits);
                    let indexed = ray_cast(mesh, Some(index.fitted()), xf, from, dir).map(bits);
                    assert_eq!(indexed, walked, "mesh {m}, transform {x}, ray {i} from {from:?} along {dir:?}");
                    landed += usize::from(walked.is_some());
                    tied += usize::from(walked.is_some() && m >= 3 && i % 5 < 2);
                }
            }
        }
        assert!(landed > 5000 && tied > 50, "{landed} rays landed and {tied} were aimed at the soup's vertices: too few to test");

        let mut refit_landed = 0;
        for (m, mesh) in meshes.iter().enumerate() {
            let Some((positions, tris)) = castable(mesh) else { panic!("mesh {m} cannot be cast at") };
            let tree = crate::bvh::Bvh::build(positions, tris);
            let moved_at: Vec<[f32; 3]> = positions
                .iter()
                .enumerate()
                .map(|(v, p)| {
                    let p = Vec3::from(*p);
                    let twist = Quat::from_rotation_y(p.y * 1.3);
                    (twist * p * 1.2 + (Vec3::new(r(70_000 + 3 * v as u32), r(70_001 + 3 * v as u32), r(70_002 + 3 * v as u32)) - Vec3::splat(0.5)) * 0.05).to_array()
                })
                .collect();
            let mut moved = mesh.clone();
            moved.insert_attribute(Mesh::ATTRIBUTE_POSITION, moved_at.clone());
            let refit = tree.refit(&moved_at);
            let corners: Vec<Vec3> = moved_at.iter().map(|p| Vec3::from(*p)).collect();
            for i in 0..2000u32 {
                let q = |k: u32| crate::hash_f32(i.wrapping_mul(6007).wrapping_add(k.wrapping_mul(92_821)).wrapping_add((m as u32) << 21));
                let from = Vec3::new(q(1), q(2), q(3)) * 3.0 - Vec3::splat(1.5);
                let dir = if i % 2 == 0 { corners[(q(4) * (corners.len() - 1) as f32) as usize] - from } else { Vec3::new(q(5), q(6), q(7)) * 0.8 - Vec3::splat(0.4) - from };
                let bits = |l: Landing| (l.t.to_bits(), l.uv.to_array().map(f32::to_bits), l.tri, l.point.to_array().map(f32::to_bits));
                let walked = ray_hit(&moved, &GlobalTransform::IDENTITY, from, dir).map(bits);
                let refitted = ray_cast(&moved, Some(tree.with(&refit)), &GlobalTransform::IDENTITY, from, dir).map(bits);
                assert_eq!(refitted, walked, "moved mesh {m}, ray {i} from {from:?} along {dir:?}");
                refit_landed += usize::from(walked.is_some());
            }
        }
        assert!(refit_landed > 3000, "{refit_landed} rays landed on the moved meshes: too few to test");
    }

    /// **A measurement, not a test**: milliseconds one `FixedUpdate` costs when it takes one hit, and
    /// when it takes a shotgun's twelve, by how many dressed surfaces the world has. Every part is a
    /// UV sphere, six to a character, so the surfaces overlap more than a body's would and the spray
    /// reaches more of them. Run it with
    /// `cargo test -p bevy_carnage --lib --release -- hit_cost_by_scene_size --ignored --nocapture`.
    #[test]
    #[ignore]
    fn hit_cost_by_scene_size() {
        for (characters, rings) in [(1usize, 24u32), (1, 64), (10, 24), (40, 24)] {
            let mut app = headless();
            let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(0.3).mesh().uv(rings * 2, rings));
            let tris = Sphere::new(0.3).mesh().uv(rings * 2, rings).indices().map(|i| i.len() / 3).unwrap_or(0);
            let mut first = None;
            for c in 0..characters {
                for part in 0..6 {
                    let at = Vec3::new((c % 8) as f32 * 1.5, 0.3 + part as f32 * 0.25, (c / 8) as f32 * 1.5);
                    let e = app
                        .world_mut()
                        .spawn((Mesh3d(mesh.clone()), Transform::from_translation(at), GlobalTransform::from(Transform::from_translation(at)), Gore::skin(Region::Torso)))
                        .id();
                    first.get_or_insert(e);
                }
            }
            let plane = app.world_mut().resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Y, Vec2::splat(30.0)).mesh().build());
            app.world_mut().spawn((Mesh3d(plane), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::floor().with_uv_per_metre(1.0 / 60.0)));
            app.world_mut().run_schedule(FixedUpdate);
            let Some(target) = first else { return };
            for pellets in [1usize, 12] {
                let mut times = Vec::new();
                for shot in 0..8 {
                    for pellet in 0..pellets {
                        let from = Vec3::new(0.02 * (shot * pellets + pellet) as f32 - 0.1, 0.3 + 0.01 * pellet as f32, 3.0);
                        app.world_mut().write_message(GoreHit::impact(target, from, Vec3::NEG_Z, 0.03, 0.5));
                    }
                    let t = std::time::Instant::now();
                    app.world_mut().run_schedule(FixedUpdate);
                    times.push(t.elapsed().as_secs_f64() * 1000.0);
                }
                times.sort_by(|a, b| a.total_cmp(b));
                println!(
                    "{characters:>3} characters x 6 parts of {tris} tris, {pellets:>2} hit(s) in one tick: median {:.2} ms, max {:.2} ms",
                    times[times.len() / 2],
                    times[times.len() - 1]
                );
            }
        }
    }

    /// The `Wounded` messages and the `BoneExposed` messages this world has written so far.
    fn announced(app: &App) -> (Vec<Wounded>, usize) {
        let wounded = app.world().resource::<Messages<Wounded>>();
        let bone = app.world().resource::<Messages<BoneExposed>>();
        (wounded.get_cursor().read(wounded).copied().collect(), bone.get_cursor().read(bone).count())
    }

    /// **A reset body is as it was dressed, and heals for good.** A limb takes a shot to the bone, a
    /// blow, a burn and a slash, and bleeds for half a second; one [`GoreReset`] later its three
    /// canvases are byte for byte a freshly dressed limb's, nothing is bleeding, bruising or open, and
    /// it draws its own shared mesh again where the slash had it drawing a torn copy, and the slash's
    /// bed is gone. A second of ticks after that it is still clean, and a second shot to the bone
    /// announces the bone again, which a handoff left spent would swallow.
    #[test]
    fn a_reset_body_is_as_it_was_dressed() {
        let limb = |app: &mut App| {
            let sphere = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(0.3).mesh().uv(24, 12));
            let body = app.world_mut().spawn((Mesh3d(sphere.clone()), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb))).id();
            app.world_mut().run_schedule(FixedUpdate);
            (body, sphere)
        };
        let state = |app: &App, body: Entity| {
            let w = app.world();
            (
                w.get::<WetCanvas>(body).map(|c| c.digest()),
                w.get::<FlayCanvas>(body).map(|c| c.digest()),
                w.get::<DermisCanvas>(body).map(|c| c.px.clone()),
            )
        };
        let mut fresh = headless();
        let (twin, _) = limb(&mut fresh);
        let clean = state(&fresh, twin);

        let mut app = headless();
        let (body, sphere) = limb(&mut app);
        let to_bone = |app: &mut App| app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 0.05, 2.0), Vec3::NEG_Z, 0.03, 60.0));
        to_bone(&mut app);
        app.world_mut().write_message(GoreHit::blunt(body, Vec3::new(0.0, -0.12, 2.0), Vec3::NEG_Z, 0.04));
        app.world_mut().write_message(GoreHit::burn(body, Vec3::new(0.12, 0.0, 2.0), Vec3::NEG_Z, 0.03, 150.0, 1.0));
        app.world_mut().write_message(GoreHit::slash(body, Vec3::new(-0.1, 0.1, 2.0), Vec3::NEG_Z, Vec3::X, 0.05, 8.0));
        for _ in 0..30 {
            app.world_mut().run_schedule(FixedUpdate);
        }
        let w = app.world();
        assert!(w.get::<Wounds>(body).is_some() && w.get::<Bruising>(body).is_some() && w.get::<Laceration>(body).is_some(), "the hits did not all land");
        assert_ne!(state(&app, body), clean, "the hits changed nothing");
        assert_ne!(w.get::<Mesh3d>(body).map(|m| m.0.id()), Some(sphere.id()), "the slash did not tear a copy");
        // `headless` runs no `LacerationPlugin`, which is what lays a slash's bed: lay one by hand.
        app.world_mut().spawn((LacerationBed, ChildOf(body)));
        let beds = |app: &mut App| app.world_mut().query_filtered::<(), With<LacerationBed>>().iter(app.world()).count();
        let bones = announced(&app).1;
        assert_eq!(bones, 1, "the shot did not reach the bone");

        app.world_mut().write_message(GoreReset::new(body));
        app.world_mut().run_schedule(FixedUpdate);
        let w = app.world();
        assert_eq!(state(&app, body), clean, "a reset limb's canvases are not a fresh one's");
        assert!(w.get::<Wounds>(body).is_none() && w.get::<Bruising>(body).is_none() && w.get::<Laceration>(body).is_none(), "a wound outlived the reset");
        assert_eq!(w.get::<Mesh3d>(body).map(|m| m.0.id()), Some(sphere.id()), "the limb still draws the torn copy");
        assert_eq!(beds(&mut app), 0, "the slash's bed outlived the reset");
        for _ in 0..60 {
            app.world_mut().run_schedule(FixedUpdate);
        }
        assert_eq!(state(&app, body), clean, "something kept bleeding after the reset");

        to_bone(&mut app);
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(announced(&app).1, bones + 1, "the bone was not announced again after the reset");
    }

    /// **A tier that draws no blood still announces the hit, and paints nothing.** Reduction is
    /// substitution, never deletion: the `Wounded` a Stylised hit writes is the one the default tier
    /// writes, point for point, so a game that puts a spark where blood would be gets the same place,
    /// normal and size. The canvases are exactly as they were, and no wound opens to bleed.
    #[test]
    fn a_tier_that_draws_no_blood_still_announces_the_hit() {
        let hit = |tier: Option<crate::GoreTier>| {
            let mut app = headless();
            if let Some(tier) = tier {
                app.insert_resource(GorePolicy::for_tier(tier));
            }
            let sphere = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(0.3).mesh().uv(24, 12));
            let body = app.world_mut().spawn((Mesh3d(sphere), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb))).id();
            app.world_mut().run_schedule(FixedUpdate);
            let (flay, wet) = (app.world().get::<FlayCanvas>(body).map(|c| c.digest()), app.world().get::<WetCanvas>(body).map(|c| c.digest()));
            app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 0.1, 2.0), Vec3::NEG_Z, 0.03, 6.0));
            app.world_mut().run_schedule(FixedUpdate);
            let untouched = app.world().get::<FlayCanvas>(body).map(|c| c.digest()) == flay && app.world().get::<WetCanvas>(body).map(|c| c.digest()) == wet;
            let opened = app.world().get::<Wounds>(body).is_some_and(|w| !w.0.is_empty());
            (announced(&app).0, untouched, opened)
        };
        let (stylised, untouched, opened) = hit(Some(crate::GoreTier::Stylised));
        let (default, default_untouched, default_opened) = hit(None);
        assert_eq!(stylised.len(), 1, "a Stylised hit announced {} wounds", stylised.len());
        assert_eq!(stylised, default, "the announcement is not the one the default tier makes");
        assert!(untouched && !opened, "a tier that draws no blood painted or opened something");
        assert!(!default_untouched && default_opened, "the control painted nothing, so the comparison proves nothing");
    }

    /// **Blood without gore keeps the body in one piece.** `GoreTier::Blood` allows the blood and
    /// refuses the mutilation: the part that took 60 mm still bleeds, the bone is still forwarded as
    /// `BoneExposed`, and nothing breaks, spills or is thrown. The default tier is the control: the
    /// same two hits break it.
    #[test]
    fn blood_without_gore_keeps_the_body_in_one_piece() {
        let run = |tier: Option<crate::GoreTier>| {
            let mut app = headless();
            if let Some(tier) = tier {
                app.insert_resource(GorePolicy::for_tier(tier));
            }
            let (root, parts, _) = breakable_body(&mut app);
            for (part, x) in parts.into_iter().zip([0.0, 1.0]) {
                app.world_mut().write_message(GoreHit::impact(part, Vec3::new(x, 1.0, 2.0), Vec3::NEG_Z, 0.03, 60.0));
                app.world_mut().run_schedule(FixedUpdate);
            }
            let thrown = app.world_mut().query_filtered::<(), Or<(With<Flying>, With<Standing>)>>().iter(app.world()).count();
            let guts = app.world_mut().query_filtered::<(), With<Gut>>().iter(app.world()).count();
            let bled = parts.iter().any(|p| app.world().get::<WetCanvas>(*p).is_some_and(|w| w.wetted_area() > 0.0));
            (app.world().get_entity(root).is_ok(), thrown, guts, bled, announced(&app).1)
        };
        let (standing, thrown, guts, bled, bones) = run(Some(crate::GoreTier::Blood));
        assert!(standing && thrown == 0 && guts == 0, "Blood broke the body: standing {standing}, {thrown} pieces, {guts} guts");
        assert!(bled, "Blood drew no blood");
        assert!(bones > 0, "the bone was not forwarded at a tier that refuses only the breaking");
        let (standing, thrown, _, _, _) = run(None);
        assert!(!standing && thrown > 0, "the control did not break: standing {standing}, {thrown} pieces");
    }

    /// **The world's share of the blood is its own switch.** With `blood_decals` off the body still
    /// takes the wound and bleeds on itself, and nothing reaches the floor — not the spray, and not the
    /// drips that fall from the wound as it keeps pulsing.
    #[test]
    fn world_blood_can_be_refused_while_the_body_bleeds() {
        let run = |decals: bool| {
            let mut app = headless();
            app.insert_resource(GorePolicy { blood_decals: decals, ..GorePolicy::default() });
            let sphere = app.world_mut().resource_mut::<Assets<Mesh>>().add(Sphere::new(0.3).mesh().uv(24, 12));
            // 40 m across, because the spray flies metres before it lands and a small slab would take only the drips.
            let plane = app.world_mut().resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Y, Vec2::splat(20.0)).mesh().build());
            let body = app.world_mut().spawn((Mesh3d(sphere), Transform::from_xyz(0.0, 1.0, 0.0), GlobalTransform::from_xyz(0.0, 1.0, 0.0), Gore::skin(Region::Limb))).id();
            let floor = app.world_mut().spawn((Mesh3d(plane), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::floor().with_uv_per_metre(1.0 / 40.0))).id();
            app.world_mut().run_schedule(FixedUpdate);
            app.world_mut().write_message(GoreHit::impact(body, Vec3::new(0.0, 1.0, 2.0), Vec3::NEG_Z, 0.03, 6.0));
            for _ in 0..120 {
                app.world_mut().run_schedule(FixedUpdate);
            }
            let area = |e: Entity| app.world().get::<WetCanvas>(e).map(|w| w.wetted_area()).unwrap_or(0.0);
            (area(body), area(floor))
        };
        let (body, floor) = run(false);
        assert!(body > 0.0, "the body took no blood with world blood off");
        assert_eq!(floor, 0.0, "world blood was off and the floor took {floor}");
        let (_, control) = run(true);
        assert!(control > 0.0, "the control put nothing on the floor, so the test proves nothing");
    }

    /// How many open wounds an entity has.
    fn open_wounds(app: &App, e: Entity) -> usize {
        app.world().get::<Wounds>(e).map_or(0, |w| w.0.len())
    }

    /// **A hit at a character wounds the skin the ray reaches first.** The game's physics finds the
    /// character, rarely the mesh. Three parts under one root: a skin in front, a skin behind it that
    /// the ray would also reach, and a shirt in front of both. Only the front skin is wounded: the
    /// nearer one of the two skins, and not the shirt, which is not what a wound opens on.
    #[test]
    fn a_hit_at_a_character_wounds_the_skin_the_ray_reaches_first() {
        let mut app = headless();
        let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.4, 0.4).mesh().build());
        let world = app.world_mut();
        let root = world.spawn((Transform::IDENTITY, GlobalTransform::IDENTITY)).id();
        let mut part = |z: f32, gore: Gore| {
            world.spawn((Mesh3d(cube.clone()), Transform::from_xyz(0.0, 0.0, z), GlobalTransform::from_xyz(0.0, 0.0, z), gore, ChildOf(root))).id()
        };
        let (behind, front, shirt) = (part(-1.0, Gore::skin(Region::Limb)), part(0.0, Gore::skin(Region::Limb)), part(0.5, Gore::cloth()));
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().write_message(GoreHit::impact(root, Vec3::new(0.0, 0.0, 3.0), Vec3::NEG_Z, 0.03, 4.0));
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!((open_wounds(&app, front), open_wounds(&app, behind), open_wounds(&app, shirt)), (1, 0, 0), "front, behind, shirt");
        let wound = app.world().get::<Wounds>(front).and_then(|w| w.0.first().copied());
        assert!(wound.is_some_and(|w| (w.wound.at[2] - 0.2).abs() < 0.02), "the wound is not on the front skin's face: {wound:?}");
        assert_eq!(announced(&app).0.len(), 1, "one hit announced more or fewer than one wound");
    }

    /// **A hit at a character finds a limb where it is drawn.** The bent arm of `rigged_arm`, now under
    /// a root, is struck by naming the root: the ray crosses the posed forearm, so it is wounded; the
    /// same ray at a straight arm crosses nothing, so no part is wounded and nothing is announced.
    #[test]
    fn a_hit_at_a_character_finds_the_posed_limb() {
        let strike = |bend: f32| {
            let (mut app, body, ..) = rigged_arm(bend);
            let root = app.world_mut().spawn((Transform::IDENTITY, GlobalTransform::IDENTITY)).id();
            app.world_mut().entity_mut(body).insert(ChildOf(root));
            app.world_mut().write_message(GoreHit::impact(root, Vec3::new(1.0, 0.7, 2.0), Vec3::NEG_Z, 0.04, 5.0));
            app.world_mut().run_schedule(FixedUpdate);
            (open_wounds(&app, body), announced(&app).0.len())
        };
        assert_eq!(strike(core::f32::consts::FRAC_PI_2), (1, 1), "the bent forearm was not found");
        assert_eq!(strike(0.0), (0, 0), "a ray through empty air wounded something");
    }

    /// **Two skins the ray reaches at the same distance go to the lower entity**, so which one is
    /// wounded does not depend on the order a query yields them.
    #[test]
    fn a_tie_between_two_skins_goes_to_the_lower_entity() {
        let mut app = headless();
        let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.4, 0.4).mesh().build());
        let world = app.world_mut();
        let root = world.spawn((Transform::IDENTITY, GlobalTransform::IDENTITY)).id();
        let mut part = || world.spawn((Mesh3d(cube.clone()), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb), ChildOf(root))).id();
        let (first, second) = (part(), part());
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().write_message(GoreHit::impact(root, Vec3::new(0.0, 0.0, 3.0), Vec3::NEG_Z, 0.03, 4.0));
        app.world_mut().run_schedule(FixedUpdate);
        // Which of the two is lower is Bevy's to say; the rule is that it is the one wounded.
        let (lower, higher) = if first < second { (first, second) } else { (second, first) };
        assert_eq!((open_wounds(&app, lower), open_wounds(&app, higher)), (1, 0));
    }

    /// What a mesh entity was dressed as, by kind, or `None` if it has no `Gore`.
    fn dressed_as(app: &App, e: Entity) -> Option<GoreKind> {
        app.world().get::<Gore>(e).map(|g| g.kind)
    }

    /// **A scene is dressed by name, as it streams in.** The root is spawned first and its meshes arrive
    /// on a later tick, the way a glTF scene's do. A mesh is matched on its own name and then on the
    /// nodes above it, nearest first: `Mesh.001` under `Head_Node` is the head. A held weapon is left
    /// alone, a mesh with its own `Gore` is not touched, and a mesh
    /// outside the scene is not dressed at all.
    #[test]
    fn a_scene_is_dressed_by_name_as_it_streams_in() {
        let mut app = headless();
        let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.4, 0.4).mesh().build());
        let scene = GoreScene::skin(Region::Limb)
            .with("head", Gore::skin(Region::Head))
            .with("shirt", Gore::cloth())
            .without("sword");
        let root = app.world_mut().spawn((Transform::IDENTITY, GlobalTransform::IDENTITY, scene)).id();
        app.world_mut().run_schedule(FixedUpdate);
        let world = app.world_mut();
        let node = world.spawn((Name::new("Head_Node"), ChildOf(root))).id();
        let mut mesh = |name: &str, parent: Entity| world.spawn((Name::new(name.to_string()), Mesh3d(cube.clone()), ChildOf(parent))).id();
        let head = mesh("Mesh.001", node);
        let shirt = mesh("Shirt.002", root);
        let sword = mesh("Sword", root);
        let arm = mesh("Arm_L", root);
        let own = world.spawn((Name::new("Head"), Mesh3d(cube.clone()), Gore::skin(Region::Torso), ChildOf(root))).id();
        let outside = world.spawn((Name::new("Head"), Mesh3d(cube.clone()))).id();
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(dressed_as(&app, head), Some(GoreKind::Skin(Region::Head)), "a mesh below a node named Head");
        assert_eq!(dressed_as(&app, shirt), Some(GoreKind::Cloth), "a rule's own name beats the default");
        assert_eq!(dressed_as(&app, sword), None, "a held weapon was dressed");
        assert_eq!(dressed_as(&app, arm), Some(GoreKind::Skin(Region::Limb)), "the default");
        assert_eq!(dressed_as(&app, own), Some(GoreKind::Skin(Region::Torso)), "a mesh with its own Gore was overwritten");
        assert_eq!(dressed_as(&app, outside), None, "a mesh outside every scene was dressed");
        assert!(app.world().get::<Dressed>(head).is_some(), "the dressed mesh got its canvases on the same tick");
    }

    /// **A scene component added after its meshes still dresses them**, and `humanoid` reads the words
    /// a rig is named with, not the letters. A Blender export puts every mesh below a node called
    /// `Armature` and a game labels its root with whatever it likes, and neither is a limb: with
    /// substring matching `arm` and `hand` dressed the body as one. `Elbow` is not a `bow`, and a rule
    /// added after `humanoid`'s own wins over it.
    #[test]
    fn a_scene_added_late_dresses_the_meshes_already_there() {
        let mut app = headless();
        let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.4, 0.4).mesh().build());
        let world = app.world_mut();
        let root = world.spawn((Name::new("Hand_Gun_Hero"), Transform::IDENTITY, GlobalTransform::IDENTITY)).id();
        let armature = world.spawn((Name::new("Armature"), ChildOf(root))).id();
        let mut mesh = |name: &str, parent: Entity| world.spawn((Name::new(name.to_string()), Mesh3d(cube.clone()), ChildOf(parent))).id();
        let (skull, thigh, jacket, gun) = (mesh("Skull", root), mesh("Thigh_R", root), mesh("Jacket", root), mesh("Rifle_Gun", root));
        let (body, elbow, hat, fore) = (mesh("Body", armature), mesh("Elbow_Arm", root), mesh("Hat", root), mesh("ForeArm.L", root));
        let all = [skull, thigh, jacket, gun, body, elbow, hat, fore];
        app.world_mut().run_schedule(FixedUpdate);
        assert!(all.iter().all(|e| dressed_as(&app, *e).is_none()), "dressed with no scene");
        app.world_mut().entity_mut(root).insert(GoreScene::humanoid().with("hat", Gore::skin(Region::Head)));
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(dressed_as(&app, skull), Some(GoreKind::Skin(Region::Head)));
        assert_eq!(dressed_as(&app, thigh), Some(GoreKind::Skin(Region::Limb)));
        assert_eq!(dressed_as(&app, jacket), Some(GoreKind::Cloth));
        assert_eq!(dressed_as(&app, gun), None, "a gun was dressed");
        assert_eq!(dressed_as(&app, body), Some(GoreKind::Skin(Region::Torso)), "a body under Armature, in a scene named for a hand and a gun");
        assert_eq!(dressed_as(&app, elbow), Some(GoreKind::Skin(Region::Limb)), "an elbow was taken for a bow, which is a weapon");
        assert_eq!(dressed_as(&app, hat), Some(GoreKind::Skin(Region::Head)), "the added rule lost to the preset's");
        assert_eq!(dressed_as(&app, fore), Some(GoreKind::Skin(Region::Limb)), "ForeArm.L has the word arm");
    }

    /// **`humanoid` against the names real characters carry**, taken from the glTF files of the game
    /// this crate is developed in: six characters from one kit and a creature. Each row is a primitive
    /// as Bevy's loader names it, `{mesh}.{material}`, under its node. The body is skin, and the
    /// wardrobe over it is cloth, and the gear on it is left alone: a belt, a backpack and a chest rig
    /// were dressed as torso skin by the first version, and a shot to the chest wounded the rig in
    /// front of the body instead of the body. A holster on a thigh is a holster, and hair on a body
    /// mesh is hair.
    #[test]
    fn humanoid_sorts_the_body_the_wardrobe_and_the_gear_of_a_real_character_kit() {
        use GoreKind::{Cloth, Skin};
        let scene = GoreScene::humanoid();
        let kind = |primitive: &str, node: &str| scene.gore_for(&[words(primitive), words(node)]).map(|g| g.kind);
        let rows: [(&str, &str, Option<GoreKind>); 24] = [
            ("valkyrie_body.valkyrie_skin", "valkyrie_body", Some(Skin(Region::Torso))),
            ("fieldop_body.fieldop_skin", "fieldop_body", Some(Skin(Region::Torso))),
            ("scientist_base.scientist_base_mat", "scientist_base", Some(Cloth)),
            ("valkyrie_bodysuit.valkyrie_bodysuit_mat", "valkyrie_bodysuit", Some(Cloth)),
            ("fieldop_jumpsuit.fieldop_jumpsuit_mat", "fieldop_jumpsuit", Some(Cloth)),
            ("scientist_labcoat.scientist_labcoat_fabric", "scientist_labcoat", Some(Cloth)),
            ("makarov_vest.makarov_vest_mat", "makarov_vest", Some(Cloth)),
            ("cipher_hazmatsuit.003.cipher_hazmatsuit_mat", "cipher_hazmatsuit", Some(Cloth)),
            ("fieldop_boots.fieldop_boots_leather", "fieldop_boots", Some(Cloth)),
            ("valkyrie_gloves.valkyrie_gloves_mat", "valkyrie_gloves", Some(Cloth)),
            ("valkyrie_boots_laces.valkyrie_boots_laces_mat", "valkyrie_boots_laces", None),
            ("valkyrie_gloves_plates.valkyrie_gloves_plates_mat", "valkyrie_gloves_plates", None),
            ("valkyrie_belt_hook_back.valkyrie_belt_hook_back_mat", "valkyrie_belt_hook_back", None),
            ("valkyrie_backpack.valkyrie_backpack_mat", "valkyrie_backpack", None),
            ("valkyrie_chestrig.valkyrie_chestrig_mat", "valkyrie_chestrig", None),
            ("valkyrie_kneepads.valkyrie_kneepads_mat", "valkyrie_kneepads", None),
            ("valkyrie_eyewear.valkyrie_eyewear_mat", "valkyrie_eyewear", None),
            ("valkyrie_facewear.valkyrie_facewear_mat", "valkyrie_facewear", None),
            ("thigh_holster.thigh_holster_mat", "thigh_holster", None),
            ("ammo_pouch.ammo_pouch_mat", "ammo_pouch", None),
            ("rifle.rifle_mat", "rifle", None),
            ("fieldop_body.001.hair_cap_fieldop", "fieldop_hair", None),
            ("SCP150_Legs.SCP150_Chitin", "SCP150_Legs", Some(Skin(Region::Limb))),
            ("cipher_hazmattrim.003.cipher_hazmattrim_mat", "cipher_hazmattrim", None),
        ];
        for (primitive, node, want) in rows {
            assert_eq!(kind(primitive, node), want, "{primitive} under {node}");
        }
    }

    /// How a name is cut into words, which is what a rule matches against.
    #[test]
    fn names_are_cut_into_words_at_case_punctuation_and_digits() {
        let cut = |s: &str| words(s).join(" ");
        assert_eq!(cut("ArmL"), "arm l");
        assert_eq!(cut("Arm_L"), "arm l");
        assert_eq!(cut("arm.L"), "arm l");
        assert_eq!(cut("Mesh.001"), "mesh 001");
        assert_eq!(cut("LegR2"), "leg r 2");
        assert_eq!(cut("XMLFile"), "xml file");
        assert_eq!(cut("Armature"), "armature");
        assert_eq!(cut("__"), "");
    }

    /// **Bevy's own fixed step is 64 Hz and the dial's is 60**, and a game that missed the checklist
    /// ran every bleed 6.7 per cent fast without a word. The check names the rate it found, and says
    /// nothing for the rate the dials were authored for, for a step that rounds to it, or for none.
    #[test]
    fn a_fixed_step_that_is_not_the_dials_rate_is_reported() {
        let step = |hz: f64| std::time::Duration::from_secs_f64(1.0 / hz);
        assert_eq!(fixed_rate_mismatch(60, step(64.0)), Some(64.0), "Bevy's default step went unreported");
        assert_eq!(fixed_rate_mismatch(60, step(60.0)), None);
        assert_eq!(fixed_rate_mismatch(60, step(60.3)), None, "a rounding of the same rate was reported");
        assert_eq!(fixed_rate_mismatch(144, step(60.0)), Some(60.0));
        assert_eq!(fixed_rate_mismatch(60, std::time::Duration::ZERO), None);
    }

    /// The system, not just the arithmetic: an app on Bevy's default 64 Hz step is told so, once, on
    /// the first tick and not again, and one on the dials' own 60 Hz is told nothing.
    #[test]
    fn the_first_tick_names_a_fixed_step_that_is_not_the_dials_rate_once() {
        use bevy::log::tracing_subscriber::fmt;
        use std::cell::RefCell;
        use std::io::Write;
        thread_local! {
            static HEARD: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        }
        struct Sink;
        impl Write for Sink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                HEARD.with(|h| h.borrow_mut().extend_from_slice(buf));
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        // **A global subscriber writing to a per-thread buffer.** A scoped one is not enough: a
        // callsite's interest is cached process-wide, so a test on another thread that reached this
        // warning first, with no subscriber, silenced it for this one — it failed one run in three on a
        // 24-thread machine. Global, every callsite is interesting from the start; per thread, every
        // other test's warnings land in its own buffer, not this one's.
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            let _ = bevy::log::tracing::subscriber::set_global_default(fmt().with_writer(|| Sink).finish());
        });
        let heard = |hz: Option<f64>| {
            HEARD.with(|h| h.borrow_mut().clear());
            let mut app = headless();
            if let Some(hz) = hz {
                app.insert_resource(Time::<Fixed>::from_hz(hz));
            }
            // On this thread, so its buffer is the one that hears it.
            app.edit_schedule(FixedUpdate, |schedule| {
                schedule.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
            });
            for _ in 0..3 {
                app.world_mut().run_schedule(FixedUpdate);
            }
            HEARD.with(|h| String::from_utf8_lossy(&h.borrow()).matches("FixedUpdate runs at").count())
        };
        assert_eq!(heard(None), 1, "the default 64 Hz step was not reported exactly once");
        assert_eq!(heard(Some(60.0)), 0, "the dials' own rate was reported");
    }

    /// **A hit from a contact point and its normal lands on that point.** A game's physics reports where
    /// something touched and the surface's normal, not a ray. The wound's own mesh-local point is
    /// compared with the point given, on a face that is not the one a ray from the origin would find.
    #[test]
    fn a_hit_at_a_contact_point_lands_on_that_point() {
        let mut app = headless();
        let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.4, 0.4).mesh().build());
        let body = app.world_mut().spawn((Mesh3d(cube), Transform::IDENTITY, GlobalTransform::IDENTITY, Gore::skin(Region::Limb))).id();
        app.world_mut().run_schedule(FixedUpdate);
        let (point, normal) = (Vec3::new(0.2, 0.05, -0.11), Vec3::X);
        app.world_mut().write_message(GoreHit::impact_at(body, point, normal, 0.03, 4.0));
        app.world_mut().run_schedule(FixedUpdate);
        let site = app.world().get::<Wounds>(body).and_then(|w| w.0.first().copied()).expect("no wound opened");
        assert!(site.at_local.distance(point) < 1.0e-3, "the wound is at {:?}, not {point:?}", site.at_local);
        let slash = GoreHit::slash(body, Vec3::ZERO, Vec3::ZERO, Vec3::Y, 0.05, 3.0).at_surface(point, normal);
        assert!(slash.dir.dot(-normal) > 0.999 && slash.from.distance(point + normal * SURFACE_STAND_OFF) < 1.0e-6, "at_surface did not place the ray");
        assert!(matches!(slash.kind, HitKind::Slash { along, .. } if along == Vec3::Y), "at_surface changed the kind");
        assert_eq!(GoreHit::impact_at(body, point, Vec3::ZERO, 0.03, 4.0).dir, Vec3::ZERO, "a zero normal gave a direction");
    }

    /// **A skinned body breaks where its skeleton is.** The fracture is baked from the mesh asset,
    /// which is the bind pose, and until this the pieces appeared there: here, at the entity's own
    /// transform, 5 m off, with the arm straight. With the elbow bent a quarter turn, the forearm's
    /// pieces must be carried up with the forearm and turned with it, and the upper arm's must stay.
    /// The breakable sits on the skinned part itself, baked in the mesh's own space.
    #[test]
    fn a_skinned_body_breaks_where_its_skeleton_is() {
        let pieces_at = |bend: f32| -> Vec<GorePiece> {
            let (mut app, body, _, _) = rigged_arm(bend);
            app.world_mut().resource_mut::<GoreDials>().spawn_fragments = false;
            let mesh = arm();
            let proxy = [
                crate::ProxyCell::from_box(Vec3::new(0.5, 0.0, 0.0), Vec3::new(0.5, 0.2, 0.2)),
                crate::ProxyCell::from_box(Vec3::new(1.5, 0.0, 0.0), Vec3::new(0.5, 0.2, 0.2)),
            ];
            let fracture = crate::fracture_mesh(&[(&mesh, Mat4::IDENTITY)], &proxy, &crate::CutSettings::new(4, 0.05, 3));
            let breakable = GoreBreakable::from_fracture(fracture, Region::Limb, &crate::cross_section::Scale::default());
            app.world_mut().entity_mut(body).insert(breakable);
            // Aimed at the forearm as it is drawn: half a metre along it from the elbow.
            let forearm = elbow(bend).transform_point(Vec3::new(0.5, 0.0, 0.0));
            app.world_mut().write_message(GoreHit::impact(body, Vec3::new(forearm.x, forearm.y, 3.0), Vec3::NEG_Z, 0.03, 60.0));
            app.world_mut().run_schedule(FixedUpdate);
            let messages = app.world().resource::<Messages<GoreFractured>>();
            let broke: Vec<GoreFractured> = messages.get_cursor().read(messages).cloned().collect();
            assert_eq!(broke.len(), 1, "the arm broke {} times at {bend}", broke.len());
            broke[0].pieces.clone()
        };
        let straight = pieces_at(0.0);
        assert!(!straight.is_empty(), "a straight arm broke into nothing");
        assert!(straight.iter().all(|p| p.transform.translation.y.abs() < 0.3 && (-0.3..2.3).contains(&p.transform.translation.x)), "a straight arm's pieces are not along it");
        let bend = core::f32::consts::FRAC_PI_2;
        let bent = pieces_at(bend);
        let highest = bent.iter().max_by(|a, b| a.transform.translation.y.total_cmp(&b.transform.translation.y)).expect("no pieces");
        assert!(highest.transform.translation.y > 0.4, "no piece was carried up with the forearm: highest at {:?}", highest.transform.translation);
        assert!(highest.transform.rotation.angle_between(Quat::from_rotation_z(bend)) < 0.05, "the forearm's piece is not turned with it: {:?}", highest.transform.rotation);
        assert!(bent.iter().any(|p| p.transform.translation.y.abs() < 0.3 && p.transform.translation.x < 1.0), "the upper arm's pieces moved with the forearm");
        assert!(bent.iter().all(|p| p.transform.translation.x < 3.0 && p.transform.translation.y < 3.0), "a piece is at the entity's own transform, 5 m off");
    }

    /// **The wound bed a slash opens is not a second skin.** The preset spawns it as a child of the
    /// slashed mesh, so it sits below a node named for the limb and would match that limb's rule.
    /// Measured on the skinned demo before this was excluded: the bed was dressed as `Skin(Limb)` and
    /// every frame after the first slash differed from the hand-dressed clip.
    #[test]
    fn a_wound_bed_below_a_dressed_skin_is_not_dressed_as_a_second_skin() {
        let mut app = headless();
        let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.4, 0.4).mesh().build());
        let world = app.world_mut();
        let root = world.spawn((Transform::IDENTITY, GlobalTransform::IDENTITY, GoreScene::skin(Region::Torso).otherwise_nothing().with("arm", Gore::skin(Region::Limb)))).id();
        let arm = world.spawn((Name::new("ArmL"), Mesh3d(cube.clone()), ChildOf(root))).id();
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(dressed_as(&app, arm), Some(GoreKind::Skin(Region::Limb)));
        let bed = app.world_mut().spawn((Mesh3d(cube), LacerationBed, ChildOf(arm))).id();
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(dressed_as(&app, bed), None, "the wound bed was dressed");
    }

    /// **A surface may carry its own canvas size**, and every canvas it owns takes it: a skin's
    /// flaymap, wetmap and dermis map, a cloth's wetmap, a floor's. One at 32 against one at the
    /// dial's 128 is a sixteenth of the image bytes, and a hit on the small one still opens a wound.
    /// Zero is not a size and leaves the dial's.
    #[test]
    fn a_surface_is_dressed_at_its_own_canvas_size() {
        let mut app = headless();
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.4, 0.4).mesh().build());
        let plane = app.world_mut().resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Y, Vec2::splat(1.0)).mesh().build());
        let world = app.world_mut();
        let mut spawn = |gore: Gore, mesh: &Handle<Mesh>| world.spawn((Mesh3d(mesh.clone()), Transform::IDENTITY, GlobalTransform::IDENTITY, gore)).id();
        let small = spawn(Gore::skin(Region::Limb).with_canvas(32), &mesh);
        let big = spawn(Gore::skin(Region::Limb), &mesh);
        let zero = spawn(Gore::skin(Region::Limb).with_canvas(0), &mesh);
        let cloth = spawn(Gore::cloth().with_canvas(64), &mesh);
        let floor = spawn(Gore::floor().with_canvas(48), &plane);
        app.world_mut().run_schedule(FixedUpdate);
        let dial = app.world().resource::<GoreDials>().canvas;
        let sizes = |e: Entity| {
            let w = app.world();
            (
                w.get::<WetCanvas>(e).map(WetCanvas::size),
                w.get::<FlayCanvas>(e).map(FlayCanvas::size),
                w.get::<DermisCanvas>(e).map(DermisCanvas::size),
            )
        };
        assert_eq!(sizes(small), (Some(32), Some(32), Some(32)), "every canvas of a skin takes its edge");
        assert_eq!(sizes(big), (Some(dial), Some(dial), Some(dial)));
        assert_eq!(sizes(zero), sizes(big), "zero was taken for a size");
        assert_eq!(sizes(cloth), (Some(64), None, None));
        assert_eq!(sizes(floor), (Some(48), None, None));
        let image = |e: Entity| app.world().get::<WetCanvas>(e).map(|c| c.image_bytes()).unwrap_or(0);
        assert_eq!(image(small) * (dial as usize / 32).pow(2), image(big), "bytes did not follow the edge squared");
        app.world_mut().write_message(GoreHit::impact(small, Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, 0.03, 4.0));
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(open_wounds(&app, small), 1, "a hit on the small canvas opened no wound");
    }

    /// **What a dressed surface costs**, from the canvases themselves. Not a test: it prints the table
    /// in `docs/preset.md`. Run it with
    /// `cargo test -p bevy_carnage --lib -- dressed_surface_memory --ignored --nocapture`.
    #[test]
    #[ignore]
    fn dressed_surface_memory() {
        let kib = |b: usize| format!("{:>7.0} KiB", b as f64 / 1024.0);
        println!("{:<12}{:>6}  {:>11}  {:>11}  {:>11}", "surface", "edge", "main heap", "images x1", "total (x2)");
        for edge in [64u32, 128, 256, 512] {
            for (name, gore) in [("skin", Gore::skin(Region::Torso)), ("cloth", Gore::cloth()), ("floor", Gore::floor()), ("wall", Gore::wall())] {
                let mut app = headless();
                let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.4, 0.4, 0.4).mesh().build());
                let e = app.world_mut().spawn((Mesh3d(mesh), Transform::IDENTITY, GlobalTransform::IDENTITY, gore.with_canvas(edge))).id();
                app.world_mut().run_schedule(FixedUpdate);
                let w = app.world();
                let dermis = w.get::<DermisCanvas>(e).or(w.get::<ClothHoles>(e).map(ClothHoles::canvas));
                let (wet, flay) = (w.get::<WetCanvas>(e), w.get::<FlayCanvas>(e));
                let heap = wet.map_or(0, WetCanvas::heap_bytes) + flay.map_or(0, FlayCanvas::heap_bytes) + dermis.map_or(0, DermisCanvas::heap_bytes);
                let image = wet.map_or(0, WetCanvas::image_bytes) + flay.map_or(0, FlayCanvas::image_bytes) + dermis.map_or(0, DermisCanvas::image_bytes);
                println!("{name:<12}{edge:>6}  {}  {}  {}", kib(heap), kib(image), kib(heap + 2 * image));
            }
        }
    }
}
