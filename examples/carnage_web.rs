//! **The flagship. One body, one shot, and all four crates in one frame.**
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example carnage_web
//! ```
//!
//! Every other demo on the site shows one idea in isolation, which is the right way to argue for
//! each of them and the wrong way to show what the framework *is*. This one fires a single round and
//! lets the whole chain run:
//!
//! 1. **The bone breaks the way that blow loaded it** — `bevy_carnage`'s `FaultPolicy::Morphology`,
//!    with the loading mode taken from where the shot landed rather than from a dial.
//! 2. **The blood pattern that fits the wound is thrown** — `bloodstain`'s percolation spatter for a
//!    hit, its arterial arc for a breached vessel, one per systole.
//! 3. **The wetmap it lands on records it in UV space** — `bevy_wetmap`, CPU-authoritative, with the
//!    digest on screen because it can have one.
//! 4. **The guts it opened spill and tether** — `bevy_viscera`'s XPBD strands and their mesenteric
//!    membrane, which tears if you pull.
//! 5. **The drying it leaves walks the colour and the gloss** — `bloodstain::dry`, over the next half
//!    minute of game time.
//!
//! # Why this demo can only be built from the monorepo
//!
//! It names four crates. Each of those crates has its own public mirror, and a mirror contains that
//! crate alone — so there is exactly one tree in which this file compiles, and it is the one that
//! owns all four. That is the reason `scripts/build_web.sh` runs here rather than in
//! `Ladvien/bevy_carnage`.
//!
//! # No particles, and that is the wasm build's rule rather than this demo's preference
//!
//! `scripts/build_web.sh` builds with `--no-default-features --features serde`, because
//! `bevy_hanabi`'s wasm support is WebGPU-compute-only. So every visual here is a mesh or a gizmo,
//! drawn from the CPU-side model — which is what all the new work in this framework is anyway.

use bevy::prelude::*;
use bevy_carnage::{
    BloodSettings, Bore, CarnageSettings, CutSettings, FaultPolicy, GorePolicy, GoreTier,
    LoadingMode, ProxyCell, TissueClass, blood, fracture_mesh,
};
use bevy_viscera::{Mesentery, Strand, ViscSettings, step, tube_mesh};
use bevy_wetmap::{WetCanvas, WetSettings};

mod common;

/// The fixed tick rate every integer clock in the three crates is quoted against.
const HZ: u32 = 60;

/// Where the subject stands. The blockout's lowest point is a leg bottom at `y = -0.92`.
const ORIGIN: Vec3 = Vec3::new(0.0, 0.92, 0.0);

/// Fragments the bake cuts down to. Enough that a limb can come off without the body becoming gibs.
const TARGET: usize = 20;

/// The seed. One seed, so a replay is the same kill.
const SEED: u32 = 0x00C0_FFEE;

/// Half-extent of the wetmap-carrying floor slab, metres. The canvas maps one UV unit to
/// `bevy_wetmap::UV_SPAN_M`, so this is also how much floor one canvas covers.
const FLOOR_HALF: f32 = 1.6;

/// Canvas edge in texels. 128 is 64 KB per upload at `Rgba8UnormSrgb`; `bevy_wetmap` names 256 as
/// the practical ceiling and says why.
const CANVAS: u32 = 128;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// **Where a shot landed, and therefore how the bone was loaded.**
///
/// The mapping is anatomy rather than a dial: a round through a limb's long axis twists it, one
/// across a limb bends it, one down the shaft loads it axially, and a centre-mass hit at rifle
/// energy comminutes. That is the whole point of `LoadingMode` — the blow decides, not the artist.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Shot {
    /// Shoulder, obliquely: the arm twists off.
    Shoulder,
    /// Across the thigh: a bend, and a butterfly wedge.
    Thigh,
    /// Down through the torso: axial.
    Torso,
    /// Centre mass at rifle energy: comminution.
    Rifle,
}

