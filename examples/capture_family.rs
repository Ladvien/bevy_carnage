//! **Headless recorder for the two blood kernels this crate composes** — one script, two clips.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example capture_family -- --out frames-family
//! ```
//!
//! Writes `frame0000.png …` and prints one digest line; two runs must print the same line. The frame
//! ranges are the README clips of the two crates, cut with `tools/gif.sh`:
//!
//! | frames | crate | what |
//! |---|---|---|
//! | 0–149 | `bloodstain` | sixteen films, eight thicknesses by two oxygen saturations, ageing to dry |
//! | 150–389 | `bevy_wetmap` | six shots on a pale sphere, running under gravity, spreading, drying |
//!
//! The first clip is the spectral model with nothing else in the frame: each swatch's colour is
//! `bevy_carnage::bloodstain::spectral::srgb` for that film, and the ageing is `dry::appearance_of`, so a
//! thin arterial smear reads pink-scarlet and a thick venous pool near-black before either dries
//! (Bosschaart et al., `doi:10.1007/s10103-013-1446-7`). The second is the wetmap consuming it: the
//! canvas's coverage byte is a depth, and the drip is what makes a run stop where it stopped.
//!
//! Native only: it writes PNGs, and the recorder's `device.poll` has no browser equivalent.

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use bevy_carnage::blood::{spectral, BloodSettings};
use bevy_carnage::wetmap::{StainShape, WetCanvas, WetSettings, WetmapPlugin};

mod common;
use common::recorder::Recorder;

/// Film thicknesses across the swatch row, millimetres — a smear, a film, a pool.
const THICKNESS_MM: [f32; 8] = [0.05, 0.1, 0.2, 0.4, 0.8, 1.5, 2.5, 4.0];
/// Rows: arterial on top, venous below.
const SO2: [f32; 2] = [spectral::SO2_ARTERIAL, spectral::SO2_VENOUS];
/// Swatch area for the drying clock, m² — a coin-sized drop, so it dries within the clip.
const SWATCH_AREA_M2: f32 = 0.002;
/// Fixed ticks of age per captured frame in the swatch clip.
const AGE_PER_FRAME: u32 = 12;
/// Wetmap canvas edge, texels — the crate's shipped default.
const CANVAS: u32 = 128;

#[derive(Component)]
struct Swatch(spectral::Film);
#[derive(Component)]
struct Subject;

