//! **The web demo: guts spill with weight, tether, and tear.**
//!
//! `Space` spills a fresh set out of a blocked-out torso, `G` (held) takes hold of the loose strand's
//! free end and drags it, `T` cuts every mesenteric link, `R` resets to the opening seed.
//!
//! Green lines are intact mesenteric links, a red cross marks one that parted, and it never comes
//! back — the tear is **monotone**, so it cannot heal and the solver cannot oscillate. Measured on
//! this spill: strands tethered every fourth node carry the hanging weight (0/6 links torn each);
//! the ones tethered every twelfth cannot (2/2 torn) and drop. `T` takes the rest at once.
//!
//! **The grip is the crate's own tether, moved by the caller.** [`Mesentery`] anchors are world points
//! the caller owns, so a hand is one more pin — pushed on *at* the node it holds, since the pin's rest
//! length is zero, then walked at a fixed speed per tick. What a visitor feels when the slack runs out
//! is `compliance_stretch = 1e-6`: the far end starts moving the instant the near end does.
//!
//! Nothing here reads a clock — the solver is integer-tick at 60 Hz and the tick shown is
//! `FixedUpdate` calls, so `R` replays the same digest at the same tick, at any frame rate.
//!
//! Formulation: Deul, Charrier & Bender, *Direct position-based solver for stiff rods*
//! (`doi:10.1111/cgf.13326`); Bergou et al., *Discrete elastic rods* (`doi:10.1145/1399504.1360662`).
//!
//! `cargo run -p bevy_viscera --example viscera_spill`

use bevy::color::palettes::css;
use bevy::prelude::*;
use bevy_carnage::viscera::{
    spill, tube_mesh, Mesentery, Strand, ViscSettings, VisceraPlugin, VisceraSystems,
    DEFAULT_TEAR_STRAIN, FIXED_DT, SPILL_RADIUS, SPILL_SEGMENTS,
};

/// Sides on the swept tube. Eight is 384 triangles over a 25-node strand.
const SIDES: u32 = 8;
/// Where the wound is, and which way it opens.
const WOUND: Vec3 = Vec3::new(0.0, 1.35, 0.12);
/// Downward as well as outward: a strand leaving horizontally must sag between its tethers, and with
/// `compliance_stretch = 1e-6` there is no slack to sag into, so the load goes into the pins. Measured:
/// at `+0.25` it is 18/24 links torn inside a second; at `-0.3` it is the 6 sparse ones.
const EXIT: Vec3 = Vec3::new(0.15, -0.3, 1.0);
/// Strands per spill. `ViscSettings::max_strands` clamps this anyway; six fits the frame.
const STRANDS: u32 = 6;
/// The seed the demo opens on, and the one `R` returns to.
const SEED: u32 = 0x5EED_0001;
/// The node a hand takes hold of: the free end of the strand.
const HAND_NODE: u32 = SPILL_SEGMENTS;
/// How fast a hand may travel, m/s. A compliant pin never reaches zero residual, so the node lags a
/// moving anchor by roughly four times the per-tick step — ~3.5 mm here, far inside the grip's
/// threshold, and slow enough to watch the slack come out of the strand.
const GRAB_SPEED: f32 = 0.30;
/// Where a held hand drags the free end to: across the floor, in front of the torso.
const GRAB_GOAL: Vec3 = Vec3::new(0.95, SPILL_RADIUS, 1.5);
/// The strain at which the **grip** slips, against the membrane's `DEFAULT_TEAR_STRAIN` of 0.35.
///
/// A hand is not a double fold of peritoneum and does not let go at 12 mm: the same pin constraint,
/// with the one threshold `Mesentery` exposes set for what is holding it — 8 rest lengths, 28 cm of
/// slip. It is also why the grabbed strand carries no anatomical fan. One entity has one threshold, so
/// a strand cannot be a hand and a membrane at once, and measured, a strand held by both parts at both.
const GRIP_TEAR_STRAIN: f32 = 8.0;

