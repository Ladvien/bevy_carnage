//! **The same subject, the same seed, four silhouettes.**
//!
//! Sellán et al., *Breaking Good* (`doi:10.1145/3549540`, §6), state the limitation this closes in as
//! many words: their fault is *the same regardless of the directionality of the impact*, and they name
//! torsion as one of the missing cases. This is that case. One bone, one seed, four loads:
//!
//! - **Torsion** cracks along a **helix**, because under torsion the tensile stress is maximum in a
//!   plane at 45° to the long axis and a material weaker in tension than in shear follows it
//!   (Miyasaka et al., `doi:10.3233/BME-1991-1102`). Long sharp ends.
//! - **Bending** throws a **butterfly wedge**: a transverse crack opens on the tension face, branches
//!   obliquely toward the compression face, and the transverse portion forms *last* (Isa et al.,
//!   `doi:10.1016/j.forsciint.2021.110899`).
//! - **Axial** fails across its narrowest cross-section — a transverse break, which is what the
//!   direction-blind weak-axis sample already produced.
//! - **Direct high energy** comminutes: no plane is preferred at all.
//!
//! `T` is the other half of the claim. **Cortical bone splits along its own grain** — osteons run
//! along the shaft and its toughness is far lower for a crack running with them — so it yields long
//! sharp slivers. **Trabecular bone crushes**, tolerating ~30 % strain against cortical's ~2 %, so the
//! crate clamps it to [`bevy_carnage::TRABECULAR_MAX_PIECES`] pieces however hard it is hit.
//!
//! And drop the impulse below the greenstick threshold: the bone **bends instead of breaking**. One
//! fragment, no fault, and a permanent bow, drawn here from [`Fracture::bent`](bevy_carnage::Fracture)
//! (`doi:10.3390/jimaging11060187`). **Greenstick is an outcome, not a mode** — there is no fifth
//! `LoadingMode` for it and there must not be one.
//!
//! ```text
//!   1 / 2 / 3 / 4   torsion / bending / axial / direct high energy
//!   T               cycle the tissue class
//!   [ / ]           impulse, N.s
//!   R               re-break at a fresh seed
//! ```
//!
//! **The subject is a bone shaft rather than `common::body`, and that is the point rather than a
//! shortcut.** Every one of these modes is *directional* and measured against a long axis: a subject
//! with no long axis makes three of the four degenerate into each other, which is exactly how a
//! demo of this feature passes for the wrong reason. It is the same shaft
//! `tests/fault_modes.rs::limb` asserts against, so the demo and the test are talking about one
//! subject. Nothing here reaches for `examples/common/`: that module's blood half draws its slicks
//! as forward decals, which live behind the `vfx` feature, and every wasm demo is built with `vfx`
//! off. So this example owns its own three-line scene, exactly as `explode.rs` does.
//!
//! The fragments are spawned pushed slightly out along their own centroids, because the seam pattern
//! is the whole read and a reassembled bone hides it.
//!
//! Run: `cargo run -p bevy_carnage --example fault_modes`

use bevy::prelude::*;
use bevy_carnage::{
    CutSettings, FaultPolicy, FractureSettings, LoadingMode, ProxyCell, TRABECULAR_MAX_PIECES,
    TissueClass, fracture_mesh,
};

/// Half-extents of the shaft: twice as long as it is wide, so there is a long axis to twist about.
/// The same figure as `tests/fault_modes.rs::limb`.
const HALF: Vec3 = Vec3::new(0.08, 0.30, 0.08);
/// Where the shaft's centre sits, clear of the floor so the debris has somewhere to fan into.
const ORIGIN: Vec3 = Vec3::new(0.0, 0.62, 0.0);
/// The long axis both morphology modes measure against.
const LONG_AXIS: Vec3 = Vec3::Y;
/// **One seed.** `R` mixes a take counter into it; at take 0 this is exactly the seed, so switching
/// mode compares like with like.
const SEED: u32 = 0x0BAD_F00D;
/// Fragment count asked for. Not derived from an energy here — that is `fragment_energy`'s demo.
const TARGET: usize = 12;
/// Small, so a 0.6 m shaft can actually reach [`TARGET`] pieces.
const MIN_FRACTION: f32 = 0.05;
/// Applied torque, N·m. Drives the helix pitch under [`LoadingMode::Torsion`].
const TORQUE: f32 = 2.0;
/// How far each fragment is pushed out along its own centroid, as a fraction of that centroid's
/// length. Zero reassembles the bone and hides the seam the demo exists to show.
const EXPLODE: f32 = 0.55;
/// N·s per press of `[` or `]`. Three steps down from [`START_IMPULSE`] lands exactly on the
/// greenstick threshold, and the fourth crosses it — which is the transition worth seeing.
const IMPULSE_STEP: f32 = 3.0;
const IMPULSE_MAX: f32 = 60.0;
const START_IMPULSE: f32 = 24.0;
/// Metres of bow drawn at `|bent| == 1`. Cosmetic: the crate reports a direction and a magnitude in
/// `[0, 1]`, and how far a caller bends the drawn mesh is the caller's look.
const BOW: f32 = 0.26;

