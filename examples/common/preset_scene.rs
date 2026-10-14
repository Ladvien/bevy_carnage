//! **The preset's scene**: the blockout body dressed by [`bevy_carnage::preset`], a cloth panel,
//! and a floor — shared by the windowed `preset` demo and the headless `capture_preset` clip.
//!
//! This file is the whole inclusion cost of the family, and that is the point of it: one plugin, one
//! component per surface, one message per hit. Compare `carnage_web.rs`, which does the same by hand.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_carnage::cross_section::{Region, Scale};
use bevy_carnage::preset::{Gore, GoreBreakable, GoreHit};
use bevy_carnage::{CutSettings, fracture_mesh};

use super::body::{self, ORIGIN, SEED};

/// The body root, so a demo can find what to hit and what to reset.
#[derive(Component)]
pub struct Body;
/// One dressed part, with its index into [`body::parts`].
#[derive(Component)]
pub struct Part(pub usize);
/// The hanging sheet.
#[derive(Component)]
pub struct Sheet;
/// The slab.
#[derive(Component)]
pub struct Slab;
/// The scene's light, so a reset does not stack another on top of it.
#[derive(Component)]
pub struct Sun;
/// A spark the game draws where a tier that draws no blood announced a wound.
#[derive(Component)]
pub struct Spark(pub u32);

/// **The game's half of "reduction is substitution".** `GorePlugin` at `GoreTier::Stylised` paints no
/// blood, but it still writes the same [`bevy_carnage::Wounded`] a bleeding hit writes, at the same
/// point with the same normal and area. This is what a game does with it: a short spark in
/// [`bevy_carnage::CarnageSettings::substitute_srgb`], where the blood would have been. A real game
/// would play its own effect; the point is that it is told where, and when, and how big.
pub fn sparks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut wounds: MessageReader<bevy_carnage::Wounded>,
    mut sparks: Query<(Entity, &mut Spark, &mut Transform)>,
    mut kit: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
) {
    let (mesh, material) = kit
        .get_or_insert_with(|| {
            let [r, g, b] = bevy_carnage::CarnageSettings::default().substitute_srgb;
            let colour = Color::srgb(r, g, b);
            (
                meshes.add(Sphere::new(1.0).mesh().uv(16, 8)),
                materials.add(StandardMaterial { base_color: colour, emissive: colour.to_linear() * 20.0, unlit: true, ..default() }),
            )
        })
        .clone();
    for wound in wounds.read() {
        let radius = (wound.area / core::f32::consts::PI).sqrt().max(0.01);
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(wound.at + wound.normal * radius).with_scale(Vec3::splat(radius * 2.2)),
            Spark(8),
        ));
    }
    for (entity, mut spark, mut transform) in &mut sparks {
        spark.0 = spark.0.saturating_sub(1);
        transform.scale *= 0.82;
        if spark.0 == 0 {
            commands.entity(entity).despawn();
        }
    }
}

/// **A tier by number or name**, for the examples' `--tier` flag: `0` or `stylised`, `1` or `blood`,
/// `2` or `gore`, `3` or `gross`.
pub fn tier_named(name: &str) -> Option<bevy_carnage::GoreTier> {
    use bevy_carnage::GoreTier::{Blood, BloodAndGore, GrossViolence, Stylised};
    match name.to_ascii_lowercase().as_str() {
        "0" | "stylised" => Some(Stylised),
        "1" | "blood" => Some(Blood),
        "2" | "gore" => Some(BloodAndGore),
        "3" | "gross" => Some(GrossViolence),
        _ => None,
    }
}

/// Which thickness row a part's cut faces and canvases use.
pub fn region_of(part: usize) -> Region {
    match part {
        0 => Region::Torso,
        1 => Region::Head,
        _ => Region::Limb,
    }
}

/// **The exposure the preset scene is lit for**: EV100 11.5, where a white surface facing the 11 000
/// lux sun renders at 1. At Bevy's default 9.7 it rendered at 3.6, so the skin, authored as a plausible
/// 0.80 0.62 0.53, came out nearly white. Everything the preset paints was lifted the same way: wounds
/// went pink, stains went pink, and dried stains went paler rather than browner (R1-07). The props keep
/// their albedos and now render as authored.
pub const EXPOSURE: bevy::camera::Exposure = bevy::camera::Exposure { ev100: bevy::camera::Exposure::EV100_BLENDER + 1.8 };

