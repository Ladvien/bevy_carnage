//! **The same kill, replayed at four tiers. Reduction is substitution, never deletion.**
//!
//! At [`GoreTier::Stylised`] the emitter still fires, on the same tick, in the same direction, at the
//! same magnitude — it swaps the blood palette for [`CarnageSettings::substitute_srgb`], a spark, and
//! drops mutilation. Vermintide 2's gore-off deleted the channel and made the game *harder to read*;
//! Gears of War 4 replaced blood with sparks and kept the hit confirmation. So there is exactly one
//! emitter path here, parameterised: [`start_kill`] always builds the same spray from the same wound
//! and the policy chooses only the **palette** and whether the subject comes apart.
//!
//! The stops map onto **rating descriptors** rather than taste: ESRB's *Animated Blood*, *Blood* and
//! *Blood and Gore* (defined as mutilation of body parts), and PEGI's gross-violence criterion, which
//! turns on emphasis and persistence.
//!
//! `F` holds down the flash button, and the meter shows [`FlashGate`] **refusing the fourth flash
//! inside any one second** — WCAG 2.1 SC 2.3.1, technique G19, whose safe harbour is 3 Hz. Hold it
//! and the overlay flashes three times a second however many times a second it is asked to; the
//! refusal counter climbs at 57 a second.
//!
//! `A` draws the aim-exclusion cone. Screen blood that would land on the reticle is refused by
//! [`occludes_aim`], because gaze concentrates at screen centre while aiming. Ten degrees, which is
//! the visual-field unit WCAG itself uses — not thirty, which is folklore. **The ring's radius is
//! bisected out of `occludes_aim` itself** rather than recomputed from a copy of its arithmetic: the
//! crate's half-field constant is private, and a second copy of it would drift silently.
//!
//! ```text
//!   1 / 2 / 3 / 4   Stylised / Blood / BloodAndGore / GrossViolence
//!   Space           replay the kill
//!   F               hold to spam flashes
//!   A               the aim cone
//! ```
//!
//! **Blood is drawn with gizmos, not with decals.** `spawn_stain` and the forward-decal path live
//! behind the `vfx` feature, and every wasm demo is built with `vfx` off — so each landed stain is an
//! ellipse whose half-axes and direction come straight from [`bevy_carnage::StainShape`].
//!
//! Run: `cargo run -p bevy_carnage --example gore_tier`

use bevy::prelude::*;
use bevy_carnage::{
    CarnageSettings, CutSettings, FlashGate, GorePolicy, GoreTier, WCAG_FLASHES_PER_SECOND, Wound,
    WoundKind, fracture_mesh, hash_f32, occludes_aim, wound_seed,
};

mod common;
use common::body;

/// The demo's fixed tick. [`FlashGate`] is an integer-tick gate, so the rate it is asked about has to
/// be the rate it is actually driven at — hence a pinned `Time<Fixed>` rather than a frame count.
const HZ: u32 = 60;
/// Ticks one admitted flash is drawn for. 6 is 0.1 s.
const FLASH_TICKS: u32 = 6;
/// How long one replay lasts before the subject is restored.
const KILL_TICKS: u32 = 340;
/// Ticks a landed stain takes to reach its full silhouette. A stain spreads on impact; this is that.
const SPREAD_TICKS: u32 = 20;
/// **The drawn lifetime [`GorePolicy::persistence_scale`] scales, in ticks.**
///
/// The shipped [`CarnageSettings::stain_lifetime_ticks`] is 3600 — a minute at 60 Hz — so at any tier
/// nothing would visibly fade inside a 5.7 s replay. This is that lifetime compressed so the dial is
/// legible, and the real number is printed on screen beside it.
const PERSIST_REF_TICKS: u32 = 260;
/// Slower than real time, because a gib set is tuned to read in a fraction of a second and the burst
/// would be over before a freshly-mapped canvas had drawn its first frames.
const PLAYBACK: f32 = 0.45;

