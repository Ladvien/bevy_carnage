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
//! `--blows` adds a second blow to the left shin, lower down, at frame 60: two bruises on one limb,
//! both ageing. `--stroll` takes no hits: the body walks along a tiled floor instead of in place,
//! through a spill of blood painted on the floor ahead of it at frame 0, and every foot it puts down is
//! sent as a `GoreStep` — so it walks out of the spill leaving prints that fade, seen from above.
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
use bevy_carnage::preset::{ClothHoles, Gore, GoreClock, GorePlugin, GoreReset, GoreStep, HitKind, Sole};
use bevy_carnage::wetmap::{StainShape, WetCanvas};

mod common;
use common::mannequin::{self, Bone, Mannequin, Part};
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
    let stroll = flag("--stroll");
    // A stroll is seen from above and to the side, the whole path in frame and the prints facing up at it.
    let at = point("--at").unwrap_or(if stroll { Vec3::new(0.05, 0.5, 0.05) } else { Vec3::new(0.12, 0.93, 0.1) });
    let camera = match point("--eye") {
        Some(eye) => Transform::from_translation(eye).looking_at(at, Vec3::Y),
        None if stroll => Transform::from_xyz(-2.9, 2.75, 0.15).looking_at(at, Vec3::Y),
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
            .add_systems(Startup, setup);
        if flag("--stroll") {
            app.add_systems(Update, mannequin::stroll_animate);
        } else {
            app.add_systems(Update, mannequin::animate);
        }
    }) else {
        return;
    };
    {
        let world = rec.world();
        let mut cam = world.query_filtered::<Entity, With<Camera3d>>();
        if let Some(entity) = cam.iter(world).next() {
            let light = preset_scene::camera_light(&mut world.resource_mut::<Assets<Image>>());
            world.entity_mut(entity).insert(light);
        }
    }
    rec.warm_up(6);
    let breaking = breaking();
    let through = flag("--through");
    let shirt = shirted();
    // `--quiet` takes no hits; `--reset-at N` heals the body and wipes every surface on frame `N`,
    // after which a run is the quiet one again, frame for frame.
    let quiet = flag("--quiet");
    let blows = flag("--blows");
    let reset_at = common::arg("--reset-at").and_then(|n| n.parse::<u32>().ok());
    let mut aim = bevy::ecs::system::SystemState::<mannequin::Aim>::new(rec.world());
    if stroll {
        spill(rec.world());
    }
    // The last step a `GoreStep` was sent for.
    let mut planted: Option<i64> = None;
    for frame in 0..FRAMES {
        {
            let world = rec.world();
            if stroll {
                let t = world.resource::<Time>().elapsed_secs();
                let mut legs = world.query::<(Entity, &Part)>();
                let feet: Vec<(Entity, Part)> = legs.iter(world).map(|(e, p)| (e, *p)).collect();
                for (k, limb, at, heading) in mannequin::touchdowns(planted, t) {
                    planted = Some(k);
                    // Each foot is its leg's mesh, so the blood it carries is kept on the leg.
                    if let Some((foot, _)) = feet.iter().find(|(_, p)| p.0 == limb) {
                        world.write_message(GoreStep::new(*foot, at, heading).with_sole(Sole { length_m: 0.28, width_m: 0.09, tread: 3 }));
                    }
                }
            }
            let hit = aim.get(world).ok().and_then(|aim| match frame {
                _ if quiet || stroll => None,
                60 if blows => aim.blow(Bone::LegLowL, 0.32, 0.04),
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

/// **A spill on the floor in the stroll's path**: three overlapping stains, half a metre across in
/// all, painted wet on the slab's canvas where the body's second and third steps land.
fn spill(world: &mut World) {
    let tick = world.resource::<GoreClock>().0;
    let mut slab = world.query_filtered::<(Entity, &Mesh3d, &GlobalTransform), With<preset_scene::Slab>>();
    let Some((entity, handle, xf)) = slab.iter(world).next().map(|(e, m, x)| (e, m.0.clone(), *x)) else { return };
    let Some(mesh) = world.resource::<Assets<Mesh>>().get(&handle).cloned() else { return };
    let Some(mut wet) = world.get_mut::<WetCanvas>(entity) else { return };
    // The slab is 3 m to a UV, so a stain's axes are its metres over three.
    for (x, z, across, along, seed) in [(0.0, -0.86, 0.50, 0.44, 11), (0.13, -0.72, 0.26, 0.24, 12), (-0.12, -1.02, 0.30, 0.27, 13)] {
        let shape = StainShape { major: across / 3.0, minor: along / 3.0, spines: 0, satellites: 0, direction: [1.0, 0.0], seed };
        let _ = wet.paint_world(&mesh, &xf, Vec3::new(x, 1.0, z), Vec3::NEG_Y, &shape, tick);
    }
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
    // **A stroll is on a tiled floor**, as a level's is: its UVs repeat the tiles, so it is dressed
    // `Gore::projected`, and at 512 a side, 5.9 mm a texel on the 3 m slab, where 256 ships — a print's
    // heel, waist and forefoot are a few centimetres each, and at 11.7 mm a texel the smoothing the
    // shader reads the canvas with rounds them into two drops. A showcase's choice, as the mannequin's
    // 256-texel skin is.
    if flag("--stroll") {
        let slab = preset_scene::tiled_slab(&mut meshes, &mut materials, &mut images);
        preset_scene::spawn_set_on(&mut commands, &mut meshes, &mut materials, slab);
    } else {
        preset_scene::spawn_set(&mut commands, &mut meshes, &mut materials);
    }
    if flag("--through") {
        preset_scene::spawn_wall(&mut commands, &mut meshes, &mut materials);
    }
}
