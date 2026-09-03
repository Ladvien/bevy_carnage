#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod audit;
mod bake;
mod bond;
mod bore;
#[cfg(feature = "vfx")]
mod decal;
mod feel;
mod mesh;
mod order;
mod policy;
mod proxy;
mod severance;
mod soup;
mod tree;
mod v3;
#[cfg(feature = "vfx")]
mod vfx;
mod wound;

pub use audit::{SolidAudit, SurfaceReport, audit_cell, audit_proxies, audit_proxy, audit_render};
pub use bake::{
    DetachedChunk, DetachedPart, EjectaChunk, Fragment, FractureBores, FractureCache, FractureProxy,
    FractureSubject, bake_fractures, materialise_fragments,
};
pub use bond::{Bond, BondGraph, BondId, BondSet};
pub use bore::{Bore, MAX_SAG, MAX_SIDES, MIN_ROUND_SIDES, sides_for};
#[cfg(feature = "vfx")]
pub use decal::{
    PoolDecal, StainMask, StainMasks, spawn_pool, spawn_stain, update_pool_decals,
};
pub use feel::{hitstop_ticks, shake_offset, trauma_for};
pub use policy::coalesce_hitstop;
pub use mesh::{Ejecta, Fracture, FragmentGeometry, FragmentSolid, fracture_mesh};
pub use policy::{
    DecalBudget, FlashGate, GorePolicy, GoreTier, WCAG_FLASHES_PER_SECOND, occludes_aim,
};
pub use proxy::ProxyCell;
pub use severance::{Reach, capsule, directional, radial, spread, swept_triangle};
pub use tree::{FragmentId, FragmentTree, TreeNode};
#[cfg(feature = "vfx")]
pub use vfx::{
    BleedingChunk, CarnageEffects, CarnageVfxPlugin, CarnageVfxSystems, EffectFade, EffectTtl,
    RibbonInstance, arterial_spurt, gib_ribbon, mist_puff, spatter_burst, wound_seep,
};
pub use wound::{
    CapFace, Wound, cap_faces, largest_cap, wound_from_ejecta, wound_of_channel, wounds_from_bonds,
    wounds_from_reach,
};

/// **Everything the blood model moved to `bloodstain`, re-exported under the names it had here.**
///
/// The crate's public surface keeps them, so `use bevy_carnage::{Droplet, Stain, Pool, hash_f32}`
/// resolves exactly as it did at `0.1.1`. What a caller gains is that every one of these is now
/// usable **without Bevy** by depending on the leaf directly — and what nothing gains is a second
/// home, because these are re-exports rather than wrappers.
///
/// `hash_f32` in particular had to keep both its name and its bits: the consuming game's
/// `tests/rng_guard.rs` asserts its own `util::hash_f32` is bit-identical to this symbol.
///
/// **`Bleed` is a plain value here, not a `Component`.** The leaf has no ECS to derive one from, so
/// the component is [`Bleeding`], a newtype over it — the same facade-newtype shape `bevy_stigmergy`
/// and `bevy_light_grid` are reached through in this workspace's game.
pub use bloodstain::{
    Appearance, BACK_SPATTER_SPEED, BLOOD_DENSITY, BLOOD_SURFACE_TENSION, Bleed, BloodSettings,
    Droplet, FORWARD_SPATTER_SPEED, Impact, PatternClass, Pool, Stain, StainShape, WELD, WoundKind,
    absorb, appearance, area_of_origin, droplet, droplet_count, droplets, flow, flows, hash_f32,
    landing, pick, pulse_period, pulse_wound, pulses_on, spread_pools, viscosity, wound_seed,
    yield_stress,
};
pub use bloodstain::stain::{impact_at_plane, rasterise, stain_radius, stain_shape, stains};
/// The blood model's own modules, re-exported so a caller can reach the parts that have no
/// single-name entry point — `bevy_carnage::blood::patterns::cast_off`, `blood::rheo::flows`,
/// `blood::dry::dry_ticks`. One path to each function, spelled the way the leaf spells it.
pub use bloodstain as blood;

use bevy::prelude::*;

/// **How a subject was loaded when it broke.**
///
/// This is the distinction the fracture literature says its own methods lack, named. Sellán et al.,
/// *Breaking Good: Fracture Modes for Realtime Destruction* (`doi:10.1145/3549540`, §6), note that
/// their fault is *the same regardless of the directionality of the impact*, and name uniaxial
/// tension, pure shear and torsion as the missing cases. That missing distinction is exactly the
/// clinical one — a spiral fracture, a transverse fracture and a butterfly wedge are the same bone
/// under three loads, and a player can tell them apart.
///
/// **Append-only**, like every other `#[repr(u32)]` in this family: the discriminant travels in
/// authored data and in a genome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u32)]
pub enum LoadingMode {
    /// Twisted about its long axis. Cracks along a **helix**, because under torsion the tensile
    /// stress is maximum in a plane at 45° to that axis and a material weaker in tension than in
    /// shear follows it (Miyasaka et al., `doi:10.3233/BME-1991-1102`).
    Torsion = 0,
    /// Bent. Opens in **tension** on the convex face and can throw a butterfly wedge toward the
    /// compression face (Isa et al., `doi:10.1016/j.forsciint.2021.110899`).
    Bending = 1,
    /// Loaded along its own axis. Fails across its narrowest cross-section, which is what the
    /// weak-axis sample already produced.
    Axial = 2,
    /// Struck hard enough that no plane is preferred — comminution. The fragment count comes from
    /// the energy rather than from an artist constant; see [`grady_mott_target`].
    DirectHighEnergy = 3,
}

/// **What is breaking.** Cortical bone, trabecular bone, or soft tissue.
///
/// The three differ in the one property that decides what the debris looks like: strain to failure.
/// Cortical bone fails at roughly 2 % strain and splinters into long thin fragments; trabecular bone
/// tolerates around 30 % and *crushes* rather than shattering; soft tissue tears. So this is not a
/// material name for flavour — it changes the fragment aspect ratio and the piece count.
///
/// **Append-only.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u32)]
pub enum TissueClass {
    /// Dense cortical bone. Splinters: long, thin, sharp.
    Cortical = 0,
    /// Cancellous/trabecular bone. Compacts: few pieces, no shards.
    Trabecular = 1,
    /// Soft tissue. What every bake before this enum existed behaved as.
    Soft = 2,
}

/// **How the cut plane is chosen.** One policy with a parameter, not a switch between two engines.
///
/// [`FaultPolicy::WeakAxis`] is what this crate always did: sample a few candidate normals and keep
/// the one the piece is longest along. [`FaultPolicy::Morphology`] adds the loading mode, and
/// [`crate::soup::choose_plane`] matches on this **once** — there is no second entry point, no
/// "legacy" branch, and no dial that selects between two implementations of the same thing.
///
/// `CutSettings::new` sets `WeakAxis`, which is why every frozen bake in this crate is unmoved by the
/// existence of the other arm.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FaultPolicy {
    /// Cut across the narrowest cross-section. Direction-blind, and correct for an axial load.
    WeakAxis,
    /// Cut the way this loading mode and this tissue actually fail.
    Morphology {
        /// How it was loaded.
        mode: LoadingMode,
        /// What is breaking.
        tissue: TissueClass,
        /// The subject's long axis, subject-local. A torsional helix is measured against it, and a
        /// bend's tension face is the side it points away from.
        axis: Vec3,
        /// Applied torque, N·m. Drives the helix pitch.
        torque: f32,
        /// Impulse delivered, N·s. Below [`FractureSettings::greenstick_impulse`] a bend produces a
        /// **greenstick** — no fault at all, and a permanent bend instead.
        impulse: f32,
    },
}