/// Fragment count for one dismemberment, and the seed. **One seed, so the launch is the same take at
/// every tier** — `body::launch` is a pure function of the fragment and the blow.
const TARGET: usize = 16;
const MIN_FRACTION: f32 = 0.10;
const SEED: u32 = 0x00C0_FFEE;

/// Where the blow landed, subject-local — the front of the torso.
const WOUND_LOCAL: Vec3 = Vec3::new(0.0, 0.10, 0.14);
/// The floor the spray stains.
const PLANE_Y: f32 = 0.0;
/// Screen-blood marks one kill throws at the camera plane.
const SCREEN_SPLATS: usize = 14;

/// The unbroken subject.
#[derive(Component)]
struct Intact;

/// Marks the status block.
///
/// **ASCII only in every `Text`.** Bevy's default font atlas has neither U+00B7 nor U+2014, so both
/// render as missing-glyph boxes — `bullet_holes.rs` found that the first time it ran.
#[derive(Component)]
struct HudStatus;

/// The full-screen flash.
#[derive(Component)]
struct FlashOverlay;

/// One of the three slots in the flash meter.
#[derive(Component)]
struct FlashSlot(u32);

/// The aim-exclusion ring.
#[derive(Component)]
struct AimRing;

/// One screen-blood mark. Spawned once at startup and shown or hidden, so a tier change is visible
/// on the next frame without any spawn churn.
#[derive(Component)]
struct ScreenSplatNode(usize);

#[derive(Resource, Default)]
struct Tick(u32);

/// One landed stain, as drawn: the crate's own silhouette, in world space.
struct Mark {
    at: Vec3,
    half: Vec2,
    angle: f32,
}

/// One screen-blood mark, in normalised device coordinates — the space [`occludes_aim`] judges.
struct Splat {
    ndc: Vec2,
    radius_ndc: f32,
}

#[derive(Resource, Default)]
struct Kill {
    /// Ticks since the wound opened, or `None` between takes.
    age: Option<u32>,
    marks: Vec<Mark>,
    splats: Vec<Splat>,
    /// Screen marks [`occludes_aim`] refused this frame.
    refused: u32,
    /// Set by `Space`, consumed by [`step`].
    replay: bool,
    /// Set by a tier change or by a finished take.
    reset: bool,
}

#[derive(Resource, Default)]
struct Flash {
    held: bool,
    /// Ticks of flash left to draw. **A countdown rather than a deadline**, so nothing has to reason
    /// about a wrapped tick and `Default`'s zero is honestly "dark" instead of "lit at tick 0".
    lit_ticks: u32,
    attempts: u32,
    admitted: u32,
    refused: u32,
}

#[derive(Resource, Default)]
struct Show {
    aim: bool,
}

#[derive(Resource)]
struct Mats {
    skin: Handle<StandardMaterial>,
    interior: Handle<StandardMaterial>,
}

fn tier_name(t: GoreTier) -> &'static str {
    match t {
        GoreTier::Stylised => "Stylised          (ESRB Animated Blood)",
        GoreTier::Blood => "Blood             (ESRB Blood)",
        GoreTier::BloodAndGore => "BloodAndGore      (ESRB Blood and Gore: mutilation of body parts)",
        GoreTier::GrossViolence => "GrossViolence     (PEGI gross violence: emphasis and persistence)",
    }
}

