//! **Blood as a shear-thinning yield-stress fluid** — a rivulet that arrests mid-wall.
//!
//! Two claims, and both fall out of the constitutive model rather than out of a timer:
//!
//! 1. **A fast rivulet is thin and races; a slow one thickens and beads.** Carreau-Yasuda with
//!    Cho & Kensey's constants ([`blood::rheo::viscosity`]) gives the apparent viscosity, and the
//!    Nusselt film relation in [`film`] turns that one number into a thickness *and* a speed.
//!    Nothing here tunes them separately, which is why they cannot disagree.
//! 2. **A rivulet stops where it is** the moment its rising yield stress overtakes the falling
//!    driving stress — [`blood::rheo::flows`] against [`blood::rheo::yield_stress`]. **That is what
//!    clotting is**, at a different age, so the clot, the arrested rivulet and a pool that stopped
//!    creeping are one mechanism instead of three special cases. The plot on the wall draws both
//!    curves and marks the crossing.
//!
//! ```text
//!   left / right   shear rate
//!   up / down      hematocrit
//!   [  ]           age
//!   Space          release a rivulet
//! ```
//!
//! Release one at age 0 and it reaches the floor. Scrub the age towards the crossing and the next
//! one **stops part-way down and stays there** — the same blood, the same wall, a different age.
//!
//! Builds all its geometry in code and ships no assets, so it runs in a browser. Every string on
//! screen is ASCII, because the embedded default font is a 95-codepoint subset and anything else
//! renders as tofu.
//!
//! Run: `cargo run --release -p bevy_carnage --example rheology`

use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use bevy_carnage::blood::dry::{SRGB_MET, SRGB_OXY};
use bevy_carnage::blood::rheo::{self, MU_INF, MU_ZERO, PERFUSION_STRESS_PA};
use bevy_carnage::{BLOOD_DENSITY, BloodSettings, blood, hash_f32};

/// The fixed tick, and the rate `bloodstain`'s shipped tick counts are authored for. Every age here
/// is an integer count of these — no `Instant` anywhere, because there is no such clock in a
/// browser.
const HZ: u32 = 60;

/// The wall, the release height and the floor, in metres.
const WALL_W: f32 = 3.4;
const WALL_H: f32 = 2.7;
const SPAWN_Y: f32 = 2.45;
const FLOOR_Y: f32 = 0.02;

/// Release slots across the left of the wall, so successive rivulets stand **side by side** and the
/// thin-and-fast against thick-and-beaded comparison is a glance rather than a memory.
const SLOTS: u32 = 9;
const SLOT_X0: f32 = -1.48;
const SLOT_DX: f32 = 0.19;

/// **Volumetric flux per unit width of one rivulet, m^2/s.** The demo's single scale choice.
///
/// At 1e-3 a centimetre-wide rivulet carries about 10 mL/s — a serious wound rather than an absurd
/// one — and it puts the wall traversal in the two-to-five second range a visitor will watch.
/// Everything else about the rivulet is derived from it and from the viscosity.
const FLUX: f32 = 1.0e-3;
/// The film is **millimetres** and the wall is **metres**, so the drawn ribbon's width is the film
/// thickness times this gain. One stated exaggeration of one quantity — not a second model.
const WIDTH_GAIN: f32 = 22.0;
/// Ribbon path resolution, bead depth as a fraction of the half-width, and the meander amplitude.
const ROW_M: f32 = 0.02;
const BEAD_GAIN: f32 = 0.45;
const MEANDER_M: f32 = 0.014;
/// Rayleigh-Plateau: a liquid thread breaks up at a wavelength of about 9 radii, which is why a
/// slow thick rivulet beads and a fast thin one does not.
const RP_WAVELENGTH: f32 = 9.02;
/// How far in front of the wall the blood and the plot sit.
const Z_BLOOD: f32 = 0.012;
const Z_PLOT: f32 = 0.03;

/// The plot's rectangle on the wall: age across, stress up.
const GX0: f32 = 0.42;
const GX1: f32 = 1.56;
const GY0: f32 = 0.55;
const GY1: f32 = 1.78;

/// Dial limits. The shear-rate span brackets Carreau-Yasuda's knee at `1 / CY_LAMBDA` on both
/// sides, so the whole shear-thinning curve is reachable from the keys.
const SHEAR_MIN: f32 = 0.05;
const SHEAR_MAX: f32 = 2000.0;
const HCT_MIN: f32 = 0.20;
const HCT_MAX: f32 = 0.70;