/// **The geometry dials for one bake**, without the ECS sizing policy that chooses `target`.
///
/// [`FractureSettings`] is the resource a game authors once; this is what a single cut actually
/// needs, and it is what [`fracture_mesh`] takes. Build one from a `FractureSettings` with
/// [`FractureSettings::cut_for`], or write it out directly when there is no `App` in sight.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CutSettings {
    /// Finest fragment count to cut down to.
    pub target: usize,
    /// Stop cutting a piece once its extent drops below this fraction of the whole subject's.
    pub min_fraction: f32,
    /// Cuts from a proxy cell to the finest fragment — the hierarchy's memory bound.
    pub max_depth: u16,
    /// **How far a cut plane may slide off the piece's centre**, as a fraction of how far the piece
    /// reaches along that plane's own normal.
    ///
    /// `0.0` puts every plane exactly through the centroid, which splits every piece near enough in
    /// half and is why an unjittered bake reads as uniform shards rather than debris. Real fragment
    /// volumes follow Mott's distribution — many small, few large — and an off-centre cut is what
    /// produces that spread, compounding with depth.
    ///
    /// Measured against the piece's own span along the normal rather than its bounding box, so the
    /// plane cannot slide out of a cell that is thin in that direction and lose the cut. Values at
    /// or above `1.0` are refused by [`FractureSettings::validate`] for that reason.
    pub plane_jitter: f32,
    /// **How much the "cut the biggest piece next" rule may be nudged**, so the sequence does not
    /// march down a strict volume order and level every piece toward the same size.
    ///
    /// `0.0` is the strict largest-first rule. Higher values let a slightly smaller piece be chosen,
    /// widening the size distribution. The nudge is a stable hash of the piece's own node id, so it
    /// is reproducible and does not shift as the frontier grows.
    pub size_spread: f32,
    /// **How hard to break a piece across its narrowest dimension instead of at a random angle.**
    ///
    /// A uniformly-sampled cut normal slices diagonally through whatever it is given, which on a
    /// limb produces long oblique wedges — and on a body reads as a statue shattering rather than
    /// something coming apart. Sellán et al.'s finding is that geometric prefracture is blind to
    /// where a shape is *weak*, and a shape is weak across its thin cross-sections.
    ///
    /// This buys most of that cheaply: sample several candidate normals and keep the one the piece
    /// is *longest* along, because the cut face is perpendicular to the normal, so the longest axis
    /// gives the smallest cross-section. `0.0` samples once and is exactly the old behaviour; `1.0`
    /// samples eight and takes the best.
    pub weak_axis: f32,
    /// **How much the drawn cut face is crumpled**, as a fraction of its own radius.
    ///
    /// A flat cut face is the visual language of cleaved stone and ice, and no amount of fragment
    /// shaping changes that — it is what a plane through a solid leaves behind. This displaces the
    /// *interior* of the emitted cap, never its boundary, so the seam against the skin stays shut.
    ///
    /// **It touches Tier B only.** The proxy cell stays exactly planar, so the collider is still one
    /// convex hull and every watertightness guarantee is untouched — [`audit_proxy`] measures the
    /// cell, not this. `0.0` leaves the cap flat.
    pub cap_relief: f32,
    /// **How much to round the drawn fragment**, so it reads as a lump rather than a shard.
    ///
    /// Sharp dihedral edges are the visual signature of brittle fracture — ice, glass, cleaved stone.
    /// Shaping the pieces differently does not change that, because the edges are simply what a plane
    /// through a solid leaves behind. This subdivides the drawn surface and relaxes it, bevelling
    /// every edge and rounding every corner, and re-derives smooth normals so each facet stops being
    /// lit as a separate plane.
    ///
    /// **Tier B only, like [`cap_relief`](Self::cap_relief).** The proxy cell is untouched, so the
    /// collider is still one exact convex hull and every watertightness guarantee holds. The drawn
    /// mesh ends up slightly *inside* its hull, which is the harmless direction.
    ///
    /// `0.0` leaves the fragment exactly as cut. Around `0.5` reads as flesh; `1.0` is a pebble.
    pub soften: f32,
    /// **How much the ejected debris is rounded**, separately from [`soften`](Self::soften).
    ///
    /// Two values because the constraint that pins one does not apply to the other, and the gap is
    /// measured. `soften` relaxes each drawn piece *independently* and does not pin the boundary it
    /// shares with the piece beside it, so raising it on a **bored** subject pulls the wedges around a
    /// channel apart: at 0.40 on the demo body the eight shards of each hole separate visibly, red
    /// gaps radiate from every entry wound and the subject reads as disassembled rather than shot.
    /// (Compact fracture fragments barely show it — a bore's shards are long thin wedges meeting over
    /// large faces through the middle of the cell, which is what makes the shrink obvious.)
    ///
    /// **Ejecta share a boundary with nothing.** They are debris that already left the subject, so
    /// nothing can open up beside them and they can be rounded freely — which is most of the
    /// difference between a plug's pieces reading as sharp coins and reading as lumps of meat. A
    /// caller wanting one look everywhere sets both the same.
    ///
    /// Tier B, like `soften`: the convex cells are untouched, so every collider and every audit
    /// verdict is identical at any value.
    pub ejecta_soften: f32,
    /// Drives every plane direction and every jitter draw — the only source of variation.
    pub seed: u32,
    /// **Channels subtracted from the proxy before any cut** — a bullet hole is one of these.
    ///
    /// Applied to the caller's cells, so a bored cell arrives at the cut loop as several convex
    /// shards and the hole is part of the subject's shape rather than part of its breakage. Empty is
    /// the whole of the previous behaviour: with no bores the bake is byte-identical to one taken
    /// before this field existed. See [`Bore`].
    #[cfg_attr(feature = "serde", serde(default))]
    pub bores: Vec<Bore>,
    /// **How the cut plane is chosen** — see [`FaultPolicy`].
    ///
    /// Defaults to [`FaultPolicy::WeakAxis`] through [`CutSettings::new`], which is what every bake
    /// this crate has ever produced already did, so the frozen fracture goldens are unmoved by this
    /// field existing. A caller that knows how the blow arrived sets `Morphology` and gets the
    /// clinical silhouette for it.
    #[cfg_attr(feature = "serde", serde(default))]
    pub fault: FaultPolicy,
    /// Degrees the helical fault rotates per successive cut under torsion — see
    /// [`FractureSettings::spiral_pitch_deg`].
    #[cfg_attr(feature = "serde", serde(default = "default_spiral_pitch_deg"))]
    pub spiral_pitch_deg: f32,
    /// Impulse below which a bend is a greenstick rather than a fault — see
    /// [`FractureSettings::greenstick_impulse`].
    #[cfg_attr(feature = "serde", serde(default = "default_greenstick_impulse"))]
    pub greenstick_impulse: f32,
    /// **What is breaking** — see [`TissueClass`]. Biases fragment shape and count.
    ///
    /// Read even under [`FaultPolicy::WeakAxis`]? **No.** The tissue bias is part of the morphology
    /// policy and is read only there, because applying it under `WeakAxis` would move every existing
    /// bake — and a dial that silently changes a frozen output is the thing this crate's goldens
    /// exist to catch. `Morphology` carries its own `tissue`; this field is what a caller sets when
    /// it wants the tissue *without* naming a loading mode, and `cut_for` copies it into the policy.
    #[cfg_attr(feature = "serde", serde(default))]
    pub tissue: TissueClass,
}

