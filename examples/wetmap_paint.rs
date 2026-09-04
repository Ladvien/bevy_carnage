//! **A wetmap you can hash** — demo 8 on the site, and the same model with a renderer attached.
//!
//! Blood lands where the subject was hit **in UV space**, runs down the actual geometry, pools where
//! the surface turns, and dries in place. The number on screen is the canvas's own digest, taken over
//! the CPU buffer: paint the same sequence twice and it matches, move one hit by a single texel and it
//! does not. Every other texture-space blood system is a GPU render target, which is why nobody can
//! hash one. Dry paint does not move — only wet paint drips or spreads, which is what makes a run stop
//! where it stopped.
//!
//! The key legend below is duplicated verbatim into `web/play.html`'s `#notes-wetmap_paint` block, and
//! **that block is the spec**: if the two disagree, the page is right and this file is wrong.
//!
//! `cargo run -p bevy_wetmap --example wetmap_paint`

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_carnage::wetmap::{StainShape, WetCanvas, WetSettings, WetmapPlugin};

/// Edge length of the canvas, in texels — the crate's shipped default. 64 KB per upload.
const CANVAS: u32 = 128;

/// The four directions gravity can point on a texture, and what to call each one.
///
/// Four rather than a slider, because the drip pass quantises to the dominant axis anyway. Bevy's UV
/// sphere puts `v = 0` at its `+Z` pole and the subject is rotated so that pole faces world `+Y`, so
/// `+v` really is down — but which way is down on an atlas is a property of the atlas, which is why
/// the crate takes it as an argument and this demo lets you turn it.
const GRAVITIES: [(Vec2, &str); 4] = [
    (Vec2::new(0.0, 1.0), "+v  (down the sphere)"),
    (Vec2::new(1.0, 0.0), "+u  (around it)"),
    (Vec2::new(0.0, -1.0), "-v  (up)"),
    (Vec2::new(-1.0, 0.0), "-u  (around, the other way)"),
];

/// Everything this demo owns beyond the canvas itself: an integer tick, which way gravity points, and
/// whether the digest is on screen.
#[derive(Resource)]
struct Demo {
    /// **The tick, counted here.** The crate refuses to read a clock, so the caller keeps this — and
    /// `std::time::Instant` in particular compiles for wasm and then panics in the browser.
    tick: u32,
    gravity: usize,
    show_digest: bool,
    mesh: Handle<Mesh>,
}

#[derive(Component)]
struct Actor;

#[derive(Component)]
struct Hud;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            // The settled shape for every demo on the site. `canvas` has no effect off the web
            // (`bevy_window-0.19.0/src/window.rs:250`), so it is inert on native rather than gated.
            primary_window: Some(Window {
                title: "wetmap_paint".into(),
                canvas: Some("#carnage-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(WetmapPlugin)
        // 60 Hz, because `WetSettings::dry_ticks` is quoted at 60 Hz. Bevy's default fixed rate is 64,
        // which would silently make the shipped 1800 ticks 28 seconds instead of 30.
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, advance_canvases)
        .add_systems(Update, (shoot_the_subject, keyboard, redraw_hud))
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    // A pale, slightly glossy skin, composited INTO the canvas on the CPU — which is why there is no
    // shader in this demo and no asset in this crate.
    let canvas = WetCanvas::new(&mut images, CANVAS, [0.80, 0.68, 0.62], 0.55);
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(canvas.albedo()),
        metallic_roughness_texture: Some(canvas.roughness()),
        // REQUIRED. Bevy multiplies these scalars by the texture
        // (`bevy_pbr-0.19.0/src/pbr_material.rs:157-163`), so the shipped defaults would scale the
        // roughness map away and lose the wet/dry contrast entirely.
        perceptual_roughness: 1.0,
        metallic: 1.0,
        ..default()
    });

    let mesh = meshes.add(Sphere::new(0.5).mesh().uv(64, 32));
    commands.insert_resource(Demo { tick: 0, gravity: 0, show_digest: true, mesh: mesh.clone() });
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        // Pole to +Y, so `GRAVITIES[0]` is honest.
        Transform::from_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
        canvas,
        Actor,
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.25, 1.7).looking_at(Vec3::ZERO, Vec3::Y),
        // A component in 0.19, and per-camera.
        AmbientLight { brightness: 420.0, ..default() },
    ));
    commands.spawn((
        DirectionalLight { illuminance: 5_500.0, shadow_maps_enabled: false, ..default() },
        Transform::from_xyz(2.0, 3.0, 2.5).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Text::new(""),
        Node { position_type: PositionType::Absolute, top: px(12), left: px(12), ..default() },
        Hud,
    ));
}