/// **The exclusion radius in NDC, taken from [`occludes_aim`] itself.**
///
/// Bisection on the predicate rather than a copy of its arithmetic: the crate's nominal half-field is
/// private, so a local constant would be a second source of truth that could drift without anything
/// going red. Twenty-four halvings of `[0, 2]` resolve it to well under a pixel.
fn exclusion_ndc(policy: &GorePolicy) -> f32 {
    if !occludes_aim(Vec2::ZERO, 0.0, policy) {
        return 0.0;
    }
    let (mut lo, mut hi) = (0.0f32, 2.0f32);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if occludes_aim(Vec2::new(mid, 0.0), 0.0, policy) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The screen-blood marks one wound throws, from the crate's frozen generator — so the set replays
/// bit-for-bit and is identical at every tier.
///
/// Deliberately spread across the exclusion boundary: some land on the reticle and some do not, which
/// is the only way a refusal count means anything.
fn screen_splats(seed: u32) -> Vec<Splat> {
    (0..SCREEN_SPLATS as u32)
        .map(|i| {
            let a = hash_f32(seed ^ i.wrapping_mul(0x9E37_79B9));
            let b = hash_f32(seed ^ i.wrapping_mul(0x85EB_CA6B).wrapping_add(7));
            let c = hash_f32(seed ^ i.wrapping_mul(0xC2B2_AE35).wrapping_add(13));
            let angle = a * std::f32::consts::TAU;
            let r = 0.05 + 0.66 * b;
            Splat {
                ndc: Vec2::new(angle.cos() * r, angle.sin() * r),
                radius_ndc: 0.018 + 0.045 * c,
            }
        })
        .collect()
}

/// The blood palette this policy paints with. **The only thing the tier changes about the emitter.**
fn blood_color(policy: &GorePolicy, s: &CarnageSettings, alpha: f32) -> Color {
    if policy.draws_blood() {
        Color::srgba(0.60, 0.04, 0.04, alpha)
    } else {
        // `substitute_srgb` is documented as linear sRGB, so it is read as linear.
        let [r, g, b] = s.substitute_srgb;
        Color::linear_rgba(r, g, b, alpha)
    }
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage - gore tier".into(),
                // The one web line, inert on native.
                canvas: Some("#carnage-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Time::<Fixed>::from_hz(HZ as f64))
        .insert_resource(GorePolicy::for_tier(GoreTier::BloodAndGore))
        .insert_resource(CarnageSettings {
            // **The measured 8-40 m/s spray throws blood tens of metres**, which is right for a real
            // wound and useless in a three-metre frame. `spatter_speed_scale` is the shipped dial for
            // exactly that; it is set once here and is the same for every tier, so the spray is the
            // same take throughout.
            blood: bevy_carnage::BloodSettings {
                spatter_speed_scale: 0.12,
                ..default()
            },
            ..default()
        })
        .init_resource::<FlashGate>()
        .init_resource::<Tick>()
        // Present from the start even though `setup` replaces it: a missing `Res<T>` panics the
        // system that asks for it rather than skipping, and a browser has nowhere to show the panic.
        .init_resource::<Kill>()
        .init_resource::<Flash>()
        .init_resource::<Show>()
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, step)
        .add_systems(Update, (input, draw_blood, meter, aim_ring, screen_blood, hud))
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    settings: Res<CarnageSettings>,
) {
    // **The one spray, built once**, before any tier has been chosen. Logged so a native run reports
    // what the emitter produced without anyone having to press a key.
    let (marks, splats) = spray(&settings);
    info!("the kill: {} landed stains, {} screen marks", marks.len(), splats.len());
    commands.insert_resource(Kill { marks, splats, ..default() });
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.05, 2.35).looking_at(body::ORIGIN + Vec3::Y * 0.08, Vec3::Y),
    ));
    // A fill, so an unlit cut face reads as shadowed rather than as a hole in the chunk.
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.62, 0.66, 0.78),
        brightness: 900.0,
        ..default()
    });
    commands.spawn((
        DirectionalLight { illuminance: 9_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(4.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Mesh::from(Plane3d::default().mesh().size(14.0, 14.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.16, 0.16, 0.18),
            perceptual_roughness: 0.95,
            ..default()
        })),
    ));
    let mats = Mats {
        skin: materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.42, 0.52),
            perceptual_roughness: 0.85,
            ..default()
        }),
        interior: materials.add(StandardMaterial {
            base_color: Color::srgb(0.46, 0.07, 0.07),
            perceptual_roughness: 0.42,
            ..default()
        }),
    };
    spawn_intact(&mut commands, &mut meshes, &mats);
    commands.insert_resource(mats);

    // Behind everything, so the legend stays readable while the screen flashes.
    commands.spawn((
        FlashOverlay,
        GlobalZIndex(-1),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100.0),
            height: percent(100.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
    ));
    commands.spawn((
        AimRing,
        // **`BorderRadius` is a field of `Node` in 0.19, not a component** — passing it in the tuple
        // is not a bundle at all.
        Node {
            position_type: PositionType::Absolute,
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BorderColor::all(Color::srgba(0.45, 0.85, 1.0, 0.85)),
    ));
    for i in 0..SCREEN_SPLATS {
        commands.spawn((
            ScreenSplatNode(i),
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(Color::NONE),
        ));
    }
    // The flash meter: one slot per admitted flash the gate will allow in a second.
    for i in 0..WCAG_FLASHES_PER_SECOND {
        commands.spawn((
            FlashSlot(i),
            Node {
                position_type: PositionType::Absolute,
                bottom: px(112),
                left: px(14 + 30 * i as i32),
                width: px(24),
                height: px(14),
                border: UiRect::all(px(1)),
                ..default()
            },
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.5)),
            BackgroundColor(Color::NONE),
        ));
    }

    commands.spawn((
        Text::new(
            "1 Stylised   2 Blood   3 BloodAndGore   4 GrossViolence   Space replay   F flashes   \
             A aim cone\n\
             The same kill, replayed at four tiers. Reduction is substitution, never deletion: at\n\
             Stylised the emitter still fires, on the same tick, in the same direction, at the same\n\
             magnitude - it swaps the blood palette for a spark. Vermintide 2's gore-off deleted the\n\
             channel and made the game harder to read; Gears of War 4 replaced blood with sparks and\n\
             kept the hit confirmation. The stops map onto rating descriptors rather than taste.\n\
             F holds the flash button and the meter shows the gate refusing the fourth flash inside\n\
             any one second - WCAG 2.1 SC 2.3.1, technique G19, safe harbour 3 Hz. A draws the\n\
             aim-exclusion cone: decals that would land on the reticle are refused, because gaze\n\
             concentrates at screen centre while aiming. Ten degrees, the visual-field unit WCAG\n\
             itself uses - not thirty, which is folklore.",
        ),
        TextFont { font_size: FontSize::Px(14.0), ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        Node { position_type: PositionType::Absolute, top: px(12), left: px(14), ..default() },
    ));
    commands.spawn((
        HudStatus,
        Text::new(""),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(1.0, 0.92, 0.55, 0.95)),
        Node { position_type: PositionType::Absolute, bottom: px(14), left: px(14), ..default() },
    ));
}

