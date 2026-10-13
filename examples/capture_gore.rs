//! **Headless recorder for the four kernels this crate composes** — one script, four clips.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example capture_gore -- --out frames-gore
//! ```
//!
//! Writes `frame0000.png …` and prints one digest line per phase; two runs must print the same
//! four lines. The frame ranges are the four README clips, cut with `tools/gif.sh`:
//!
//! | frames | crate | what |
//! |---|---|---|
//! | 0–89 | `cross_section` | the body exploded slowly, every cap banded by its region |
//! | 90–179 | `fracture_modes` | a blow on the shoulder; the islands the modes part fly off |
//! | 180–299 | `flaymap` | a hit every ten frames at one spot, cratering the torso to bone |
//! | 300–419 | `laceration` | a slash down a round thigh, a spindle gaping onto its wet bed, bleeding |
//!
//! Native only: it writes PNGs, and the recorder's `device.poll` has no browser equivalent.

use bevy::prelude::*;
use bevy_carnage::cross_section::{CrossSectionAtlas, CrossSectionPlugin, CrossSectionSettings, Layers, Region, Scale};
use bevy_carnage::flaymap::{FlayCanvas, FlaySettings, FlaymapPlugin};
use bevy_carnage::flesh::{FleshMaterial, FleshMode, FleshParams, FleshPlugin, FleshTables};
use bevy_carnage::fracture_modes::ModeSettings;
use bevy_carnage::laceration::{Gape, Laceration, LacerationClock, LacerationPlugin, Tension};
use bevy_carnage::wetmap::{StainShape, WetCanvas, WetSettings, WetmapPlugin};
use bevy_carnage::{BondSet, CutSettings, FragmentGeometry, fracture_mesh, modal};

mod common;
use common::body;
use common::recorder::Recorder;

const ORIGIN: Vec3 = body::ORIGIN;
const SEED: u32 = 0x00C0_FFEE;

#[derive(Component)]
struct Phase(u32);
#[derive(Component)]
struct Drift(Vec3, bool);
#[derive(Component)]
struct Torso;
#[derive(Component)]
struct Thigh;

fn region_of(part: usize) -> Region {
    match part {
        0 => Region::Torso,
        1 => Region::Head,
        _ => Region::Limb,
    }
}