/// One spilled strand, and its spawn index — the canonical order the digest is folded in, since ECS
/// query order is not stable across runs.
#[derive(Component)]
struct Gut {
    index: u32,
}

/// The one strand a hand can take hold of.
#[derive(Component)]
struct Grabbable;

/// Marks the legend text.
#[derive(Component)]
struct Legend;

/// The palette, the live seed, and the tick count since the last spill.
#[derive(Resource)]
struct Show {
    gut: Handle<StandardMaterial>,
    seed: u32,
    tick: u32,
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "viscera_spill".into(),
                    canvas: Some("#carnage-canvas".into()),
                    ..default()
                }),
                ..default()
            }),
            VisceraPlugin,
        ))
        // The crate is integer-tick at 60 Hz and never reads the clock; Bevy's default is 64.
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .add_systems(Startup, setup)
        .add_systems(Update, (spill_keys, tear_mesentery, draw_tethers, update_legend))
        .add_systems(
            FixedUpdate,
            (
                // The anchors move before the solve that reads them, as the crate's set documents.
                drive_hand.before(VisceraSystems),
                (count_ticks, rebuild_tubes).after(VisceraSystems),
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    settings: Res<ViscSettings>,
) {
    // Exactly one `Camera3d`: a second one makes every `Single<.., With<Camera3d>>` skip its system,
    // and `AmbientLight` is a per-camera component in 0.19, so it rides this entity.
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.9, 1.4, 2.9).looking_at(Vec3::new(0.15, 0.7, 0.3), Vec3::Y),
        AmbientLight { brightness: 260.0, ..default() },
    ));
    commands.spawn((
        DirectionalLight { illuminance: 9_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(3.0, 6.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(12.0, 12.0))),
        MeshMaterial3d(matte(&mut materials, [0.10, 0.10, 0.12], 0.95)),
    ));
    // A blocked-out torso for the guts to leave. Geometry in code, no asset files.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.46, 0.72, 0.26))),
        MeshMaterial3d(matte(&mut materials, [0.22, 0.20, 0.22], 0.8)),
        Transform::from_xyz(0.0, 1.42, 0.0),
    ));

    let gut = matte(&mut materials, [0.52, 0.16, 0.15], 0.28);
    // Spill once through the same function every key press calls, so there is one path and the page
    // opens on something moving rather than on a bare torso.
    spill_guts(&mut commands, &mut meshes, &gut, &settings, SEED);
    commands.insert_resource(Show { gut, seed: SEED, tick: 0 });

    commands.spawn((
        Text::new("spilling…"),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        Node { position_type: PositionType::Absolute, top: px(12), left: px(12), ..default() },
        Legend,
    ));
}

/// One rough, unlit-looking surface. Every material in the demo is this, so the code is not four
/// copies of the same struct literal.
fn matte(
    materials: &mut Assets<StandardMaterial>,
    srgb: [f32; 3],
    roughness: f32,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: Color::srgb(srgb[0], srgb[1], srgb[2]),
        perceptual_roughness: roughness,
        ..default()
    })
}

/// The one place guts are built.
fn spill_guts(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    gut: &Handle<StandardMaterial>,
    settings: &ViscSettings,
    seed: u32,
) {
    let strands = spill(WOUND, EXIT, STRANDS, seed, settings);
    // The last strand spilled is the one a hand can hold, and it is the last so the `i % 2` pattern
    // below reads the same for every strand that has a fan.
    let grabbed = strands.len().saturating_sub(1);
    for (i, strand) in strands.into_iter().enumerate() {
        // Every other fanned strand is tethered densely enough to hold; the rest are not, and part.
        // No anatomical anchor sits on `HAND_NODE`, which is what keeps a grip distinguishable from a
        // fold of peritoneum — in the counts, in the gizmos, and under `T`.
        let stride = if i % 2 == 0 { 4 } else { 12 };
        let anchors: Vec<(u32, Vec3)> = strand
            .nodes()
            .iter()
            .enumerate()
            .filter(|(n, _)| i != grabbed && n % stride == 0 && (*n as u32) < HAND_NODE)
            .map(|(n, p)| (n as u32, *p))
            .collect();
        let tear_strain = if i == grabbed { GRIP_TEAR_STRAIN } else { DEFAULT_TEAR_STRAIN };
        let torn = vec![false; anchors.len()];

        let mut gut_entity = commands.spawn((
            Mesh3d(meshes.add(tube_mesh(&strand, SIDES))),
            MeshMaterial3d(gut.clone()),
            Mesentery { anchors, torn, tear_strain },
            strand,
            Gut { index: i as u32 },
        ));
        if i == grabbed {
            gut_entity.insert(Grabbable);
        }
    }
}