/// **The ambient that goes with [`EXPOSURE`]**: the old 260 raised by the same 3.5, so the side the sun
/// does not reach renders as it did rather than three and a half times darker. The preset scenes'
/// cameras no longer use it (see [`camera_light`]); the scenes that keep a uniform ambient do.
pub const AMBIENT: f32 = 910.0;

/// **The studio's mean radiance**, over every direction, in an ambient's units: tuned so the lit front
/// of the body renders as it did under [`AMBIENT`]. The room sends more of its light toward what faces
/// the camera and the ceiling than a uniform sky does, so at the ambient's own 910 the clips came out
/// a fifth brighter and flatter.
pub const STUDIO: f32 = 480.0;

/// **What the preset scenes' camera is lit with**: [`EXPOSURE`], and the studio
/// (`common::studio::environment`) at [`STUDIO`], with no uniform ambient on top. A uniform ambient is
/// a sky of one brightness from every direction, and a wet film's coat mirrored it: runs of blood read
/// as paint and a pool at a grazing angle as a pale veil. In a room the runs carry the softbox's
/// reflection and a pool mirrors a dark wall.
pub fn camera_light(images: &mut Assets<Image>) -> (AmbientLight, bevy::camera::Exposure, bevy::light::EnvironmentMapLight) {
    (AmbientLight { brightness: 0.0, ..default() }, EXPOSURE, super::studio::environment(images, STUDIO))
}

/// **Where the windowed demos' camera stands, `seconds` in**: a slow sweep across the front, never an
/// orbit.
///
/// Every hit the scene scripts lands on the body's front, and the sun stands front-left, so a full
/// orbit spent half of every lap on the side where no wound shows and only the fill lights the body:
/// one flat brown shape (R1-12). It also passed behind the sheet, which stands front-right, and lost
/// the body below the shoulders for seconds at a time (R1-13). The sweep stays between 70° left and
/// 5° right of straight ahead: every wound is lit and in view, and the sheet stands beside the body.
/// Past about 10° right it starts to cover the right arm and the scalded thigh. One swing takes 25 s.
pub fn sweep(seconds: f32) -> Transform {
    view(-0.57 + 0.65 * (seconds * 0.25).sin())
}

/// The camera on the sweep's circle, `angle` radians right of straight ahead. Aimed low and in front
/// of the body rather than at its chest, because the spatter a torso shot throws lands on the floor
/// in front of it, and aimed at the chest the frame cut most of it off at the bottom.
pub fn view(angle: f32) -> Transform {
    Transform::from_xyz(2.9 * angle.sin() + 0.3, 1.5, 2.9 * angle.cos())
        .looking_at(Vec3::new(0.15, 0.55, 0.3), Vec3::Y)
}

/// Spawn everything. `skin` is the body's own material; the preset dresses over it.
pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, images: &mut Assets<Image>) {
    // **Textured under a white factor**, the way a glTF exporter writes a textured character, so the
    // demo exercises what a game hands the preset: the wounds open in an authored skin and leave the
    // rest of it as it was.
    let skin = materials.add(StandardMaterial {
        base_color_texture: Some(skin_texture(images)),
        perceptual_roughness: 0.55,
        ..default()
    });

    // The bake, once: the same blockout every other demo cuts, at the finest frontier.
    let subject = body::subject();
    let parts: Vec<(&Mesh, Mat4)> = subject.iter().map(|(m, x)| (m, *x)).collect();
    let proxy = body::proxy();
    let cut = CutSettings { soften: 0.0, ..CutSettings::new(body::TARGET, 0.08, SEED) };
    let fracture = fracture_mesh(&parts, &proxy, &cut);
    let breakable = GoreBreakable::from_fracture(fracture, Region::Limb, &Scale::default());

    commands
        .spawn((Transform::from_translation(ORIGIN), Visibility::default(), breakable, Body))
        .with_children(|root| {
            for (i, (_, centre, half)) in body::parts().into_iter().enumerate() {
                // A cuboid's atlas spans each face from 0 to 1, so its UV density is one over the
                // face's longer edge.
                let face_m = half.x.max(half.y) * 2.0;
                root.spawn((
                    Mesh3d(meshes.add(Cuboid::new(half.x * 2.0, half.y * 2.0, half.z * 2.0))),
                    MeshMaterial3d(skin.clone()),
                    Transform::from_translation(centre),
                    // A `Cuboid`'s front face grows UV `v` up the face, so down is `-v` there; with
                    // the glTF default `(0, 1)` every wound on this body bled upward.
                    Gore::skin(region_of(i)).with_uv_per_metre(1.0 / face_m).with_gravity_uv(Vec2::new(0.0, -1.0)),
                    Part(i),
                ));
            }
        });
    spawn_set(commands, meshes, materials);
}

