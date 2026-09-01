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
use bevy_hanabi::{
    AccelModifier, Attribute, ColorOverLifetimeModifier, EffectAsset, EffectSpawner, ExprWriter,
    Gradient, HanabiPlugin, KillAabbModifier, LinearDragModifier, OrientMode, OrientModifier,
    ParticleEffect, ScalarType, SetAttributeModifier, SetPositionCone3dModifier,
    SetVelocitySphereModifier, ShapeDimension, SimulationCondition, SimulationSpace, SpawnerSettings,
};

use crate::spatter::{BACK_SPATTER_SPEED, FORWARD_SPATTER_SPEED, wound_seed};
use crate::wound::{Wound, WoundKind};
use crate::{CarnageSettings, Wounded};

/// The five blood effects, built once at startup.
///
/// Five assets rather than one parameterised asset because a particle effect's **capacity and spawner
/// are baked into the asset** and cannot be changed per instance — so "a burst of 300" and "a steady
/// trickle" are necessarily different assets, not different settings on one.
#[derive(Resource, Debug, Clone)]
pub struct CarnageEffects {
    /// The impact spray: many droplets, one burst, world space.
    pub spatter: Handle<EffectAsset>,
    /// The fine mist that hangs where the round went through.
    pub mist: Handle<EffectAsset>,
    /// One jet per heartbeat, for a wound that is still pumping.
    pub spurt: Handle<EffectAsset>,
    /// The steady seep of a wound that has stopped pumping, in local space so it rides the fragment.
    pub seep: Handle<EffectAsset>,
    /// The ribbon a flying gib leaves behind it.
    pub trail: Handle<EffectAsset>,
}

/// **How long an effect instance may live after its spawner finishes**, in this crate's own ticks.
///
/// Not a Hanabi type — 0.19 has none. See the module docs for why one condition is not enough.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectTtl(pub u32);

/// Blood, as a colour ramp: arterial red fading to a darker, transparent clot.
///
/// One gradient shared by four of the five effects, because blood is blood — the difference between a
/// spurt and a seep is its rate and its speed, not its colour, and two nearly-identical gradients
/// would drift apart the first time one was tweaked.
fn blood_gradient() -> Gradient<Vec4> {
    let mut g = Gradient::new();
    g.add_key(0.0, Vec4::new(0.62, 0.05, 0.05, 1.0));
    g.add_key(0.55, Vec4::new(0.40, 0.02, 0.02, 0.95));
    g.add_key(1.0, Vec4::new(0.18, 0.01, 0.01, 0.0));
    g
}

/// The shared skeleton of every blood effect: a Y-axis cone of positions, a sphere of velocities
/// fanned out through them, a randomised lifetime, gravity, drag, a kill volume, and blood-coloured
/// billboards oriented along their own velocity.
///
/// **One builder, five callers.** The five effects genuinely differ in spawner, capacity, simulation
/// space and a handful of numbers; everything else was identical five times over in the first draft,
/// and five copies of a modifier stack is five places for a look fix to be applied four times.
struct BloodEffect {
    name: &'static str,
    /// Cone half-width at the emitter, metres.
    base_radius: f32,
    /// Cone height, metres — how far ahead of the wound the droplets start.
    height: f32,
    /// Speed range, m/s.
    speed: [f32; 2],
    /// Lifetime range, seconds.
    lifetime: [f32; 2],
    /// Drag multiplier over the settings' own dial.
    drag_scale: f32,
    /// Billboard size, metres.
    size: f32,
    space: SimulationSpace,
    condition: SimulationCondition,
    spawner: SpawnerSettings,
}

impl BloodEffect {
    fn build(self, s: &CarnageSettings) -> EffectAsset {
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

        // Shrink over life, from each droplet's own starting size: `size = F32_0 * (1 - age/lifetime)`
        // clamped at zero, so a droplet dwindles instead of blinking out and every droplet keeps its
        // own scale while doing it.
        let update_size = SetAttributeModifier::new(
            Attribute::SIZE,
            writer
                .attr(Attribute::F32_0)
                .mul(
                    writer
                        .lit(1.0)
                        .sub(writer.attr(Attribute::AGE).div(writer.attr(Attribute::LIFETIME)))
                        .max(writer.lit(0.0)),
                )
                .expr(),
        );

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
        let gravity = AccelModifier::constant(&mut module, Vec3::NEG_Y * s.gravity);
        let drag = LinearDragModifier::constant(&mut module, s.drag * self.drag_scale);
        let kill = KillAabbModifier::new(kill_center, kill_half);

        EffectAsset::new(s.effect_capacity, self.spawner, module)
            .with_name(self.name)
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
            .render(ColorOverLifetimeModifier::new(blood_gradient()))
            // Along velocity, so a droplet reads as a streak in the direction it is travelling
            // rather than as a sphere — which is most of what makes a spray look fast.
            .render(OrientModifier::new(OrientMode::AlongVelocity))
    }
}