/// The subject before anything happens to it — one entity per shell, the skin material on both.
fn spawn_intact(commands: &mut Commands, meshes: &mut Assets<Mesh>, mats: &Mats) {
    for (mesh, xform) in body::subject() {
        commands.spawn((
            Intact,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mats.skin.clone()),
            Transform::from_matrix(Mat4::from_translation(body::ORIGIN) * xform),
        ));
    }
}

/// Keys only. Every scene mutation happens on the fixed tick, so a replay lands on a tick the flash
/// gate and the kill clock agree about.
fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mut policy: ResMut<GorePolicy>,
    mut kill: ResMut<Kill>,
    mut flash: ResMut<Flash>,
    mut show: ResMut<Show>,
) {
    for (key, tier) in [
        (KeyCode::Digit1, GoreTier::Stylised),
        (KeyCode::Digit2, GoreTier::Blood),
        (KeyCode::Digit3, GoreTier::BloodAndGore),
        (KeyCode::Digit4, GoreTier::GrossViolence),
    ] {
        if keys.just_pressed(key) && policy.tier != tier {
            *policy = GorePolicy::for_tier(tier);
            kill.reset = true;
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        kill.replay = true;
    }
    if keys.just_pressed(KeyCode::KeyA) {
        show.aim = !show.aim;
    }
    flash.held = keys.pressed(KeyCode::KeyF);
}

/// The tick: the flash gate, the kill clock, and the debris.
fn step(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mats: Res<Mats>,
    policy: Res<GorePolicy>,
    mut gate: ResMut<FlashGate>,
    mut tick: ResMut<Tick>,
    mut flash: ResMut<Flash>,
    mut kill: ResMut<Kill>,
    intact: Query<Entity, With<Intact>>,
    chunks: Query<Entity, With<body::Chunk>>,
    mut moving: Query<(&mut body::Chunk, &mut Transform)>,
) {
    tick.0 = tick.0.wrapping_add(1);
    let now = tick.0;

    flash.lit_ticks = flash.lit_ticks.saturating_sub(1);
    if flash.held {
        flash.attempts = flash.attempts.wrapping_add(1);
        if gate.admit(now, HZ, &policy) {
            flash.admitted = flash.admitted.wrapping_add(1);
            flash.lit_ticks = FLASH_TICKS;
        } else {
            flash.refused = flash.refused.wrapping_add(1);
        }
    }

    if kill.replay || kill.reset {
        let replay = kill.replay;
        kill.replay = false;
        kill.reset = false;
        for e in chunks.iter().chain(intact.iter()) {
            commands.entity(e).despawn();
        }
        kill.age = None;
        if replay {
            begin_kill(&mut commands, &mut meshes, &mats, &policy, &mut kill);
        } else {
            spawn_intact(&mut commands, &mut meshes, &mats);
        }
    } else if let Some(age) = kill.age {
        if age >= KILL_TICKS {
            kill.reset = true;
        } else {
            kill.age = Some(age + 1);
        }
    }

    let dt = PLAYBACK / HZ as f32;
    for (mut chunk, mut transform) in &mut moving {
        body::integrate(&mut chunk, &mut transform, dt);
    }
}

/// **The whole spray, computed once.**
///
/// It is a pure function of the wound, so "the same kill at every tier" stops being a claim about
/// reproducibility and becomes structural: there is one spray, built at startup, and a tier cannot
/// reach it. What a tier chooses is the **palette** ([`blood_color`]) and whether the subject comes
/// apart ([`begin_kill`]) — never the emitter.
fn spray(settings: &CarnageSettings) -> (Vec<Mark>, Vec<Splat>) {
    let wound = Wound {
        at: body::ORIGIN + WOUND_LOCAL,
        normal: Vec3::new(0.0, -0.25, 0.968),
        area: 0.020,
        severity: 1.0,
        kind: WoundKind::Channel,
    };
    let blood = common::blood_wound(&wound);
    let marks = common::stains_with_shapes(&blood, &settings.blood, PLANE_Y)
        .into_iter()
        .map(|(stain, shape)| Mark {
            at: common::v3(stain.at),
            half: Vec2::new(shape.major * 0.5, shape.minor * 0.5),
            angle: shape.direction[1].atan2(shape.direction[0]),
        })
        .collect();
    (marks, screen_splats(wound_seed(&blood)))
}

/// Open the wound. **The policy chooses the subject's form and nothing else** — the spray was already
/// built, at startup, identically for every tier.
fn begin_kill(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &Mats,
    policy: &GorePolicy,
    kill: &mut Kill,
) {
    kill.age = Some(0);
    info!(
        "tier {:?}: {} stains, {} screen marks, dismemberment {}",
        policy.tier,
        kill.marks.len(),
        kill.splats.len(),
        policy.dismemberment
    );

    if !policy.dismemberment {
        // **The kill still happened.** Mutilation is the channel this tier drops; the subject takes
        // the hit and stays whole, and the blood emitter above already fired.
        spawn_intact(commands, meshes, mats);
        return;
    }
    let owned = body::subject();
    let parts: Vec<(&Mesh, Mat4)> = owned.iter().map(|(m, x)| (m, *x)).collect();
    let cut = CutSettings::new(TARGET, MIN_FRACTION, SEED);
    for f in fracture_mesh(&parts, &body::proxy(), &cut).into_leaves() {
        // `body::launch` is a pure function of the fragment and the blow — the crate's own frozen
        // hash, no RNG — so every tier that dismembers throws the same pieces the same way.
        let (velocity, spin) = body::launch(f.id, f.center_local, WOUND_LOCAL, f.cell.volume());
        let lowest = f.cell.points().iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        let drop_to_rest = (f.cell.center().y - lowest).max(0.0);
        let chunk = commands
            .spawn((
                body::Chunk { velocity, spin, drop_to_rest, fragment: Some(f.id) },
                Transform::from_translation(body::ORIGIN + f.center_local),
                Visibility::default(),
            ))
            .id();
        commands.entity(chunk).with_children(|parent| {
            if let Some(outer) = f.outer {
                parent.spawn((Mesh3d(meshes.add(outer)), MeshMaterial3d(mats.skin.clone())));
            }
            if let Some(cap) = f.cap {
                parent.spawn((Mesh3d(meshes.add(cap)), MeshMaterial3d(mats.interior.clone())));
            }
        });
    }
}

/// The landed spray, as the crate's own stain silhouettes. Fades over
/// [`PERSIST_REF_TICKS`] × [`GorePolicy::persistence_scale`].
fn draw_blood(
    mut gizmos: Gizmos,
    kill: Res<Kill>,
    policy: Res<GorePolicy>,
    settings: Res<CarnageSettings>,
) {
    let Some(age) = kill.age else { return };
    if !policy.blood_decals {
        return;
    }
    let life = (PERSIST_REF_TICKS as f32 * policy.persistence_scale).max(1.0);
    let alpha = 1.0 - age as f32 / life;
    if alpha <= 0.0 {
        return;
    }
    // A stain spreads on impact; sqrt so it opens fast and settles.
    let grow = (age as f32 / SPREAD_TICKS as f32).min(1.0).sqrt();
    let color = blood_color(&policy, &settings, alpha.min(1.0));
    // The gizmo ellipse lies in its own XY plane, so this lays it on the floor and then turns it to
    // the direction the droplet was travelling.
    let flat = Quat::from_rotation_arc(Vec3::Z, Vec3::Y);
    for m in &kill.marks {
        gizmos.ellipse(
            Isometry3d::new(m.at, flat * Quat::from_rotation_z(m.angle)),
            m.half * grow,
            color,
        );
    }
}

/// The flash overlay and the three-slot meter. Both read the gate, neither decides anything.
fn meter(
    tick: Res<Tick>,
    flash: Res<Flash>,
    gate: Res<FlashGate>,
    policy: Res<GorePolicy>,
    settings: Res<CarnageSettings>,
    mut overlay: Query<&mut BackgroundColor, (With<FlashOverlay>, Without<FlashSlot>)>,
    mut slots: Query<(&mut BackgroundColor, &FlashSlot), Without<FlashOverlay>>,
) {
    let color = if flash.lit_ticks > 0 {
        blood_color(&policy, &settings, 0.32)
    } else {
        Color::NONE
    };
    for mut bg in &mut overlay {
        bg.0 = color;
    }
    let recent = gate.recent(tick.0, HZ);
    for (mut bg, slot) in &mut slots {
        bg.0 = if slot.0 < recent {
            blood_color(&policy, &settings, 0.9)
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.08)
        };
    }
}