/// **The set the preset's body stands in**: the sheet hanging in the spray, the slab, and the sun and
/// its fill. Split out of [`spawn`] so another body can stand in the same light, on the same floor,
/// beside the same sheet — which is how `skinned` and `capture_skinned` are lit.
pub fn spawn_set(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    // The slab, at a concrete's albedo. It was 0.34 0.33 0.32, about 9 % — dark asphalt — which only
    // read as pale because `EXPOSURE` was not set; exposed for the light, blood needs a lighter
    // surface than that to read against.
    let slab = (
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(1.5)).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.55, 0.54, 0.52),
            perceptual_roughness: 0.9,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Gore::floor().with_uv_per_metre(1.0 / 3.0),
        Slab,
    );
    spawn_set_on(commands, meshes, materials, slab);
}

/// [`spawn_set`] with another floor in the slab's place: the sheet and the lights, and `slab`.
pub fn spawn_set_on(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, slab: impl Bundle) {
    // A sheet to the right, hanging in the spray.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Z, Vec2::new(0.28, 0.45)).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.72, 0.68, 0.56),
            perceptual_roughness: 0.95,
            // A sheet has two sides and blood soaks through a thin one.
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        // In the spray: forward-right of the body, turned to face it.
        Transform::from_xyz(0.75, 0.75, 0.55).looking_to(Vec3::new(0.6, 0.0, 0.8), Vec3::Y),
        Gore::cloth().with_uv_per_metre(1.0 / 0.9),
        Sheet,
    ));

    commands.spawn(slab);

    commands.spawn((
        DirectionalLight { illuminance: 11_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(-1.5, 4.0, 3.0).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
        Sun,
    ));
    // **A fill from the far side**, a quarter of the sun and unshadowed. With the sun alone every face
    // turned from it saw only ambient, and from the back the whole body read as one flat dark brown
    // (R1-12). The demos no longer go round the back (see [`sweep`]), but the faces the sun misses on
    // the right still read as shaped rather than flat. Marked `Sun` so a reset despawns it with the
    // rest of the scene.
    commands.spawn((
        DirectionalLight { illuminance: 2_750.0, shadow_maps_enabled: false, ..default() },
        Transform::from_xyz(1.5, 2.5, -3.0).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
        Sun,
    ));
}

/// **A tiled floor, as a level's is**: the slab's 3 m, its `UV_0` repeating a sheet of sixteen
/// ceramic tiles two and a half times each way, so the tiles are 30 cm, and dressed with
/// `Gore::projected` — its UVs run to 2.5, which the canvas cannot hold — at 512 a side, 5.9 mm a
/// texel, so a footprint's heel, waist and forefoot read.
pub fn tiled_slab(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, images: &mut Assets<Image>) -> impl Bundle {
    let mut mesh = Plane3d::new(Vec3::Y, Vec2::splat(1.5)).mesh().build();
    if let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0) {
        uvs.iter_mut().for_each(|uv| *uv = [uv[0] * 2.5, uv[1] * 2.5]);
    }
    (
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(tile_texture(images)),
            perceptual_roughness: 0.6,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Gore::floor().projected().with_canvas(512),
        Slab,
    )
}

