//! **The two-line inclusion, live.** `GorePlugin`, a `Gore` on every surface, a `GoreHit` per key.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example preset
//! ```
//!
//! | key | what |
//! |---|---|
//! | `1` | a shot to the torso — peel, blood on the body, the sheet and the slab, a bleed; the third at one spot shows cortex and the body comes apart |
//! | `2` | a blow to the left arm — a bruise that ages in time-lapse under intact skin |
//! | `3` | a hot iron on the right thigh — a burn whose degree the damage integral decides |
//! | `4` | a slash across the left thigh — a laceration that bleeds and drips |
//! | `R` | reset |
//!
//! Every visual is the preset's and the flesh material's. Compare `carnage_web.rs`, which wired the
//! same eight crates by hand until 0.4.0 and now stands on this same scene.

use bevy::prelude::*;
use bevy_carnage::preset::{GorePlugin, GoreSystems};

mod common;
use common::preset_scene::{self, Body, Part, Sheet, Slab, Sun};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "bevy_carnage — preset".into(), ..default() }),
            ..default()
        }))
        .add_plugins(GorePlugin)
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .add_systems(Startup, setup)
        .add_systems(Update, (keys, orbit))
        .configure_sets(FixedUpdate, GoreSystems)
        .run();
}

#[derive(Component)]
struct Orbit;

/// How many shots have landed on the torso, so each lands a hair from the last.
#[derive(Resource, Default)]
struct Shots(u32);

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(1.1, 1.5, 2.9).looking_at(Vec3::new(0.15, 0.7, 0.0), Vec3::Y),
        AmbientLight { brightness: 260.0, ..default() },
        Orbit,
    ));
    commands.init_resource::<Shots>();
    preset_scene::spawn(&mut commands, &mut meshes, &mut materials);
    commands.spawn((
        Text::new("1 shot   2 blow   3 iron   4 slash   R reset"),
        TextFont { font_size: bevy::text::FontSize::Px(14.0), ..default() },
        TextColor(Color::srgb(0.92, 0.88, 0.86)),
        Node { position_type: PositionType::Absolute, left: Val::Px(12.0), top: Val::Px(10.0), ..default() },
    ));
}

fn keys(
    input: Res<ButtonInput<KeyCode>>,
    mut shots: ResMut<Shots>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    parts: Query<(Entity, &Part)>,
    scene: Query<Entity, Or<(With<Body>, With<Sheet>, With<Slab>, With<Sun>)>>,
    pieces: Query<
        Entity,
        Or<(With<bevy_carnage::preset::Flying>, With<bevy_carnage::preset::Gut>, With<bevy_carnage::preset::Standing>)>,
    >,
) {
    let part = |i: usize| parts.iter().find(|(_, p)| p.0 == i).map(|(e, _)| e);
    if input.just_pressed(KeyCode::Digit1)
        && let Some(torso) = part(0)
    {
        commands.write_message(preset_scene::shot(torso, shots.0));
        shots.0 += 1;
    }
    if input.just_pressed(KeyCode::Digit2)
        && let Some(arm) = part(2)
    {
        commands.write_message(preset_scene::blow(arm));
    }
    if input.just_pressed(KeyCode::Digit3)
        && let Some(leg) = part(5)
    {
        commands.write_message(preset_scene::scald(leg));
    }
    if input.just_pressed(KeyCode::Digit4)
        && let Some(leg) = part(4)
    {
        commands.write_message(preset_scene::slash(leg));
    }
    if input.just_pressed(KeyCode::KeyR) {
        for e in scene.iter().chain(pieces.iter()) {
            commands.entity(e).despawn();
        }
        shots.0 = 0;
        preset_scene::spawn(&mut commands, &mut meshes, &mut materials);
    }
}

fn orbit(time: Res<Time>, mut cams: Query<&mut Transform, With<Orbit>>) {
    let t = time.elapsed_secs() * 0.15;
    for mut tf in &mut cams {
        *tf = Transform::from_xyz(2.9 * t.sin() + 0.3, 1.5, 2.9 * t.cos()).looking_at(Vec3::new(0.15, 0.7, 0.0), Vec3::Y);
    }
}