/// Plot colours, shared by the gizmo curves and the legend text so the two cannot drift apart.
const DRIVING_RGB: [f32; 3] = [1.00, 0.30, 0.26];
const YIELD_RGB: [f32; 3] = [0.42, 0.86, 1.00];
const CROSS_RGB: [f32; 3] = [1.00, 0.88, 0.35];

/// Everything the keys move, plus the blood the whole demo reads.
#[derive(Resource)]
struct Dials {
    /// Shear rate fed to [`blood::rheo::viscosity`], 1/s.
    shear_rate: f32,
    /// The release age in ticks, held as a float only so the scrub is smooth.
    age: f32,
    /// **The one settings block.** Hematocrit lives here rather than beside it.
    blood: BloodSettings,
    /// The tick the two stress curves cross — scanned once, because it is a property of `blood`.
    crossing: u32,
}

impl Default for Dials {
    fn default() -> Self {
        let blood = BloodSettings::default();
        let crossing = crossing_tick(&blood);
        // 10 1/s: past the knee, so the default rivulet is briskly thin and both dials move it
        // visibly in either direction.
        Dials { shear_rate: 10.0, age: 0.0, blood, crossing }
    }
}

impl Dials {
    fn age_ticks(&self) -> u32 {
        self.age.max(0.0).round() as u32
    }
}

/// The integer fixed-tick counter. Ages are differences of this and nothing else.
#[derive(Resource, Default)]
struct Ticks(u32);

/// Releases asked for by `Space` in `Update`, spent by `FixedUpdate`.
///
/// `just_pressed` is cleared in `PreUpdate` and `FixedUpdate` can run zero or several times in one
/// frame — so the press is counted where it is legible and spent where the tick is.
#[derive(Resource, Default)]
struct Pending(u32);

/// The release counter, which also picks the slot.
#[derive(Resource, Default)]
struct Serial(u32);

/// The two states blood is drawn in here: flowing, and arrested by its own yield stress.
#[derive(Resource)]
struct Paint {
    flowing: Handle<StandardMaterial>,
    arrested: Handle<StandardMaterial>,
}

/// Whether a rivulet is still moving, and if not, why it stopped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Flowing,
    /// Its yield stress overtook the stress driving it. **Terminal** — both curves are monotone, so
    /// a clot here cannot un-form.
    Arrested,
    /// It reached the floor. A different fact from arresting, and the readout says which.
    Landed,
}

/// The silhouette of one rivulet: where it runs, how wide, and how much it beads.
///
/// Sampled **at release** and kept, which is what makes two rivulets side by side a comparison
/// rather than two views of the same number.
struct Shape {
    x: f32,
    /// Drawn half-width with no beading, metres.
    half_width: f32,
    /// How far up the viscosity span this blood sits, `[0, 1]` — the beading amplitude.
    bead: f32,
    seed: u32,
}

/// One rivulet: its silhouette, its speed, where its head is, and the path it has drawn.
#[derive(Component)]
struct Rivulet {
    serial: u32,
    born: u32,
    age0: u32,
    shape: Shape,
    head: f32,
    next_row: f32,
    /// Mean film speed, m/s.
    speed: f32,
    /// Committed path rows, `(x, y)`, top first.
    rows: Vec<[f32; 2]>,
    state: State,
    mesh: Handle<Mesh>,
}

/// Marks the live readout.
#[derive(Component)]
struct Readout;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage - rheology".into(),
                canvas: Some("#carnage-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Time::<Fixed>::from_hz(HZ as f64))
        .init_resource::<Dials>()
        .init_resource::<Ticks>()
        .init_resource::<Pending>()
        .init_resource::<Serial>()
        .add_systems(Startup, setup)
        .add_systems(Update, (dials, readout, plot).chain())
        .add_systems(FixedUpdate, (count, release, advance).chain())
        .run();
}

/// **The tick the driving stress falls below the yield stress**, found by asking the same predicate
/// a rivulet asks.
///
/// Not solved algebraically on purpose: `flows` *is* the definition of arrest, so scanning it means
/// the marked crossing and the rivulets cannot disagree about where the clot is.
fn crossing_tick(s: &BloodSettings) -> u32 {
    for age in 0..=s.clot_ticks {
        let driving = PERFUSION_STRESS_PA * blood::bleed::envelope(age, s);
        if !rheo::flows(driving, rheo::yield_stress(age, HZ, s)) {
            return age;
        }
    }
    s.clot_ticks
}