fn main() {
    let out = common::arg("--out").unwrap_or_else(|| "frames-gore".to_string());
    let camera = Transform::from_xyz(1.9, 1.5, 2.3).looking_at(Vec3::new(0.0, 0.85, 0.0), Vec3::Y);
    let Some(mut rec) = Recorder::new_with(640, 480, camera, &out, |app| {
        // The flaymap clip's torso is dressed the way a game dresses skin: the flaymap for what is
        // peeled, the wetmap for the blood over it, and `FleshMaterial` to composite the two. Blood
        // stays wet for the whole clip, so `dry_ticks` is longer than it.
        app.add_plugins((CrossSectionPlugin, FlaymapPlugin, LacerationPlugin, WetmapPlugin, FleshPlugin))
            .insert_resource(WetSettings { dry_ticks: 900, edge_samples: 4, ..default() });
    }) else {
        return;
    };

    // Lights and floor.
    {
        let world = rec.world();
        world.spawn((
            DirectionalLight { illuminance: 9_000.0, shadow_maps_enabled: true, ..default() },
            Transform::from_xyz(3.0, 6.0, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let floor = world.resource_mut::<Assets<Mesh>>().add(Plane3d::default().mesh().size(4.0, 4.0).build());
        let floor_material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color: Color::srgb(0.20, 0.19, 0.18),
            perceptual_roughness: 0.85,
            ..default()
        });
        world.spawn((Mesh3d(floor), MeshMaterial3d(floor_material)));
    }
    // One frame so the strips bake (`CrossSectionPlugin` runs on `Startup`).
    rec.step();

    let mut digests = [0u64; 4];

    // ---- Phase 1: cross-section. The whole body exploded slowly. ----------------------------
    let (leaves, regions, _bake_solids) = bake_body(20, SEED);
    aim(rec.world(), Vec3::new(1.1, 1.3, 1.5), Vec3::new(0.0, 0.85, 0.0));
    spawn_pieces(rec.world(), &leaves, &regions, 1, false, |g| g.center_local.normalize_or_zero() * 0.22 + Vec3::Y * 0.05);
    rec.warm_up(2);
    for frame in 0..90 {
        drift(rec.world(), frame);
        rec.shoot();
    }
    digests[0] = fnv_positions(rec.world());
    clear(rec.world(), 1);

    // ---- Phase 2: fracture modes. A blow on the shoulder. -----------------------------------
    let (leaves, regions, bake) = bake_body(20, SEED);
    let shot = Vec3::new(-0.24, 0.20, 0.06);
    let mut thrown: Vec<(bevy_carnage::FragmentId, Vec3)> = Vec::new();
    if let Ok(modal) = modal::bake_modes(&bake.0, |id| bake.1.get(id.index()), &ModeSettings { k: 6, ..Default::default() }) {
        let struck = bake
            .1
            .iter()
            .enumerate()
            .filter(|(i, _)| leaves.iter().any(|g| g.id.index() == *i))
            .min_by(|a, b| {
                a.1.center().distance_squared(shot).partial_cmp(&b.1.center().distance_squared(shot)).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| bevy_carnage::FragmentId(i as u32));
        if let Some(struck) = struck {
            // Two pieces at least, a little harder: the shoulder shot takes the arm and whatever the
            // modes find weak beside it.
            let magnitude = modal.impulse_for(struck, 2).unwrap_or(0.1) * 1.6;
            let broken = modal.break_at(struck, magnitude);
            let mut severed = BondSet::new(&bake.0);
            severed.sever_all(&broken);
            let islands = bake.0.islands(bake.0.members(), &severed);
            let keep = islands.iter().enumerate().max_by_key(|(i, x)| (x.len(), std::cmp::Reverse(*i))).map(|(i, _)| i);
            // **The kept island stays only while it stands on something** — a cell reaching the
            // body's lowest extent, the feet. Kept unconditionally, it hung at standing height for
            // the rest of the clip once the modes parted the legs.
            let lowest = |id: &bevy_carnage::FragmentId| {
                bake.1.get(id.index()).map_or(f32::INFINITY, |c| c.points().iter().map(|p| p.y).fold(f32::INFINITY, f32::min))
            };
            let floor = bake.0.members().iter().map(&lowest).fold(f32::INFINITY, f32::min);
            let stands = keep.and_then(|k| islands.get(k)).is_some_and(|island| island.iter().any(|id| lowest(id) <= floor + 1.0e-3));
            for (i, island) in islands.iter().enumerate() {
                if Some(i) == keep && stands {
                    continue;
                }
                let falls = Some(i) == keep;
                let middle = island.iter().filter_map(|id| bake.1.get(id.index()).map(|c| c.center())).sum::<Vec3>() / island.len().max(1) as f32;
                for id in island {
                    let centre = bake.1.get(id.index()).map(|c| c.center()).unwrap_or(Vec3::ZERO);
                    let v = if falls {
                        // Not hit, so not thrown: a slow outward slump, and gravity brings it down.
                        Vec3::new(centre.x - middle.x, 0.0, centre.z - middle.z).normalize_or_zero() * 0.3 - Vec3::Y * 0.01
                    } else {
                        ((centre - shot).normalize_or_zero() + Vec3::new(0.0, 0.5, 0.6)) * 0.9
                    };
                    thrown.push((*id, v));
                }
            }
            println!("gore: modes broke {} bond(s) into {} piece(s)", broken.len(), islands.len());
        }
    }
    spawn_pieces(rec.world(), &leaves, &regions, 2, true, |g| thrown.iter().find(|(id, _)| *id == g.id).map(|(_, v)| *v).unwrap_or(Vec3::ZERO));
    rec.warm_up(2);
    for frame in 90..180 {
        // The camera follows the body down: it starts on the standing figure and ends close on the
        // heap the modes leave, which at the old fixed view was a small pile at the bottom edge.
        let t = ((frame - 90) as f32 / 60.0).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        aim(
            rec.world(),
            Vec3::new(1.4, 1.4, 1.9).lerp(Vec3::new(0.65, 0.6, 1.0), t),
            Vec3::new(0.0, 0.85, 0.0).lerp(Vec3::new(0.05, 0.15, 0.05), t),
        );
        drift(rec.world(), frame);
        rec.shoot();
    }
    digests[1] = fnv_positions(rec.world());
    clear(rec.world(), 2);

    // ---- Phase 3: flaymap. The intact body; the torso is cratered every ten frames. ---------
    // The hits wander a little, as shots do, so the crater grows ragged rather than as a target; the
    // camera swings across the wound so the normal map's walls catch the light from both sides.
    spawn_intact(rec.world());
    rec.warm_up(2);
    for frame in 180..300 {
        let swing = ((frame - 180) as f32 / 120.0 - 0.5) * 0.9;
        let target = Vec3::new(0.05, 1.02, 0.14);
        aim(rec.world(), target + Quat::from_rotation_y(swing) * Vec3::new(0.30, 0.16, 0.66), target);
        if frame % 10 == 0 {
            // Shallow hits that deepen: skin and fat first, muscle by the middle of the clip, the rib
            // only at the end. At the old 4 mm rising 2.5 mm a hit, the fourth reached bone and most
            // of the clip's crater was rib and marrow.
            let n = (frame - 180) / 10;
            peel(rec.world(), frame, n, 2.0 + n as f32 * 0.35);
        }
        // The blood runs: one wetmap tick per frame, down the face. On a `Cuboid`'s side faces `v`
        // grows *up* the face, so down the body is `-v`; with `+v` the drips climbed the chest.
        {
            let world = rec.world();
            let settings = world.get_resource::<WetSettings>().cloned().unwrap_or_default();
            let mut wets = world.query::<&mut WetCanvas>();
            for mut wet in wets.iter_mut(world) {
                wet.tick(frame, Vec2::new(0.0, -1.0), &settings);
            }
        }
        rec.shoot();
    }
    digests[2] = rec.world().query::<&FlayCanvas>().iter(rec.world()).next().map(|c| c.digest()).unwrap_or(0);

    // ---- Phase 4: laceration. A slash across the thigh, opening, and bleeding. --------------
    // The blockout's legs are boxes, and a cut on a flat box face read as a ribbon on a plank; the
    // clip stands the body on two round, tapering legs instead, dense enough for the cut to open.
    round_legs(rec.world());
    lacerate(rec.world());
    rec.warm_up(2);
    for frame in 300..420 {
        // A slow swing across the slash, so the lips and the trough's walls catch the light from
        // both sides rather than reading as stripes painted on the thigh.
        let swing = ((frame - 300) as f32 / 120.0 - 0.5) * 0.7;
        let target = Vec3::new(-0.13, 0.34, 0.12);
        aim(rec.world(), target + Quat::from_rotation_y(swing) * Vec3::new(0.20, 0.10, 0.48), target);
        bleed_cut(rec.world(), frame);
        rec.shoot();
    }
    digests[3] = fnv_positions(rec.world());

    let n = rec.finish();
    println!(
        "gore: frames={n} cross_section={:016x} fracture_modes={:016x} flaymap={:016x} laceration={:016x}",
        digests[0], digests[1], digests[2], digests[3]
    );
}

/// Bake the blockout to `target` leaves; returns the leaf geometry, the region each leaf's cap was
/// banded for, and `(bond graph, cells)`.
#[allow(clippy::type_complexity)]
fn bake_body(target: usize, seed: u32) -> (Vec<FragmentGeometry>, Vec<Region>, (bevy_carnage::BondGraph, Vec<bevy_carnage::ProxyCell>)) {
    let subject = body::subject();
    let parts: Vec<(&Mesh, Mat4)> = subject.iter().map(|(m, x)| (m, *x)).collect();
    let proxy = body::proxy();
    let cut = CutSettings { soften: 0.0, ..CutSettings::new(target, 0.08, seed) };
    let bake = fracture_mesh(&parts, &proxy, &cut);
    let cells: Vec<bevy_carnage::ProxyCell> = bake.solids().iter().map(|s| s.cell.clone()).collect();
    let bonds = bake.bonds.clone();
    let tree = bake.tree.clone();
    let mut leaves = bake.into_leaves();
    // Band every cap by the region of the part it came from.
    let settings = CrossSectionSettings::default();
    let mut regions = Vec::with_capacity(leaves.len());
    for g in &mut leaves {
        let part = tree.root_of(g.id).unwrap_or(g.id).index();
        let region = region_of(part);
        g.annotate_cap(settings.layers(region), &settings.scale);
        regions.push(region);
    }
    (leaves, regions, (bonds, cells))
}

/// Point the recorder's camera at `at` from `from`.
fn aim(world: &mut World, from: Vec3, at: Vec3) {
    let mut q = world.query_filtered::<&mut Transform, With<Camera3d>>();
    for mut tf in q.iter_mut(world) {
        *tf = Transform::from_translation(from).looking_at(at, Vec3::Y);
    }
}

fn spawn_pieces(
    world: &mut World,
    leaves: &[FragmentGeometry],
    regions: &[Region],
    phase: u32,
    falls: bool,
    velocity: impl Fn(&FragmentGeometry) -> Vec3,
) {
    let skin = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: Color::srgb(0.62, 0.52, 0.48),
        perceptual_roughness: 0.7,
        ..default()
    });
    let fallback = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.18, 0.18),
        ..default()
    });
    for (g, &region) in leaves.iter().zip(regions) {
        let v = velocity(g);
        let at = Transform::from_translation(ORIGIN + g.center_local);
        // **The strip the cap was banded for.** `annotate_cap` wrote `UV_1` against this region's
        // thickness row; a material guessed from the piece's height put the head's strip on the
        // upper torso, and its cuts showed skull where the trunk is muscle.
        let cap_material = world.resource::<CrossSectionAtlas>().material(region).unwrap_or_else(|| fallback.clone());
        if let Some(outer) = g.outer.clone() {
            let h = world.resource_mut::<Assets<Mesh>>().add(outer);
            world.spawn((Mesh3d(h), MeshMaterial3d(skin.clone()), at, Drift(v, falls), Phase(phase)));
        }
        if let Some(cap) = g.cap.clone() {
            let h = world.resource_mut::<Assets<Mesh>>().add(cap);
            world.spawn((Mesh3d(h), MeshMaterial3d(cap_material), at, Drift(v, falls), Phase(phase)));
        }
    }
}

