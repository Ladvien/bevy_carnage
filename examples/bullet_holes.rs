//! **A bullet hole that goes through, and you can put it where you like.**
//!
//! Every other example in this crate breaks the subject *apart*. This one leaves it standing and
//! takes a channel out of it: `Bore { from, to, radius, sides, jaggedness, flare }` is a convex prism
//! subtracted from the proxy before any cut, so the hole is real geometry — it has a wall, the wall
//! takes the interior material like every other cut face, and every shard around it is still a closed
//! convex solid and still one convex-hull collider.
//!
//! ```text
//!   arrows / WASD   move the aim marker
//!   Space           fire a channel straight through, entering at the marker
//!   [ / ]           smaller / larger calibre
//!   J               jaggedness — cycle how ragged the barrel is (0 / 0.35 / 0.7 / 1.0)
//!   F               flare — cycle how much wider the exit is (0 / 0.25 / 0.6)
//!   K               shatter — how many pieces the ejected plug breaks into (1 / 2 / 4 / 6 / 8)
//!   R               reset to an unbored subject
//! ```
//!
//! **Press `K` first.** At 1 the plug leaves whole, and a plug is one convex prism — so it reads as a
//! dowel, because the channel was cut by something corer-shaped and the material it removed is
//! exactly that shape. Anything above 1 breaks it with the crate's own cut policy, and the same shot
//! reads as gore.
//!
//! **A shot re-bakes.** A bore is a bake *input*, not damage applied afterwards, because a channel is
//! part of the subject's shape rather than part of its breakage. That is also why `sever` has no bore
//! key: re-baking there would reset exactly the accumulated severance the example exists to show.
//!
//! The subject and the bake are `common::body`, which `capture_holes.rs` drives too — so
//! `docs/holes.gif` is this example on rails rather than a re-implementation that could drift.
//!
//! Needs a GPU.
//!
//! Run: `cargo run -p bevy_carnage --example bullet_holes`

use bevy::prelude::*;
use bevy_carnage::Bore;

mod common;
use common::body::{self, BodyMaterials, Chunk, ORIGIN, SHOTS};
use common::light_and_floor;

/// Rendered flat: see `capture_holes.rs`'s `SOFTEN` for the measurement. Relaxing each shard's skin
/// independently opens a hairline along every wedge boundary radiating from a hole.
const SOFTEN: f32 = 0.0;

/// The coarsest frontier — the bore's own shards plus the body parts, with no fracture cut between
/// them. This example is about the channel, not about the break.
const GRANULARITY: usize = 0;

/// The calibres `[` and `]` step through, subject-local. The smallest is near the crate's own floor;
/// the largest is a cannon on a 1.0-tall subject.
const CALIBRES: [f32; 5] = [0.015, 0.025, 0.035, 0.05, 0.08];

/// The raggedness settings `J` cycles.
const JAGGEDNESS: [f32; 4] = [0.0, 0.35, 0.7, 1.0];

/// The exit-flare settings `F` cycles.
const FLARES: [f32; 3] = [0.0, 0.25, 0.6];

/// How many pieces the plug breaks into, as `K` cycles them. **1 is included deliberately**: it is the
/// corer look, and having it on the dial is the only way to see what the others are fixing.
const SHATTERS: [u32; 5] = [1, 2, 4, 6, 8];

/// **How far in front of the subject the aim marker floats.**
///
/// `Aim` is a point on the bore's *axis*, not on the surface — so drawn at the aim point itself the
/// marker sits **inside the torso** and is invisible, which is exactly what the first run of this
/// example showed. Pushed out along `+z` it sits on the line the shot travels instead.
///
/// **Small on purpose.** The first fix used 0.42, which is visible but wrong in a subtler way: the
/// camera is off-axis, so a marker that far forward parallaxes away from the hole it predicts and
/// stops being an aiming aid. The subject's own front faces sit at `z = 0.10` (arms) to `0.14`
/// (torso), so 0.20 clears the skin by more than the marker's radius while staying close enough that
/// marker and entry wound read as the same place. `sever.rs` has the same latent problem and a
/// different excuse: its blows are regions, not rays, so its marker has no line to sit on.
const MARKER_STANDOFF: f32 = 0.20;

/// Where the next shot enters, in subject-local space.
#[derive(Resource)]
struct Aim(Vec3);

/// Marks the little sphere that shows [`Aim`].
#[derive(Component)]
struct AimMarker;

/// **Every channel fired so far, and it is the whole state of this example.** Kept rather than
/// derived: each shot re-bakes from the accumulated list, so two overlapping shots produce one
/// channel rather than two contradictory ones.
#[derive(Resource, Default)]
struct Bores(Vec<Bore>);

/// The dials, as indices into [`CALIBRES`], [`JAGGEDNESS`] and [`FLARES`].
#[derive(Resource)]
struct Dials {
    calibre: usize,
    jaggedness: usize,
    flare: usize,
    shatter: usize,
}

impl Default for Dials {
    fn default() -> Self {
        // The middle calibre, and the shipped `Bore::new` look for the rest — including its
        // `shatter: 4`, which is index 2 here.
        Dials { calibre: 2, jaggedness: 1, flare: 1, shatter: 2 }
    }
}

