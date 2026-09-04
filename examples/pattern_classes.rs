//! **Six bloodstain classes, as six mechanisms.**
//!
//! ```text
//!   1  impact spatter      4  expirated
//!   2  arterial            5  drip trail   (press again to walk faster)
//!   3  cast-off            6  transfer
//!   C  clear
//! ```
//!
//! # Why six generators and not one cone at six intensities
//!
//! The SWGSTAIN / ASB TR-033 taxonomy is a classification of *mechanisms* — an analyst reads a scene
//! backwards from stain morphology to whatever produced it — so a game that throws one isotropic cone
//! for every event has one pattern at six volumes, and a player reads it as one thing happening
//! repeatedly. Each key here drives a different `bevy_carnage::bloodstain::patterns` generator, and each one is
//! visibly its own thing:
//!
//! - **`2` fires once per systole**, not continuously: `arterial_arc` returns nothing between beats,
//!   and its reach falls linearly to zero over `pressure_decay_ticks` — exsanguination, in one number.
//!   The readout prints the pressure and counts the systoles.
//! - **`3` is tangential, and that is the measured fact rather than the folklore** (Williams et al.,
//!   `doi:10.1111/1556-4029.13855`): the drop leaves along the tip's instantaneous velocity, never
//!   radially outward. Watch the swing — the fast stroke sheds, the slow return does not, because
//!   `cast_off` refuses below `CAST_OFF_MIN_V`. Droplet diameter is *inversely* proportional to tip
//!   speed, and Adam (`doi:10.1016/j.forsciint.2019.109934`) caps the pendant volume at 150 µL, so a
//!   weapon swung repeatedly sheds less each swing.
//! - **`4` usually shows no bubble rings**, deliberately: Donaldson et al.
//!   (`doi:10.1007/s00414-010-0498-5`) find them in only ~20 % of expirated patterns and only in
//!   stains above 3 mm, so the ring count on screen is usually `0`.
//! - **`5`'s spacing encodes speed.** Press `5` again to walk faster; the drips separate by
//!   `drip_spacing_ref · speed`, which is the number an analyst reads a stride off.
//! - **`6` spends a conserved budget.** A dragged body runs out of blood: the smear holds its width
//!   while blood remains, thins on the last contact, and then the drag leaves nothing at all.
//!
//! **The three spending classes share one counter, and it is the point.** `cast_off`, `drip_trail` and
//! `transfer` all take `load_ml: &mut f32` and decrement it, so the pattern ends because the blood ran
//! out rather than because a lifetime expired. The readout shows it draining.
//!
//! # Where the blood is drawn, and what it is drawn with
//!
//! Two surfaces: the floor, and a wall behind the subject. A droplet's landing point on the **floor**
//! is `bevy_carnage::bloodstain::landing`'s own closed form and its impact is `impact_at_plane`'s; the **wall** is a
//! vertical plane, which that pair does not cover, so the crossing is solved here — the same ballistic
//! parabola, read against `z` instead of `y`. Because `z(t)` is linear and monotone the crossing is
//! unique, so "the wall crossing happens above the floor" *is* the test for which surface came first,
//! and no second landing model is needed to decide it.
//!
//! Stains are meshes, not decals: `spawn_stain` lives behind the `vfx` feature and the web build does
//! not have it. Each is an ellipse whose aspect is the impact's own `minor/major` — the `sin θ`
//! relation, visible — at the placement scale `stain_radius` returns.
//!
//! Run: `cargo run --release -p bevy_carnage --example pattern_classes`

use std::collections::VecDeque;
use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;

use bevy_carnage::blood::dry::appearance;
use bevy_carnage::blood::stain::{Impact, impact_at_plane, stain_radius, stain_shape};
use bevy_carnage::blood::{self, Bleed, BloodSettings, Droplet, PatternClass, WoundKind, patterns};

/// The fixed-tick rate every duration in `BloodSettings` is quoted at.
const HZ: u32 = 60;
/// The floor, metres from the centre to an edge.
const FLOOR_HALF: f32 = 3.0;
/// The wall's plane, its half-width and its top — a droplet crossing `z = WALL_Z` inside those bounds
/// stains the wall instead of the floor.
const WALL_Z: f32 = -1.0;
const WALL_HALF_X: f32 = 1.8;
const WALL_TOP_Y: f32 = 2.4;

