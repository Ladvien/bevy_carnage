//! **The flesh material's scene**: a banded limb, a skin sphere over a flaymap and a wetmap, and a
//! hanging sheet — the three [`FleshMode`]s — shared by the windowed `flesh` demo and the headless
//! `capture_flesh` clip.

use std::f32::consts::FRAC_PI_2;

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy_carnage::cross_section::{CrossSectionAtlas, Layers, Region, Scale};
use bevy_carnage::flaymap::{FlayCanvas, FlaySettings};
use bevy_carnage::flesh::{FleshMaterial, FleshMode, FleshParams, FleshTables};
use bevy_carnage::wetmap::{StainShape, WetCanvas, WetSettings};

/// Canvas edge for the three subjects.
pub const CANVAS: u32 = 256;

/// The banded cylinder.
#[derive(Component)]
pub struct Limb;
/// The skin sphere.
#[derive(Component)]
pub struct Skin;
/// The hanging sheet.
#[derive(Component)]
pub struct Cloth;
/// The light that walks round the subjects.
#[derive(Component)]
pub struct Sun;

/// The three subjects and a light.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut flesh: ResMut<Assets<FleshMaterial>>,
    mut plain: ResMut<Assets<StandardMaterial>>,
    atlas: Res<CrossSectionAtlas>,
    tables: Res<FleshTables>,
) {
    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(3.0, 2.5, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
        Sun,
    ));
    // A slab to stand on.
    let floor = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(3.0)).mesh().build());
    commands.spawn((
        Mesh3d(floor),
        MeshMaterial3d(plain.add(StandardMaterial { base_color: Color::srgb(0.32, 0.31, 0.30), perceptual_roughness: 0.9, ..default() })),
        Transform::from_xyz(0.0, -0.35, 0.0),
    ));

    // ---- The limb: a cylinder whose UV_1.x walks the Limb row from skin to marrow. ------------
    let layers = Layers::for_region(Region::Limb);
    let scale = Scale::default();
    let limb_len = 1.1;
    let mut cyl: Mesh = Cylinder::new(0.16, limb_len).mesh().resolution(64).build();
    if let Some(VertexAttributeValues::Float32x3(pos)) = cyl.attribute(Mesh::ATTRIBUTE_POSITION) {
        let uv1: Vec<[f32; 2]> = pos
            .iter()
            .map(|p| {
                let along = (p[1] + limb_len * 0.5) / limb_len;
                // Around the limb, one strip repeat per `tile_units` of circumference.
                let theta = p[2].atan2(p[0]);
                [along.clamp(0.0, 0.999), theta * 0.16 / scale.tile_units.max(1.0e-3)]
            })
            .collect();
        cyl.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    }
    let limb_mesh = meshes.add(cyl);
    let strip = atlas
        .material(Region::Limb)
        .and_then(|h| plain.get(&h).cloned())
        .unwrap_or_else(|| StandardMaterial { base_color: Color::srgb(0.6, 0.2, 0.2), ..default() });
    let limb_wet = WetCanvas::new(&mut images, CANVAS, [0.5, 0.5, 0.5], 0.5);
    let limb_params = FleshParams::for_layers(&layers, FleshMode::Cap, scale.mm_per_unit);
    let limb_mat = flesh.add(tables.material(strip, limb_params, Some(limb_wet.roughness()), None));
    commands.spawn((
        Mesh3d(limb_mesh),
        MeshMaterial3d(limb_mat),
        Transform::from_xyz(-0.85, 0.0, 0.0).with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
        limb_wet,
        Limb,
    ));

    // ---- The skin: a sphere over a flaymap and a wetmap. ------------------------------------
    let sphere_mesh = meshes.add(Sphere::new(0.34).mesh().uv(64, 36));
    let flay = FlayCanvas::new(&mut images, CANVAS, Region::Limb, layers, [0.80, 0.62, 0.53], 0.55);
    let wet = WetCanvas::new(&mut images, CANVAS, [0.80, 0.62, 0.53], 0.55);
    let skin_base = StandardMaterial {
        base_color_texture: Some(flay.albedo()),
        metallic_roughness_texture: Some(flay.roughness()),
        perceptual_roughness: 1.0,
        ..default()
    };
    let skin_params = FleshParams::for_layers(&layers, FleshMode::Canvas, scale.mm_per_unit);
    let skin_mat = flesh.add(tables.material(skin_base, skin_params, Some(wet.roughness()), Some(flay.roughness())));
    commands.spawn((Mesh3d(sphere_mesh), MeshMaterial3d(skin_mat), Transform::from_xyz(0.05, 0.0, 0.0), flay, wet, Skin));

    // ---- The cloth: a hanging sheet, blood composited over its weave on the GPU. -------------
    let sheet = meshes.add(Plane3d::new(Vec3::Z, Vec2::new(0.32, 0.42)).mesh().build());
    let cloth_wet = WetCanvas::new(&mut images, CANVAS, [0.70, 0.66, 0.55], 0.9);
    let cloth_base = StandardMaterial {
        base_color: Color::srgb(0.70, 0.66, 0.55),
        perceptual_roughness: 0.95,
        ..default()
    };
    let mut cloth_params = FleshParams::for_layers(&layers, FleshMode::Cloth, scale.mm_per_unit);
    cloth_params.wet.z = 1.0;
    let cloth_mat = flesh.add(tables.material(cloth_base, cloth_params, Some(cloth_wet.roughness()), None));
    commands.spawn((Mesh3d(sheet), MeshMaterial3d(cloth_mat), Transform::from_xyz(0.95, 0.05, 0.0), cloth_wet, Cloth));
}