fn drift(world: &mut World, _frame: u32) {
    let dt = 1.0 / 60.0;
    let mut q = world.query::<(&mut Drift, &mut Transform)>();
    for (mut d, mut tf) in q.iter_mut(world) {
        if d.0 == Vec3::ZERO {
            continue;
        }
        if d.1 {
            if tf.translation.y <= 0.05 && d.0.y < 0.0 {
                d.0 = Vec3::ZERO;
                continue;
            }
            d.0.y -= 9.8 * dt * 0.5;
        }
        tf.translation += d.0 * dt;
    }
}

fn clear(world: &mut World, phase: u32) {
    let ents: Vec<Entity> = world.query::<(Entity, &Phase)>().iter(world).filter(|(_, p)| p.0 == phase).map(|(e, _)| e).collect();
    for e in ents {
        world.despawn(e);
    }
}

fn spawn_intact(world: &mut World) {
    let flesh = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: Color::srgb(0.62, 0.52, 0.48),
        perceptual_roughness: 0.7,
        ..default()
    });
    for (i, (mesh, xf)) in body::subject().into_iter().enumerate() {
        let at = Transform::from_matrix(Mat4::from_translation(ORIGIN) * xf);
        if i == 0 {
            const SKIN: [f32; 3] = [0.62, 0.52, 0.48];
            let layers = Layers::for_region(Region::Torso);
            let (flay, wet) = {
                let mut images = world.resource_mut::<Assets<Image>>();
                // 256, the crate's stated ceiling, because this clip fills the frame with one face of
                // the torso: at 128 a texel was five pixels and the rim stair-stepped.
                (
                    FlayCanvas::new(&mut images, 256, Region::Torso, layers, SKIN, 0.7),
                    WetCanvas::new(&mut images, 256, SKIN, 0.7),
                )
            };
            // The material's colour is the one the canvas was filled with; the flesh material
            // swaps that for the material's own skin wherever the skin is intact.
            let base = StandardMaterial { base_color: Color::srgb(SKIN[0], SKIN[1], SKIN[2]), ..default() };
            let params = FleshParams::for_layers(&layers, FleshMode::Canvas, Scale::default().mm_per_unit);
            let material = world.resource::<FleshTables>().material(base, params, Some(wet.roughness()), Some(&flay));
            let material = world.resource_mut::<Assets<FleshMaterial>>().add(material);
            // Tangents, or Bevy draws the normal map flat.
            let mesh = mesh.clone().with_generated_tangents().unwrap_or(mesh);
            let h = world.resource_mut::<Assets<Mesh>>().add(mesh);
            world.spawn((Mesh3d(h), MeshMaterial3d(material), at, flay, wet, Torso, Phase(3)));
        } else {
            let h = world.resource_mut::<Assets<Mesh>>().add(mesh);
            let mut e = world.spawn((Mesh3d(h), MeshMaterial3d(flesh.clone()), at, Phase(3)));
            if i >= 4 {
                e.insert(Leg);
            }
        }
    }
}

