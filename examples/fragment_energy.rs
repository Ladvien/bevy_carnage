//! **The fragment count is not an artist constant.**
//!
//! Grady's energy balance (`doi:10.1063/1.329934`) gives a characteristic fragment size
//!
//! ```text
//!   s = (24 * G_c / (rho * strain_rate^2))^(1/3)
//! ```
//!
//! and the count is the subject's volume over `s^3` — so a faster load makes smaller pieces,
//! **cubically**. The energy delivered is a second, hard ceiling: creating *n* fragments costs `G_c`
//! per unit of new surface, and a blow cannot buy more than it brought.
//!
//! Those are two independent bounds, [`grady_mott_target`] takes the **minimum** of them, and *which
//! one is binding* is the interesting number — so it is on screen. A pistol leaves two pieces and a
//! wedge; a rifle comminutes.
//!
//! The histogram is the other half. Real brittle fragment volumes follow Mott's distribution
//! (`doi:10.1098/rspa.1947.0042`) — many small, few large — and that spread comes from
//! [`CutSettings::plane_jitter`] and [`CutSettings::size_spread`], **not** from the count. So the two
//! halves of "how does it break" are separate and each is shown where it lives: the count from
//! Grady, the spread against Mott.
//!
//! ```text
//!   [ / ]   impact energy, J
//!   , / .   strain rate, 1/s
//!   T       cycle the tissue class
//!   H       the histogram
//! ```
//!
//! **The reference curve is Mott's qualitative shape, not a fitted PDF**, and the screen says so.
//! Nothing here estimates a distribution parameter from the bake; `mu` is the mean fragment volume,
//! which is the only scale the bake offers. The crate's own *quantitative* measure of the same claim
//! is the largest-over-smallest volume ratio (`src/audit.rs`'s Mott-spread test), and that is printed
//! beside the bars.
//!
//! **The subject is a bone in metres**, ~1.98e-4 m³, because Grady's law is SI and reads a real
//! volume: a 22 cm shaft 3 cm thick. Nothing here reaches for `examples/common/` — that module's
//! blood half draws forward decals, which live behind the `vfx` feature, and every wasm demo is
//! built with `vfx` off.
//!
//! Run: `cargo run -p bevy_carnage --example fragment_energy`

use bevy::prelude::*;
use bevy_carnage::{
    CutSettings, FaultPolicy, FractureSettings, LoadingMode, ProxyCell, TissueClass, fracture_mesh,
    grady_mott_target,
};

/// Half-extents of the shaft, metres. `0.03 x 0.22 x 0.03` — a long bone, and the volume Grady's law
/// actually reads.
const HALF: Vec3 = Vec3::new(0.015, 0.11, 0.015);
/// Where the shaft's centre sits.
const ORIGIN: Vec3 = Vec3::new(0.0, 0.16, 0.0);
/// The long axis the morphology policy measures against.
const LONG_AXIS: Vec3 = Vec3::Y;
/// One seed for every bake, so the **energy** is the only variable.
const SEED: u32 = 0x00C0_FFEE;
/// Small, so a 22 cm shaft can actually reach the 40-piece ceiling when the energy says so.
const MIN_FRACTION: f32 = 0.03;
/// How far each fragment is pushed out along its own centroid.
const EXPLODE: f32 = 0.7;
/// N·s. **Irrelevant to this demo and deliberately fixed:** greenstick is an outcome of
/// [`LoadingMode::Bending`] alone, and this demo loads the bone with a direct blow.
const IMPULSE: f32 = 90.0;

/// Multiplier per press of `[` / `]`.
const ENERGY_STEP: f32 = 1.5;
const ENERGY_MIN: f32 = 10.0;
const ENERGY_MAX: f32 = 5_000.0;
/// **A rifle round, and chosen so the opening frame is Grady's answer rather than the clamp's.** At
/// 500 J / 800 s^-1 the rate bound is 4.0 and `min_pieces` raises it to 6, so the first thing on
/// screen would be the clamp binding rather than the law. Here `volume / s^3` is about 25 and the
/// energy bound is an order above it, which is the rate-limited case the notes describe; stepping
/// `[` down four times reaches the pistol, where the clamp does take over and says so.
const START_ENERGY: f32 = 1_700.0;
/// Multiplier per press of `,` / `.`.
const RATE_STEP: f32 = 1.8;
const RATE_MIN: f32 = 1.0;
const RATE_MAX: f32 = 5_000.0;
const START_RATE: f32 = 2_000.0;

