//! **What every carnage example shares**: the demo subject, the scene furniture, and the two
//! conversions across the `bloodstain` boundary.
//!
//! The headless PNG recorder lives one file over, in [`recorder`], and is native-only — see that
//! module's own header for why that is a platform fact rather than a feature gate.
//!
//! Rendering a demo to PNGs needs the same eight awkward things every time: `DefaultPlugins` with no
//! window and no winit, shaders compiled synchronously so frame 0 is not empty, a render target that
//! is not a surface, a camera pointed at it, a hand-pumped update loop, a device poll so a screenshot
//! does not read a half-drawn frame, and a few extra pumps at the end because screenshot readback
//! lands a frame or two late. None of that is interesting, all of it is easy to get subtly wrong, and
//! there are two recorders — so it lives here once rather than twice.
//!
//! What each recorder still owns is its *scene* and its *script*: what stands there, what happens to
//! it, and on which frame. That is the part worth reading.
//!
//! Not an example itself — Cargo only auto-discovers `examples/*.rs` and `examples/*/main.rs`, so a
//! bare `mod.rs` in a subdirectory is compiled only by the examples that `mod common;` it.

// Each recorder uses a subset of this, so the other's share reads as dead to the compiler. The
// alternative is splitting the harness by consumer, which would put the awkward parts back in two
// places — exactly what this module exists to prevent.
#![allow(dead_code)]

pub mod body;
/// **The headless PNG recorder — native only.**
///
/// A recorder writes PNGs to a filesystem and a browser has no filesystem, so "absent in a browser"
/// is a fact about the platform rather than a choice between two implementations. It also carries
/// every one of this crate's wasm blockers (`std::fs`, `save_to_disk`, a blocking `device.poll`),
/// and `body.rs` reaches this module's `material` helper — so before the split, importing the demo
/// subject dragged the recorder in and the wasm build failed on `std::fs`.
#[cfg(not(target_arch = "wasm32"))]
pub mod recorder;

use bevy::prelude::*;
use bevy_carnage::{
    BloodSettings, Stain, StainShape, Wound, blood, droplets, impact_at_plane, landing, stain_shape,
    stains,
};

/// `--<flag> <value>`, hand-parsed.
///
/// Deliberately not a CLI crate: one flag does not justify an entry in this repo's dependency graph,
/// not even a dev-dependency. A flag given with no value is `warn!`ed and treated as absent, never
/// silently substituted — the same rule the crate applies to a mesh with no positions.
pub fn arg(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == flag {
            match args.next() {
                Some(v) => return Some(v),
                None => {
                    warn!("capture: {flag} given with no value; ignoring it");
                    return None;
                }
            }
        }
    }
    None
}

/// The furniture every recorded scene shares: a key light, a fill, and a floor to land on.
pub fn light_and_floor(world: &mut World) {
    // **The fill is not a nicety.** With a single directional light, every surface turned away from
    // it renders at zero — and a cut face at zero, against a dark background, does not read as a
    // shadowed face. It reads as a *hole*, and the fragment looks like an open shell you can see
    // through. That was reported as missing geometry and was in fact missing light.
    world.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.62, 0.66, 0.78),
        brightness: 900.0,
        ..default()
    });
    world.spawn((
        DirectionalLight { illuminance: 9_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(4.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let floor = world
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::from(Plane3d::default().mesh().size(14.0, 14.0)));
    let dark = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.16, 0.18),
        perceptual_roughness: 0.95,
        ..default()
    });
    world.spawn((Mesh3d(floor), MeshMaterial3d(dark)));
}

/// Add a material to the world in one line, since a recorder builds several by hand.
pub fn material(world: &mut World, color: Color, roughness: f32) -> Handle<StandardMaterial> {
    world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: color,
        perceptual_roughness: roughness,
        ..default()
    })
}

/// **The examples' own `[f32; 3]` ↔ `Vec3` boundary.**
///
/// `bloodstain` names no math library, so its public API is `[f32; 3]`; `bevy_carnage`'s is `glam`.
/// The crate converts in one private module, `bevy_carnage::v3` — private on purpose, because it is
/// an implementation detail of the facade rather than a promise — so a caller converts at its own
/// boundary. This is these examples' one.
pub fn blood_wound(w: &Wound) -> blood::Wound {
    blood::Wound {
        at: [w.at.x, w.at.y, w.at.z],
        normal: [w.normal.x, w.normal.y, w.normal.z],
        area: w.area,
        severity: w.severity,
        kind: w.kind,
    }
}

/// The other half of that boundary: a `bloodstain` vector back into `glam`.
pub fn v3(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}

/// **Every stain one wound leaves, paired with the silhouette its own droplet implies.**
///
/// `stains` answers *where* and *how wide*; `stain_shape` answers *what shape*, and it needs the
/// impact conditions the droplet arrived with. The two are matched by re-running `landing` — the same
/// public predicate `stains` skips a droplet on — so the nth droplet that reaches the plane is the
/// nth stain, and a stain is never drawn with another droplet's silhouette.
pub fn stains_with_shapes(
    w: &blood::Wound,
    s: &BloodSettings,
    plane_y: f32,
) -> Vec<(Stain, StainShape)> {
    let landed = droplets(w, s)
        .into_iter()
        .filter(|d| landing(w.at, d, s.gravity, plane_y).is_some());
    stains(w, s, plane_y)
        .into_iter()
        .zip(landed)
        .map(|(stain, d)| {
            let impact = impact_at_plane(&d, w.at, plane_y, s);
            (stain, stain_shape(&impact, s, stain.seed))
        })
        .collect()
}
