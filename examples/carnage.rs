//! **The whole crate, bleeding.** Hit it, shoot through it, and watch what leaves the wounds.
//!
//! `sever` shows where a subject comes apart. `bullet_holes` shows a channel taken out of one. This
//! shows the layer on top of both: every one of those openings is a [`Wound`], and blood, stains, hit
//! stop and camera shake all read a wound and nothing else. Key `1` and key `6` open geometrically
//! different holes and go through *the same* blood code.
//!
//! ```text
//!   arrows / WASD   move the aim marker
//!   1               a projectile   — nearest fragment, then outward along the bonds
//!   2               a slash        — falloff from the segment a blade travelled
//!   3               a swept blade  — every bond the swing passed through, no falloff
//!   4               a blast        — falloff from a point in open space
//!   5               a pull         — weighted by how squarely each face meets it
//!   6               fire a channel straight through, entering at the marker
//!   R               reset
//! ```
//!
//! # What is the crate's and what is this file's
//!
//! The crate hands over, and applies none of it:
//!
//! - `wounds_from_bonds` — the severances a blow left, as wounds.
//! - `wound_from_ejecta` — the channel a shot left, as a wound.
//! - `spatter::stains` — where the blood lands, deterministically, on the CPU.
//! - `bleed::pulse_wound` — the wound a heartbeat throws, at a falling severity.
//! - `feel::trauma_for` / `hitstop_ticks` / `shake_offset` — numbers.
//!
//! **This file owns every application of them**, and that division is the point of `feel.rs`'s two
//! prohibitions. The camera shake lives in a `Shake` component *this example* drives; the hit stop
//! is this example's own tick counter. The crate writes neither a `Transform` nor `Time<Virtual>`,
//! because in the consuming game those belong to `camera.rs` and to `juice.rs` respectively, and a
//! second writer of either is a frame-to-frame fight.
//!
//! # The tick counter, and why there is one
//!
//! Everything in the deterministic half takes `tick: u32`. This example keeps its own counter and
//! advances it once per frame, which is exactly what a game's `FixedUpdate` would do — so the pulse
//! train, the hit stop and the shake are all functions of a tick rather than of `delta_secs()`.
//!
//! Needs a GPU — and the camera carries `DepthPrepass`, without which the stain decals render as
//! opaque quads or not at all.
//!
//! Run: `cargo run --release --example carnage`

use std::collections::HashSet;

use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy_carnage::{
    Bleed, BondId, CarnagePlugin, CarnageSettings, CarnageVfxPlugin, FragmentId, SplatTextures,
    Wound, WoundKind, Wounded, clotted, hitstop_ticks, largest_cap, pulse_wound, shake_offset,
    spawn_stain, stains, trauma_for, wound_of_channel, wounds_from_bonds,
};

mod common;
use common::body::{self, Blow, BodyMaterials, Chunk, ORIGIN};
use common::light_and_floor;

/// The frontier this demo stands at — index into [`body::GRANULARITIES`].
///
/// The finest, because a fine frontier means a blow takes small pieces off and each one is a wound
/// worth bleeding. At the coarsest, a hit removes a quarter of the body and there is one wound.
const GRANULARITY: usize = 3;

/// Rendered flat, for the reason `bullet_holes.rs` records: relaxing each shard's skin independently
/// opens a hairline along every wedge boundary radiating from a hole, and this demo fires channels.
const SOFTEN: f32 = 0.0;

/// The fixed-tick rate this demo pretends to run at. The shipped [`CarnageSettings`] tick counts are
/// derived for 60 Hz, so claiming anything else here would put the pulse train out of step with the
/// dials it is driven by.
const HZ: u32 = 60;

/// The calibre key `6` fires. Mid-range on `bullet_holes`' own dial.
const CALIBRE: f32 = 0.035;

/// The floor plane stains land on, in **world** space. `light_and_floor` puts the floor at `y = 0`.
const FLOOR_Y: f32 = 0.0;