/// Histogram buckets over fragment volume. Ten reads as a distribution and still fits beside the
/// Mott bar in each slot.
const BUCKETS: usize = 10;

/// A fragment of the current break.
#[derive(Component)]
struct Shard;

/// Marks the status line.
///
/// **ASCII only in every `Text`.** Bevy's default font atlas has neither U+00B7 nor U+2014, so both
/// render as missing-glyph boxes — `bullet_holes.rs` found that the first time it ran.
#[derive(Component)]
struct HudStatus;

/// One bar of the histogram: which bucket, and whether it is the observed count or Mott's shape.
#[derive(Component)]
struct HistoBar {
    bucket: usize,
    expected: bool,
}

/// Everything `H` hides.
#[derive(Component)]
struct HistoPanel;

#[derive(Resource)]
struct Dials {
    energy_j: f32,
    strain_rate: f32,
    tissue: TissueClass,
    histogram: bool,
    /// Set by any change, cleared by the re-break. `true` initially, which bakes frame one.
    dirty: bool,
}

impl Default for Dials {
    fn default() -> Self {
        Dials {
            energy_j: START_ENERGY,
            strain_rate: START_RATE,
            tissue: TissueClass::Cortical,
            histogram: true,
            dirty: true,
        }
    }
}

/// What the last bake produced, and the arithmetic that asked for it.
#[derive(Resource, Default)]
struct Report {
    /// [`grady_mott_target`]'s answer — the crate's, and the one the bake was asked for.
    target: usize,
    /// What the bake actually returned. Below `target` when `min_fraction` or the tree bound first.
    leaves: usize,
    /// Grady's characteristic fragment size, metres.
    size_m: f32,
    /// `volume / s^3` — the rate-limited bound, unclamped.
    by_rate: f32,
    /// `energy / (6 s^2 G_c)` — the energy-limited bound, unclamped.
    by_energy: f32,
    counts: [u32; BUCKETS],
    expected: [f32; BUCKETS],
    /// Upper edge of the last bucket, m³.
    vmax: f32,
    /// Largest over smallest fragment volume — the crate's own Mott-spread measure. `None` when
    /// there is nothing to compare.
    spread: Option<f32>,
}

#[derive(Resource)]
struct Mats {
    skin: Handle<StandardMaterial>,
    interior: Handle<StandardMaterial>,
}

fn tissue_name(t: TissueClass) -> &'static str {
    match t {
        TissueClass::Cortical => "Cortical",
        TissueClass::Trabecular => "Trabecular",
        TissueClass::Soft => "Soft",
    }
}

fn next_tissue(t: TissueClass) -> TissueClass {
    match t {
        TissueClass::Cortical => TissueClass::Trabecular,
        TissueClass::Trabecular => TissueClass::Soft,
        TissueClass::Soft => TissueClass::Cortical,
    }
}

/// The `G_c` [`grady_mott_target`] reads for this tissue, J/m².
fn toughness(t: TissueClass, s: &FractureSettings) -> f32 {
    match t {
        TissueClass::Cortical => s.toughness_cortical,
        TissueClass::Trabecular => s.toughness_trabecular,
        TissueClass::Soft => s.toughness_soft,
    }
}

/// What that much energy is, roughly. Derived from the number rather than authored beside it, so the
/// label cannot disagree with the dial.
fn energy_label(j: f32) -> &'static str {
    if j < 60.0 {
        "a stumble"
    } else if j < 350.0 {
        "a bat"
    } else if j < 900.0 {
        "a 9 mm pistol round"
    } else if j < 2_000.0 {
        "a magnum round"
    } else {
        "a rifle round"
    }
}

fn rate_label(r: f32) -> &'static str {
    if r < 5.0 {
        "quasi-static"
    } else if r < 120.0 {
        "a fall"
    } else if r < 500.0 {
        "a blunt blow"
    } else if r < 1_500.0 {
        "a pistol round"
    } else {
        "a rifle round"
    }
}

