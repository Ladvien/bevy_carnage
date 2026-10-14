//! **The GPU half.** Blood you can see, and nothing that decides anything.
//!
//! Entirely behind the `vfx` feature. The deterministic half of this crate never references this
//! module, which is what lets a headless harness take the bake, the wounds, the spatter and the bleed
//! schedule with no render stack in the dependency graph at all.
//!
//! # Write-only, and the library enforces it
//!
//! Particles here are **output**. Nothing reads them back, and nothing can: `bevy_hanabi` 0.19 has no
//! public GPU→CPU readback path whatsoever — the only `map_async` in the crate is behind
//! `#[cfg(all(test, feature = "gpu_tests"))]`, and the `copy_buffer_to_buffer` calls are internal
//! buffer reallocation. So a particle's position is physically unable to reach a golden, a hash or a
//! simulation.
//!
//! That is stated rather than left implicit because the rule survives the library: a future idea of
//! the form "read the particle positions back to place decals" must be refused. Stains are computed
//! on the CPU by [`crate::spatter::stains`], deterministically, and the particles are a separate,
//! cosmetic account of the same event.
//!
//! # Everything is authored around local +Y
//!
//! `SetPositionCone3dModifier` is a **Y-axis** cone — its WGSL sets `let y = h;` — so every asset here
//! sprays along local `+Y`, and an emitter is aimed with exactly one operation:
//! `Quat::from_rotation_arc(Vec3::Y, wound_normal)`. There is no per-effect axis convention to
//! remember and no second way to aim one.
//!
//! **There is no cone-velocity modifier in 0.19.** A cone spray is the Y-axis cone *position* modifier
//! plus a sphere *velocity* centred behind the cone's apex, so the velocities fan out through the
//! positions. That is why every effect below pairs those two.
//!
//! # Ticks and TTL
//!
//! `EffectTtl` is **this crate's** component, not Hanabi's — 0.19 has no such type. It exists because
//! `EffectSpawner::has_completed()` reports that the *spawner* finished emitting, which happens long
//! before the particles it emitted have died. Despawning on that alone cuts a spray off mid-flight.
//! Both conditions are required, which is what [`despawn_finished_effects`] checks.

use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_hanabi::{
    AccelModifier, Attribute, ColorOverLifetimeModifier, EffectAsset, EffectMaterial, EffectSpawner, ExprWriter,
    Gradient, HanabiPlugin, ImageSampleMapping, KillAabbModifier, LinearDragModifier, MotionIntegration, OrientMode,
    OrientModifier, ParticleEffect, ParticleTextureModifier, RoundModifier, ScalarType, SetAttributeModifier,
    SetPositionCone3dModifier, SetVelocitySphereModifier, ShapeDimension, SimulationCondition, SimulationSpace,
    SizeOverLifetimeModifier, SpawnerSettings,
};

use std::sync::LazyLock;

use crate::bloodstain::spectral::Film;
use crate::bloodstain::{BACK_SPATTER_SPEED, FORWARD_SPATTER_SPEED, PatternClass, wound_seed};
use crate::wound::{Wound, WoundKind};
use crate::{CarnageSettings, Wounded};

/// The blood effects, built once at startup: the four a wound throws, twice, and the ribbon.
///
/// Separate assets rather than one parameterised asset because a particle effect's **capacity,
/// spawner and colour ramp are baked into the asset** and cannot be changed per instance — so "a
/// burst of 300" and "a steady trickle" are necessarily different assets, and so are a burst of blood
/// and the same burst in the substitute palette.
#[derive(Resource, Debug, Clone)]
pub struct CarnageEffects {
    /// The four a wound throws, in blood.
    pub blood: Bursts,
    /// **The same four in [`CarnageSettings::substitute_srgb`]**, thrown instead where the
    /// [`GorePolicy`](crate::GorePolicy) draws no blood: the same emitter, at the same tick, in the
    /// same direction and number, in a spark's colour.
    pub substitute: Bursts,
    /// The blood ribbon a flying gib drags behind it. One instance per chunk, each its own strand.
    pub ribbon: Handle<EffectAsset>,
    /// [`puff_image`], the soft disc the mist is drawn with. An instance of either `mist` needs it as
    /// its [`EffectMaterial`], which the spawn system gives it.
    pub puff: Handle<Image>,
}

/// The four effects a wound throws, in one [`Palette`].
#[derive(Debug, Clone)]
pub struct Bursts {
    /// The impact spray: many droplets, one burst, world space.
    pub spatter: Handle<EffectAsset>,
    /// The fine mist that hangs where the round went through.
    pub mist: Handle<EffectAsset>,
    /// One jet per heartbeat, for a wound that is still pumping.
    pub spurt: Handle<EffectAsset>,
    /// The steady seep of a wound that has stopped pumping, in local space so it rides the fragment.
    pub seep: Handle<EffectAsset>,
}

/// **What a burst is drawn in.** The droplets' size, speed, number and flight are the same in both;
/// only the colour ramp differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    /// The film model's blood, at the thickness of what each effect is made of: computed, never
    /// authored.
    Blood,
    /// [`CarnageSettings::substitute_srgb`], the colour a tier that draws no blood substitutes for it.
    Substitute,
}

impl Palette {
    /// The palette's colour for an effect made of `film`, linear, as a ramp takes it.
    ///
    /// **Linear, because Hanabi blends in linear space.** This was `spectral::srgb` — the gamma-encoded
    /// triple every *material* in this crate is authored in — and a particle gradient takes its values
    /// as linear. Encoded values read as linear display lighter and greyer, so the spray rendered as
    /// the dusty pink of round 1's R1-03 rather than as the blood the film model computes. Hanabi's own
    /// examples convert with `to_linear()` before building a gradient for the same reason.
    fn linear(self, s: &CarnageSettings, film: &Film) -> [f32; 3] {
        match self {
            Palette::Blood => crate::bloodstain::spectral::linear_srgb(film),
            Palette::Substitute => s.substitute_srgb,
        }
    }
}

/// **How long an effect instance may live after its spawner finishes**, in this crate's own ticks.
///
/// Not a Hanabi type — 0.19 has none. See the module docs for why one condition is not enough.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectTtl(pub u32);

/// **The film a spatter droplet in flight is**, and therefore its colour.
///
/// 0.4 mm of arterial blood over a dark substrate: a droplet is a thin lens of blood that mostly
/// sees itself, and `crate::bloodstain::spectral` turns a thickness and an oxygen saturation into a colour
/// through 81 wavelengths of Kubelka–Munk over Bosschaart's whole-blood tables
/// (`doi:10.1007/s10103-013-1446-7`). **No blood colour is authored in this crate.** A droplet is
/// arterial rather than venous because spatter is thrown from a fresh opening — the same reason
/// `bloodstain`'s spray model uses the arterial saturation.
const DROPLET_FILM: Film = Film { thickness_mm: 0.4, so2: crate::bloodstain::SO2_ARTERIAL, substrate: 0.2 };

