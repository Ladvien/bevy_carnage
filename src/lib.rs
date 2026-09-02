#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod audit;
mod bake;
mod bleed;
mod bond;
mod bore;
#[cfg(feature = "vfx")]
mod decal;
mod feel;
mod mesh;
mod order;
mod pool;
mod proxy;
mod severance;
mod soup;
mod spatter;
mod tree;
#[cfg(feature = "vfx")]
mod vfx;
mod wound;

pub use audit::{SolidAudit, SurfaceReport, audit_cell, audit_proxies, audit_proxy, audit_render};
pub use bake::{
    DetachedChunk, DetachedPart, EjectaChunk, Fragment, FractureBores, FractureCache, FractureProxy,
    FractureSubject, bake_fractures, materialise_fragments,
};
pub use bleed::{Bleed, clotted, flow, pulse_period, pulse_wound, pulses_on};
pub use bond::{Bond, BondGraph, BondId, BondSet};
pub use bore::Bore;
#[cfg(feature = "vfx")]
pub use decal::{
    PoolDecal, SPLAT_VARIANTS, SplatTextures, build_splats, spawn_pool, spawn_stain, splat_image,
    update_pool_decals,
};
pub use feel::{hitstop_ticks, shake_offset, trauma_for};
pub use mesh::{Ejecta, Fracture, FragmentGeometry, FragmentSolid, fracture_mesh};
pub use pool::{Pool, absorb, spread_pools};
pub use proxy::ProxyCell;
pub use severance::{Reach, capsule, directional, radial, spread, swept_triangle};
pub use soup::hash_f32;
pub use spatter::{
    BACK_SPATTER_SPEED, BLOOD_DENSITY, BLOOD_SURFACE_TENSION, Droplet, FORWARD_SPATTER_SPEED, Stain,
    droplet, droplet_count, droplets, landing, stain_radius, stains, wound_seed,
};
pub use tree::{FragmentId, FragmentTree, TreeNode};
#[cfg(feature = "vfx")]
pub use vfx::{
    BleedingChunk, CarnageEffects, CarnageVfxPlugin, CarnageVfxSystems, EffectFade, EffectTtl,
    RibbonInstance, arterial_spurt, gib_ribbon, mist_puff, spatter_burst, wound_seep,
};
pub use wound::{
    CapFace, Wound, WoundKind, cap_faces, largest_cap, wound_from_ejecta, wound_of_channel,
    wounds_from_bonds, wounds_from_reach,
};

use bevy::prelude::*;

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
}

