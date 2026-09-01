//! **`carnage`, on rails, rendered headless — and the determinism check for the whole chain.**
//!
//! Two jobs, and the second is the reason this file is worth more than a GIF.
//!
//! **It records `docs/carnage.gif`.** The same subject, the same blows and the same channel the
//! windowed `carnage` example fires on a keypress, on a fixed timestep and a fixed script, so frame
//! 62 of one run is frame 62 of the next.
//!
//! **It prints a digest, and two runs must print the same one.** The final line is
//!
//! ```text
//! carnage: frames=<n> wounds=<n> stains=<n> digest=<hex>
//! ```
//!
//! where the digest is FNV-1a over every stain position in placement order. That covers the entire new
//! layer end to end: the bake, the bond graph, wound extraction and its canonical sort, the wound
//! seed, the droplet draws, the ballistic solve and the pulse schedule. **A digest that differs between
//! two runs means something in that chain read a clock, an `Entity`, or an unsorted iteration order**
//! — the three failures the determinism contract names — and it is a much sharper instrument than
//! looking at two GIFs.
//!
//! Run it twice and `diff` the two lines. That is the check.
//!
//! | frame | what |
//! |---|---|
//! | 0 | intact, standing at the finest frontier |
//! | 18 | a channel straight through the chest — the hole bleeds from its own wall |
//! | 54 | a projectile to the left shoulder |
//! | 90 | one to the head |
//! | 126 | a slash at the right shoulder |
//! | 162 | a swept blade through the waist, taking the legs |
//! | 162 → | the tail: every severed piece keeps pulsing until it clots, stains piling up |
//!
//! # Two things this recorder needs that the fracture recorders do not
//!
//! **`TimeUpdateStrategy::ManualDuration`.** The particle system simulates against
//! `Time<EffectSimulation>`, whose own docs define its speed relative to `Time<Virtual>`, which
//! advances from `Time<Real>`. A hand-pumped loop with no winit runner advances real time by however
//! long the frame took, so the particles would step by a wall-clock amount and the recording would not
//! be reproducible. Pinning the real clock to a constant fixes the whole chain with one resource.
//!
//! **`DepthPrepass` on the recorder's camera.** `Recorder::new` spawns it as
//! `(Camera3d, RenderTarget, camera)` — no prepass — and a forward decal without one renders as an
//! opaque quad or not at all. So the stains would be missing from the very clip that exists to show
//! them.
//!
//! Frames land in `--out <dir>` (default `frames-carnage/`). Turn them into a GIF with `tools/gif.sh`.
//!
//! Run: `cargo run --release --example capture_carnage -- --out frames-carnage`

use std::time::Duration;

use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy_carnage::{
    Bleed, BondId, CarnagePlugin, CarnageSettings, CarnageVfxPlugin, FragmentId, SplatTextures,
    Stain, Wound, WoundKind, Wounded, clotted, largest_cap, pulse_wound, spawn_stain, stains,
    wound_of_channel, wounds_from_bonds,
};

mod common;
use common::body::{self, Blow, Chunk, ORIGIN};
use common::{Recorder, arg, light_and_floor};

/// Capture size, matching the other recorders so the GIFs sit together on a page.
const WIDTH: u32 = 720;
const HEIGHT: u32 = 540;

/// **Fixed timestep** — the reason a recorder exists alongside a windowed demo. The same constant the
/// other two use, so the four clips move at one speed.
const DT: f32 = 1.0 / 30.0 * 0.55;

/// The fixed-tick rate the bleed schedule is driven at. The shipped [`CarnageSettings`] tick counts
/// are derived for 60 Hz.
const HZ: u32 = 60;

/// The finest frontier — index into [`body::GRANULARITIES`]. A fine frontier means each blow takes
/// small pieces off, and each one is a wound worth bleeding.
const GRANULARITY: usize = 3;

/// Rendered flat, for the reason `capture_holes.rs` measures: relaxing each shard's skin independently
/// pulls the wedges around a channel apart, and this clip fires one.
const SOFTEN: f32 = 0.0;

