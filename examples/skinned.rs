//! **The preset on a skinned, animated body, live.** A character walks in place with its left arm up
//! and waving; every key fires a shot, a slash, a burn or a blow at where that part of the body *is
//! drawn right now*, from a muzzle in front of it.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example skinned
//! ```
//!
//! | key | what |
//! |---|---|
//! | `1` | a shot to the raised left forearm |
//! | `2` | a slash across the right thigh |
//! | `3` | a shot to the chest — hold it down at one spot and the crater deepens |
//! | `4` | a hot iron on the right forearm |
//! | `5` | a blow to the left shin |
//! | `R` | reset |
//!
//! The wounds are the preset's and ride the limb they are on: the arm that took the shot keeps waving
//! with its wound on it, blood runs along it and drips from where the wound is now.

use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use bevy_carnage::preset::{GoreHit, GorePlugin, GoreSystems};

mod common;
use common::mannequin::{self, Bone, Limb, Mannequin};
use common::preset_scene::{self, Sheet, Slab, Sun};

fn main() {
    App::new()
        .add_plugins(common::autopilot::plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "bevy_carnage — skinned".into(), ..default() }),
            ..default()
        })))
        .add_plugins(common::autopilot::Autopilot)
        .add_plugins(GorePlugin)
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .add_systems(Startup, setup)
        .add_systems(Update, (mannequin::animate, keys))
        .configure_sets(FixedUpdate, GoreSystems)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    commands.spawn((
        Camera3d::default(),
        mannequin::view(),
        AmbientLight { brightness: preset_scene::AMBIENT, ..default() },
        preset_scene::EXPOSURE,
    ));
    mannequin::spawn(&mut commands, &mut meshes, &mut materials, &mut images, &mut bindposes);
    preset_scene::spawn_set(&mut commands, &mut meshes, &mut materials);
    commands.spawn((
        Text::new("1 shot to the forearm   2 slash   3 chest   4 iron   5 blow   R reset"),
        TextFont { font_size: bevy::text::FontSize::Px(14.0), ..default() },
        TextColor(Color::srgb(0.92, 0.88, 0.86)),
        Node { position_type: PositionType::Absolute, left: Val::Px(12.0), top: Val::Px(10.0), ..default() },
    ));
}

/// Everything a reset despawns: the character, the sheet, the slab and the lights.
type Scene = Or<(With<Mannequin>, With<Sheet>, With<Slab>, With<Sun>)>;

#[allow(clippy::too_many_arguments)]
fn keys(
    input: Res<ButtonInput<KeyCode>>,
    aim: mannequin::Aim,
    mut hits: MessageWriter<GoreHit>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    scene: Query<Entity, Scene>,
) {
    let hit = if input.just_pressed(KeyCode::Digit1) {
        aim.shot(Limb::ArmL, Bone::ArmLowL, 0.13, 0.03, 7.0)
    } else if input.just_pressed(KeyCode::Digit2) {
        aim.slash(Limb::LegR, Bone::LegUpR, 0.20, Vec3::new(0.8, -0.6, 0.0).normalize(), 0.07, 6.0)
    } else if input.just_pressed(KeyCode::Digit3) {
        aim.shot(Limb::Torso, Bone::Spine, -0.22, 0.035, 9.0)
    } else if input.just_pressed(KeyCode::Digit4) {
        aim.iron(Limb::ArmR, Bone::ArmLowR, 0.10, 0.035)
    } else if input.just_pressed(KeyCode::Digit5) {
        aim.blow(Limb::LegL, Bone::LegLowL, 0.12, 0.04)
    } else {
        None
    };
    if let Some(hit) = hit {
        hits.write(hit);
    }
    if input.just_pressed(KeyCode::KeyR) {
        for entity in &scene {
            commands.entity(entity).despawn();
        }
        mannequin::spawn(&mut commands, &mut meshes, &mut materials, &mut images, &mut bindposes);
        preset_scene::spawn_set(&mut commands, &mut meshes, &mut materials);
    }
}