/// The shipped [`FractureSettings::ejecta_soften`], for serde to reach when an authored file predates
/// the field. **Must agree with [`FractureSettings::default`]**, or a file that omits the dial gets a
/// different look than one that never had it — pinned by `the_serde_default_matches_the_shipped_one`.
fn default_ejecta_soften() -> f32 {
    0.55
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
        }
    }
}

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
/// for a 60 Hz fixed tick — see the module docs of [`bleed`](crate::bleed) for what to re-derive if
/// yours is not.
#[derive(Resource, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct CarnageSettings {
    /// Droplets a wound throws per square metre of wound area, at severity 1.
    ///
    /// **Count scales with area, not per hit**, because a wound is a surface and the amount of blood
    /// that leaves it is a property of how much of it is open — a graze and a bisection are the same
    /// event with two areas, and one dial covers both.
    #[cfg_attr(feature = "serde", serde(default = "default_droplets_per_m2"))]
    pub droplets_per_m2: f32,
    /// Hard ceiling on one wound's droplet count, so a huge cut cannot exceed
    /// [`effect_capacity`](Self::effect_capacity) in a single burst.
    #[cfg_attr(feature = "serde", serde(default = "default_max_droplets_per_wound"))]
    pub max_droplets_per_wound: u32,
    /// Scales the measured 8…40 m/s spatter span. **`1.0` is the paper's own numbers, and it is a
    /// physical measurement rather than a look.**
    ///
    /// Ship it at 1.0 because [`FORWARD_SPATTER_SPEED`](crate::FORWARD_SPATTER_SPEED) and
    /// [`BACK_SPATTER_SPEED`](crate::BACK_SPATTER_SPEED) are measurements, and a default that quietly
    /// divided them would make the constants lie about what they are. Scaling them is a game-feel
    /// decision, and this crate does not take game-feel decisions on a caller's behalf — the same
    /// division [`feel`](crate::feel) enforces by returning numbers instead of applying them.
    ///
    /// **Expect to lower it, and here is the arithmetic.** At 1.0 a droplet leaving straight up at
    /// 40 m/s under the shipped 18 m/s² gravity rises `40² / (2·18) ≈ 44` metres. That is correct for
    /// a real gunshot and absurd on a 1.8 m subject: the spray leaves frame and the stains land far
    /// outside any floor. Both examples in this crate set **0.25**, which puts the throw at roughly
    /// 1–3 metres — measured against the demo subject, and the reason they set it rather than the
    /// default being changed.
    #[cfg_attr(feature = "serde", serde(default = "default_spatter_speed_scale"))]
    pub spatter_speed_scale: f32,
    /// Half-angle of the forward spray cone, degrees, about the wound normal.
    #[cfg_attr(feature = "serde", serde(default = "default_spatter_cone_deg"))]
    pub spatter_cone_deg: f32,
    /// Smallest droplet diameter, metres — the indivisible droplet end of the cluster span.
    #[cfg_attr(feature = "serde", serde(default = "default_droplet_size_min"))]
    pub droplet_size_min: f32,
    /// Largest droplet diameter, metres — the many-droplet-cluster end of the span.
    #[cfg_attr(feature = "serde", serde(default = "default_droplet_size_max"))]
    pub droplet_size_max: f32,
    /// Downward acceleration used to fly a droplet to its landing point, m/s².
    ///
    /// **Not 9.81, and deliberately.** It matches the examples' own integrator, because blood and
    /// gibs falling at different rates in one scene reads as blood floating. A game sets this to
    /// whatever its own physics uses.
    #[cfg_attr(feature = "serde", serde(default = "default_gravity"))]
    pub gravity: f32,
    /// Linear drag on a droplet, 1/s — the game-scale stand-in for the two-phase air entrainment the
    /// spatter paper models properly.
    #[cfg_attr(feature = "serde", serde(default = "default_drag"))]
    pub drag: f32,
    /// Heartbeat rate driving the pulse train, beats per minute.
    #[cfg_attr(feature = "serde", serde(default = "default_spurt_bpm"))]
    pub spurt_bpm: f32,
    /// Ticks of full-flow spurting before the taper starts. `210` is 3.5 s at 60 Hz.
    #[cfg_attr(feature = "serde", serde(default = "default_spurt_ticks"))]
    pub spurt_ticks: u32,
    /// Ticks from opening to a clot, where flow reaches exactly zero. `360` is 6.0 s at 60 Hz.
    ///
    /// Must be at least [`spurt_ticks`](Self::spurt_ticks) — see [`CarnageSettings::validate`].
    #[cfg_attr(feature = "serde", serde(default = "default_clot_ticks"))]
    pub clot_ticks: u32,
    /// Smallest stain radius, metres.
    #[cfg_attr(feature = "serde", serde(default = "default_stain_radius_min"))]
    pub stain_radius_min: f32,
    /// Largest stain radius, metres.
    #[cfg_attr(feature = "serde", serde(default = "default_stain_radius_max"))]
    pub stain_radius_max: f32,
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
    /// Stains landing within this distance of a pool join it instead of starting their own, metres.
    #[cfg_attr(feature = "serde", serde(default = "default_pool_merge_radius"))]
    pub pool_merge_radius: f32,
    /// Multiplier from a pool's wetted-area-equivalent radius to its drawn radius.
    ///
    /// Above 1 because blood spreads thinner than the discs that fed it: the area a droplet *wets* on
    /// impact is measured at the moment of impact, and a slick keeps creeping outward after.
    #[cfg_attr(feature = "serde", serde(default = "default_pool_spread"))]
    pub pool_spread: f32,
    /// Fraction of the remaining gap between drawn and target radius a pool closes per tick.
    ///
    /// Must be in `(0, 1]` — see [`CarnageSettings::validate`].
    #[cfg_attr(feature = "serde", serde(default = "default_pool_spread_rate"))]
    pub pool_spread_rate: f32,
    /// Hard ceiling on live pools. Past it a stain joins a nearby pool if it can and is dropped if it
    /// cannot — dropping is correct at the ceiling of a system whose whole job is to accumulate.
    #[cfg_attr(feature = "serde", serde(default = "default_max_pools"))]
    pub max_pools: u32,
}