/// **How many stains may be on the floor at once.**
///
/// A cap rather than a lifetime: a stain that faded would say the blood dried, and blood on a floor
/// does not. The oldest is despawned when a new one would exceed this, which is the same policy the
/// consuming game's own pool ring uses.
const MAX_STAINS: usize = 900;

/// **How much of the paper's measured spatter speed this subject gets, and it is a measurement too.**
///
/// `CarnageSettings` ships this at 1.0 because 8…40 m/s is what Comiskey et al. recorded, and the
/// crate will not quietly divide its own constants. At 1.0 on a 1.8 m subject under the examples'
/// 18 m/s² gravity a droplet thrown straight up rises about 44 metres — measured, and it looks like a
/// fountain leaving frame. At 0.25 the throw is roughly 1–3 metres, which is a body's worth of blood
/// on a floor you can see.
const SPEED_SCALE: f32 = 0.25;

/// Where the next blow lands, in subject-local space.
#[derive(Resource)]
struct Aim(Vec3);

/// Marks the little sphere that shows [`Aim`].
#[derive(Component)]
struct AimMarker;

/// Every channel fired so far. Kept rather than derived, exactly as in `bullet_holes`: each shot
/// re-bakes from the accumulated list, so two overlapping shots make one channel.
#[derive(Resource, Default)]
struct Bores(Vec<bevy_carnage::Bore>);

/// **This example's own fixed-tick counter.** The crate reads no clock; this is what it reads instead.
#[derive(Resource, Default)]
struct Tick(u32);

/// **This example's own hit stop.** Ticks remaining, and the crate never touches it.
///
/// Applied by *skipping this file's own integrator*, not by writing `Time<Virtual>` — which is the
/// prohibition `feel.rs` states, and which exists because the consuming game documents `juice.rs` as
/// the single writer of the virtual clock's relative speed.
#[derive(Resource, Default)]
struct Hitstop(u32);

/// **This example's own camera shake.** The crate returns an offset; this component applies it.
#[derive(Component)]
struct Shake {
    /// Where the camera sits when nothing is shaking.
    rest: Vec3,
    /// The accumulated trauma, decaying linearly — the model `feel.rs` and the game share.
    trauma: f32,
    /// Which way the last wound faced, world space. The "semantically significant direction".
    dir: Vec3,
}

/// A stain entity, so the ring can despawn the oldest.
#[derive(Component)]
struct StainMark;

/// Live stains in the order they were stamped, so the cap evicts the oldest.
#[derive(Resource, Default)]
struct StainRing(Vec<Entity>);

/// Marks the line reporting what the last blow did and what is bleeding.
///
/// **ASCII only**, for the reason `bullet_holes.rs` records: Bevy's default font atlas has neither
/// `·` nor `—`, so both render as missing-glyph boxes.
#[derive(Component)]
struct HudStatus;

/// What to say about the last blow.
#[derive(Resource)]
struct Status(String);

impl Default for Status {
    fn default() -> Self {
        Status("hit it: 1 projectile  2 slash  3 blade  4 blast  5 pull  6 shoot through".into())
    }
}

