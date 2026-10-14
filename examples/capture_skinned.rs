//! **Headless recorder for a skinned character** — the preset on a body that is never in its bind pose.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example capture_skinned -- --out frames-skinned
//! ```
//!
//! `--eye x,y,z` renders from another camera position, aimed at the chest or at `--at x,y,z`. Neither
//! changes a digest.
//! `--break` makes the body breakable and takes a heavy shot to the chest at frame 200: it breaks where
//! its skeleton is, mid-walk with the arm raised, and its pieces fall to the floor. `--shirt` dresses
//! the body in a sleeveless shirt: the two chest shots hole it and the wound is seen through the hole,
//! with the blood the shirt soaks up around it. The holes are folded into the digest. `--through` puts
//! a wall behind the body and every shot goes through it: an exit wound opens on the far side and its
//! forward spatter lands on the wall and the floor behind, and runs down the wall. `--quiet` takes no
//! hits at all. `--reset-at N` sends a `GoreReset` for the body and every dressed surface on frame
//! `N`: once the canvases have uploaded, every frame after is the quiet run's, and so is the digest.
//!
//! Writes `frame0000.png …` and prints one digest line; two runs must print the same line.
//!
//! The subject is `common/mannequin.rs`: six skinned meshes on eleven joints, walking in place with
//! its left arm raised and waving. **Every shot is aimed at where the body is drawn at that frame**,
//! from the joint transforms the renderer last used, so a wound that opens somewhere else is the
//! preset casting at the bind pose — arms hanging at the sides — instead of at the pose.
//!
//! | frame | what |
//! |---|---|
//! | 0–29 | the body walks, dressed in the flesh material, the left arm up |
//! | 30 | a shot to the raised left forearm |
//! | 66 | a slash across the right thigh, mid-stride |
//! | 100 | a hot iron on the right forearm, which swings |
//! | 120, 150 | two shots to the chest, at one spot |
//! | 190 | a blow to the left shin |
//! | 192–359 | every wound rides the limb it is on, bleeding and dripping to the floor until it clots |

use std::time::Duration;

use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy_carnage::preset::{ClothHoles, Gore, GoreClock, GorePlugin, GoreReset, HitKind};
use bevy_carnage::wetmap::WetCanvas;

mod common;
use common::mannequin::{self, Bone, Mannequin};
use common::preset_scene;
use common::recorder::Recorder;

const FRAMES: u32 = 360;