/// How much surface the impact and arterial wounds have come open by, m². Droplet count is
/// `area · droplets_per_m2 · severity`, so this is the dial that decides how much blood a burst throws.
const WOUND_AREA: f32 = 0.05;

/// The impact wound, and the way it faces: up and back, so the fast half of the cone reaches the wall
/// and the slow half falls on the floor.
const IMPACT_AT: Vec3 = Vec3::new(0.15, 1.15, -0.25);
const IMPACT_NORMAL: Vec3 = Vec3::new(0.05, 0.72, -0.69);
/// The arterial wound: a neck, facing the wall.
const ARTERIAL_AT: Vec3 = Vec3::new(0.0, 1.45, -0.30);
const ARTERIAL_NORMAL: Vec3 = Vec3::new(0.0, 0.36, -0.93);
/// The mouth, and the direction of the breath that carries the mist.
const BREATH_AT: Vec3 = Vec3::new(-0.20, 1.50, -0.10);
const BREATH_DIR: Vec3 = Vec3::new(-0.08, -0.18, -0.98);
/// Millilitres of blood in the breath, and the air impulse behind it, m/s.
const BREATH_ML: f32 = 2.0;
const BREATH_IMPULSE: f32 = 3.0;

/// The swing: where it pivots, how long the arm is, and the arc it sweeps, degrees.
const PIVOT: Vec3 = Vec3::new(0.30, 1.55, 0.10);
const ARM: f32 = 0.50;
const SWING_FROM_DEG: f32 = 100.0;
const SWING_TO_DEG: f32 = -30.0;
/// Ticks of fast stroke, then the whole cycle. The return stroke covers the same arc over the rest of
/// the cycle, which puts it below `CAST_OFF_MIN_V` — so it sheds nothing, and the model is what says so.
const SWING_TICKS: u32 = 12;
const SWING_PERIOD: u32 = 78;

/// Where the walker's floor track runs, and how far along it goes.
const WALK_FROM: Vec3 = Vec3::new(-1.7, 0.0, 0.85);
const WALK_TO: Vec3 = Vec3::new(1.7, 0.0, 1.15);
/// Walking speeds `5` cycles, m/s: a walk, a jog, a run.
const WALK_SPEEDS: [f32; 3] = [0.6, 1.4, 2.8];

/// The drag: where the body is pulled from and to, and how fast, m/s.
const DRAG_FROM: Vec3 = Vec3::new(-1.6, 0.0, 0.30);
const DRAG_TO: Vec3 = Vec3::new(1.5, 0.0, -0.45);
const DRAG_SPEED: f32 = 0.30;

/// Millilitres a swung weapon carries, a bleeding walker carries, and a dragged body carries.
///
/// Three numbers rather than one, because they are three different reservoirs — and each is sized so
/// the counter empties while you are watching it, which is the whole demonstration.
const CAST_OFF_LOAD_ML: f32 = 4.0;
const WALK_LOAD_ML: f32 = 1.5;
const DRAG_LOAD_ML: f32 = 6.0;

/// Live stains. Past it the oldest is despawned, in spawn order, which is total by construction.
const MAX_STAINS: usize = 4000;

/// The key legend, and it says the same thing as `web/play.html`'s `notes-pattern_classes` block.
///
/// ASCII only: Bevy 0.19's default font carries 95 codepoints, so a mid-dot or an arrow draws as
/// nothing at all.
const LEGEND: &str = "1 impact spatter    2 arterial    3 cast-off    4 expirated    \
                      5 drip trail    6 transfer    C clear";

/// The line reporting the active class, its mechanism and its budget.
#[derive(Component)]
struct Readout;

/// **The scene's blood state**: one tick counter, one active class, and each mechanism's own state.
#[derive(Resource)]
struct Blood {
    settings: BloodSettings,
    tick: u32,
    active: Option<PatternClass>,
    /// The tick the current class was armed on. Every mechanism's phase is measured from it, so the
    /// swing, the walk and the drag all start at zero rather than wherever the global tick happened
    /// to be.
    armed: u32,
    /// A one-shot class fires on the next fixed tick and clears this; a continuous one ignores it.
    pending: bool,
    /// Arterial: this wound's own clock, so its pressure envelope is measured from when *it* opened.
    bleed: Bleed,
    systoles: u32,
    /// The conserved budget the three spending classes draw down, millilitres.
    load: f32,
    load_start: f32,
    /// Cast-off: where the tip was last tick, and which swing this is.
    tip: Vec3,
    swings: u32,
    /// Drip trail: which speed, how far the walker has gone, and where the last drip fell.
    walk: usize,
    walked: f32,
    dripped: f32,
    /// Transfer: how far the drag has gone and how many contacts moved blood.
    dragged: f32,
    contacts: u32,
    /// The last burst: how many droplets left, how many landed in view, and the bubble-ring count.
    emitted: usize,
    landed: usize,
    rings: u8,
}

