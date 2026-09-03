//! **`examples/pooling.rs` on rails** — three channels through the same spot, rendered off-screen to
//! a PNG sequence, with a digest of what pooled.
//!
//! Run: `cargo run --release -p bevy_carnage --example capture_pooling -- --out /tmp/pooling`
//!
//! # The two obligations every recorder in this directory has
//!
//! **`TimeUpdateStrategy::ManualDuration`**, so the clip steps a constant amount per pumped frame
//! rather than by however long the frame took, and **`DepthPrepass`** on the camera the recorder
//! spawned, without which every slick renders as an opaque quad or not at all.
//!
//! # Unlike `capture_ribbons`, this one *does* print a digest
//!
//! Pooling is CPU-side and deterministic — that is the whole reason it is in the crate's core half
//! rather than behind `vfx` — so the final line is reproducible across two runs of the same binary.
//! Two runs disagreeing means something read a clock, an `Entity` or an `AssetId`. (The PNGs are a
//! different matter: `capture_carnage` proved they are not byte-reproducible on Apple silicon once
//! GPU particles are on screen. This clip has none, but the digest is over the model either way,
//! which is what makes it a gate that can fail for a real reason.)

use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy_carnage::{Bore, CarnageVfxPlugin, Pool};
use std::time::Duration;

mod common;
use common::body::{self, Chunk, ORIGIN};
use common::recorder::Recorder;
use common::{arg, light_and_floor};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 680;

/// Coarse, because the clip is about the floor: a body that falls apart takes the aim point with it.
const GRANULARITY: usize = 0;
const SOFTEN: f32 = 0.5;
const CALIBRE: f32 = 0.05;
const SHATTER: u32 = 6;

/// A constant step per pumped frame, matching every other recorder here.
const DT: f32 = 1.0 / 30.0 * 0.55;

/// Frames after the last shot, so the last plugs land, settle and spread visibly.
const TAIL: u32 = 150;

/// `(frame, where)` — **the same place three times**, which is the point: the first shot's stains are
/// discrete, and by the third they are one slick.
const SHOTS: [(u32, Vec3); 3] =
    [(8, Vec3::new(0.0, 0.10, 0.0)), (70, Vec3::new(0.04, 0.06, 0.0)), (132, Vec3::new(-0.03, 0.13, 0.0))];

fn main() {
    let out = arg("--out").unwrap_or_else(|| "frames-pooling".to_string());
    let camera = Transform::from_xyz(1.55, 1.55, 2.10).looking_at(ORIGIN - Vec3::Y * 0.70, Vec3::Y);
    let Some(mut rec) = Recorder::new_with(WIDTH, HEIGHT, camera, &out, |app| {
        app.add_plugins(CarnageVfxPlugin);
    }) else {
        return;
    };

    rec.world().insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(DT)));
    rec.world().init_resource::<body::Thrown>();
    rec.world().init_resource::<body::Pools>();

    let cameras: Vec<Entity> =
        rec.world().query_filtered::<Entity, With<Camera3d>>().iter(rec.world()).collect();
    for entity in cameras {
        rec.world().entity_mut(entity).insert(DepthPrepass);
    }

    light_and_floor(rec.world());
    let mut bores: Vec<Bore> = Vec::new();
    rebake(&mut rec, &bores);
    // `build_splats` is the plugin's own `Startup` system, so the splat textures exist before the
    // first slick asks for a material.
    rec.warm_up(4);

    // After the scene, so the frames before the first shot are perfectly still.
    rec.app().main.add_systems(Update, (integrate, body::bleed).chain());

    let last = SHOTS.iter().map(|(f, _)| *f).max().unwrap_or(0);
    for frame in 0..last + TAIL {
        for (at_frame, at) in SHOTS {
            if frame == at_frame {
                bores.push(body::bore_at(at, CALIBRE, SHATTER));
                info!("capture_pooling: frame {frame} — bore at {at:?}");
                rebake(&mut rec, &bores);
            }
        }
        rec.shoot();
    }

    let pools: Vec<Pool> = rec.world().resource::<body::Pools>().0.clone();
    let n = rec.finish();
    let line = format!("pooling: frames={n} pools={} digest={:016x}", pools.len(), digest(&pools));
    info!("{line}");
    println!("{line}");
}

/// FNV-1a over every pool's position, radius and wetted area, in placement order.
///
/// **Hand-rolled FNV, and the same constants `capture_carnage::Ledger::digest` uses**:
/// `DefaultHasher` is not guaranteed stable across toolchains, so it has no business producing a
/// number two runs on two machines are compared by. Raw bits rather than formatted floats, because
/// formatting rounds and a rounded digest would hide exactly the last-bit drift this looks for.
///
/// Placement order rather than sorted: the order *is* part of what is being checked. `absorb` only
/// ever pushes, so two runs producing the same pools in a different order have a different iteration
/// order somewhere — which is precisely the failure this exists to catch.
fn digest(pools: &[Pool]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |x: u32| {
        for byte in x.to_le_bytes() {
            h ^= byte as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    };
    for p in pools {
        eat(p.at[0].to_bits());
        eat(p.at[1].to_bits());
        eat(p.at[2].to_bits());
        eat(p.radius.to_bits());
        eat(p.wetted.to_bits());
        eat(p.seed);
    }
    h
}

/// Re-cut the subject with the accumulated channels, stand it back up, and throw what came out.
fn rebake(rec: &mut Recorder, bores: &[Bore]) {
    body::clear(rec.world());
    let baked = body::Baked::bake(rec.world(), SOFTEN, bores);
    let damage = body::Damage::fresh(&baked, GRANULARITY);
    rec.world().insert_resource(baked);
    let materials = body::BodyMaterials::new(rec.world());
    rec.world().insert_resource(materials);
    rec.world().insert_resource(damage);
    body::stand(rec.world(), GRANULARITY);
    body::spawn_gore(rec.world());
}

/// The recorder's own fixed step, so the arcs are a function of the frame number and nothing else.
fn integrate(mut chunks: Query<(&mut Chunk, &mut Transform)>) {
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, DT);
    }
}
