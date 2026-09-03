//! **`examples/ribbons.rs` on rails** — the same subject, the same blows, one blow per scripted
//! frame, rendered off-screen to a PNG sequence.
//!
//! Run: `cargo run --release -p bevy_carnage --example capture_ribbons -- --out /tmp/ribbons`
//!
//! # Two things this file must repeat that `Recorder` does not do
//!
//! **`TimeUpdateStrategy::ManualDuration`.** Hanabi's particle clock is derived from `Time<Virtual>`,
//! which is derived from `Time<Real>`. A hand-pumped loop advances real time by however long the
//! frame took, so without this the strands would be stepped by wall-clock and the clip would look
//! different on a slower machine. `capture_carnage.rs` carries the same line for the same reason.
//!
//! **`DepthPrepass` on the recorder's camera.** `CarnageVfxPlugin` brings the decal half along with
//! the particles, and a forward decal without a prepass renders as an opaque quad or not at all.
//! `Recorder::new_with` spawns a bare `Camera3d`.
//!
//! # No digest line, deliberately
//!
//! There is nothing CPU-side to fold. Particles are output only — Hanabi 0.19 has no GPU→CPU readback
//! path at all — so the one thing this clip shows cannot reach a hash. And the PNGs themselves are
//! already proven **not** byte-reproducible on Apple silicon with GPU particles on screen: two runs of
//! `capture_carnage`'s own binary differ in 202 of 382 frames while reporting the same digest. A
//! digest here would be a gate that cannot fail for a real reason, which is worse than no gate.

use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy_carnage::{BleedingChunk, CarnageVfxPlugin};
use std::time::Duration;

mod common;
use common::body::{self, Blow, Chunk, ORIGIN};
use common::recorder::Recorder;
use common::{arg, light_and_floor};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 680;

/// The frontier the clip stands at — the finest, because the point is many small pieces in the air.
const GRANULARITY: usize = body::GRANULARITIES.len() - 1;
/// Rounded, so the chunks read as torn rather than as cleaved ice.
const SOFTEN: f32 = 0.5;

/// **A constant step per pumped frame**, matching `capture_carnage`'s. The `0.55` is the same slow
/// factor the shared integrator applies, so the arcs match the windowed demo's.
const DT: f32 = 1.0 / 30.0 * 0.55;

/// Frames to keep rolling after the last blow, so the final strands finish fading rather than being
/// cut off mid-air. Comfortably longer than `RIBBON_LIFETIME` plus the fade.
const TAIL: u32 = 90;

/// `(frame, blow, where)` — one blow at a time, spaced so each set of strands is legible before the
/// next arrives.
const SCRIPT: [(u32, Blow, Vec3); 4] = [
    (10, Blow::Projectile, Vec3::new(0.00, 0.46, 0.0)),
    (46, Blow::Slash, Vec3::new(-0.32, 0.06, 0.0)),
    (82, Blow::SweptBlade, Vec3::new(0.32, 0.06, 0.0)),
    (118, Blow::Blast, Vec3::new(0.00, -0.30, 0.0)),
];

fn main() {
    let out = arg("--out").unwrap_or_else(|| "frames-ribbons".to_string());
    let camera = Transform::from_xyz(2.05, 1.30, 2.70).looking_at(ORIGIN - Vec3::Y * 0.16, Vec3::Y);
    // Plugins must go in before the recorder finishes building: `CarnageVfxPlugin` brings in Hanabi,
    // which registers render pipelines and extraction systems, and an `App` will not accept a plugin
    // after `finish`/`cleanup`.
    let Some(mut rec) = Recorder::new_with(WIDTH, HEIGHT, camera, &out, |app| {
        app.add_plugins(CarnageVfxPlugin);
    }) else {
        return;
    };

    rec.world().insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(DT)));

    let cameras: Vec<Entity> =
        rec.world().query_filtered::<Entity, With<Camera3d>>().iter(rec.world()).collect();
    for entity in cameras {
        rec.world().entity_mut(entity).insert(DepthPrepass);
    }

    light_and_floor(rec.world());
    let baked = body::Baked::bake(rec.world(), SOFTEN, &[], &[GRANULARITY]);
    let damage = body::Damage::fresh(&baked, GRANULARITY);
    rec.world().insert_resource(baked);
    let materials = body::BodyMaterials::new(rec.world());
    rec.world().insert_resource(materials);
    rec.world().insert_resource(damage);
    body::stand(rec.world(), GRANULARITY);

    // `build_effects` and `build_splats` are the plugin's own `Startup` systems, so they run on the
    // first pumped frames — the ribbon asset has to exist before the first chunk flies.
    rec.warm_up(4);

    // Added after the scene, so the frames before the first blow are perfectly still — the ordering
    // trick every recorder in this directory uses.
    rec.app().main.add_systems(Update, (mark_bleeding, integrate).chain());

    let last = SCRIPT.iter().map(|(f, _, _)| *f).max().unwrap_or(0);
    for frame in 0..last + TAIL {
        for (at_frame, blow, at) in SCRIPT {
            if frame == at_frame {
                body::strike(rec.world(), blow, at);
            }
        }
        rec.shoot();
    }

    let n = rec.finish();
    info!("ribbons: frames={n}");
    println!("ribbons: frames={n}");
}

/// The same consumer-side decision `examples/ribbons.rs` makes: in the air bleeds, landed does not.
fn mark_bleeding(
    mut commands: Commands,
    chunks: Query<(Entity, &Chunk, &Transform, Option<&BleedingChunk>)>,
) {
    for (entity, chunk, transform, bleeding) in &chunks {
        let grounded = transform.translation.y <= chunk.drop_to_rest + 1.0e-3;
        match (grounded, bleeding.is_some()) {
            (false, false) => {
                commands.entity(entity).insert(BleedingChunk);
            }
            (true, true) => {
                commands.entity(entity).remove::<BleedingChunk>();
            }
            _ => {}
        }
    }
}

/// The recorder's own fixed step, so the arcs are a function of the frame number and nothing else.
fn integrate(mut chunks: Query<(&mut Chunk, &mut Transform)>) {
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, DT);
    }
}