impl CutSettings {
    /// The three dials a caller always has an opinion about, with [`FractureSettings`]'s shipped
    /// defaults for the rest. Assign to the remaining fields to change them.
    pub fn new(target: usize, min_fraction: f32, seed: u32) -> Self {
        let d = FractureSettings::default();
        CutSettings {
            target,
            min_fraction,
            max_depth: d.max_depth,
            plane_jitter: d.plane_jitter,
            size_spread: d.size_spread,
            weak_axis: d.weak_axis,
            cap_relief: d.cap_relief,
            soften: d.soften,
            ejecta_soften: d.ejecta_soften,
            seed,
            bores: Vec::new(),
            // The values every bake before these fields existed behaved as. Stated rather than
            // derived from `Default`, because the whole point is that they are the *old* behaviour.
            fault: FaultPolicy::WeakAxis,
            tissue: TissueClass::Soft,
            spiral_pitch_deg: d.spiral_pitch_deg,
            greenstick_impulse: d.greenstick_impulse,
        }
    }
}

/// How hard to break things. Twelve dials, all bake-time — nothing here decides how a chunk *moves*
/// after it exists, because that is the caller's physics, not this crate's business.
///
/// The piece count is driven by the mesh's own bounding size rather than authored per asset:
/// `pieces_base` is the count at `ref_extent`, scaled by how much bigger or smaller this mesh actually
/// is, then clamped. So a rat and an ogre both break sensibly from one setting.
///
/// **These set the *finest* granularity, not the only one.** A bake keeps the whole hierarchy it cut
/// through, so `max_pieces` is the ceiling a caller can ask for, not the count it must take —
/// [`FragmentTree::frontier_of`] reads the same bake back at three pieces or at all of them.
#[derive(Resource, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct FractureSettings {
    /// Fragment count at [`ref_extent`](Self::ref_extent); scaled by the mesh's actual bounding size.
    pub pieces_base: i32,
    /// Reference subject half-extent that `pieces_base` is tuned for.
    pub ref_extent: f32,
    /// Clamp on the fragment count (lower).
    pub min_pieces: i32,
    /// Clamp on the fragment count (upper — this one bounds mesh and entity growth, so it is the
    /// dial that keeps a huge mesh from becoming a thousand rigid bodies).
    pub max_pieces: i32,
    /// Stop cutting a piece once its extent drops below this fraction of the whole mesh's extent.
    pub min_fraction: f32,
    /// **Cuts from a proxy cell to the finest fragment — the hierarchy's memory bound.**
    ///
    /// A bake keeps every piece it ever held, not only the final ones, because that forest is what
    /// lets one bake answer both "break this into three" and "break this into forty". The cost is
    /// payload: each level of the forest holds the whole subject over again, so total geometry is
    /// roughly this number times the subject's own triangle count.
    ///
    /// The default is deliberately slack enough never to bind at the default `max_pieces` — it is a
    /// guard against a pathologically unbalanced subject, not a tuning dial. Lower it when memory
    /// matters more than the finest granularity.
    pub max_depth: u16,
    /// How far a cut plane may slide off centre — see [`CutSettings::plane_jitter`].
    pub plane_jitter: f32,
    /// How much the largest-first cut order may be nudged — see [`CutSettings::size_spread`].
    pub size_spread: f32,
    /// How hard to cut across the narrow dimension — see [`CutSettings::weak_axis`].
    pub weak_axis: f32,
    /// How much the drawn cut face is crumpled — see [`CutSettings::cap_relief`].
    pub cap_relief: f32,
    /// How much the drawn fragment is rounded — see [`CutSettings::soften`].
    pub soften: f32,
    /// How much the ejected debris is rounded — see [`CutSettings::ejecta_soften`].
    ///
    /// **Carries a serde default, and it is the first field here that needed one.** This struct is
    /// `deny_unknown_fields` with no struct-level default, so every field is *required* on
    /// deserialize — which means adding one silently breaks every authored file that enumerated the
    /// others, at load time, in a way no compile catches. Measured on the consuming game: its
    /// `config.ron` lists all eleven previous dials exhaustively, so shipping this field without a
    /// default would have refused the config at startup.
    ///
    /// The pair is the right combination rather than a weakening: a *misspelled* field still fails,
    /// because `deny_unknown_fields` catches it, while a *missing* one takes the shipped value. Any
    /// dial added here from now on should do the same.
    #[cfg_attr(feature = "serde", serde(default = "default_ejecta_soften"))]
    pub ejecta_soften: f32,
    /// **Degrees the helical fault rotates per successive cut under torsion.**
    ///
    /// A torsional fracture is a spiral, and a spiral is a sequence of planes each rotated a little
    /// about the long axis. 30° over the handful of cuts a limb takes traces most of a half-turn,
    /// which is what gives the long sharp ends a spiral fracture is recognised by.
    #[cfg_attr(feature = "serde", serde(default = "default_spiral_pitch_deg"))]
    pub spiral_pitch_deg: f32,
    /// **Impulse below which a bend produces a greenstick instead of a fault**, N·s.
    ///
    /// Greenstick is not a loading mode, it is an *outcome*: the tension cortex opens, the far cortex
    /// does not, and the bone stays permanently bent
    /// (`doi:10.3390/jimaging11060187`). Below this, `choose_plane` returns **no plane at all** and
    /// the caller gets one fragment plus [`Fracture::bent`].
    #[cfg_attr(feature = "serde", serde(default = "default_greenstick_impulse"))]
    pub greenstick_impulse: f32,
    /// Fracture toughness of cortical bone, J/m² — the `G_c` of Grady's energy balance.
    #[cfg_attr(feature = "serde", serde(default = "default_toughness_cortical"))]
    pub toughness_cortical: f32,
    /// Fracture toughness of trabecular bone, J/m². An order below cortical: it crushes.
    #[cfg_attr(feature = "serde", serde(default = "default_toughness_trabecular"))]
    pub toughness_trabecular: f32,
    /// Fracture toughness of soft tissue, J/m². High, because flesh tears rather than shatters.
    #[cfg_attr(feature = "serde", serde(default = "default_toughness_soft"))]
    pub toughness_soft: f32,
    /// Density used by Grady's fragment-size law, kg/m³. Bone is about 1900.
    #[cfg_attr(feature = "serde", serde(default = "default_density_kg_m3"))]
    pub density_kg_m3: f32,
}

/// The shipped [`FractureSettings::ejecta_soften`], for serde to reach when an authored file predates
/// the field. **Must agree with [`FractureSettings::default`]**, or a file that omits the dial gets a
/// different look than one that never had it — pinned by `the_serde_default_matches_the_shipped_one`.
fn default_ejecta_soften() -> f32 {
    0.55
}

/// Degrees of helix per successive torsional cut. See [`FractureSettings::spiral_pitch_deg`].
fn default_spiral_pitch_deg() -> f32 {
    30.0
}
/// N·s below which a bend is a greenstick. See [`FractureSettings::greenstick_impulse`].
fn default_greenstick_impulse() -> f32 {
    12.0
}
/// J/m². Cortical bone's fracture toughness, the low end of the measured range for transverse
/// crack growth — the value that makes a rifle round comminute and a pistol round wedge.
fn default_toughness_cortical() -> f32 {
    2500.0
}
/// J/m². Trabecular bone, an order below cortical: it compacts rather than splitting.
fn default_toughness_trabecular() -> f32 {
    250.0
}
/// J/m². Soft tissue tears, so its toughness is high relative to the energies involved and the
/// characteristic fragment size it implies is large — which is why flesh yields few pieces.
fn default_toughness_soft() -> f32 {
    9000.0
}
/// kg/m³. Cortical bone is about 1900; the same value serves the whole subject because Grady's law
/// takes one density and a per-tissue density would be a second material model.
fn default_density_kg_m3() -> f32 {
    1900.0
}

impl Default for FaultPolicy {
    /// [`FaultPolicy::WeakAxis`] — what every bake before this enum existed did.
    fn default() -> Self {
        FaultPolicy::WeakAxis
    }
}