fn main() {
    let out = common::arg("--out").unwrap_or_else(|| "frames-skinned".to_string());
    // `--eye x,y,z` looks at the body from somewhere else, and `--at x,y,z` at another point than
    // the chest: the digest depends on neither, so they are how a render is checked from an angle,
    // or as close, as the committed clip does not use.
    let point = |name: &str| {
        common::arg(name)
            .map(|e| e.split(',').filter_map(|v| v.trim().parse::<f32>().ok()).collect::<Vec<_>>())
            .and_then(|v| (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2])))
    };
    let at = point("--at").unwrap_or(Vec3::new(0.12, 0.93, 0.1));
    let camera = match point("--eye") {
        Some(eye) => Transform::from_translation(eye).looking_at(at, Vec3::Y),
        None => mannequin::view(),
    };
    let Some(mut rec) = Recorder::new_with(800, 500, camera, &out, |app| {
        // **The clock is the frame, not the wall**, for the reason `capture_preset` gives: one sixtieth
        // per update and a 60 Hz fixed step is one tick per frame, and the animation reads the same
        // clock, so frame 62 of one run is frame 62 of the next. With `vfx`, every wound throws its
        // burst of blood in the air; the digest cannot see it.
        #[cfg(feature = "vfx")]
        app.add_plugins(bevy_carnage::CarnageVfxPlugin);
        app.add_plugins(GorePlugin)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)))
            .insert_resource(Time::<Fixed>::from_hz(60.0))
            .add_systems(Startup, setup)
            .add_systems(Update, mannequin::animate);
    }) else {
        return;
    };
    {
        let world = rec.world();
        let mut cam = world.query_filtered::<Entity, With<Camera3d>>();
        if let Some(entity) = cam.iter(world).next() {
            world.entity_mut(entity).insert((AmbientLight { brightness: preset_scene::AMBIENT, ..default() }, preset_scene::EXPOSURE));
        }
    }
    rec.warm_up(6);
    let breaking = breaking();
    let through = flag("--through");
    let shirt = shirted();
    // `--quiet` takes no hits; `--reset-at N` heals the body and wipes every surface on frame `N`,
    // after which a run is the quiet one again, frame for frame.
    let quiet = flag("--quiet");
    let reset_at = common::arg("--reset-at").and_then(|n| n.parse::<u32>().ok());
    let mut aim = bevy::ecs::system::SystemState::<mannequin::Aim>::new(rec.world());
    for frame in 0..FRAMES {
        {
            let world = rec.world();
            let hit = aim.get(world).ok().and_then(|aim| match frame {
                _ if quiet => None,
                30 => aim.shot(Bone::ArmLowL, 0.13, 0.03, 7.0),
                66 => aim.slash(Bone::LegUpR, 0.20, Vec3::new(0.8, -0.6, 0.0).normalize(), 0.07, 6.0),
                100 => aim.iron(Bone::ArmLowR, 0.10, 0.035),
                120 | 150 => aim.shot(Bone::Spine, -0.22, 0.035, 9.0),
                190 => aim.blow(Bone::LegLowL, 0.12, 0.04),
                200 if breaking => aim.shot(Bone::Spine, -0.22, 0.035, 60.0),
                200 if shirt => aim.slash(Bone::Spine, -0.05, Vec3::new(1.0, -0.35, 0.0).normalize(), 0.09, 6.0),
                _ => None,
            });
            if let Some(hit) = hit {
                let hit = if through && matches!(hit.kind, HitKind::Impact { .. }) { hit.through() } else { hit };
                world.write_message(hit);
            }
            if reset_at == Some(frame) {
                let mut surfaces = world.query_filtered::<Entity, Or<(With<Mannequin>, With<Gore>)>>();
                let all: Vec<Entity> = surfaces.iter(world).collect();
                for entity in all {
                    world.write_message(GoreReset::new(entity));
                }
            }
        }
        rec.shoot();
    }
    let world = rec.world();
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    // Sorted before folding, for the reason `capture_preset` gives.
    let mut wets = world.query::<&WetCanvas>();
    let mut values: Vec<u64> = wets.iter(world).map(|w| w.digest()).collect();
    // Only a cloth that was holed, so a clip in which nothing was holed keeps the digest it had.
    let mut holes = world.query::<&ClothHoles>();
    values.extend(holes.iter(world).filter(|h| h.is_holed()).map(ClothHoles::digest));
    values.sort_unstable();
    for value in values {
        digest ^= value;
        digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let tick = world.resource::<GoreClock>().0;
    let frames = rec.finish();
    println!("skinned: frames={frames} ticks={tick} digest={digest:016x}");
}
/// Whether `--break` was given: the mannequin is made breakable and takes a hit to bone at frame 200.
fn breaking() -> bool {
    std::env::args().any(|a| a == "--break")
}

/// Whether `--shirt` was given: the mannequin wears a shirt.
fn shirted() -> bool {
    flag("--shirt")
}

/// Whether a bare flag was given.
fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    let root = mannequin::spawn(&mut commands, &mut meshes, &mut materials, &mut images, &mut bindposes, shirted());
    if breaking() {
        commands.entity(root).insert(mannequin::breakable());
    }
    preset_scene::spawn_set(&mut commands, &mut meshes, &mut materials);
    if flag("--through") {
        preset_scene::spawn_wall(&mut commands, &mut meshes, &mut materials);
    }
}