/// What a stain is drawn with, and every stain drawn so far.
#[derive(Resource)]
struct Paint {
    disc: Handle<Mesh>,
    blood: Handle<StandardMaterial>,
    stains: VecDeque<Entity>,
    spawned: usize,
}

/// One stain, placed: where, facing which way, and how long by how wide.
struct Hit {
    at: Vec3,
    normal: Vec3,
    rotation: Quat,
    long: f32,
    short: f32,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "pattern_classes".into(),
                canvas: Some("#carnage-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        // The blood dials are authored in ticks at 60 Hz, so the demo runs its mechanisms on a 60 Hz
        // fixed tick and counts integers. No clock is read anywhere in here.
        .insert_resource(Time::<Fixed>::from_hz(f64::from(HZ)))
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, advance)
        .add_systems(Update, (arm, markers, hud))
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut settings = BloodSettings::default();
    // **The one dial this demo authors, and the arithmetic is why.** `spatter_speed_scale = 1.0` is
    // the spatter paper's own 8-40 m/s, and the crate's own note says what that means at human
    // scale: a droplet leaving straight up at 40 m/s under the shipped 18 m/s^2 gravity rises
    // `40^2 / (2*18)` = 44 metres and comes down twenty metres away, off any room. The reference
    // demos use 0.25, which puts the throw at 1-3 m — but at 0.25 an arterial jet leaves at 2 m/s
    // and carries `2^2 / 18` = 0.22 m, so it would never reach the wall 0.7 m away and the spurt
    // this demo exists to show would land at the victim's feet. 0.5 is the value that keeps both
    // honest: a 4 m/s jet carries 0.89 m and paints the wall, and the spatter cone stays in the room.
    settings.spatter_speed_scale = 0.5;
    // **Fresh blood's colour and gloss come from the drying model at age zero**, so this demo and
    // `examples/drying` cannot disagree about what wet blood looks like.
    let fresh = appearance(0, HZ, 1.0e-3, &settings);
    let blood = materials.add(StandardMaterial {
        base_color: Color::srgb(fresh.srgb[0], fresh.srgb[1], fresh.srgb[2]),
        perceptual_roughness: fresh.roughness,
        ..default()
    });
    // A disc in its own XY plane, normal `+Z`; every stain is this mesh scaled to its own ellipse.
    let disc = meshes.add(Circle::new(0.5).mesh().resolution(20));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(2.25, 1.75, 2.65).looking_at(Vec3::new(0.0, 0.85, -0.30), Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.62, 0.66, 0.78),
        brightness: 700.0,
        ..default()
    });
    commands.spawn((
        DirectionalLight { illuminance: 8_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(3.0, 6.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    let grey = materials.add(StandardMaterial {
        base_color: Color::srgb(0.20, 0.20, 0.22),
        perceptual_roughness: 0.95,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(FLOOR_HALF * 2.0, FLOOR_HALF * 2.0))),
        MeshMaterial3d(grey.clone()),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(WALL_HALF_X * 2.0, WALL_TOP_Y))),
        MeshMaterial3d(grey),
        Transform::from_xyz(0.0, WALL_TOP_Y * 0.5, WALL_Z),
    ));

    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(12.0)),
            row_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|root| {
            root.spawn((
                Text::new(LEGEND),
                TextFont { font_size: FontSize::Px(15.0), ..default() },
                TextColor(Color::srgb(0.98, 0.72, 0.42)),
            ));
            root.spawn((
                Text::new(String::new()),
                TextFont { font_size: FontSize::Px(13.0), ..default() },
                TextColor(Color::srgb(0.88, 0.88, 0.92)),
                Readout,
            ));
        });

    commands.insert_resource(Blood {
        settings,
        tick: 0,
        active: None,
        armed: 0,
        pending: false,
        bleed: Bleed::new(0, &wound_at(ARTERIAL_AT, ARTERIAL_NORMAL)),
        systoles: 0,
        load: 0.0,
        load_start: 0.0,
        tip: PIVOT + swing_offset(0),
        swings: 0,
        walk: 0,
        walked: 0.0,
        dripped: 0.0,
        dragged: 0.0,
        contacts: 0,
        emitted: 0,
        landed: 0,
        rings: 0,
    });
    commands.insert_resource(Paint { disc, blood, stains: VecDeque::new(), spawned: 0 });
}

