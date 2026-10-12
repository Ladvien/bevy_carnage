//! **Headless recorder for a skinned character** — the preset on a body that is never in its bind pose.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example capture_skinned -- --out frames-skinned
//! ```
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
use bevy_carnage::preset::{GoreClock, GorePlugin};
use bevy_carnage::wetmap::WetCanvas;

mod common;
use common::mannequin::{self, Bone, Limb};
use common::preset_scene;
use common::recorder::Recorder;

const FRAMES: u32 = 360;

fn main() {
    let out = common::arg("--out").unwrap_or_else(|| "frames-skinned".to_string());
    let camera = mannequin::view();
    let Some(mut rec) = Recorder::new_with(800, 500, camera, &out, |app| {
        // **The clock is the frame, not the wall**, for the reason `capture_preset` gives: one sixtieth
        // per update and a 60 Hz fixed step is one tick per frame, and the animation reads the same
        // clock, so frame 62 of one run is frame 62 of the next.
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
    let mut aim = bevy::ecs::system::SystemState::<mannequin::Aim>::new(rec.world());
    for frame in 0..FRAMES {
        {
            let world = rec.world();
            let hit = aim.get(world).ok().and_then(|aim| match frame {
                30 => aim.shot(Limb::ArmL, Bone::ArmLowL, 0.13, 0.03, 7.0),
                66 => aim.slash(Limb::LegR, Bone::LegUpR, 0.20, Vec3::new(0.8, -0.6, 0.0).normalize(), 0.07, 6.0),
                100 => aim.iron(Limb::ArmR, Bone::ArmLowR, 0.10, 0.035),
                120 | 150 => aim.shot(Limb::Torso, Bone::Spine, -0.22, 0.035, 9.0),
                190 => aim.blow(Limb::LegL, Bone::LegLowL, 0.12, 0.04),
                _ => None,
            });
            if let Some(hit) = hit {
                world.write_message(hit);
            }
        }
        rec.shoot();
    }
    let world = rec.world();
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    // Sorted before folding, for the reason `capture_preset` gives.
    let mut wets = world.query::<&WetCanvas>();
    let mut values: Vec<u64> = wets.iter(world).map(|w| w.digest()).collect();
    values.sort_unstable();
    for value in values {
        digest ^= value;
        digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let tick = world.resource::<GoreClock>().0;
    let frames = rec.finish();
    println!("skinned: frames={frames} ticks={tick} digest={digest:016x}");
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    mannequin::spawn(&mut commands, &mut meshes, &mut materials, &mut images, &mut bindposes);
    preset_scene::spawn_set(&mut commands, &mut meshes, &mut materials);
}
