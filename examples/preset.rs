//! **The two-line inclusion, live.** `GorePlugin`, a `Gore` on every surface, a `GoreHit` per key.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example preset
//! ```
//!
//! | key | what |
//! |---|---|
//! | `1` | a shot to the torso — peel, blood on the body, the sheet and the slab, a bleed; the third at one spot shows cortex and the body comes apart |
//! | `2` | a blow to the left arm — a bruise that ages in time-lapse under intact skin |
//! | `3` | a hot iron on the right thigh — a burn whose degree the damage integral decides |
//! | `4` | a slash across the left thigh — a laceration that bleeds and drips |
//! | `T` | the player's gore setting: stylised (sparks, no blood), blood (no guts, nothing breaks), blood and gore |
//! | `R` | heal the body and wipe what it bled on, in place (`GoreReset`); a body that broke is rebuilt |
//!
//! Every visual is the preset's and the flesh material's. Compare `carnage_web.rs`, which wired the
//! same eight crates by hand until 0.4.0 and now stands on this same scene.

use bevy::prelude::*;
use bevy_carnage::preset::{GorePlugin, GoreReset, GoreSystems};
use bevy_carnage::{GorePolicy, GoreTier};

mod common;
use common::preset_scene::{self, Body, Part, Sheet, Slab, Sun};

fn main() {
    let mut app = App::new();
    app.add_plugins(common::autopilot::plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window { title: "bevy_carnage — preset".into(), ..default() }),
        ..default()
    })))
    .add_plugins(common::autopilot::Autopilot)
    .add_plugins(GorePlugin)
    .insert_resource(Time::<Fixed>::from_hz(60.0))
    .add_systems(Startup, setup)
    .add_systems(Update, (keys, sweep, tier_key))
    .configure_sets(FixedUpdate, GoreSystems);
    // Blood in the air from every wound, and at a tier that draws none the same burst in the
    // substitute palette. Without `vfx`, the game's own spark stands in for it.
    #[cfg(feature = "vfx")]
    app.add_plugins(bevy_carnage::CarnageVfxPlugin);
    #[cfg(not(feature = "vfx"))]
    app.add_systems(Update, preset_scene::sparks.run_if(|policy: Res<GorePolicy>| !policy.draws_blood()));
    app.run();
}

#[derive(Component)]
struct Sweep;

/// How many shots have landed on the torso, so each lands a hair from the last.
#[derive(Resource, Default)]
struct Shots(u32);

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    commands.spawn((
        Camera3d::default(),
        preset_scene::sweep(0.0),
        preset_scene::camera_light(&mut images),
        Sweep,
    ));
    commands.init_resource::<Shots>();
    preset_scene::spawn(&mut commands, &mut meshes, &mut materials, &mut images);
    commands.spawn((
        Text::new(format!("1 shot   2 blow   3 iron   4 slash   T gore: {}   R reset", label(GoreTier::BloodAndGore))),
        Hud,
        TextFont { font_size: bevy::text::FontSize::Px(14.0), ..default() },
        TextColor(Color::srgb(0.92, 0.88, 0.86)),
        Node { position_type: PositionType::Absolute, left: Val::Px(12.0), top: Val::Px(10.0), ..default() },
    ));
}

/// The legend, so the tier readout can be rewritten in place.
#[derive(Component)]
struct Hud;

/// The words for a tier, as a settings menu would print them.
fn label(tier: GoreTier) -> &'static str {
    match tier {
        GoreTier::Stylised => "stylised: sparks, no blood",
        GoreTier::Blood => "blood: no guts, nothing breaks",
        GoreTier::BloodAndGore | GoreTier::GrossViolence => "blood and gore",
    }
}

/// **`T` is the player's violence setting**: one `GorePolicy` resource, replaced from the tier. The
/// next hit follows it, and nothing already painted is taken back. The preset reads `Stylised` as
/// "announce, do not paint", `Blood` as "blood, but no guts and no breaking", and the other two as
/// everything.
fn tier_key(input: Res<ButtonInput<KeyCode>>, mut policy: ResMut<GorePolicy>, mut hud: Single<&mut Text, With<Hud>>) {
    if input.just_pressed(KeyCode::KeyT) {
        let next = match policy.tier {
            GoreTier::Stylised => GoreTier::Blood,
            GoreTier::Blood => GoreTier::BloodAndGore,
            GoreTier::BloodAndGore | GoreTier::GrossViolence => GoreTier::Stylised,
        };
        *policy = GorePolicy::for_tier(next);
    }
    if policy.is_changed() {
        hud.0 = format!("1 shot   2 blow   3 iron   4 slash   T gore: {}   R reset", label(policy.tier));
    }
}

#[allow(clippy::too_many_arguments)]
fn keys(
    input: Res<ButtonInput<KeyCode>>,
    mut shots: ResMut<Shots>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    parts: Query<(Entity, &Part)>,
    scene: Query<Entity, Or<(With<Body>, With<Sheet>, With<Slab>, With<Sun>)>>,
    surfaces: Query<Entity, Or<(With<Body>, With<Sheet>, With<Slab>)>>,
    pieces: Query<
        Entity,
        Or<(With<bevy_carnage::preset::Flying>, With<bevy_carnage::preset::Gut>, With<bevy_carnage::preset::Standing>)>,
    >,
) {
    let part = |i: usize| parts.iter().find(|(_, p)| p.0 == i).map(|(e, _)| e);
    if input.just_pressed(KeyCode::Digit1)
        && let Some(torso) = part(0)
    {
        commands.write_message(preset_scene::shot(torso, shots.0));
        shots.0 += 1;
    }
    if input.just_pressed(KeyCode::Digit2)
        && let Some(arm) = part(2)
    {
        commands.write_message(preset_scene::blow(arm));
    }
    if input.just_pressed(KeyCode::Digit3)
        && let Some(leg) = part(5)
    {
        commands.write_message(preset_scene::scald(leg));
    }
    if input.just_pressed(KeyCode::Digit4)
        && let Some(leg) = part(4)
    {
        commands.write_message(preset_scene::slash(leg));
    }
    if input.just_pressed(KeyCode::KeyR) {
        for e in &pieces {
            commands.entity(e).despawn();
        }
        shots.0 = 0;
        // **A whole body heals in place**: a `GoreReset` for it and the surfaces it bled on. One the
        // last shot broke has been despawned, so the scene is built again.
        if parts.is_empty() {
            for e in &scene {
                commands.entity(e).despawn();
            }
            preset_scene::spawn(&mut commands, &mut meshes, &mut materials, &mut images);
        } else {
            for e in &surfaces {
                commands.write_message(GoreReset::new(e));
            }
        }
    }
}

fn sweep(time: Res<Time>, mut cams: Query<&mut Transform, With<Sweep>>) {
    for mut tf in &mut cams {
        *tf = preset_scene::sweep(time.elapsed_secs());
    }
}
