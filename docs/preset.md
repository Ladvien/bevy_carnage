# `bevy_carnage::preset` — putting the gore on your character

![The preset on a blocked-out body: shots crater and bleed, blows bruise, the body breaks along its fracture modes and the pieces fall](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/preset.gif)

The preset is the two-line inclusion: `GorePlugin`, a `Gore` on every surface that can be hurt, and a
`GoreHit` per injury. The README shows those lines. This page is what they assume, written for a
real character rather than the demos' cuboids. Every item below is something that, done wrong, gives
a plausible-looking result that is quietly wrong.

## The checklist

| | What | Why it matters |
|---|---|---|
| 1 | **Run `FixedUpdate` at `GoreDials::hz`.** Bevy's default fixed step is 64 Hz and the preset's `hz` is 60: `app.insert_resource(Time::<Fixed>::from_hz(60.0))`, or set `GoreDials::hz` to your step. Since 0.5.26 the preset says so, once, in the log on its first tick, when the step it finds is not `hz`. | The preset ticks once per `FixedUpdate`. Bleeds, bruise ageing, drying and the cloth's wicking are measured in ticks of `hz`, so a mismatch runs every one of them fast or slow: at Bevy's 64 Hz, 6.7 per cent fast. |
| 2 | **Put `Gore` on the entity that holds the `Mesh3d`**, not on the scene root, or put a `GoreScene` on the root and let it do that for you (below). A glTF scene's meshes sit under node entities. | `Gore` dresses the mesh on its own entity. `GoreHit::entity` may be that entity or any ancestor of it (see 7). |
| 3 | **Give every dressed mesh a `UV_0` that covers it once, inside `[0, 1]`.** No mirrored or overlapping islands on skin. | The canvases live in UV space. The flaymap refuses a UV outside the unit square with one warning; the wetmap clamps it to the edge. A mirrored island paints a wound on both limbs it maps. |
| 4 | **Let the preset measure how big the atlas is, or say so: `Gore::with_uv_per_metre`.** Unstated, it is measured when the mesh is dressed: the square root of the mesh's UV area over its world area, at its world scale, stored back in `Gore::uv_per_metre`. State it when that average is wrong where the wounds land, as on an atlas that packs the face far denser than the body. | Hit radii are in metres and canvases are in UV, so the density converts one to the other. Before 0.5.18 an unstated density was `1.0`, and on a densely packed atlas a 3 cm wound was a dot. |
| 5 | **Say which way blood runs: `Gore::with_gravity_uv`.** `(0, 1)`, the default, is down on a glTF atlas whose islands stand upright. | A wrong sign is invisible in a still and obvious in motion: the drips climb. Bevy's own primitives disagree: a `Cuboid`'s front face wants `(0, -1)`, and a `Sphere::mesh().uv(..)` must be turned pole-up first. |
| 6 | **State the scale once: `GoreDials::mm_per_unit`.** `1000` for a game in metres. A game in centimetres also sets `CrossSectionSettings::scale` and `FlaySettings::scale` to match, before adding the plugin. | Tissue depths are millimetres, and the cut faces, the flaymap's grain and the preset each convert from mesh units. |
| 7 | **Aim `GoreHit` with a ray, and address it to the character.** `from` is the ray's origin in world space and `dir` its direction. The preset re-casts it against what is drawn: the mesh, or, on a skinned mesh, the pose on screen. Since 0.5.22 `GoreHit::entity` may be **any ancestor of the dressed meshes**, so send the hit at whatever your physics found (the character, or the capsule's owner), and the preset hurts the nearest `Gore::skin` under it along the ray. A mesh with its own `Gore` is still hurt directly. | A physics hit point is not enough on its own. Send the ray your weapon fired, or one from just outside the surface along the hit normal reversed. Bevy's `MeshRayCast` tests a skinned mesh's bind pose, so it can miss a posed limb or find the wrong part: do not use it to pick the mesh. Clothing is never the nearest, because a wound opens on skin and the spray soaks the cloth. |
| 8 | **Choose who owns the pieces: `GoreDials::spawn_fragments`.** `true` throws them ballistically to `GoreDials::floor_y`. A game with physics sets `false`, reads `GoreFractured`, gives each of its `pieces` a rigid body, and despawns or hides the body itself. | The preset's pieces are kinematic and fall to one plane. They will fight a physics engine. |
| 9 | **Gate the bake on your own state.** The plugin adds its systems with no run condition. | The caller owns the schedule, everywhere in this crate. |

Tangents are handled for you since 0.5.10: a skin mesh without `ATTRIBUTE_TANGENT` gets them when it
is dressed, because the crater's normal map needs them.

## Dressing a loaded scene

Since 0.5.24 a `GoreScene` on the entity a glTF scene is spawned under dresses every mesh that appears
below it, whenever it appears, so there is no system of your own to walk the hierarchy:

```rust,ignore
commands.spawn((
    WorldAssetRoot(assets.load("enemy.glb#Scene0")),
    GoreScene::humanoid()                       // `Head` is a head, `Arm_L` a limb, `Shirt` cloth, `Gun` left alone
        .with("tabard", Gore::cloth())          // rules added after it win
        .with("torso", Gore::skin(Region::Torso).with_uv_per_metre(2.0)),
));
```

