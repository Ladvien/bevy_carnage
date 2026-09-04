//! **A wall of cells with a weak seam, broken by a keypress.** Six by four unit cubes bonded by
//! their shared faces; the faces across one column are a tenth the area of the rest. `Space`
//! strikes the bottom-left cell, harder each press; the wall comes apart where its modes say, and
//! `R` puts it back. The point is that the seam gives first, whichever corner is struck.
//!
//! ```sh
//! cargo run --example fracture_modes
//! ```
//!
//! Also the wasm demo `fracture_modes` on the monorepo's demo site.

use bevy::prelude::*;
use bevy_carnage::fracture_modes::{CellGraph, FractureModeCache, FractureModesPlugin, Impact, ModeSettings};

const COLS: usize = 6;
const ROWS: usize = 4;
/// The column whose right-hand faces are the weak seam.
const SEAM: usize = 2;
const KEY: u64 = 1;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "bevy_fracture_modes".into(), ..default() }),
            ..default()
        }))
        .add_plugins(FractureModesPlugin)
        .insert_resource(ModeSettings { k: 4, ..Default::default() })
        .init_resource::<Wall>()
        .add_systems(Startup, setup)
        .add_systems(Update, (strike, fly, orbit))
        .run();
}

#[derive(Component)]
struct Cell(usize);

#[derive(Component)]
struct Orbit;

/// What the wall is doing: at rest, or flying apart along a partition.
#[derive(Resource, Default)]
struct Wall {
    /// Per-cell group after the last strike; empty at rest.
    group_of: Vec<usize>,
    /// Per-group velocity.
    velocity: Vec<Vec3>,
    /// Presses so far, which sets the impulse.
    presses: u32,
}

fn wall_graph() -> CellGraph {
    let n = COLS * ROWS;
    let centers: Vec<Vec3> = (0..n).map(|i| cell_center(i)).collect();
    let mut g = CellGraph::new(vec![1.0; n], centers);
    for r in 0..ROWS {
        for c in 0..COLS {
            let i = r * COLS + c;
            if c + 1 < COLS {
                let area = if c == SEAM { 0.1 } else { 1.0 };
                g.bond(i, i + 1, area, (cell_center(i) + cell_center(i + 1)) * 0.5, Vec3::X);
            }
            if r + 1 < ROWS {
                g.bond(i, i + COLS, 1.0, (cell_center(i) + cell_center(i + COLS)) * 0.5, Vec3::Y);
            }
        }
    }
    g
}

fn cell_center(i: usize) -> Vec3 {
    let (r, c) = (i / COLS, i % COLS);
    Vec3::new(c as f32 - (COLS as f32 - 1.0) * 0.5, r as f32 + 0.5, 0.0)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: ResMut<FractureModeCache>,
    settings: Res<ModeSettings>,
) {
    match cache.bake(KEY, &wall_graph(), &settings) {
        Ok(set) => info!("baked {} modes over {} cells", set.modes.len(), set.cells),
        Err(e) => error!("bake refused: {e:?}"),
    }
    let cube = meshes.add(Cuboid::new(0.96, 0.96, 0.96));
    let stone = materials.add(StandardMaterial { base_color: Color::srgb(0.72, 0.70, 0.66), ..default() });
    let seam = materials.add(StandardMaterial { base_color: Color::srgb(0.82, 0.58, 0.50), ..default() });
    for i in 0..COLS * ROWS {
        let mat = if i % COLS == SEAM || i % COLS == SEAM + 1 { seam.clone() } else { stone.clone() };
        commands.spawn((Mesh3d(cube.clone()), MeshMaterial3d(mat), Transform::from_translation(cell_center(i)), Cell(i)));
    }
    commands.spawn((
        DirectionalLight { illuminance: 10_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(3.0, 6.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 3.0, 9.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y), Orbit));
}

fn strike(
    keys: Res<ButtonInput<KeyCode>>,
    cache: Res<FractureModeCache>,
    mut wall: ResMut<Wall>,
    mut cells: Query<(&Cell, &mut Transform)>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        wall.group_of.clear();
        wall.velocity.clear();
        wall.presses = 0;
        for (c, mut tf) in &mut cells {
            tf.translation = cell_center(c.0);
        }
        return;
    }
    if !keys.just_pressed(KeyCode::Space) {
        return;
    }
    let Some(set) = cache.get(KEY) else { return };
    wall.presses += 1;
    let magnitude = 0.002 * 2f32.powi(wall.presses as i32);
    let p = set.partition(&Impact { cell: 0, magnitude });
    info!("impulse {magnitude:.4}: {} piece(s), {} broken face(s)", p.fragment_count(), p.broken.len());
    // Each group flies away from the strike, faster the smaller it is.
    let strike_at = cell_center(0);
    wall.velocity = p
        .groups
        .iter()
        .map(|g| {
            let centroid = g.iter().map(|&i| cell_center(i)).sum::<Vec3>() / g.len().max(1) as f32;
            let away = (centroid - strike_at).normalize_or_zero() + Vec3::Y * 0.3;
            away * (2.0 / g.len() as f32).clamp(0.2, 2.0)
        })
        .collect();
    wall.group_of = p.group_of;
}

fn fly(time: Res<Time>, wall: Res<Wall>, mut cells: Query<(&Cell, &mut Transform)>) {
    if wall.group_of.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    for (c, mut tf) in &mut cells {
        let Some(&g) = wall.group_of.get(c.0) else { continue };
        let Some(v) = wall.velocity.get(g) else { continue };
        tf.translation += *v * dt;
    }
}

fn orbit(time: Res<Time>, mut cams: Query<&mut Transform, With<Orbit>>) {
    let t = time.elapsed_secs() * 0.2;
    for mut tf in &mut cams {
        tf.translation = Vec3::new(t.sin() * 9.0, 3.0, t.cos() * 9.0);
        tf.look_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y);
    }
}
