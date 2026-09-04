//! **A slash across a patch of skin, opening over about three seconds.**
//!
//! ```sh
//! cargo run --example laceration_open
//! ```
//!
//! Everything is at real scale: the patch is 30 cm across, the cut is 12 mm wide when fully open,
//! and the bed floor sits 6 mm down — which is subcutaneous fat on a limb, so the trough is painted
//! with `bevy_cross_section`'s banded strip rather than a flat dark colour. Press **Space** to close
//! the wound and watch it open again.
//!
//! The Langer lines run along `z` here and the cut runs mostly along `x`, so this is the *across*
//! case: full gape. Rotate the cut towards `z` in [`CUT`] and it opens to 57 % of this width.
//!
//! No filesystem access, so it also builds for `wasm32-unknown-unknown`.

use bevy::prelude::*;
use bevy_carnage::laceration::{
    Gape, Laceration, LacerationClock, LacerationPlugin, Region, Tension, skin_patch,
};

/// The patch: 30 cm across in 5 mm cells, which is fine enough that a 6 mm lip has vertices to land on.
const PATCH_M: f32 = 0.30;
const PATCH_CELLS: u32 = 60;

/// The cut, in the patch's own space — a shallow diagonal, mostly across the Langer lines.
const CUT: [Vec3; 3] = [
    Vec3::new(-0.10, 0.0, -0.03),
    Vec3::new(0.0, 0.0, 0.01),
    Vec3::new(0.10, 0.0, -0.01),
];

/// Which way the collagen runs. Along `z`, so a cut along `x` severs it and gapes fully.
const LANGER: [f32; 3] = [0.0, 0.0, 1.0];

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_laceration — space to re-open".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(bevy_carnage::cross_section::CrossSectionPlugin)
        .add_plugins(LacerationPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (orbit, reopen))
        .run();
}

/// The camera, so `orbit` can find it without a `With<Camera3d>` filter — a second camera would make
/// that filter ambiguous, and in Bevy 0.19 an ambiguous `Single<..>` silently skips its system.
#[derive(Component)]
struct Orbit;

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let patch = skin_patch(PATCH_CELLS, PATCH_M);
    // **Two handles, one patch.** The entity draws the first; the component cuts from the second every
    // time the gape moves. One handle for both would mean each retear cut the previous result, and the
    // plugin refuses that outright rather than letting the wound drift.
    let drawn = meshes.add(patch.clone());
    let source = meshes.add(patch);

    commands.spawn((
        Mesh3d(drawn),
        MeshMaterial3d(materials.add(StandardMaterial {
            // Unlit-looking skin: a warm mid tone, matte, no metal.
            base_color: Color::srgb(0.82, 0.62, 0.52),
            perceptual_roughness: 0.72,
            ..default()
        })),
        Transform::IDENTITY,
        Laceration {
            path: CUT.to_vec(),
            normal: Vec3::Y,
            // 12 mm fully open, 95 % of it at 180 ticks — three seconds at 60 Hz.
            gape: Gape { width_max: 0.012, open_ticks: 180 },
            tension: Tension { skin: 0.9, langer: Some(LANGER) },
            influence: 0.030,
            bed_depth_mm: 6.0,
            region: Region::Limb,
            source,
            ..default()
        },
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.16, 0.22).looking_at(Vec3::ZERO, Vec3::Y),
        // A component in 0.19, and per-camera — not a resource.
        AmbientLight { brightness: 400.0, ..default() },
        Orbit,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            // 0.19 spells this `shadow_maps_enabled`, not `shadows_enabled` — and the shadow is what
            // makes a 6 mm trough read as a trough rather than as a dark stripe.
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(0.12, 0.25, 0.08).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// Slow orbit, so the gape is read from more than one angle.
fn orbit(time: Res<Time>, mut cams: Query<&mut Transform, With<Orbit>>) {
    let angle = time.elapsed_secs() * 0.25;
    for mut tf in &mut cams {
        let r = 0.24;
        *tf = Transform::from_xyz(angle.sin() * r, 0.15, angle.cos() * r).looking_at(Vec3::ZERO, Vec3::Y);
    }
}

/// Space closes the wound and lets it open again: `opened_at` is moved up to now, and `last_gape` is
/// reset to the "never torn" sentinel so the next tick re-cuts even though the width barely moved.
fn reopen(keys: Res<ButtonInput<KeyCode>>, clock: Option<Res<LacerationClock>>, mut wounds: Query<&mut Laceration>) {
    if !keys.just_pressed(KeyCode::Space) {
        return;
    }
    let Some(clock) = clock else {
        return;
    };
    for mut wound in &mut wounds {
        wound.opened_at = clock.0;
        wound.last_gape = -1.0;
    }
}