/// **The impact spray.** One burst, the full measured speed span, world space so it detaches from the
/// body that threw it.
///
/// `SpawnerSettings::once(1.0)` is a placeholder count: the per-instance [`EffectSpawner`] the spawn
/// system inserts overrides it with the wound's own [`crate::droplet_count`], which is the only place
/// that number can come from because it depends on the wound's area.
pub fn spatter_burst(s: &CarnageSettings) -> EffectAsset {
    BloodEffect {
        name: "carnage:spatter",
        base_radius: 0.03,
        height: 0.06,
        speed: [BACK_SPATTER_SPEED, FORWARD_SPATTER_SPEED],
        lifetime: [0.35, 0.85],
        drag_scale: 1.0,
        size: 0.022,
        space: SimulationSpace::Global,
        condition: SimulationCondition::WhenVisible,
        spawner: SpawnerSettings::once(1.0.into()),
    }
    .build(s)
}

/// **The mist.** The fine fraction that leaves fastest and stops almost immediately — the paper's
/// forward spatter at full speed against six times the drag, which is what makes a puff rather than a
/// spray.
pub fn mist_puff(s: &CarnageSettings) -> EffectAsset {
    BloodEffect {
        name: "carnage:mist",
        base_radius: 0.05,
        height: 0.04,
        speed: [FORWARD_SPATTER_SPEED, FORWARD_SPATTER_SPEED],
        lifetime: [0.10, 0.22],
        drag_scale: 6.0,
        size: 0.010,
        space: SimulationSpace::Global,
        condition: SimulationCondition::WhenVisible,
        spawner: SpawnerSettings::once(1.0.into()),
    }
    .build(s)
}

/// **The arterial jet.** One burst per heartbeat, at the wound's own rate.
///
/// `SimulationCondition::Always` because an off-screen body must keep bleeding: a wound that pauses
/// while the camera looks away and resumes when it looks back would be visibly wrong the moment the
/// camera came back to a corpse that had bled for no time at all.
pub fn arterial_spurt(s: &CarnageSettings) -> EffectAsset {
    let period = if s.spurt_bpm > 0.0 { 60.0 / s.spurt_bpm } else { 1.0 };
    BloodEffect {
        name: "carnage:spurt",
        base_radius: 0.015,
        height: 0.05,
        speed: [FORWARD_SPATTER_SPEED * 0.35, FORWARD_SPATTER_SPEED * 0.6],
        lifetime: [0.45, 0.95],
        drag_scale: 0.8,
        size: 0.026,
        space: SimulationSpace::Global,
        condition: SimulationCondition::Always,
        spawner: SpawnerSettings::burst(24.0.into(), period.into()),
    }
    .build(s)
}

/// **The seep.** A slow steady rate in **local** space, so it rides the fragment it is attached to
/// instead of being left behind as the chunk tumbles.
pub fn wound_seep(s: &CarnageSettings) -> EffectAsset {
    BloodEffect {
        name: "carnage:seep",
        base_radius: 0.012,
        height: 0.02,
        speed: [0.15, 0.6],
        lifetime: [0.5, 1.1],
        drag_scale: 2.0,
        size: 0.014,
        space: SimulationSpace::Local,
        condition: SimulationCondition::Always,
        spawner: SpawnerSettings::rate(26.0.into()),
    }
    .build(s)
}

/// **The trail behind a flying gib.** A steady rate in **global** space — that is precisely what makes
/// it a trail: each droplet is left at the world position the gib had when it spawned, so the emitter
/// moving away from them draws the line.
///
/// **Deliberately not a `RIBBON_ID` ribbon.** Hanabi supports one ribbon chain per effect asset, so a
/// single ribbon asset cannot serve several simultaneous gibs — every gib would be threaded onto one
/// strand. Independent droplets in global space cost one asset and work for any number.
pub fn gib_trail(s: &CarnageSettings) -> EffectAsset {
    BloodEffect {
        name: "carnage:trail",
        base_radius: 0.02,
        height: 0.02,
        speed: [0.2, 1.2],
        lifetime: [0.30, 0.70],
        drag_scale: 1.5,
        size: 0.012,
        space: SimulationSpace::Global,
        condition: SimulationCondition::WhenVisible,
        spawner: SpawnerSettings::rate(60.0.into()),
    }
    .build(s)
}

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
            // **Both halves of the cosmetic layer, registered here.** The splat textures back
            // [`crate::spawn_stain`], so a caller that added this plugin and then had to remember a
            // second registration would get silently stain-free blood — which is exactly the failure
            // that put this line here.
            .add_systems(Startup, (build_effects, crate::decal::build_splats))
            .add_systems(
                Update,
                (spawn_wound_effects, despawn_finished_effects).in_set(CarnageVfxSystems),
            );
    }
}

