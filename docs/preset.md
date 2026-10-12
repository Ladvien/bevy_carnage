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
| 4 | **Say how big the atlas is: `Gore::with_uv_per_metre`**, `1 / (metres the atlas spans)`. A 40 cm forearm whose UVs run 0 to 1 is `2.5`. | Hit radii are in metres and canvases assume one UV per metre. Left at the default `1.0`, a 3 cm wound on a densely packed atlas is a dot. |
| 5 | **Say which way blood runs: `Gore::with_gravity_uv`.** `(0, 1)`, the default, is down on a glTF atlas whose islands stand upright. | A wrong sign is invisible in a still and obvious in motion: the drips climb. Bevy's own primitives disagree: a `Cuboid`'s front face wants `(0, -1)`, and a `Sphere::mesh().uv(..)` must be turned pole-up first. |
| 6 | **State the scale once: `GoreDials::mm_per_unit`.** `1000` for a game in metres. A game in centimetres also sets `CrossSectionSettings::scale` and `FlaySettings::scale` to match, before adding the plugin. | Tissue depths are millimetres, and the cut faces, the flaymap's grain and the preset each convert from mesh units. |
| 7 | **Aim `GoreHit` with a ray.** `from` is the ray's origin in world space and `dir` its direction. The preset re-casts it against the mesh to find the UV. | A physics hit point is not enough on its own. Send the ray your weapon fired, or one from just outside the surface along the hit normal reversed. |
| 8 | **Choose who owns the pieces: `GoreDials::spawn_fragments`.** `true` throws them ballistically to `GoreDials::floor_y`. A game with physics sets `false`, reads `GoreFractured`, and spawns its own bodies from the bake. | The preset's pieces are kinematic and fall to one plane. They will fight a physics engine. |
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
- `GoreFractured` — a body came apart: its root and where the blow landed.
- `Wounds`, a component on each hurt part: every wound it is bleeding, each a UV on its wetmap and
  a point on its mesh. A hit away from the open wounds opens another, up to
  `GoreDials::open_wounds` (16), and a hit inside one reopens it. A wound leaves the list once its
  bleed has clotted.

## Known limits

- **A wound is found against the bind pose, once.** The hit's ray is cast against the mesh asset
  placed by the entity's `GlobalTransform`, so on a skinned mesh posed far from its bind pose the
  hit can land somewhere other than where it struck. After that the wound is a spot on the skin:
  it bleeds from there, and drips from where that spot is now, however the body moves.
- **A skin's roughness map and a crater's slope give way.** See *What a dressed skin keeps*.
- **One ageing bruise per mesh entity.** A second blow on the same entity starts a new bruise, and
  the first stops ageing where it was. Wounds have no such limit.
- **One gravity direction per canvas.** On a mesh whose UV islands point different ways, blood runs
  correctly on the islands that agree with `gravity_uv`.

## Examples

```sh
cargo run -p bevy_carnage --release --example preset       # the scene above, keys to hit
cargo run -p bevy_carnage --release --example carnage_web  # the same preset as the web demo
```
