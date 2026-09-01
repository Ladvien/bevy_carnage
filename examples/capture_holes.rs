//! **Bullet holes, on rails, rendered headless — the GIF `docs/holes.gif` is made of.**
//!
//! Five shots go through the subject and the channels stay: each one is a convex prism subtracted
//! from the proxy before the cut, so the hole is geometry with a red wall rather than a decal, and
//! the pieces around it stay bonded so the subject keeps standing.
//!
//! **The scene and the bake are `common::body`, not copies of them.** `bullet_holes.rs` fires the
//! same [`body::bore_at`] at the same [`body::SHOTS`] on a keypress, so the clip is that example on
//! rails rather than a re-implementation that could drift.
//!
//! | frame | what |
//! |---|---|
//! | 0 | intact, no holes |
//! | 12 | a shot through the torso, high right |
//! | 32 | one through the torso, low left |
//! | 52 | one through the head |
//! | 72 | one through the torso, low right |
//! | 92 | one through the left arm |
//! | 104 → | a third of a turn, so the channels read as depth rather than as dark discs |
//!
//! Frames land in `--out <dir>` (default `frames-holes/`). Turn them into a GIF with `tools/gif.sh`.
//!
//! Run: `cargo run --release --example capture_holes -- --out /tmp/frames-holes`

use bevy::prelude::*;

mod common;
use common::body::{self, Chunk, ORIGIN, SHOTS};
use common::{Recorder, arg, light_and_floor};
use bevy_carnage::Bore;

/// Capture size, matching the other recorders so the GIFs sit together on a page.
const WIDTH: u32 = 720;
const HEIGHT: u32 = 540;

/// **The body is rendered flat, and that is a measurement rather than a taste.** `soften` relaxes
/// each fragment's drawn skin *independently* and does not pin the boundary it shares with its
/// neighbour, so on a bored subject the wedges around a channel pull apart. Measured at 0.40: the
/// eight shards of every hole separate outright, red gaps radiate from each entry wound, and the
/// subject reads as disassembled rather than shot — much worse than the hairline AG-022 predicted.
/// At `0.0` the shards share their boundary vertices exactly and the only opening is the bore.
///
/// **The gore is rounded anyway**, on `CutSettings::ejecta_soften`, which `Baked::bake` leaves at its
/// shipped 0.55. Debris shares a boundary with nothing, so nothing can open up beside it.
const SOFTEN: f32 = 0.0;

/// The finest frontier: index into [`body::GRANULARITIES`]. The coarsest, because this clip is about
/// the holes and not about the fracture — at index 0 the standing pieces are the bore's own shards
/// plus the body parts, with no fracture cut between them.
const GRANULARITY: usize = 0;

/// Frames to hold after the last shot before the camera starts moving.
const TAIL: u32 = 12;

/// Frames of orbit at the end — a third of a turn, so the exit side comes into view.
const ORBIT: u32 = 56;

/// **Fixed timestep**, the reason a recorder exists alongside a windowed demo: the gore's
/// trajectories are then a pure function of the seed rather than of how fast the machine rendered.
/// The same constant `capture_sever` uses.
const DT: f32 = 1.0 / 30.0 * 0.55;

fn main() {
    let out = arg("--out").unwrap_or_else(|| "frames-holes".to_string());
    // **A closer camera than the other four clips, on purpose.** The shared framing exists so the
    // *body* clips are comparable to each other; this one is not one of them. A 0.035 hole on a
    // 1.0-tall subject is about 16 px at that distance, which is a smudge — so this sits nearer and
    // gives up the comparison.
    // Tilted down a little from the AG-022 framing, because the clip now has to show the floor: the
    // plugs land about 1.2 units out and the pools they leave are the end of the story.
    let camera = Transform::from_xyz(1.30, 1.20, 1.75).looking_at(ORIGIN - Vec3::Y * 0.16, Vec3::Y);
    let Some(mut rec) = Recorder::new(WIDTH, HEIGHT, camera, &out) else { return };

    light_and_floor(rec.world());
    let mut bores: Vec<Bore> = Vec::new();
    // Nothing has been thrown yet. The counter lives in the world so `spawn_gore` stays idempotent
    // across the four re-bakes this clip performs.
    rec.world().init_resource::<body::Thrown>();
    rebake(&mut rec, &bores);
    rec.warm_up(4);
    // Added after the scene, so the frames before the first shot are perfectly still — the same
    // ordering trick `capture_sever` uses for its intact frames.
    rec.app().main.add_systems(Update, (integrate, body::bleed).chain());

    let last = SHOTS.last().map(|(f, _, _, _)| *f).unwrap_or(0);
    for frame in 0..last + TAIL + ORBIT {
        for (at_frame, at, radius, shatter) in SHOTS {
            if frame == at_frame {
                // **Every shot re-bakes**, because a bore is a bake input: the channel is part of the
                // subject's shape, so a new hole is a new subject rather than an edit to this one.
                bores.push(body::bore_at(at, radius, shatter));
                info!("capture_holes: frame {frame} — bore at {at:?}, radius {radius}, plug into {shatter}");
                rebake(&mut rec, &bores);
            }
        }
        // The orbit runs after the last shot has settled. Writing the camera transform straight from
        // the loop keeps this a script rather than a system with a resource behind it.
        if frame >= last + TAIL {
            let t = (frame - (last + TAIL)) as f32 / ORBIT as f32;
            let angle = t * std::f32::consts::TAU / 3.0;
            let (s, c) = angle.sin_cos();
            let (x, z) = (1.30 * c + 1.75 * s, 1.75 * c - 1.30 * s);
            // **Pulling back and rising as it goes round, which the AG-022 orbit did not have to do.**
            // The gore flies out the exit side, so a third of a turn puts the camera on the same side
            // as the pools — and at the fixed radius they ended up a few tenths of a unit in front of
            // the lens, i.e. out of frame entirely. Backing off to 1.6x and lifting while aiming lower
            // keeps the exit wounds and the stains they left in the same shot, which is the whole
            // point of ending here.
            let out = 1.0 + 0.45 * t;
            let look = ORIGIN - Vec3::Y * (0.16 + 0.30 * t);
            let moved =
                Transform::from_xyz(x * out, 1.20 + 0.48 * t, z * out).looking_at(look, Vec3::Y);
            let mut cams = rec.world().query_filtered::<&mut Transform, With<Camera3d>>();
            for mut cam in cams.iter_mut(rec.world()) {
                *cam = moved;
            }
        }
        rec.shoot();
    }
    let n = rec.finish();
    info!("capture_holes: wrote {n} frames to {out}");
}

/// Re-cut the subject with the accumulated channels, stand it back up, and throw what came out.
///
/// The same sequence `sever`'s `T` key performs, plus [`body::spawn_gore`]. `clear` deliberately
/// leaves gore and pools alone, and `spawn_gore` only throws plugs it has not thrown before — so
/// re-baking on every shot neither recalls debris already in the air nor duplicates it.
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

/// Gravity, a ground bounce and tumbling for the ejected plugs — on a fixed `DT`, so the run is
/// reproducible. The standing shards are never touched: they are attached, not chunks.
fn integrate(mut chunks: Query<(&mut Chunk, &mut Transform)>) {
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, DT);
    }
}