/// `Space` spills a fresh set; `R` returns to the opening seed, so the digest replays.
fn spill_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut show: ResMut<Show>,
    settings: Res<ViscSettings>,
    existing: Query<Entity, With<Gut>>,
) {
    let reset = keys.just_pressed(KeyCode::KeyR);
    let respill = keys.just_pressed(KeyCode::Space);
    if !reset && !respill {
        return;
    }
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    show.seed = if reset { SEED } else { show.seed.wrapping_add(1) };
    show.tick = 0;
    let (seed, gut) = (show.seed, show.gut.clone());
    spill_guts(&mut commands, &mut meshes, &gut, &settings, seed);
}

/// `T` cuts every mesenteric link at once. A cut is a caller's scalpel rather than a strain failure,
/// and it obeys the same rule the solver's own tear does: the flag is set, never cleared.
fn tear_mesentery(keys: Res<ButtonInput<KeyCode>>, mut tethers: Query<&mut Mesentery, With<Gut>>) {
    if !keys.just_pressed(KeyCode::KeyT) {
        return;
    }
    for mut tether in &mut tethers {
        let count = tether.anchors.len();
        if tether.torn.len() < count {
            tether.torn.resize(count, false);
        }
        let Mesentery { anchors, torn, .. } = &mut *tether;
        for ((node, _), flag) in anchors.iter().zip(torn.iter_mut()) {
            // The hand survives the cut: it is a grip, not a membrane.
            if *node != HAND_NODE {
                *flag = true;
            }
        }
    }
}

/// Hold `G`: push a pin onto the free end and walk it across the floor. Release: take it off again.
fn drive_hand(
    keys: Res<ButtonInput<KeyCode>>,
    mut grabbable: Query<(&Strand, &mut Mesentery), With<Grabbable>>,
) {
    // `pressed`, not `just_pressed`: `FixedUpdate` can run zero or several times per input frame, so an
    // edge read here is an edge that can be missed or seen twice.
    let want = keys.pressed(KeyCode::KeyG);
    for (strand, mut tether) in &mut grabbable {
        let hand = tether.anchors.last().copied().filter(|(node, _)| *node == HAND_NODE);
        match (want, hand) {
            (true, None) => {
                // The pin's rest length is zero, so it is born ON the node. Anywhere else is already
                // past `tear_strain` at the first projection, and the grip would part on contact.
                if let Some(at) = strand.nodes().get(HAND_NODE as usize) {
                    tether.anchors.push((HAND_NODE, *at));
                    tether.torn.push(false);
                }
            }
            (true, Some((_, point))) => {
                let step = (GRAB_GOAL - point).clamp_length_max(GRAB_SPEED * FIXED_DT);
                if let Some(hand) = tether.anchors.last_mut() {
                    hand.1 = point + step;
                }
            }
            (false, Some(_)) => {
                tether.anchors.pop();
                // `truncate`, so the flags stay parallel and no earlier tear can be cleared.
                let count = tether.anchors.len();
                tether.torn.truncate(count);
            }
            (false, None) => {}
        }
    }
}