#[derive(Component)]
struct IntactThigh(Handle<Mesh>);

/// A leg of the blockout, replaced by a round one for the laceration clip.
#[derive(Component)]
struct Leg;

/// Peel the torso with hit `n`, and bleed into it. Each hit lands a little off the last and a little
/// wider or narrower, on a golden-angle spiral — deterministic, and enough to grow a ragged crater
/// rather than rings.
fn peel(world: &mut World, tick: u32, n: u32, depth_mm: f32) {
    let settings = world.get_resource::<FlaySettings>().cloned().unwrap_or_default();
    let wet_settings = world.get_resource::<WetSettings>().cloned().unwrap_or_default();
    // `libm`, not `std`: the hit positions reach the flaymap digest.
    let a = n as f32 * 2.399_963;
    let wander = 0.014 * (n as f32).sqrt();
    let entry = Vec3::new(0.05 + libm::cosf(a) * wander, 0.10 + libm::sinf(a) * wander, 0.14) + ORIGIN;
    let radius = 0.12 + 0.05 * (((n * 7) % 5) as f32 / 4.0);
    // The wound's blood: a stain a little smaller than the crater, so the rim stays readable and the
    // floor fills.
    let shape = StainShape {
        major: radius * 0.75,
        minor: radius * 0.55,
        spines: 5,
        satellites: 3,
        direction: [0.0, 1.0],
        seed: 0x5EED_u32 ^ n.wrapping_mul(0x9E37_79B9),
    };
    let mut q = world.query_filtered::<(&mut FlayCanvas, Option<&mut WetCanvas>, &Mesh3d, &GlobalTransform), With<Torso>>();
    let meshes = world.resource::<Assets<Mesh>>();
    let mut results = Vec::new();
    // Two borrows of the world cannot overlap, so the mesh is cloned out first.
    let handles: Vec<(Handle<Mesh>, GlobalTransform)> = q.iter(world).map(|(_, _, m, xf)| (m.0.clone(), *xf)).collect();
    // One slot per canvas, `None` where the handle does not resolve — never filtered, or the zip
    // below would pair every later canvas with the next entity's mesh.
    let meshes_cloned: Vec<Option<(Mesh, GlobalTransform)>> = handles.iter().map(|(h, xf)| meshes.get(h).cloned().map(|m| (m, *xf))).collect();
    for ((mut canvas, wet, _, _), slot) in q.iter_mut(world).zip(meshes_cloned.iter()) {
        let Some((mesh, xf)) = slot else { continue };
        if let Some(handoff) = canvas.paint_world(mesh, xf, entry + Vec3::Z * 0.5, -Vec3::Z, radius, depth_mm, tick) {
            results.push(handoff.bone_reached);
        }
        canvas.shade(&settings);
        if let Some(mut wet) = wet {
            let _ = wet.paint_world_with(mesh, xf, entry + Vec3::Z * 0.5, -Vec3::Z, &shape, tick, &wet_settings);
        }
    }
    if results.iter().any(|b| *b) {
        println!("gore: bone exposed on tick {tick}");
    }
}