/// What the last blow produced, for the HUD.
#[derive(Resource, Default)]
struct LastBlow {
    wounds: usize,
    stains: usize,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage — carnage (1-5 hit, 6 shoot through, arrows aim, R reset)"
                    .into(),
                resolution: (960u32, 680u32).into(),
                ..default()
            }),
            ..default()
        }))
        // **The tuned dials go in before the plugin.** `CarnageVfxPlugin` `init_resource`s
        // `CarnageSettings`, which no-ops when it is already present — so a caller that authors its
        // own block inserts it first and its values win. There is no merge and no partial default.
        .insert_resource(CarnageSettings {
            spatter_speed_scale: SPEED_SCALE,
            ..CarnageSettings::default()
        })
        // The deterministic half and the cosmetic half are separate plugins on purpose — a headless
        // harness adds only the first. `CarnageVfxPlugin` is what brings in Hanabi.
        .add_plugins((CarnagePlugin, CarnageVfxPlugin))
        .insert_resource(Aim(Vec3::new(-0.30, 0.16, 0.0)))
        .init_resource::<Bores>()
        // **`body::spawn_gore` and `reset` both read this**, and it is not `init_resource`d by
        // anything in `common::body` — a plug counter is the caller's bookkeeping. Missing, key `6`
        // and key `R` both panic on a missing resource, which is what a smoke test of this example
        // found the first time it was run.
        .init_resource::<body::Thrown>()
        .init_resource::<Tick>()
        .init_resource::<Hitstop>()
        .init_resource::<StainRing>()
        .init_resource::<LastBlow>()
        .init_resource::<Status>()
        // `build_splats` is `CarnageVfxPlugin`'s own `Startup` system — registering it again here
        // would build a second set of splat textures and overwrite the resource with them.
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (advance_tick, aim_marker, strike, bleed_wounds, integrate, camera_shake, hud).chain(),
        )
        .run();
}

fn setup(world: &mut World) {
    let camera = Transform::from_xyz(2.25, 1.35, 2.95).looking_at(Vec3::new(0.0, 0.76, 0.0), Vec3::Y);
    // **`DepthPrepass` is not optional.** A forward decal reconstructs the surface it lies on from
    // the depth buffer; without a prepass the stains render as opaque quads or not at all. That is
    // the first thing to check if the floor looks wrong.
    world.spawn((
        Camera3d::default(),
        DepthPrepass,
        camera,
        Shake { rest: camera.translation, trauma: 0.0, dir: Vec3::X },
    ));
    light_and_floor(world);

    let baked = body::Baked::bake(world, SOFTEN, &[]);
    let materials = BodyMaterials::new(world);
    let damage = body::Damage::fresh(&baked, GRANULARITY);

    let marker = world.resource_mut::<Assets<Mesh>>().add(Mesh::from(Sphere::new(0.05)));
    world.spawn((
        AimMarker,
        Mesh3d(marker),
        MeshMaterial3d(materials.aim.clone()),
        Transform::from_translation(ORIGIN),
    ));

    world.insert_resource(baked);
    world.insert_resource(materials);
    world.insert_resource(damage);
    body::stand(world, GRANULARITY);
    spawn_hud(world);
}

/// **An on-screen legend, because without one the feature set is invisible** — the lesson AG-023
/// recorded against `sever`, which applies here twice over since this demo has more on keys.
fn spawn_hud(world: &mut World) {
    world.spawn((
        Text::new(
            "arrows / WASD  aim\n             1 projectile   2 slash   3 blade   4 blast   5 pull\n             6 shoot through   R reset",
        ),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        Node { position_type: PositionType::Absolute, top: px(12), left: px(14), ..default() },
    ));
    world.spawn((
        HudStatus,
        Text::new(""),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(1.0, 0.92, 0.55, 0.95)),
        Node { position_type: PositionType::Absolute, bottom: px(14), left: px(14), ..default() },
    ));
}

/// One tick per frame, and the hit stop counts down on the same tick.
///
/// **This is the demo's whole clock discipline.** A game would advance this in `FixedUpdate`; the
/// point is only that the crate is handed an integer it did not read from anywhere.
fn advance_tick(mut tick: ResMut<Tick>, mut hitstop: ResMut<Hitstop>) {
    tick.0 = tick.0.wrapping_add(1);
    hitstop.0 = hitstop.0.saturating_sub(1);
}