/// The caller's schedule, which is the whole point of the crate registering none of its own.
fn advance_canvases(
    mut demo: ResMut<Demo>,
    settings: Res<WetSettings>,
    mut canvases: Query<&mut WetCanvas>,
) {
    demo.tick = demo.tick.wrapping_add(1);
    let (gravity, _) = GRAVITIES[demo.gravity % GRAVITIES.len()];
    let tick = demo.tick;
    for mut canvas in &mut canvases {
        canvas.tick(tick, gravity, &settings);
    }
}

/// Click: a ray from the cursor, straight into `paint_world`.
fn shoot_the_subject(
    buttons: Res<ButtonInput<MouseButton>>,
    demo: Res<Demo>,
    meshes: Res<Assets<Mesh>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut actors: Query<(&GlobalTransform, &mut WetCanvas), With<Actor>>,
) {
    // Every one of these is a legitimate "nothing to do this frame": no button, no camera yet, no
    // cursor over the window, the mesh asset not resident, or a cursor outside the viewport.
    if !buttons.pressed(MouseButton::Left) {
        return;
    }
    let (Ok(window), Ok((camera, camera_xf))) = (windows.single(), cameras.single()) else {
        return;
    };
    let (Some(cursor), Some(mesh)) = (window.cursor_position(), meshes.get(&demo.mesh)) else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(camera_xf, cursor) else {
        return;
    };
    for (actor_xf, mut canvas) in &mut actors {
        // A miss returns `false` and paints nothing — which is why this loop ignores the answer: the
        // caller who cares about a miss is the one deciding whether to play an impact sound.
        canvas.paint_world(mesh, actor_xf, ray.origin, *ray.direction, &shot(demo.tick), demo.tick);
    }
}

/// The keyboard, in one pass.
///
/// `Space` puts three stains near the top, placed by UV rather than by ray so they have the whole
/// sphere to run down. `G` turns gravity: a run already laid down does not restart, because dry paint
/// has stopped — only the wet paint starts running the new way, which is the clearest demonstration
/// the model has. `D` hides the digest.
fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut demo: ResMut<Demo>,
    mut actors: Query<&mut WetCanvas, With<Actor>>,
) {
    if keys.just_pressed(KeyCode::KeyD) {
        demo.show_digest = !demo.show_digest;
    }
    if keys.just_pressed(KeyCode::KeyG) {
        demo.gravity = (demo.gravity + 1) % GRAVITIES.len();
    }
    if !keys.just_pressed(KeyCode::Space) {
        return;
    }
    let tick = demo.tick;
    for mut canvas in &mut actors {
        for (i, u) in [0.34_f32, 0.50, 0.66].into_iter().enumerate() {
            canvas.paint_uv(Vec2::new(u, 0.16), &shot(tick ^ (i as u32 * 977)), tick);
        }
    }
}

fn redraw_hud(
    demo: Res<Demo>,
    settings: Res<WetSettings>,
    actors: Query<&WetCanvas, With<Actor>>,
    mut hud: Query<&mut Text, With<Hud>>,
) {
    let (Ok(canvas), Ok(mut text)) = (actors.single(), hud.single_mut()) else {
        return;
    };
    let (_, gravity_label) = GRAVITIES[demo.gravity % GRAVITIES.len()];
    let digest = if demo.show_digest {
        format!("digest        0x{:016x}\n", canvas.digest())
    } else {
        String::from("digest        hidden  (D)\n")
    };
    text.0 = format!(
        "wetmap_paint — a wetmap you can hash\n\
         \n\
         {digest}\
         wetted area   {area:.6} m^2\n\
         gravity_uv    {gravity_label}\n\
         tick          {tick}   dries in {dry} ticks at 60 Hz\n\
         \n\
         click  shoot the subject\n\
         space  a burst\n\
         G      gravity direction\n\
         D      the digest",
        area = canvas.wetted_area(),
        tick = demo.tick,
        dry = settings.dry_ticks,
    );
}

/// One shot's stain silhouette.
///
/// Built by hand rather than through `bevy_carnage::bloodstain::stain::stain_shape`, so this demo has exactly one
/// subject: the wetmap. The morphology model has its own demo, `stain_morphology`.
fn shot(seed: u32) -> StainShape {
    StainShape {
        // ~4.5 cm long, which is six texels at this canvas size — enough to read, and enough to run.
        major: 0.045,
        minor: 0.030,
        spines: 5,
        satellites: 2,
        direction: [0.0, 1.0],
        seed: seed.wrapping_mul(0x9E37_79B9) ^ 0x5EED,
    }
}
