//! **Guts stream out with the piece that took them.**
//!
//! `ribbons.rs` gives every flying chunk a thread of blood. This gives the ones that came out of the
//! *torso* something with a solver behind it: `bevy_viscera` strands, tethered to the chunk itself, so
//! a gut pays out behind the piece as it flies, sags under its own weight, drags across the floor, and
//! **tears loose** when the piece outruns what a fold of peritoneum can carry.
//!
//! ```text
//!   arrows / WASD   move the aim marker
//!   1               a projectile   2 a slash   3 a swept blade   4 a blast   5 a pull
//!   R               reset
//! ```
//!
//! # Why the tether follows the chunk, and why that is the caller's job
//!
//! [`Mesentery`] anchors are **world points the caller owns** — the crate never reads a transform and
//! never spawns. `bevy_viscera`'s own `viscera_spill` example walks one anchor to make a hand; this
//! walks a few of them to make a moving body. So the anchors are stored as offsets in the chunk's own
//! frame and rewritten every tick **before** [`VisceraSystems`], which is the ordering the crate's set
//! documents: the solve reads the anchors that were written this tick.
//!
//! Only the first few nodes are anchored. Anchoring the whole strand would make it a rigid attachment
//! that cannot stream; anchoring nothing would leave a gut where the body was standing. The base holds
//! and the rest is free, which is what "streaming" is.
//!
//! # Anatomy is the caller's business, which is the whole point of the split
//!
//! A gut comes out of an abdomen, so only fragments whose root cell is the torso get one —
//! [`body::part_of`] walks a fragment back to the body part it was cut from. An arm chunk trails blood
//! in `ribbons.rs` and nothing here, which is correct: `bevy_viscera` knows what a strand is and has
//! no opinion about where a subject keeps its bowel.
//!
//! Needs a GPU.
//!
//! Run: `cargo run --release -p bevy_carnage --example entrails`

use bevy::math::Isometry3d;
use bevy::prelude::*;
use bevy_viscera::{
    Mesentery, SPILL_SEGMENTS, Strand, ViscSettings, VisceraPlugin,
    VisceraSystems, spill, tube_mesh,
};

mod common;
use common::body::{self, Blow, BodyMaterials, Chunk, ORIGIN};
use common::light_and_floor;

/// The frontier this demo stands at — the finest, so a blow takes small pieces off and several of
/// them are torso.
const GRANULARITY: usize = body::GRANULARITIES.len() - 1;

/// Rounded, so the chunks read as torn rather than as cleaved ice. Matches `ribbons.rs`.
const SOFTEN: f32 = 0.5;

/// Sides on the swept tube. Eight is what `bevy_viscera`'s own demo uses: 384 triangles per strand.
const SIDES: u32 = 8;

/// Strands one torso chunk pulls out with it.
///
/// Two, not six: `viscera_spill` fills a frame with one spill, and this scene can have a dozen torso
/// chunks in the air at once. `ViscSettings::max_strands` is the crate's own ceiling and
/// [`MAX_GUTS`] is this demo's.
const STRANDS_PER_CHUNK: u32 = 2;

/// **The most strands alive at once.** Each is a solved polyline of 25 nodes *and* a tube mesh
/// rebuilt every tick, so this is a real budget rather than a formality. First come, first served —
/// nothing already streaming is ever evicted, which is `ribbons.rs`' rule for the same reason: a gut
/// that vanished mid-flight would read as a bug in the solver.
const MAX_GUTS: usize = 8;

/// How many nodes at the base are pinned to the chunk.
///
/// One anchor is a swivel and the strand spins about it; the whole strand is a rigid attachment that
/// cannot stream. Three holds the mouth of the wound and leaves twenty-two nodes free.
const PINNED_NODES: u32 = 3;

/// **The strain at which the gut parts from the piece it came out of**, against the membrane's
/// `DEFAULT_TEAR_STRAIN` of 0.35.
///
/// A gib leaves at metres per second and a compliant pin lags a moving anchor, so at 0.35 — twelve
/// millimetres, a fold of peritoneum — every tether parts on the first tick and nothing streams at
/// all. Measured in `capture_entrails`: 24 of 24 links torn, eight straight rods left in the air.
/// This pin is not a fold of peritoneum; it is the gut's own continuity with the piece of abdomen it
/// is still inside, so it gets the one threshold [`Mesentery`] exposes set for what is actually
/// holding it. Same reasoning `bevy_viscera`'s `viscera_spill` applies to a hand, which is not a
/// membrane either. At 8.0 the same clip parts none of them and the guts pay out.
const CHUNK_TEAR_STRAIN: f32 = 8.0;

/// Where the next blow lands, in subject-local space.
#[derive(Resource)]
struct Aim(Vec3);

/// **The aim ring's radius** — the radius of the opaque sphere it replaces.
const AIM_RADIUS: f32 = 0.05;

/// The aim ring's colour: the aim material's own `base_color`, so the marker did not change
/// appearance when it stopped being a mesh (`examples/common/body.rs`, `BodyMaterials::new`).
const AIM_COLOR: Color = Color::srgb(0.95, 0.85, 0.25);

