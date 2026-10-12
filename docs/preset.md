# `bevy_carnage::preset` — putting the gore on your character

![The preset on a blocked-out body: shots crater and bleed, blows bruise, the body breaks along its fracture modes and the pieces fall](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/preset.gif)

The preset is the two-line inclusion: `GorePlugin`, a `Gore` on every surface that can be hurt, and a
`GoreHit` per injury. The README shows those lines. This page is what they assume, written for a
real character rather than the demos' cuboids. Every item below is something that, done wrong, gives
a plausible-looking result that is quietly wrong.

## The checklist

| | What | Why it matters |
|---|---|---|
| 1 | **Run `FixedUpdate` at `GoreDials::hz`.** Bevy's default fixed step is 64 Hz and the preset's `hz` is 60: `app.insert_resource(Time::<Fixed>::from_hz(60.0))`, or set `GoreDials::hz` to your step. | The preset ticks once per `FixedUpdate`. Bleeds, bruise ageing, drying and the cloth's wicking are measured in ticks of `hz`, so a mismatch runs every one of them fast or slow. |
| 2 | **Put `Gore` on the entity that holds the `Mesh3d`**, not on the scene root. A glTF scene's meshes sit under node entities; walk the spawned scene's descendants and dress each mesh that can be hurt. | `Gore` dresses the mesh on its own entity, and `GoreHit::entity` must be that same entity. |
| 3 | **Give every dressed mesh a `UV_0` that covers it once, inside `[0, 1]`.** No mirrored or overlapping islands on skin. | The canvases live in UV space. The flaymap refuses a UV outside the unit square with one warning; the wetmap clamps it to the edge. A mirrored island paints a wound on both limbs it maps. |
| 4 | **Let the preset measure how big the atlas is, or say so: `Gore::with_uv_per_metre`.** Unstated, it is measured when the mesh is dressed: the square root of the mesh's UV area over its world area, at its world scale, stored back in `Gore::uv_per_metre`. State it when that average is wrong where the wounds land, as on an atlas that packs the face far denser than the body. | Hit radii are in metres and canvases are in UV, so the density converts one to the other. Before 0.5.18 an unstated density was `1.0`, and on a densely packed atlas a 3 cm wound was a dot. |
| 5 | **Say which way blood runs: `Gore::with_gravity_uv`.** `(0, 1)`, the default, is down on a glTF atlas whose islands stand upright. | A wrong sign is invisible in a still and obvious in motion: the drips climb. Bevy's own primitives disagree: a `Cuboid`'s front face wants `(0, -1)`, and a `Sphere::mesh().uv(..)` must be turned pole-up first. |
| 6 | **State the scale once: `GoreDials::mm_per_unit`.** `1000` for a game in metres. A game in centimetres also sets `CrossSectionSettings::scale` and `FlaySettings::scale` to match, before adding the plugin. | Tissue depths are millimetres, and the cut faces, the flaymap's grain and the preset each convert from mesh units. |
| 7 | **Aim `GoreHit` with a ray.** `from` is the ray's origin in world space and `dir` its direction. The preset re-casts it against the mesh to find the UV. On a skinned mesh it re-casts against the pose on screen, so send the ray at what the player sees. | A physics hit point is not enough on its own. Send the ray your weapon fired, or one from just outside the surface along the hit normal reversed. Bevy's `MeshRayCast` tests a skinned mesh's bind pose, so a posed limb can be missed or the wrong part found: pick the part with your physics' per-bone colliders and let the preset aim the ray. |
| 8 | **Choose who owns the pieces: `GoreDials::spawn_fragments`.** `true` throws them ballistically to `GoreDials::floor_y`. A game with physics sets `false`, reads `GoreFractured`, gives each of its `pieces` a rigid body, and despawns or hides the body itself. | The preset's pieces are kinematic and fall to one plane. They will fight a physics engine. |
| 9 | **Gate the bake on your own state.** The plugin adds its systems with no run condition. | The caller owns the schedule, everywhere in this crate. |

Tangents are handled for you since 0.5.10: a skin mesh without `ATTRIBUTE_TANGENT` gets them when it
is dressed, because the crater's normal map needs them.

## What a dressed skin keeps

Since 0.5.15 a skin keeps the material it was authored with: its colour factor, its
`base_color_texture`, the mesh's vertex colours, its occlusion and emissive maps, and its own
normal map. The wounds are drawn over that skin, and the pieces a broken body throws wear the hit
part's material as it was before the preset dressed it. Two things are the flaymap's. The skin's
metallic-roughness texture is replaced by the flaymap's roughness image, so intact skin takes its
roughness from the material's `perceptual_roughness` alone. And a skin that brings its own normal
map keeps it, so its craters shade without the slope of their walls. Neither gets a texture slot of
its own, because every texture the flesh material adds counts against WebGL2's sixteen per stage.

## Skinned characters