/// **Thickness and mean speed of a gravity-driven film at one viscosity.**
///
/// Nusselt's falling film: at a fixed flux per unit width, `t = (3 mu q / (rho g))^(1/3)`, and the
/// mean speed is `q / t`. So a thin film is *necessarily* the fast one and a thick film the slow
/// one — the two halves of the first claim are one equation, and the only input that moves them is
/// the viscosity the crate computed.
fn film(mu: f32, s: &BloodSettings) -> (f32, f32) {
    let g = if s.gravity.is_finite() && s.gravity > 0.0 { s.gravity } else { 18.0 };
    let mu = if mu.is_finite() && mu > 0.0 { mu } else { MU_INF };
    let t = (3.0 * mu * FLUX / (BLOOD_DENSITY * g)).cbrt().max(1.0e-5);
    (t, FLUX / t)
}

/// Where a rivulet's centreline sits at a height, and how wide it is there.
fn profile(sh: &Shape, y: f32) -> (f32, f32) {
    let fallen = (SPAWN_Y - y).max(0.0);
    let phase = hash_f32(sh.seed) * std::f32::consts::TAU;
    let lambda = (RP_WAVELENGTH * sh.half_width).max(0.03);
    let swell = 1.0 + BEAD_GAIN * sh.bead * (fallen / lambda * std::f32::consts::TAU + phase).sin();
    let wander = MEANDER_M * (fallen * 3.1 + phase).sin() * (0.35 + 0.65 * sh.bead);
    (sh.x + wander, (sh.half_width * swell).max(0.001))
}