/// The floor plane stains land on, in world space.
const FLOOR_Y: f32 = 0.0;

/// The calibre the channel shot uses. Mid-range on `bullet_holes`' own dial.
const CALIBRE: f32 = 0.035;

/// **The script: the channel first, then four severing blows.**
///
/// The clip answers the question the crate exists to answer — a bullet hole and a severance are
/// geometrically different openings that bleed through the same code — so it needs both, and this is
/// the only order in which it gets both.
///
/// **The shot must come first, because a bore re-bakes.** A channel is a bake *input*, not damage
/// applied afterwards, so firing one re-cuts the subject from scratch and the accumulated severance
/// goes with it — the same reason `sever.rs` has no bore key, recorded in `bullet_holes.rs`. Shot
/// last, the clip ended on an intact bored body with its dismemberment undone; shot first, the hole
/// persists (the bore list is kept) and the blows pile up on top of it, so the last frame carries
/// everything the clip did.
const SHOT: (u32, Vec3) = (18, Vec3::new(0.06, 0.20, 0.0));

/// Where each severing blow lands and what kind it is, after the channel.
const SCRIPT: [(u32, Blow, Vec3); 4] = [
    (54, Blow::Projectile, Vec3::new(-0.30, 0.16, 0.0)),
    (90, Blow::Projectile, Vec3::new(0.00, 0.48, 0.0)),
    (126, Blow::Slash, Vec3::new(0.30, 0.16, 0.0)),
    (162, Blow::SweptBlade, Vec3::new(0.00, -0.30, 0.0)),
];

/// The spatter speed scale, **matching `carnage.rs` exactly** — the recorder is that example on
/// rails, and a different dial here would make the GIF a picture of something you cannot reproduce
/// by keypress. See `carnage.rs`'s own constant for the 44-metre arithmetic behind the number.
const SPEED_SCALE: f32 = 0.25;

/// Frames to keep rolling after the last blow.
///
/// **Long enough to contain a clot, which is what makes it 220 rather than a round 100.** The shipped
/// `clot_ticks` is 360, so a wound opened on frame 18 stops bleeding on frame 378 — inside a
/// `162 + 220` clip. So the tail shows the whole arc the schedule describes: full arterial pulses at
/// roughly 1.6 per second, a taper, and then nothing, while the last blow's wounds are still going.
/// A shorter tail would end mid-spurt, and the clot would be a claim the clip does not support.
const TAIL: u32 = 220;

/// **How many stain decals may be on the floor at once.**
///
/// A cap rather than a lifetime: a stain that faded would say the blood dried. Generous enough that
/// the last frame of this clip still carries the first blow's blood, which is one of the things the
/// GIF is checked for.
const MAX_STAINS: usize = 900;

/// Live stain entities in placement order, so the cap evicts the oldest.
///
/// **Only the drawn decals are capped — the ledger is not.** The digest is taken over every stain the
/// deterministic half ever computed, so a rendering budget cannot change the number two runs are
/// compared by.
#[derive(Resource, Default)]
struct StainRing(Vec<Entity>);

/// **Everything the digest is taken over**, accumulated in placement order.
///
/// Not a `HashSet` and not sorted afterwards: the order *is* part of what is being checked. A run that
/// produced the same stains in a different order has a different iteration order somewhere, which is
/// exactly one of the three failures this exists to catch.
#[derive(Default)]
struct Ledger {
    wounds: usize,
    stains: Vec<Stain>,
}

impl Ledger {
    /// FNV-1a over every stain position's raw bits, in placement order.
    ///
    /// **Hand-rolled FNV, for the same reason `bake::seed_from_path` is**: `DefaultHasher` is not
    /// guaranteed stable across toolchains, so it has no business producing a number two runs on two
    /// machines are compared by. Raw bits rather than formatted floats, because formatting rounds and
    /// a rounded digest would hide exactly the last-bit drift this is looking for.
    fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |x: u32| {
            for byte in x.to_le_bytes() {
                h ^= byte as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        };
        for s in &self.stains {
            eat(s.at.x.to_bits());
            eat(s.at.y.to_bits());
            eat(s.at.z.to_bits());
            eat(s.radius.to_bits());
            eat(s.seed);
        }
        h
    }
}