/// The keys: one class each, `C` to clear. Pressing `5` again walks faster.
fn arm(
    keys: Res<ButtonInput<KeyCode>>,
    mut blood: ResMut<Blood>,
    mut paint: ResMut<Paint>,
    mut commands: Commands,
) {
    let picked = if keys.just_pressed(KeyCode::Digit1) {
        Some(PatternClass::Impact)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(PatternClass::ArterialSpurt)
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(PatternClass::CastOff)
    } else if keys.just_pressed(KeyCode::Digit4) {
        Some(PatternClass::Expirated)
    } else if keys.just_pressed(KeyCode::Digit5) {
        Some(PatternClass::DripTrail)
    } else if keys.just_pressed(KeyCode::Digit6) {
        Some(PatternClass::Transfer)
    } else {
        None
    };

    if keys.just_pressed(KeyCode::KeyC) {
        blood.active = None;
        for entity in paint.stains.drain(..) {
            commands.entity(entity).despawn();
        }
    }

    let Some(class) = picked else {
        return;
    };
    // Pressing the drip trail again steps the walking speed, which is the one dial the page promises
    // and the only key that means something different the second time.
    if class == PatternClass::DripTrail && blood.active == Some(PatternClass::DripTrail) {
        blood.walk = (blood.walk + 1) % WALK_SPEEDS.len();
    }
    let tick = blood.tick;
    blood.active = Some(class);
    blood.armed = tick;
    blood.pending = true;
    // The arterial wound is the only one this bleed schedules, so it is the one the phase and the
    // area come from. See `Bleed::seed`.
    blood.bleed = Bleed::new(tick, &wound_at(ARTERIAL_AT, ARTERIAL_NORMAL));
    blood.systoles = 0;
    blood.swings = 0;
    blood.tip = PIVOT + swing_offset(0);
    blood.walked = 0.0;
    blood.dripped = 0.0;
    blood.dragged = 0.0;
    blood.contacts = 0;
    blood.emitted = 0;
    blood.landed = 0;
    blood.rings = 0;
    blood.load = match class {
        PatternClass::CastOff => CAST_OFF_LOAD_ML,
        PatternClass::DripTrail => WALK_LOAD_ML,
        PatternClass::Transfer => DRAG_LOAD_ML,
        _ => 0.0,
    };
    blood.load_start = blood.load;
}