/// The exclusion cone, sized by [`exclusion_ndc`]. NDC spans 2 across the viewport, so a length `L`
/// in NDC is `L * 50` percent of it.
fn aim_ring(show: Res<Show>, policy: Res<GorePolicy>, mut ring: Query<&mut Node, With<AimRing>>) {
    let r = exclusion_ndc(&policy);
    for mut node in &mut ring {
        node.display = if show.aim && r > 0.0 { Display::Flex } else { Display::None };
        node.width = percent(r * 100.0);
        node.height = percent(r * 100.0);
        node.left = percent(50.0 - r * 50.0);
        node.top = percent(50.0 - r * 50.0);
    }
}

/// Screen blood, and the refusals. **`occludes_aim` is the only thing that decides** — this system
/// shows what it admitted and counts what it refused.
fn screen_blood(
    mut kill: ResMut<Kill>,
    policy: Res<GorePolicy>,
    settings: Res<CarnageSettings>,
    mut nodes: Query<(&mut Node, &mut BackgroundColor, &ScreenSplatNode)>,
) {
    let active = kill.age.is_some();
    let mut refused = 0u32;
    for (mut node, mut bg, which) in &mut nodes {
        let Some(splat) = kill.splats.get(which.0) else {
            node.display = Display::None;
            continue;
        };
        if !active || !policy.screen_blood {
            node.display = Display::None;
            continue;
        }
        if occludes_aim(splat.ndc, splat.radius_ndc, &policy) {
            refused = refused.wrapping_add(1);
            node.display = Display::None;
            continue;
        }
        node.display = Display::Flex;
        node.width = percent(splat.radius_ndc * 100.0);
        node.height = percent(splat.radius_ndc * 100.0);
        node.left = percent((splat.ndc.x - splat.radius_ndc + 1.0) * 50.0);
        node.top = percent((1.0 - splat.ndc.y - splat.radius_ndc) * 50.0);
        bg.0 = blood_color(&policy, &settings, 0.8);
    }
    if kill.refused != refused {
        kill.refused = refused;
    }
}

