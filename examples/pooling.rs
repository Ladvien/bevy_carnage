//! **Blood that stops being drops and becomes a puddle.**
//!
//! Shoot the subject in the same place repeatedly. The first channel throws a handful of plugs that
//! land as discrete stains; by the third the region under the body is **one** slick whose radius is
//! still visibly growing, rather than a pile of overlapping circles.
//!
//! ```text
//!   arrows / WASD   move the aim marker
//!   space           shoot a channel through the subject at the aim point
//!   R               reset — the floor too
//! ```
//!
//! # What changed, and why it is not `vfx`
//!
//! `spatter::stains` returns independent [`Stain`](bevy_carnage::Stain)s that never interact, and the
//! version of `common::body` this replaced spawned one scaled disc per landed plug and never merged —
//! so a dozen plugs landing together left a dozen coincident circles. [`bevy_carnage::absorb`] is the
//! fold that turns them into a slick, and it lives in the crate's **core** half rather than behind
//! `vfx`: in the consuming game, where blood pools is read as a chemoattractant, so it has to be
//! deterministic and available headless. Only the drawing is optional.
//!
//! Needs a GPU — and the camera carries `DepthPrepass`, without which every slick renders as an
//! opaque quad or not at all.
//!
//! Run: `cargo run --release -p bevy_carnage --example pooling`

use bevy::prelude::*;
use bevy_carnage::{CarnageSettings, CarnageVfxPlugin, PoolDecal};

mod common;
use common::body::{self, Chunk, ORIGIN};
use common::light_and_floor;

/// The frontier the subject stands at — coarse, because this demo is about the floor rather than the
/// body, and a body that falls apart takes the aim point with it.
const GRANULARITY: usize = 0;
/// **Rendered flat, because this demo fires channels** — the same measurement `bullet_holes.rs`,
/// `capture_holes.rs` and `carnage.rs` all record, and this was the one bore-firing example that
/// still carried the other value.
///
/// `soften` relaxes each fragment's drawn skin *independently* and does not pin the boundary it
/// shares with its neighbour, so on a bored subject the wedges around a channel pull apart. At 0.5,
/// captured offscreen: the eight shards of the hole separate into a pinwheel of red slices radiating
/// from the entry wound — reported as "a spiral pattern, like a nautilus shell". At `0.0` the shards
/// share their boundary vertices exactly and the only opening is the bore.
///
/// **The gore is still rounded**, on `CutSettings::ejecta_soften`, which `Baked::bake` leaves at its
/// shipped 0.55 — and the plugs are what this demo exists to watch land. Debris shares a boundary
/// with nothing, so nothing can open up beside it.
const SOFTEN: f32 = 0.0;
/// The channel's radius. Wide enough that each shot throws several plugs worth pooling.
const CALIBRE: f32 = 0.05;
/// How many shards the plug shatters into — more plugs, more stains landing near each other.
const SHATTER: u32 = 6;

/// Where the next channel goes, in subject-local space.
#[derive(Resource)]
struct Aim(Vec3);

/// Marks the little sphere that shows [`Aim`].
#[derive(Component)]
struct AimMarker;

/// The channels bored so far. A bore is a bake input, so each shot re-bakes with the whole list.
#[derive(Resource, Default)]
struct Bores(Vec<bevy_carnage::Bore>);

/// Marks the line reporting stains against slicks.
#[derive(Component)]
struct HudStatus;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage — pooling (space to shoot, arrows to aim, R reset)".into(),
                resolution: (960u32, 680u32).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(CarnageVfxPlugin)
        .insert_resource(Aim(Vec3::new(0.0, 0.10, 0.0)))
        .init_resource::<Bores>()
        .init_resource::<body::Thrown>()
        .init_resource::<body::Pools>()
        .add_systems(Startup, setup)
        .add_systems(Update, (aim_marker, shoot, integrate, body::bleed, hud).chain())
        .run();
}

fn setup(world: &mut World) {
    // Tilted down harder than the other body demos: the floor is the subject here.
    let camera = Transform::from_xyz(1.55, 1.55, 2.10).looking_at(ORIGIN - Vec3::Y * 0.70, Vec3::Y);
    world.spawn((Camera3d::default(), bevy::core_pipeline::prepass::DepthPrepass, camera));
    light_and_floor(world);

    let baked = body::Baked::bake(world, SOFTEN, &[], &[GRANULARITY]);
    let materials = body::BodyMaterials::new(world);
    let damage = body::Damage::fresh(&baked, GRANULARITY);

    let marker = world.resource_mut::<Assets<Mesh>>().add(Mesh::from(Sphere::new(0.04)));
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

    world.spawn((
        Text::new("arrows / WASD  aim\n             space shoot   R reset"),
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

/// **The number that is the whole demo**: plugs thrown against slicks on the floor. Shoot the same
/// spot three times and the first climbs while the second stops — that gap is the merge working.
fn hud(
    pools: Res<body::Pools>,
    settings: Res<CarnageSettings>,
    thrown: Res<body::Thrown>,
    decals: Query<(), With<PoolDecal>>,
    mut line: Query<&mut Text, With<HudStatus>>,
) {
    let widest = pools.0.iter().map(|p| p.radius).fold(0.0f32, f32::max);
    let text = format!(
        "{} plug(s) thrown  ->  {} slick(s) of {} max, widest {widest:.3} m  |  merge radius {:.2} m",
        thrown.0,
        decals.iter().count(),
        settings.blood.max_pools,
        settings.blood.pool_merge_radius,
    );
    for mut t in &mut line {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}

/// Move the aim marker, and keep the sphere on it.
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
    aim.0 = aim.0.clamp(Vec3::new(-0.4, -0.5, -0.4), Vec3::new(0.4, 0.6, 0.4));
    for mut t in &mut marker {
        t.translation = ORIGIN + aim.0;
    }
}

/// Bore a channel at the aim point, or reset.
///
/// **Every shot re-bakes**, because a bore is a bake input: the channel is part of the subject's
/// shape, so a new hole is a new subject rather than an edit to this one. The same sequence
/// `capture_holes` performs.
fn shoot(world: &mut World) {
    let pressed =
        |world: &World, key: KeyCode| world.resource::<ButtonInput<KeyCode>>().just_pressed(key);

    if pressed(world, KeyCode::KeyR) {
        world.resource_mut::<Bores>().0.clear();
        world.resource_mut::<body::Thrown>().0 = 0;
        // `wipe` clears the pool list as well as the decals — a reset that left the blood behind
        // would say the subject had been shot when it had not.
        body::wipe(world);
        rebake(world);
        return;
    }
    if !pressed(world, KeyCode::Space) {
        return;
    }
    let at = world.resource::<Aim>().0;
    let bore = body::bore_at(at, CALIBRE, SHATTER);
    world.resource_mut::<Bores>().0.push(bore);
    rebake(world);
}

fn rebake(world: &mut World) {
    let bores = world.resource::<Bores>().0.clone();
    body::clear(world);
    let baked = body::Baked::bake(world, SOFTEN, &bores, &[GRANULARITY]);
    let damage = body::Damage::fresh(&baked, GRANULARITY);
    world.insert_resource(baked);
    world.insert_resource(damage);
    body::stand(world, GRANULARITY);
    body::spawn_gore(world);
}

/// The example's whole solver — the crate names none.
fn integrate(time: Res<Time>, mut chunks: Query<(&mut Chunk, &mut Transform)>) {
    let dt = time.delta_secs() * 0.55;
    if dt <= 0.0 {
        return;
    }
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, dt);
    }
}