/// **One fixed tick of whichever mechanism is armed.** Every class reaches its generator from here, so
/// there is one place that paints and one integer clock driving all six.
fn advance(mut blood: ResMut<Blood>, mut paint: ResMut<Paint>, mut commands: Commands) {
    let b = &mut *blood;
    b.tick = b.tick.wrapping_add(1);
    let (Some(class), tick) = (b.active, b.tick) else {
        return;
    };

    match class {
        PatternClass::Impact => {
            if !b.pending {
                return;
            }
            b.pending = false;
            let wound = wound_at(IMPACT_AT, IMPACT_NORMAL);
            let drops = patterns::impact_spatter(&wound, &b.settings);
            b.emitted = drops.len();
            b.landed = paint_drops(&mut commands, &mut paint, &b.settings, IMPACT_AT, &drops);
        }
        PatternClass::ArterialSpurt => {
            let wound = wound_at(ARTERIAL_AT, ARTERIAL_NORMAL);
            // Fires only on a systole tick and only while the blood still flows — `arterial_arc`
            // owns both predicates, so an arterial wound clots by the same mechanism as any other.
            let drops = patterns::arterial_arc(&wound, &b.bleed, tick, HZ, &b.settings);
            if drops.is_empty() {
                return;
            }
            b.systoles += 1;
            b.emitted = drops.len();
            let n = paint_drops(&mut commands, &mut paint, &b.settings, ARTERIAL_AT, &drops);
            b.landed += n;
        }
        PatternClass::CastOff => {
            let phase = tick.wrapping_sub(b.armed) % SWING_PERIOD.max(1);
            if phase == 0 {
                b.swings += 1;
            }
            let previous = b.tip;
            b.tip = PIVOT + swing_offset(phase);
            let drops =
                patterns::cast_off(arr(previous), arr(b.tip), &mut b.load, HZ, &b.settings);
            if drops.is_empty() {
                return;
            }
            b.emitted = drops.len();
            let n = paint_drops(&mut commands, &mut paint, &b.settings, b.tip, &drops);
            b.landed += n;
        }
        PatternClass::Expirated => {
            if !b.pending {
                return;
            }
            b.pending = false;
            let (drops, rings) =
                patterns::expirated(arr(BREATH_DIR), BREATH_ML, BREATH_IMPULSE, tick, &b.settings);
            b.emitted = drops.len();
            b.rings = rings;
            b.landed = paint_drops(&mut commands, &mut paint, &b.settings, BREATH_AT, &drops);
        }
        PatternClass::DripTrail => {
            let speed = WALK_SPEEDS.get(b.walk).copied().unwrap_or(1.0);
            let length = (WALK_TO - WALK_FROM).length();
            if b.walked >= length {
                return;
            }
            b.walked = (b.walked + speed / HZ as f32).min(length);
            // The segment since the LAST drip, not since last tick: a per-tick step is far shorter
            // than one drip spacing, and `drip_trail` places drips per spacing over the segment it is
            // given. So the function decides when a drip falls, from the speed, exactly as intended.
            let from = along(WALK_FROM, WALK_TO, b.dripped);
            let to = along(WALK_FROM, WALK_TO, b.walked);
            let stains = patterns::drip_trail(arr(from), arr(to), speed, &mut b.load, &b.settings);
            if stains.is_empty() {
                return;
            }
            b.dripped = b.walked;
            b.emitted = stains.len();
            for stain in &stains {
                // A free-falling drip arrives perpendicular, so its stain is a disc — the aspect
                // relation's own boundary case, and no direction to fabricate.
                let at = Vec3::new(stain.at[0], 0.0, stain.at[2]);
                paint_flat(&mut commands, &mut paint, at, stain.radius);
                b.landed += 1;
            }
        }
        PatternClass::Transfer => {
            let length = (DRAG_TO - DRAG_FROM).length();
            if b.dragged >= length {
                return;
            }
            let previous = along(DRAG_FROM, DRAG_TO, b.dragged);
            b.dragged = (b.dragged + DRAG_SPEED / HZ as f32).min(length);
            let contact = along(DRAG_FROM, DRAG_TO, b.dragged);
            let tangent = (contact - previous).normalize_or_zero();
            // `None` once the load is gone: "nothing was transferred" is a different fact from "a
            // tiny amount was", and this is where the drag stops leaving marks.
            let Some(stain) =
                patterns::transfer(arr(contact), arr(tangent), &mut b.load, &b.settings)
            else {
                return;
            };
            b.contacts += 1;
            let at = Vec3::new(stain.at[0], 0.0, stain.at[2]);
            paint_flat(&mut commands, &mut paint, at, stain.radius);
            b.landed += 1;
        }
    }
}

/// Draw the moving parts, so a mechanism is visible and not just its output.
fn markers(blood: Res<Blood>, mut gizmos: Gizmos) {
    let Some(class) = blood.active else {
        return;
    };
    let wound = Color::srgb(0.95, 0.35, 0.35);
    let rig = Color::srgb(0.45, 0.85, 0.98);
    match class {
        PatternClass::Impact => cross(&mut gizmos, IMPACT_AT, 0.07, wound),
        PatternClass::ArterialSpurt => cross(&mut gizmos, ARTERIAL_AT, 0.07, wound),
        PatternClass::CastOff => {
            // The arc the tip travels, then the arm on it. The stroke sheds along this path; the slow
            // return over the same path sheds nothing.
            gizmos.linestrip(
                (0..=24).map(|i| PIVOT + swing_offset(i * SWING_TICKS.max(1) / 24)),
                rig.with_alpha(0.35),
            );
            gizmos.line(PIVOT, blood.tip, rig);
            cross(&mut gizmos, blood.tip, 0.05, wound);
        }
        PatternClass::Expirated => {
            cross(&mut gizmos, BREATH_AT, 0.06, wound);
            gizmos.line(BREATH_AT, BREATH_AT + BREATH_DIR.normalize_or_zero() * 0.35, rig);
        }
        PatternClass::DripTrail => {
            let at = along(WALK_FROM, WALK_TO, blood.walked);
            gizmos.line(WALK_FROM, WALK_TO, rig.with_alpha(0.3));
            gizmos.line(at, at + Vec3::Y * 0.95, rig);
            cross(&mut gizmos, at + Vec3::Y * 0.95, 0.06, wound);
        }
        PatternClass::Transfer => {
            let at = along(DRAG_FROM, DRAG_TO, blood.dragged);
            gizmos.line(DRAG_FROM, DRAG_TO, rig.with_alpha(0.3));
            // The dragged body's footprint, so "the smear stopped but the drag did not" is visible.
            gizmos.lineloop(
                [
                    at + Vec3::new(-0.22, 0.01, -0.12),
                    at + Vec3::new(0.22, 0.01, -0.12),
                    at + Vec3::new(0.22, 0.01, 0.12),
                    at + Vec3::new(-0.22, 0.01, 0.12),
                ],
                rig,
            );
        }
    }
}

