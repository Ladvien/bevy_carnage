//! **Every flying chunk drags a strand of blood behind it.**
//!
//! `sever.rs` shows the fracture; this shows what a piece of it looks like *in the air*. Hit the
//! subject and each fragment that comes loose trails a dark red ribbon that stays where it was
//! emitted while the chunk moves away from it, thinning and fading over about nine tenths of a
//! second, and stopping cleanly when the chunk lands.
//!
//! ```text
//!   arrows / WASD   move the aim marker
//!   1               a projectile   2 a slash   3 a swept blade   4 a blast   5 a pull
//!   R               reset
//! ```
//!
//! # What to look for, because three things fail in ways that look like a look
//!
//! - **Every chunk on one strand.** That would mean the instances are not getting their own slices of
//!   the particle slab. They are: `allocate()` hands each a disjoint contiguous sub-slice and the
//!   ribbon shader indexes strictly inside it, which is why one asset with a constant `RIBBON_ID`
//!   serves any number of chunks. See `gib_ribbon`'s docs — the crate used to claim the opposite.
//! - **The strand following the chunk instead of trailing behind it.** That is
//!   `SimulationSpace::Local` or motion integration left on.
//! - **Strands that never disappear.** That is `fade_effects` never seeing an `EffectSpawner`,
//!   because Hanabi adds that component lazily in `PostUpdate` and a plain `&mut` query skips an
//!   instance on its first frame.
//!
//! And the cap: throw more than `CarnageSettings::max_ribbons` chunks at once and the later ones
//! simply have no ribbon. No running ribbon ever vanishes to make room — first come, first served.
//!
//! Needs a GPU.
//!
//! Run: `cargo run --release -p bevy_carnage --example ribbons`

use bevy::prelude::*;
use bevy_carnage::{BleedingChunk, CarnageSettings, CarnageVfxPlugin, RibbonInstance};

mod common;
use common::body::{self, Blow, BodyMaterials, Chunk, ORIGIN};
use common::light_and_floor;

/// The frontier this demo stands at — index into [`body::GRANULARITIES`]. The finest, because the
/// point is a lot of small pieces in the air at once.
const GRANULARITY: usize = body::GRANULARITIES.len() - 1;

/// Rounded, so the chunks read as torn rather than as cleaved ice.
const SOFTEN: f32 = 0.5;

/// Where the next blow lands, in subject-local space.
#[derive(Resource)]
struct Aim(Vec3);

/// Marks the little sphere that shows [`Aim`].
#[derive(Component)]
struct AimMarker;

/// Marks the line reporting the ribbon count against the cap.
#[derive(Component)]
struct HudStatus;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage — ribbons (1-5 to hit, arrows to aim, R reset)".into(),
                resolution: (960u32, 680u32).into(),
                ..default()
            }),
            ..default()
        }))
        // The deterministic half is not needed here: `common::body` bakes through `fracture_mesh`,
        // the pure path. `CarnageVfxPlugin` is what brings in Hanabi and the ribbon systems.
        .add_plugins(CarnageVfxPlugin)
        .insert_resource(Aim(Vec3::new(0.0, 0.25, 0.0)))
        .add_systems(Startup, setup)
        .add_systems(Update, (aim_marker, strike, mark_bleeding, integrate, hud).chain())
        .run();
}

fn setup(world: &mut World) {
    let camera = Transform::from_xyz(2.25, 1.35, 2.95).looking_at(Vec3::new(0.0, 0.76, 0.0), Vec3::Y);
    // **`DepthPrepass` is not optional.** `CarnageVfxPlugin` brings the decal half along with the
    // particles, and a forward decal without a prepass renders as an opaque quad or not at all.
    world.spawn((Camera3d::default(), bevy::core_pipeline::prepass::DepthPrepass, camera));
    light_and_floor(world);

    let baked = body::Baked::bake(world, SOFTEN, &[]);
    let materials = BodyMaterials::new(world);
    let damage = body::Damage::fresh(&baked, GRANULARITY);

    let marker = world.resource_mut::<Assets<Mesh>>().add(Mesh::from(Sphere::new(0.05)));
    world.spawn((
        AimMarker,
        Mesh3d(marker),
        MeshMaterial3d(materials.aim.clone()),
        Transform::from_translation(ORIGIN),
    ));

    world.insert_resource(baked);
    world.insert_resource(materials);
    world.insert_resource(damage);
    body::stand(world, GRANULARITY);

    world.spawn((
        Text::new(
            "arrows / WASD  aim\n             1 projectile   2 slash   3 blade   4 blast   5 pull\n             R reset",
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

/// **The consumer's decision, which is the whole point of the marker being a marker.** A chunk in the
/// air bleeds; a chunk that has touched down does not. The crate never learns what a chunk is — it
/// only knows that something carrying [`BleedingChunk`] gets a strand.
///
/// One way: a chunk that stops does not start again. The blood it had to give left with the throw.
fn mark_bleeding(
    mut commands: Commands,
    chunks: Query<(Entity, &Chunk, &Transform, Option<&BleedingChunk>)>,
) {
    for (entity, chunk, transform, bleeding) in &chunks {
        let grounded = transform.translation.y <= chunk.drop_to_rest + 1.0e-3;
        match (grounded, bleeding.is_some()) {
            (false, false) => {
                commands.entity(entity).insert(BleedingChunk);
            }
            (true, true) => {
                commands.entity(entity).remove::<BleedingChunk>();
            }
            _ => {}
        }
    }
}

/// Report the live ribbon count against the cap, so the first-come-first-served rule is visible
/// rather than something you have to take on trust.
fn hud(
    settings: Res<CarnageSettings>,
    ribbons: Query<(), With<RibbonInstance>>,
    flying: Query<(), With<BleedingChunk>>,
    mut line: Query<&mut Text, With<HudStatus>>,
) {
    let text = format!(
        "{} chunk(s) bleeding  |  {} of {} ribbons live (the cap never evicts a running one)",
        flying.iter().count(),
        ribbons.iter().count(),
        settings.max_ribbons,
    );
    for mut t in &mut line {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}

/// Move the aim marker, and keep the sphere on it.
fn aim_marker(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut aim: ResMut<Aim>,
    mut marker: Query<&mut Transform, With<AimMarker>>,
) {
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
    for mut t in &mut marker {
        t.translation = ORIGIN + aim.0;
    }
}

/// Read the keyboard and hand the chosen region to [`body::strike`].
fn strike(world: &mut World) {
    let pressed =
        |world: &World, key: KeyCode| world.resource::<ButtonInput<KeyCode>>().just_pressed(key);

    if pressed(world, KeyCode::KeyR) {
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

/// The example's whole solver — the crate names none.
fn integrate(time: Res<Time>, mut chunks: Query<(&mut Chunk, &mut Transform)>) {
    let dt = time.delta_secs() * 0.55;
    if dt <= 0.0 {
        return;
    }
    for (mut chunk, mut transform) in &mut chunks {
        body::integrate(&mut chunk, &mut transform, dt);
    }
}