/// **Sixteen ceramic tiles and their grout**, 256² and seamless when repeated: each tile 64 texels, a
/// pale warm grey a few per cent lighter or darker than its neighbours and faintly mottled, its edge a
/// shade darker, in grout two texels wide. Mipmapped in linear light to a single texel, and
/// anisotropically filtered, because a floor seen low squeezes a tile into a few pixels and an
/// unfiltered grout line crawls.
fn tile_texture(images: &mut Assets<Image>) -> Handle<Image> {
    const SIZE: u32 = 256;
    const TILE: u32 = 64;
    let mut linear = Vec::with_capacity((SIZE * SIZE * 3) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (tx, ty) = (x / TILE, y / TILE);
            let (ix, iy) = (x % TILE, y % TILE);
            let tint = 1.0 + 0.07 * (hash(tx as i32, ty as i32, 21) - 0.5);
            // Mottled over the whole sheet, so it repeats seamlessly with it: the lattice wraps at 16.
            let (u, v) = (x as f32 / SIZE as f32 * 16.0, y as f32 / SIZE as f32 * 16.0);
            let mottle = 1.0 + 0.05 * (wrapped_noise(u, v, 16, 22) - 0.5);
            let from_edge = ix.min(TILE - 1 - ix).min(iy).min(TILE - 1 - iy);
            let colour = if from_edge < 1 {
                [0.30, 0.29, 0.27]
            } else {
                let lip = if from_edge < 3 { 0.93 } else { 1.0 };
                [0.62, 0.60, 0.56].map(|c| c * tint * mottle * lip)
            };
            linear.extend(colour.map(|c: f32| srgb_to_linear(c.clamp(0.0, 1.0))));
        }
    }
    // The chain, every level a box filter of the one above, in linear light.
    let mut data: Vec<u8> = Vec::new();
    let (mut level, mut edge, mut levels) = (linear, SIZE, 0u32);
    loop {
        for rgb in level.chunks_exact(3) {
            data.extend(rgb.iter().map(|c| (linear_to_srgb(*c) * 255.0).round() as u8));
            data.push(255);
        }
        levels += 1;
        if edge == 1 {
            break;
        }
        let half = edge / 2;
        let mut next = Vec::with_capacity((half * half * 3) as usize);
        for y in 0..half {
            for x in 0..half {
                for c in 0..3 {
                    let at = |dx: u32, dy: u32| level[(((2 * y + dy) * edge + 2 * x + dx) * 3 + c) as usize];
                    next.push(0.25 * (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)));
                }
            }
        }
        level = next;
        edge = half;
    }
    let mut image = Image::new_uninit(
        Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    images.add(image)
}

/// Value noise in `[0, 1]` whose lattice wraps every `period` cells, so a texture of it tiles.
fn wrapped_noise(x: f32, y: f32, period: i32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let h = |i: i32, j: i32| hash(i.rem_euclid(period), j.rem_euclid(period), seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * sx;
    let bottom = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * sx;
    top + (bottom - top) * sy
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// The wall behind the body: what [`spawn_wall`] puts up.
#[derive(Component)]
pub struct Backdrop;

/// **A wall behind the body**, for the shots that go through it: plaster, three metres wide, at the
/// back of the slab, dressed as [`Gore::wall`] so the exit's forward spatter lands on it and runs.
/// Its canvas is 512 a side, since a spatter dot at the floor's 12 mm a texel is a smudge.
pub fn spawn_wall(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Z, Vec2::new(1.5, 1.2)).mesh().build())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.74, 0.72, 0.68),
            perceptual_roughness: 0.85,
            ..default()
        })),
        Transform::from_xyz(0.0, 1.2, -1.2),
        Gore::wall().with_canvas(512),
        Backdrop,
    ));
}

/// The skin's tone, sRGB: the flat colour this scene's body wore before it had a texture.
const SKIN: [f32; 3] = [0.80, 0.62, 0.53];

