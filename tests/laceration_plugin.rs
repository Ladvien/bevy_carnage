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
use bevy::camera::visibility::DynamicSkinnedMeshBounds;
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::hierarchy::Children;
use bevy::math::{Mat4, Vec3};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, Mesh, Mesh3d, MeshPlugin, VertexAttributeValues};
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

/// **A skinned skin lays a skinned bed.** A patch whose half at `x < 0` is weighted to one joint and
/// whose other half to a second is cut across both: the bed carries the skin's `SkinnedMesh` and its
/// `DynamicSkinnedMeshBounds`, every bed vertex is bound to the joint of the half it lies in, and the
/// bed mesh has the bounds Bevy culls a skinned mesh by. A bed that was not would be drawn in the bind
/// pose, standing off a limb that had moved.
#[test]
fn a_skinned_skin_lays_a_bed_bound_to_its_joints() {
    let mut app = headless_app();
    let mut patch = skin_patch(20, 1.0);
    let joint_of = |x: f32| u16::from(x >= 0.0);
    let joints: Vec<[u16; 4]> = match patch.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(pos)) => pos.iter().map(|p| [joint_of(p[0]), 0, 0, 0]).collect(),
        _ => panic!("the patch has Float32x3 positions"),
    };
    let weights = vec![[1.0f32, 0.0, 0.0, 0.0]; joints.len()];
    patch.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
    patch.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    patch.generate_skinned_mesh_bounds().expect("the patch is skinned");
    let (drawn, source) = {
        let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
        (meshes.add(patch.clone()), meshes.add(patch))
    };
    let inverse_bindposes =
        app.world_mut().resource_mut::<Assets<SkinnedMeshInverseBindposes>>().add(SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY; 2]));
    let bones = vec![app.world_mut().spawn_empty().id(), app.world_mut().spawn_empty().id()];
    let subject = app
        .world_mut()
        .spawn((
            Mesh3d(drawn),
            SkinnedMesh { inverse_bindposes, joints: bones.clone() },
            DynamicSkinnedMeshBounds,
            Laceration {
                path: vec![Vec3::new(-0.4, 0.0, 0.0), Vec3::new(0.4, 0.0, 0.0)],
                normal: Vec3::Y,
                gape: Gape { width_max: 0.2, open_ticks: 60 },
                tension: Tension { skin: 1.0, langer: Some([0.0, 0.0, 1.0]) },
                influence: 0.15,
                bed_depth_mm: 6.0,
                region: Region::Limb,
                opened_at: 0,
                source,
                ..Default::default()
            },
        ))
        .id();
    for _ in 0..80 {
        app.update();
    }

    let bed = app.world().get::<Laceration>(subject).and_then(|lac| lac.bed).expect("the plugin must have spawned a bed");
    assert_eq!(app.world().get::<SkinnedMesh>(bed).map(|s| s.joints.clone()), Some(bones), "the bed is not skinned by its skin's joints");
    assert!(app.world().get::<DynamicSkinnedMeshBounds>(bed).is_some(), "the bed is not culled the way its skin is");
    let handle = app.world().get::<Mesh3d>(bed).expect("the bed needs a Mesh3d").0.clone();
    let meshes = app.world().resource::<Assets<Mesh>>();
    let mesh = meshes.get(&handle).expect("the bed's mesh must exist");
    assert!(mesh.skinned_mesh_bounds().is_some(), "the bed has no skinned bounds to be culled by");
    let (Some(VertexAttributeValues::Float32x3(pos)), Some(VertexAttributeValues::Uint16x4(bound)), Some(VertexAttributeValues::Float32x4(weights))) = (
        mesh.attribute(Mesh::ATTRIBUTE_POSITION),
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX),
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT),
    ) else {
        panic!("the bed carries no joint indices or weights");
    };
    assert!(triangles(mesh) > 0 && bound.len() == pos.len() && weights.len() == pos.len(), "one binding per bed vertex");
    // A cell is 0.05 wide: a bed vertex within one of the seam may be nearest either half.
    let (mut left, mut right) = (0, 0);
    for (p, j) in pos.iter().zip(bound) {
        if p[0].abs() > 0.06 {
            assert_eq!(j[0], joint_of(p[0]), "a bed vertex at x = {} is bound to the other half's joint", p[0]);
            if p[0] < 0.0 { left += 1 } else { right += 1 }
        }
    }
    assert!(left > 0 && right > 0, "the cut did not lay bed on both halves ({left} left, {right} right)");
}
