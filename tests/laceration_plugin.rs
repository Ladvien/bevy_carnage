//! **The plugin's contract**, in a headless `App`: the clock advances, the wound widens, the bed
//! appears, and a wound that has finished opening stops costing anything.
//!
//! The app is deliberately the smallest one that can hold a `Mesh`: `TaskPoolPlugin` (the asset
//! arena needs its IO pool), `AssetPlugin` and `bevy::mesh::MeshPlugin`, which is what actually calls
//! `init_asset::<Mesh>`. No render device, no window, no PBR — so `CrossSectionAtlas` is absent and
//! the bed comes out without a material, which is exactly the configuration the retear system takes
//! `Option<Res<..>>` for.

use bevy::app::{App, TaskPoolPlugin};
use bevy::asset::{AssetPlugin, Assets};
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::hierarchy::Children;
use bevy::math::Vec3;
use bevy::mesh::{Indices, Mesh, Mesh3d, MeshPlugin};
use bevy_carnage::laceration::{
    Gape, Laceration, LacerationBed, LacerationClock, LacerationPlugin, Region, Tension, skin_patch,
};

/// Triangles in a mesh, without borrowing the kernel's own helpers.
fn triangles(mesh: &Mesh) -> usize {
    match mesh.try_indices_option() {
        Ok(Some(Indices::U32(i))) => i.len() / 3,
        Ok(Some(Indices::U16(i))) => i.len() / 3,
        _ => 0,
    }
}

fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default(), MeshPlugin, LacerationPlugin));
    app
}

#[test]
fn the_plugin_retears_as_the_clock_advances() {
    let mut app = headless_app();

    // Two handles from one patch: the entity draws the first, the component cuts from the second.
    // They must be different assets — the plugin refuses to overwrite its own intact source.
    let patch = skin_patch(20, 1.0);
    let (drawn, source) = {
        let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
        (meshes.add(patch.clone()), meshes.add(patch.clone()))
    };
    let intact = triangles(&patch);

    let subject = app
        .world_mut()
        .spawn((
            Mesh3d(drawn.clone()),
            Laceration {
                path: vec![Vec3::new(-0.4, 0.0, 0.0), Vec3::new(0.4, 0.0, 0.0)],
                normal: Vec3::Y,
                gape: Gape { width_max: 0.2, open_ticks: 60 },
                tension: Tension { skin: 1.0, langer: Some([0.0, 0.0, 1.0]) },
                influence: 0.15,
                bed_depth_mm: 6.0,
                region: Region::Limb,
                opened_at: 0,
                source: source.clone(),
                ..Default::default()
            },
        ))
        .id();

    app.update();
    assert_eq!(app.world().resource::<LacerationClock>().0, 1, "the clock must advance once per update");
    let first = {
        let meshes = app.world().resource::<Assets<Mesh>>();
        let drawn = meshes.get(&drawn).expect("the drawn mesh must still exist after one update");
        triangles(drawn)
    };
    assert!(first > 0, "the first update must leave a mesh, not an empty one");

    for _ in 0..80 {
        app.update();
    }

    let open = {
        let meshes = app.world().resource::<Assets<Mesh>>();
        let drawn = meshes.get(&drawn).expect("the drawn mesh must survive the whole open");
        triangles(drawn)
    };
    assert!(
        open < first,
        "the wound did not widen: {first} triangles after one update, {open} after 81"
    );
    assert!(open > 0, "the wound swallowed the entire patch — check the gape against the mesh size");

    // The intact source is never written, which is what makes the gape a function of the clock rather
    // than of how many times the system happened to run.
    let untouched = {
        let meshes = app.world().resource::<Assets<Mesh>>();
        triangles(meshes.get(&source).expect("the source must still exist"))
    };
    assert_eq!(untouched, intact, "the source mesh was edited; retearing is no longer idempotent");

    // The bed exists, is a child, is marked, and carries geometry.
    let bed = app
        .world()
        .get::<Laceration>(subject)
        .and_then(|lac| lac.bed)
        .expect("the plugin must have recorded the bed it spawned");
    assert!(app.world().get::<LacerationBed>(bed).is_some(), "the bed child must carry the marker");
    let children = app.world().get::<Children>(subject).map(|c| c.len()).unwrap_or(0);
    assert_eq!(children, 1, "exactly one bed child, respawned never");
    let bed_tris = {
        let handle = app.world().get::<Mesh3d>(bed).expect("the bed needs a Mesh3d").0.clone();
        let meshes = app.world().resource::<Assets<Mesh>>();
        triangles(meshes.get(&handle).expect("the bed's mesh must exist"))
    };
    assert!(bed_tris > 0, "the wound bed has no triangles");
}

#[test]
fn a_wound_that_has_finished_opening_costs_nothing() {
    let mut app = headless_app();
    let patch = skin_patch(12, 1.0);
    let (drawn, source) = {
        let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
        (meshes.add(patch.clone()), meshes.add(patch))
    };
    let subject = app
        .world_mut()
        .spawn((
            Mesh3d(drawn),
            Laceration {
                path: vec![Vec3::new(-0.3, 0.0, 0.0), Vec3::new(0.3, 0.0, 0.0)],
                normal: Vec3::Y,
                gape: Gape { width_max: 0.15, open_ticks: 10 },
                tension: Tension { skin: 1.0, langer: None },
                source,
                ..Default::default()
            },
        ))
        .id();

    // Long past `open_ticks`, the exponential is flat to well inside GAPE_EPSILON.
    for _ in 0..200 {
        app.update();
    }
    let settled = app.world().entity(subject).get_ref::<Laceration>().map(|lac| lac.last_changed());
    app.update();
    let after = app.world().entity(subject).get_ref::<Laceration>().map(|lac| lac.last_changed());
    assert_eq!(
        settled, after,
        "a fully open wound still rewrote its component, which means it is still rebuilding its mesh every frame"
    );
}

#[test]
fn a_laceration_that_draws_its_own_source_is_refused_rather_than_destroying_it() {
    let mut app = headless_app();
    let patch = skin_patch(8, 1.0);
    let intact = triangles(&patch);
    let shared = app.world_mut().resource_mut::<Assets<Mesh>>().add(patch);
    app.world_mut().spawn((
        Mesh3d(shared.clone()),
        Laceration {
            path: vec![Vec3::new(-0.3, 0.0, 0.0), Vec3::new(0.3, 0.0, 0.0)],
            normal: Vec3::Y,
            gape: Gape { width_max: 0.2, open_ticks: 5 },
            tension: Tension { skin: 1.0, langer: None },
            // The mistake: one handle for both roles.
            source: shared.clone(),
            ..Default::default()
        },
    ));
    for _ in 0..20 {
        app.update();
    }
    let still = {
        let meshes = app.world().resource::<Assets<Mesh>>();
        triangles(meshes.get(&shared).expect("the shared mesh must survive"))
    };
    assert_eq!(still, intact, "the intact mesh was cut into; the guard did not hold");
}