/// Move the aim marker, and keep the sphere on it. Same clamp as `sever`'s.
fn aim_marker(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut aim: ResMut<Aim>,
    mut marker: Query<&mut Transform, With<AimMarker>>,
) {
    let step = 1.1 * time.delta_secs();
    let mut d = Vec3::ZERO;
    for (key, delta) in [
        (KeyCode::ArrowUp, Vec3::Y),
        (KeyCode::KeyW, Vec3::Y),
        (KeyCode::ArrowDown, -Vec3::Y),
        (KeyCode::KeyS, -Vec3::Y),
        (KeyCode::ArrowLeft, -Vec3::X),
        (KeyCode::KeyA, -Vec3::X),
        (KeyCode::ArrowRight, Vec3::X),
        (KeyCode::KeyD, Vec3::X),
    ] {
        if keys.pressed(key) {
            d += delta;
        }
    }
    aim.0 += d * step;
    aim.0 = aim.0.clamp(Vec3::new(-0.8, -0.7, -0.6), Vec3::new(0.8, 1.2, 0.6));
    for mut t in &mut marker {
        t.translation = ORIGIN + aim.0;
    }
}

/// Read the keyboard, land the blow, and turn what it opened into carnage.
///
/// Exclusive, because both `body::strike` and `body::Baked::bake` work on `&mut World` — the shape
/// the headless recorder can drive too, which is what keeps `docs/carnage.gif` honest.
fn strike(world: &mut World) {
    let pressed =
        |world: &World, key: KeyCode| world.resource::<ButtonInput<KeyCode>>().just_pressed(key);

    if pressed(world, KeyCode::KeyR) {
        reset(world);
        return;
    }

    if pressed(world, KeyCode::Digit6) {
        shoot_through(world);
        return;
    }

    let blow = [
        (KeyCode::Digit1, Blow::Projectile),
        (KeyCode::Digit2, Blow::Slash),
        (KeyCode::Digit3, Blow::SweptBlade),
        (KeyCode::Digit4, Blow::Blast),
        (KeyCode::Digit5, Blow::Pull),
    ]
    .into_iter()
    .find(|(key, _)| pressed(world, *key))
    .map(|(_, blow)| blow);

    let Some(blow) = blow else { return };
    let at = world.resource::<Aim>().0;

    // **Snapshot the severed set, strike, diff.** `body::strike` reports counts, not identities, and
    // what a wound needs is which bonds went — so the diff is taken here rather than by widening the
    // shared harness for one caller.
    let before: HashSet<BondId> = world.resource::<body::Damage>().broken.iter().collect();
    let out = body::strike(world, blow, at);
    let newly: Vec<BondId> = {
        let damage = world.resource::<body::Damage>();
        damage.broken.iter().filter(|id| !before.contains(id)).collect()
    };

    let wounds = {
        let damage = world.resource::<body::Damage>();
        wounds_from_bonds(&damage.bonds, &newly)
    };
    let stamped = open_wounds(world, &wounds);
    attach_bleeds(world);

    let said = match (out.newly, out.off) {
        (0, _) if out.reached == 0 => {
            format!("{}: landed on nothing - the aim is off the body", blow.label())
        }
        (0, _) => format!(
            "{}: nothing left to break here. Move the aim (arrows) or reset (R)",
            blow.label()
        ),
        (n, off) => format!(
            "{}: {n} bond(s) severed, {off} came off, {} wound(s) bleeding",
            blow.label(),
            wounds.len()
        ),
    };
    world.resource_mut::<Status>().0 = said;
    let mut last = world.resource_mut::<LastBlow>();
    last.wounds = wounds.len();
    last.stains = stamped;
}

