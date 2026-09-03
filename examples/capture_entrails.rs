//! **`examples/entrails.rs` on rails** — the same subject, the same blows, one blow per scripted
//! frame, rendered off-screen to a PNG sequence.
//!
//! Run: `cargo run --release -p bevy_carnage --example capture_entrails -- --out /tmp/entrails`
//!
//! # One thing this file must set that `Recorder` does not
//!
//! **`TimeUpdateStrategy::ManualDuration` of exactly one fixed period.** `bevy_viscera` solves on
//! `FixedUpdate` at 60 Hz and never reads a clock, but *how many* fixed steps a pumped frame runs is
//! decided by `Time<Virtual>`, which a hand-pumped loop advances by however long the frame took. One
//! period per pumped frame makes that exactly one solver tick per captured frame, so the fall is a
//! function of the frame number and nothing else.
//!
//! # A digest line, unlike `capture_ribbons`
//!
//! There is something CPU-side to fold here: `Strand::digest` is FNV-1a over the nodes, and the
//! solver is fixed-substep, fixed-iteration, fixed-order by construction. Folded in spawn order
//! rather than query order, because ECS query order is not stable across `App` instances.

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy_viscera::{
    Mesentery, SPILL_SEGMENTS, Strand, ViscSettings, VisceraPlugin,
    VisceraSystems, spill, tube_mesh,
};
use std::time::Duration;

mod common;
use common::body::{self, Blow, Chunk, ORIGIN};
use common::recorder::Recorder;
use common::{arg, light_and_floor};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 680;

/// The frontier the clip stands at — the finest, so a blow takes small torso pieces off.
const GRANULARITY: usize = body::GRANULARITIES.len() - 1;
/// Rounded, so the chunks read as torn rather than as cleaved ice.
const SOFTEN: f32 = 0.5;

/// The chunk integrator's step per pumped frame, matching `capture_ribbons`. The `0.55` is the same
/// slow factor the shared integrator applies, so the arcs match the windowed demo's.
const DT: f32 = 1.0 / 30.0 * 0.55;

/// One solver tick per pumped frame. See the module docs.
const FIXED_HZ: f64 = 60.0;

/// Frames after the last blow, so the strands finish falling, drag and tear rather than being cut off
/// mid-air.
const TAIL: u32 = 150;

/// Sides on the swept tube, and the strand budget — both matching `entrails.rs`.
const SIDES: u32 = 8;
const STRANDS_PER_CHUNK: u32 = 2;
const MAX_GUTS: usize = 8;
const PINNED_NODES: u32 = 3;

/// **The strain at which the gut parts from the piece it came out of**, against the membrane's
/// `DEFAULT_TEAR_STRAIN` of 0.35.
///
/// A gib leaves at metres per second, and a compliant pin lags a moving anchor — so at 0.35 (twelve
/// millimetres, a fold of peritoneum) every tether parts on the first tick and nothing streams:
/// measured, 24 of 24 links torn and eight straight rods left in the air. This pin is not a fold of
/// peritoneum. It is the gut's own continuity with the piece of abdomen it is still inside, and it
/// gets the one threshold [`Mesentery`] exposes set for what is actually holding it — the same
/// reasoning `bevy_viscera`'s `viscera_spill` applies to a hand, which is not a membrane either.
const CHUNK_TEAR_STRAIN: f32 = 8.0;

/// `(frame, blow, where)` — abdomen height, so the pieces that come off are torso.
const SCRIPT: [(u32, Blow, Vec3); 3] = [
    (10, Blow::Projectile, Vec3::new(0.00, 0.10, 0.0)),
    (60, Blow::Slash, Vec3::new(0.06, -0.06, 0.0)),
    (110, Blow::Blast, Vec3::new(0.00, 0.10, 0.0)),
];

/// One spilled strand, its chunk, and the anchor offsets in that chunk's frame.
#[derive(Component)]
struct Entrail {
    chunk: Entity,
    offsets: Vec<Vec3>,
    /// Spawn order, so the digest is folded in a stable order rather than in query order.
    index: u32,
}

/// Marks a chunk that has already been considered for a spill.
#[derive(Component)]
struct Spilled;

/// The gut material, made once.
#[derive(Resource)]
struct GutMaterial(Handle<StandardMaterial>);

/// How many strands have been spawned, which is the next one's index.
#[derive(Resource, Default)]
struct Spawned(u32);