impl Shot {
    fn name(self) -> &'static str {
        match self {
            Shot::Shoulder => "shoulder, oblique",
            Shot::Thigh => "across the thigh",
            Shot::Torso => "down the torso",
            Shot::Rifle => "centre mass, rifle",
        }
    }

    /// Entry point, subject-local.
    fn at(self) -> Vec3 {
        match self {
            Shot::Shoulder => Vec3::new(-0.24, 0.20, 0.06),
            Shot::Thigh => Vec3::new(-0.13, -0.50, 0.06),
            Shot::Torso => Vec3::new(0.04, 0.10, 0.10),
            Shot::Rifle => Vec3::new(0.0, 0.02, 0.12),
        }
    }

    /// The long axis of whatever it hit — what a twist twists about and a bend bends across.
    fn axis(self) -> Vec3 {
        match self {
            // A limb's long axis is vertical in this blockout.
            Shot::Shoulder | Shot::Thigh => Vec3::Y,
            Shot::Torso | Shot::Rifle => Vec3::Y,
        }
    }

    fn mode(self) -> LoadingMode {
        match self {
            Shot::Shoulder => LoadingMode::Torsion,
            Shot::Thigh => LoadingMode::Bending,
            Shot::Torso => LoadingMode::Axial,
            Shot::Rifle => LoadingMode::DirectHighEnergy,
        }
    }

    /// `(torque N·m, impulse N·s, energy J, strain rate 1/s)`.
    fn load(self) -> (f32, f32, f32, f32) {
        match self {
            Shot::Shoulder => (14.0, 40.0, 420.0, 300.0),
            Shot::Thigh => (0.0, 55.0, 500.0, 260.0),
            Shot::Torso => (0.0, 48.0, 460.0, 240.0),
            Shot::Rifle => (0.0, 120.0, 3400.0, 3000.0),
        }
    }

    /// The pattern class this wound throws. A limb through-and-through is impact spatter; a torso hit
    /// opens something that pumps.
    fn class(self) -> blood::PatternClass {
        match self {
            Shot::Shoulder | Shot::Thigh => blood::PatternClass::Impact,
            Shot::Torso | Shot::Rifle => blood::PatternClass::ArterialSpurt,
        }
    }

    /// Does this wound open the abdomen? Only the two torso shots spill.
    fn spills(self) -> bool {
        matches!(self, Shot::Torso | Shot::Rifle)
    }
}

/// Everything the demo holds between ticks. One resource, because one shot drives all of it.
#[derive(Resource)]
struct Flagship {
    /// Integer tick. **Not a clock** — every crate here takes `tick: u32`, and reading
    /// `std::time::Instant` would panic in a browser besides.
    tick: u32,
    /// The tick the current shot was fired on. Every age in the demo is measured from it.
    fired_at: u32,
    shot: Shot,
    tissue: TissueClass,
    policy: GorePolicy,
    settings: CarnageSettings,
    /// Fragments the bake produced, and the residual bend if it was a greenstick.
    fragments: usize,
    bent: Vec3,
    /// The blood in flight: `(position, velocity, born_tick)`, integrated on the fixed tick.
    drops: Vec<(Vec3, Vec3, u32)>,
    /// Stains that landed on the floor, with the age they landed at.
    landed: Vec<(Vec3, f32, u32)>,
    /// Bleed state for the arterial arc.
    bleed: blood::Bleed,
    /// The guts, if this shot opened the abdomen.
    strands: Vec<Strand>,
    mesentery: Vec<Mesentery>,
    visc: ViscSettings,
    wet: WetSettings,
    show_numbers: bool,
}

/// The floor's wetmap canvas, and the mesh the ray is cast against.
#[derive(Component)]
struct WetFloor;

/// One rendered gut, and which strand it draws.
///
/// **The mesh is regenerated every tick** by `bevy_viscera::tube_mesh` and written back into the same
/// handle, which is what the crate is built for: it hands back a `Mesh` and never spawns, so the
/// caller owns both the entity and the asset. An eight-sided tube over 24 nodes is 384 triangles.
#[derive(Component)]
struct Gut(usize);

/// Anything spawned by the current shot, so a reset is one query.
#[derive(Component)]
struct Ephemeral;