/// The shipped [`CarnageSettings`] values, one function per dial.
///
/// **These are the single source, and [`CarnageSettings::default`] calls them.** The alternative —
/// literals in `Default` and a parallel set of `serde` default functions — is exactly the drift that
/// `the_serde_default_matches_the_shipped_one` had to be written to catch on `FractureSettings`. Here
/// the two cannot disagree, because there is only one of them.
mod shipped {
    // Count scales with wound area, not per hit.
    pub(super) fn droplets_per_m2() -> f32 {
        2400.0
    }
    // Keeps one burst inside `effect_capacity`.
    pub(super) fn max_droplets_per_wound() -> u32 {
        512
    }
    // Scales the measured 8…40 m/s span.
    pub(super) fn spatter_speed_scale() -> f32 {
        1.0
    }
    // Forward spray half-angle.
    pub(super) fn spatter_cone_deg() -> f32 {
        32.0
    }
    // Metres; the indivisible droplet.
    pub(super) fn droplet_size_min() -> f32 {
        0.000_8
    }
    // Metres; the cluster span's far end.
    pub(super) fn droplet_size_max() -> f32 {
        0.006
    }
    // Matches the examples' own integrator (`examples/common/body.rs`'s `GRAVITY`) so blood and gibs
    // fall in one world; 9.81 would make blood float relative to the chunks.
    pub(super) fn gravity() -> f32 {
        18.0
    }
    // Stands in for the paper's two-phase air entrainment.
    pub(super) fn drag() -> f32 {
        1.6
    }
    // Pulse period is `60 / bpm`.
    pub(super) fn spurt_bpm() -> f32 {
        96.0
    }
    // 3.5 s at 60 Hz.
    pub(super) fn spurt_ticks() -> u32 {
        210
    }
    // 6.0 s at 60 Hz.
    pub(super) fn clot_ticks() -> u32 {
        360
    }
    // Metres.
    pub(super) fn stain_radius_min() -> f32 {
        0.02
    }
    // Metres.
    pub(super) fn stain_radius_max() -> f32 {
        0.12
    }
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
    // Metres. About a hand's width — close enough that two spatter discs read as one wet patch.
    pub(super) fn pool_merge_radius() -> f32 {
        0.10
    }
    // Blood creeps outward after the impact area was measured.
    pub(super) fn pool_spread() -> f32 {
        1.35
    }
    // Fraction of the remaining gap per tick; ≈0.2 s to close half the distance at 60 Hz.
    pub(super) fn pool_spread_rate() -> f32 {
        0.08
    }
    // Live slicks. A forward decal each, so this is a draw-call ceiling like `max_ribbons`.
    pub(super) fn max_pools() -> u32 {
        256
    }
}

// `serde(default = "path")` needs a free function per field. Each one forwards to `shipped`, which is
// also what `Default` reads — so there is exactly one value per dial in this file.
fn default_droplets_per_m2() -> f32 {
    shipped::droplets_per_m2()
}
fn default_max_droplets_per_wound() -> u32 {
    shipped::max_droplets_per_wound()
}
fn default_spatter_speed_scale() -> f32 {
    shipped::spatter_speed_scale()
}
fn default_spatter_cone_deg() -> f32 {
    shipped::spatter_cone_deg()
}
fn default_droplet_size_min() -> f32 {
    shipped::droplet_size_min()
}
fn default_droplet_size_max() -> f32 {
    shipped::droplet_size_max()
}
fn default_gravity() -> f32 {
    shipped::gravity()
}
fn default_drag() -> f32 {
    shipped::drag()
}
fn default_spurt_bpm() -> f32 {
    shipped::spurt_bpm()
}
fn default_spurt_ticks() -> u32 {
    shipped::spurt_ticks()
}
fn default_clot_ticks() -> u32 {
    shipped::clot_ticks()
}
fn default_stain_radius_min() -> f32 {
    shipped::stain_radius_min()
}
fn default_stain_radius_max() -> f32 {
    shipped::stain_radius_max()
}
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
fn default_pool_merge_radius() -> f32 {
    shipped::pool_merge_radius()
}
fn default_pool_spread() -> f32 {
    shipped::pool_spread()
}
fn default_pool_spread_rate() -> f32 {
    shipped::pool_spread_rate()
}
fn default_max_pools() -> u32 {
    shipped::max_pools()
}