/// **The rivulet as one mesh**: a three-vertex-per-row ribbon, so a rivulet costs one draw call
/// rather than two hundred spheres. Rebuilt when a row is committed, every [`ROW_M`] of travel.
fn ribbon(sh: &Shape, rows: &[[f32; 2]]) -> Mesh {
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(rows.len() * 3);
    let mut nrm: Vec<[f32; 3]> = Vec::with_capacity(rows.len() * 3);
    let mut uv: Vec<[f32; 2]> = Vec::with_capacity(rows.len() * 3);
    let mut idx: Vec<u32> = Vec::with_capacity(rows.len() * 12);

    // A rounded ridge rather than a flat strip: the middle vertex stands off the wall, so the
    // specular highlight runs down the rivulet and wet reads as wet.
    let left = Vec3::new(-1.0, 0.0, 1.0).normalize().to_array();
    let mid = [0.0, 0.0, 1.0];
    let right = Vec3::new(1.0, 0.0, 1.0).normalize().to_array();

    for (i, row) in rows.iter().enumerate() {
        let (_, w) = profile(sh, row[1]);
        let (cx, y) = (row[0], row[1]);
        pos.push([cx - w, y, Z_BLOOD]);
        pos.push([cx, y, Z_BLOOD + w * 0.7]);
        pos.push([cx + w, y, Z_BLOOD]);
        nrm.push(left);
        nrm.push(mid);
        nrm.push(right);
        let v = i as f32 * ROW_M * 8.0;
        uv.push([0.0, v]);
        uv.push([0.5, v]);
        uv.push([1.0, v]);
    }
    for i in 0..rows.len().saturating_sub(1) {
        let top = (i * 3) as u32;
        let bot = ((i + 1) * 3) as u32;
        for k in 0..2u32 {
            idx.extend_from_slice(&[top + k, bot + k, bot + k + 1]);
            idx.extend_from_slice(&[top + k, bot + k + 1, top + k + 1]);
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
    .with_inserted_indices(Indices::U32(idx))
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    dials: Res<Dials>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.34, 3.45).looking_at(Vec3::new(0.0, 1.26, 0.0), Vec3::Y),
    ));
    // A key light almost in line with the camera, so a low-roughness surface glares back at the
    // viewer and a high-roughness one does not. The wetness channel is specular; this is what makes
    // it visible at all.
    commands.spawn((
        DirectionalLight { illuminance: 11_000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_xyz(0.4, 2.4, 3.0).looking_at(Vec3::new(0.0, 1.2, 0.0), Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.60, 0.64, 0.76),
        brightness: 420.0,
        ..default()
    });

    let wall_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.21, 0.22, 0.25),
        perceptual_roughness: 0.93,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Mesh::from(Cuboid::new(WALL_W, WALL_H, 0.1)))),
        MeshMaterial3d(wall_mat),
        Transform::from_xyz(0.0, WALL_H * 0.5, -0.05),
    ));
    let floor_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.12, 0.14),
        perceptual_roughness: 0.96,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Mesh::from(Plane3d::default().mesh().size(9.0, 9.0)))),
        MeshMaterial3d(floor_mat),
    ));

    commands.insert_resource(Paint {
        // Fresh blood: oxyhaemoglobin, and glossy, because wet is a specular channel.
        flowing: materials.add(StandardMaterial {
            base_color: Color::srgb(SRGB_OXY[0], SRGB_OXY[1], SRGB_OXY[2]),
            perceptual_roughness: dials.blood.wet_roughness,
            ..default()
        }),
        // Arrested blood is a clot: the same fluid past its own yield stress, so it goes matte and
        // takes the methaemoglobin step the drying model names.
        arrested: materials.add(StandardMaterial {
            base_color: Color::srgb(SRGB_MET[0], SRGB_MET[1], SRGB_MET[2]),
            perceptual_roughness: dials.blood.dry_roughness,
            ..default()
        }),
    });

    let font = TextFont { font_size: FontSize::Px(14.0), ..default() };
    commands.spawn((
        Text::new(
            "bevy_carnage - RHEOLOGY: blood as a shear-thinning yield-stress fluid\n\
             \n\
             left / right   shear rate\n\
             up / down      hematocrit\n\
             [  ]           age\n\
             Space          release a rivulet\n\
             \n\
             A fast rivulet is thin and races; a slow one thickens and beads.\n\
             Scrub the age towards the crossing and the next one ARRESTS MID-WALL.",
        ),
        font.clone(),
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.88)),
        Node { position_type: PositionType::Absolute, top: px(12), left: px(14), ..default() },
    ));
    commands.spawn((
        Readout,
        Text::new(""),
        font.clone(),
        TextColor(Color::srgba(1.0, 0.94, 0.72, 0.95)),
        Node { position_type: PositionType::Absolute, top: px(12), right: px(14), ..default() },
    ));

    // The plot's legend. One text block with the curve colours named in words rather than three
    // colour-matched nodes: the crossing tick is a property of the settings, so all of it is static.
    commands.spawn((
        Text::new(format!(
            "the plot on the wall, age across and stress up:\n\
             red    driving stress = PERFUSION_STRESS_PA x envelope(age)\n\
             blue   yield stress, Casson fresh, climbing to clot_yield_pa\n\
             yellow CROSSING at tick {} ({:.2} s) -- that crossing IS the clot\n\
             white  the age the next release carries",
            dials.crossing,
            dials.crossing as f32 / HZ as f32,
        )),
        font,
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        Node { position_type: PositionType::Absolute, bottom: px(12), right: px(14), ..default() },
    ));
}

/// The integer clock. Wrapping, like [`bevy_carnage::Bleed::age`], so a page left open cannot panic
/// on a subtraction.
fn count(mut ticks: ResMut<Ticks>) {
    ticks.0 = ticks.0.wrapping_add(1);
}

fn dials(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut d: ResMut<Dials>,
    mut pending: ResMut<Pending>,
) {
    let dt = time.delta_secs();
    // Multiplicative, because shear rate spans four decades and a linear dial would spend its whole
    // travel above the knee.
    let factor = 2.0f32.powf(dt * 3.0);
    if keys.pressed(KeyCode::ArrowRight) {
        d.shear_rate *= factor;
    }
    if keys.pressed(KeyCode::ArrowLeft) {
        d.shear_rate /= factor;
    }
    d.shear_rate = d.shear_rate.clamp(SHEAR_MIN, SHEAR_MAX);

    if keys.pressed(KeyCode::ArrowUp) {
        d.blood.hematocrit += dt * 0.12;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        d.blood.hematocrit -= dt * 0.12;
    }
    d.blood.hematocrit = d.blood.hematocrit.clamp(HCT_MIN, HCT_MAX);

    if keys.pressed(KeyCode::BracketRight) {
        d.age += dt * 110.0;
    }
    if keys.pressed(KeyCode::BracketLeft) {
        d.age -= dt * 110.0;
    }
    d.age = d.age.clamp(0.0, d.blood.clot_ticks as f32);

    if keys.just_pressed(KeyCode::Space) {
        pending.0 = pending.0.saturating_add(1);
    }
}

