//! **Headless recorder for the flesh material** — the pixel half, in one clip.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example capture_flesh -- --out frames-flesh
//! ```
//!
//! Writes `frame0000.png …` and prints one digest line; two runs must print the same line (the
//! digest is over the CPU canvases the shader reads, which is the only state there is).
//!
//! | frames | what |
//! |---|---|
//! | 0–89 | a pale limb (banded cylinder) and a skin sphere under a light that walks past the terminator: Penner's wrap, per tissue |
//! | 90–239 | hits every 25 frames peel the sphere's canvas to bone and throw blood on it, the limb and the cloth |
//! | 240–299 | the blood dries: the clear coat fades, the cloth keeps its stain |
//!
//! Three subjects, three [`FleshMode`]s: the limb is `Cap` (its tissue is `UV_1`), the sphere is
//! `Canvas` (tissue from a flaymap, wetness from a wetmap), the sheet is `Cloth` (blood composited
//! over the fabric on the GPU). Native only.

use std::f32::consts::PI;

use bevy::prelude::*;
use bevy_carnage::cross_section::{CrossSectionPlugin, CrossSectionSystems};
use bevy_carnage::flaymap::{FlayCanvas, FlaymapPlugin};
use bevy_carnage::flesh::FleshPlugin;
use bevy_carnage::wetmap::{WetCanvas, WetSettings, WetmapPlugin};

mod common;
use common::flesh_scene::{self, Sun};
use common::recorder::Recorder;

const FRAMES: u32 = 300;

fn main() {
    let out = common::arg("--out").unwrap_or_else(|| "frames-flesh".to_string());
    let camera = Transform::from_xyz(0.0, 0.9, 2.6).looking_at(Vec3::new(0.0, 0.15, 0.0), Vec3::Y);
    let Some(mut rec) = Recorder::new_with(800, 500, camera, &out, |app| {
        app.add_plugins((CrossSectionPlugin, FlaymapPlugin, WetmapPlugin, FleshPlugin))
            .insert_resource(WetSettings { dry_ticks: 120, ..default() })
            .add_systems(Startup, flesh_scene::setup.after(CrossSectionSystems).after(bevy_carnage::flesh::FleshSystems));
    }) else {
        return;
    };
    {
        let world = rec.world();
        let mut cam = world.query_filtered::<Entity, With<Camera3d>>();
        if let Some(entity) = cam.iter(world).next() {
            world.entity_mut(entity).insert(AmbientLight { brightness: 300.0, ..default() });
        }
    }

    let mut tick: u32 = 0;
    for frame in 0..FRAMES {
        {
            let world = rec.world();
            // The light walks around the subjects so the terminator crosses every tissue.
            let angle = frame as f32 / FRAMES as f32 * 2.0 * PI;
            let mut suns = world.query_filtered::<&mut Transform, With<Sun>>();
            for mut t in suns.iter_mut(world) {
                *t = Transform::from_xyz(3.0 * angle.cos(), 2.5, 3.0 * angle.sin()).looking_at(Vec3::ZERO, Vec3::Y);
            }

            if (90..240).contains(&frame) && (frame - 90) % 25 == 0 {
                flesh_scene::hit(world, (frame - 90) / 25, tick);
            }
            let settings = world.resource::<WetSettings>().clone();
            let mut wets = world.query::<&mut WetCanvas>();
            for mut w in wets.iter_mut(world) {
                w.tick(tick, Vec2::new(0.0, 1.0), &settings);
            }
        }
        rec.shoot();
        tick += 1;
    }
    let world = rec.world();
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    let mut wets = world.query::<&WetCanvas>();
    for w in wets.iter(world) {
        digest ^= w.digest();
        digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut flays = world.query::<&FlayCanvas>();
    for f in flays.iter(world) {
        digest ^= f.digest();
        digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let frames = rec.finish();
    println!("flesh: frames={frames} digest={digest:016x}");
}