impl Default for CarnageSettings {
    fn default() -> Self {
        CarnageSettings {
            droplets_per_m2: shipped::droplets_per_m2(),
            max_droplets_per_wound: shipped::max_droplets_per_wound(),
            spatter_speed_scale: shipped::spatter_speed_scale(),
            spatter_cone_deg: shipped::spatter_cone_deg(),
            droplet_size_min: shipped::droplet_size_min(),
            droplet_size_max: shipped::droplet_size_max(),
            gravity: shipped::gravity(),
            drag: shipped::drag(),
            spurt_bpm: shipped::spurt_bpm(),
            spurt_ticks: shipped::spurt_ticks(),
            clot_ticks: shipped::clot_ticks(),
            stain_radius_min: shipped::stain_radius_min(),
            stain_radius_max: shipped::stain_radius_max(),
            trauma_per_wound: shipped::trauma_per_wound(),
            hitstop_seconds: shipped::hitstop_seconds(),
            shake_amplitude: shipped::shake_amplitude(),
            shake_ticks: shipped::shake_ticks(),
            effect_capacity: shipped::effect_capacity(),
            max_ribbons: shipped::max_ribbons(),
            pool_merge_radius: shipped::pool_merge_radius(),
            pool_spread: shipped::pool_spread(),
            pool_spread_rate: shipped::pool_spread_rate(),
            max_pools: shipped::max_pools(),
        }
    }
}