fn main() {
    let out = common::arg("--out").unwrap_or_else(|| "frames-family".to_string());
    let camera = Transform::from_xyz(0.0, 0.0, 3.2).looking_at(Vec3::ZERO, Vec3::Y);
    let Some(mut rec) = Recorder::new_with(640, 480, camera, &out, |app| {
        // `edge_samples: 4` — four subsamples per texel axis on a stain's edge. The crate ships `1`
        // so its frozen digests stay frozen; a recorder has no golden to protect and every reason to
        // show the smoother rim.
        app.add_plugins(WetmapPlugin)
            .insert_resource(WetSettings { dry_ticks: 150, edge_samples: 4, ..default() });
    }) else {
        return;
    };

    {
        let world = rec.world();
        world.spawn((
            DirectionalLight { illuminance: 5_500.0, shadow_maps_enabled: false, ..default() },
            Transform::from_xyz(2.0, 3.0, 2.5).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let mut cam = world.query_filtered::<Entity, With<Camera3d>>();
        if let Some(entity) = cam.iter(world).next() {
            world.entity_mut(entity).insert(AmbientLight { brightness: 420.0, ..default() });
        }
    }

    // ---- Phase 1: bloodstain. Sixteen films on a neutral card, ageing. ----------------------
    let blood = BloodSettings::default();
    {
        let world = rec.world();
        let card = world.resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Z, Vec2::new(1.75, 1.0)).mesh().build());
        let card_material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color: Color::srgb(0.86, 0.84, 0.80),
            unlit: true,
            ..default()
        });
        world.spawn((Mesh3d(card), MeshMaterial3d(card_material), Transform::from_xyz(0.0, 0.0, -0.01)));
        let quad = world.resource_mut::<Assets<Mesh>>().add(Plane3d::new(Vec3::Z, Vec2::new(0.17, 0.30)).mesh().build());
        for (row, so2) in SO2.iter().enumerate() {
            for (col, thickness_mm) in THICKNESS_MM.iter().enumerate() {
                let film = spectral::Film { thickness_mm: *thickness_mm, so2: *so2, substrate: 0.86 };
                let material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial { unlit: true, ..default() });
                let x = -1.4 + col as f32 * 0.4;
                let y = 0.4 - row as f32 * 0.8;
                world.spawn((Mesh3d(quad.clone()), MeshMaterial3d(material), Transform::from_xyz(x, y, 0.0), Swatch(film)));
            }
        }
    }
    let mut swatch_digest: u64 = 0xcbf2_9ce4_8422_2325;
    rec.warm_up(2);
    for frame in 0..150u32 {
        let age = frame * AGE_PER_FRAME;
        let world = rec.world();
        let mut q = world.query::<(&Swatch, &MeshMaterial3d<StandardMaterial>)>();
        let mut updates: Vec<(Handle<StandardMaterial>, [f32; 3], f32)> = Vec::new();
        for (swatch, material) in q.iter(world) {
            let look = bevy_carnage::blood::appearance_of(age, 60, SWATCH_AREA_M2, &blood, &swatch.0);
            updates.push((material.0.clone(), look.srgb, look.roughness));
        }
        // Spawn order is deterministic; fold in that order rather than query order.
        updates.sort_by(|a, b| a.0.id().cmp(&b.0.id()));
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        for (handle, srgb, roughness) in &updates {
            if let Some(mut m) = materials.get_mut(handle) {
                m.base_color = Color::srgb(srgb[0], srgb[1], srgb[2]);
                m.perceptual_roughness = *roughness;
            }
            for c in srgb {
                for byte in c.to_bits().to_le_bytes() {
                    swatch_digest ^= u64::from(byte);
                    swatch_digest = swatch_digest.wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        rec.shoot();
    }
    {
        let world = rec.world();
        let doomed: Vec<Entity> = world.query_filtered::<Entity, With<Mesh3d>>().iter(world).collect();
        for e in doomed {
            world.despawn(e);
        }
    }

    // ---- Phase 2: wetmap. A pale sphere, shot six times, running and drying. ----------------
    {
        let world = rec.world();
        let canvas = WetCanvas::new(&mut world.resource_mut::<Assets<Image>>(), CANVAS, [0.80, 0.68, 0.62], 0.55);
        let material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color_texture: Some(canvas.albedo()),
            metallic_roughness_texture: Some(canvas.roughness()),
            perceptual_roughness: 1.0,
            metallic: 1.0,
            ..default()
        });
        let mesh = world.resource_mut::<Assets<Mesh>>().add(Sphere::new(0.5).mesh().uv(64, 32));
        world.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
            canvas,
            Subject,
        ));
        let mut cam = world.query_filtered::<&mut Transform, With<Camera3d>>();
        for mut t in cam.iter_mut(world) {
            *t = Transform::from_xyz(0.0, 0.25, 1.7).looking_at(Vec3::ZERO, Vec3::Y);
        }
    }
    rec.warm_up(2);
    let gravity = Vec2::new(0.0, 1.0);
    /// Where the six shots land on the face the camera sees, metres in the sphere's XY plane.
    const SHOTS: [(f32, f32); 6] = [(-0.22, 0.30), (0.12, 0.34), (0.30, 0.12), (-0.05, 0.10), (-0.30, -0.02), (0.18, -0.12)];
    for frame in 150..390u32 {
        let tick = frame - 150;
        let shot = tick % 15 == 0 && tick < 90;
        let world = rec.world();
        let settings = world.resource::<WetSettings>().clone();
        // The mesh is only needed to cast a ray, so it is only cloned on the six shot frames.
        let mesh_handle = world.query_filtered::<&Mesh3d, With<Subject>>().iter(world).next().map(|m| m.0.clone());
        let xf = world.query_filtered::<&GlobalTransform, With<Subject>>().iter(world).next().copied();
        let mesh = if shot { mesh_handle.and_then(|h| world.resource::<Assets<Mesh>>().get(&h).cloned()) } else { None };
        let mut q = world.query_filtered::<&mut WetCanvas, With<Subject>>();
        for mut canvas in q.iter_mut(world) {
            if shot {
                let i = (tick / 15) as usize;
                let (x, y) = SHOTS[i.min(SHOTS.len() - 1)];
                let shape = StainShape {
                    major: 0.11,
                    minor: 0.07,
                    spines: 6,
                    satellites: 3,
                    direction: [0.0, 1.0],
                    seed: (tick ^ (i as u32 * 977)).wrapping_mul(0x9E37_79B9) ^ 0x5EED,
                };
                if let (Some(mesh), Some(xf)) = (mesh.as_ref(), xf.as_ref()) {
                    canvas.paint_world_with(
                        mesh,
                        xf,
                        Vec3::new(x, y, 1.5),
                        -Vec3::Z,
                        &shape,
                        tick,
                        &settings,
                    );
                }
            }
            canvas.tick(tick, gravity, &settings);
        }
        rec.shoot();
    }
    let wet_digest = {
        let world = rec.world();
        world.query_filtered::<&WetCanvas, With<Subject>>().iter(world).next().map(|c| c.digest()).unwrap_or(0)
    };

    let n = rec.finish();
    println!("family: frames={n} bloodstain={swatch_digest:016x} wetmap={wet_digest:016x}");
}