impl Default for TissueClass {
    /// [`TissueClass::Soft`] — likewise.
    fn default() -> Self {
        TissueClass::Soft
    }
}

impl Default for FractureSettings {
    fn default() -> Self {
        FractureSettings {
            pieces_base: 14,
            ref_extent: 0.5,
            min_pieces: 6,
            max_pieces: 40,
            min_fraction: 0.18,
            max_depth: 12,
            plane_jitter: 0.35,
            size_spread: 0.5,
            weak_axis: 0.75,
            cap_relief: 0.30,
            soften: 0.5,
            // **Rounded even when the body is not.** Debris shares a boundary with nothing, so the
            // constraint that forces `soften` to 0 on a bored subject does not reach it.
            ejecta_soften: 0.55,
            spiral_pitch_deg: default_spiral_pitch_deg(),
            greenstick_impulse: default_greenstick_impulse(),
            toughness_cortical: default_toughness_cortical(),
            toughness_trabecular: default_toughness_trabecular(),
            toughness_soft: default_toughness_soft(),
            density_kg_m3: default_density_kg_m3(),
        }
    }
}

impl FractureSettings {
    /// Reject a settings block that cannot produce a sane clamp.
    ///
    /// **This is a real crash, not a hypothetical.** [`bake_fractures`] feeds these straight into
    /// `i32::clamp(min, max)`, which panics when `min > max` — so an inverted pair authored in a data
    /// file takes the process down at the first death. Call this at load; failing loudly at the door is
    /// the one path, and silently swapping the pair would be a fallback.
    pub fn validate(&self) -> Result<(), String> {
        if self.min_pieces > self.max_pieces {
            return Err(format!(
                "bevy_carnage: min_pieces ({}) > max_pieces ({}) — `i32::clamp` panics on an inverted \
                 range, so fix the authored values",
                self.min_pieces, self.max_pieces
            ));
        }
        // Zero depth permits no cut at all, so the bake would return the caller's proxy cells
        // unchanged and report success. That is precisely the silent-degraded-result this crate
        // refuses to produce: a subject that never breaks is a settings bug, not a fracture.
        if self.max_depth == 0 {
            return Err(
                "bevy_carnage: max_depth is 0 — no cut is permitted, so the bake would return the \
                 proxy cells unfractured and call it done. Use 1 or more."
                    .to_string(),
            );
        }
        // At 1.0 the jitter can put the plane exactly on the far extreme of the piece, where it
        // divides nothing and the cut is lost — quietly, as one fewer fragment. Refuse at the door
        // rather than emit a bake that is short of what was asked for.
        if !(0.0..1.0).contains(&self.plane_jitter) {
            return Err(format!(
                "bevy_carnage: plane_jitter is {} — it must be in [0, 1). At 1.0 a plane can land on \
                 the edge of the piece it is meant to divide, silently costing a fragment.",
                self.plane_jitter
            ));
        }
        for (name, v) in [
            ("weak_axis", self.weak_axis),
            ("cap_relief", self.cap_relief),
            ("soften", self.soften),
            ("ejecta_soften", self.ejecta_soften),
        ] {
            if !(0.0..=1.0).contains(&v) {
                return Err(format!("bevy_carnage: {name} is {v} — it must be in [0, 1]."));
            }
        }
        if self.size_spread < 0.0 {
            return Err(format!(
                "bevy_carnage: size_spread is {} — negative would invert the cut order into \
                 smallest-first, which is not a spread. Use 0.0 or more.",
                self.size_spread
            ));
        }
        Ok(())
    }

    /// The geometry dials for one bake, with the piece count and seed the caller resolved.
    ///
    /// `target` comes from this resource's sizing policy applied to a particular mesh, and `seed`
    /// from that asset's path — neither is a property of the settings alone, which is why they are
    /// arguments rather than fields.
    /// `bores` are per-subject and come from the [`FractureBores`] component, never from this
    /// resource: a shot is an event, not an authored dial. A parameter rather than a default so the
    /// ECS path cannot forget to pass them.
    pub fn cut_for(&self, target: usize, seed: u32, bores: Vec<Bore>) -> CutSettings {
        CutSettings {
            target,
            min_fraction: self.min_fraction,
            max_depth: self.max_depth,
            plane_jitter: self.plane_jitter,
            size_spread: self.size_spread,
            weak_axis: self.weak_axis,
            cap_relief: self.cap_relief,
            soften: self.soften,
            ejecta_soften: self.ejecta_soften,
            seed,
            bores,
            // A bake driven from the resource has no blow to describe, so it takes the
            // direction-blind policy — the one every bake before `FaultPolicy` existed used. A
            // caller that *does* know how the blow arrived builds the `CutSettings` itself and sets
            // `Morphology`; there is deliberately no `cut_for_blow` beside this, because two entry
            // points to one bake is how they drift.
            fault: FaultPolicy::WeakAxis,
            tissue: TissueClass::Soft,
            spiral_pitch_deg: self.spiral_pitch_deg,
            greenstick_impulse: self.greenstick_impulse,
        }
    }
}

/// **How many fragments an impact should produce, from the energy — not from an artist constant.**
///
/// # Grady's energy balance
///
/// Grady, *"Local inertial effects in dynamic fragmentation"* (`doi:10.1063/1.329934`), balances the
/// local kinetic energy of an expanding fragment against the fracture energy needed to create its
/// surface, and gets a **characteristic fragment size**
///
/// > `s = (24 · G_c / (ρ · ε̇²))^(1/3)`
///
/// where `G_c` is the fracture toughness (J/m²), `ρ` the density and `ε̇` the strain rate. The count
/// is the subject's volume divided by `s³`. That is the whole of it: **a faster load makes smaller
/// pieces**, cubically, which is why a rifle round comminutes a bone that a pistol round wedges.
///
/// # The energy ceiling, and why both arguments are read
///
/// Grady's size depends on the strain rate alone, so on its own it would let a gentle blow at a high
/// rate produce a hundred fragments it has no energy to create. Creating `n` fragments of size `s`
/// makes roughly `6 n s²` of new surface, and that surface costs `G_c` per unit area — so the energy
/// actually delivered is a **hard ceiling** on the count, and this returns the smaller of the two.
/// Both bounds are physics; neither is a fudge factor.
///
/// # What this does not do
///
/// It returns a **count**, not a size distribution. The *spread* of fragment volumes comes from
/// [`CutSettings::plane_jitter`] and [`size_spread`](CutSettings::size_spread), and
/// `audit::the_shape_dials_widen_the_fragment_size_spread` is what measures it against Mott's
/// qualitative shape — many small, few large (`doi:10.1098/rspa.1947.0042`). So the two halves of
/// "how does it break" are separate and each is measured where it lives.
///
/// # Tissue
///
/// The toughness comes from `tissue`, and the two bone classes differ by an order of magnitude:
/// cortical bone splits, trabecular bone **compacts**. So a trabecular subject is clamped to at most
/// [`TRABECULAR_MAX_PIECES`] pieces no matter how hard it is hit, because a crushed cancellous bone
/// does not produce shards — it produces a shorter bone.
///
/// Non-finite or non-positive inputs return [`FractureSettings::min_pieces`]: a blow nobody described
/// is not a reason to invent a fragment count.
pub fn grady_mott_target(
    volume_m3: f32,
    energy_j: f32,
    strain_rate: f32,
    tissue: TissueClass,
    s: &FractureSettings,
) -> usize {
    let lo = s.min_pieces.max(1) as usize;
    let hi = s.max_pieces.max(s.min_pieces).max(1) as usize;
    let toughness = match tissue {
        TissueClass::Cortical => s.toughness_cortical,
        TissueClass::Trabecular => s.toughness_trabecular,
        TissueClass::Soft => s.toughness_soft,
    };
    let ok = [volume_m3, energy_j, strain_rate, toughness, s.density_kg_m3]
        .iter()
        .all(|v| v.is_finite() && *v > 0.0);
    if !ok {
        return lo;
    }

    // Grady's characteristic fragment size.
    let size = (24.0 * toughness / (s.density_kg_m3 * strain_rate * strain_rate)).cbrt();
    if !size.is_finite() || size <= 0.0 {
        return lo;
    }
    let by_rate = volume_m3 / (size * size * size);

    // The energy ceiling: `6 n s²` of new surface at `G_c` per unit area.
    let per_fragment = 6.0 * size * size * toughness;
    let by_energy = if per_fragment > 0.0 { energy_j / per_fragment } else { f32::INFINITY };

    let n = by_rate.min(by_energy);
    if !n.is_finite() {
        return lo;
    }
    let ceiling = match tissue {
        // Crush, not shatter: trabecular bone tolerates ~30 % strain and compacts.
        TissueClass::Trabecular => hi.min(TRABECULAR_MAX_PIECES),
        _ => hi,
    };
    (n.round().max(0.0) as usize).clamp(lo.min(ceiling), ceiling)
}