/// Every claim on screen, measured rather than described.
fn hud(
    tick: Res<Tick>,
    flash: Res<Flash>,
    gate: Res<FlashGate>,
    kill: Res<Kill>,
    policy: Res<GorePolicy>,
    settings: Res<CarnageSettings>,
    mut line: Query<&mut Text, With<HudStatus>>,
) {
    let on = |b: bool| if b { "ON " } else { "off" };
    let r = exclusion_ndc(&policy);
    let text = format!(
        "tier {}\n\
         palette {}   dismemberment {}   viscera {}   screen blood {}   ragdolls {}\n\
         persistence {:.2} (stain_lifetime {} ticks -> {:.0}; drawn over a {}-tick reference so the \
         dial is visible)\n\
         intensity {:.2}, wetness {:.2} - shown, NOT applied: the claim is that the emitter is \
         unchanged at every tier\n\
         flashes in the last second {} of {}   attempts {}   admitted {}   REFUSED {}   (WCAG 2.1 \
         SC 2.3.1 G19)\n\
         aim exclusion {:.1} deg -> {:.3} NDC   screen marks refused {} of {}   {} stains, same \
         places at every tier",
        tier_name(policy.tier),
        if policy.draws_blood() { "BLOOD    " } else { "SUBSTITUTE (spark)" },
        on(policy.dismemberment),
        on(policy.viscera),
        on(policy.screen_blood),
        on(policy.ragdolls),
        policy.persistence_scale,
        settings.stain_lifetime_ticks,
        settings.stain_lifetime_ticks as f32 * policy.persistence_scale,
        PERSIST_REF_TICKS,
        policy.intensity,
        policy.wetness,
        gate.recent(tick.0, HZ),
        policy.max_flashes_per_second.min(WCAG_FLASHES_PER_SECOND),
        flash.attempts,
        flash.admitted,
        flash.refused,
        policy.aim_exclusion_deg,
        r,
        kill.refused,
        kill.splats.len(),
        kill.marks.len(),
    );
    for mut t in &mut line {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}
