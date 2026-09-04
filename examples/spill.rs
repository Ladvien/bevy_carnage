//! **Press Space: guts fall out, hang from the mesentery, and tear loose.**
//!
//! The crate never spawns, so this example does all of it — the entity, the material, the mesh handle
//! and the tick where the tube is rebuilt. What the crate supplies is the strand, the tether and the
//! `Mesh`.
//!
//! Green lines are intact mesenteric links. A line that vanishes is a link whose strain passed
//! `tear_strain`, and it never comes back: the tear is monotone. Every other strand is tethered every
//! fourth node, which is about four nodes of hanging weight per link, and it holds; the ones in
//! between are tethered every twelfth, which is eleven, and a sheet of peritoneum does not take that,
//! so they part and drop.
//!
//! `cargo run -p bevy_viscera --example spill`

use bevy::color::palettes::css;
use bevy::prelude::*;
use bevy_carnage::viscera::{spill, tube_mesh, Mesentery, Strand, ViscSettings, VisceraPlugin, VisceraSystems};

/// Sides on the swept tube. Eight is 384 triangles over a 25-node strand.
const SIDES: u32 = 8;
/// Where the wound is, and which way it opens.
const WOUND: Vec3 = Vec3::new(0.0, 1.35, 0.12);
const EXIT: Vec3 = Vec3::new(0.15, 0.25, 1.0);

/// Marks an entity whose `Mesh3d` is rebuilt from its `Strand` every fixed tick.
#[derive(Component)]
struct Gut;

/// Marks the legend text.
#[derive(Component)]
struct Legend;

/// The palette and the seed counter, so each press spills a different set.
#[derive(Resource)]
struct Spawner {
    gut: Handle<StandardMaterial>,
    presses: u32,
}

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VisceraPlugin))
        // The crate is integer-tick at 60 Hz and never reads the clock; Bevy's default is 64.
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .add_systems(Startup, setup)
        .add_systems(Update, (spill_on_space, draw_tethers, update_legend))
        .add_systems(FixedUpdate, rebuild_tubes.after(VisceraSystems))
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    settings: Res<ViscSettings>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.5, 3.2).looking_at(Vec3::new(0.0, 0.85, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 8_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(3.0, 6.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(12.0, 12.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.10, 0.10, 0.12),
            perceptual_roughness: 0.95,
            ..default()
        })),
    ));

    // A blocked-out torso for the guts to leave. Geometry in code, no asset files.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.46, 0.72, 0.26))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.22, 0.20, 0.22),
            perceptual_roughness: 0.8,
            ..default()
        })),
        Transform::from_xyz(0.0, 1.42, 0.0),
    ));

    let gut = materials.add(StandardMaterial {
        base_color: Color::srgb(0.52, 0.16, 0.15),
        perceptual_roughness: 0.28,
        ..default()
    });
    // Spill once immediately, through the same function the key handler calls, so the window opens on
    // something moving rather than on a bare torso.
    spill_guts(&mut commands, &mut meshes, &gut, &settings, 1);
    commands.insert_resource(Spawner { gut, presses: 1 });

    commands.spawn((
        Text::new("SPACE  spill a fresh set of guts\nR      clear\n"),
        Node { position_type: PositionType::Absolute, top: px(12), left: px(12), ..default() },
        Legend,
    ));
}

fn spill_on_space(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut spawner: ResMut<Spawner>,
    settings: Res<ViscSettings>,
    existing: Query<Entity, With<Gut>>,
) {
    let clear = keys.just_pressed(KeyCode::KeyR);
    let respill = keys.just_pressed(KeyCode::Space);
    if !clear && !respill {
        return;
    }
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    if !respill {
        return;
    }
    spawner.presses = spawner.presses.wrapping_add(1);
    let presses = spawner.presses;
    let gut = spawner.gut.clone();
    spill_guts(&mut commands, &mut meshes, &gut, &settings, presses);
}

/// The one place guts are built. Called at startup and on every press, so there is one path.
fn spill_guts(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    gut: &Handle<StandardMaterial>,
    settings: &ViscSettings,
    presses: u32,
) {
    let seed = 0x5EED_0000u32.wrapping_add(presses);
    for (i, strand) in spill(WOUND, EXIT, 6, seed, settings).into_iter().enumerate() {
        // Every other strand is tethered densely enough to hold; the rest are not, and part.
        let stride = if i % 2 == 0 { 4 } else { 12 };
        let anchors: Vec<(u32, Vec3)> = strand
            .nodes()
            .iter()
            .enumerate()
            .filter(|(n, _)| n % stride == 0)
            .map(|(n, p)| (n as u32, *p))
            .collect();
        let torn = vec![false; anchors.len()];

        commands.spawn((
            Mesh3d(meshes.add(tube_mesh(&strand, SIDES))),
            MeshMaterial3d(gut.clone()),
            Mesentery { anchors, torn, ..default() },
            strand,
            Gut,
        ));
    }
}

/// Rebuild each tube from the nodes the solver just moved. This is the caller's job by design.
fn rebuild_tubes(mut meshes: ResMut<Assets<Mesh>>, guts: Query<(&Strand, &Mesh3d), With<Gut>>) {
    for (strand, handle) in &guts {
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            *mesh = tube_mesh(strand, SIDES);
        }
    }
}

/// One line per intact link. A link that tore simply stops being drawn.
fn draw_tethers(mut gizmos: Gizmos, guts: Query<(&Strand, &Mesentery), With<Gut>>) {
    for (strand, mesentery) in &guts {
        for (slot, (node, point)) in mesentery.anchors.iter().enumerate() {
            if mesentery.torn.get(slot).copied().unwrap_or(true) {
                continue;
            }
            let Some(at) = strand.nodes().get(*node as usize) else {
                continue;
            };
            gizmos.line(*point, *at, css::LIME);
        }
    }
}

fn update_legend(
    guts: Query<&Mesentery, With<Gut>>,
    mut legend: Query<&mut Text, With<Legend>>,
    settings: Res<ViscSettings>,
) {
    let links: usize = guts.iter().map(|m| m.torn.len()).sum();
    let torn: usize = guts.iter().flat_map(|m| m.torn.iter()).filter(|t| **t).count();
    for mut text in &mut legend {
        text.0 = format!(
            "SPACE  spill a fresh set of guts\nR      clear\n\n\
             strands   {}\nmesentery {torn}/{links} links torn (never heals)\n\
             solver    {} substeps x {} iterations, gravity {}",
            guts.iter().len(),
            settings.substeps,
            settings.iterations,
            settings.gravity,
        );
    }
}