/// The numbers the claim rests on: the mechanism, its clock, and the budget it is spending.
fn hud(blood: Res<Blood>, paint: Res<Paint>, mut readout: Query<&mut Text, With<Readout>>) {
    let s = &blood.settings;
    let live = paint.stains.len();
    let text = match blood.active {
        None => "press 1-6 to fire a class; C clears the scene\n\
                 each key is a different mechanism, not one cone at six volumes"
            .to_string(),
        Some(PatternClass::Impact) => format!(
            "impact spatter: the percolation cone\n\
             {} droplets thrown, {} landed in view, {live} stains live\n\
             droplet size is inversely correlated with speed, which is the model, not a look",
            blood.emitted, blood.landed
        ),
        Some(PatternClass::ArterialSpurt) => {
            let age = blood.bleed.age(blood.tick);
            let pressure =
                (1.0 - age as f32 / s.pressure_decay_ticks.max(1) as f32).clamp(0.0, 1.0);
            format!(
                "arterial: ONE arc per systole, at {:.0} bpm\n\
                 age {age} ticks   pressure {:.0} %   systoles {}   last arc {} droplets   \
                 {live} stains live\n\
                 reach falls to zero over pressure_decay_ticks = {}; the arc thins as the body empties",
                s.spurt_bpm,
                pressure * 100.0,
                blood.systoles,
                blood.emitted,
                s.pressure_decay_ticks
            )
        }
        Some(PatternClass::CastOff) => format!(
            "cast-off: tangential to the tip's path (Williams 10.1111/1556-4029.13855)\n\
             load {:.2} ml of {:.2}   swing {}   last shed {} droplets   {live} stains live\n\
             pendant volume capped at {:.2} ml (Adam 2019); the slow return is below \
             CAST_OFF_MIN_V and sheds nothing",
            blood.load, blood.load_start, blood.swings, blood.emitted, s.cast_off_max_ml
        ),
        Some(PatternClass::Expirated) => format!(
            "expirated: a fine mist, and a bubble-ring count that is usually zero\n\
             {} droplets, {} landed in view   bubble rings {}   {live} stains live\n\
             rings occur in only {:.0} % of patterns and only above {:.0} mm \
             (Donaldson 10.1007/s00414-010-0498-5)",
            blood.emitted,
            blood.landed,
            blood.rings,
            s.expirated_ring_fraction * 100.0,
            s.expirated_ring_min_mm
        ),
        Some(PatternClass::DripTrail) => {
            let speed = WALK_SPEEDS.get(blood.walk).copied().unwrap_or(1.0);
            format!(
                "drip trail: spacing encodes speed. press 5 again to walk faster\n\
                 speed {speed:.1} m/s   spacing {:.2} m   load {:.2} ml of {:.2}   \
                 {} drips   {live} stains live\n\
                 spacing is drip_spacing_ref {:.2} m per m/s; each drip costs blood, so the trail \
                 ends rather than continuing",
                s.drip_spacing_ref * speed,
                blood.load,
                blood.load_start,
                blood.landed,
                s.drip_spacing_ref
            )
        }
        Some(PatternClass::Transfer) => format!(
            "transfer: a dragged body runs out of blood\n\
             load {:.2} ml of {:.2}   contacts {}   dragged {:.2} m of {:.2}   {live} stains live\n\
             moved = min(load, transfer_rate {:.2} ml) per contact, so the smear holds its width \
             while blood remains, thins on the last contact, then the drag leaves nothing",
            blood.load,
            blood.load_start,
            blood.contacts,
            blood.dragged,
            (DRAG_TO - DRAG_FROM).length(),
            s.transfer_rate
        ),
    };
    for mut line in &mut readout {
        line.0 = text.clone();
    }
}