/// **Mott's shape over one bucket**, exactly: `N(>m) = exp(-(m/mu)^(1/3))` evaluated at both edges
/// and differenced (`doi:10.1098/rspa.1947.0042`).
///
/// Differenced rather than sampled from a density, because the density diverges at `m = 0` and a
/// sampled first bucket would be an artefact of where the sample was taken. `mu` is the mean fragment
/// volume — the only scale the bake offers — so this is the **qualitative** claim "many small, few
/// large" and not a fit to the data beside it.
fn mott_bucket(lo: f32, hi: f32, mu: f32) -> f32 {
    if !(mu > 0.0) {
        return 0.0;
    }
    let survive = |m: f32| (-(m.max(0.0) / mu).cbrt()).exp();
    (survive(lo) - survive(hi)).max(0.0)
}

/// The shaft, as a render mesh and as the one convex cell the caller decomposed it into.
fn subject() -> (Mesh, Vec<ProxyCell>) {
    (
        Mesh::from(Cuboid::new(HALF.x * 2.0, HALF.y * 2.0, HALF.z * 2.0)),
        vec![ProxyCell::from_box(Vec3::ZERO, HALF)],
    )
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage - fragment energy".into(),
                // The one web line, inert on native.
                canvas: Some("#carnage-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        .init_resource::<Dials>()
        .init_resource::<Report>()
        .add_systems(Startup, setup)
        .add_systems(Update, (drive, hud, histogram))
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.34, 0.28, 0.44).looking_at(ORIGIN, Vec3::Y),
    ));
    // A fill, so an unlit cut face reads as shadowed rather than as a hole in the shard.
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
        Mesh3d(meshes.add(Mesh::from(Plane3d::default().mesh().size(4.0, 4.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.16, 0.16, 0.18),
            perceptual_roughness: 0.95,
            ..default()
        })),
    ));
    commands.insert_resource(Mats {
        skin: materials.add(StandardMaterial {
            base_color: Color::srgb(0.86, 0.84, 0.74),
            perceptual_roughness: 0.55,
            ..default()
        }),
        interior: materials.add(StandardMaterial {
            base_color: Color::srgb(0.46, 0.07, 0.07),
            perceptual_roughness: 0.42,
            ..default()
        }),
    });

    commands.spawn((
        Text::new(
            "[ / ]  impact energy      , / .  strain rate      T tissue      H the histogram\n\
             The fragment count is not an artist constant. Grady's energy balance gives a\n\
             characteristic fragment size s = (24 G_c / (rho rate^2))^(1/3), and the count is the\n\
             subject's volume over s^3 - so a faster load makes smaller pieces, cubically. The\n\
             energy delivered is a second, hard ceiling: n fragments cost G_c per unit of new\n\
             surface, and a blow cannot buy more than it brought. A pistol leaves two pieces and\n\
             a wedge; a rifle comminutes. The histogram is drawn against Mott's distribution -\n\
             many small, few large.",
        ),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        Node { position_type: PositionType::Absolute, top: px(12), left: px(14), ..default() },
    ));
    commands.spawn((
        HudStatus,
        Text::new(""),
        TextFont { font_size: FontSize::Px(16.0), ..default() },
        TextColor(Color::srgba(1.0, 0.92, 0.55, 0.95)),
        Node { position_type: PositionType::Absolute, bottom: px(14), left: px(14), ..default() },
    ));

    // The histogram: one slot per bucket, each holding the observed count beside Mott's shape.
    commands.spawn((
        HistoPanel,
        Text::new("fragment volume ->   solid: this bake    outline: Mott shape (qualitative)"),
        TextFont { font_size: FontSize::Px(13.0), ..default() },
        TextColor(Color::srgba(0.85, 0.90, 1.0, 0.85)),
        Node { position_type: PositionType::Absolute, bottom: px(184), right: px(16), ..default() },
    ));
    let panel = commands
        .spawn((
            HistoPanel,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(96),
                right: px(16),
                width: px(330),
                height: px(84),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexEnd,
                column_gap: px(6),
                ..default()
            },
        ))
        .id();
    for bucket in 0..BUCKETS {
        let slot = commands
            .spawn((
                ChildOf(panel),
                Node {
                    width: px(27),
                    height: percent(100.0),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::FlexEnd,
                    column_gap: px(3),
                    ..default()
                },
            ))
            .id();
        for expected in [false, true] {
            commands.spawn((
                ChildOf(slot),
                HistoBar { bucket, expected },
                Node { width: px(11), height: percent(0.0), ..default() },
                BackgroundColor(if expected {
                    Color::srgba(0.45, 0.72, 1.0, 0.55)
                } else {
                    Color::srgb(0.92, 0.36, 0.22)
                }),
            ));
        }
    }
}