/// **How much of a droplet's flight it is drawn along**, seconds: a ninetieth. A drop is drawn its
/// width plus its speed times this, along its velocity, so at the 8 to 40 m/s it leaves a wound at it
/// is a streak of 9 to 44 cm, which drag shortens to a dash of a few centimetres as it slows to its
/// fall. A sixtieth, the time a film camera's shutter is open, drew the fastest as needles half a metre
/// long across the frame. This crate's own number.
pub const DROPLET_STREAK_S: f32 = 1.0 / 90.0;

/// **The film a puff of mist is.** The fine fraction is droplets a few tens of micrometres across, a
/// lens a twentieth of a millimetre thick that lets most of what is behind it through, so the cloud is
/// paler than the spray — the same model, at the thickness of what it is made of.
const MIST_FILM: Film = Film { thickness_mm: 0.05, so2: crate::bloodstain::SO2_ARTERIAL, substrate: 0.2 };

/// **The film a ribbon strand is.** A strand dragged off a flying gib is a running film rather than
/// a lens, so it is thicker and venous — the same two dials, one model.
const RIBBON_FILM: Film = Film { thickness_mm: 1.0, so2: crate::bloodstain::SO2_VENOUS, substrate: 0.2 };

/// The strand colour, computed once and linear, for the reason [`Palette::linear`] gives.
static RIBBON_LINEAR: LazyLock<[f32; 3]> = LazyLock::new(|| crate::bloodstain::spectral::linear_srgb(&RIBBON_FILM));

/// A droplet's colour over its life, from `colour` (linear), fading to a darker, transparent end.
///
/// One ramp shared by the four wound effects, because blood is blood — the difference between a spurt
/// and a seep is its rate and its speed, not its colour, and two nearly-identical gradients would
/// drift apart the first time one was tweaked. The substitute palette goes through the same ramp, so a
/// spark fades as a droplet does.
///
/// **The blood colour is [`DROPLET_FILM`]'s; only the *fade* is authored.** The two later keys scale
/// that one colour down and take the alpha out, which is a lifetime curve rather than a claim about
/// what blood looks like: a droplet thins and dries as it flies, and the crate has no spectral model of
/// a droplet mid-flight to ask instead. The factors are the ratios the hand-authored ramp had.
fn burst_gradient([r, g_, b]: [f32; 3], opacity: f32) -> Gradient<Vec4> {
    let key = |scale: f32, alpha: f32| Vec4::new(r * scale, g_ * scale, b * scale, alpha * opacity);
    let mut g = Gradient::new();
    g.add_key(0.0, key(1.0, 1.0));
    g.add_key(0.55, key(0.645, 0.95));
    g.add_key(1.0, key(0.29, 0.0));
    g
}

/// The shared skeleton of the four wound effects: a Y-axis cone of positions, a sphere of velocities
/// fanned out through them, a randomised lifetime, gravity, drag, a kill volume, and billboards in the
/// palette's colour — a hard-edged drop drawn out along its velocity, or for the mist a soft disc that
/// faces the eye and spreads.
///
/// **One builder, four callers.** The effects genuinely differ in spawner, capacity, simulation space
/// and a handful of numbers; everything else was identical five times over in the first draft, and
/// five copies of a modifier stack is five places for a look fix to be applied four times.
struct BloodEffect {
    name: &'static str,
    /// Cone half-width at the emitter, metres.
    base_radius: f32,
    /// Cone height, metres — how far ahead of the wound the droplets start.
    height: f32,
    /// Speed range, m/s, **already scaled** — the caller applies
    /// [`crate::BloodSettings::spatter_speed_scale`] when its numbers come from the measured
    /// constants.
    speed: [f32; 2],
    /// Lifetime range, seconds.
    lifetime: [f32; 2],
    /// Drag multiplier over the settings' own dial.
    drag_scale: f32,
    /// Billboard size, metres: a droplet's width, which its speed draws out (see
    /// [`DROPLET_STREAK_S`]), or a puff's diameter.
    size: f32,
    /// How big a particle is at the end of its life, as a multiple of its start: `0` dwindles to
    /// nothing, as a droplet that thins in flight does; above `1` it spreads, as a cloud does.
    grow: f32,
    /// A soft disc that faces the camera ([`puff_image`]), for a cloud, rather than a hard-edged drop
    /// drawn out along its velocity.
    soft: bool,
    /// The ramp's alpha at birth, `1` for a droplet.
    opacity: f32,
    /// What it is made of, for its colour in [`Palette::Blood`].
    film: Film,
    /// How much of the settings' gravity it falls under, `1` for a droplet.
    gravity_scale: f32,
    space: SimulationSpace,
    condition: SimulationCondition,
    spawner: SpawnerSettings,
}

