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
use common::body::{self, ORIGIN, SHOTS};
use common::{Recorder, arg, light_and_floor};
use bevy_autogib::Bore;

/// Capture size, matching the other recorders so the GIFs sit together on a page.
const WIDTH: u32 = 720;
const HEIGHT: u32 = 540;

/// **Rendered flat, and that is a measurement rather than a taste.** `soften` relaxes each
/// fragment's drawn skin *independently*, so where two shards share a boundary the two relaxations
/// pull apart and a hairline opens along every wedge boundary radiating from a hole — which in a clip
/// about holes reads as cracks. At `0.0` the shards share their boundary vertices exactly and the
/// only opening in the subject is the one that was bored.
const SOFTEN: f32 = 0.0;

/// The finest frontier: index into [`body::GRANULARITIES`]. The coarsest, because this clip is about
/// the holes and not about the fracture — at index 0 the standing pieces are the bore's own shards
/// plus the body parts, with no fracture cut between them.
const GRANULARITY: usize = 0;

/// Frames to hold after the last shot before the camera starts moving.
const TAIL: u32 = 12;

/// Frames of orbit at the end — a third of a turn, so the exit side comes into view.
const ORBIT: u32 = 56;

fn main() {
    let out = arg("--out").unwrap_or_else(|| "frames-holes".to_string());
    // **A closer camera than the other four clips, on purpose.** The shared framing exists so the
    // *body* clips are comparable to each other; this one is not one of them. A 0.035 hole on a
    // 1.0-tall subject is about 16 px at that distance, which is a smudge — so this sits nearer and
    // gives up the comparison.
    let camera = Transform::from_xyz(1.15, 1.05, 1.55).looking_at(ORIGIN, Vec3::Y);
    let Some(mut rec) = Recorder::new(WIDTH, HEIGHT, camera, &out) else { return };

    light_and_floor(rec.world());
    let mut bores: Vec<Bore> = Vec::new();
    rebake(&mut rec, &bores);
    rec.warm_up(4);

    let last = SHOTS.last().map(|(f, _, _)| *f).unwrap_or(0);
    for frame in 0..last + TAIL + ORBIT {
        for (at_frame, at, radius) in SHOTS {
            if frame == at_frame {
                // **Every shot re-bakes**, because a bore is a bake input: the channel is part of the
                // subject's shape, so a new hole is a new subject rather than an edit to this one.
                bores.push(body::bore_at(at, radius));
                info!("capture_holes: frame {frame} — bore at {at:?}, radius {radius}");
                rebake(&mut rec, &bores);
            }
        }
        // The orbit runs after the last shot has settled. Writing the camera transform straight from
        // the loop keeps this a script rather than a system with a resource behind it.
        if frame >= last + TAIL {
            let t = (frame - (last + TAIL)) as f32 / ORBIT as f32;
            let angle = t * std::f32::consts::TAU / 3.0;
            let (s, c) = angle.sin_cos();
            let (x, z) = (1.15 * c + 1.55 * s, 1.55 * c - 1.15 * s);
            let moved = Transform::from_xyz(x, 1.05, z).looking_at(ORIGIN, Vec3::Y);
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

/// Re-cut the subject with the accumulated channels and stand it back up.
///
/// The same sequence `sever`'s `T` key performs: clear, bake, fresh damage, stand. Shared with
/// nothing because it *is* the script — what differs between the two recorders is only this.
fn rebake(rec: &mut Recorder, bores: &[Bore]) {
    body::clear(rec.world());
    let baked = body::Baked::bake(rec.world(), SOFTEN, bores);
    let damage = body::Damage::fresh(&baked, GRANULARITY);
    rec.world().insert_resource(baked);
    let materials = body::BodyMaterials::new(rec.world());
    rec.world().insert_resource(materials);
    rec.world().insert_resource(damage);
    body::stand(rec.world(), GRANULARITY);
}