/// **Key `6`: a channel through the subject, and its interior bleeds.**
///
/// The demo that proves bullet holes and blood are one system. The bore re-bakes (a channel is a bake
/// input, not damage applied afterwards), and the wound comes from
/// [`wound_from_ejecta`](bevy_carnage::wound_from_ejecta) — whose area is the *channel wall*, which
/// `ProxyCell::face_is_cut` reports as open because a bore wall is raw interior.
fn shoot_through(world: &mut World) {
    let at = world.resource::<Aim>().0;
    let bore = body::bore_at(at, CALIBRE, 6);
    world.resource_mut::<Bores>().0.push(bore);
    let bores = world.resource::<Bores>().0.clone();

    body::clear(world);
    let baked = body::Baked::bake(world, SOFTEN, &bores);
    let damage = body::Damage::fresh(&baked, GRANULARITY);
    world.insert_resource(baked);
    world.insert_resource(damage);
    body::stand(world, GRANULARITY);
    body::spawn_gore(world);

    // Every plug the accumulated bores ejected is a channel wound. Only the newest shot's plugs are
    // new, but re-deriving all of them is harmless: the derivation is a pure function of geometry.
    //
    // **`wound_of_channel` off the plug's own cell**, not an area guessed from its volume. The wound
    // is the channel *wall* — the plug's raw-interior faces — and `GorePart` keeps the cell precisely
    // so this is the real number.
    let wounds: Vec<Wound> = {
        let baked = world.resource::<body::Baked>();
        baked
            .gore
            .iter()
            .map(|g| wound_of_channel(&g.cell, g.exit, g.direction))
            .collect()
    };
    let stamped = open_wounds(world, &wounds);
    attach_bleeds(world);

    let holes = bores.len();
    world.resource_mut::<Status>().0 =
        format!("shot through: {holes} hole(s), {} channel wound(s) bleeding", wounds.len());
    let mut last = world.resource_mut::<LastBlow>();
    last.wounds = wounds.len();
    last.stains = stamped;
}

/// **Everything a wound causes, in one place.** Announce it, stain the floor, and feed the two feel
/// numbers into this example's own accumulators.
///
/// Returns how many stains were stamped. The wound is converted to world space through the subject's
/// own offset — [`Wound::to_world`] is the method for a real `GlobalTransform`, and this demo's
/// subject is a translation, so the two agree.
fn open_wounds(world: &mut World, wounds: &[Wound]) -> usize {
    if wounds.is_empty() {
        return 0;
    }
    let settings = world.resource::<CarnageSettings>().clone();

    // The two feel numbers, accumulated before anything is spawned so the borrows are done with.
    let (mut trauma, mut stop_ticks, mut dir) = (0.0f32, 0u32, Vec3::X);
    for w in wounds {
        trauma += trauma_for(w, &settings);
        stop_ticks = stop_ticks.max(hitstop_ticks(w, HZ, &settings));
        dir = w.normal;
    }

    // Stains, on the CPU, deterministically. **Core, not `vfx`** — where blood lands is read by
    // simulation on the consuming side, so this half exists with the render feature off.
    let mut world_stains = Vec::new();
    for w in wounds {
        let world_wound = Wound { at: ORIGIN + w.at, ..*w };
        world_stains.extend(stains(&world_wound, &settings, FLOOR_Y));
    }

    // The message the particle half reads. Written here rather than by the crate: the crate does not
    // decide when a wound happens.
    world.resource_mut::<Messages<Wounded>>().write_batch(wounds.iter().map(|w| Wounded {
        at: ORIGIN + w.at,
        normal: w.normal,
        area: w.area,
        severity: w.severity,
        kind: w.kind,
    }));

    let stamped = world_stains.len();
    let Some(splats) = world.remove_resource::<SplatTextures>() else {
        // The splats are built on `Startup`; a blow in the same frame arrives before them.
        return 0;
    };
    let mut spawned = Vec::with_capacity(stamped);
    world.commands();
    {
        let mut commands = world.commands();
        for stain in &world_stains {
            let entity = spawn_stain(&mut commands, &splats, stain);
            spawned.push(entity);
        }
    }
    world.flush();
    for entity in &spawned {
        world.entity_mut(*entity).insert(StainMark);
    }
    world.insert_resource(splats);

    // The ring: oldest out when the cap is exceeded. A stain that faded would say blood dries.
    {
        let mut ring = world.remove_resource::<StainRing>().unwrap_or_default();
        ring.0.extend(spawned);
        let excess = ring.0.len().saturating_sub(MAX_STAINS);
        let evicted: Vec<Entity> = ring.0.drain(..excess).collect();
        world.insert_resource(ring);
        for entity in evicted {
            if let Ok(e) = world.get_entity_mut(entity) {
                e.despawn();
            }
        }
    }

    // Feel, applied by this example and by nothing in the crate.
    let mut hitstop = world.resource_mut::<Hitstop>();
    hitstop.0 = hitstop.0.max(stop_ticks);
    let mut shakes = world.query::<&mut Shake>();
    for mut shake in shakes.iter_mut(world) {
        shake.trauma = (shake.trauma + trauma).clamp(0.0, 1.0);
        shake.dir = dir;
    }

    stamped
}