/// The shaft, as a render mesh and as the one convex cell the caller decomposed it into.
///
/// **One cell, deliberately.** A greenstick produces one fragment *per uncut cell*, so a six-cell
/// subject would answer the greenstick case with six intact pieces and bury the claim.
fn subject() -> (Mesh, Vec<ProxyCell>) {
    (
        Mesh::from(Cuboid::new(HALF.x * 2.0, HALF.y * 2.0, HALF.z * 2.0)),
        vec![ProxyCell::from_box(Vec3::ZERO, HALF)],
    )
}

/// A fragment of the current break. Despawned wholesale when anything changes.
#[derive(Component)]
struct Shard;

/// Marks the status line.
///
/// **Everything written into a `Text` here is ASCII.** Bevy's default font atlas carries neither
/// U+00B7 `·` nor U+2014 `—`, so both render as missing-glyph boxes — `bullet_holes.rs` found that
/// the first time it was run. The module docs and the window title are exempt: they go to a reader
/// and to the window manager, not to the atlas.
#[derive(Component)]
struct HudStatus;

/// Where the dials sit, and whether the bake still matches them.
#[derive(Resource)]
struct Dials {
    mode: LoadingMode,
    tissue: TissueClass,
    impulse: f32,
    /// Bumped by `R`. Mixed into the seed, so the *mode's signature* can be watched surviving a
    /// change of seed — which is a stronger claim than one lucky layout.
    takes: u32,
    /// Set by any change, cleared by the re-break. `true` initially, which is what bakes frame one.
    dirty: bool,
}

impl Default for Dials {
    fn default() -> Self {
        Dials {
            mode: LoadingMode::Torsion,
            tissue: TissueClass::Cortical,
            impulse: START_IMPULSE,
            takes: 0,
            dirty: true,
        }
    }
}

/// What the last bake actually produced — the on-screen claim, measured rather than described.
#[derive(Resource, Default)]
struct Report {
    fragments: usize,
    /// Longest over shortest bounding extent of the biggest shard. `None` for a degenerate cell,
    /// which is reported as `n/a` rather than as a fabricated ratio.
    aspect: Option<f32>,
    /// [`bevy_carnage::Fracture::bent`]. Non-zero means a greenstick and nothing else.
    bent: Vec3,
}

/// One material per tissue, plus the raw interior every cut face takes. The contrast between the two
/// is the entire visual read: give the cap the skin material and a break reads as a disassembly.
#[derive(Resource)]
struct Mats {
    cortical: Handle<StandardMaterial>,
    trabecular: Handle<StandardMaterial>,
    soft: Handle<StandardMaterial>,
    interior: Handle<StandardMaterial>,
}

impl Mats {
    fn skin(&self, t: TissueClass) -> Handle<StandardMaterial> {
        match t {
            TissueClass::Cortical => self.cortical.clone(),
            TissueClass::Trabecular => self.trabecular.clone(),
            TissueClass::Soft => self.soft.clone(),
        }
    }
}

fn mode_name(m: LoadingMode) -> &'static str {
    match m {
        LoadingMode::Torsion => "Torsion",
        LoadingMode::Bending => "Bending",
        LoadingMode::Axial => "Axial",
        LoadingMode::DirectHighEnergy => "DirectHighEnergy",
    }
}

/// What that mode is supposed to look like, so the screen states the claim the geometry is making.
fn mode_says(m: LoadingMode) -> &'static str {
    match m {
        LoadingMode::Torsion => "a helix - tension peaks in a plane 45 deg to the long axis",
        LoadingMode::Bending => "a butterfly wedge - transverse on the tension face, then oblique",
        LoadingMode::Axial => "transverse - it fails across its narrowest cross-section",
        LoadingMode::DirectHighEnergy => "comminution - no plane is preferred at all",
    }
}

fn tissue_name(t: TissueClass) -> &'static str {
    match t {
        TissueClass::Cortical => "Cortical",
        TissueClass::Trabecular => "Trabecular",
        TissueClass::Soft => "Soft",
    }
}