/// The camera, marked positively.
///
/// **`With<Camera3d>` is not a filter**: `Single<..>` silently *skips* its system on a non-unique
/// match, so any second camera would disable every system filtering that way. A marker of our own
/// cannot be added to by anything else.
#[derive(Component)]
struct MainCamera;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "carnage_web".into(),
                        // The one web-specific line, and it is inert on native: `Window::canvas` is
                        // documented as having no effect off the web
                        // (`bevy_window-0.19.0/src/window.rs`), so there is no `cfg` here.
                        canvas: Some("#carnage-canvas".into()),
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .insert_resource(Time::<Fixed>::from_hz(HZ as f64))
        .add_systems(Startup, setup)
        .add_systems(Update, (keys, draw_hud_text, orbit).chain())
        .add_systems(FixedUpdate, (advance, paint_and_flush, retube).chain())
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    commands.spawn((
        Camera3d::default(),
        MainCamera,
        AmbientLight { brightness: 240.0, ..default() },
        Transform::from_xyz(2.4, 1.5, 2.8).looking_at(Vec3::new(0.0, 0.85, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 9_000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_xyz(3.0, 6.0, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // **`AmbientLight` is a COMPONENT in Bevy 0.19 and applies per camera**, not a resource. It goes
    // on the camera above; inserting it as a resource does not compile, which is the honest outcome.


    // **The floor is the wetmap.** One canvas, one slab, and the mesh is kept so `paint_world` can
    // cast against the same triangles the player sees.
    let canvas = WetCanvas::new(&mut images, CANVAS, [0.20, 0.19, 0.18], 0.85);
    let floor_mesh = meshes.add(
        Plane3d::default().mesh().size(FLOOR_HALF * 2.0, FLOOR_HALF * 2.0).build(),
    );
    let floor_material = materials.add(StandardMaterial {
        base_color_texture: Some(canvas.albedo()),
        metallic_roughness_texture: Some(canvas.roughness()),
        perceptual_roughness: 1.0,
        ..default()
    });
    commands.spawn((Mesh3d(floor_mesh), MeshMaterial3d(floor_material), WetFloor, canvas));

    // The subject: the shared six-cell blockout every other demo uses.
    let flesh = materials.add(StandardMaterial {
        base_color: Color::srgb(0.62, 0.52, 0.48),
        perceptual_roughness: 0.7,
        ..default()
    });
    for (mesh, xf) in common::body::subject() {
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(flesh.clone()),
            Transform::from_matrix(Mat4::from_translation(ORIGIN) * xf),
        ));
    }

    commands.insert_resource(Flagship {
        tick: 0,
        fired_at: 0,
        shot: Shot::Shoulder,
        tissue: TissueClass::Cortical,
        policy: GorePolicy::for_tier(GoreTier::BloodAndGore),
        // A quarter of the measured spatter speed, which is what puts the throw on a 1.8 m subject
        // instead of 44 m up. Both of this crate's other windowed examples set the same.
        settings: CarnageSettings {
            blood: BloodSettings { spatter_speed_scale: 0.25, ..default() },
            ..default()
        },
        fragments: 0,
        bent: Vec3::ZERO,
        drops: Vec::new(),
        landed: Vec::new(),
        bleed: blood::Bleed::new(0, 0.0),
        strands: Vec::new(),
        mesentery: Vec::new(),
        visc: ViscSettings::default(),
        wet: WetSettings::default(),
        show_numbers: true,
    });

    commands.spawn((
        Text::new(""),
        TextFont { font_size: bevy::text::FontSize::Px(13.0), ..default() },
        TextColor(Color::srgb(0.92, 0.88, 0.86)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(12.0),
            ..default()
        },
        Hud,
    ));
}

/// The on-screen readout.
#[derive(Component)]
struct Hud;

fn keys(
    input: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<Flagship>,
    mut commands: Commands,
    ephemeral: Query<Entity, With<Ephemeral>>,
) {
    let mut fire = false;
    for (key, shot) in [
        (KeyCode::Digit1, Shot::Shoulder),
        (KeyCode::Digit2, Shot::Thigh),
        (KeyCode::Digit3, Shot::Torso),
        (KeyCode::Digit4, Shot::Rifle),
    ] {
        if input.just_pressed(key) {
            state.shot = shot;
            fire = true;
        }
    }
    for (key, tier) in [
        (KeyCode::KeyQ, GoreTier::Stylised),
        (KeyCode::KeyW, GoreTier::Blood),
        (KeyCode::KeyE, GoreTier::BloodAndGore),
        (KeyCode::KeyR, GoreTier::GrossViolence),
    ] {
        if input.just_pressed(key) {
            state.policy = GorePolicy::for_tier(tier);
        }
    }
    if input.just_pressed(KeyCode::KeyT) {
        state.tissue = match state.tissue {
            TissueClass::Cortical => TissueClass::Trabecular,
            TissueClass::Trabecular => TissueClass::Soft,
            TissueClass::Soft => TissueClass::Cortical,
        };
        fire = true;
    }
    if input.just_pressed(KeyCode::KeyH) {
        state.show_numbers = !state.show_numbers;
    }
    if input.just_pressed(KeyCode::Space) {
        fire = true;
    }

    if !fire {
        return;
    }
    for e in &ephemeral {
        commands.entity(e).despawn();
    }
    shoot(&mut state);
}

/// **The whole chain, in one function, in the order the physics happens.**
fn shoot(state: &mut Flagship) {
    let shot = state.shot;
    let tissue = state.tissue;
    let (torque, impulse, energy, strain_rate) = shot.load();

    // ---- 1. The bone breaks the way it was loaded. -------------------------
    let fracture_settings = bevy_carnage::FractureSettings::default();
    // The fragment count comes from Grady's energy balance rather than from an artist constant, and
    // the tissue picks the toughness — so a rifle round comminutes a bone a pistol round wedges, and
    // trabecular bone refuses to shard at any energy.
    let volume: f32 = common::body::parts()
        .iter()
        .map(|(_, _, h)| 8.0 * h.x * h.y * h.z)
        .sum();
    let target = bevy_carnage::grady_mott_target(volume, energy, strain_rate, tissue, &fracture_settings)
        .clamp(1, TARGET);

    let subject = common::body::subject();
    let parts: Vec<(&Mesh, Mat4)> = subject.iter().map(|(m, x)| (m, *x)).collect();
    let proxy: Vec<ProxyCell> = common::body::proxy();
    let cut = CutSettings {
        fault: FaultPolicy::Morphology {
            mode: shot.mode(),
            tissue,
            axis: shot.axis(),
            torque,
            impulse,
        },
        tissue,
        bores: vec![Bore::new(shot.at(), Vec3::Z, 0.035)],
        ..CutSettings::new(target, 0.08, SEED)
    };
    let bake = fracture_mesh(&parts, &proxy, &cut);
    state.bent = bake.bent;
    state.fragments = bake.into_leaves().len();

    // ---- 2. The blood pattern that fits the wound. -------------------------
    let wound = blood::Wound {
        at: [
            shot.at().x + ORIGIN.x,
            shot.at().y + ORIGIN.y,
            shot.at().z + ORIGIN.z,
        ],
        normal: [0.0, 0.35, 0.94],
        area: 0.006,
        severity: 1.0,
        kind: blood::WoundKind::Channel,
    };
    state.bleed = blood::Bleed::new(state.tick, wound.area);
    state.fired_at = state.tick;
    state.drops.clear();
    state.landed.clear();

    // **The class decides the mechanism, not the intensity.** An impact wound throws the percolation
    // cone once; an arterial one throws one arc per systole, and `advance` keeps asking for those.
    let first = match shot.class() {
        blood::PatternClass::ArterialSpurt => blood::patterns::arterial_arc(
            &wound,
            &state.bleed,
            state.tick,
            HZ,
            &state.settings.blood,
        ),
        _ => blood::patterns::impact_spatter(&wound, &state.settings.blood),
    };
    launch(state, &wound, &first);

    // ---- 4. The guts it opened. -------------------------------------------
    state.strands.clear();
    state.mesentery.clear();
    if shot.spills() && state.policy.viscera {
        let from = Vec3::new(shot.at().x, shot.at().y, 0.16) + ORIGIN;
        state.strands = bevy_viscera::spill(from, Vec3::new(0.0, -0.4, 1.0), 5, SEED, &state.visc);
        // Tethered every fourth node to the wound: `bevy_viscera`'s own example measured that as the
        // spacing that holds, against every twelfth, which parts.
        for strand in &state.strands {
            let anchors = (0..strand.nodes().len() as u32)
                .step_by(4)
                .map(|i| (i, from))
                .collect();
            state.mesentery.push(Mesentery { anchors, ..default() });
        }
    }
}

/// Turn a set of droplets into blood in flight, respecting the policy's tier.
///
/// **Reduction is substitution.** At `Stylised` the same droplets are launched, on the same tick, in
/// the same directions, at the same speeds — only the palette changes, to
/// `CarnageSettings::substitute_srgb`. Deleting the channel is how a gore-off setting makes a game
/// harder to read.
fn launch(state: &mut Flagship, wound: &blood::Wound, drops: &[blood::Droplet]) {
    let cap = state.policy.max_decals as usize;
    let intensity = state.policy.intensity.clamp(0.0, 1.0);
    let wanted = ((drops.len() as f32) * intensity).round() as usize;
    for d in drops.iter().take(wanted.min(cap)) {
        let from = Vec3::new(wound.at[0], wound.at[1], wound.at[2]);
        let vel = Vec3::new(d.dir[0], d.dir[1], d.dir[2]) * d.speed;
        state.drops.push((from, vel, state.tick));
    }
}

fn advance(mut state: ResMut<Flagship>) {
    state.tick = state.tick.wrapping_add(1);
    let tick = state.tick;
    let dt = 1.0 / HZ as f32;
    let gravity = state.settings.blood.gravity;

    // ---- 2b. One arc per systole, while the wound still flows. -------------
    if state.shot.class() == blood::PatternClass::ArterialSpurt {
        let wound = blood::Wound {
            at: [
                state.shot.at().x + ORIGIN.x,
                state.shot.at().y + ORIGIN.y,
                state.shot.at().z + ORIGIN.z,
            ],
            normal: [0.0, 0.35, 0.94],
            area: 0.006,
            severity: 1.0,
            kind: blood::WoundKind::Channel,
        };
        let bleed = state.bleed;
        let arc = blood::patterns::arterial_arc(&wound, &bleed, tick, HZ, &state.settings.blood);
        if !arc.is_empty() {
            launch(&mut state, &wound, &arc);
        }
    }

    // ---- 3a. Fly the blood, and record where it lands. ---------------------
    let mut landed = Vec::new();
    state.drops.retain_mut(|(pos, vel, born)| {
        vel.y -= gravity * dt;
        *pos += *vel * dt;
        if pos.y <= 0.0 {
            landed.push((Vec3::new(pos.x, 0.0, pos.z), vel.length(), *born));
            return false;
        }
        // Anything that leaves the slab is gone rather than pinned to its edge.
        pos.x.abs() < FLOOR_HALF * 2.0 && pos.z.abs() < FLOOR_HALF * 2.0 && pos.y < 8.0
    });
    for (at, speed, _) in landed {
        state.landed.push((at, speed, tick));
    }
    let cap = state.policy.max_decals as usize;
    if state.landed.len() > cap {
        // Oldest-first, in the order they landed — a total order, so which stain is evicted is a
        // function of the record rather than of iteration.
        let excess = state.landed.len() - cap;
        state.landed.drain(0..excess);
    }

    // ---- 4b. Step the guts. ------------------------------------------------
    if !state.strands.is_empty() {
        let Flagship { strands, mesentery, visc, .. } = &mut *state;
        step(strands, mesentery, visc);
    }
}

/// Paint what landed into the wetmap, then upload at most the budget's worth.
fn paint_and_flush(
    mut state: ResMut<Flagship>,
    mut images: ResMut<Assets<Image>>,
    mut floor: Query<(&mut WetCanvas, &Mesh3d, &GlobalTransform), With<WetFloor>>,
    meshes: Res<Assets<Mesh>>,
) {
    let tick = state.tick;
    let wet = state.wet.clone();
    let blood_settings = state.settings.blood.clone();
    let fresh: Vec<(Vec3, f32, u32)> =
        state.landed.iter().copied().filter(|(_, _, born)| *born == tick).collect();

    for (mut canvas, mesh3d, xf) in &mut floor {
        if let Some(mesh) = meshes.get(&mesh3d.0) {
            for (at, speed, _) in &fresh {
                // The impact the stain arrived with, through the same model that would draw it: the
                // silhouette is a measurement of the droplet rather than a texture choice.
                let impact = blood::stain::Impact {
                    speed: *speed,
                    diameter: 0.003,
                    angle_rad: std::f32::consts::FRAC_PI_2 * 0.75,
                    roughness: blood_settings.substrate_roughness,
                    travel: [1.0, 0.0],
                };
                let shape = blood::stain::stain_shape(&impact, &blood_settings, tick ^ hash_bits(at));
                // Cast straight down onto the slab from just above the landing point.
                canvas.paint_world(mesh, xf, *at + Vec3::Y * 0.2, -Vec3::Y, &shape, tick);
            }
        }
        canvas.tick(tick, Vec2::new(0.0, 1.0), &wet);
        canvas.flush(&mut images);
    }
    let _ = &mut state;
}

/// **Regenerate every gut's tube mesh from its strand.** One asset write per strand per tick.
///
/// Spawns an entity for a strand that does not have one yet and despawns the ones whose strand is
/// gone, so a reset needs no bookkeeping beyond `Flagship::strands`.
fn retube(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    state: Res<Flagship>,
    guts: Query<(Entity, &Gut, &Mesh3d)>,
) {
    for (entity, gut, mesh3d) in &guts {
        match state.strands.get(gut.0) {
            Some(strand) => {
                if let Some(mut slot) = meshes.get_mut(&mesh3d.0) {
                    // Replaced wholesale rather than edited in place: `tube_mesh` builds every
                    // attribute and every index from the strand, so a partial update would be two
                    // ways of writing one mesh.
                    let fresh = tube_mesh(strand, 8);
                    *slot = fresh;
                }
            }
            None => commands.entity(entity).despawn(),
        }
    }
    let drawn = guts.iter().count();
    for index in drawn..state.strands.len() {
        let Some(strand) = state.strands.get(index) else { continue };
        commands.spawn((
            Mesh3d(meshes.add(tube_mesh(strand, 8))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.55, 0.20, 0.22),
                perceptual_roughness: 0.35,
                ..default()
            })),
            Gut(index),
            Ephemeral,
        ));
    }
}

