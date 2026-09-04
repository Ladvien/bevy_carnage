//! **The preset's scene**: the blockout body dressed by [`bevy_carnage::preset`], a cloth panel,
//! and a floor — shared by the windowed `preset` demo and the headless `capture_preset` clip.
//!
//! This file is the whole inclusion cost of the family, and that is the point of it: one plugin, one
//! component per surface, one message per hit. Compare `carnage_web.rs`, which does the same by hand.

use bevy::prelude::*;
use bevy_carnage::cross_section::{Region, Scale};
use bevy_carnage::preset::{Gore, GoreBreakable, GoreHit};
use bevy_carnage::{CutSettings, fracture_mesh};

use super::body::{self, ORIGIN, SEED};

/// The body root, so a demo can find what to hit and what to reset.
#[derive(Component)]
pub struct Body;
/// One dressed part, with its index into [`body::parts`].
#[derive(Component)]
pub struct Part(pub usize);
/// The hanging sheet.
#[derive(Component)]
pub struct Sheet;
/// The slab.
#[derive(Component)]
pub struct Slab;

/// Which thickness row a part's cut faces and canvases use.
pub fn region_of(part: usize) -> Region {
    match part {
        0 => Region::Torso,
        1 => Region::Head,
        _ => Region::Limb,
    }
}

/// Spawn everything. `skin` is the body's own material; the preset dresses over it.
pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let skin = materials.add(StandardMaterial {
        base_color: Color::srgb(0.80, 0.62, 0.53),
        perceptual_roughness: 0.55,
        ..default()
    });

    // The bake, once: the same blockout every other demo cuts, at the finest frontier.
    let subject = body::subject();
    let parts: Vec<(&Mesh, Mat4)> = subject.iter().map(|(m, x)| (m, *x)).collect();
    let proxy = body::proxy();
    let cut = CutSettings { soften: 0.0, ..CutSettings::new(body::TARGET, 0.08, SEED) };
    let fracture = fracture_mesh(&parts, &proxy, &cut);
    let breakable = GoreBreakable::from_fracture(fracture, Region::Limb, &Scale::default());

    commands
        .spawn((Transform::from_translation(ORIGIN), Visibility::default(), breakable, Body))
        .with_children(|root| {
            for (i, (_, centre, half)) in body::parts().into_iter().enumerate() {
                // A cuboid's atlas spans each face from 0 to 1, so its UV density is one over the
                // face's longer edge.
                let face_m = half.x.max(half.y) * 2.0;
                root.spawn((
                    Mesh3d(meshes.add(Cuboid::new(half.x * 2.0, half.y * 2.0, half.z * 2.0))),
                    MeshMaterial3d(skin.clone()),
                    Transform::from_translation(centre),
                    Gore::skin(region_of(i)).with_uv_per_metre(1.0 / face_m),
                    Part(i),
                ));
            }
        });

    // A sheet to the right, hanging in the spray.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Z, Vec2::new(0.28, 0.45)).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.72, 0.68, 0.56),
            perceptual_roughness: 0.95,
            // A sheet has two sides and blood soaks through a thin one.
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        // In the spray: forward-right of the body, turned to face it.
        Transform::from_xyz(0.75, 0.75, 0.55).looking_to(Vec3::new(0.6, 0.0, 0.8), Vec3::Y),
        Gore::cloth().with_uv_per_metre(1.0 / 0.9),
        Sheet,
    ));

    // The slab.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(1.5)).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.34, 0.33, 0.32),
            perceptual_roughness: 0.9,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Gore::floor().with_uv_per_metre(1.0 / 3.0),
        Slab,
    ));

    commands.spawn((
        DirectionalLight { illuminance: 11_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(-1.5, 4.0, 3.0).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
    ));
}

/// A ray onto the front face of a part, offset from its centre by `offset` (subject-local).
pub fn ray_at(part: usize, offset: Vec3) -> (Vec3, Vec3) {
    let (_, centre, half) = body::parts().get(part).copied().unwrap_or(("", Vec3::ZERO, Vec3::splat(0.1)));
    let world = ORIGIN + centre + offset;
    (world + Vec3::new(0.0, 0.0, half.z + 1.0), Vec3::NEG_Z)
}

/// The scripted shot `n` on the torso: the same spot, a hair of jitter, so the crater deepens to bone.
pub fn shot(torso: Entity, n: u32) -> GoreHit {
    let jitter = Vec3::new(((n * 7) % 5) as f32 * 0.008 - 0.016, ((n * 11) % 5) as f32 * 0.01 - 0.02 + 0.06, 0.0);
    let (from, dir) = ray_at(0, jitter);
    GoreHit::impact(torso, from, dir, 0.035, 9.0)
}

/// A blow to the upper left arm: a bruise, no blood.
pub fn blow(arm: Entity) -> GoreHit {
    let (from, dir) = ray_at(2, Vec3::new(0.0, 0.08, 0.0));
    GoreHit::blunt(arm, from, dir, 0.03)
}

/// A hot iron against the right thigh: 150 °C for a second.
pub fn scald(leg: Entity) -> GoreHit {
    let (from, dir) = ray_at(5, Vec3::new(0.0, 0.1, 0.0));
    GoreHit::burn(leg, from, dir, 0.035, 150.0, 1.0)
}

/// A slash across the left thigh, gaping onto its bed.
pub fn slash(leg: Entity) -> GoreHit {
    let (from, dir) = ray_at(4, Vec3::new(0.0, 0.05, 0.0));
    GoreHit::slash(leg, from, dir, Vec3::new(0.7, -0.7, 0.0).normalize(), 0.06, 8.0)
}