impl BloodEffect {
    fn build(self, s: &CarnageSettings, palette: Palette) -> EffectAsset {
        let writer = ExprWriter::new();

        // Positions on the surface of a Y-axis cone: the spray's footprint at the wound.
        let init_pos = SetPositionCone3dModifier {
            height: writer.lit(self.height).expr(),
            base_radius: writer.lit(self.base_radius).expr(),
            top_radius: writer.lit(self.base_radius * 0.15).expr(),
            dimension: ShapeDimension::Volume,
        };

        // **One draw drives both size and speed, inversely — the paper's correlation, on the GPU.**
        //
        // `t` in `[0, 1)` is the size fraction, exactly as `spatter::droplet` uses it: the diameter
        // lerps min→max across `t` while the speed lerps fast→slow across the *same* `t`, so the
        // biggest droplet is the slowest. Without this the particles are all one size travelling at
        // random speeds, which is what a first pass produced and which reads as confetti — the
        // failure mode the module docs name.
        let t = writer.rand(ScalarType::Float);

        // Velocities radiating from a centre *behind* the cone's apex, so each droplet flies outward
        // through its own position and the set fans into a cone. This pairing is the substitute for
        // the cone-velocity modifier 0.19 does not have.
        let init_vel = SetVelocitySphereModifier {
            center: writer.lit(Vec3::new(0.0, -self.height, 0.0)).expr(),
            speed: (writer.lit(self.speed[1])
                + t.clone() * writer.lit(self.speed[0] - self.speed[1]))
            .expr(),
        };

        // The drawn size, stashed on a spare per-particle float so the update pass can shrink it
        // without losing what it started at. `SIZE` itself is overwritten every update, which is why
        // the initial value cannot live there — the pattern Hanabi's own `puffs` example uses.
        let init_size = SetAttributeModifier::new(
            Attribute::F32_0,
            (writer.lit(self.size * 0.45) + t * writer.lit(self.size * 1.1)).expr(),
        );

        let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
        let init_lifetime = SetAttributeModifier::new(
            Attribute::LIFETIME,
            (writer.lit(self.lifetime[0])
                + writer.rand(ScalarType::Float) * writer.lit(self.lifetime[1] - self.lifetime[0]))
            .expr(),
        );

        // Size over life, from each particle's own starting size:
        // `size = F32_0 * (1 + (grow - 1) * age/lifetime)` clamped at zero, so a droplet (`grow` 0)
        // dwindles instead of blinking out and a puff (`grow` above 1) spreads, each keeping its own
        // scale while doing it.
        let shrink = writer
            .lit(1.0)
            .add(writer.lit(self.grow - 1.0).mul(writer.attr(Attribute::AGE).div(writer.attr(Attribute::LIFETIME))))
            .max(writer.lit(0.0));
        let width = writer.attr(Attribute::F32_0).mul(shrink.clone());
        let update_size = if self.soft {
            SetAttributeModifier::new(Attribute::SIZE, width.expr())
        } else {
            // **A drop is as long as the way it travels while the eye takes it in**: its width, and
            // along its velocity [`DROPLET_STREAK_S`] of its flight more, dwindling as the drop does.
            // A drop drawn as wide as it is long read as a red disc hanging in the air however fast it
            // flew; drawn out, a spray is fast at the wound and slows to round drops as drag takes it.
            let streak = writer.attr(Attribute::VELOCITY).length().mul(writer.lit(DROPLET_STREAK_S)).mul(shrink);
            SetAttributeModifier::new(Attribute::SIZE2, width.clone().add(streak).vec2(width).expr())
        };
        let texture_slot = writer.lit(0u32).expr();

        // The kill volume is generous rather than tight: it exists so a droplet that escapes the
        // scene is reclaimed, not to clip the spray. Sized off the throw distance the fastest droplet
        // could manage in its longest lifetime.
        let reach = (self.speed[1] * self.lifetime[1]).max(4.0);
        let kill_center = writer.lit(Vec3::ZERO).expr();
        let kill_half = writer.lit(Vec3::splat(reach)).expr();

        let mut module = writer.finish();
        // **The same gravity the CPU spatter model flies its droplets under**, so a particle and the
        // stain it corresponds to agree about where blood goes. Two gravities would put the visible
        // spray and the deterministic stain in different places.
        let gravity = AccelModifier::constant(&mut module, Vec3::NEG_Y * s.blood.gravity * self.gravity_scale);
        let drag = LinearDragModifier::constant(&mut module, s.blood.drag * self.drag_scale);
        let kill = KillAabbModifier::new(kill_center, kill_half);
        // **A drop, not a square.** A bare billboard is a square quad, and a cloud of hundreds of
        // them read as pink confetti hanging round the body (visual pass round 1, R1-03). The
        // ellipse discards the corners in the fragment stage, so each droplet is a round drop.
        let round = RoundModifier::ellipse(&mut module);
        if self.soft {
            module.add_texture_slot("puff");
        }

        let asset = EffectAsset::new(s.effect_capacity, self.spawner, module)
            .with_name(match palette {
                Palette::Blood => self.name.to_string(),
                Palette::Substitute => format!("{}:substitute", self.name),
            })
            .with_simulation_space(self.space)
            .with_simulation_condition(self.condition)
            .init(init_pos)
            .init(init_vel)
            .init(init_size)
            .init(init_age)
            .init(init_lifetime)
            .update(gravity)
            .update(drag)
            .update(update_size)
            .update(kill)
            .render(ColorOverLifetimeModifier::new(burst_gradient(palette.linear(s, &self.film), self.opacity)));
        if self.soft {
            // A cloud has no direction of travel worth showing; it faces the eye, its edge soft.
            asset
                .render(OrientModifier::new(OrientMode::FaceCameraPosition))
                .render(ParticleTextureModifier { texture_slot, sample_mapping: ImageSampleMapping::ModulateOpacityFromR })
        } else {
            // Along velocity, so a droplet reads as a streak in the direction it is travelling
            // rather than as a sphere — which is most of what makes a spray look fast.
            asset.render(OrientModifier::new(OrientMode::AlongVelocity)).render(round)
        }
    }
}