/// A wound as `bloodstain` spells it. This demo's `[f32; 3]` boundary, crossed once per wound.
fn wound_at(at: Vec3, normal: Vec3) -> blood::Wound {
    blood::Wound {
        at: arr(at),
        normal: arr(normal.normalize_or_zero()),
        area: WOUND_AREA,
        severity: 1.0,
        kind: WoundKind::Severance,
    }
}

/// `glam` into `bloodstain`.
fn arr(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

/// A point `distance` metres along a segment.
fn along(from: Vec3, to: Vec3, distance: f32) -> Vec3 {
    from + (to - from).normalize_or_zero() * distance
}

/// Where the tip is at `phase` ticks into the swing cycle.
///
/// The stroke covers the arc in [`SWING_TICKS`]; the return covers the same arc over the rest of the
/// period, which is slow enough that [`patterns::cast_off`] refuses it. Two speeds on one path, so the
/// asymmetry a photograph shows is a property of the swing rather than of a flag.
fn swing_offset(phase: u32) -> Vec3 {
    let stroke = SWING_TICKS.max(1);
    let t = if phase < stroke {
        phase as f32 / stroke as f32
    } else {
        let rest = SWING_PERIOD.max(stroke + 1) - stroke;
        1.0 - (phase - stroke) as f32 / rest as f32
    };
    let angle = (SWING_FROM_DEG + (SWING_TO_DEG - SWING_FROM_DEG) * t.clamp(0.0, 1.0)).to_radians();
    // The swing plane contains `z`, so the stroke ends pointing at the wall.
    Vec3::new(0.0, angle.cos(), angle.sin()) * ARM
}

/// Every droplet of a burst, painted where it actually reached. Returns how many landed in view.
fn paint_drops(
    commands: &mut Commands,
    paint: &mut Paint,
    s: &BloodSettings,
    from: Vec3,
    drops: &[Droplet],
) -> usize {
    let mut landed = 0;
    for (index, drop) in drops.iter().enumerate() {
        let Some(hit) = first_hit(from, drop, s, drop_seed(from, index)) else {
            continue;
        };
        spawn_stain(commands, paint, &hit);
        landed += 1;
    }
    landed
}

/// A per-stain seed from a **place and an ordinal**, quantised on `WELD` before hashing.
///
/// The rule `bevy_carnage::bloodstain::wound_seed` uses, and for the same reason: two runs that place a burst a
/// float ULP apart must seed identically, so no jitter here comes from an accumulator or an entity id.
fn drop_seed(from: Vec3, index: usize) -> u32 {
    let q = |x: f32| (x / blood::WELD).round() as i64 as u32;
    q(from.x)
        ^ q(from.y).wrapping_mul(0x9E37_79B9)
        ^ q(from.z).wrapping_mul(2_654_435_761)
        ^ (index as u32).wrapping_mul(0x85EB_CA6B)
}

/// **Which surface a droplet reached, and the stain it left there.**
///
/// The wall first: `z(t)` is linear and monotone, so a droplet has exactly one wall-plane crossing,
/// and that crossing landing above the floor and inside the wall's bounds *is* the test for "the wall
/// came first". Otherwise the floor, through `bevy_carnage::bloodstain::landing` and `impact_at_plane` — the crate's
/// own closed form, unduplicated.
fn first_hit(from: Vec3, drop: &Droplet, s: &BloodSettings, seed: u32) -> Option<Hit> {
    let velocity = Vec3::new(drop.dir[0], drop.dir[1], drop.dir[2]) * drop.speed;
    if let Some((at, arrival)) = wall_crossing(from, velocity, s.gravity) {
        // The wall's own frame: `z` is through the surface, `(x, y)` lie in it.
        let through = arrival.z.abs();
        let in_plane = Vec2::new(arrival.x, arrival.y);
        let impact = Impact {
            speed: arrival.length(),
            diameter: drop.diameter,
            angle_rad: if in_plane.length() > 0.0 {
                through.atan2(in_plane.length())
            } else {
                FRAC_PI_2
            },
            roughness: s.substrate_roughness,
            travel: in_plane.normalize_or_zero().to_array(),
        };
        let shape = stain_shape(&impact, s, seed);
        let radius = stain_radius(drop, impact.speed, s);
        let aspect = if shape.major > 0.0 { shape.minor / shape.major } else { 1.0 };
        let facing = shape.direction;
        return Some(Hit {
            at,
            normal: Vec3::Z,
            // The disc's own normal is already `+Z`, so the whole rotation is the in-plane one.
            rotation: Quat::from_rotation_z(facing[1].atan2(facing[0])),
            long: radius * 2.0,
            short: radius * 2.0 * aspect,
        });
    }

    let landed = blood::landing(arr(from), drop, s.gravity, 0.0)?;
    let at = Vec3::new(landed[0], 0.0, landed[2]);
    if at.x.abs() > FLOOR_HALF || at.z.abs() > FLOOR_HALF {
        return None;
    }
    let impact = impact_at_plane(drop, arr(from), 0.0, s);
    let shape = stain_shape(&impact, s, seed);
    let radius = stain_radius(drop, impact.speed, s);
    let aspect = if shape.major > 0.0 { shape.minor / shape.major } else { 1.0 };
    let facing = shape.direction;
    Some(Hit {
        at,
        normal: Vec3::Y,
        // Lay the disc down, then turn it so its long axis runs along the droplet's own track.
        rotation: Quat::from_rotation_y((-facing[1]).atan2(facing[0]))
            * Quat::from_rotation_x(-FRAC_PI_2),
        long: radius * 2.0,
        short: radius * 2.0 * aspect,
    })
}

/// Where a droplet crosses the wall plane, and how fast it is going when it does.
///
/// `None` when it is heading away from the wall, starts behind it, or crosses outside the wall's
/// bounds or below the floor — each of those means the wall was not what it hit.
fn wall_crossing(from: Vec3, velocity: Vec3, gravity: f32) -> Option<(Vec3, Vec3)> {
    if velocity.z >= 0.0 || from.z <= WALL_Z {
        return None;
    }
    let t = (WALL_Z - from.z) / velocity.z;
    if !(t > 0.0) || !t.is_finite() {
        return None;
    }
    let y = from.y + velocity.y * t - 0.5 * gravity * t * t;
    let x = from.x + velocity.x * t;
    if y < 0.0 || y > WALL_TOP_Y || x.abs() > WALL_HALF_X {
        return None;
    }
    Some((Vec3::new(x, y, WALL_Z), Vec3::new(velocity.x, velocity.y - gravity * t, velocity.z)))
}

/// A disc on the floor, at the placement radius the model returned.
fn paint_flat(commands: &mut Commands, paint: &mut Paint, at: Vec3, radius: f32) {
    let hit = Hit {
        at,
        normal: Vec3::Y,
        rotation: Quat::from_rotation_x(-FRAC_PI_2),
        long: radius * 2.0,
        short: radius * 2.0,
    };
    spawn_stain(commands, paint, &hit);
}

/// One stain entity, and the oldest one retired if the scene is at its ceiling.
fn spawn_stain(commands: &mut Commands, paint: &mut Paint, hit: &Hit) {
    // Coplanar quads z-fight. Each stain sits a fraction of a millimetre further off its surface than
    // the last, cycling, so neighbours never share a plane and the offset never grows visible.
    let lift = 0.002 + (paint.spawned % 32) as f32 * 0.0003;
    let entity = commands
        .spawn((
            Mesh3d(paint.disc.clone()),
            MeshMaterial3d(paint.blood.clone()),
            Transform {
                translation: hit.at + hit.normal * lift,
                rotation: hit.rotation,
                scale: Vec3::new(hit.long.max(1.0e-4), hit.short.max(1.0e-4), 1.0),
            },
        ))
        .id();
    paint.spawned = paint.spawned.wrapping_add(1);
    paint.stains.push_back(entity);
    while paint.stains.len() > MAX_STAINS {
        let Some(oldest) = paint.stains.pop_front() else {
            break;
        };
        commands.entity(oldest).despawn();
    }
}

/// Three lines through a point — a marker drawn with the one gizmo call that returns nothing to
/// ignore.
fn cross(gizmos: &mut Gizmos, at: Vec3, radius: f32, color: Color) {
    gizmos.line(at - Vec3::X * radius, at + Vec3::X * radius, color);
    gizmos.line(at - Vec3::Y * radius, at + Vec3::Y * radius, color);
    gizmos.line(at - Vec3::Z * radius, at + Vec3::Z * radius, color);
}