/// **Ceiling on the fragment count for trabecular bone.**
///
/// Cancellous bone tolerates roughly 30 % strain against cortical bone's 2 %, so it *compacts* under
/// a blow rather than splitting: the failure reads as a shorter, denser bone, not as shards. Three is
/// the count at which that still reads as a break rather than as a shatter.
pub const TRABECULAR_MAX_PIECES: usize = 3;

/// **The carnage dials** — how a wound bleeds, sprays, stains and hits, with nothing in it that
/// decides *when* any of that happens.
///
/// Separate from [`FractureSettings`] because the two are authored by different people at different
/// times: fracture dials are tuned once against a subject's silhouette and then left alone, while
/// these are tuned against how a fight feels and are the ones that move. Sharing one struct would
/// mean re-blessing a bake's look every time the blood was retuned.
///
/// **Every field carries an explicit `serde` default, and every default is the function
/// [`Default`](CarnageSettings::default) itself calls** — so the pair cannot drift, which is the trap
/// [`FractureSettings::ejecta_soften`]'s own writeup records. The struct stays
/// `deny_unknown_fields`: a *missing* dial takes the shipped value, a *misspelled* one is still an
/// error, and that combination is what makes the default safe rather than a weakening.
///
/// **Ticks, not seconds, wherever time appears.** Nothing in the deterministic half of this crate
/// reads a clock; a caller supplies its own fixed-tick counter. The shipped tick counts are derived
/// for a 60 Hz fixed tick — see `bloodstain::settings` for what to re-derive if yours is not.
///
/// # The blood dials are not here, and that is the `0.2.0` break
///
/// Everything about blood as a *material* — droplet counts, spray speeds, stain radii, the pulse
/// train, pool spreading, clotting, drying — lives in [`blood`](Self::blood), which is
/// `bloodstain::BloodSettings`. **One dial, one home:** a `droplets_per_m2` on this struct *and* on
/// the leaf would be two values for one quantity, and they would disagree the first time either was
/// authored. What stays here is what the leaf cannot own: hit feel, camera shake, particle capacity
/// and the render budgets, all of which need an engine to mean anything.
#[derive(Resource, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct CarnageSettings {
    /// **Blood as a material.** Every dial the spatter, stain, pool, bleed and drying models read.
    ///
    /// Nested rather than flattened, deliberately: an authored file says `blood: (droplets_per_m2:
    /// 2400.0, …)`, which makes it obvious at a glance which dials belong to the leaf and are usable
    /// without Bevy. Flattening would hide the boundary the whole extraction exists to draw.
    #[cfg_attr(feature = "serde", serde(default))]
    pub blood: BloodSettings,
    /// Trauma one severity-1 wound is worth, in `[0, 1]`, **before** the caller's own scaling.
    #[cfg_attr(feature = "serde", serde(default = "default_trauma_per_wound"))]
    pub trauma_per_wound: f32,
    /// Hit-stop for a severity-1 wound, seconds — converted to whole ticks against the caller's rate.
    ///
    /// `0.055` is about three ticks at 60 Hz, which is the "just a few frames" the game-feel survey
    /// reports as the practised value.
    #[cfg_attr(feature = "serde", serde(default = "default_hitstop_seconds"))]
    pub hitstop_seconds: f32,
    /// Shake displacement at trauma 1, metres. The caller applies it; this crate never does.
    #[cfg_attr(feature = "serde", serde(default = "default_shake_amplitude"))]
    pub shake_amplitude: f32,
    /// Period of one shake cycle, ticks. `11` is about 0.18 s at 60 Hz.
    ///
    /// Must be non-zero: it is a modulus — see [`CarnageSettings::validate`].
    #[cfg_attr(feature = "serde", serde(default = "default_shake_ticks"))]
    pub shake_ticks: u32,
    /// Particles one effect asset may hold.
    ///
    /// A dial rather than a constant because a particle effect's capacity is fixed when the asset is
    /// built and cannot be raised afterwards, so the ceiling has to be authored with the rest.
    #[cfg_attr(feature = "serde", serde(default = "default_effect_capacity"))]
    pub effect_capacity: u32,
    /// **Hard ceiling on live blood ribbons, first-come-first-served.**
    ///
    /// Past this, `attach_ribbons` refuses to start a new one and never evicts a running one — a
    /// ribbon that vanishes mid-flight reads as a glitch, while a chunk with no ribbon reads as a
    /// chunk. Each instance is its own draw call *and* its own sort dispatch, so this is a real
    /// ceiling; see [`crate::gib_ribbon`]. 24 × 64 particles is 1,536, comfortably inside one slab.
    #[cfg_attr(feature = "serde", serde(default = "default_max_ribbons"))]
    pub max_ribbons: u32,
    /// **The palette a `Stylised` tier substitutes for blood**, linear sRGB.
    ///
    /// A spark yellow by default. This is what makes reduction a *substitution*: at
    /// [`GoreTier::Stylised`] the same emitter fires at the same tick with the same magnitude and
    /// paints with this instead. Gears of War 4 shipped exactly this trade and kept its hit
    /// confirmation; Vermintide 2's gore-off deleted the channel and made the game harder to read.
    #[cfg_attr(feature = "serde", serde(default = "default_substitute_srgb"))]
    pub substitute_srgb: [f32; 3],
    /// **Fraction of a second that may be spent frozen**, across every wound in it.
    ///
    /// `0.1` is six ticks at 60 Hz. Read by [`coalesce_hitstop`], which takes the *maximum* pending
    /// stop rather than the sum — hit stop spent everywhere reads as impact nowhere
    /// (`doi:10.1109/tg.2021.3072241` §III-C).
    #[cfg_attr(feature = "serde", serde(default = "default_hitstop_budget_per_second"))]
    pub hitstop_budget_per_second: f32,
    /// Ticks a stain decal lives before it is despawned, before
    /// [`GorePolicy::persistence_scale`]. `3600` is a minute at 60 Hz.
    ///
    /// **Per class, and that is the shipped decomposition rather than a preference** — Killing Floor
    /// 2 exposes exactly these three lifetimes separately, because a stain, a slick and a chunk have
    /// different costs and different meanings.
    #[cfg_attr(feature = "serde", serde(default = "default_stain_lifetime_ticks"))]
    pub stain_lifetime_ticks: u32,
    /// Ticks a pool decal lives. Longer than a stain: a slick is the evidence a body was here.
    #[cfg_attr(feature = "serde", serde(default = "default_pool_lifetime_ticks"))]
    pub pool_lifetime_ticks: u32,
    /// Ticks a detached chunk lives. Shortest of the three: a chunk is an entity with a collider.
    #[cfg_attr(feature = "serde", serde(default = "default_chunk_lifetime_ticks"))]
    pub chunk_lifetime_ticks: u32,
}