fn main() {
    let out = arg("--out").unwrap_or_else(|| "frames-carnage".to_string());
    let camera =
        Transform::from_xyz(1.95, 1.25, 2.55).looking_at(ORIGIN - Vec3::Y * 0.22, Vec3::Y);
    // **The plugins must go in before the recorder finishes building.** `CarnageVfxPlugin` brings in
    // Hanabi, which registers render pipelines and extraction systems — none of which can be added to
    // an `App` after `cleanup`, which is why `Recorder::new_with` exists.
    let Some(mut rec) = Recorder::new_with(WIDTH, HEIGHT, camera, &out, |app| {
        // Before the plugins, so the plugin's own `init_resource` no-ops and these values win.
        app.insert_resource(CarnageSettings {
            spatter_speed_scale: SPEED_SCALE,
            ..CarnageSettings::default()
        })
        .add_plugins((CarnagePlugin, CarnageVfxPlugin));
    }) else {
        return;
    };

    // **The whole clock chain, pinned with one resource.** `Time<Real>` → `Time<Virtual>` →
    // `Time<EffectSimulation>`: the particle clock's speed is defined relative to the virtual one, so
    // fixing real time fixes all three and the particles step by a constant per pumped frame.
    rec.world().insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(DT)));
    rec.world().init_resource::<body::Thrown>();

    // **`DepthPrepass` on the camera the recorder already spawned.** Without it the forward decals
    // render as opaque quads or not at all, and the stains would be missing from the clip that exists
    // to show them. `Recorder::new_with` spawns exactly one `Camera3d`.
    let cameras: Vec<Entity> =
        rec.world().query_filtered::<Entity, With<Camera3d>>().iter(rec.world()).collect();
    for entity in cameras {
        rec.world().entity_mut(entity).insert(DepthPrepass);
    }

    light_and_floor(rec.world());
    let mut bores: Vec<bevy_carnage::Bore> = Vec::new();
    rebake(&mut rec, &bores);
    // The plugins' own `Startup` systems — `build_effects` and `build_splats` — run on the first
    // pumped frame, so the effect assets and splat textures exist before the first blow lands.
    rec.warm_up(4);

    // Added after the scene, so the frames before the first blow are perfectly still — the ordering
    // trick both other recorders use.
    rec.app().main.add_systems(Update, (integrate, body::bleed).chain());

    let mut ledger = Ledger::default();
    // The last scripted event, whichever it is — the shot is first now, so this is the script's tail.
    let last = SCRIPT
        .iter()
        .map(|(f, _, _)| *f)
        .chain(std::iter::once(SHOT.0))
        .max()
        .unwrap_or(0);
    for frame in 0..last + TAIL {
        // **The channel, first.** A bore re-bakes, so it has to land before any severance the clip
        // wants to keep — and its wound comes off the plug's own cell.
        if frame == SHOT.0 {
            bores.push(body::bore_at(SHOT.1, CALIBRE, 6));
            rebake(&mut rec, &bores);
            let wounds: Vec<Wound> = {
                let baked = rec.world().resource::<body::Baked>();
                baked.gore.iter().map(|g| wound_of_channel(&g.cell, g.exit, g.direction)).collect()
            };
            open(&mut rec, &wounds, &mut ledger);
            attach_bleeds(&mut rec, frame);
        }

        // A severing blow: take the wounds from the bonds that newly gave way.
        for (at_frame, blow, at) in SCRIPT {
            if frame != at_frame {
                continue;
            }
            let before: Vec<BondId> = rec.world().resource::<body::Damage>().broken.iter().collect();
            body::strike(rec.world(), blow, at);
            let newly: Vec<BondId> = {
                let damage = rec.world().resource::<body::Damage>();
                damage.broken.iter().filter(|id| !before.contains(id)).collect()
            };
            let wounds = {
                let damage = rec.world().resource::<body::Damage>();
                wounds_from_bonds(&damage.bonds, &newly)
            };
            open(&mut rec, &wounds, &mut ledger);
            attach_bleeds(&mut rec, frame);
        }

        // Every bleeding fragment, pulsing. Driven from the frame counter, which is this recorder's
        // fixed tick — the crate reads no clock, so there is nothing else it could be driven from.
        pulse(&mut rec, frame, &mut ledger);

        rec.shoot();
    }

    let n = rec.finish();
    // **The line two runs are compared by.** Everything the new layer computed, in one string.
    info!(
        "carnage: frames={n} wounds={} stains={} digest={:016x}",
        ledger.wounds,
        ledger.stains.len(),
        ledger.digest()
    );
    println!(
        "carnage: frames={n} wounds={} stains={} digest={:016x}",
        ledger.wounds,
        ledger.stains.len(),
        ledger.digest()
    );
}