/// Marks the line reporting the strand count and what has torn.
#[derive(Component)]
struct HudStatus;

/// **A strand and the chunk it left with.** The offsets are in the chunk's own frame, so a tumbling
/// piece carries the wound's mouth with it instead of dragging the gut through itself.
#[derive(Component)]
struct Entrail {
    chunk: Entity,
    offsets: Vec<Vec3>,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage — entrails (1-5 to hit, arrows to aim, R reset)".into(),
                resolution: (960u32, 680u32).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(VisceraPlugin)
        // **The crate is integer-tick at 60 Hz and never reads a clock**; Bevy's `Time<Fixed>`
        // defaults to 64, which would make the guts fall 7% fast. The crate's README states this.
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(Aim(Vec3::new(0.0, 0.10, 0.0)))
        .add_systems(Startup, (setup, aim_on_top))
        .add_systems(
            Update,
            (aim_marker, strike, spill_entrails, integrate, hud, draw_aim).chain(),
        )
        .add_systems(
            FixedUpdate,
            (
                // The anchors move before the solve that reads them, as `VisceraSystems` documents.
                carry_anchors.before(VisceraSystems),
                rebuild_tubes.after(VisceraSystems),
            ),
        )
        .run();
}

fn setup(world: &mut World) {
    let camera = Transform::from_xyz(2.25, 1.35, 2.95).looking_at(Vec3::new(0.0, 0.76, 0.0), Vec3::Y);
    world.spawn((Camera3d::default(), camera));
    light_and_floor(world);

    let baked = body::Baked::bake(world, SOFTEN, &[], &[GRANULARITY]);
    let materials = BodyMaterials::new(world);
    let damage = body::Damage::fresh(&baked, GRANULARITY);

    // The gut's own surface: darker and wetter than skin, rough enough not to read as plastic.
    let gut = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: Color::srgb(0.52, 0.16, 0.15),
        perceptual_roughness: 0.28,
        ..default()
    });
    world.insert_resource(GutMaterial(gut));

    world.insert_resource(baked);
    world.insert_resource(materials);
    world.insert_resource(damage);
    body::stand(world, GRANULARITY);

    world.spawn((
        Text::new(
            "a torso chunk pulls its guts out with it - they pay out, sag, drag, and tear loose\n\
             \n             arrows / WASD  aim\n             1 projectile   2 slash   3 blade   4 blast   5 pull\n             R reset",
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

/// The gut material, made once. A handle per strand would be a material per strand.
#[derive(Resource)]
struct GutMaterial(Handle<StandardMaterial>);

/// Marks a chunk that has already spilled, so it does not spill again every frame.
#[derive(Component)]
struct Spilled;

/// **Give every fresh torso chunk its guts**, up to [`MAX_GUTS`].
///
/// The chunk's own velocity is the exit direction, which is what makes the strand come out *behind*
/// the piece rather than in an authored direction: the wound faces the way the fragment left.
fn spill_entrails(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    gut: Res<GutMaterial>,
    settings: Res<ViscSettings>,
    baked: Res<body::Baked>,
    live: Query<(), With<Entrail>>,
    fresh: Query<(Entity, &Chunk, &Transform), Without<Spilled>>,
) {
    let mut budget = MAX_GUTS.saturating_sub(live.iter().count());
    for (entity, chunk, transform) in &fresh {
        // Marked either way: a chunk that was not a torso fragment, or arrived over the cap, must not
        // be reconsidered every frame — that is the difference between a budget and a queue.
        commands.entity(entity).insert(Spilled);
        if budget == 0 {
            continue;
        }
        let Some(id) = chunk.fragment else { continue }; // a plug, never a fragment
        if body::part_of(id, &baked.tree) != "torso" {
            continue;
        }

        let at = transform.translation;
        // Out along the throw, and downward: a strand leaving horizontally has no slack to sag into,
        // so the load goes straight into the pins and the membrane parts at once. `viscera_spill`
        // measured that and biases the same way.
        let exit = (chunk.velocity.normalize_or_zero() - Vec3::Y * 0.3).normalize_or_zero();
        // From the fragment's own position and the id it carries, never from its `Entity` — an entity
        // id is a slot index assigned by allocation order, which this workspace refuses to seed from.
        let seed = at.x.to_bits() ^ at.z.to_bits().rotate_left(16) ^ (id.index() as u32);
        for strand in spill(at, exit, STRANDS_PER_CHUNK, seed, &settings) {
            let anchors: Vec<(u32, Vec3)> = strand
                .nodes()
                .iter()
                .enumerate()
                .filter(|(n, _)| (*n as u32) < PINNED_NODES.min(SPILL_SEGMENTS))
                .map(|(n, p)| (n as u32, *p))
                .collect();
            let offsets: Vec<Vec3> = anchors
                .iter()
                .map(|(_, p)| transform.rotation.inverse() * (*p - transform.translation))
                .collect();
            let torn = vec![false; anchors.len()];
            commands.spawn((
                Mesh3d(meshes.add(tube_mesh(&strand, SIDES))),
                MeshMaterial3d(gut.0.clone()),
                Mesentery { anchors, torn, tear_strain: CHUNK_TEAR_STRAIN },
                strand,
                Entrail { chunk: entity, offsets },
            ));
            budget = budget.saturating_sub(1);
            if budget == 0 {
                break;
            }
        }
    }
}

/// **Carry each tether's anchors on the chunk that owns them**, in the chunk's own frame.
///
/// A chunk that has been despawned — a reset — leaves its strand alone rather than snapping it to the
/// origin: the anchors simply stop moving, which is what a gut on the floor should do.
fn carry_anchors(
    chunks: Query<&Transform, With<Chunk>>,
    mut guts: Query<(&Entrail, &mut Mesentery)>,
) {
    for (entrail, mut tether) in &mut guts {
        let Ok(chunk) = chunks.get(entrail.chunk) else { continue };
        for (slot, (_, at)) in tether.anchors.iter_mut().enumerate() {
            let Some(offset) = entrail.offsets.get(slot) else { continue };
            *at = chunk.translation + chunk.rotation * *offset;
        }
    }
}

/// Rebuild each tube from the nodes the solver just moved. The crate hands back a mesh and never
/// spawns, so this is the caller's job by design.
fn rebuild_tubes(mut meshes: ResMut<Assets<Mesh>>, guts: Query<(&Strand, &Mesh3d)>) {
    for (strand, handle) in &guts {
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            *mesh = tube_mesh(strand, SIDES);
        }
    }
}

/// Report the strands live against the cap, and how many mesenteric links have parted — the tear is
/// monotone, so this only ever climbs until a reset.
fn hud(
    guts: Query<&Mesentery, With<Entrail>>,
    flying: Query<(), With<Chunk>>,
    mut line: Query<&mut Text, With<HudStatus>>,
) {
    let (links, torn) = guts.iter().fold((0usize, 0usize), |(l, t), m| {
        (l + m.anchors.len(), t + m.torn.iter().filter(|x| **x).count())
    });
    let text = format!(
        "{} chunk(s) in play  |  {} of {} strand(s) streaming  |  {torn} of {links} mesenteric \
         link(s) torn (a tear never heals)",
        flying.iter().count(),
        guts.iter().count(),
        MAX_GUTS,
    );
    for mut t in &mut line {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}

/// Move the aim point. What *draws* it is [`draw_aim`].
fn aim_marker(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, mut aim: ResMut<Aim>) {
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
}

/// **The aim point is inside the subject as often as not**, and an opaque marker there is simply
/// invisible — measured: this example's sphere sat at the aim point with no standoff, inside a torso
/// 0.28 deep. A gizmo at `depth_bias = -1.0` renders in front of everything, so the marker is
/// readable at any aim and at any camera angle. Same fix `carnage.rs` and `sever.rs` carry.
fn aim_on_top(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.depth_bias = -1.0;
}

/// Draw the aim: a ring, and a cross that gives it a centre to read when it is behind geometry.
fn draw_aim(mut gizmos: Gizmos, aim: Res<Aim>) {
    let at = Isometry3d::from_translation(ORIGIN + aim.0);
    gizmos.sphere(at, AIM_RADIUS, AIM_COLOR).resolution(24);
    gizmos.cross(at, AIM_RADIUS * 1.8, AIM_COLOR);
}

/// Read the keyboard and hand the chosen region to [`body::strike`].
fn strike(world: &mut World) {
    let pressed =
        |world: &World, key: KeyCode| world.resource::<ButtonInput<KeyCode>>().just_pressed(key);

    if pressed(world, KeyCode::KeyR) {
        // The strands go with the reset: a gut on the floor of an intact subject is a lie about what
        // has happened to it.
        let strands: Vec<Entity> =
            world.query_filtered::<Entity, With<Entrail>>().iter(world).collect();
        for entity in strands {
            if let Ok(e) = world.get_entity_mut(entity) {
                e.despawn();
            }
        }
        body::clear(world);
        body::wipe(world);
        let damage = {
            let baked = world.resource::<body::Baked>();
            body::Damage::fresh(baked, GRANULARITY)
        };
        world.insert_resource(damage);
        body::stand(world, GRANULARITY);
        return;
    }

    let blow = [
        (KeyCode::Digit1, Blow::Projectile),
        (KeyCode::Digit2, Blow::Slash),
        (KeyCode::Digit3, Blow::SweptBlade),
        (KeyCode::Digit4, Blow::Blast),
        (KeyCode::Digit5, Blow::Pull),
    ]
    .into_iter()
    .find(|(key, _)| pressed(world, *key))
    .map(|(_, blow)| blow);

    if let Some(blow) = blow {
        let at = world.resource::<Aim>().0;
        body::strike(world, blow, at);
    }
}

/// The example's whole solver for the chunks — the crate names none. The strands are the viscera
/// crate's own integer-tick solver and are deliberately not integrated here.
fn integrate(time: Res<Time>, mut chunks: Query<(&mut Chunk, &mut Transform)>) {
    let dt = time.delta_secs() * 0.55;
    if dt <= 0.0 {
        return;
    }
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, dt);
    }
}