fn tissue_says(t: TissueClass) -> &'static str {
    match t {
        TissueClass::Cortical => "fails at ~2 pct strain: long sharp slivers",
        TissueClass::Trabecular => "tolerates ~30 pct strain: it crushes, never shatters",
        TissueClass::Soft => "tears - what every bake before the enum existed behaved as",
    }
}

fn next_tissue(t: TissueClass) -> TissueClass {
    match t {
        TissueClass::Cortical => TissueClass::Trabecular,
        TissueClass::Trabecular => TissueClass::Soft,
        TissueClass::Soft => TissueClass::Cortical,
    }
}

/// The cut for the dials as they stand. **One policy with a parameter** — there is no second entry
/// point and no branch selecting between two cut engines.
fn cut(d: &Dials) -> CutSettings {
    CutSettings {
        fault: FaultPolicy::Morphology {
            mode: d.mode,
            tissue: d.tissue,
            axis: LONG_AXIS,
            torque: TORQUE,
            impulse: d.impulse,
        },
        tissue: d.tissue,
        ..CutSettings::new(TARGET, MIN_FRACTION, SEED ^ d.takes.wrapping_mul(2_654_435_761))
    }
}

/// Longest over shortest bounding extent of one cell. `None` when the cell is flat in some axis,
/// because a ratio against zero is not a measurement.
///
/// The same measure `tests/fault_modes.rs::cortical_bone_produces_a_splinter` asserts against, so
/// the number on screen is the number the test pins.
fn aspect_of(cell: &ProxyCell) -> Option<f32> {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for p in cell.points() {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let e = hi - lo;
    let long = e.x.max(e.y).max(e.z);
    let short = e.x.min(e.y).min(e.z);
    (short > 1.0e-6 && long.is_finite()).then_some(long / short)
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage - fault modes".into(),
                // The one web line, inert on native: the browser build draws into the page's canvas.
                canvas: Some("#carnage-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        .init_resource::<Dials>()
        .init_resource::<Report>()
        .add_systems(Startup, setup)
        .add_systems(Update, (drive, hud, draw_axis_and_bend))
        .run();
}

/// The camera, the furniture, the materials and the legend. Nothing here changes afterwards.
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(1.05, 0.92, 1.35).looking_at(ORIGIN, Vec3::Y),
    ));
    // **The fill light is not a nicety.** With one directional light every surface turned away from
    // it renders at zero, and a cut face at zero against a dark background does not read as a
    // shadowed face - it reads as a hole, and the fragment looks like an open shell.
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

    let mut skin = |r: f32, g: f32, b: f32, roughness: f32| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            perceptual_roughness: roughness,
            ..default()
        })
    };
    // Bone white, spongy tan, and flesh. The skin colour follows `T`, so the tissue is legible
    // before reading a word of the status line.
    commands.insert_resource(Mats {
        cortical: skin(0.86, 0.84, 0.74, 0.55),
        trabecular: skin(0.78, 0.66, 0.48, 0.85),
        soft: skin(0.58, 0.33, 0.31, 0.75),
        interior: skin(0.46, 0.07, 0.07, 0.42),
    });

    commands.spawn((
        Text::new(
            "1 torsion   2 bending   3 axial   4 direct high energy\n\
             T cycles tissue     [ / ]  impulse     R re-break\n\
             Same subject, same seed, four silhouettes. Cortical bone splits along its own\n\
             grain and yields slivers; trabecular bone crushes and is clamped to three pieces\n\
             however hard it is hit. Below the greenstick impulse the bone bends instead of\n\
             breaking: one fragment, no fault, a permanent bow. An outcome, not a mode.",
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
    for (key, mode) in [
        (KeyCode::Digit1, LoadingMode::Torsion),
        (KeyCode::Digit2, LoadingMode::Bending),
        (KeyCode::Digit3, LoadingMode::Axial),
        (KeyCode::Digit4, LoadingMode::DirectHighEnergy),
    ] {
        if keys.just_pressed(key) && dials.mode != mode {
            dials.mode = mode;
            dials.dirty = true;
        }
    }
    if keys.just_pressed(KeyCode::KeyT) {
        dials.tissue = next_tissue(dials.tissue);
        dials.dirty = true;
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        dials.impulse = (dials.impulse - IMPULSE_STEP).max(0.0);
        dials.dirty = true;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        dials.impulse = (dials.impulse + IMPULSE_STEP).min(IMPULSE_MAX);
        dials.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        dials.takes = dials.takes.wrapping_add(1);
        dials.dirty = true;
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

/// Fracture the shaft and spawn every leaf, pushed out along its own centroid.
fn break_it(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &Mats,
    d: &Dials,
) -> Report {
    let (mesh, proxy) = subject();
    let bake = fracture_mesh(&[(&mesh, Mat4::IDENTITY)], &proxy, &cut(d));
    let bent = bake.bent;
    let leaves = bake.into_leaves();
    let fragments = leaves.len();

    // The biggest shard, and its aspect ratio. Largest-volume rather than largest-box, because the
    // cell is the shape and the box is only a bound.
    let mut biggest = f32::NEG_INFINITY;
    let mut aspect = None;
    for f in &leaves {
        let v = f.cell.volume();
        if v > biggest {
            biggest = v;
            aspect = aspect_of(&f.cell);
        }
    }
    info!(
        "{:?} / {:?} at impulse {:.1}: {} fragment(s), biggest aspect {:?}, bent {:?}",
        d.mode, d.tissue, d.impulse, fragments, aspect, bent
    );

    let skin = mats.skin(d.tissue);
    for f in leaves {
        let at = ORIGIN + f.center_local * (1.0 + EXPLODE);
        let shard = commands
            .spawn((Shard, Transform::from_translation(at), Visibility::default()))
            .id();
        // Both meshes are already recentred on the fragment's own centre, so this is a translation
        // and not an orbit.
        commands.entity(shard).with_children(|parent| {
            if let Some(outer) = f.outer {
                parent.spawn((Mesh3d(meshes.add(outer)), MeshMaterial3d(skin.clone())));
            }
            if let Some(cap) = f.cap {
                parent.spawn((Mesh3d(meshes.add(cap)), MeshMaterial3d(mats.interior.clone())));
            }
        });
    }
    Report { fragments, aspect, bent }
}

/// The long axis, always; the residual bow, only when there is one.
///
/// **`bent` is exactly zero for a subject that parted** — the crate promises that rather than a small
/// number, so this is an equality test and not a threshold.
fn draw_axis_and_bend(mut gizmos: Gizmos, report: Res<Report>) {
    let a = ORIGIN - LONG_AXIS * HALF.y * 1.35;
    let b = ORIGIN + LONG_AXIS * HALF.y * 1.35;
    gizmos.line(a, b, Color::srgba(0.45, 0.55, 0.70, 0.55));
    if report.bent == Vec3::ZERO {
        return;
    }
    let bow = report.bent * BOW;
    // The bowed centre line: pinned at both ends, displaced most in the middle, which is what a
    // greenstick is — the tension cortex opened, the far cortex held.
    let strip = (0..=16).map(|i| {
        let t = i as f32 / 16.0;
        let along = a.lerp(b, t);
        along + bow * (t * std::f32::consts::PI).sin()
    });
    gizmos.linestrip(strip, Color::srgb(1.0, 0.62, 0.20));
    gizmos.arrow(ORIGIN, ORIGIN + report.bent.normalize_or_zero() * 0.32, Color::srgb(1.0, 0.45, 0.12));
}

/// Keep the status line current: the dials, what the bake produced, and the greenstick call-out.
fn hud(dials: Res<Dials>, report: Res<Report>, mut line: Query<&mut Text, With<HudStatus>>) {
    let threshold = FractureSettings::default().greenstick_impulse;
    let aspect = match report.aspect {
        Some(a) => format!("{a:.1}x its own thickness"),
        None => "n/a (degenerate cell)".to_string(),
    };
    let head = format!(
        "mode {} - {}\ntissue {} - {}\nimpulse {:.1} N.s   (greenstick below {:.1})   seed take #{}",
        mode_name(dials.mode),
        mode_says(dials.mode),
        tissue_name(dials.tissue),
        tissue_says(dials.tissue),
        dials.impulse,
        threshold,
        dials.takes,
    );
    let body = if report.bent == Vec3::ZERO {
        let clamp = if dials.tissue == TissueClass::Trabecular {
            format!("   (trabecular ceiling {TRABECULAR_MAX_PIECES})")
        } else {
            String::new()
        };
        format!(
            "{} fragments{}   biggest shard {}",
            report.fragments, clamp, aspect
        )
    } else {
        format!(
            "GREENSTICK - an outcome, not a mode: {} fragment, no fault at all.\n\
             residual bend {:.2} along ({:.2}, {:.2}, {:.2}) - the tension cortex opened, the far \
             cortex held.",
            report.fragments,
            report.bent.length(),
            report.bent.x,
            report.bent.y,
            report.bent.z,
        )
    };
    let text = format!("{head}\n{body}");
    for mut t in &mut line {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}