/// **Two round legs in place of the blockout's boxes**, for the laceration clip. Each is a tapering
/// elliptical tube the size of its box, dense enough — 3–6 mm between vertices — for a cut a few
/// centimetres wide to open, in a skin texture under the flesh material. The left thigh is the one the
/// slash cuts: its drawn mesh and its intact source are separate handles, and it carries a wetmap
/// for the blood.
fn round_legs(world: &mut World) {
    let boxes: Vec<Entity> = world.query_filtered::<Entity, With<Leg>>().iter(world).collect();
    for e in boxes {
        world.despawn(e);
    }
    let (skin, wet) = {
        let mut images = world.resource_mut::<Assets<Image>>();
        (common::preset_scene::skin_texture(&mut images), WetCanvas::new(&mut images, 256, [1.0; 3], 0.55))
    };
    let layers = Layers::for_region(Region::Limb);
    let params = FleshParams::for_layers(&layers, FleshMode::Canvas, Scale::default().mm_per_unit);
    let base = StandardMaterial { base_color_texture: Some(skin), perceptual_roughness: 0.55, ..default() };
    let (cut, whole) = {
        let tables = world.resource::<FleshTables>().clone();
        (tables.material(base.clone(), params, Some(wet.roughness()), None), tables.material(base, params, None, None))
    };
    let (cut, whole) = {
        let mut flesh = world.resource_mut::<Assets<FleshMaterial>>();
        (flesh.add(cut), flesh.add(whole))
    };
    let mut wet = Some(wet);
    for (i, (_, c, h)) in body::parts().into_iter().enumerate().skip(4) {
        let tube = limb_tube(h);
        let at = Transform::from_translation(ORIGIN + c);
        if i == 4
            && let Some(wet) = wet.take()
        {
            let source = world.resource_mut::<Assets<Mesh>>().add(tube.clone());
            let drawn = world.resource_mut::<Assets<Mesh>>().add(tube);
            world.spawn((Mesh3d(drawn), MeshMaterial3d(cut.clone()), at, Phase(4), Thigh, IntactThigh(source), wet));
        } else {
            let h = world.resource_mut::<Assets<Mesh>>().add(tube);
            world.spawn((Mesh3d(h), MeshMaterial3d(whole.clone()), at, Phase(4)));
        }
    }
}