/// A stable per-stain key from where it landed. Quantised, so a float ULP does not change the mask.
fn hash_bits(at: &Vec3) -> u32 {
    let q = |x: f32| (x / bevy_carnage::WELD).round() as i64 as u32;
    q(at.x) ^ q(at.z).wrapping_mul(0x9E37_79B9)
}

/// Draw the blood, the guts and the tethers. Meshes and gizmos only — there are no particles in a
/// wasm build of this crate.
fn orbit(
    mut gizmos: Gizmos,
    state: Res<Flagship>,
    time: Res<Time>,
    mut camera: Query<&mut Transform, With<MainCamera>>,
) {
    // A slow orbit, so a still page still shows the geometry. `Res<Time>`, never `Instant`.
    let angle = time.elapsed_secs() * 0.15;
    for mut xf in &mut camera {
        let r = 3.4;
        *xf = Transform::from_xyz(angle.sin() * r, 1.6, angle.cos() * r)
            .looking_at(Vec3::new(0.0, 0.85, 0.0), Vec3::Y);
    }

    // The palette: blood, or the substitute at the stylised tier.
    let srgb = if state.policy.draws_blood() {
        [0.62, 0.04, 0.04]
    } else {
        state.settings.substitute_srgb
    };
    let fresh = Color::srgb(srgb[0], srgb[1], srgb[2]);

    for (pos, vel, _) in &state.drops {
        // A short trail rather than a point: a single pixel at 60 Hz is invisible.
        gizmos.line(*pos, *pos - *vel * (1.0 / HZ as f32) * 2.0, fresh);
    }

    // ---- 5. The drying, on every stain that landed. ------------------------
    for (at, _, born) in &state.landed {
        let age = state.tick.wrapping_sub(*born);
        let look = blood::dry::appearance(age, HZ, 1.0e-4, &state.settings.blood);
        let c = if state.policy.draws_blood() {
            Color::srgb(look.srgb[0], look.srgb[1], look.srgb[2])
        } else {
            fresh
        };
        gizmos.circle(
            Isometry3d::new(*at + Vec3::Y * 0.002, Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            0.02 + 0.01 * look.rim,
            c,
        );
    }

    // The tethers, as lines, so a tear is visible the moment it happens. The guts themselves are
    // real tube meshes — see `retube`.
    for (i, strand) in state.strands.iter().enumerate() {
        let nodes = strand.nodes();
        if let Some(m) = state.mesentery.get(i) {
            for (node, anchor) in &m.anchors {
                let Some(p) = nodes.get(*node as usize) else { continue };
                let torn = m.torn.get(*node as usize).copied().unwrap_or(false);
                if torn {
                    continue;
                }
                gizmos.line(*p, *anchor, Color::srgb(0.40, 0.26, 0.30));
            }
        }
    }

    // The residual bend of a greenstick, if this shot produced one.
    if state.bent.length() > 0.0 {
        let base = ORIGIN + state.shot.at();
        gizmos.arrow(base, base + state.bent * 0.4, Color::srgb(0.95, 0.85, 0.25));
    }
}

fn draw_hud_text(
    state: Res<Flagship>,
    floor: Query<&WetCanvas, With<WetFloor>>,
    mut hud: Query<&mut Text, With<Hud>>,
) {
    let Ok(mut text) = hud.single_mut() else { return };
    if !state.show_numbers {
        **text = "[H] numbers".into();
        return;
    }
    let digest = floor.iter().next().map(|c| c.digest()).unwrap_or(0);
    let wetted = floor.iter().next().map(|c| c.wetted_area()).unwrap_or(0.0);
    let age = state.tick.wrapping_sub(state.fired_at);
    let torn: usize = state.mesentery.iter().map(|m| m.torn.iter().filter(|t| **t).count()).sum();
    let links: usize = state.mesentery.iter().map(|m| m.anchors.len()).sum();
    let look = blood::dry::appearance(age, HZ, 1.0e-3, &state.settings.blood);
    let (_, impulse, energy, rate) = state.shot.load();

    **text = format!(
        "carnage_web — one body, one shot, four crates\n\
         \n\
         [1-4] shot   [Q W E R] gore tier   [T] tissue   [Space] replay   [H] numbers\n\
         \n\
         1 bone     {shot}  ->  {mode:?} / {tissue:?}\n\
         \x20          {energy:.0} J at {rate:.0} 1/s, impulse {impulse:.0} N.s  ->  {frag} fragments{bent}\n\
         2 blood    {class:?}  ->  {inflight} in flight, {landed} landed\n\
         3 wetmap   digest 0x{digest:016x}   wetted {wetted:.5} m^2  (CPU-authoritative)\n\
         4 viscera  {strands} strands, {torn}/{links} mesentery links torn\n\
         5 drying   age {age} ticks  rgb {r:.2} {g:.2} {b:.2}  rough {rough:.2}  rim {rim:.2}  \
         halo {halo:.2}  cracks {crack:.2}\n\
         \n\
         tier {tier:?}  intensity {intensity:.2}  {palette}",
        shot = state.shot.name(),
        mode = state.shot.mode(),
        tissue = state.tissue,
        energy = energy,
        rate = rate,
        impulse = impulse,
        frag = state.fragments,
        bent = if state.bent.length() > 0.0 { "  (GREENSTICK: bent, not broken)" } else { "" },
        class = state.shot.class(),
        inflight = state.drops.len(),
        landed = state.landed.len(),
        digest = digest,
        wetted = wetted,
        strands = state.strands.len(),
        torn = torn,
        links = links,
        age = age,
        r = look.srgb[0],
        g = look.srgb[1],
        b = look.srgb[2],
        rough = look.roughness,
        rim = look.rim,
        halo = look.halo,
        crack = look.craquelure,
        tier = state.policy.tier,
        intensity = state.policy.intensity,
        palette = if state.policy.draws_blood() {
            "blood"
        } else {
            "SUBSTITUTED palette — the emitter still fires, at the same tick and magnitude"
        },
    );
}
