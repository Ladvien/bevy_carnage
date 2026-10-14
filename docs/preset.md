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
| 3 | **Give every dressed mesh a `UV_0` that covers it once, inside `[0, 1]`.** No mirrored or overlapping islands on skin. **A floor or a wall whose `UV_0` tiles its texture**, as a level's does, is dressed with `Gore::projected()` instead (0.5.57). | The canvases live in UV space. The flaymap refuses a UV outside the unit square with one warning; the wetmap clamps it to the edge, so on a tiled floor every drop lands on the canvas's border — since 0.5.57 the preset says so, once, when it dresses one. A mirrored island paints a wound on both limbs it maps. |
| 4 | **Let the preset measure how big the atlas is, or say so: `Gore::with_uv_per_metre`.** Unstated, it is measured when the mesh is dressed: the square root of the mesh's UV area over its world area, at its world scale, stored back in `Gore::uv_per_metre`. State it when that average is wrong where the wounds land, as on an atlas that packs the face far denser than the body. | Hit radii are in metres and canvases are in UV, so the density converts one to the other. Before 0.5.18 an unstated density was `1.0`, and on a densely packed atlas a 3 cm wound was a dot. |
| 5 | **Say which way blood runs: `Gore::with_gravity_uv`.** `(0, 1)`, the default, is down on a glTF atlas whose islands stand upright. | A wrong sign is invisible in a still and obvious in motion: the drips climb. Bevy's own primitives disagree: a `Cuboid`'s front face wants `(0, -1)`, and a `Sphere::mesh().uv(..)` must be turned pole-up first. |
| 6 | **State the scale once: `GoreDials::mm_per_unit`.** `1000` for a game in metres. A game in centimetres also sets `CrossSectionSettings::scale` and `FlaySettings::scale` to match, before adding the plugin. | Tissue depths are millimetres, and the cut faces, the flaymap's grain and the preset each convert from mesh units. |
| 7 | **Aim `GoreHit` with a ray, and address it to the character.** `from` is the ray's origin in world space and `dir` its direction. If your physics reports a **contact point and its normal** instead, `GoreHit::impact_at(entity, point, normal, ..)` (or `.at_surface(point, normal)` on any hit) starts the ray `SURFACE_STAND_OFF` outside the point and sends it in. The preset re-casts it against what is drawn: the mesh, or, on a skinned mesh, the pose on screen. Since 0.5.22 `GoreHit::entity` may be **any ancestor of the dressed meshes**, so send the hit at whatever your physics found (the character, or the capsule's owner), and the preset hurts the nearest `Gore::skin` under it along the ray. A mesh with its own `Gore` is still hurt directly. | A physics hit point is not enough on its own. Send the ray your weapon fired, or one from just outside the surface along the hit normal reversed. Bevy's `MeshRayCast` tests a skinned mesh's bind pose, so it can miss a posed limb or find the wrong one. |
| 8 | **Choose who owns the pieces: `GoreDials::spawn_fragments`.** `true` throws them ballistically to `GoreDials::floor_y`. A game with physics sets `false`, reads `GoreFractured`, gives each of its `pieces` a rigid body, and despawns or hides the body itself. | The preset's pieces are kinematic and fall to one plane. They will fight a physics engine. |
| 9 | **Gate the bake on your own state.** The plugin adds its systems with no run condition. | The caller owns the schedule, everywhere in this crate. |
| 10 | **Heal a character you reuse with `GoreReset`**, not by despawning and dressing it again: a respawn, a pooled enemy, a restarted round. Send it with the root, and once more with each floor or wall you want wiped. See [Healing, respawning and pooling](#healing-respawning-and-pooling). | Dressing builds every canvas and image again; a reset overwrites the bytes it already has, re-arms the bone handoff and puts a slashed skin's own mesh back. |
| 11 | **Send a `GoreStep` from your footstep event**, one per foot per step, and dress the floors characters walk on at about 6 mm a texel. See [Footprints](#footprints). | The preset cannot tell when a foot lands. Without steps nobody tracks blood; on a coarse floor a print is two drops. |

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

**A shot through clothing holes it** (0.5.29). Clothing is not something a hit stops at: the skin under
a suit is wounded as before, and every cloth the shot's ray crosses on the way — the suit, a curtain in
front of the body — gets a hole where it crossed, a little smaller than the blow, ragged and ringed
grey as a bullet wipe, so the wound shows through it. A cloth behind the skin is not reached. The
hole is an alpha cut: the cloth's material turns into an `AlphaMode::Mask` at its first hole, and the
flesh material's prepass shader cuts it from the shadows and the depth prepass too. The blood that
comes through soaks into the weave along a frayed front, pinker at its edge and matte where it is
wet, and runs down the cloth only once it is saturated (`GoreDials::cloth_holds`). Before 0.5.29 the
wound under a suit was never seen and the suit took a flat, glossy patch that ran in a straight column.

**A blade cuts a slit** (0.5.31). A slash cuts the cloth over the skin it opens: a lens along the cut,
as long as the blade and `SLIT_WIDTH` (0.3) of its half-length wide at the middle, its edges wavering
and frayed, with no wipe around it. Its direction is the blade's travel read in the cloth's own UVs at
the triangle it crossed, so it lies along the cut whatever way the garment's atlas is laid out. What
shows through is the laceration as it looks on bare skin: a spindle closed at both ends, its bed the
fat layer under a film of its own blood (0.5.33, `laceration::GAPE_TAPER` and `BED_BLOOD`).

**A slash bleeds along its cut** (0.5.34). Its wound is the cut it opened: a spindle as long as the blade
and `SLASH_OPEN_M` (6 mm) wide at the middle, `π/4 · length · width` of skin, which is what it bleeds
for, sprays for and announces in `Wounded`. Its blood is a smear along the cut. The laceration opens
`SLASH_OPEN_M` wide in the world whatever the mesh's scale. Before 0.5.34 a slash was a disc its own
half-length in radius, 23 times the skin a 14 cm cut opens, and on a thigh its blood was one sheet the
leg's width from the cut to the foot.

**A wound bleeds in runs** (0.5.35). Every pulse beads at `BLEED_WELLS` (3) places where the wound's
blood would spill, each bead narrow and drawn out down the slope: along a cut's lower lip, or fanned
50° either side of downhill over a hole's rim (all round it, on a level surface). A cut's wells stay put
for the wound's life and a hole's follow its downhill, so each run is fed over the track the last pulse
left wet, which the wetmap's drip crosses without loss. Before 0.5.35 a hole pulsed one ellipse at its
centre and its blood ran as one smooth column the wound's own width. **Runs need texels:** on the
skinned mannequin's limbs at the shipped 128 a texel is 7 mm and a cut's three runs blurred into one
patch; at 256 (`Gore::with_canvas`), 3.6 mm, they come apart, at four times the bytes.

**A shot that goes through** (0.5.30; through the whole body since 0.5.62). Whether a projectile
leaves a body is the game's ballistics — calibre, energy, armour — so the game says so:
`GoreHit::impact(..).through()`. The preset walks the shot's line through the body it was sent at —
every skin under the entity the hit names, or that one mesh — and opens a wound at every way in and
every way out: a forearm held across the chest is gone into and out of, and then the chest. It counts
the shot in and out of each mesh by the winding of the triangles it crosses, so where an arm's mesh
overlaps the torso's at the shoulder the shot is inside the body all through the overlap and opens
nothing there. An exit is `EXIT_SCALE` (1.6) times the entry's radius, cut from outside along the way
the shot left, and so is a way back in after the first, the projectile being the one that came out.
Every wound bleeds and is announced as a `Wounded` of its own, in order along the shot, and each exit
throws the **forward spatter**: a cone along the shot, `FORWARD_MIST` (4) times the blood its area
alone would throw, onto whatever is behind — the chest behind the forearm, the wall behind the chest.
Every cloth the shot crosses is holed at every crossing: the shirt over the wound, the back of a
sleeve it leaves, the shirt over the chest it goes on into, the back of the shirt, a curtain behind.
Without `through()` nothing changes: the shot stops in the first part it reaches and nothing behind it
is touched.

**Walls.** `Gore::wall()` dresses an upright hard surface. The spray's droplets are cast at it as at a
sheet, but each lands as its own stain from the bloodstain model's impact law: sized by its drop's
diameter and speed (the spread factor), drawn out to `1/sin θ` by the angle it struck at, and turned so
its spines point the way it was going, through the triangle's UVs. A body part takes each droplet the
same way since 0.5.62; a sheet still takes a larger blob for each, standing for the many a game cannot
afford to cast. A wall is cast at up
to 128 droplets a wound, where every other surface takes 24; it is nearly always a few triangles. Give
a wall a canvas fine enough for the drops, `Gore::wall().with_canvas(512)` for a three-metre wall,
and mind the bill below.

**A drop lies flat** (0.5.63). The flesh material lifts a film of blood by its depth so a pool's edge
catches the light as a meniscus does (0.5.47), and it lifted a drop a few texels across the same way:
all edge and no interior, a dome, so a wall's spatter read as red beads stuck on. A drop of blood that struck a wall has spread to a tenth of a millimetre deep and centimetres
across. The shader now compares the blood four texels around a fragment with its own and draws a film
whose surround holds under a quarter of it with 0.15 of the relief, all of it from 0.6 up; a ratio, so a
wide film keeps its relief however thin. Four more taps of the wet map per wet fragment. The same spatter
on the `--through` wall, square on and at a slant, before and after:

![Blood spatter on a plaster wall seen close, square on and at a slant: at 0.5.62 every drop is a shaded dome, like a red bead stuck on; at 0.5.63 the same drops are flat stains with a crisp edge](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/small_films.png)

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
  level. A forearm held up has its blood run toward the elbow however its atlas is laid out, and
  since 0.5.40 a run follows that direction rather than the texture axis nearest it.
- **A slash is laid out on the bind pose**, which is what the laceration tears: the point, the face
  normal, the direction of travel and the length are carried back through the pose at the triangle
  that was hit. Its wound bed is skinned by the joints of the skin around it (0.5.49), so it rides
  the limb with the lips; before, it stayed in the bind pose and stood off a limb that had moved.
  Its path is walked on the skin from the hit (0.5.50), eight steps of the blade's half-length each
  way, so a cut round a limb follows the limb; laid as a straight line in the plane of the skin at
  the hit, its ends and its bed stood off a curved thigh.
- **A bruise and a burn live in UV space**, and so ride the limb with no extra work. A burn is
  drawn as Jackson's three zones as far in as its degree reached (0.5.43, `burn_zone`): a third
  degree is a dark eschar in a pale ring of stasis in a red flare out to `BURN_FLARE` times
  `GoreHit::radius_m`, its margin off round by up to `BURN_WOBBLE`. The flare is wider than the
  contact, so give the radius of what touched the skin, not of the mark you want.
  A bruise is the model's colour over an outline off round by up to `BRUISE_WOBBLE` and a density
  mottled by `BRUISE_MOTTLE` (0.5.44). Its hue is the model's and goes where the model goes: a dull
  darkening in its first hours, bilirubin's orange from about a day of model time on, which
  `GoreDials::bruise_hours_per_tick` (0.02 h) puts twenty seconds into a fight at 60 Hz. Every bruise
  on a part ages (0.5.55), up to `MAX_BRUISES` (8), the oldest retiring past that; where two overlap
  they darken the skin together, each redrawn over what the part holds without its bruises every
  model step, so neither bleaches the other.
- **The atlas density is measured from the pose**, because the scale of a rigged character lives in
  its joints, not in the mesh entity.
- **A skin with no pose yet is not cast at the bind pose.** While the joints have no transform, or
  the inverse bind poses have not loaded, a hit on it is dropped with one warning, and a part whose
  density is unstated waits to be dressed.

- **A body that breaks breaks where its skeleton is** (0.5.28). The fracture is baked from a mesh
  asset, which is the bind pose, and before 0.5.28 its pieces appeared there: a character caught
  mid-stride burst with its arms at its sides. Now each piece is carried rigidly by the bone of the
  skinned vertex nearest its centre, to where that bone is on the tick the body breaks, turned as that
  bone is turned, and the blow's place is read off the bind pose, where the fracture was cut.
  **The contract: bake the fracture in the skinned mesh's own space**, `Mat4::IDENTITY` in
  `fracture_mesh`'s parts, since a skinned mesh is drawn from its joints and its entity's transform
  plays no part. Put the `GoreBreakable` on the root or on the part; every skinned mesh under the
  root that shares the hit part's joints is searched for the nearest vertex. The proxy is the game's
  own, as ever: a cut face is a proxy cell's cross-section, so one box to a tapering limb grows a flap
  beyond it, and the mannequin's eleven boxes that hug the figure (`mannequin::breakable`) are the
  example to follow.

`examples/skinned.rs` is that, on a character made in code: eleven joints, six skinned meshes sharing
them, a walk, and a wave that holds the left forearm above the head. Every key aims at where that
part of the body is drawn that frame, and the third chest shot breaks it. `capture_skinned --break`
records it.

`capture_skinned --shirt` dresses the same body in a sleeveless shirt, a seventh skinned mesh named
`Shirt`, and the two chest shots hole it; `C` in the windowed demo puts it on or takes it off.

![The mannequin walks in a pale sleeveless shirt; two shots to the chest punch a ragged hole in it, the wound shows through, and the blood soaks out through the cotton along a frayed, pinkish front](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/skinned_shirt.gif)

`capture_skinned --through` stands a wall behind the body and sends every shot through it; `6` in
the windowed demo fires one through the chest.

![The mannequin walks in front of a plaster wall; a shot through the raised forearm and two through the chest throw a cone of fine spatter onto the wall behind, each drop its own small stain](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/skinned_through.gif)

`capture_skinned --guard` stands the body on guard, its right forearm across its chest, and sends
one shot through the forearm: it goes on into the chest, and when the arm comes down the wound it hid
is there.

![The mannequin stands with its right forearm held across its chest; one shot goes through the forearm and on into the chest, and as the arm is lowered the chest wound it hid is seen bleeding, the wall behind spattered by the chest's exit](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/skinned_guard.gif)

`capture_skinned --blows` lands a second blow on the left shin, and both bruises age.

![Close on the mannequin's left shin as it walks: one brown bruise mid-shin, then a blow just below the knee raises a paler one above it; both darken as they age, and where they overlap they are darker still](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/skinned_bruises.gif)

![The mannequin walks with its left arm raised; a heavy shot to the chest breaks it mid-stride into pieces that keep the pose, the raised arm up and the legs mid-step, before they fall and settle among its guts](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/skinned_break.gif)

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
- `CutFaceBleed`, a component on each spawned piece's cut face (0.5.37): the face as a wound — its
  area, its centre, its facing — opened on the tick the body broke. Each pulse pours a stain
  `CUT_FACE_POUR` (1.5) of the size a wound of that area pulses at onto the floor, off the face's side
  by as far as the face reaches, so a heap of pieces lies in a pool of its own blood until the
  perfusion runs out; before 0.5.37 a burst body left its floor clean. A game that spawns the pieces
  itself keeps the pours by inserting `CutFaceBleed::of(cap_mesh, &transform, tick)` on its cut face.
  The face itself wears a film of blood in `GoreFractured::cap` (0.5.38): `CUT_FACE_FILM_MM` (1 mm)
  over the muscle and a quarter of it over the fat at the rim, with its clear coat
  (`FleshParams::with_cut_film`); bare, the fat band read as cheese. A skin's craters wear the same
  film over the tissue they bare (0.5.42), none on the graze and all of it in the muscle; bare, a
  crater was a ring of yellow fat.
- `Wounds`, a component on each hurt part: every wound it is bleeding, each a UV on its wetmap and
  a point on its mesh. A hit away from the open wounds opens another, up to
  `GoreDials::open_wounds` (16), and a hit inside one reopens it. A wound leaves the list once its
  bleed has clotted.

## Blood in the air

The preset paints what blood leaves behind; what flies at the moment of the hit is particles, behind
the `vfx` feature and in a plugin of their own, because they need a GPU with compute shaders (native,
or WebGPU on the web — not WebGL2) and a headless harness must be able to leave them out:

```rust,ignore
app.add_plugins((GorePlugin, bevy_carnage::CarnageVfxPlugin));
```

That is all the wiring there is. `CarnageVfxPlugin` reads every `Wounded` the preset writes and throws
the burst its pattern class calls for, at the wound, along its normal: a gunshot a spray and a puff of
mist, an arterial wound a jet. **The mist is what a player reads as the hit** (0.5.51): the spray's
droplets leave at the 8 to 40 m/s the paper measured and are out of a body-sized view within two
frames, while the fine fraction stops within a few centimetres of the wound, as Stokes' law has a
droplet tens of micrometres across do, and hangs there. So each gunshot puffs `MIST_PER_DROPLET` (3)
soft discs per droplet of its spray, which grow to three times their size and fade over a quarter to
half a second, in the colour the film model gives blood as thin as they are.

**It follows the player's setting.** Where `GorePolicy::draws_blood` is false the same bursts are
thrown in `CarnageSettings::substitute_srgb` instead (`CarnageEffects::substitute`): the same tick,
place, direction and number, a yellow flash where the blood would have been. Before 0.5.51 the
particles read no policy, and a game that turned blood off still sprayed it.

Nothing here is read back: the particles are output, and no digest sees them. On the Linux machine
the clips are recorded on, two runs of `capture_skinned` with them on are byte-identical in all 360
frames, and two of `capture_skinned --through` differ inside the mist round each shot, by up to 8
code values; on Apple silicon `capture_carnage`'s were not ([`DEMOS.md`](DEMOS.md)).

## Floors and walls whose UVs tile

A level's floor repeats its texture: its `UV_0` runs from 0 to 20 so a 15 cm tile shows 133 times
across 20 m. That cannot hold blood, because a canvas covers `[0, 1]` once and the wetmap clamps every
UV outside it to the edge. Dress such a surface with `Gore::projected()` (0.5.57):

```rust,ignore
commands.entity(level_floor).insert(Gore::floor().projected().with_canvas(1024));
```

The preset gives the entity a copy of its mesh whose `UV_0` is the mesh seen square-on from the side
it mostly faces — from above for a floor, from the front for a wall, `+v` down a wall so blood runs
down it — scaled into `[0, 1]` without stretching, and whose `UV_1` is the tiling it had. The surface's
own textures (base colour, emissive, metallic-roughness, normal and occlusion) read `UV_1`, so its tiles
stay exactly as they were; every canvas reads the projection, and `Gore::uv_per_metre` is set from it.
The entity's `Mesh3d` is the copy from then on: a mesh other entities share is not touched, and a
collider built from the handle you spawned keeps it.

It is refused, with one warning, on a mesh that already has a `UV_1` — it may be a lightmap's, which
the copy would overwrite — and on a skin or a cloth, whose canvases follow the body's own atlas. A
surface folded over itself (a floor slab's top and bottom faces) gets one canvas for both; only the
side a camera sees shows it. **One canvas covers the whole mesh**, so its texel is the mesh's extent
over the canvas edge: a 3 m floor at 512 is 5.9 mm a texel, a 20 m one at 1024 is 2 cm — fine for
pools, too coarse for a footprint's outline; split a large floor into pieces near where blood lands.

`capture_skinned --stroll` stands on such a floor: 3 m of 30 cm ceramic tiles, its UVs running to 2.5,
dressed `projected` at 512.

## Footprints

A foot put down in wet blood picks some of it up, and a foot carrying blood leaves a print where it
next lands, fainter at every step until it has walked it off (0.5.56). The preset does not find feet
or decide when one lands: your animation already raises a footstep for its sound, so send one
`GoreStep` per foot per step from the same place:

```rust,ignore
writer.write(GoreStep::new(left_foot, sole_centre_on_the_ground, toe_direction).with_sole(Sole::BOOT));
```

`foot` is any entity, the same one for every step of one foot (its bone, or its mesh): the blood the
sole carries is kept there as a `Tread`, public, so a game can read `Tread::load` to darken the shoe or
play a wetter step. `at` is where the sole meets the ground; the floor it lands on is the nearest
straight below a point `STEP_REACH_M` (0.25 m) above it. `Sole::BOOT` is 29 by 11 cm, and
`Sole::tread` is any number fixed to one shoe: it picks which patches of the sole give up blood first,
so one shoe leaves the same gaps every step.

Each step, on that floor's wetmap:

- **The sole lifts first.** It takes `TREAD_LIFT` (30 %) of the wet blood under its outline, which
  leaves a lighter, sole-shaped dent in a pool for the pool's own spread to fill, and its load grows
  by the mean coverage it stood in, to at most `1`. **Dried blood is neither taken nor counted**: a
  sole crossing an old, dried pool comes away clean.
- **Then it prints what it carried**, and sheds `TREAD_SHED` (30 %) of it. A print is a boot's
  outline — a squared heel, a narrow waist, a forefoot widest at the ball — inked where the sole pressed
  hardest and its tread holds blood: a full load prints all of it save a few gaps at the rim, half a
  load about half of it, speckled, and the waist goes first. A fading print is paler as well as
  sparser. So prints out of a pool fade geometrically, the sixth at about a sixth of the first.
  Lifting before printing is what lets a sole walk its blood off; printed first, it would pick its
  own wet print back up.
- A print is wet blood like any other on the floor, and dries on the floor's clock.

A tier that draws no blood or no decals takes no steps. `GoreReset` of the foot, or anything above
it, wipes its `Tread`.

**A print needs a floor fine enough to hold it**: about 6 mm a texel. A print's heel, waist and
forefoot are a few centimetres each, and the shader reads the canvas through a B-spline that rounds
anything a few texels across. On the demos' 3 m slab at the shipped 256 (11.7 mm a texel), a print
read as a heel and a ball, two drops; at 512 (5.9 mm), as `capture_skinned --stroll` dresses it, the
whole outline reads, seen from above and from the walker's side, on 30 cm tiles. Raise `GoreDials::floor_canvas`, or
`Gore::with_canvas` on the floor, for the floors your characters walk on; the cost is the floor's
texels in memory, and a tick of the blood on it, the texels round each pool and print rather than
the floor between them, while any of it is wet (0.5.58).

![The mannequin walks across a tiled floor through a spill of blood and out the other side, leaving a line of bloody boot prints, left and right, each paler and more broken than the last](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/skinned_stroll.gif)

## Healing, respawning and pooling

A character that respawns, a pooled enemy coming back, a round that restarts: send
`GoreReset::new(entity)` (0.5.48) with the character's root, or any dressed surface. Every surface at
or below it goes back to how it was dressed, on the next `FixedUpdate`, before that tick's hits:

- its wetmap, flaymap and dermis canvases are cleared in place and re-uploaded clean, and a cloth's
  holes and slits are mended (`WetCanvas::clear`, `FlayCanvas::clear`, `DermisCanvas::clear`,
  `ClothHoles::clear`, each public for a game that keeps its own canvases);
- its bleeding, bruising and soaking stop (`Wounds`, `Bruising` and a cloth's pending soaks go), and
  a foot's `Tread` is wiped;
- a slashed skin draws its own mesh again, the shared handle it wore before the slash, and its wound
  bed is despawned;
- the bone handoff is armed again, so the next wound to reach the cortex sends `BoneExposed`;
- a subject broken with `spawn_fragments` off can break again.

Nothing is reallocated: the canvases and their images are the ones dressing made, so a reset costs
the bytes it overwrites. Despawning the character and spawning it again builds every canvas and image
once more. **What it leaves** is what has left the body: thrown pieces (`Flying`, `Standing`), spilled
guts (`Gut`), and the stains on a floor, a sheet or a wall — send each of those surfaces its own
`GoreReset` to wipe it, and despawn the pieces and guts. A body the preset broke into pieces has been
despawned and is gone.

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

**Where a tick goes, by system (0.5.45).** Measured on an M4 Pro by timing each of the preset's
systems in its own chain, with twelve hits a tick at one character and three quiet ticks after each,
parts of 2,208 triangles. On a quiet host before this release, a twelve-hit tick spent 1.3 ms in
`take_hits` with one character dressed and 12.9 ms with forty; `tick_canvases` was 0.36 ms and
2.7 ms of *every* tick, hit or not; the other nine systems were under 0.1 ms together. Sampled at
forty characters, about half of the busy main thread was the canvases' own passes (`WetCanvas::shade`,
`spread`, `drip`, `advance`) and about a third was drawing stains (`bloodstain::stain::rasterise` and
the `exp`, `atan2` and trigonometry it called); casting was about one sample in twenty. 0.5.45 casts
each droplet nearest box first and stops once nothing further can be nearer (239 casts a shot become
167 at forty characters), and stops `rasterise` computing what cannot change a byte. Interleaved
against 0.5.44 on the same host, under another user's load: `take_hits` at forty characters went from
25.0 and 29.7 ms to 16.1 and 15.3, at ten from 7.9 and 8.9 to 6.3 and 7.3, at one from 4.6 to 4.3,
while `tick_canvases`, which it does not touch, matched within five per cent. **What was left in a
crowd was the canvases' tick**, the rectangle each canvas's blood spans, dried stains included, for as
long as any of it is wet. At forty characters 49 canvases ticked over 186,000 texels holding blood in
48,000. Since 0.5.46 the canvases tick across Bevy's compute pool — each reads and writes only itself,
so the bytes are the same in any order — and on the same quiet M4 Pro `tick_canvases` went from 2.8 to
0.86 ms at forty characters and from 1.04 to 0.45 at ten. Where Bevy's `multi_threaded` feature is
off, as on the web, it is the loop it was, and a game that dresses only the characters near the
player pays for those.

**Since 0.5.58 a canvas ticks the blood on it, not the rectangle round it.** Each 32-texel tile keeps
the rectangle round its own blood, and the passes and their snapshot cover those grown by the two
texels a tick can move blood, where they covered the one rectangle round all of it and copied the
whole canvas twice a tick. A 1024 floor with a pool in each of two opposite corners ticked every
texel between them, 25.5 ms a tick; it ticks 0.46 now. Measured on `big` in release, interleaved
with 0.5.57: one blob on a 1024 canvas 0.34 to 0.23 ms a tick, a trail of twelve 1.59 to 0.29, and
`hit_cost_by_scene_size`, a whole `FixedUpdate`, at one character and one hit 0.82 to 0.62 ms, at ten
characters and twelve 4.6 to 4.2, at forty and twelve 15.2 to 14.6. The bytes are the same:
`the_live_tiles_change_nothing_but_the_cost` ticks one canvas this way and another over every texel and
requires them equal on every tick.

**Since 0.5.59 a flaymap redraws only where it was peeled.** The preset shades a skin's flaymap
after every hit, and every shade redrew the whole canvas, so a shotgun's twelve pellets redrew it
twelve times: timed stage by stage on `big`, 2.3 ms of a 3.5 ms `take_hits` on one character. A
shade now redraws the texels peeled since the last and the normals one texel round them, to the same
bytes. `hit_cost_by_scene_size`, interleaved with 0.5.58: twelve hits on one character 3.69 to
1.29 ms a tick, on ten 4.02 to 1.66, on forty 14.6 to 12.1, and one hit on one character 0.61 to
0.46. What is left at forty is the spray at the other characters' parts, 9.6 ms of it.

**Since 0.5.60 a rigid mesh is cast at through an index.** Every cast walked every triangle of the
mesh: twelve hits at forty characters tested 368,000 triangles a tick, and one hit on a part of
16,128 triangles walked it at least twice. A rigid surface's mesh now carries a bounding-volume
hierarchy over its triangles, built the first tick it is dressed and kept while the mesh stays as it
was, and a cast tests only the triangles whose boxes it enters. It lands where walking every
triangle lands, every field to the bit, ties included (`an_indexed_cast_lands_where_walking_every_triangle_does`),
and every frame of the skinned clip, the stroll, the break and the three tiers is byte-identical. A
skinned surface is cast at its pose, a new mesh each batch, and still walks it. Each surface's box
also carries its world-space box, which a droplet tests first without a transform. On `big`,
interleaved with 0.5.59: one hit on one character 0.50 to 0.31 ms a tick, one on a character of
16,128-triangle parts 1.53 to 0.59, twelve on it 4.65 to 3.48, twelve on forty characters 12.1 to
9.3. Building the index for a 16,128-triangle mesh took 3 ms, on the tick it was dressed. What is
left at forty is not casting — 0.12 ms of the spray — but the stains the droplets leave on the other
bodies, 6.1 ms: each is rasterised at the edge's supersampling.

**Since 0.5.61 a skinned crowd costs what a rigid one does.** Every measurement above is of rigid
parts. A game's crowd is skinned, and the same scene with every part skinned took twelve hits in
23.2 ms a tick, because every batch posed every skinned surface in the world — a blend of up to four
joint matrices per vertex — to box it for the spray, and every cast at a pose walked all its
triangles. Now a skinned surface is boxed by its joints without being posed: each joint's
bind-space box of the vertices it moves, carried by the joint's matrix, contains every vertex it
blends, padded for weights that do not add to one, with the origin for a vertex that has none
(`a_skinned_mesh_is_boxed_by_its_joints_without_being_posed`). A surface is posed only when a ray
reaches that box; the pose reuses the blend of the vertex before when its joints and weights are the
same; and the index built over its bind pose is refitted to the pose, its boxes moved and its
triangles not, so a cast at it tests only what it can reach and lands exactly where walking them all
does. On `big`, interleaved with 0.5.60: forty skinned characters 23.2 to 10.5 ms a tick, against
9.4 for forty rigid ones; one skinned character 1.94 to 1.41, its worst tick 6.5 to 2.6. Every frame
of the skinned clips and the tiers is byte-identical.

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
| cloth (wetmap, holes) | 128 | 320 KiB | 192 KiB | 704 KiB |
| floor (wetmap) | **256**, ships | 1.0 MiB | 512 KiB | **2.0 MiB** |
| wall (wetmap) | 256, ships | 1.0 MiB | 512 KiB | 2.0 MiB |
| wall (wetmap) | 512 | 4.0 MiB | 2.0 MiB | 8.0 MiB |

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

- **A piece of a skinned body is rigid.** It is carried by one bone, to where that bone is, and does
  not bend with the pose: a thigh cut across a bent knee is a straight piece standing where the thigh
  was. The break itself is exact to the bone; the shape of each piece is the bind pose's.
- **A hole or a slit does not fray or tear further.** It holds the shape it was cut with. A broken
  body's pieces are skin: the shirt goes with the body it was on.
- **A shot goes through one body.** `GoreHit::through()` goes through everything of the body it was
  sent at and stops there: it is not carried on into a second character behind, since whether it still
  could is the game's ballistics, and the hit's last `Wounded` is where it left. A part is read as
  closed — the shot is counted in and out of a mesh by the winding of the triangles it crosses — so a
  part left open where the shot leaves it opens no exit there.
- **A droplet flies straight and stops at the first thing it reaches** (0.5.32). A body part, a
  sheet, a wall or the shirt the wound is under catches it, a hole in a cloth lets it through, and a
  droplet something caught does not also land on the floor. The cast is a straight line, so a
  droplet that would arc over a low wall onto the floor behind it is caught by the wall instead, and
  only the first `SPRAY_CAST` (24) droplets of a wound are cast at all — 128 when a wall is in
  their path; the rest go to the floors as before. Until 0.5.32 every surface took every droplet
  aimed at it, so a wall behind a shirt was spattered as if the shirt were not there.
- **Wet blood needs a room to look wet.** Its look is a clear coat, and a coat shows what it
  reflects. Under a plain `AmbientLight`, a sky of one brightness from every direction, runs of blood
  on skin read as paint and a fresh pool seen low and from far off as a pale pink shape with a darker
  rim. Give the camera an `EnvironmentMapLight` — your level's, or a probe's — and drop the uniform
  ambient. The preset's clips do since 0.5.54, with a small procedural studio built and prefiltered on
  the CPU (`examples/common/studio.rs`, so it holds on WebGL2): the runs carry the reflection of a
  softbox above the camera and the pools mirror a dark wall. A pool at a light's own mirror angle
  still shows that light, as it should.
- **A skin's roughness map and a crater's slope give way.** See *What a dressed skin keeps*.
- **A sole prints blood but is not painted with it.** The blood a foot carries is a number on its
  `Tread`, not a stain on the shoe; draw that from `Tread::load` if the camera comes close to feet.
- **One gravity direction per canvas.** On a rigid mesh whose UV islands point different ways, blood
  runs correctly on the islands that agree with `gravity_uv`. A skinned part follows the pose at its
  wounds instead, and the same limit holds across the islands it has.

## Examples

```sh
cargo run -p bevy_carnage --release --example preset       # the scene above, keys to hit
cargo run -p bevy_carnage --release --example carnage_web  # the same preset as the web demo
cargo run -p bevy_carnage --release --example skinned      # the preset on an animated, skinned body
```