/// The shipped [`CarnageSettings`] values, one function per dial.
///
/// **These are the single source, and [`CarnageSettings::default`] calls them.** The alternative —
/// literals in `Default` and a parallel set of `serde` default functions — is exactly the drift that
/// `the_serde_default_matches_the_shipped_one` had to be written to catch on `FractureSettings`. Here
/// the two cannot disagree, because there is only one of them.
mod shipped {
    // At severity 1, before the caller's own scaling.
    pub(super) fn trauma_per_wound() -> f32 {
        0.55
    }
    // ≈3 fixed ticks at 60 Hz — the survey's "just a few frames".
    pub(super) fn hitstop_seconds() -> f32 {
        0.055
    }
    // Metres at trauma 1.
    pub(super) fn shake_amplitude() -> f32 {
        0.045
    }
    // ≈0.18 s at 60 Hz.
    pub(super) fn shake_ticks() -> u32 {
        11
    }
    // A particle asset's capacity is immutable once built, so it is a dial.
    pub(super) fn effect_capacity() -> u32 {
        4096
    }
    // One draw call and one sort dispatch each; 24 × 64 particles fits one slab.
    pub(super) fn max_ribbons() -> u32 {
        24
    }
    // A spark yellow: reads as a hit without reading as blood.
    pub(super) fn substitute_srgb() -> [f32; 3] {
        [1.0, 0.78, 0.25]
    }
    // 0.1 s — six ticks at 60 Hz, across every wound in that second.
    pub(super) fn hitstop_budget_per_second() -> f32 {
        0.1
    }
    // One minute at 60 Hz.
    pub(super) fn stain_lifetime_ticks() -> u32 {
        3600
    }
    // Two minutes: a slick is the evidence a body was here.
    pub(super) fn pool_lifetime_ticks() -> u32 {
        7200
    }
    // Twenty seconds: a chunk is an entity with a collider.
    pub(super) fn chunk_lifetime_ticks() -> u32 {
        1200
    }
}

// `serde(default = "path")` needs a free function per field. Each one forwards to `shipped`, which is
// also what `Default` reads — so there is exactly one value per dial in this file.
fn default_trauma_per_wound() -> f32 {
    shipped::trauma_per_wound()
}
fn default_hitstop_seconds() -> f32 {
    shipped::hitstop_seconds()
}
fn default_shake_amplitude() -> f32 {
    shipped::shake_amplitude()
}
fn default_shake_ticks() -> u32 {
    shipped::shake_ticks()
}
fn default_effect_capacity() -> u32 {
    shipped::effect_capacity()
}
fn default_max_ribbons() -> u32 {
    shipped::max_ribbons()
}
fn default_substitute_srgb() -> [f32; 3] {
    shipped::substitute_srgb()
}
fn default_hitstop_budget_per_second() -> f32 {
    shipped::hitstop_budget_per_second()
}
fn default_stain_lifetime_ticks() -> u32 {
    shipped::stain_lifetime_ticks()
}
fn default_pool_lifetime_ticks() -> u32 {
    shipped::pool_lifetime_ticks()
}
fn default_chunk_lifetime_ticks() -> u32 {
    shipped::chunk_lifetime_ticks()
}

impl Default for CarnageSettings {
    fn default() -> Self {
        CarnageSettings {
            blood: BloodSettings::default(),
            trauma_per_wound: shipped::trauma_per_wound(),
            hitstop_seconds: shipped::hitstop_seconds(),
            shake_amplitude: shipped::shake_amplitude(),
            shake_ticks: shipped::shake_ticks(),
            effect_capacity: shipped::effect_capacity(),
            max_ribbons: shipped::max_ribbons(),
            substitute_srgb: shipped::substitute_srgb(),
            hitstop_budget_per_second: shipped::hitstop_budget_per_second(),
            stain_lifetime_ticks: shipped::stain_lifetime_ticks(),
            pool_lifetime_ticks: shipped::pool_lifetime_ticks(),
            chunk_lifetime_ticks: shipped::chunk_lifetime_ticks(),
        }
    }
}

