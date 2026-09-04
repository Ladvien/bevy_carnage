//! **The flesh material, live.** A banded limb, a skin sphere and a sheet under a light that walks
//! round them: Penner's wrap past the terminator per tissue, a wet clear coat where blood lands, a
//! film composited over a weave. Hits every second peel the sphere and throw blood on all three.
//!
//! ```sh
//! cargo run --release -p bevy_carnage --example flesh
//! ```
//!
//! `capture_flesh` is the same scene, recorded.

use bevy::prelude::*;
use bevy_carnage::cross_section::{CrossSectionPlugin, CrossSectionSystems};
use bevy_carnage::flaymap::FlaymapPlugin;
use bevy_carnage::flesh::{FleshPlugin, FleshSystems};
use bevy_carnage::wetmap::{WetCanvas, WetSettings, WetmapPlugin};

mod common;
use common::flesh_scene::{self, Sun};

/// Fixed ticks so far — the clock every canvas is measured against.
#[derive(Resource, Default)]
struct Tick(u32);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage — flesh".into(),
                canvas: Some("#bevy".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins((CrossSectionPlugin, FlaymapPlugin, WetmapPlugin, FleshPlugin))
        .insert_resource(WetSettings { dry_ticks: 600, edge_samples: 4, ..default() })
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .init_resource::<Tick>()
        .add_systems(Startup, (camera, flesh_scene::setup.after(CrossSectionSystems).after(FleshSystems)))
        .add_systems(Update, walk_the_sun)
        .add_systems(FixedUpdate, tick)
        .run();
}

fn camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.9, 2.6).looking_at(Vec3::new(0.0, 0.15, 0.0), Vec3::Y),
        AmbientLight { brightness: 300.0, ..default() },
    ));
}

fn walk_the_sun(time: Res<Time>, mut suns: Query<&mut Transform, With<Sun>>) {
    let angle = time.elapsed_secs() * 0.35;
    for mut t in &mut suns {
        *t = Transform::from_xyz(3.0 * angle.cos(), 2.5, 3.0 * angle.sin()).looking_at(Vec3::ZERO, Vec3::Y);
    }
}

/// One fixed tick: a hit every second for the first ten, then the canvases run and dry.
fn tick(world: &mut World) {
    let t = {
        let mut tick = world.resource_mut::<Tick>();
        tick.0 = tick.0.wrapping_add(1);
        tick.0
    };
    if t % 60 == 30 && t < 600 {
        flesh_scene::hit(world, t / 60, t);
    }
    let settings = world.resource::<WetSettings>().clone();
    let mut wets = world.query::<&mut WetCanvas>();
    for mut w in wets.iter_mut(world) {
        w.tick(t, Vec2::new(0.0, 1.0), &settings);
    }
}
