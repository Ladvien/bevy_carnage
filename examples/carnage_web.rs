//! **The flagship. One body, every injury, and the whole family behind one plugin.**
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example carnage_web
//! ```
//!
//! Until 0.4.0 this file was a thousand lines that named eight crates and ran their ticks by hand,
//! in the right order, for one shot. That was an honest demonstration of the framework and a
//! dishonest demonstration of its cost. It now stands on [`bevy_carnage::preset`], which is the
//! same eight crates behind `GorePlugin`, a `Gore` per surface and a `GoreHit` per injury — and it
//! draws with `bevy_carnage::flesh`, the material every dressed surface wears.
//!
//! | key | what |
//! |---|---|
//! | `click` / `1` | a shot to the torso: the flaymap peels, the wound bleeds onto the body, the sheet and the slab, the third at one spot shows cortex and the modes part the body |
//! | `2` | a blow to the left arm: a bruise, Stam's kinetics in time-lapse under intact skin |
//! | `3` | a hot iron on the right thigh: a burn, Henriques' damage integral picking the degree |
//! | `4` | a slash across the left thigh: a laceration that bleeds and drips |
//! | `5` | an arterial shot: the same wound spurting arterial blood |
//! | `H` | the digests — every canvas is CPU-owned and hashable |
//! | `R` | reset |
//!
//! # No particles, and that is the wasm build's rule rather than this demo's preference
//!
//! `scripts/build_web.sh` builds with `--no-default-features --features serde,flesh`, because
//! `bevy_hanabi`'s wasm support is WebGPU-compute-only. Every visual here is a mesh and a canvas,
//! drawn by the flesh material from the CPU-side model — which is what all the new work in this
//! framework is anyway.

use bevy::prelude::*;
use bevy_carnage::flaymap::FlayCanvas;
use bevy_carnage::preset::{GoreClock, GorePlugin, GoreSystems};
use bevy_carnage::wetmap::WetCanvas;

mod common;
use common::preset_scene::{self, Body, Part, Sheet, Slab};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage — the flagship".into(),
                canvas: Some("#bevy".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(GorePlugin)
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .init_resource::<Shots>()
        .add_systems(Startup, setup)
        .add_systems(Update, (keys, orbit, hud))
        .configure_sets(FixedUpdate, GoreSystems)
        .run();
}

#[derive(Component)]
struct Orbit;
#[derive(Component)]
struct Hud;

/// Shots landed on the torso, and whether the numbers are on screen.
#[derive(Resource, Default)]
struct Shots {
    n: u32,
    numbers: bool,
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(1.1, 1.5, 2.9).looking_at(Vec3::new(0.15, 0.7, 0.0), Vec3::Y),
        AmbientLight { brightness: 260.0, ..default() },
        Orbit,
    ));
    preset_scene::spawn(&mut commands, &mut meshes, &mut materials);
    commands.spawn((
        Text::new(""),
        TextFont { font_size: bevy::text::FontSize::Px(13.0), ..default() },
        TextColor(Color::srgb(0.92, 0.88, 0.86)),
        Node { position_type: PositionType::Absolute, left: Val::Px(12.0), top: Val::Px(10.0), ..default() },
        Hud,
    ));
}

#[allow(clippy::too_many_arguments)]
fn keys(
    input: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut shots: ResMut<Shots>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    parts: Query<(Entity, &Part)>,
    scene: Query<Entity, Or<(With<Body>, With<Sheet>, With<Slab>)>>,
    pieces: Query<Entity, Or<(With<bevy_carnage::preset::Flying>, With<bevy_carnage::preset::Gut>)>>,
) {
    let part = |i: usize| parts.iter().find(|(_, p)| p.0 == i).map(|(e, _)| e);
    if (input.just_pressed(KeyCode::Digit1) || mouse.just_pressed(MouseButton::Left))
        && let Some(torso) = part(0)
    {
        commands.write_message(preset_scene::shot(torso, shots.n));
        shots.n += 1;
    }
    if input.just_pressed(KeyCode::Digit5)
        && let Some(torso) = part(0)
    {
        commands.write_message(preset_scene::shot(torso, shots.n).arterial());
        shots.n += 1;
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
    if input.just_pressed(KeyCode::KeyH) {
        shots.numbers = !shots.numbers;
    }
    if input.just_pressed(KeyCode::KeyR) {
        for e in scene.iter().chain(pieces.iter()) {
            commands.entity(e).despawn();
        }
        shots.n = 0;
        preset_scene::spawn(&mut commands, &mut meshes, &mut materials);
    }
}

fn orbit(time: Res<Time>, mut cams: Query<&mut Transform, With<Orbit>>) {
    let t = time.elapsed_secs() * 0.12;
    for mut tf in &mut cams {
        *tf = Transform::from_xyz(2.9 * t.sin() + 0.3, 1.5, 2.9 * t.cos()).looking_at(Vec3::new(0.15, 0.7, 0.0), Vec3::Y);
    }
}

/// The readout: the keys, and — behind `H` — the digests of every canvas, which exist because the
/// canvases are CPU-owned.
fn hud(
    shots: Res<Shots>,
    clock: Res<GoreClock>,
    wets: Query<&WetCanvas>,
    flays: Query<&FlayCanvas>,
    mut hud: Query<&mut Text, With<Hud>>,
) {
    let mut text = String::from("click/1 shot   2 blow   3 iron   4 slash   5 arterial   H numbers   R reset");
    if shots.numbers {
        text.push_str(&format!("\ntick {}   shots {}", clock.0, shots.n));
        for (i, w) in wets.iter().enumerate() {
            text.push_str(&format!("\nwetmap {i}  {:016x}  wet {:.1} texels", w.digest(), w.wetted_area()));
        }
        for (i, f) in flays.iter().enumerate() {
            text.push_str(&format!("\nflaymap {i} {:016x}  bone {} texels", f.digest(), f.bone_texels()));
        }
    }
    for mut t in &mut hud {
        t.0 = text.clone();
    }
}