/// Read the keys, and re-break whenever anything moved. Also bakes frame one, because [`Dials`]
/// starts dirty — one spawn path rather than a startup copy of it.
fn drive(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mats: Res<Mats>,
    mut dials: ResMut<Dials>,
    mut report: ResMut<Report>,
    keys: Res<ButtonInput<KeyCode>>,
    shards: Query<Entity, With<Shard>>,
) {
    if keys.just_pressed(KeyCode::BracketLeft) {
        dials.energy_j = (dials.energy_j / ENERGY_STEP).max(ENERGY_MIN);
        dials.dirty = true;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        dials.energy_j = (dials.energy_j * ENERGY_STEP).min(ENERGY_MAX);
        dials.dirty = true;
    }
    if keys.just_pressed(KeyCode::Comma) {
        dials.strain_rate = (dials.strain_rate / RATE_STEP).max(RATE_MIN);
        dials.dirty = true;
    }
    if keys.just_pressed(KeyCode::Period) {
        dials.strain_rate = (dials.strain_rate * RATE_STEP).min(RATE_MAX);
        dials.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        dials.tissue = next_tissue(dials.tissue);
        dials.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyH) {
        dials.histogram = !dials.histogram;
    }
    if !dials.dirty {
        return;
    }
    dials.dirty = false;
    for e in &shards {
        commands.entity(e).despawn();
    }
    *report = break_it(&mut commands, &mut meshes, &mats, &dials);
}

/// Ask Grady for a count, fracture to it, and measure what came back.
fn break_it(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &Mats,
    d: &Dials,
) -> Report {
    let fs = FractureSettings::default();
    let (mesh, proxy) = subject();
    let volume = HALF.x * HALF.y * HALF.z * 8.0;
    let target = grady_mott_target(volume, d.energy_j, d.strain_rate, d.tissue, &fs);

    // **The two bounds the crate collapses into one number.** `grady_mott_target` returns the
    // clamped minimum and exposes neither term, so they are recomputed here purely to *display*
    // them — and the crate's own answer is printed beside `min(by_rate, by_energy)`, which makes the
    // duplication a live cross-check rather than a second source of truth. The expressions are
    // `bevy_carnage::grady_mott_target`'s, line for line.
    let g_c = toughness(d.tissue, &fs);
    let size_m = (24.0 * g_c / (fs.density_kg_m3 * d.strain_rate * d.strain_rate)).cbrt();
    let by_rate = volume / (size_m * size_m * size_m);
    let by_energy = d.energy_j / (6.0 * size_m * size_m * g_c);
    let cut = CutSettings {
        fault: FaultPolicy::Morphology {
            // A direct blow: the load this demo's energy axis describes.
            mode: LoadingMode::DirectHighEnergy,
            tissue: d.tissue,
            axis: LONG_AXIS,
            torque: 0.0,
            impulse: IMPULSE,
        },
        tissue: d.tissue,
        ..CutSettings::new(target, MIN_FRACTION, SEED)
    };
    let leaves = fracture_mesh(&[(&mesh, Mat4::IDENTITY)], &proxy, &cut).into_leaves();

    // The volume distribution, and the spread the crate's own Mott test measures.
    let mut lo = f32::INFINITY;
    let mut hi = 0.0f32;
    let mut total = 0.0f32;
    for f in &leaves {
        let v = f.cell.volume();
        lo = lo.min(v);
        hi = hi.max(v);
        total += v;
    }
    let n = leaves.len();
    let spread = (n > 1 && lo > 0.0 && hi.is_finite()).then_some(hi / lo);
    info!(
        "{:.0} J at {:.0} /s on {:?}: s = {:.1} mm, by_rate {:.1}, by_energy {:.1}, target {}, \
         leaves {}, spread {:?}",
        d.energy_j,
        d.strain_rate,
        d.tissue,
        size_m * 1_000.0,
        by_rate,
        by_energy,
        target,
        n,
        spread
    );
    let vmax = if hi > 0.0 { hi } else { 1.0 };
    let mut counts = [0u32; BUCKETS];
    for f in &leaves {
        // `min` on the index, not on the value: the largest fragment belongs in the last bucket
        // rather than one past the end.
        let slot = ((f.cell.volume() / vmax) * BUCKETS as f32) as usize;
        if let Some(c) = counts.get_mut(slot.min(BUCKETS - 1)) {
            *c += 1;
        }
    }
    let mu = if n > 0 { total / n as f32 } else { 0.0 };
    let mut expected = [0.0f32; BUCKETS];
    let width = vmax / BUCKETS as f32;
    let mut weight_sum = 0.0f32;
    for (i, e) in expected.iter_mut().enumerate() {
        *e = mott_bucket(i as f32 * width, (i + 1) as f32 * width, mu);
        weight_sum += *e;
    }
    if weight_sum > 0.0 {
        for e in expected.iter_mut() {
            *e = *e / weight_sum * n as f32;
        }
    }

    let skin = mats.skin.clone();
    for f in leaves {
        let at = ORIGIN + f.center_local * (1.0 + EXPLODE);
        let shard = commands
            .spawn((Shard, Transform::from_translation(at), Visibility::default()))
            .id();
        commands.entity(shard).with_children(|parent| {
            if let Some(outer) = f.outer {
                parent.spawn((Mesh3d(meshes.add(outer)), MeshMaterial3d(skin.clone())));
            }
            if let Some(cap) = f.cap {
                parent.spawn((Mesh3d(meshes.add(cap)), MeshMaterial3d(mats.interior.clone())));
            }
        });
    }

    Report { target, leaves: n, size_m, by_rate, by_energy, counts, expected, vmax, spread }
}