/// Columns around and rings along a [`limb_tube`].
const TUBE_AROUND: u32 = 128;
const TUBE_ALONG: u32 = 160;

/// **A tapering elliptical tube** standing in a box of half-extents `h`: the box's radii at the top,
/// three quarters of them at the bottom, `+z` at angle zero so the front of the leg faces the camera.
/// `UV_0` is isotropic — a metre is the same UV distance around as along — so a blood stain on it is
/// round, and `+v` runs down the leg.
fn limb_tube(h: Vec3) -> Mesh {
    let (around, along) = (TUBE_AROUND, TUBE_ALONG);
    let girth = std::f32::consts::TAU * ((h.x * h.x + h.z * h.z) * 0.5).sqrt();
    let span = girth.max(h.y * 2.0);
    let (mut pos, mut nrm, mut uv) = (Vec::new(), Vec::new(), Vec::new());
    for j in 0..=along {
        let t = j as f32 / along as f32;
        let s = 1.0 - 0.25 * t;
        let y = h.y - t * h.y * 2.0;
        for k in 0..=around {
            let a = std::f32::consts::TAU * k as f32 / around as f32;
            let (rx, rz) = (h.x * s, h.z * s);
            pos.push([rx * a.sin(), y, rz * a.cos()]);
            nrm.push(Vec3::new(a.sin() / rx, 0.0, a.cos() / rz).normalize_or_zero().to_array());
            uv.push([k as f32 / around as f32 * girth / span, t * h.y * 2.0 / span]);
        }
    }
    let row = around + 1;
    let mut idx = Vec::new();
    for j in 0..along {
        for k in 0..around {
            let (a, b, c, d) = (j * row + k, j * row + k + 1, (j + 1) * row + k, (j + 1) * row + k + 1);
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_indices(bevy::mesh::Indices::U32(idx));
    mesh
}

/// **Where the slash runs**, in the left thigh's own space: down and across the front of the leg,
/// sampled on the tube's surface every 2 cm so the polyline lies on the skin it cuts.
fn slash_path() -> Vec<Vec3> {
    let Some((_, _, h)) = body::parts().get(4).copied() else { return Vec::new() };
    let (from, to) = (Vec2::new(-0.045, 0.14), Vec2::new(0.035, -0.06));
    (0..=10)
        .map(|i| {
            let p = from.lerp(to, i as f32 / 10.0);
            let s = 1.0 - 0.25 * ((h.y - p.y) / (h.y * 2.0));
            let (rx, rz) = (h.x * s, h.z * s);
            Vec3::new(p.x, p.y, rz * (1.0 - (p.x / rx).powi(2)).max(0.0).sqrt())
        })
        .collect()
}

fn lacerate(world: &mut World) {
    let now = world.get_resource::<LacerationClock>().map_or(0, |c| c.0);
    let ent: Option<(Entity, Handle<Mesh>)> =
        world.query_filtered::<(Entity, &IntactThigh), With<Thigh>>().iter(world).next().map(|(e, s)| (e, s.0.clone()));
    let Some((entity, source)) = ent else { return };
    // Metres, on the thigh: a 21 cm slash that gapes 3 cm across, its skin pushed back over 3 cm.
    world.entity_mut(entity).insert(Laceration {
        path: slash_path(),
        normal: Vec3::Z,
        gape: Gape { width_max: 0.04, open_ticks: 80 },
        // A thigh's Langer lines run round the limb, so a cut down it is across them and gapes wide.
        tension: Tension { skin: 0.9, langer: Some([1.0, 0.0, 0.0]) },
        influence: 0.03,
        // Into the muscle, well short of the femur at 27.8 mm. At 14 mm the trough was half fat, and
        // a slash read as yellow stripes on the thigh rather than as a cut.
        bed_depth_mm: 24.0,
        region: Region::Limb,
        opened_at: now,
        source,
        ..default()
    });
}

/// **The cut bleeds.** Blood wells over the lower lip at three points and runs: a drop every other
/// frame at each, small enough that the wetmap's drip pass draws each into its own rivulet down the
/// leg rather than a pond around the wound. Painted by UV from the tube's own parametrisation, since a
/// ray at the wound now goes through it.
fn bleed_cut(world: &mut World, frame: u32) {
    let settings = world.get_resource::<WetSettings>().cloned().unwrap_or_default();
    let Some((_, _, h)) = body::parts().get(4).copied() else { return };
    let girth = std::f32::consts::TAU * ((h.x * h.x + h.z * h.z) * 0.5).sqrt();
    let span = girth.max(h.y * 2.0);
    let uv_of = |p: Vec3| {
        let t = (h.y - p.y) / (h.y * 2.0);
        let s = 1.0 - 0.25 * t;
        let a = (p.x / (h.x * s)).atan2(p.z / (h.z * s)).rem_euclid(std::f32::consts::TAU);
        Vec2::new(a / std::f32::consts::TAU * girth / span, t * h.y * 2.0 / span)
    };
    let path = slash_path();
    // The lower lip is on the cut's `+x` side: perpendicular to its run, which falls to the right.
    let (Some(first), Some(last)) = (path.first(), path.last()) else { return };
    let run = (*last - *first).truncate().normalize_or_zero();
    let below = Vec3::new(-run.y, run.x, 0.0) * if run.x > 0.0 { -1.0 } else { 1.0 };
    let mut q = world.query_filtered::<&mut WetCanvas, With<Thigh>>();
    for mut wet in q.iter_mut(world) {
        let since = frame.saturating_sub(306);
        // The margins first: a smear along both lips as the cut opens, as a blade leaves them.
        if frame == 306 {
            // Every 4 mm along the cut, either side, as far out as the lip is at that point: the
            // wound is a spindle, so the smear closes in toward both ends with it.
            let steps = 54;
            for j in 0..=steps {
                let u = j as f32 / steps as f32;
                let at = u * (path.len() - 1) as f32;
                let (i0, f) = (at.floor() as usize, at.fract());
                let (Some(a), Some(b)) = (path.get(i0), path.get((i0 + 1).min(path.len() - 1))) else { continue };
                let p = a.lerp(*b, f);
                let open = (std::f32::consts::PI * u).sin().max(0.0).powf(bevy_carnage::laceration::GAPE_TAPER);
                for side in [1.0f32, -1.0] {
                    let shape = StainShape {
                        major: 0.007 / span,
                        minor: 0.0035 / span,
                        spines: 0,
                        satellites: 0,
                        direction: [run.x, -run.y],
                        seed: (j as u32).wrapping_mul(0x85EB_CA6B) ^ side.to_bits(),
                    };
                    wet.paint_uv_with(uv_of(p + below * (0.0125 * open * side)), &shape, frame, &settings);
                }
            }
        }
        if frame >= 306 && since % 2 == 0 && since < 100 {
            // Three runs over the lower lip, each fed its own drop and a little different in size.
            for (n, k) in [4usize, 7, 9].into_iter().enumerate() {
                let Some(p) = path.get(k) else { continue };
                let lip = *p + below * 0.013;
                let size = (0.0026 + 0.0008 * n as f32) / span;
                let shape = StainShape {
                    major: size * 1.4,
                    minor: size,
                    spines: 0,
                    satellites: 0,
                    direction: [0.0, 1.0],
                    seed: (n as u32).wrapping_mul(0x9E37_79B9) ^ frame,
                };
                wet.paint_uv_with(uv_of(lip), &shape, frame, &settings);
            }
        }
        wet.tick(frame, Vec2::new(0.0, 1.0), &settings);
    }
}

/// FNV-1a over every transform's translation bits, in entity order — the phase's digest.
fn fnv_positions(world: &mut World) -> u64 {
    let mut rows: Vec<(u64, [u32; 3])> = world
        .query::<(Entity, &Transform)>()
        .iter(world)
        .map(|(e, t)| (e.to_bits(), [t.translation.x.to_bits(), t.translation.y.to_bits(), t.translation.z.to_bits()]))
        .collect();
    rows.sort_unstable();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (_, p) in rows {
        for v in p {
            for byte in v.to_le_bytes() {
                h ^= byte as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    h
}