/// Re-cut the subject with the accumulated channels, stand it back up, and throw what came out.
///
/// The same sequence `capture_holes` performs, and for the same reason: a bore is a bake input.
fn rebake(rec: &mut Recorder, bores: &[bevy_carnage::Bore]) {
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

/// **A wound opens: announce it, stamp its stains, and record them.**
///
/// Both halves of the crate driven from one place, so the digest covers what the clip shows. The
/// particle burst goes through [`Wounded`] for the *look*; the stains are computed on the CPU and are
/// what the digest is taken over — the deterministic half is the one being checked, and it is the one
/// that would still exist with the render feature off.
fn open(rec: &mut Recorder, wounds: &[Wound], ledger: &mut Ledger) {
    if wounds.is_empty() {
        return;
    }
    let settings = rec.world().resource::<CarnageSettings>().clone();
    ledger.wounds += wounds.len();

    let mut fresh: Vec<Stain> = Vec::new();
    for w in wounds {
        let world_wound = Wound { at: ORIGIN + w.at, ..*w };
        fresh.extend(stains(&world_wound, &settings, FLOOR_Y));
    }
    ledger.stains.extend(fresh.iter().copied());

    // The message the particle half reads. Written here rather than by the crate: the crate does not
    // decide when a wound happens.
    let announced: Vec<Wounded> = wounds
        .iter()
        .map(|w| Wounded {
            at: ORIGIN + w.at,
            normal: w.normal,
            area: w.area,
            severity: w.severity,
            kind: w.kind,
        })
        .collect();
    rec.world().resource_mut::<Messages<Wounded>>().write_batch(announced);

    stamp(rec, &fresh);
}

/// Turn stains into decal entities.
///
/// **Capped, and the oldest goes first.** A stain that faded would say blood dries; a floor with an
/// unbounded number of decals on it is a different problem. The cap is generous enough that the last
/// frame of the clip still has the first blow's blood on it, which is one of the things the GIF is
/// checked for.
fn stamp(rec: &mut Recorder, fresh: &[Stain]) {
    if fresh.is_empty() {
        return;
    }
    let Some(splats) = rec.world().remove_resource::<SplatTextures>() else {
        // Built by `CarnageVfxPlugin` on `Startup`, which has run by the time `warm_up` returns.
        warn!("capture_carnage: no splat textures yet — a stain was computed but not drawn");
        return;
    };
    let mut spawned = Vec::with_capacity(fresh.len());
    {
        let mut commands = rec.world().commands();
        for stain in fresh {
            spawned.push(spawn_stain(&mut commands, &splats, stain));
        }
    }
    rec.world().flush();
    rec.world().insert_resource(splats);

    let mut ring = rec.world().remove_resource::<StainRing>().unwrap_or_default();
    ring.0.extend(spawned);
    let excess = ring.0.len().saturating_sub(MAX_STAINS);
    let evicted: Vec<Entity> = ring.0.drain(..excess).collect();
    rec.world().insert_resource(ring);
    for entity in evicted {
        if let Ok(e) = rec.world().get_entity_mut(entity) {
            e.despawn();
        }
    }
}

/// **The cut face a chunk bled from, in the chunk's own local space.** Alongside [`Bleed`], which
/// carries *when* and *how much* but deliberately not *where*.
#[derive(Component)]
struct ChunkWound(Wound);

/// Give every detached chunk that has none a bleed schedule and the wound it bleeds from.
///
/// The wound is [`largest_cap`] of **that chunk's own** convex cell, looked up through the
/// `FragmentId` a `Chunk` carries — the widest raw-interior face it came away with, with its real
/// centroid, normal and area. A chunk with no cut face gets no `Bleed`, which is the honest answer for
/// a plug: it was never part of the frontier and has no severance wound.
fn attach_bleeds(rec: &mut Recorder, frame: u32) {
    let fresh: Vec<(Entity, Wound)> = {
        let mut q = rec.world().query_filtered::<(Entity, &Chunk), Without<Bleed>>();
        let candidates: Vec<(Entity, Option<FragmentId>)> =
            q.iter(rec.world()).map(|(e, c)| (e, c.fragment)).collect();
        let baked = rec.world().resource::<body::Baked>();
        candidates
            .into_iter()
            .filter_map(|(e, id)| {
                let part = baked.parts.get(id?.index())?;
                let cap = largest_cap(&part.cell)?;
                Some((
                    e,
                    Wound {
                        at: cap.centroid - part.center_local,
                        normal: cap.normal,
                        area: cap.area,
                        severity: 1.0,
                        kind: WoundKind::Severance,
                    },
                ))
            })
            .collect()
    };
    for (entity, wound) in fresh {
        rec.world()
            .entity_mut(entity)
            .insert((Bleed::new(frame, wound.area), ChunkWound(wound)));
    }
}

/// **Every bleeding fragment, one heartbeat at a time**, until it clots — and the stains it leaves go
/// into the digest.
///
/// Driven by the frame counter, which is this recorder's fixed tick. A clotted wound loses its
/// component, which is what makes "once clotted, never again" true of the scene as well as of the
/// arithmetic.
fn pulse(rec: &mut Recorder, frame: u32, ledger: &mut Ledger) {
    let settings = rec.world().resource::<CarnageSettings>().clone();
    let bleeding: Vec<(Entity, Bleed, Wound, GlobalTransform)> = {
        let mut q = rec.world().query::<(Entity, &Bleed, &ChunkWound, &GlobalTransform)>();
        q.iter(rec.world()).map(|(e, b, w, x)| (e, *b, w.0, *x)).collect()
    };
    let mut clot = Vec::new();
    let mut announced: Vec<Wounded> = Vec::new();
    let mut fresh: Vec<Stain> = Vec::new();
    for (entity, bleed, wound, xf) in bleeding {
        if clotted(&bleed, frame, HZ, &settings) {
            clot.push(entity);
            continue;
        }
        // **The chunk's own cut face**, carried since it detached and rotated into world space by the
        // chunk's transform — so blood leaves the wound the way the wound faces, and a tumbling gib's
        // spray tumbles with it.
        let Some(p) = pulse_wound(&bleed, &wound, frame, HZ, &settings) else { continue };
        let out = p.to_world(&xf);
        announced.push(out);
        let cpu = Wound {
            at: out.at,
            normal: out.normal,
            area: out.area,
            severity: out.severity,
            kind: out.kind,
        };
        ledger.wounds += 1;
        fresh.extend(stains(&cpu, &settings, FLOOR_Y));
    }
    // A clotted wound stops being a wound. Removing the component is what makes "once clotted, never
    // again" true of the scene as well as of the arithmetic.
    for entity in clot {
        rec.world().entity_mut(entity).remove::<Bleed>();
        rec.world().entity_mut(entity).remove::<ChunkWound>();
    }
    if !announced.is_empty() {
        rec.world().resource_mut::<Messages<Wounded>>().write_batch(announced);
    }
    ledger.stains.extend(fresh.iter().copied());
    stamp(rec, &fresh);
}

/// Gravity, a ground bounce and tumbling — on a fixed `DT`, so the run is reproducible.
fn integrate(mut chunks: Query<(&mut Chunk, &mut Transform)>) {
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, DT);
    }
}