Since 0.5.19 a hit lands on the body **as it is drawn**. A mesh is skinned, to the preset, when its
entity carries a `SkinnedMesh` and the mesh has joint indices and weights: the two things Bevy's
renderer asks for. Nothing is asked of the game. Put `Gore` on the skinned mesh as on any other, and:

- **The ray is cast at the pose.** `bevy_carnage::skin` blends each vertex's four joint matrices by
  its weights exactly as `bevy_pbr`'s vertex shader does, from the joints' `GlobalTransform`s as the
  last `PostUpdate` propagated them, which are the matrices the frame on screen was drawn with. The
  mesh entity's own transform plays no part, as in rendering. Before 0.5.19 the ray was cast at the
  mesh asset, which is the bind pose, so a shot at a raised forearm missed it and did nothing.
- **A wound is a triangle and a place in it** (`SkinAnchor`), not a point. Each pulse asks where that
  spot of skin is under the pose the body has now, so the drip falls from the wound however the limb
  moves. `WoundSite::at_local` is the same spot on the bind pose.
- **Blood runs downhill in the world.** While a skinned part has an open wound, `RunDirection` is
  rewritten each tick: the UV direction whose image on the surface points straight down, averaged
  over the triangles its wounds are on, and none where the surface is within about seven degrees of
  level. A forearm held up has its blood run toward the elbow however its atlas is laid out.
- **A slash is laid out on the bind pose**, which is what the laceration tears: the point, the face
  normal, the direction of travel and the length are carried back through the pose at the triangle
  that was hit.
- **A bruise and a burn live in UV space**, and so ride the limb with no extra work.
- **The atlas density is measured from the pose**, because the scale of a rigged character lives in
  its joints, not in the mesh entity.
- **A skin with no pose yet is not cast at the bind pose.** While the joints have no transform, or
  the inverse bind poses have not loaded, a hit on it is dropped with one warning, and a part whose
  density is unstated waits to be dressed.

`examples/skinned.rs` is that, on a character made in code: eleven joints, six skinned meshes sharing
them, a walk, and a wave that holds the left forearm above the head. Every key aims at where that
part of the body is drawn that frame.

## Units, in one place

| Field | Unit |
|---|---|
| `GoreHit::radius_m` | metres on the surface: the crater's, bruise's or burn's radius, or half a slash's length |
| `HitKind::Impact::depth_mm`, `Slash::depth_mm` | millimetres of tissue removed at the centre |
| `HitKind::Burn::{temp_c, seconds}` | contact temperature, °C, and contact time, s |
| `GoreDials::hz` | fixed ticks per second |
| `GoreDials::canvas`, `floor_canvas` | texels per canvas edge; 128 and 256 ship, and 256 is the ceiling |
| `GoreDials::floor_y`, `gravity` | world units, world units per second squared |

A torso's skin and fat are 18 mm deep and its muscle wall reaches 25.5 mm, so an impact of 5 mm opens
skin and fat, 20 mm reaches muscle and spills guts, and a run of hits that passes 25.5 mm shows bone
and breaks a body that can break.

## What you get back

- `Wounded` — every open wound: where, its normal, its area, and the bloodstain pattern class. Drive
  hit feedback, sound or a decal budget from it.
- `BoneExposed` — once per canvas, the first hit to reach the cortex.
- `GoreFractured` — a body came apart, once: its root, where the blow landed, the material for each
  side of a piece, and every piece. Each `GorePiece` has its skin and cut-face meshes, its world
  transform at the break, its half extents for a collider, the velocity the preset would launch it
  at, and its fate: thrown by the blow, fallen with a body that lost its footing, or still standing.
  Written whether or not the preset spawns the pieces. When it does, they carry `Flying` or
  `Standing`, and a settled piece costs nothing per tick; despawning them is the game's call.
- `Wounds`, a component on each hurt part: every wound it is bleeding, each a UV on its wetmap and
  a point on its mesh. A hit away from the open wounds opens another, up to
  `GoreDials::open_wounds` (16), and a hit inside one reopens it. A wound leaves the list once its
  bleed has clotted.

## Known limits

- **A skinned body that breaks throws its pieces in the bind pose.** The fracture is baked from the
  mesh asset, so a `GoreBreakable` or a baked `FractureSubject` on an animated character is cut
  from the pose it was authored in, and its pieces appear there. A rigid body is unaffected.
- **A skin's roughness map and a crater's slope give way.** See *What a dressed skin keeps*.
- **One ageing bruise per mesh entity.** A second blow on the same entity starts a new bruise, and
  the first stops ageing where it was. Wounds have no such limit.
- **One gravity direction per canvas.** On a rigid mesh whose UV islands point different ways, blood
  runs correctly on the islands that agree with `gravity_uv`. A skinned part follows the pose at its
  wounds instead, and the same limit holds across the islands it has.

## Examples

```sh
cargo run -p bevy_carnage --release --example preset       # the scene above, keys to hit
cargo run -p bevy_carnage --release --example carnage_web  # the same preset as the web demo
cargo run -p bevy_carnage --release --example skinned      # the preset on an animated, skinned body
```