fn release(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut serial: ResMut<Serial>,
    mut meshes: ResMut<Assets<Mesh>>,
    ticks: Res<Ticks>,
    dials: Res<Dials>,
    paint: Res<Paint>,
    live: Query<(Entity, &Rivulet)>,
) {
    if pending.0 == 0 {
        return;
    }
    pending.0 -= 1;

    // The wall holds one rivulet per slot. Past that the oldest goes, which frees exactly the slot
    // the next serial lands in.
    if live.iter().count() >= SLOTS as usize {
        if let Some(e) = live.iter().min_by_key(|(_, r)| r.serial).map(|(e, _)| e) {
            commands.entity(e).despawn();
        }
    }

    let s = &dials.blood;
    let mu = rheo::viscosity(dials.shear_rate, s.hematocrit, s);
    let (film_m, speed) = film(mu, s);
    let n = serial.0;
    serial.0 = serial.0.wrapping_add(1);

    let shape = Shape {
        x: SLOT_X0 + SLOT_DX * (n % SLOTS) as f32,
        half_width: film_m * WIDTH_GAIN * 0.5,
        bead: ((mu - MU_INF) / (MU_ZERO - MU_INF)).clamp(0.0, 1.0),
        seed: hash_f32(n ^ 0x5F37_59DF).to_bits() ^ n,
    };
    // Two rows, so the ribbon has a quad from the first frame: a bead appears at the release point
    // and then runs, which is what a wound does.
    let rows = vec![
        [profile(&shape, SPAWN_Y).0, SPAWN_Y],
        [profile(&shape, SPAWN_Y - ROW_M).0, SPAWN_Y - ROW_M],
    ];
    let mesh = meshes.add(ribbon(&shape, &rows));

    commands.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(paint.flowing.clone()),
        Transform::IDENTITY,
        Rivulet {
            serial: n,
            born: ticks.0,
            age0: dials.age_ticks(),
            shape,
            head: SPAWN_Y - ROW_M,
            next_row: SPAWN_Y - 2.0 * ROW_M,
            speed,
            rows,
            state: State::Flowing,
            mesh,
        },
    ));
}

