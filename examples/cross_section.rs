//! **Three limbs, three regions, three cut faces.** A prism per region — sized like a thigh, a
//! trunk and a skull — sliced flat, its cap annotated with depth-below-skin and painted with that
//! region's strip. Orbit with the mouse wheel; the bands are what the whole crate is for.
//!
//! ```sh
//! cargo run --example cross_section
//! ```
//!
//! Also the wasm demo `cross_section` on the monorepo's demo site.

use bevy::prelude::*;
use bevy_carnage::cross_section::{
    CrossSectionAtlas, CrossSectionPlugin, CrossSectionSettings, Region, SkinPlane, annotate_cap,
};

/// Sides of the prism standing in for a limb. Enough that the cap reads as round.
const SIDES: usize = 32;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "bevy_cross_section".into(), ..default() }),
            ..default()
        }))
        .add_plugins(CrossSectionPlugin)
        .add_systems(Startup, setup.after(bevy_carnage::cross_section::CrossSectionSystems))
        .add_systems(Update, orbit)
        .run();
}

#[derive(Component)]
struct Orbit;

/// A prism of `radius` and `height`, cut flat at its top: the sides are the skin, the top is the cap.
///
/// Returns the side mesh, the cap mesh (already annotated), and nothing else — a skin material and
/// the region's cap material are the caller's to pair.
fn limb(radius: f32, height: f32, region: Region, settings: &CrossSectionSettings) -> (Mesh, Mesh) {
    let mut ring: Vec<Vec3> = Vec::with_capacity(SIDES);
    for i in 0..SIDES {
        let a = i as f32 / SIDES as f32 * std::f32::consts::TAU;
        ring.push(Vec3::new(radius * a.cos(), 0.0, radius * a.sin()));
    }
    // Skin planes: one per side, outward.
    let planes: Vec<SkinPlane> = (0..SIDES)
        .map(|i| {
            let a = ring[i];
            let b = ring[(i + 1) % SIDES];
            let mid = (a + b) * 0.5;
            let normal = Vec3::new(mid.x, 0.0, mid.z).normalize_or_zero();
            SkinPlane { point: mid, normal }
        })
        .collect();

    // Sides.
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uv = Vec::new();
    let mut idx = Vec::new();
    for i in 0..SIDES {
        let a = ring[i];
        let b = ring[(i + 1) % SIDES];
        let n = Vec3::new((a.x + b.x) * 0.5, 0.0, (a.z + b.z) * 0.5).normalize_or_zero();
        let base = pos.len() as u32;
        for p in [a, b, b + Vec3::Y * height, a + Vec3::Y * height] {
            pos.push(p.to_array());
            nrm.push(n.to_array());
            uv.push([0.0, 0.0]);
        }
        idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    let mut sides = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    sides.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    sides.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    sides.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    sides.insert_indices(bevy::mesh::Indices::U32(idx));

    // Cap: a two-ring fan like a real cut face, so the bands have interior vertices to land on.
    let top = Vec3::Y * height;
    let mut cpos = vec![top.to_array()];
    let mut cuv = vec![[0.0f32, 0.0]];
    for r in [0.5f32, 1.0] {
        for p in &ring {
            let q = *p * r + top;
            cpos.push(q.to_array());
            cuv.push([q.x, q.z]);
        }
    }
    let mut cidx = Vec::new();
    for i in 0..SIDES as u32 {
        let j = (i + 1) % SIDES as u32;
        let (mi, mj) = (1 + i, 1 + j);
        let (oi, oj) = (1 + SIDES as u32 + i, 1 + SIDES as u32 + j);
        cidx.extend([0, mj, mi]);
        cidx.extend([mi, mj, oj, mi, oj, oi]);
    }
    let mut cap = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    cap.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; cpos.len()]);
    cap.insert_attribute(Mesh::ATTRIBUTE_POSITION, cpos);
    cap.insert_attribute(Mesh::ATTRIBUTE_UV_0, cuv);
    cap.insert_indices(bevy::mesh::Indices::U32(cidx));
    annotate_cap(&mut cap, &planes, Vec3::ZERO, settings.layers(region), &settings.scale);
    (sides, cap)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    atlas: Res<CrossSectionAtlas>,
    settings: Res<CrossSectionSettings>,
) {
    let skin = materials.add(StandardMaterial {
        base_color: Color::srgb(0.80, 0.62, 0.55),
        perceptual_roughness: 0.6,
        ..default()
    });
    // A thigh, a trunk and a skull — radii that put the bands at their measured widths.
    let subjects = [(Region::Limb, 0.075, -0.45), (Region::Torso, 0.12, 0.0), (Region::Head, 0.08, 0.45)];
    for (region, radius, x) in subjects {
        let (sides, cap) = limb(radius, 0.18, region, &settings);
        let Some(cap_material) = atlas.material(region) else {
            error!("no strip material for {region:?}");
            continue;
        };
        let at = Transform::from_xyz(x, -0.09, 0.0);
        commands.spawn((Mesh3d(meshes.add(sides)), MeshMaterial3d(skin.clone()), at));
        commands.spawn((Mesh3d(meshes.add(cap)), MeshMaterial3d(cap_material), at));
    }

    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(1.0, 2.0, 1.5).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.7, 0.75).looking_at(Vec3::ZERO, Vec3::Y),
        Orbit,
    ));
}

fn orbit(time: Res<Time>, mut cams: Query<&mut Transform, With<Orbit>>) {
    let t = time.elapsed_secs() * 0.25;
    for mut tf in &mut cams {
        tf.translation = Vec3::new(t.sin() * 0.8, 0.7, t.cos() * 0.8);
        tf.look_at(Vec3::ZERO, Vec3::Y);
    }
}