/// **The cut face a chunk bled from, in the chunk's own local space.**
///
/// Alongside [`Bleed`], which carries *when* and *how much* but deliberately not *where*: a wound's
/// position and direction are geometry the caller already holds, and putting them in the crate's
/// component would make it a scene graph rather than a schedule.
#[derive(Component)]
struct ChunkWound(Wound);

/// **Give every detached chunk that has none a bleed schedule and the wound it bleeds from**, opened
/// at the current tick.
///
/// The wound is [`largest_cap`] of **that chunk's own** convex cell — the widest raw-interior face it
/// came away with, which is exactly where a severed piece bleeds. `Chunk` carries its `FragmentId` so
/// this can be looked up.
///
/// **Two invented numbers were removed to get here, and both showed on screen.** The area started as
/// the mean cut-face area over every part, which made a fingertip bleed like a torso. The normal
/// started as local `+Y`, which made every resting gib fountain straight up regardless of which way
/// its wound faced. The cell was available for both.
///
/// A chunk with no cut face gets no `Bleed`: a plug was never part of the frontier, and a piece with
/// no raw interior has no wound.
fn attach_bleeds(world: &mut World) {
    let tick = world.resource::<Tick>().0;
    let fresh: Vec<(Entity, Wound)> = {
        let mut q = world.query_filtered::<(Entity, &Chunk), Without<Bleed>>();
        let candidates: Vec<(Entity, Option<FragmentId>)> =
            q.iter(world).map(|(e, c)| (e, c.fragment)).collect();
        let baked = world.resource::<body::Baked>();
        candidates
            .into_iter()
            .filter_map(|(e, id)| {
                let part = baked.parts.get(id?.index())?.as_ref()?;
                let cap = largest_cap(&part.cell)?;
                // The cell is subject-local and the chunk's entity sits at its own centre, so the
                // wound's offset within the chunk is the cap's centroid minus that centre.
                Some((
                    e,
                    Wound {
                        at: cap.centroid - part.center_local,
                        normal: cap.normal,
                        area: cap.area,
                        severity: 1.0,
                        kind: WoundKind::Severance,
                    },
                ))
            })
            .collect()
    };
    for (entity, wound) in fresh {
        world.entity_mut(entity).insert((Bleed::new(tick, wound.area), ChunkWound(wound)));
    }
}

/// **Every bleeding fragment, pulsing.** One heartbeat at a time, at a falling severity, until it
/// clots — and the same spatter model serves the first jet and the last seep.
fn bleed_wounds(
    mut commands: Commands,
    tick: Res<Tick>,
    settings: Res<CarnageSettings>,
    splats: Option<Res<SplatTextures>>,
    mut ring: ResMut<StainRing>,
    mut wounded: MessageWriter<Wounded>,
    bleeding: Query<(Entity, &Bleed, &ChunkWound, &GlobalTransform)>,
) {
    let t = tick.0;
    for (entity, bleed, wound, xf) in &bleeding {
        if clotted(bleed, t, HZ, &settings) {
            // A clotted wound stops being a wound. Removing the components is what makes "once
            // clotted, never again" true of the scene as well as of the arithmetic.
            commands.entity(entity).remove::<Bleed>();
            commands.entity(entity).remove::<ChunkWound>();
            continue;
        }
        // **The chunk's own cut face**, carried since it detached and rotated into world space by the
        // chunk's transform — so blood leaves the wound the way the wound is facing, and a tumbling
        // gib's spray tumbles with it.
        let Some(pulse) = pulse_wound(bleed, &wound.0, t, HZ, &settings) else { continue };
        let world_wound = pulse.to_world(xf);
        wounded.write(world_wound);

        if let Some(splats) = splats.as_ref() {
            let cpu = Wound {
                at: world_wound.at,
                normal: world_wound.normal,
                area: world_wound.area,
                severity: world_wound.severity,
                kind: world_wound.kind,
            };
            for stain in stains(&cpu, &settings, FLOOR_Y) {
                let e = spawn_stain(&mut commands, splats, &stain);
                commands.entity(e).insert(StainMark);
                ring.0.push(e);
            }
        }
    }
    let excess = ring.0.len().saturating_sub(MAX_STAINS);
    for entity in ring.0.drain(..excess) {
        commands.entity(entity).try_despawn();
    }
}