/// **The soft disc the mist is drawn with**: 64 × 64, single channel, `(1 − r²)²` from the centre to
/// nothing at the edge, which a particle reads as its opacity. Built from that formula, not loaded.
pub fn puff_image() -> Image {
    const N: u32 = 64;
    let data: Vec<u8> = (0..N * N)
        .map(|i| {
            let (x, y) = ((i % N) as f32 + 0.5, (i / N) as f32 + 0.5);
            let (dx, dy) = (x / N as f32 * 2.0 - 1.0, y / N as f32 * 2.0 - 1.0);
            let r2 = (dx * dx + dy * dy).min(1.0);
            ((1.0 - r2) * (1.0 - r2) * 255.0).round() as u8
        })
        .collect();
    Image::new(
        Extent3d { width: N, height: N, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// **The impact spray.** One burst, the full measured speed span, world space so it detaches from the
/// body that threw it.
///
/// `SpawnerSettings::once(1.0)` is a placeholder count: the per-instance [`EffectSpawner`] the spawn
/// system inserts overrides it with the wound's own [`crate::droplet_count`], which is the only place
/// that number can come from because it depends on the wound's area.
pub fn spatter_burst(s: &CarnageSettings, palette: Palette) -> EffectAsset {
    BloodEffect {
        name: "carnage:spatter",
        base_radius: 0.03,
        height: 0.06,
        speed: [
            BACK_SPATTER_SPEED * s.blood.spatter_speed_scale,
            FORWARD_SPATTER_SPEED * s.blood.spatter_speed_scale,
        ],
        // **Tuned, not measured.** At `drag = drag * 2.6` a droplet's speed e-folds in about a
        // quarter of a second, so it visibly decelerates and then falls at the terminal speed
        // `gravity/drag` instead of flying flat; the longer lifetime is what lets that fall be seen
        // rather than the droplet dying at the top of its arc. The size is a streak's width, 5 to 19
        // mm ([`DROPLET_STREAK_S`] gives its length): until 0.5.64 a drop was a disc 14 to 47 mm
        // across, which had to be that wide to read as a drop rather than as dust, and in the two
        // frames a spray is in a body-sized view read as red polka dots.
        lifetime: [0.55, 1.30],
        drag_scale: 2.6,
        size: 0.012,
        grow: 0.0,
        soft: false,
        opacity: 1.0,
        film: DROPLET_FILM,
        gravity_scale: 1.0,
        space: SimulationSpace::Global,
        condition: SimulationCondition::WhenVisible,
        spawner: SpawnerSettings::once(1.0.into()),
    }
    .build(s, palette)
}

/// **The mist: a puff that hangs where the round went in.** The fine fraction of a spray leaves at the
/// paper's forward speed and still air stops it almost at once: a droplet 20 µm across relaxes in
/// `ρd²/18μ`, about 1.3 ms, so even at 40 m/s it stops within 5 cm of the wound (Stokes) and hangs.
/// So the mist is born in a cone 4 cm deep at the wound and spreads slowly from there, each puff a
/// soft disc ([`puff_image`]) that grows to three times its size and fades over a quarter to half a
/// second, barely falling, in the colour of a film as thin as its droplets ([`MIST_FILM`]). What a player reads as the hit is this cloud: the spray's
/// droplets at 8 to 40 m/s are gone from a body-sized view within two frames.
///
/// The speeds, sizes, lifetimes, opacity and the twentieth of gravity are **tuned, not measured**; the
/// stopping distance is what they were tuned to. An instance needs [`CarnageEffects::puff`] as its
/// [`EffectMaterial`].
pub fn mist_puff(s: &CarnageSettings, palette: Palette) -> EffectAsset {
    BloodEffect {
        name: "carnage:mist",
        base_radius: 0.03,
        height: 0.04,
        speed: [0.4, 1.6],
        lifetime: [0.25, 0.50],
        drag_scale: 2.0,
        size: 0.06,
        grow: 3.0,
        soft: true,
        opacity: 0.3,
        film: MIST_FILM,
        gravity_scale: 0.05,
        space: SimulationSpace::Global,
        condition: SimulationCondition::WhenVisible,
        spawner: SpawnerSettings::once(1.0.into()),
    }
    .build(s, palette)
}

/// **The arterial jet.** One burst per heartbeat, at the wound's own rate.
///
/// `SimulationCondition::Always` because an off-screen body must keep bleeding: a wound that pauses
/// while the camera looks away and resumes when it looks back would be visibly wrong the moment the
/// camera came back to a corpse that had bled for no time at all.
pub fn arterial_spurt(s: &CarnageSettings, palette: Palette) -> EffectAsset {
    let period = if s.blood.spurt_bpm > 0.0 { 60.0 / s.blood.spurt_bpm } else { 1.0 };
    BloodEffect {
        name: "carnage:spurt",
        base_radius: 0.015,
        height: 0.05,
        speed: [
            FORWARD_SPATTER_SPEED * 0.35 * s.blood.spatter_speed_scale,
            FORWARD_SPATTER_SPEED * 0.6 * s.blood.spatter_speed_scale,
        ],
        // Tuned, not measured — same reasoning as the burst above, at a lighter drag because a jet
        // is meant to carry further than an impact spray.
        lifetime: [0.60, 1.40],
        drag_scale: 1.8,
        size: 0.014,
        grow: 0.0,
        soft: false,
        opacity: 1.0,
        film: DROPLET_FILM,
        gravity_scale: 1.0,
        space: SimulationSpace::Global,
        condition: SimulationCondition::Always,
        spawner: SpawnerSettings::burst(24.0.into(), period.into()),
    }
    .build(s, palette)
}

/// **The seep.** A slow steady rate in **local** space, so it rides the fragment it is attached to
/// instead of being left behind as the chunk tumbles.
pub fn wound_seep(s: &CarnageSettings, palette: Palette) -> EffectAsset {
    BloodEffect {
        name: "carnage:seep",
        base_radius: 0.012,
        height: 0.02,
        // `spatter_speed_scale` is deliberately **not** applied: these are not the paper's numbers,
        // they are authored at demo scale already, and scaling them would apply the dial twice for
        // one meaning.
        speed: [0.15, 0.6],
        lifetime: [0.5, 1.1],
        drag_scale: 2.0,
        size: 0.014,
        grow: 0.0,
        soft: false,
        opacity: 1.0,
        film: DROPLET_FILM,
        gravity_scale: 1.0,
        space: SimulationSpace::Local,
        condition: SimulationCondition::Always,
        spawner: SpawnerSettings::rate(26.0.into()),
    }
    .build(s, palette)
}

/// **How long one strand particle lives, seconds — and it must be a literal constant.**
///
/// There is no code-level rejection of a randomised ribbon lifetime, which is why this is written
/// down: a particle that dies in the *middle* of a strand reorders the chain visibly. Upstream states
/// the rule at `bevy_hanabi/examples/ribbon.rs:132-137`. This is the one place the shipped
/// [`CarnageSettings`] must **not** be consulted.
const RIBBON_LIFETIME: f32 = 0.9;

/// **Emission rate, particles per second ≈ the frame rate.**
///
/// Hanabi 0.19 does not interpolate position between frames, so every particle spawned in one frame
/// lands at the same point and any rate above the frame rate is pure waste
/// (`bevy_hanabi/examples/ribbon.rs:65-68`).
const RIBBON_RATE: f32 = 60.0;

/// `RIBBON_RATE * RIBBON_LIFETIME` is 54 live particles; this is that plus slack, following upstream's
/// own arithmetic (`ribbon.rs:71-75`).
///
/// **Deliberately not [`CarnageSettings::effect_capacity`]** (4096): that dial is sized for a spatter
/// burst, and a per-gib ribbon is two orders of magnitude smaller. Over-allocating is not free — each
/// instance reserves its capacity inside a 65,536-particle slab, so 4096 would fit sixteen gibs.
const RIBBON_CAPACITY: u32 = 64;

/// **How wide a strand is over its first half, metres**, tapering to zero at the tail.
///
/// **This is the strand's whole width, and `Attribute::SIZE` cannot set it.** Hanabi's
/// [`SizeOverLifetimeModifier`] *assigns* rather than scales — its generated vertex code is
/// `size = gradient(age / lifetime)` (`bevy_hanabi-0.19.0/src/modifier/output.rs:446`) — so a
/// `SetAttributeModifier` on `SIZE` at init is dead code and the gradient's own values are the width
/// in world units.
///
/// That is a trap this crate fell into: the gradient was `Vec3::ONE → Vec3::ZERO`, copied from
/// `bevy_hanabi/examples/ribbon.rs`, whose scene is fifty units across — so every gib dragged a
/// **one-metre-wide** sheet on a 1.8 m subject. Captured offscreen: each flying chunk read as a red
/// fan the size of the torso, which is what "it doesn't look like organic carnage at all" was.
///
/// 20 mm is a dragged stream of blood at human scale, about a thumb's width — still an order of
/// magnitude under the 0.14 m half-depth of the torso the gibs come out of. It was 12 mm tapering
/// from the head, so most of every strand was thinner than a finger and read as fishing line from a
/// few metres away (visual pass round 1, R1-23); the gradient now holds the width over the first half.
const RIBBON_WIDTH: f32 = 0.020;

/// **The blood ribbon a flying gib drags behind it.** One connected strand per instance, left in world
/// space, thinning and darkening over [`RIBBON_LIFETIME`].
///
/// # One asset serves every gib, and the crate used to claim otherwise
///
/// This function replaced a `gib_trail` whose doc read: *"Deliberately not a `RIBBON_ID` ribbon.
/// Hanabi supports one ribbon chain per effect asset, so a single ribbon asset cannot serve several
/// simultaneous gibs — every gib would be threaded onto one strand."* **That was false**, and the
/// correction is what makes this feature cheap, so it is recorded rather than quietly deleted. Read
/// from `bevy_hanabi` 0.19.0:
///
/// - Instances of one asset are packed into a shared slab, but `allocate()` hands each instance a
///   **disjoint contiguous sub-slice** (`src/render/effect_cache.rs:850-867`).
/// - The ribbon shader resolves the owning instance's `base_particle` per vertex and indexes strictly
///   inside that slice (`src/render/vfx_render.wgsl:214-241`). A chain cannot walk out of its own
///   instance.
/// - Upstream's own `examples/ribbon.rs:146-151` therefore uses a literal `RIBBON_ID = 0u32` for every
///   particle — "they all share the same RIBBON_ID, which can be any value" — and spawns and despawns
///   those instances continuously at runtime.
///
/// So: one asset, one [`ParticleEffect`] entity per gib, constant `RIBBON_ID`, each gib its own
/// independent chain. Distinct ids are needed only for several logical chains inside a *single*
/// instance, which is upstream's `worms.rs` topology and not ours.
///
/// # Why it cannot use [`BloodEffect::build`]
///
/// That builder emits a position cone plus sphere velocity plus full motion integration, which is the
/// opposite of a ribbon: a strand's particles must stay exactly where they were emitted, and the
/// illusion of motion comes entirely from the emitter moving away from them.
///
/// `AGE` is not optional here — shader generation fails outright with a `Validate` error when
/// `RIBBON_ID` is present without it (`bevy_hanabi/src/lib.rs:847-856`), and the render node asserts
/// both offsets (`src/render/mod.rs:7567-7568`).
pub fn gib_ribbon() -> EffectAsset {
    let writer = ExprWriter::new();

    let init_pos =
        SetAttributeModifier::new(Attribute::POSITION, writer.lit(Vec3::ZERO).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let init_lifetime =
        SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(RIBBON_LIFETIME).expr());
    let init_ribbon_id = SetAttributeModifier::new(Attribute::RIBBON_ID, writer.lit(0u32).expr());

    EffectAsset::new(RIBBON_CAPACITY, SpawnerSettings::rate(RIBBON_RATE.into()), writer.finish())
        .with_name("carnage:ribbon")
        // Particles must stay where they were emitted; see above.
        .with_motion_integration(MotionIntegration::None)
        // `Global` bakes the emitter translation into POSITION once at init
        // (`bevy_hanabi/src/lib.rs:513-530`) and renders POSITION as world space. Translation only —
        // emitter rotation and scale are ignored, which is correct for a strand.
        .with_simulation_space(SimulationSpace::Global)
        // A gib that leaves the camera must keep trailing, for the same reason an off-screen body
        // must keep bleeding: it comes back with a strand that never happened otherwise.
        .with_simulation_condition(SimulationCondition::Always)
        .init(init_pos)
        .init(init_age)
        .init(init_lifetime)
        .init(init_ribbon_id)
        // **The width, absolutely, because this modifier assigns rather than scales** — hence no
        // `SIZE` init above; one would be dead code. See [`RIBBON_WIDTH`].
        .render(SizeOverLifetimeModifier {
            gradient: {
                let mut g = Gradient::new();
                g.add_key(0.0, Vec3::splat(RIBBON_WIDTH));
                g.add_key(0.5, Vec3::splat(RIBBON_WIDTH));
                g.add_key(1.0, Vec3::ZERO);
                g
            },
            ..default()
        })
        // Darkens and fades rather than turning blue like the upstream demo — this is blood. The
        // colour is [`DROPLET_FILM`]'s, thickened: a strand dragged off a gib is a running film
        // rather than a droplet, so it is the same optics at 1.0 mm and venous, and only the fade is
        // authored. `burst_gradient` is not reusable here because a strand wants two keys, not three.
        .render(ColorOverLifetimeModifier::new(Gradient::linear(
            {
                let [r, g, b] = *RIBBON_LINEAR;
                Vec4::new(r, g, b, 1.0)
            },
            {
                let [r, g, b] = *RIBBON_LINEAR;
                Vec4::new(r * 0.29, g * 0.29, b * 0.29, 0.0)
            },
        )))
}

/// **Marks an entity that should trail blood while it flies.** Insert it on anything moving that
/// should bleed — a gib, a severed limb, a thrown corpse — and remove it when the thing comes to rest.
///
/// The crate deliberately does not learn what a gib is; the consumer owns the decision, and this owns
/// the asset, the cap and the lifetime.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BleedingChunk;

/// The spawned ribbon instance, a child of the entity it trails. Internal bookkeeping, public only so
/// a consumer can query for one in a test.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RibbonInstance;

/// **Stop emitting now, despawn in this many ticks.**
///
/// [`EffectTtl`] cannot retire a rate spawner: [`despawn_finished_effects`] requires
/// `spawner.has_completed()`, and that returns `!settings.is_forever() && …`
/// (`bevy_hanabi/src/spawn.rs:794-804`) — a rate spawner *is* forever, so the condition is never met
/// and a ribbon retired that way would leak for the life of the process.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectFade(pub u32);

/// Ticks between a chunk coming to rest and its ribbon entity going away.
///
/// At least `RIBBON_LIFETIME * 60 = 54`, so the last particle emitted finishes its life first.
const RIBBON_FADE_TICKS: u32 = 60;

/// The set this plugin's systems run in. **Gate and order against this, not against the systems** —
/// the same contract [`crate::CarnageSystems`] carries.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CarnageVfxSystems;

/// **The cosmetic plugin.** Particles, and nothing that a simulation reads.
///
/// Separate from [`CarnagePlugin`](crate::CarnagePlugin) rather than a feature-gated branch inside it,
/// because a headless harness must be able to add the deterministic plugin and *not* this one. A
/// `#[cfg]` inside one plugin would make that a compile-time choice for the whole binary instead of a
/// per-`App` one.
///
/// Everything runs on **`Update`, never `FixedUpdate`**: these are frames of visuals, and putting them
/// on the fixed schedule would tie how much blood is drawn to the simulation rate.
pub struct CarnageVfxPlugin;

impl Plugin for CarnageVfxPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<HanabiPlugin>() {
            app.add_plugins(HanabiPlugin);
        }
        app.init_resource::<CarnageSettings>()
            // **The message is registered beside the system that READS it.**
            //
            // `spawn_wound_effects` takes `MessageReader<Wounded>`, and in Bevy 0.19 a reader whose
            // `Messages<T>` resource is absent does not skip — the system PANICS with "Parameter
            // messages failed validation". Registering it only in `CarnagePlugin` meant any app that
            // added the vfx half alone died on its first `Update`, which is four of this crate's own
            // examples. `add_message` is guarded by `contains_resource`
            // (`bevy_app-0.19.0/src/sub_app.rs:386`), so both plugins declaring it is idempotent
            // rather than a double registration — and a plugin that registers a reader without its
            // message is a plugin that only works next to another one.
            .add_message::<Wounded>()
            // **Both halves of the cosmetic layer, registered here.** The stain-mask cache backs
            // [`crate::spawn_stain`], so a caller that added this plugin and then had to remember a
            // second registration would get silently stain-free blood — which is exactly the failure
            // that put this line here.
            .add_systems(Startup, build_effects)
            .init_resource::<crate::decal::StainMasks>()
            .add_systems(
                Update,
                (
                    spawn_wound_effects,
                    despawn_finished_effects,
                    attach_ribbons,
                    fade_landed_ribbons,
                    fade_effects,
                )
                    .in_set(CarnageVfxSystems),
            );
    }
}

/// Build the nine assets once, from whatever [`CarnageSettings`] is present at startup.
fn build_effects(
    mut commands: Commands,
    mut assets: ResMut<Assets<EffectAsset>>,
    mut images: ResMut<Assets<Image>>,
    settings: Res<CarnageSettings>,
) {
    let s = &*settings;
    let mut bursts = |palette: Palette| Bursts {
        spatter: assets.add(spatter_burst(s, palette)),
        mist: assets.add(mist_puff(s, palette)),
        spurt: assets.add(arterial_spurt(s, palette)),
        seep: assets.add(wound_seep(s, palette)),
    };
    let (blood, substitute) = (bursts(Palette::Blood), bursts(Palette::Substitute));
    commands.insert_resource(CarnageEffects { blood, substitute, ribbon: assets.add(gib_ribbon()), puff: images.add(puff_image()) });
}

/// Spawn one burst per [`Wounded`] message.
///
/// **`prng_seed` is set on every instance, never left `None`** — from the wound's own position, so
/// even the GPU's randomness is a function of where the wound was. Two runs that open the same wound
/// get the same spray, which is not required for correctness (nothing reads it back) but means a
/// recorded demo looks the same twice.
///
/// The aim is one operation: `Quat::from_rotation_arc(Vec3::Y, normal)`, because every asset is
/// authored around local +Y.
///
/// **The player's setting decides the palette**, read as it is when the wound opens: where
/// [`GorePolicy::draws_blood`](crate::GorePolicy::draws_blood) is false the same bursts are thrown
/// from [`CarnageEffects::substitute`]. Before, this read no policy, and a game that turned blood off
/// still sprayed it from every wound the preset announced.
fn spawn_wound_effects(
    mut commands: Commands,
    mut wounded: MessageReader<Wounded>,
    effects: Option<Res<CarnageEffects>>,
    settings: Res<CarnageSettings>,
    policy: Option<Res<crate::GorePolicy>>,
) {
    let Some(effects) = effects else {
        // The assets are built on `Startup`; a message in the same frame arrives before them. Warning
        // per message on a hot path would be worse than the one frame of missing blood.
        return;
    };
    let bursts = if policy.as_deref().is_none_or(crate::GorePolicy::draws_blood) { &effects.blood } else { &effects.substitute };
    for w in wounded.read() {
        let wound = Wound {
            at: w.at,
            normal: w.normal,
            area: w.area,
            severity: w.severity,
            kind: w.kind,
        };
        // One conversion, at the boundary, per wound — `src/v3.rs` says why it lives there and
        // nowhere else.
        let mirror = crate::v3::wound(&wound);
        let count = crate::bloodstain::droplet_count(&mirror, &settings.blood);
        if count == 0 {
            continue;
        }
        let seed = wound_seed(&mirror);
        let rotation = Quat::from_rotation_arc(Vec3::Y, w.normal.normalize_or_zero());
        let transform = Transform { translation: w.at, rotation, scale: Vec3::ONE };

        // **Keyed on the PATTERN CLASS, not on what opened the wound.** `kind` says a severance or a
        // bullet channel; `class` says impact spatter, an arterial arc, a cast-off line, expirated
        // mist, a drip trail or a transfer smear — and it is the class that decides what the blood
        // *does*, so it is the class that decides what is drawn. A channel still mists, because a
        // gunshot's class carries that; before `0.2.0` the mist was welded to `kind` and an arterial
        // severance and a cut looked identical.
        let handles: &[(&Handle<EffectAsset>, u32)] = match w.class {
            // The percolation cone. A channel additionally hangs mist where the round went through.
            PatternClass::Impact => match w.kind {
                WoundKind::Severance => &[(&bursts.spatter, count)],
                WoundKind::Channel => &[(&bursts.spatter, count), (&bursts.mist, count * MIST_PER_DROPLET)],
            },
            // One jet per systole: the spurt effect exists for exactly this and was previously
            // reachable only by a caller wiring it by hand.
            PatternClass::ArterialSpurt => &[(&bursts.spurt, count)],
            // Flung tangentially from a moving tip — a spray, without the mist a bore leaves.
            PatternClass::CastOff => &[(&bursts.spatter, count)],
            // Air-driven and fine: mist is the whole of it.
            PatternClass::Expirated => &[(&bursts.mist, count * MIST_PER_DROPLET)],
            // Gravity-fed. Both are seeps at different rates, and the marks they leave are decals
            // rather than particles — `crate::bloodstain::patterns::{drip_trail, transfer}` return stains.
            PatternClass::DripTrail | PatternClass::Transfer => &[(&bursts.seep, count / 4)],
        };
        for (handle, n) in handles.iter().copied() {
            if n == 0 {
                continue;
            }
            let mut instance = commands.spawn((
                ParticleEffect { handle: handle.clone(), prng_seed: Some(seed) },
                EffectSpawner::new(&SpawnerSettings::once((n as f32).into())),
                transform,
                EffectTtl(TTL_TICKS),
            ));
            if handle.id() == bursts.mist.id() {
                instance.insert(EffectMaterial { images: vec![effects.puff.clone()] });
            }
        }
    }
}

/// **How many puffs of mist a wound throws per droplet of its spray.** Tuned, not measured: the
/// droplet count is the spray that stains, a few to a few dozen for a gunshot, and the mist is the
/// unstained fine fraction, which at one puff a droplet was a handful of discs rather than a cloud.
const MIST_PER_DROPLET: u32 = 3;

/// **How long a one-shot instance is kept after its spawner finishes.**
///
/// Generous on purpose: it bounds the entity's life, it does not time the spray. The particles die on
/// their own `LIFETIME`, and cutting the entity before they do is the exact failure this constant and
/// the two-condition despawn exist to prevent.
const TTL_TICKS: u32 = 180;

/// Despawn a one-shot instance once its spawner has completed **and** its TTL has run out.
///
/// **Both, and that is the point.** `has_completed()` is true as soon as the spawner stops *emitting*,
/// which for a one-shot burst is almost immediately — despawning there deletes the effect while its
/// droplets are still in the air. The TTL alone would leak an instance whose spawner never finishes.
fn despawn_finished_effects(
    mut commands: Commands,
    mut q: Query<(Entity, &EffectSpawner, &mut EffectTtl)>,
) {
    for (entity, spawner, mut ttl) in &mut q {
        ttl.0 = ttl.0.saturating_sub(1);
        if spawner.has_completed() && ttl.0 == 0 {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Give every [`BleedingChunk`] that has no ribbon yet a child ribbon instance, up to the cap.
///
/// **A child, deliberately.** Bevy propagates the parent's transform, so the emitter follows the
/// chunk for free, and Bevy despawns children with the parent — so a chunk that is culled takes its
/// ribbon with it and needs no cleanup path at all.
///
/// The cap is first-come-first-served and never evicts: a ribbon that vanishes mid-flight reads as a
/// glitch, while a chunk with no ribbon reads as a chunk. It is a real ceiling rather than a
/// precaution — Hanabi spawns one `EffectBatch`, and therefore one draw call, per instance
/// (`bevy_hanabi/src/render/mod.rs:4660`), and every *ribbon* instance additionally queues its own
/// sort dispatch (`mod.rs:4684-4698`). Upstream's CHANGELOG records a crash from "many sorted
/// (e.g. ribbon) effects alive at once", fixed only in 0.19.0. Zeler & Rohleder
/// (`10.22630/mgv.2016.25.1.4`, Techland) make the same point about uncapped sub-emission generally.
fn attach_ribbons(
    mut commands: Commands,
    effects: Option<Res<CarnageEffects>>,
    settings: Res<CarnageSettings>,
    chunks: Query<(Entity, &GlobalTransform, Option<&Children>), With<BleedingChunk>>,
    ribbons: Query<(), With<RibbonInstance>>,
) {
    let Some(effects) = effects else { return }; // assets land on `Startup`
    let mut live = ribbons.iter().count() as u32;
    for (entity, at, children) in &chunks {
        if live >= settings.max_ribbons {
            return;
        }
        let already = children
            .is_some_and(|c| c.iter().any(|child| ribbons.get(child).is_ok()));
        if already {
            continue;
        }
        // **Seeded from the chunk's position, never from `Entity`.** `prng_seed` is the only
        // per-instance randomness Hanabi exposes (`bevy_hanabi/src/lib.rs:659-664`), and an `Entity`
        // is a slot index assigned by allocation order — the crate's standing rule (`seed_from_path`)
        // forbids seeding anything from one.
        let p = at.translation();
        let q = |x: f32| (x / crate::soup::WELD).round() as i64 as u32;
        let seed = q(p.x)
            ^ q(p.y).wrapping_mul(0x9E37_79B9)
            ^ q(p.z).wrapping_mul(2_654_435_761);
        // **Silenced, because the chunk may be gone by the time this runs.** Commands apply at the
        // next sync point, and a consumer that despawns its chunks from a system this crate cannot be
        // ordered against (a reset, a cull) does so in between, which logged "Entity despawned" once
        // per ribbon in the `ribbons` example's every reset. The same holds for the three commands
        // below, which use the `try_` forms. A chunk that no longer exists needs no ribbon.
        let ribbon = (
            ParticleEffect { handle: effects.ribbon.clone(), prng_seed: Some(seed) },
            RibbonInstance,
            Transform::default(),
            Visibility::default(),
        );
        commands.entity(entity).queue_silenced(move |mut chunk: EntityWorldMut| {
            chunk.with_child(ribbon);
        });
        live += 1;
    }
}

/// Start the fade on any ribbon whose parent has stopped bleeding — the chunk came to rest, or the
/// consumer decided it should stop. Inserted once; [`EffectFade`]'s presence is the latch.
fn fade_landed_ribbons(
    mut commands: Commands,
    ribbons: Query<(Entity, &ChildOf), (With<RibbonInstance>, Without<EffectFade>)>,
    bleeding: Query<(), With<BleedingChunk>>,
) {
    for (entity, parent) in &ribbons {
        if bleeding.get(parent.parent()).is_err() {
            commands.entity(entity).try_insert(EffectFade(RIBBON_FADE_TICKS));
        }
    }
}

/// Stop a fading effect emitting on the first tick, then despawn it when the counter runs out.
///
/// **`Option<&mut EffectSpawner>`, and the `Option` is load-bearing.** Hanabi adds that component
/// lazily in its own `tick_spawners()` during `PostUpdate` (`bevy_hanabi/src/spawn.rs:634-640`), so it
/// is *absent* on the first frame of a freshly spawned instance — a plain `&mut EffectSpawner` query
/// would silently skip exactly the instances being retired. Upstream's `examples/ribbon.rs:228-241`
/// has the same shape for the same reason.
fn fade_effects(
    mut commands: Commands,
    mut fading: Query<(Entity, &mut EffectFade, Option<&mut EffectSpawner>)>,
) {
    for (entity, mut fade, spawner) in &mut fading {
        if let Some(mut spawner) = spawner {
            spawner.active = false;
        }
        fade.0 = fade.0.saturating_sub(1);
        if fade.0 == 0 {
            commands.entity(entity).try_despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The four droplet effects must build at the authored capacity**, in either palette, which is
    /// a dial rather than a literal because an asset's capacity is fixed when it is built and cannot be
    /// raised afterwards.
    ///
    /// The ribbon is checked separately below: it deliberately does **not** read `effect_capacity`.
    #[test]
    fn the_droplet_effects_build_at_the_authored_capacity() {
        let s = CarnageSettings::default();
        let built: Vec<(&str, EffectAsset)> = [Palette::Blood, Palette::Substitute]
            .into_iter()
            .flat_map(|p| {
                [
                    ("spatter", spatter_burst(&s, p)),
                    ("mist", mist_puff(&s, p)),
                    ("spurt", arterial_spurt(&s, p)),
                    ("seep", wound_seep(&s, p)),
                ]
            })
            .collect();
        for (what, asset) in &built {
            assert_eq!(
                asset.capacity(),
                s.effect_capacity,
                "{what} was built at a capacity other than the dial"
            );
            assert!(asset.name.contains("carnage:"), "{what} is missing its namespaced name");
        }
        let ribbon = gib_ribbon();
        let mut names: Vec<&str> =
            built.iter().map(|(_, a)| a.name.as_str()).chain([ribbon.name.as_str()]).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 9, "two effects share a name, so they are not nine effects");
    }

    /// The five differ in the ways that make them five effects rather than one asset spawned five
    /// times — which is worth asserting because the shared builder makes accidental sameness easy.
    #[test]
    fn the_effects_differ_where_they_must() {
        let s = CarnageSettings::default();
        assert_eq!(
            wound_seep(&s, Palette::Blood).simulation_space,
            SimulationSpace::Local,
            "the seep must ride the fragment it is on"
        );
        assert_eq!(
            arterial_spurt(&s, Palette::Blood).simulation_condition,
            SimulationCondition::Always,
            "an off-screen body must keep bleeding"
        );
        assert_eq!(
            wound_seep(&s, Palette::Blood).simulation_condition,
            SimulationCondition::Always,
            "and so must keep seeping"
        );
    }

    /// **The three properties a ribbon breaks silently if any one of them is wrong.**
    ///
    /// Global space is what leaves the strand behind instead of dragging it along; motion integration
    /// off is what keeps each particle exactly where it was emitted; and the capacity must be the
    /// ribbon's own small number rather than `effect_capacity`, because each instance reserves its
    /// capacity inside a shared 65,536-particle slab and 4096 would fit sixteen gibs.
    ///
    /// None of these can be caught by looking at the screen quickly — (a) reads as one shared strand,
    /// (b) as a strand glued to the chunk, and (c) as ribbons that stop appearing after the
    /// sixteenth chunk.
    #[test]
    fn the_ribbon_is_a_detached_uncapacitied_strand() {
        let asset = gib_ribbon();
        assert_eq!(
            asset.simulation_space,
            SimulationSpace::Global,
            "a ribbon is global space — that is what leaves the strand where it was emitted"
        );
        assert_eq!(
            asset.motion_integration,
            bevy_hanabi::MotionIntegration::None,
            "a ribbon particle must not integrate velocity; the emitter's motion draws the line"
        );
        assert_eq!(
            asset.capacity(),
            RIBBON_CAPACITY,
            "the ribbon must not be sized off `effect_capacity` — see the constant's docs"
        );
        assert!(
            RIBBON_CAPACITY as f32 >= RIBBON_RATE * RIBBON_LIFETIME,
            "capacity {RIBBON_CAPACITY} cannot hold {RIBBON_RATE} particles/s alive for \
             {RIBBON_LIFETIME}s, so the strand would be truncated"
        );
        assert!(
            RIBBON_FADE_TICKS as f32 >= RIBBON_LIFETIME * 60.0,
            "the fade must outlast one particle's life, or a landed chunk's strand is cut off"
        );
    }

    /// The heartbeat period the spurt asset is built with must agree with the CPU schedule's own, or
    /// the visible jets and the deterministic pulses drift apart.
    #[test]
    fn the_spurt_period_matches_the_bleed_schedule() {
        let s = CarnageSettings::default();
        let asset_period = 60.0 / s.blood.spurt_bpm;
        let schedule_period = crate::bloodstain::pulse_period(60, &s.blood) as f32 / 60.0;
        assert!(
            (asset_period - schedule_period).abs() < 0.02,
            "the spurt asset pulses every {asset_period:.4}s but the schedule every \
             {schedule_period:.4}s — the blood and the model would disagree"
        );
    }

    /// **A tier that draws no blood throws the substitute.** The preset still announces every wound at
    /// `Stylised`, so the game can confirm the hit; the particles it turns into are the same bursts in
    /// `substitute_srgb`, and at a tier that draws blood they are blood. The same gunshot under each:
    /// the spray and the mist it spawns are the one set's or the other's, never a mix.
    #[test]
    fn a_tier_that_draws_no_blood_throws_the_substitute() {
        let thrown = |tier: Option<crate::GoreTier>| {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
                .init_asset::<EffectAsset>()
                .init_asset::<Image>()
                .init_resource::<CarnageSettings>()
                .add_message::<Wounded>()
                .add_systems(Startup, build_effects)
                .add_systems(Update, spawn_wound_effects);
            if let Some(tier) = tier {
                app.insert_resource(crate::GorePolicy::for_tier(tier));
            }
            app.update();
            app.world_mut().write_message(Wounded {
                at: Vec3::new(0.0, 1.0, 0.0),
                normal: Vec3::Z,
                area: 2.0e-2,
                severity: 1.0,
                kind: WoundKind::Channel,
                class: PatternClass::Impact,
            });
            app.update();
            let mut q = app.world_mut().query::<&ParticleEffect>();
            let spawned: Vec<AssetId<EffectAsset>> = q.iter(app.world()).map(|e| e.handle.id()).collect();
            let effects = app.world().resource::<CarnageEffects>().clone();
            (spawned, effects)
        };
        for (tier, substitute) in [(Some(crate::GoreTier::Stylised), true), (Some(crate::GoreTier::Blood), false), (None, false)] {
            let (spawned, effects) = thrown(tier);
            let (want, not) = if substitute { (&effects.substitute, &effects.blood) } else { (&effects.blood, &effects.substitute) };
            assert!(!spawned.is_empty(), "the gunshot threw nothing at {tier:?}");
            assert!(spawned.contains(&want.spatter.id()) && spawned.contains(&want.mist.id()), "at {tier:?} the spray is not the {} set's", if substitute { "substitute" } else { "blood" });
            assert!(!spawned.contains(&not.spatter.id()) && !spawned.contains(&not.mist.id()), "at {tier:?} the other palette was thrown too");
        }
    }
}
