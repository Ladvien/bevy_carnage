//! **A wound you can hash**, with a renderer attached.
//!
//! A lit slab is hit every twenty frames at the same spot while the camera orbits it. The crater opens
//! through skin, then fat, then muscle, and stops when it reaches bone — and the rim is the whole
//! point: because a hit falls off smoothly to nothing at its radius, every layer it has passed through
//! is on show at once, at the depths those tissues were measured at. The number on screen is the
//! canvas's own digest, taken over the CPU depth buffer.
//!
//! Nothing here reads a clock or the filesystem: the cadence, the orbit and the digest all come off a
//! `Local<u32>` frame counter, so this example builds for `wasm32-unknown-unknown` as-is.
//!
//! `cargo run -p bevy_flaymap --example flaymap_peel`

use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::prelude::*;
use bevy_carnage::flaymap::{BoneExposed, FlayCanvas, FlaySettings, FlaymapPlugin, Layer, Layers, Region};

/// Edge length of the canvas, in texels.
const CANVAS: u32 = 128;
/// Frames between hits. Twenty at 60 Hz is a hit every third of a second — slow enough to watch a
/// layer come off, fast enough to reach bone inside a few seconds.
const EVERY: u32 = 20;
/// Where every hit lands, in the slab's UVs. Bevy's `Cuboid` gives each face the full unit square, so
/// this is the middle of whichever face you are looking at.
const SPOT: Vec2 = Vec2::new(0.5, 0.5);
/// Radius of a hit, in UV units.
const RADIUS: f32 = 0.13;
/// Millimetres of tissue a single hit takes off at its centre.
const BITE_MM: f32 = 1.6;

/// What the demo has learned from the crate, for the readout. Written by the message reader, so the
/// `Handoff` → [`BoneExposed`] path this crate exists to serve is what puts it on screen.
#[derive(Resource, Default)]
struct Peel {
    /// Where the one-shot handoff said bone came through, once it has.
    bone_uv: Option<Vec2>,
}

#[derive(Component)]
struct Subject;

#[derive(Component)]
struct Hud;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            // `canvas` has no effect off the web (`bevy_window-0.19.0/src/window.rs:250`), so it is
            // inert on native rather than gated.
            primary_window: Some(Window {
                title: "flaymap_peel".into(),
                canvas: Some("#carnage-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(FlaymapPlugin)
        .init_resource::<Peel>()
        .add_systems(Startup, setup)
        .add_systems(Update, (peel, announce, redraw_hud).chain())
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let region = Region::Limb;
    // Intact skin, written INTO the canvas on the CPU — which is why there is no shader in this demo
    // and no asset in this crate.
    let canvas = FlayCanvas::new(
        &mut images,
        CANVAS,
        region,
        Layers::for_region(region),
        [0.80, 0.68, 0.62],
        0.55,
    );
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(canvas.albedo()),
        metallic_roughness_texture: Some(canvas.roughness()),
        // REQUIRED. Bevy multiplies this scalar by the texture
        // (`bevy_pbr-0.19.0/src/pbr_material.rs:157-163`), so the shipped default would scale the
        // roughness map away — and wet muscle against dry cortex is mostly a gloss difference.
        perceptual_roughness: 1.0,
        ..default()
    });

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 0.25).mesh())),
        MeshMaterial3d(material),
        // The canvas rides the entity it paints: `FlayCanvas` is a `Component`, and the plugin's
        // upload budget queries for it. Handing the images to a material and forgetting this is the
        // one mistake that looks like the crate doing nothing.
        canvas,
        Subject,
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.5, 1.9).looking_at(Vec3::ZERO, Vec3::Y),
        // A component in 0.19, and per-camera.
        AmbientLight { brightness: 380.0, ..default() },
    ));
    commands.spawn((
        DirectionalLight { illuminance: 6_000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_xyz(2.0, 3.0, 2.5).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Text::new(""),
        Node { position_type: PositionType::Absolute, top: px(12), left: px(12), ..default() },
        Hud,
    ));
}

/// **The caller's schedule, which is the whole point of the crate registering none of its own.**
///
/// One hit every [`EVERY`] frames, then `shade` once, then the orbit. The frame counter is a
/// `Local<u32>` rather than a `Time` read because this crate takes a tick *number*: a demo that
/// derived the cadence from wall-clock seconds would peel a different wound on a slower machine.
fn peel(
    mut tick: Local<u32>,
    settings: Res<FlaySettings>,
    mut subjects: Query<(Entity, &mut FlayCanvas), With<Subject>>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mut bone: MessageWriter<BoneExposed>,
) {
    let now = *tick;
    *tick = now.wrapping_add(1);

    if now % EVERY == 0 {
        for (entity, mut canvas) in &mut subjects {
            let handoff = canvas.paint_uv(SPOT, RADIUS, BITE_MM, now);
            // The crate never writes this message itself: only the caller knows whether the thing it
            // peeled has a skeleton something else owns.
            if let Some(msg) = BoneExposed::from_handoff(entity, &handoff) {
                bone.write(msg);
            }
            // After the last paint of the tick, before the plugin's upload budget runs.
            canvas.shade(&settings);
        }
    }

    // A slow orbit, off the same counter, so the crater can be read from every angle.
    let angle = now as f32 * 0.006;
    for mut cam in &mut cameras {
        cam.translation = Vec3::new(angle.sin() * 1.9, 0.5, angle.cos() * 1.9);
        cam.look_at(Vec3::ZERO, Vec3::Y);
    }
}

/// The reader on the other side of the handoff. In a game this is where a fracture proxy, a
/// bone-scrape sound or a wound-tier bump would be spawned; here it writes a line for the readout.
fn announce(mut inbox: MessageReader<BoneExposed>, mut peel: ResMut<Peel>) {
    for msg in inbox.read() {
        peel.bone_uv = Some(msg.uv);
        info!(
            "bone exposed on {} at uv ({:.2}, {:.2}); mesh point {:?}, normal {:?}",
            msg.entity, msg.uv.x, msg.uv.y, msg.at, msg.normal
        );
    }
}

fn redraw_hud(
    peel: Res<Peel>,
    subjects: Query<&FlayCanvas, With<Subject>>,
    mut hud: Query<&mut Text, With<Hud>>,
) {
    let Some(canvas) = subjects.iter().next() else {
        return;
    };
    let centre = canvas.size() / 2;
    let removed = canvas.depth_at(centre, centre).unwrap_or(0.0);

    // ASCII only, and not for tidiness: Bevy's embedded default font carries 95 codepoints, so an
    // em dash or a mid-dot in on-screen text draws as a tofu box. Confirmed in a capture of this
    // very readout.
    let bone = match peel.bone_uv {
        Some(uv) => format!(
            "BONE at uv ({:.2}, {:.2}) - {} texels of cortex",
            uv.x,
            uv.y,
            canvas.bone_texels()
        ),
        None => "no bone yet".to_string(),
    };
    let text = format!(
        "flaymap_peel - {:?}, {}x{} texels\n\
         removed at centre  {removed:6.2} mm\n\
         skin {}  fat {}  muscle {}  cortex {}  marrow {}\n\
         {bone}\n\
         digest 0x{:016x}",
        canvas.region(),
        canvas.size(),
        canvas.size(),
        canvas.exposed_area(Layer::Skin),
        canvas.exposed_area(Layer::Fat),
        canvas.exposed_area(Layer::Muscle),
        canvas.exposed_area(Layer::Cortex),
        canvas.exposed_area(Layer::Marrow),
        canvas.digest(),
    );
    for mut t in &mut hud {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}