/// **A skin texture, made here so the demo needs no asset**: [`SKIN`] mottled at two scales, under a
/// redder flush. 256² because every face of a part maps the whole image, so a torso's front gets
/// about 2.3 mm a texel, and no finer detail survives the clips' framing. Freckles were tried and
/// dropped: at that framing each was one dark pixel, and the skin read as dirty.
pub fn skin_texture(images: &mut Assets<Image>) -> Handle<Image> {
    const SIZE: u32 = 256;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (u, v) = ((x as f32 + 0.5) / SIZE as f32, (y as f32 + 0.5) / SIZE as f32);
            // Blotches a few centimetres across, a redder flush over larger patches, and a fine grain.
            let blotch = (noise(u * 7.0, v * 7.0, 11) + 0.5 * noise(u * 15.0, v * 15.0, 12)) / 1.5 - 0.5;
            let flush = noise(u * 3.0, v * 3.0, 13) - 0.5;
            let grain = noise(u * 60.0, v * 60.0, 14) - 0.5;
            let shade = 1.0 + 0.16 * blotch + 0.05 * grain;
            let tone = [SKIN[0] * shade + 0.05 * flush, SKIN[1] * shade - 0.02 * flush, SKIN[2] * shade - 0.02 * flush];
            data.extend(tone.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
            data.push(255);
        }
    }
    images.add(Image::new(
        Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

/// Smooth value noise in `[0, 1]` over an integer lattice.
fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = hash(ix, iy, seed) + (hash(ix + 1, iy, seed) - hash(ix, iy, seed)) * sx;
    let bottom = hash(ix, iy + 1, seed) + (hash(ix + 1, iy + 1, seed) - hash(ix, iy + 1, seed)) * sx;
    top + (bottom - top) * sy
}

/// A lattice point's value in `[0, 1)`.
fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// A ray onto the front face of a part, offset from its centre by `offset` (subject-local).
pub fn ray_at(part: usize, offset: Vec3) -> (Vec3, Vec3) {
    let (_, centre, half) = body::parts().get(part).copied().unwrap_or(("", Vec3::ZERO, Vec3::splat(0.1)));
    let world = ORIGIN + centre + offset;
    (world + Vec3::new(0.0, 0.0, half.z + 1.0), Vec3::NEG_Z)
}

/// The scripted shot on the torso: **the same spot every time**, so the crater deepens to bone. Each
/// shot removes 9 mm; the torso row's cortex starts at 25.5 mm (2.2 skin + 16.0 fat + 7.3 muscle), so
/// the third shot is the one that shows bone and hands off. A hair of jitter looked more natural and
/// meant the third shot never did — caught in review.
pub fn shot(torso: Entity, _n: u32) -> GoreHit {
    let (from, dir) = ray_at(0, Vec3::new(0.0, 0.06, 0.0));
    GoreHit::impact(torso, from, dir, 0.035, 9.0)
}

/// A shallow graze high on the chest, away from where [`shot`] lands, so the torso bleeds from two
/// wounds at once: the graze keeps bleeding after the shots open the second.
pub fn graze(torso: Entity) -> GoreHit {
    let (from, dir) = ray_at(0, Vec3::new(-0.12, 0.18, 0.0));
    GoreHit::impact(torso, from, dir, 0.02, 4.0)
}

/// A blow to the upper left arm: a bruise, no blood.
pub fn blow(arm: Entity) -> GoreHit {
    let (from, dir) = ray_at(2, Vec3::new(0.0, 0.08, 0.0));
    GoreHit::blunt(arm, from, dir, 0.03)
}

/// A hot iron against the right thigh: 150 °C for a second.
pub fn scald(leg: Entity) -> GoreHit {
    let (from, dir) = ray_at(5, Vec3::new(0.0, 0.1, 0.0));
    GoreHit::burn(leg, from, dir, 0.035, 150.0, 1.0)
}

/// A slash across the left thigh, gaping onto its bed.
pub fn slash(leg: Entity) -> GoreHit {
    let (from, dir) = ray_at(4, Vec3::new(0.0, 0.05, 0.0));
    GoreHit::slash(leg, from, dir, Vec3::new(0.7, -0.7, 0.0).normalize(), 0.06, 8.0)
}