impl CarnageSettings {
    /// Reject a settings block that cannot produce a sane schedule.
    ///
    /// **One of these is a real crash, not a hypothetical**, which is the same standard
    /// [`FractureSettings::validate`] is held to: `shake_ticks` is a modulus, so zero panics the
    /// first time a camera shakes. The rest catch dials that silently switch a feature off through a
    /// ceiling.
    ///
    /// **The blood dials are validated by their owner.** This forwards to
    /// [`BloodSettings::validate`] rather than re-checking them, because a second copy of
    /// "`clot_ticks` must not be below `spurt_ticks`" is a second place for that rule to be wrong.
    ///
    /// Call it at load. Failing loudly at the door is the one path; clamping a bad pair here would be
    /// a second, quieter one.
    pub fn validate(&self) -> Result<(), String> {
        self.blood.validate()?;
        if self.shake_ticks == 0 {
            return Err(
                "carnage: shake_ticks is 0 — it is the modulus of the shake phase, so the first \
                 shake would panic on a division by zero. Use 1 or more."
                    .to_string(),
            );
        }
        if !(0.0..=1.0).contains(&self.trauma_per_wound) {
            return Err(format!(
                "carnage: trauma_per_wound is {} — trauma is in [0, 1] and the caller accumulates \
                 it, so a value outside that is not a stronger hit, it is a broken one.",
                self.trauma_per_wound
            ));
        }
        if self.effect_capacity == 0 {
            return Err(
                "carnage: effect_capacity is 0 — an effect that can hold no particles renders \
                 nothing, which is a settings bug rather than a look."
                    .to_string(),
            );
        }
        if self.max_ribbons == 0 {
            return Err(
                "carnage: max_ribbons is 0 — that disables blood ribbons entirely, which is a \
                 content decision made by not inserting `CarnageVfxPlugin`, not by a dial that \
                 leaves the systems running and refusing every one."
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// **A wound, in world space, announced.** The crate's one message.
///
/// [`Wound`] is subject-local, because that is the space a bake lives in. Anything that *renders* a
/// wound needs it in world space, so the conversion is a method rather than something every caller
/// re-derives — and [`Wound::to_world`] is where the one mistake that conversion invites is
/// prevented.
///
/// **Declared in core, not behind `vfx`.** A consuming game writes this from its own gore drain on
/// `FixedUpdate` whether or not anything is rendering; a headless simulation reads it to place blood
/// pools that feed further simulation. Only the particle *reader* is cosmetic.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct Wounded {
    /// Where it is, world space.
    pub at: Vec3,
    /// Which way it faces, world space, unit.
    pub normal: Vec3,
    /// How much surface came open, world units squared.
    pub area: f32,
    /// How badly, in `[0, 1]`.
    pub severity: f32,
    /// Which of the two things happened.
    pub kind: WoundKind,
    /// **Which forensic pattern this wound throws** — impact spatter, an arterial arc, a cast-off
    /// line, expirated mist, a drip trail or a transfer smear.
    ///
    /// Distinct from [`kind`](Self::kind), which says what *opened* the wound. A severance can bleed
    /// arterially or not; a bullet channel can spurt or seep. The two answer different questions and
    /// collapsing them would make the distinction the whole pattern layer exists to draw invisible.
    pub class: PatternClass,
}

impl Wound {
    /// This wound in world space, through a subject's transform.
    ///
    /// **A normal is not a point, and this method exists to stop one being treated as the other.**
    /// `transform_point` applies the translation, which is correct for `at` and wrong for `normal` —
    /// a direction put through it comes back pointing from the world origin toward the subject, so
    /// blood would leave every wound in roughly the same direction and the bug would look like "the
    /// spray angle is off" rather than like a category error. The direction goes through the affine's
    /// linear part alone and is renormalised, because a non-uniform scale does not preserve length.
    ///
    /// `area` is carried across unchanged: scaling it correctly would need the scale factors in the
    /// wound's own plane, and a subject that is uniformly scaled is the only case where a single
    /// number would be right. A caller with a scaled subject scales `area` itself, knowingly.
    ///
    /// **`class` is a parameter and not a default, deliberately.** A default here would make the
    /// arterial/impact distinction invisible at the call site, which is precisely the distinction
    /// [`PatternClass`] exists to make — so every caller states it, and a caller that has not thought
    /// about it is made to.
    pub fn to_world(self, xf: &GlobalTransform, class: PatternClass) -> Wounded {
        Wounded {
            at: xf.transform_point(self.at),
            normal: (xf.affine().matrix3 * self.normal).normalize_or_zero(),
            area: self.area,
            severity: self.severity,
            kind: self.kind,
            class,
        }
    }
}

/// **A bleeding thing, as an entity.** The ECS half of [`Bleed`].
///
/// A newtype rather than a derive on `Bleed` itself, because `Bleed` lives in `bloodstain`, which has
/// no engine in it to derive a `Component` from. This is the same facade-newtype shape this
/// workspace's game reaches `bevy_stigmergy` and `bevy_light_grid` through, and it was chosen over
/// the two alternatives on purpose: implementing a foreign trait for a foreign type is forbidden by
/// the orphan rule, and giving the leaf an optional `bevy` dependency would have put an engine inside
/// the crate whose whole point is not having one.
///
/// `Deref`/`DerefMut` to the value, so `bleeding.age(tick)` and `bleeding.area` read exactly as they
/// did when `Bleed` was the component.
#[derive(Component, Clone, Copy, Debug, PartialEq, Deref, DerefMut)]
pub struct Bleeding(pub Bleed);

impl Bleeding {
    /// Open a bleed at `tick` for a wound — [`Bleed::new`], wrapped.
    ///
    /// The wound crosses [`v3`](crate::v3)'s boundary on the way in, because `Bleed`'s seed is
    /// [`bloodstain::wound_seed`] of the leaf's own `Wound`. Five field copies, once per wound.
    pub fn new(opened_at: u32, wound: &Wound) -> Self {
        Bleeding(Bleed::new(opened_at, &crate::v3::wound(wound)))
    }
}

/// The set [`bake_fractures`] runs in. **Gate and order against this, not against the system.**
///
/// The plugin puts the bake on `Update` and nothing else: it is an asset bake gated on streaming, so
/// it has no business on a fixed timestep. What it must *not* do is decide when your game is running —
/// so the run condition is yours:
///
/// ```ignore
/// app.add_plugins(bevy_carnage::CarnagePlugin)
///     .configure_sets(Update, CarnageSystems.run_if(in_state(MyState::Playing)))
///     .add_systems(Update, tag_my_weapon.before(CarnageSystems));
/// ```
///
/// Anything that inserts [`DetachedPart`] onto a streamed-in subtree must run `.before` this set, or
/// the bake reads the scene a frame before the marker lands and swallows the part into the body.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CarnageSystems;

/// Registers the fracture cache, its settings, the one-shot bake, and the [`Wounded`] message.
///
/// [`FractureSettings`] is `init_resource`d, which does nothing when the resource is already present —
/// so a caller that loads the dials from a config file inserts them *before* adding this plugin and its
/// values win. There is no merge and no partial default: one owner, one value.
///
/// **Still no run condition, and still nothing on `FixedUpdate`.** [`Wounded`] is registered rather
/// than written here: the crate does not decide when a wound happens, it only gives the caller a
/// channel to say one did. [`CarnageSettings`] is deliberately *not* `init_resource`d, because the
/// deterministic half takes it as a function argument and only the optional `vfx` half needs it as a
/// resource — inserting it here would put a dial in every headless app that has no use for one.
pub struct CarnagePlugin;

impl Plugin for CarnagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FractureCache>()
            .init_resource::<FractureSettings>()
            // **The framework surface, `init_resource`d rather than inserted.** A caller that owns
            // the tone and the budgets inserts them *before* this plugin and its values win; there
            // is no merge and no partial default, which is the same rule `FractureSettings` keeps.
            //
            // `FlashGate` and `DecalBudget` are state rather than policy, so they have no authored
            // form at all — but they must exist, because a system taking `ResMut<FlashGate>` PANICS
            // when the resource is absent rather than skipping.
            .init_resource::<GorePolicy>()
            .init_resource::<FlashGate>()
            .init_resource::<DecalBudget>()
            .add_message::<Wounded>()
            // **Chained, not merely grouped.** `bake_fractures` requests the finest frontier as its
            // last act, and `materialise_fragments` is what turns a request into a mesh — a bake and
            // a request in the same frame must materialise in that frame, or the first frame after a
            // death spawns nothing.
            .add_systems(
                Update,
                (bake_fractures, materialise_fragments).chain().in_set(CarnageSystems),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A serde default and a `Default` impl that disagree are worse than either alone.**
    ///
    /// [`FractureSettings`] is `deny_unknown_fields` with every field required, so a new dial needs an
    /// explicit `serde(default = …)` or it breaks every authored file that enumerated the others — at
    /// load time, which no compile catches. The trap that replaces it: the two defaults drift, and a
    /// config that *omits* the dial then renders differently from one that never had it, which is a
    /// difference nobody would think to look for.
    ///
    /// Pinned per field that carries an explicit serde default. There is one today.
    #[test]
    fn the_serde_default_matches_the_shipped_one() {
        assert_eq!(
            default_ejecta_soften(),
            FractureSettings::default().ejecta_soften,
            "serde would hand an authored file that omits `ejecta_soften` a different value than \
             `FractureSettings::default()` uses, so the same config would look different depending \
             on whether the field was written down"
        );
    }

    /// **An authored file written before this dial existed must still load.**
    ///
    /// The regression test for the defect that produced `default_ejecta_soften`: this struct is
    /// `deny_unknown_fields` with every field required, so a new dial refuses every config that listed
    /// the others — at load time, which no build catches. The block below is *exactly* the eleven
    /// fields the consuming game's `config.ron` enumerated before AG-024, copied rather than
    /// paraphrased, because a paraphrase would stop being the thing that broke.
    ///
    /// The second half is the pairing that makes the default safe rather than a weakening: a
    /// **misspelled** field is still refused. Missing takes the shipped value; unknown is an error.
    #[test]
    #[cfg(feature = "serde")]
    fn a_config_written_before_ejecta_soften_still_loads() {
        let authored = r#"(
            pieces_base: 14,
            ref_extent: 0.5,
            min_pieces: 6,
            max_pieces: 40,
            min_fraction: 0.18,
            max_depth: 12,
            plane_jitter: 0.35,
            size_spread: 0.5,
            weak_axis: 0.75,
            cap_relief: 0.30,
            soften: 0.5,
        )"#;
        let s: FractureSettings = ron::from_str(authored)
            .expect("a config listing the eleven pre-AG-024 dials must still deserialize");
        assert_eq!(
            s.ejecta_soften,
            default_ejecta_soften(),
            "the omitted dial must take the shipped default"
        );
        assert_eq!(s.pieces_base, 14, "the authored fields must survive");
        assert_eq!(s.soften, 0.5, "the authored fields must survive");
        s.validate().expect("an authored config plus the default dial must validate");

        let typo = authored.replace("soften: 0.5,", "soften: 0.5, sofen: 0.9,");
        assert!(
            ron::from_str::<FractureSettings>(&typo).is_err(),
            "a misspelled field must still be refused — `deny_unknown_fields` is what makes the \
             serde default safe, and a default that also swallowed typos would be a fallback"
        );
    }

    /// The shipped dials must pass the crate's own door check — otherwise every caller that starts
    /// from `default()` and validates (which is what the crate tells them to do) is refused.
    #[test]
    fn the_shipped_settings_validate() {
        FractureSettings::default().validate().expect("the shipped FractureSettings must validate");
        CarnageSettings::default().validate().expect("the shipped CarnageSettings must validate");
    }

    /// **[`CarnageSettings`]' serde defaults cannot drift from its `Default`, by construction** —
    /// both read `shipped`. This test is what proves the construction actually holds, field by
    /// field, so a later hand-written literal in either place fails here rather than in a config
    /// file six months on.
    ///
    /// **The blood dials are not listed, because they are not this struct's** — they live in
    /// `bloodstain::BloodSettings`, which pins the same construction with its own test over its own
    /// `shipped` module. Listing them here would be the second home this whole extraction removed.
    #[test]
    fn every_carnage_serde_default_is_the_shipped_value() {
        let d = CarnageSettings::default();
        let pairs_f32: &[(&str, f32, f32)] = &[
            ("trauma_per_wound", default_trauma_per_wound(), d.trauma_per_wound),
            ("hitstop_seconds", default_hitstop_seconds(), d.hitstop_seconds),
            ("shake_amplitude", default_shake_amplitude(), d.shake_amplitude),
        ];
        for (name, serde_value, shipped_value) in pairs_f32 {
            assert_eq!(
                serde_value.to_bits(),
                shipped_value.to_bits(),
                "{name}: the serde default and the shipped default disagree, so a config that \
                 omits the dial behaves differently from one that never had it"
            );
        }
        let pairs_u32: &[(&str, u32, u32)] = &[
            ("shake_ticks", default_shake_ticks(), d.shake_ticks),
            ("effect_capacity", default_effect_capacity(), d.effect_capacity),
            ("max_ribbons", default_max_ribbons(), d.max_ribbons),
        ];
        for (name, serde_value, shipped_value) in pairs_u32 {
            assert_eq!(serde_value, shipped_value, "{name}: serde and shipped defaults disagree");
        }
        // And the nested block takes the leaf's shipped values, not a second set written here.
        assert_eq!(
            d.blood,
            BloodSettings::default(),
            "the blood block must be exactly the leaf's shipped dials"
        );
    }

    /// **An empty authored block must load as the shipped dials, and a typo must still be refused.**
    ///
    /// The whole point of per-field defaults on a `deny_unknown_fields` struct: adding a dial later
    /// cannot refuse a config that enumerated the others, while a misspelling is still an error at
    /// the door rather than a silently ignored line.
    #[test]
    #[cfg(feature = "serde")]
    fn an_empty_carnage_config_loads_as_the_shipped_dials() {
        let s: CarnageSettings =
            ron::from_str("()").expect("a config that omits every carnage dial must deserialize");
        assert_eq!(s, CarnageSettings::default(), "omitting every dial must give the shipped block");

        // The blood dials are authored under their own block now, which is the `0.2.0` shape: one
        // dial, one owner, and an authored file that shows the boundary.
        let partial: CarnageSettings = ron::from_str("(blood: (spurt_bpm: 120.0))")
            .expect("a config naming one dial must take the shipped values for the rest");
        assert_eq!(partial.blood.spurt_bpm, 120.0, "the authored dial must survive");
        assert_eq!(
            partial.blood.clot_ticks,
            BloodSettings::default().clot_ticks,
            "an unauthored dial must take the shipped value"
        );
        assert_eq!(
            partial.shake_ticks,
            default_shake_ticks(),
            "and an unauthored dial outside the blood block too"
        );

        assert!(
            ron::from_str::<CarnageSettings>("(blood: (spurt_bmp: 120.0))").is_err(),
            "a misspelled dial must be refused — `deny_unknown_fields` is what keeps the per-field \
             defaults from becoming a fallback that swallows typos"
        );
    }

    /// **Each door check must actually fire.** A `validate` nobody has seen fail is a comment.
    #[test]
    fn the_carnage_door_refuses_what_would_panic_or_invert() {
        let bad = |f: fn(&mut CarnageSettings)| {
            let mut s = CarnageSettings::default();
            f(&mut s);
            s.validate().expect_err("this block must be refused")
        };
        assert!(bad(|s| s.shake_ticks = 0).contains("shake_ticks"));
        assert!(bad(|s| s.trauma_per_wound = 1.5).contains("trauma_per_wound"));
        assert!(bad(|s| s.effect_capacity = 0).contains("effect_capacity"));
        assert!(bad(|s| s.max_ribbons = 0).contains("max_ribbons"));
        // **And the blood dials are still refused, by their owner.** `validate` forwards rather than
        // re-checking, so this is what proves the forward is wired — a delegation nobody has seen
        // fire is the same comment a `validate` nobody has seen fail is.
        assert!(bad(|s| s.blood.spurt_bpm = 0.0).contains("spurt_bpm"));
        assert!(bad(|s| s.blood.clot_ticks = 10).contains("clot_ticks"));
        assert!(bad(|s| s.blood.droplet_size_max = 0.0).contains("droplet_size"));
        assert!(bad(|s| s.blood.stain_radius_min = 0.0).contains("stain_radius"));
        assert!(bad(|s| s.blood.spatter_cone_deg = 200.0).contains("spatter_cone_deg"));
    }

    /// **A normal is not a point.** The bug this test exists for: putting a direction through
    /// `transform_point` adds the translation, so every wound on a subject standing away from the
    /// origin sprays in roughly the same direction — which looks like a tuning problem and is a
    /// category error. Checked against a transform with a translation large enough that the two
    /// answers cannot be confused.
    #[test]
    fn to_world_moves_the_point_and_only_rotates_the_normal() {
        let w = Wound {
            at: Vec3::new(0.1, 0.2, 0.3),
            normal: Vec3::X,
            area: 0.004,
            severity: 0.75,
            kind: WoundKind::Channel,
        };

        let shifted = GlobalTransform::from(Transform::from_xyz(100.0, 50.0, -20.0));
        let out = w.to_world(&shifted, PatternClass::Impact);
        assert_eq!(out.at, Vec3::new(100.1, 50.2, -19.7), "the point must be translated");
        assert_eq!(out.normal, Vec3::X, "a pure translation must not turn the normal at all");
        assert_eq!(out.area, w.area, "area is carried across unchanged");
        assert_eq!(out.severity, w.severity);
        assert_eq!(out.kind, w.kind);

        let turned = GlobalTransform::from(
            Transform::from_xyz(3.0, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        );
        let out = turned_normal(&w, &turned);
        assert!(
            (out - Vec3::Y).length() < 1.0e-5,
            "a quarter turn about Z must take +X to +Y, got {out:?}"
        );

        // A non-uniform scale does not preserve length, so the normal must come back renormalised.
        let scaled = GlobalTransform::from(
            Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::new(4.0, 0.5, 2.0)),
        );
        let out = turned_normal(&w, &scaled);
        assert!((out.length() - 1.0).abs() < 1.0e-5, "normal length {} is not unit", out.length());
    }

    fn turned_normal(w: &Wound, xf: &GlobalTransform) -> Vec3 {
        w.to_world(xf, PatternClass::Impact).normal
    }
}