/// **The hit stop, applied by skipping this example's own integrator.**
///
/// Not by writing `Time<Virtual>` — see the module docs. That is the one thing `feel.rs` forbids and
/// the reason it hands back a tick count instead of applying one.
fn integrate(
    time: Res<Time>,
    hitstop: Res<Hitstop>,
    mut chunks: Query<(&mut Chunk, &mut Transform)>,
) {
    if hitstop.0 > 0 {
        return;
    }
    let dt = time.delta_secs() * 0.55;
    if dt <= 0.0 {
        return;
    }
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, dt);
    }
}

/// **The camera, shaken by this example.** The crate returned a vector; this moves the camera.
fn camera_shake(tick: Res<Tick>, settings: Res<CarnageSettings>, mut q: Query<(&mut Shake, &mut Transform)>) {
    for (mut shake, mut transform) in &mut q {
        // Linear decay over the shake period — Eiserloh's model, which `feel.rs` and the consuming
        // game's `juice.rs` both cite, so all three agree.
        shake.trauma = (shake.trauma - 1.0 / settings.shake_ticks.max(1) as f32).max(0.0);
        let offset = shake_offset(shake.trauma, shake.dir, tick.0, &settings);
        transform.translation = shake.rest + offset;
    }
}

/// Keep the status line current.
fn hud(
    status: Res<Status>,
    last: Res<LastBlow>,
    ring: Res<StainRing>,
    hitstop: Res<Hitstop>,
    standing: Query<(), With<body::Attached>>,
    bleeding: Query<(), With<Bleed>>,
    mut line: Query<&mut Text, With<HudStatus>>,
) {
    let text = format!(
        "{}\n{} standing  |  {} wound(s) last blow  |  {} stain(s) on the floor  |  {} bleeding{}",
        status.0,
        standing.iter().count(),
        last.wounds,
        ring.0.len(),
        bleeding.iter().count(),
        if hitstop.0 > 0 { format!("  |  hitstop {}", hitstop.0) } else { String::new() },
    );
    for mut t in &mut line {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}

/// Back to an intact, unbored, unstained subject.
fn reset(world: &mut World) {
    world.resource_mut::<Bores>().0.clear();
    body::clear(world);
    body::wipe(world);
    world.resource_mut::<body::Thrown>().0 = 0;

    let baked = body::Baked::bake(world, SOFTEN, &[]);
    let damage = body::Damage::fresh(&baked, GRANULARITY);
    world.insert_resource(baked);
    world.insert_resource(damage);
    body::stand(world, GRANULARITY);

    let stains: Vec<Entity> =
        world.query_filtered::<Entity, With<StainMark>>().iter(world).collect();
    for entity in stains {
        if let Ok(e) = world.get_entity_mut(entity) {
            e.despawn();
        }
    }
    world.resource_mut::<StainRing>().0.clear();
    world.resource_mut::<LastBlow>().wounds = 0;
    world.resource_mut::<Status>().0 = "reset - an intact, unstained subject".into();
    for mut shake in world.query::<&mut Shake>().iter_mut(world) {
        shake.trauma = 0.0;
    }
}