impl CarnageSettings {
    /// Reject a settings block that cannot produce a sane schedule.
    ///
    /// **Two of these are real crashes, not hypotheticals**, which is the same standard
    /// [`FractureSettings::validate`] is held to. `shake_ticks` is a modulus, so zero panics the
    /// first time a camera shakes; `spurt_bpm` at zero divides by zero deriving the pulse period. The
    /// remaining checks catch inverted ranges, which do not panic but silently invert the model — a
    /// `clot_ticks` below `spurt_ticks` would make flow rise before it fell.
    ///
    /// Call it at load. Failing loudly at the door is the one path; clamping a bad pair here would be
    /// a second, quieter one.
    pub fn validate(&self) -> Result<(), String> {
        if self.shake_ticks == 0 {
            return Err(
                "carnage: shake_ticks is 0 — it is the modulus of the shake phase, so the first \
                 shake would panic on a division by zero. Use 1 or more."
                    .to_string(),
            );
        }
        if !(self.spurt_bpm > 0.0) || !self.spurt_bpm.is_finite() {
            return Err(format!(
                "carnage: spurt_bpm is {} — the pulse period is `60 / bpm`, so this must be finite \
                 and positive.",
                self.spurt_bpm
            ));
        }
        if self.clot_ticks < self.spurt_ticks {
            return Err(format!(
                "carnage: clot_ticks ({}) < spurt_ticks ({}) — flow is full until `spurt_ticks` and \
                 zero at `clot_ticks`, so an inverted pair would have it rise before it fell.",
                self.clot_ticks, self.spurt_ticks
            ));
        }
        for (name, lo, hi) in [
            ("droplet_size", self.droplet_size_min, self.droplet_size_max),
            ("stain_radius", self.stain_radius_min, self.stain_radius_max),
        ] {
            if !(lo > 0.0) || !(hi >= lo) || !lo.is_finite() || !hi.is_finite() {
                return Err(format!(
                    "carnage: {name}_min ({lo}) and {name}_max ({hi}) must be finite with \
                     0 < min <= max — the pair is lerped, and an inverted one reverses the model."
                ));
            }
        }
        if !(0.0..=1.0).contains(&self.trauma_per_wound) {
            return Err(format!(
                "carnage: trauma_per_wound is {} — trauma is in [0, 1] and the caller accumulates \
                 it, so a value outside that is not a stronger hit, it is a broken one.",
                self.trauma_per_wound
            ));
        }
        if !(0.0..=180.0).contains(&self.spatter_cone_deg) {
            return Err(format!(
                "carnage: spatter_cone_deg is {} — it is a half-angle about the wound normal, so it \
                 must be in [0, 180].",
                self.spatter_cone_deg
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
        if self.max_pools == 0 {
            return Err(
                "carnage: max_pools is 0 — every stain would be dropped and blood would never \
                 accumulate, which is the whole feature switched off by a ceiling."
                    .to_string(),
            );
        }
        for (name, v) in
            [("pool_merge_radius", self.pool_merge_radius), ("pool_spread", self.pool_spread)]
        {
            if !(v > 0.0) || !v.is_finite() {
                return Err(format!(
                    "carnage: {name} is {v} — it scales a radius, so it must be finite and positive."
                ));
            }
        }
        if !(self.pool_spread_rate > 0.0 && self.pool_spread_rate <= 1.0) {
            return Err(format!(
                "carnage: pool_spread_rate is {} — it is the fraction of the remaining gap closed \
                 per tick, so it must be in (0, 1]. At 0 a pool never spreads; above 1 it \
                 overshoots and oscillates.",
                self.pool_spread_rate
            ));
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
    pub fn to_world(self, xf: &GlobalTransform) -> Wounded {
        Wounded {
            at: xf.transform_point(self.at),
            normal: (xf.affine().matrix3 * self.normal).normalize_or_zero(),
            area: self.area,
            severity: self.severity,
            kind: self.kind,
        }
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
    #[test]
    fn every_carnage_serde_default_is_the_shipped_value() {
        let d = CarnageSettings::default();
        let pairs_f32: &[(&str, f32, f32)] = &[
            ("droplets_per_m2", default_droplets_per_m2(), d.droplets_per_m2),
            ("spatter_speed_scale", default_spatter_speed_scale(), d.spatter_speed_scale),
            ("spatter_cone_deg", default_spatter_cone_deg(), d.spatter_cone_deg),
            ("droplet_size_min", default_droplet_size_min(), d.droplet_size_min),
            ("droplet_size_max", default_droplet_size_max(), d.droplet_size_max),
            ("gravity", default_gravity(), d.gravity),
            ("drag", default_drag(), d.drag),
            ("spurt_bpm", default_spurt_bpm(), d.spurt_bpm),
            ("stain_radius_min", default_stain_radius_min(), d.stain_radius_min),
            ("stain_radius_max", default_stain_radius_max(), d.stain_radius_max),
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
            ("max_droplets_per_wound", default_max_droplets_per_wound(), d.max_droplets_per_wound),
            ("spurt_ticks", default_spurt_ticks(), d.spurt_ticks),
            ("clot_ticks", default_clot_ticks(), d.clot_ticks),
            ("shake_ticks", default_shake_ticks(), d.shake_ticks),
            ("effect_capacity", default_effect_capacity(), d.effect_capacity),
        ];
        for (name, serde_value, shipped_value) in pairs_u32 {
            assert_eq!(serde_value, shipped_value, "{name}: serde and shipped defaults disagree");
        }
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

        let partial: CarnageSettings = ron::from_str("(spurt_bpm: 120.0)")
            .expect("a config naming one dial must take the shipped values for the rest");
        assert_eq!(partial.spurt_bpm, 120.0, "the authored dial must survive");
        assert_eq!(
            partial.clot_ticks,
            default_clot_ticks(),
            "an unauthored dial must take the shipped value"
        );

        assert!(
            ron::from_str::<CarnageSettings>("(spurt_bmp: 120.0)").is_err(),
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
        assert!(bad(|s| s.spurt_bpm = 0.0).contains("spurt_bpm"));
        assert!(bad(|s| s.clot_ticks = 10).contains("clot_ticks"));
        assert!(bad(|s| s.droplet_size_max = 0.0).contains("droplet_size"));
        assert!(bad(|s| s.stain_radius_min = 0.0).contains("stain_radius"));
        assert!(bad(|s| s.trauma_per_wound = 1.5).contains("trauma_per_wound"));
        assert!(bad(|s| s.spatter_cone_deg = 200.0).contains("spatter_cone_deg"));
        assert!(bad(|s| s.effect_capacity = 0).contains("effect_capacity"));
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
        let out = w.to_world(&shifted);
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
        w.to_world(xf).normal
    }
}