- **A name is read as words.** `ArmL`, `Arm_L` and `arm.L` are the words `arm` `l`; a rule matches when
  its words are a run of the name's, so `"arm"` matches all three and **not** `Armature`, the node a
  Blender export puts above its meshes, nor `Armor`. The scene root's own name is ignored.
- **A mesh is matched on its own name and then on each node above it, nearest first**, so `Mesh.001`
  under a node called `Head` is the head. That is what Bevy's glTF loader gives you: a node entity is
  named for its node, and each primitive is a child of it named `{mesh}.{material}`, so **a material
  called `shirt` dresses its primitive as cloth wherever the node sits**. The last rule that matches
  wins. A mesh nothing matches gets the scene's default: left alone for `humanoid`, whatever you
  gave `skin` or `new` otherwise.
- **A mesh with its own `Gore` is never touched**, so one surface can be dressed by hand with a stated
  density or gravity and the rest by name. The wound bed a slash opens under a dressed skin
  (`LacerationBed`) is the preset's own and is never dressed.
- `GoreScene::skin(region)` and `GoreScene::new(gore)` start from one default for every mesh, and
  `.without(name)` leaves a prop or a held weapon alone.

**`humanoid()` sorts a real character into a body, a wardrobe and gear.** A glTF character is not one
skin: it is a body mesh with clothing over it and gear on it. `humanoid()` knows the words a character
kit uses. They were read from the node and material names in the glTF files of six characters and a
creature from the game this crate is developed in (`valkyrie`, `fieldop`, `makarov`, `scientist`,
`researcher`, `cipher`, `scp-150`), and run in the engine on one of them, `valkyrie`:

