//! **Headless recorder for the preset** — the two-line inclusion, scripted.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example capture_preset -- --out frames-preset
//! ```
//!
//! Writes `frame0000.png …` and prints one digest line; two runs must print the same line.
//!
//! | frames | what |
//! |---|---|
//! | 0–29 | the body stands, dressed in the flesh material |
//! | 30 | a blow to the left arm (a bruise under the skin), a hot iron on the right thigh (a burn), a slash across the left thigh |
//! | 60–119 | a shot every 30 frames on the torso, at one spot: peel, blood on the body, the sheet and the slab, a bleed that runs and dries; the bruise ages in time-lapse |
//! | 120– | the third shot shows cortex and hands off — the body parts along its fracture modes, caps banded, and the pieces fall |
//!
//! Every visual is the preset's: `GorePlugin`, a `Gore` per surface, a `GoreHit` per shot.

use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy_carnage::preset::{GoreClock, GorePlugin};
use bevy_carnage::wetmap::WetCanvas;

mod common;
use common::preset_scene::{self, Part};
use common::recorder::Recorder;

const FRAMES: u32 = 360;

fn main() {
    let out = common::arg("--out").unwrap_or_else(|| "frames-preset".to_string());
    let camera = Transform::from_xyz(1.1, 1.5, 2.9).looking_at(Vec3::new(0.15, 0.7, 0.0), Vec3::Y);
    let Some(mut rec) = Recorder::new_with(800, 500, camera, &out, |app| {
        // **The clock is the frame, not the wall.** The preset runs on `FixedUpdate`; with the default
        // real-time strategy the number of fixed ticks per recorded frame depends on how long the
        // screenshot took, and the digest would depend on the machine. One sixtieth per update, and
        // a 60 Hz fixed step, is exactly one tick per frame.
        app.add_plugins(GorePlugin)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)))
            .insert_resource(Time::<Fixed>::from_hz(60.0))
            .add_systems(Startup, setup);
    }) else {
        return;
    };
    {
        let world = rec.world();
        let mut cam = world.query_filtered::<Entity, With<Camera3d>>();
        if let Some(entity) = cam.iter(world).next() {
            world.entity_mut(entity).insert(AmbientLight { brightness: 260.0, ..default() });
        }
    }

    // Frame 0 is captured on the update that first populates the scene, so it comes back blank —
    // and a GIF's still preview is its first frame. The flesh material's pipeline is specialised on
    // its first draw and the cache compiles it off-thread, so this needs more frames than the
    // family clip's two: measured, frame 0 was still blank at two and stable at six.
    rec.warm_up(6);
    for frame in 0..FRAMES {
        {
            let world = rec.world();
            let part = |world: &mut World, i: usize| {
                let mut parts = world.query::<(Entity, &Part)>();
                parts.iter(world).find(|(_, p)| p.0 == i).map(|(e, _)| e)
            };
            if frame == 30 {
                if let Some(arm) = part(world, 2) {
                    world.write_message(preset_scene::blow(arm));
                }
                if let Some(leg) = part(world, 5) {
                    world.write_message(preset_scene::scald(leg));
                }
                if let Some(leg) = part(world, 4) {
                    world.write_message(preset_scene::slash(leg));
                }
            }
            if frame >= 60 && (frame - 60) % 30 == 0 {
                let n = (frame - 60) / 30;
                if let Some(torso) = part(world, 0) {
                    world.write_message(preset_scene::shot(torso, n));
                }
            }
        }
        rec.shoot();
    }
    let world = rec.world();
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    // Sorted before folding: XOR-then-multiply is order-dependent and ECS query order is not a
    // total order across two `App`s, which is exactly the comparison this line exists for.
    let mut wets = world.query::<&WetCanvas>();
    let mut values: Vec<u64> = wets.iter(world).map(|w| w.digest()).collect();
    values.sort_unstable();
    for value in values {
        digest ^= value;
        digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let tick = world.resource::<GoreClock>().0;
    let frames = rec.finish();
    println!("preset: frames={frames} ticks={tick} digest={digest:016x}");
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    preset_scene::spawn(&mut commands, &mut meshes, &mut materials);
}