/// Build the five assets once, from whatever [`CarnageSettings`] is present at startup.
fn build_effects(
    mut commands: Commands,
    mut assets: ResMut<Assets<EffectAsset>>,
    settings: Res<CarnageSettings>,
) {
    let s = &*settings;
    commands.insert_resource(CarnageEffects {
        spatter: assets.add(spatter_burst(s)),
        mist: assets.add(mist_puff(s)),
        spurt: assets.add(arterial_spurt(s)),
        seep: assets.add(wound_seep(s)),
        trail: assets.add(gib_trail(s)),
    });
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
fn spawn_wound_effects(
    mut commands: Commands,
    mut wounded: MessageReader<Wounded>,
    effects: Option<Res<CarnageEffects>>,
    settings: Res<CarnageSettings>,
) {
    let Some(effects) = effects else {
        // The assets are built on `Startup`; a message in the same frame arrives before them. Warning
        // per message on a hot path would be worse than the one frame of missing blood.
        return;
    };
    for w in wounded.read() {
        let wound = Wound {
            at: w.at,
            normal: w.normal,
            area: w.area,
            severity: w.severity,
            kind: w.kind,
        };
        let count = crate::spatter::droplet_count(&wound, &settings);
        if count == 0 {
            continue;
        }
        let seed = wound_seed(&wound);
        let rotation = Quat::from_rotation_arc(Vec3::Y, w.normal.normalize_or_zero());
        let transform = Transform { translation: w.at, rotation, scale: Vec3::ONE };

        // A channel mists as well as sprays — that contrast is what makes a gunshot read differently
        // from a cut, and it is the only place the wound kind changes what is drawn.
        let handles: &[(&Handle<EffectAsset>, u32)] = match w.kind {
            WoundKind::Severance => &[(&effects.spatter, count)],
            WoundKind::Channel => &[(&effects.spatter, count), (&effects.mist, count / 2)],
        };
        for (handle, n) in handles.iter().copied() {
            if n == 0 {
                continue;
            }
            commands.spawn((
                ParticleEffect { handle: handle.clone(), prng_seed: Some(seed) },
                EffectSpawner::new(&SpawnerSettings::once((n as f32).into())),
                transform,
                EffectTtl(TTL_TICKS),
            ));
        }
    }
}

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
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every effect must actually build**, and its capacity must be the authored dial rather than a
    /// literal — a capacity baked wrong cannot be raised afterwards, which is why it is a setting.
    #[test]
    fn the_five_effects_build_at_the_authored_capacity() {
        let s = CarnageSettings::default();
        let built = [
            ("spatter", spatter_burst(&s)),
            ("mist", mist_puff(&s)),
            ("spurt", arterial_spurt(&s)),
            ("seep", wound_seep(&s)),
            ("trail", gib_trail(&s)),
        ];
        for (what, asset) in &built {
            assert_eq!(
                asset.capacity(),
                s.effect_capacity,
                "{what} was built at a capacity other than the dial"
            );
            assert!(asset.name.contains("carnage:"), "{what} is missing its namespaced name");
        }
        let mut names: Vec<&str> = built.iter().map(|(_, a)| a.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 5, "two effects share a name, so they are not five effects");
    }

    /// The five differ in the ways that make them five effects rather than one asset spawned five
    /// times — which is worth asserting because the shared builder makes accidental sameness easy.
    #[test]
    fn the_effects_differ_where_they_must() {
        let s = CarnageSettings::default();
        assert_eq!(
            wound_seep(&s).simulation_space,
            SimulationSpace::Local,
            "the seep must ride the fragment it is on"
        );
        assert_eq!(
            gib_trail(&s).simulation_space,
            SimulationSpace::Global,
            "a trail is global space — that is what leaves the droplets behind"
        );
        assert_eq!(
            arterial_spurt(&s).simulation_condition,
            SimulationCondition::Always,
            "an off-screen body must keep bleeding"
        );
        assert_eq!(
            wound_seep(&s).simulation_condition,
            SimulationCondition::Always,
            "and so must keep seeping"
        );
    }

    /// The heartbeat period the spurt asset is built with must agree with the CPU schedule's own, or
    /// the visible jets and the deterministic pulses drift apart.
    #[test]
    fn the_spurt_period_matches_the_bleed_schedule() {
        let s = CarnageSettings::default();
        let asset_period = 60.0 / s.spurt_bpm;
        let schedule_period = crate::bleed::pulse_period(60, &s) as f32 / 60.0;
        assert!(
            (asset_period - schedule_period).abs() < 0.02,
            "the spurt asset pulses every {asset_period:.4}s but the schedule every \
             {schedule_period:.4}s — the blood and the model would disagree"
        );
    }
}