/// Bar heights, and `H`. Both bars share one vertical scale, or the comparison would be decoration.
fn histogram(
    dials: Res<Dials>,
    report: Res<Report>,
    mut bars: Query<(&mut Node, &HistoBar)>,
    mut panel: Query<&mut Node, (With<HistoPanel>, Without<HistoBar>)>,
) {
    let peak = report
        .counts
        .iter()
        .map(|c| *c as f32)
        .chain(report.expected.iter().copied())
        .fold(1.0f32, f32::max);
    for (mut node, bar) in &mut bars {
        let v = if bar.expected {
            report.expected.get(bar.bucket).copied().unwrap_or(0.0)
        } else {
            report.counts.get(bar.bucket).map_or(0.0, |c| *c as f32)
        };
        let want = percent((v / peak * 100.0).clamp(0.0, 100.0));
        if node.height != want {
            node.height = want;
        }
    }
    let want = if dials.histogram { Display::Flex } else { Display::None };
    for mut node in &mut panel {
        if node.display != want {
            node.display = want;
        }
    }
}

/// The status line: the dials, Grady's size, both bounds, which one binds, and the Mott spread.
fn hud(dials: Res<Dials>, report: Res<Report>, mut line: Query<&mut Text, With<HudStatus>>) {
    let fs = FractureSettings::default();
    let g_c = toughness(dials.tissue, &fs);
    // Strictly less, so a tie is reported as the rate bound rather than as two winners.
    let energy_binds = report.by_energy < report.by_rate;
    let spread = match report.spread {
        Some(s) => format!("{s:.1}x"),
        None => "n/a (one fragment)".to_string(),
    };
    let trabecular = if dials.tissue == TissueClass::Trabecular {
        "  (trabecular crushes: the crate also ceilings it at 3)"
    } else {
        ""
    };
    let text = format!(
        "energy {:.0} J ({})    strain rate {:.0} /s ({})    tissue {} (G_c {:.0} J/m2, rho {:.0} \
         kg/m3)\n\
         Grady size s = {:.1} mm    volume / s^3 = {:.1}    energy / (6 s^2 G_c) = {:.1}    \
         BINDING: {}\n\
         grady_mott_target -> {} pieces, clamped to [{}, {}]{}    min(bounds) {:.1}    bake \
         produced {} leaves\n\
         largest / smallest fragment volume {}   (Mott spread: many small, few large - the \
         histogram's claim, measured)   histogram spans 0 to {:.1} mm3",
        dials.energy_j,
        energy_label(dials.energy_j),
        dials.strain_rate,
        rate_label(dials.strain_rate),
        tissue_name(dials.tissue),
        g_c,
        fs.density_kg_m3,
        report.size_m * 1_000.0,
        report.by_rate,
        report.by_energy,
        if energy_binds { "energy-limited" } else { "rate-limited" },
        report.target,
        fs.min_pieces,
        fs.max_pieces,
        trabecular,
        report.by_rate.min(report.by_energy),
        report.leaves,
        spread,
        // m3 -> mm3, so the axis is readable at a bone's scale.
        report.vmax * 1.0e9,
    );
    for mut t in &mut line {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}