/// Marks the line reporting the dials and what the last shot did.
///
/// **Everything that reaches this `Text` is ASCII, and that is a constraint rather than a style.**
/// Bevy's default font atlas has neither U+00B7 `·` nor U+2014 `—`, so both render as missing-glyph
/// boxes — found the first time this example was run, in the status line and then again in the message
/// `K` produces. `sever.rs` uses plain hyphens throughout for the same reason. The window title and
/// the `info!` logs are exempt: those go to the window manager and the terminal, not the atlas.
#[derive(Component)]
struct HudStatus;

/// What to say about the last shot.
#[derive(Resource)]
struct Status(String);

impl Default for Status {
    fn default() -> Self {
        Status("Space fires a channel through the subject at the marker".into())
    }
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage — bullet holes (Space to fire, arrows to aim, R reset)".into(),
                resolution: (960u32, 680u32).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Aim(SHOTS[0].1))
        .init_resource::<Bores>()
        .init_resource::<Dials>()
        .init_resource::<Status>()
        .init_resource::<body::Thrown>()
        .add_systems(Startup, setup)
        // `fire` re-bakes and throws; `integrate` and `bleed` carry what was thrown through its whole
        // life, from flying chunk to flat stain. Chained so a plug cannot be integrated and settled in
        // the same frame it was spawned.
        .add_systems(Update, (aim_marker, fire, integrate, body::bleed, hud).chain())
        .run();
}

fn setup(world: &mut World) {
    // Nearer than the other examples: a 0.035 hole on a 1.0-tall subject is a smudge at the shared
    // framing, and this example exists to look at the hole.
    //
    // **Aimed below `ORIGIN`, because `ORIGIN` is the subject's feet-on-floor anchor and not its
    // middle.** Pointed at `ORIGIN` the subject's legs ran off the bottom of the window — measured on
    // the first run of this example, at the shipped 960x680.
    let camera = Transform::from_xyz(1.50, 1.15, 1.95).looking_at(ORIGIN - Vec3::Y * 0.16, Vec3::Y);
    world.spawn((Camera3d::default(), camera));
    light_and_floor(world);

    let baked = body::Baked::bake(world, SOFTEN, &[]);
    let materials = BodyMaterials::new(world);
    let damage = body::Damage::fresh(&baked, GRANULARITY);

    let marker = world.resource_mut::<Assets<Mesh>>().add(Mesh::from(Sphere::new(0.035)));
    let aim = world.resource::<Aim>().0;
    world.spawn((
        AimMarker,
        Mesh3d(marker),
        MeshMaterial3d(materials.aim.clone()),
        Transform::from_translation(ORIGIN + aim + Vec3::Z * MARKER_STANDOFF),
    ));

    world.insert_resource(baked);
    world.insert_resource(materials);
    world.insert_resource(damage);
    body::stand(world, GRANULARITY);
    body::spawn_gore(world);
    spawn_hud(world);
}

/// **An on-screen legend, because without one the feature set is invisible.** Everything this example
/// can do lives on a key, and a window that opens with no text tells you none of it.
fn spawn_hud(world: &mut World) {
    world.spawn((
        Text::new(
            "arrows / WASD  aim\n        Space  fire a channel through the subject\n        [ / ]  calibre     J jaggedness     F flare     K shatter     R reset",
        ),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        Node { position_type: PositionType::Absolute, top: px(12), left: px(14), ..default() },
    ));
    world.spawn((
        HudStatus,
        Text::new(""),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(1.0, 0.92, 0.55, 0.95)),
        Node { position_type: PositionType::Absolute, bottom: px(14), left: px(14), ..default() },
    ));
}