fn main() {
    let out = arg("--out").unwrap_or_else(|| "frames-entrails".to_string());
    let camera = Transform::from_xyz(2.05, 1.30, 2.70).looking_at(ORIGIN - Vec3::Y * 0.16, Vec3::Y);
    let Some(mut rec) = Recorder::new_with(WIDTH, HEIGHT, camera, &out, |app| {
        app.add_plugins(VisceraPlugin);
    }) else {
        return;
    };

    rec.world().insert_resource(Time::<Fixed>::from_hz(FIXED_HZ));
    rec.world().insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / FIXED_HZ,
    )));

    light_and_floor(rec.world());
    let baked = body::Baked::bake(rec.world(), SOFTEN, &[], &[GRANULARITY]);
    let damage = body::Damage::fresh(&baked, GRANULARITY);
    rec.world().insert_resource(baked);
    let materials = body::BodyMaterials::new(rec.world());
    rec.world().insert_resource(materials);
    rec.world().insert_resource(damage);
    body::stand(rec.world(), GRANULARITY);

    let gut = rec.world().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: Color::srgb(0.52, 0.16, 0.15),
        perceptual_roughness: 0.28,
        ..default()
    });
    rec.world().insert_resource(GutMaterial(gut));
    rec.world().init_resource::<Spawned>();
    rec.warm_up(4);

    // Added after the scene, so the frames before the first blow are perfectly still.
    rec.app().main.add_systems(Update, (spill_entrails, integrate).chain());
    rec.app().main.add_systems(
        FixedUpdate,
        (carry_anchors.before(VisceraSystems), rebuild_tubes.after(VisceraSystems)),
    );

    let last = SCRIPT.iter().map(|(f, _, _)| *f).max().unwrap_or(0);
    for frame in 0..last + TAIL {
        for (at_frame, blow, at) in SCRIPT {
            if frame == at_frame {
                body::strike(rec.world(), blow, at);
            }
        }
        rec.shoot();
    }

    let mut folded: Vec<(u32, u64)> = rec
        .world()
        .query::<(&Entrail, &Strand)>()
        .iter(rec.world())
        .map(|(e, s)| (e.index, s.digest()))
        .collect();
    // SORT-OK: by spawn index, which is unique per strand.
    folded.sort_unstable_by_key(|(i, _)| *i);
    let digest = folded.iter().fold(0xcbf2_9ce4_8422_2325u64, |acc, (_, d)| {
        (acc ^ d).wrapping_mul(0x1000_0000_01b3)
    });
    let torn: usize = rec
        .world()
        .query::<&Mesentery>()
        .iter(rec.world())
        .map(|m| m.torn.iter().filter(|x| **x).count())
        .sum();

    let n = rec.finish();
    info!("entrails: frames={n} strands={} torn={torn} digest=0x{digest:016x}", folded.len());
    println!("entrails: frames={n} strands={} torn={torn} digest=0x{digest:016x}", folded.len());
}

/// The same consumer-side decision `examples/entrails.rs` makes: a torso fragment pulls guts out.
fn spill_entrails(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    gut: Res<GutMaterial>,
    settings: Res<ViscSettings>,
    baked: Res<body::Baked>,
    mut spawned: ResMut<Spawned>,
    live: Query<(), With<Entrail>>,
    fresh: Query<(Entity, &Chunk, &Transform), Without<Spilled>>,
) {
    let mut budget = MAX_GUTS.saturating_sub(live.iter().count());
    for (entity, chunk, transform) in &fresh {
        commands.entity(entity).insert(Spilled);
        if budget == 0 {
            continue;
        }
        let Some(id) = chunk.fragment else { continue };
        if body::part_of(id, &baked.tree) != "torso" {
            continue;
        }
        let at = transform.translation;
        let exit = (chunk.velocity.normalize_or_zero() - Vec3::Y * 0.3).normalize_or_zero();
        let seed = at.x.to_bits() ^ at.z.to_bits().rotate_left(16) ^ (id.index() as u32);
        for strand in spill(at, exit, STRANDS_PER_CHUNK, seed, &settings) {
            let anchors: Vec<(u32, Vec3)> = strand
                .nodes()
                .iter()
                .enumerate()
                .filter(|(n, _)| (*n as u32) < PINNED_NODES.min(SPILL_SEGMENTS))
                .map(|(n, p)| (n as u32, *p))
                .collect();
            let offsets: Vec<Vec3> = anchors
                .iter()
                .map(|(_, p)| transform.rotation.inverse() * (*p - transform.translation))
                .collect();
            let torn = vec![false; anchors.len()];
            let index = spawned.0;
            spawned.0 = spawned.0.wrapping_add(1);
            commands.spawn((
                Mesh3d(meshes.add(tube_mesh(&strand, SIDES))),
                MeshMaterial3d(gut.0.clone()),
                Mesentery { anchors, torn, tear_strain: CHUNK_TEAR_STRAIN },
                strand,
                Entrail { chunk: entity, offsets, index },
            ));
            budget = budget.saturating_sub(1);
            if budget == 0 {
                break;
            }
        }
    }
}

/// Carry each tether's anchors on the chunk that owns them, in that chunk's own frame.
fn carry_anchors(
    chunks: Query<&Transform, With<Chunk>>,
    mut guts: Query<(&Entrail, &mut Mesentery)>,
) {
    for (entrail, mut tether) in &mut guts {
        let Ok(chunk) = chunks.get(entrail.chunk) else { continue };
        for (slot, (_, at)) in tether.anchors.iter_mut().enumerate() {
            let Some(offset) = entrail.offsets.get(slot) else { continue };
            *at = chunk.translation + chunk.rotation * *offset;
        }
    }
}

/// Rebuild each tube from the nodes the solver just moved.
fn rebuild_tubes(mut meshes: ResMut<Assets<Mesh>>, guts: Query<(&Strand, &Mesh3d)>) {
    for (strand, handle) in &guts {
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            *mesh = tube_mesh(strand, SIDES);
        }
    }
}

/// The recorder's own fixed step, so the arcs are a function of the frame number and nothing else.
fn integrate(mut chunks: Query<(&mut Chunk, &mut Transform)>) {
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, DT);
    }
}