| class | words it reads | what the preset does |
|---|---|---|
| skin | `body`, `skin` (a kit's body material), `torso`, `chest`; `head`, `face`; `arm`, `leg`, `hand`, `foot`, `thigh`, `shin`... | wounded, flayed, bleeds: what a hit hurts |
| cloth | `bodysuit`, `jumpsuit`, `labcoat`, `vest`, `shirt`, `boots`, `gloves`, `base`... | soaks the blood; never the thing a hit peels |
| gear | `belt`, `holster`, `pouch`, `backpack`, `chestrig`, `kneepads`, `eyewear`, `facewear`, `laces`, `buttons`, `hair`, every weapon | left alone; a rule here beats the others, so `thigh_holster` is a holster |
| anything else | | **left alone**: on a real character it is almost always gear |

The first version dressed the unknown as torso skin. On `valkyrie.glb` that dressed 19 of 22 meshes as
skin, a chest rig and a belt among them, and a shot at the chest wounded the rig in front of the body
it hangs on. Now one is skin, three are cloth and 18 are left alone, and the same shot soaks the
bodysuit. The cost is that a character whose body mesh has a name it does not know is not dressed at
all: name it, `.with("thatname", Gore::skin(Region::Torso))`. `humanoid()` is a name heuristic, which is
all a glTF offers; rename what it gets wrong rather than trusting it blind.

**What it does not do yet: a hole in the cloth.** Clothing is not something a hit stops at, so the
body under a suit is wounded and the suit is stained by the blood that comes through it. On a
character in a full bodysuit the wound itself is never seen, only its soak.

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
| `GoreDials::canvas`, `floor_canvas`, `Gore::canvas` | texels per canvas edge; 128 and 256 ship, and 256 is the ceiling. `Gore::with_canvas` gives one surface its own |
| `GoreDials::floor_y`, `gravity` | world units, world units per second squared |

A torso's skin and fat are 18 mm deep and its muscle wall reaches 25.5 mm, so an impact of 5 mm opens
skin and fat, 20 mm reaches muscle and spills guts, and a run of hits that passes 25.5 mm shows bone
and breaks a body that can break.

## The player's settings

![One scripted scene three times: at the stylised tier a yellow spark marks each shot and nothing bleeds; at the blood tier the wounds bleed and pool and the body stays whole; at blood and gore the body comes apart](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/tiers.gif)

A violence setting is a rating requirement in some markets and an accessibility one in most, and
since 0.5.21 the preset follows it. `GorePolicy` is read at every hit, so a settings menu replaces
the resource and the next hit obeys it; nothing already painted is taken back.

```rust,ignore
// Before or after the plugin: the plugin only `init_resource`s one.
app.insert_resource(GorePolicy::for_tier(GoreTier::Blood));
// Later, from a menu:
*policy = GorePolicy::for_tier(GoreTier::Stylised);
```

| `GorePolicy` | what the preset does |
|---|---|
| `GoreTier::Stylised` (`draws_blood()` is false) | A hit that would have opened a wound is found and announced as `Wounded`, at the same point with the same normal and area, and nothing is painted, torn, spilled or thrown. **The game puts a spark, a flash or a sound where the blood would have been.** A blow's bruise and a burn's eschar are not blood and are unchanged. |
| `blood_decals: false` | The body takes the wound and bleeds on itself. Nothing reaches the world: not the spray on the floor, a sheet or the next body, and not the drips off the wound. |
| `viscera: false` | A torso opened to the muscle spills nothing. |
| `dismemberment: false` | The first hit to show cortex still forwards `BoneExposed`, and the body does not break: no `GoreFractured`, no pieces. |

`GorePolicy::for_tier` sets the switches that go with a tier: `Blood` is `viscera` and
`dismemberment` off, `BloodAndGore` and `GrossViolence` are all on, and `Stylised` turns all three
off as well. The tier on its own is documentation of intent, except that `Stylised` is what
`draws_blood()` reads. The preset does not read the rest of the policy: `persistence_scale`, because
the wetmap's blood dries and never fades by design, and `screen_blood`, `ragdolls`, `shake_scale`,
the flash limit and the aim cone, because they belong to the code that draws what the preset does
not. The default is `BloodAndGore`, with every switch on, so a game that never inserts a policy
sees exactly what it saw before 0.5.21.

`examples/preset.rs` has the setting on `T`, and `capture_preset -- --tier 0|1|2` replays the clip
under a tier; it draws the spark at `Stylised` from the `Wounded` message, the way a game would.

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

## What it costs

Milliseconds one `FixedUpdate` takes when it takes one hit, and when it takes a shotgun's twelve at
one body, by how much is dressed. Release build, one Apple M5. Each part is a UV sphere, six to a
character, closer together than a body's parts are, so the spray reaches more of them than it would.
An idle tick in the same scenes is 0.03 to 0.06 ms. The left figure of each pair is 0.5.19, the right
is 0.5.20.

| dressed | one hit | twelve hits |
|---|---|---|
| 1 character, 6 parts of 2,208 triangles | 0.91 → 0.66 | 4.81 → 2.37 |
| 1 character, 6 parts of 16,128 triangles | 1.70 → 0.74 | 19.44 → 3.10 |
| 10 characters, 60 surfaces | 2.99 → 0.92 | 28.60 → 2.58 |
| 40 characters, 240 surfaces | 9.91 → 1.35 | 129.10 → 26.53 |

What was slow was the spray: each hit casts 24 droplets at every other dressed surface in the world,
and each cast walked every triangle of it. A droplet is now tested against that surface's bounding
box first, and cast only if its ray can enter it. The box is one pass over the vertices once per
batch of hits, so a shotgun pays for it once. Nothing it skips could have landed, and the clips are
byte-identical.

What is left at 240 surfaces is the blood itself. Profiled while hits kept landing on 240
canvases, `WetCanvas::shade`, `spread`, `tick` and `drip` were nearly all of the main thread and
`ray_uv` was under two samples in a hundred. A wetmap costs what its blood covers, so a game that
dresses only the characters near the player pays for those. The measurement is the ignored test
`hit_cost_by_scene_size`: `cargo test -p bevy_carnage --lib --release -- hit_cost_by_scene_size
--ignored --nocapture`.

## What it costs in memory

Every dressed surface owns canvases, and a canvas is a few bytes a texel: the table is what one costs,
read off the canvases themselves (`heap_bytes` and `image_bytes` on `WetCanvas`, `FlayCanvas` and
`DermisCanvas`; the ignored test `dressed_surface_memory` prints it). *Main heap* is what the canvas
owns on the CPU. *Images* is the size of what it uploads, which is held once in `Assets<Image>` and
once on the GPU, so the total counts it twice. The sizes do not depend on the build profile. The stamp
scratch a first wound allocates is on top.

| surface | edge | main heap | images, once | total |
|---|---|---|---|---|
| skin (flaymap, wetmap, dermis map) | 64 | 136 KiB | 96 KiB | 328 KiB |
| skin | **128**, ships | 544 KiB | 384 KiB | **1.3 MiB** |
| skin | 256 | 2.1 MiB | 1.5 MiB | 5.1 MiB |
| cloth (wetmap) | 128 | 256 KiB | 128 KiB | 512 KiB |
| floor (wetmap) | **256**, ships | 1.0 MiB | 512 KiB | **2.0 MiB** |

So a character of six skin parts at the shipped 128 is **7.7 MiB**, forty of them 308 MiB, and the
same at 64 is 1.9 MiB and 77 MiB. The cost is the edge squared: it is the dial to turn on a web build
or a crowd. `GoreDials::canvas` sets it for everything, and `Gore::with_canvas` sets it for one
surface, so a head, a hand or a background character can be dressed at 64 while the torso keeps its
128 (`GoreScene::with` takes a `Gore`, so the same works by name).

**64 is not free, and this is what it costs.** The skinned demo was recorded with every part but the
torso at 64 and set beside the shipped clip, the same frame enlarged 4×. A wound's edge is
stair-stepped where it was smooth, steps of three or four pixels at the clip's own size, and the
wound is not quite the same wound: on the thigh it reached higher up the leg, probably because blood
runs on texels twice as wide. Nothing else was seen to change. It is the right trade for a character
the player never sees up close, and the wrong one for the one they shoot.

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