/// Keep the status line current: what the last shot did, and where the four dials sit.
fn hud(
    status: Res<Status>,
    dials: Res<Dials>,
    bores: Res<Bores>,
    standing: Query<(), With<body::Attached>>,
    mut line: Query<&mut Text, With<HudStatus>>,
) {
    let text = format!(
        // **ASCII only, and that is not fussiness.** The `·` separators here rendered as
        // missing-glyph boxes the first time this example was ever actually run: Bevy's default font
        // atlas has no U+00B7. The legend above survived because it was already ASCII.
        "{}\n{} hole(s)  |  {} shards standing  |  radius {:.3}  |  jaggedness {:.2}  |  flare \
         {:.2}  |  plug into {}",
        status.0,
        bores.0.len(),
        standing.iter().count(),
        CALIBRES[dials.calibre],
        JAGGEDNESS[dials.jaggedness],
        FLARES[dials.flare],
        SHATTERS[dials.shatter],
    );
    for mut t in &mut line {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}

/// Move the aim marker, and keep the sphere on it. Same clamp as `sever`'s.
fn aim_marker(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut aim: ResMut<Aim>,
    mut marker: Query<&mut Transform, With<AimMarker>>,
) {
    let step = 1.1 * time.delta_secs();
    let mut d = Vec3::ZERO;
    for (key, delta) in [
        (KeyCode::ArrowUp, Vec3::Y),
        (KeyCode::KeyW, Vec3::Y),
        (KeyCode::ArrowDown, -Vec3::Y),
        (KeyCode::KeyS, -Vec3::Y),
        (KeyCode::ArrowLeft, -Vec3::X),
        (KeyCode::KeyA, -Vec3::X),
        (KeyCode::ArrowRight, Vec3::X),
        (KeyCode::KeyD, Vec3::X),
    ] {
        if keys.pressed(key) {
            d += delta;
        }
    }
    aim.0 += d * step;
    aim.0 = aim.0.clamp(Vec3::new(-0.8, -0.7, -0.6), Vec3::new(0.8, 1.2, 0.6));
    for mut t in &mut marker {
        t.translation = ORIGIN + aim.0 + Vec3::Z * MARKER_STANDOFF;
    }
}

/// Read the keyboard, fire a channel, and re-bake.
///
/// An exclusive system, because the bake works on `&mut World` — the shape the headless recorder can
/// drive too, and sharing it is what keeps `docs/holes.gif` honest.
fn fire(world: &mut World) {
    let pressed =
        |world: &World, key: KeyCode| world.resource::<ButtonInput<KeyCode>>().just_pressed(key);

    if pressed(world, KeyCode::BracketLeft) || pressed(world, KeyCode::BracketRight) {
        let up = pressed(world, KeyCode::BracketRight);
        let mut d = world.resource_mut::<Dials>();
        d.calibre = if up {
            (d.calibre + 1).min(CALIBRES.len() - 1)
        } else {
            d.calibre.saturating_sub(1)
        };
        let now = CALIBRES[d.calibre];
        world.resource_mut::<Status>().0 = format!("calibre {now:.3} - fire (Space) to see it");
        return;
    }
    if pressed(world, KeyCode::KeyJ) {
        let mut d = world.resource_mut::<Dials>();
        d.jaggedness = (d.jaggedness + 1) % JAGGEDNESS.len();
        let now = JAGGEDNESS[d.jaggedness];
        world.resource_mut::<Status>().0 =
            format!("jaggedness {now:.2} - each barrel plane bites inward, never outward");
        return;
    }
    if pressed(world, KeyCode::KeyK) {
        let mut d = world.resource_mut::<Dials>();
        d.shatter = (d.shatter + 1) % SHATTERS.len();
        let now = SHATTERS[d.shatter];
        world.resource_mut::<Status>().0 = if now == 1 {
            "plug into 1 - whole, which is one convex prism: the corer look".into()
        } else {
            format!("plug into {now} - broken by the same cut policy the body uses")
        };
        return;
    }
    if pressed(world, KeyCode::KeyF) {
        let mut d = world.resource_mut::<Dials>();
        d.flare = (d.flare + 1) % FLARES.len();
        let now = FLARES[d.flare];
        world.resource_mut::<Status>().0 =
            format!("flare {now:.2} - the exit is that much wider than the entry");
        return;
    }

    let reset = pressed(world, KeyCode::KeyR);
    let shot = pressed(world, KeyCode::Space);
    if !reset && !shot {
        return;
    }

    if reset {
        world.resource_mut::<Bores>().0.clear();
    }
    if shot {
        let at = world.resource::<Aim>().0;
        let (radius, jaggedness, flare, shatter) = {
            let d = world.resource::<Dials>();
            (CALIBRES[d.calibre], JAGGEDNESS[d.jaggedness], FLARES[d.flare], SHATTERS[d.shatter])
        };
        let bore = Bore { jaggedness, flare, ..body::bore_at(at, radius, shatter) };
        info!(
            "bore: entry {:?} → {:?}, radius {radius}, {} sides, jaggedness {jaggedness}, flare \
             {flare}, plug into {shatter} — re-baking, because a channel is part of the subject's \
             shape",
            bore.from, bore.to, bore.sides
        );
        world.resource_mut::<Bores>().0.push(bore);
    }

    let bores = world.resource::<Bores>().0.clone();
    body::clear(world);
    let baked = body::Baked::bake(world, SOFTEN, &bores);
    let damage = body::Damage::fresh(&baked, GRANULARITY);
    world.insert_resource(baked);
    world.insert_resource(damage);
    body::stand(world, GRANULARITY);
    // **After `stand`, and only the plugs not already thrown.** A reset clears the bore list, so the
    // counter goes back with it and the next shot's gore is thrown fresh.
    if reset {
        body::wipe(world);
        world.resource_mut::<body::Thrown>().0 = 0;
    }
    body::spawn_gore(world);

    let standing = world.resource::<body::Baked>().tree.roots().len();
    world.resource_mut::<Status>().0 = if reset {
        "reset - an unbored subject".into()
    } else {
        format!("fired: {} hole(s), the proxy is now {standing} cells", bores.len())
    };
}

/// The example's whole solver for the ejected plugs — the crate names none.
fn integrate(time: Res<Time>, mut chunks: Query<(&mut Chunk, &mut Transform)>) {
    let dt = time.delta_secs() * 0.55;
    if dt <= 0.0 {
        return;
    }
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, dt);
    }
}