/// Solver ticks since the last spill. Integer, never a clock: `Instant` panics in a browser.
fn count_ticks(mut show: ResMut<Show>) {
    show.tick = show.tick.saturating_add(1);
}

/// Rebuild each tube from the nodes the solver just moved. This is the caller's job by design.
fn rebuild_tubes(mut meshes: ResMut<Assets<Mesh>>, guts: Query<(&Strand, &Mesh3d), With<Gut>>) {
    for (strand, handle) in &guts {
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            *mesh = tube_mesh(strand, SIDES);
        }
    }
}

/// Green line per intact link, red cross where one parted, yellow line for the hand.
fn draw_tethers(mut gizmos: Gizmos, guts: Query<(&Strand, &Mesentery), With<Gut>>) {
    for (strand, tether) in &guts {
        for (slot, (node, point)) in tether.anchors.iter().enumerate() {
            let parted = tether.torn.get(slot).copied().unwrap_or(true);
            let Some(at) = strand.nodes().get(*node as usize) else {
                continue;
            };
            match (parted, *node == HAND_NODE) {
                // A parted link is drawn as the mark it left, not as a line it no longer exerts.
                (true, _) => gizmos.cross(Isometry3d::from_translation(*point), 0.03, css::RED),
                (false, true) => {
                    gizmos.line(*point, *at, css::YELLOW);
                    gizmos.cross(Isometry3d::from_translation(*point), 0.04, css::YELLOW);
                }
                (false, false) => gizmos.line(*point, *at, css::LIME),
            }
        }
    }
}

fn update_legend(
    guts: Query<(&Gut, &Strand, &Mesentery)>,
    grip: Query<&Mesentery, With<Grabbable>>,
    show: Res<Show>,
    settings: Res<ViscSettings>,
    mut legend: Query<&mut Text, With<Legend>>,
) {
    let mut nodes = 0usize;
    let mut links = 0usize;
    let mut torn = 0usize;
    // Spawn index, not query order: ECS iteration order is not stable across runs, and a digest folded
    // in an unstable order would not be the reproducible number this demo is claiming.
    let mut digests: Vec<(u32, u64)> = Vec::with_capacity(STRANDS as usize);
    for (gut, strand, tether) in &guts {
        nodes += strand.nodes().len();
        // A grip is not a mesenteric link and is not counted as one.
        for (slot, _) in tether.anchors.iter().enumerate().filter(|(_, (n, _))| *n != HAND_NODE) {
            links += 1;
            torn += usize::from(tether.torn.get(slot).copied().unwrap_or(false));
        }
        digests.push((gut.index, strand.digest()));
    }
    digests.sort_unstable_by_key(|(index, _)| *index);
    let digest = digests
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |acc, (_, d)| (acc ^ d).wrapping_mul(0x1000_0000_01b3));

    let held = grip.iter().next().and_then(|m| {
        let slot = m.anchors.len().checked_sub(1)?;
        let (node, _) = m.anchors.get(slot).copied()?;
        (node == HAND_NODE).then(|| m.torn.get(slot).copied().unwrap_or(false))
    });
    let grip = match held {
        None => "free — hold G to take the loose strand by its free end".to_string(),
        Some(false) => format!("holding node {HAND_NODE}, dragging at {GRAB_SPEED} m/s"),
        Some(true) => "the grip's pin parted — release G and take hold again".to_string(),
    };

    for mut text in &mut legend {
        text.0 = format!(
            "SPACE  spill        G  hold: grab and drag the free end\n\
             T      tear the mesentery        R  reset (same seed)\n\n\
             strands    {} · nodes {nodes}\n\
             mesentery  {torn}/{links} links torn — monotone, a tear never heals\n\
             grip       {grip}\n\
             tick       {} · digest {digest:#018x}\n\
             solver     {} substeps × {} iterations, gravity {}, seed {:#010x}",
            digests.len(),
            show.tick,
            settings.substeps,
            settings.iterations,
            settings.gravity,
            show.seed,
        );
    }
}