/// One hit: peel the sphere where the shot lands, and throw a stain on all three subjects.
pub fn hit(world: &mut World, n: u32, tick: u32) {
    let flay_settings = world.resource::<FlaySettings>().clone();
    // The wet settings carry `edge_samples`; only `paint_world_with` reads it.
    let wet_settings = world.resource::<WetSettings>().clone();
    let seed = 0x9E37_79B9u32.wrapping_mul(n + 1);
    let shape = StainShape {
        major: 0.09 + 0.01 * (n % 3) as f32,
        minor: 0.06,
        spines: 5,
        satellites: 3,
        direction: [0.0, 1.0],
        seed,
    };

    // The sphere: crater deeper each time at one spot, and blood around it.
    let mut skins = world.query_filtered::<(Entity, &Mesh3d, &GlobalTransform), With<Skin>>();
    let targets: Vec<(Entity, Handle<Mesh>, GlobalTransform)> =
        skins.iter(world).map(|(e, m, x)| (e, m.0.clone(), *x)).collect();
    for (entity, mesh_handle, xf) in targets {
        let Some(mesh) = world.resource::<Assets<Mesh>>().get(&mesh_handle).cloned() else { continue };
        let from = Vec3::new(0.05, 0.05, 2.0);
        let dir = Vec3::new(0.0, 0.0, -1.0);
        if let Some(mut flay) = world.get_mut::<FlayCanvas>(entity) {
            let depth = 4.0 + 6.0 * n as f32;
            let _ = flay.paint_world(&mesh, &xf, from, dir, 0.05, depth, tick);
            flay.shade(&flay_settings);
        }
        if let Some(mut wet) = world.get_mut::<WetCanvas>(entity) {
            let _ = wet.paint_world_with(&mesh, &xf, from, dir, &shape, tick, &wet_settings);
        }
    }

    // The limb and the cloth: blood lands on them too.
    let mut others = world.query_filtered::<(Entity, &Mesh3d, &GlobalTransform), Or<(With<Limb>, With<Cloth>)>>();
    let targets: Vec<(Entity, Handle<Mesh>, GlobalTransform)> =
        others.iter(world).map(|(e, m, x)| (e, m.0.clone(), *x)).collect();
    for (entity, mesh_handle, xf) in targets {
        let Some(mesh) = world.resource::<Assets<Mesh>>().get(&mesh_handle).cloned() else { continue };
        let centre = xf.translation();
        let jitter = Vec3::new(((n * 37) % 7) as f32 * 0.03 - 0.09, ((n * 53) % 5) as f32 * 0.04 - 0.08, 0.0);
        let from = centre + jitter + Vec3::new(0.0, 0.0, 2.0);
        let dir = Vec3::new(0.0, 0.0, -1.0);
        if let Some(mut wet) = world.get_mut::<WetCanvas>(entity) {
            let _ = wet.paint_world_with(&mesh, &xf, from, dir, &shape, tick, &wet_settings);
        }
    }
}