/// **The whole model, once per rivulet per tick.**
///
/// One predicate decides whether it moves, and it is the same one that decides whether a wound has
/// clotted. There is no second `age > n` guard anywhere in this file.
fn advance(
    ticks: Res<Ticks>,
    dials: Res<Dials>,
    paint: Res<Paint>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut rivulets: Query<(&mut Rivulet, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let s = &dials.blood;
    for (mut r, mut skin) in &mut rivulets {
        if r.state != State::Flowing {
            continue;
        }
        let age = r.age0.saturating_add(ticks.0.wrapping_sub(r.born));
        let driving = PERFUSION_STRESS_PA * blood::bleed::envelope(age, s);
        let yielded = rheo::yield_stress(age, HZ, s);

        if !rheo::flows(driving, yielded) {
            // Arrested. Commit the head where it stands and go matte: the same fluid, past its own
            // yield stress. This is the frame the whole demo is for.
            let head = r.head;
            let cx = profile(&r.shape, head).0;
            r.rows.push([cx, head]);
            r.state = State::Arrested;
            *skin = MeshMaterial3d(paint.arrested.clone());
            rebuild(&r, &mut meshes);
            continue;
        }

        r.head -= r.speed / HZ as f32;
        if r.head <= FLOOR_Y {
            r.head = FLOOR_Y;
            r.state = State::Landed;
        }
        let mut dirty = false;
        while r.head <= r.next_row {
            let y = r.next_row;
            let cx = profile(&r.shape, y).0;
            r.rows.push([cx, y]);
            r.next_row -= ROW_M;
            dirty = true;
        }
        if r.state == State::Landed {
            let head = r.head;
            let cx = profile(&r.shape, head).0;
            r.rows.push([cx, head]);
            dirty = true;
        }
        if dirty {
            rebuild(&r, &mut meshes);
        }
    }
}

/// Replace the rivulet's mesh in place, so the handle every draw already holds stays valid.
fn rebuild(r: &Rivulet, meshes: &mut Assets<Mesh>) {
    let mesh = ribbon(&r.shape, &r.rows);
    if let Some(mut slot) = meshes.get_mut(&r.mesh) {
        *slot = mesh;
    }
}

fn readout(dials: Res<Dials>, rivulets: Query<&Rivulet>, mut out: Query<&mut Text, With<Readout>>) {
    let s = &dials.blood;
    let age = dials.age_ticks();
    let mu = rheo::viscosity(dials.shear_rate, s.hematocrit, s);
    let (film_m, speed) = film(mu, s);
    let bead = ((mu - MU_INF) / (MU_ZERO - MU_INF)).clamp(0.0, 1.0) * 100.0;
    let driving = PERFUSION_STRESS_PA * blood::bleed::envelope(age, s);
    let yielded = rheo::yield_stress(age, HZ, s);
    let arrested = rivulets.iter().filter(|r| r.state == State::Arrested).count();
    let landed = rivulets.iter().filter(|r| r.state == State::Landed).count();
    let (shear, hct, mpas) = (dials.shear_rate, s.hematocrit, mu * 1000.0);
    let (mm, secs, live) = (film_m * 1000.0, age as f32 / HZ as f32, rivulets.iter().count());
    let verdict = if rheo::flows(driving, yielded) { "FLOWS" } else { "ARRESTED" };

    let text = format!(
        "shear rate {shear:>7.2} 1/s    hematocrit {hct:.2}    viscosity {mpas:>6.2} mPa.s\n\
         next rivulet: film {mm:.2} mm, {speed:.2} m/s, beading {bead:>3.0}%\n\
         release age {age:>3} t ({secs:>4.2} s)  driving {driving:>6.3} Pa  \
         yield {yielded:>6.3} Pa  {verdict}\n\
         on the wall: {live} of {SLOTS} slots, {arrested} arrested, {landed} reached the floor",
    );
    for mut t in &mut out {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}

/// **The plot, and it is the claim.** Driving stress falling, yield stress rising, and the crossing
/// where a rivulet stops being a fluid.
fn plot(mut gizmos: Gizmos, dials: Res<Dials>) {
    let s = &dials.blood;
    let span = s.clot_ticks.max(1) as f32;
    let gx = |age: f32| GX0 + (age / span).clamp(0.0, 1.0) * (GX1 - GX0);
    let gy = |pa: f32| GY0 + (pa / PERFUSION_STRESS_PA).clamp(0.0, 1.0) * (GY1 - GY0);
    let at = |age: f32, pa: f32| Vec3::new(gx(age), gy(pa), Z_PLOT);

    let mid = Vec3::new((GX0 + GX1) * 0.5, (GY0 + GY1) * 0.5, Z_PLOT);
    let frame = Color::srgba(0.62, 0.66, 0.74, 0.55);
    gizmos.rect(mid, Vec2::new(GX1 - GX0, GY1 - GY0), frame);

    let step = (s.clot_ticks / 90).max(1);
    let mut driving = Vec::with_capacity(96);
    let mut yielded = Vec::with_capacity(96);
    let mut age = 0u32;
    loop {
        driving.push(at(age as f32, PERFUSION_STRESS_PA * blood::bleed::envelope(age, s)));
        yielded.push(at(age as f32, rheo::yield_stress(age, HZ, s)));
        if age >= s.clot_ticks {
            break;
        }
        age = (age + step).min(s.clot_ticks);
    }
    gizmos.linestrip(driving, Color::srgb(DRIVING_RGB[0], DRIVING_RGB[1], DRIVING_RGB[2]));
    gizmos.linestrip(yielded, Color::srgb(YIELD_RGB[0], YIELD_RGB[1], YIELD_RGB[2]));

    // The crossing: a full-height marker plus a box around the point, because this is the one place
    // on screen where the whole argument lands.
    let cross = Color::srgb(CROSS_RGB[0], CROSS_RGB[1], CROSS_RGB[2]);
    let cage = dials.crossing as f32;
    let cpa = PERFUSION_STRESS_PA * blood::bleed::envelope(dials.crossing, s);
    gizmos.line(at(cage, 0.0), at(cage, PERFUSION_STRESS_PA), cross);
    gizmos.rect(at(cage, cpa), Vec2::splat(0.07), cross);

    // Where the next release sits on that axis.
    let now = dials.age_ticks() as f32;
    gizmos.line(at(now, 0.0), at(now, PERFUSION_STRESS_PA), Color::srgba(1.0, 1.0, 1.0, 0.75));
}
