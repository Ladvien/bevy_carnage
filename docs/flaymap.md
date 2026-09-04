# `bevy_carnage::flaymap`

Texture-space flaying for Bevy: hit the same place twice and the skin comes off, hit it a dozen times and you are through the fat, the muscle and into bone — at the depths those tissues were actually measured at, shaded from the same palette a cut face is, and **on the CPU in integers, so you can hash it.**

> **A module of `bevy_carnage` since 0.5.0.** This was a crate of its own — the family below — until 2026-09-04, when the seven leaves were folded back into the one crate a game is meant to depend on. This page is its module documentation, kept whole; the paths in it are spelled the way a consumer reaches them now.

![A torso cratered deeper with every hit until the cortex shows](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/flaymap.gif)

## The family

This module is one of seven kernels `bevy_carnage` composes; the umbrella README lists them all with the path each is reached by. Nothing here is a separate dependency any more.

## Why CPU-authoritative is the whole point

Texture-space damage masking is old, shipped technology: Frostbite 2 wrote destruction masks into textures to reveal a layered material underneath, and every engine that has done it since has done it the same way — as a **GPU render target** written by a fragment shader (Kihl, *Destruction Masking in Frostbite 2 using Volume Distance Fields*, SIGGRAPH 2010 Advances in Real-Time Rendering course). That is why nobody can hash one. A render target's contents are a function of driver rounding, of raster order, and of when an asynchronous readback landed. It is a picture, not a state.

This one keeps the state on the CPU in integers and treats the GPU as **output only** — the rule `bevy_carnage` states at `src/vfx.rs:6-19` and this crate inherits from its siblings. Three consequences, and they are the product:

- **`digest()` is a fold over the wound, not over a screenshot.** FNV-1a over one `u16` per texel, row-major. Two runs of the same scripted hits give the same `u64`; one hit moved by a single texel gives a different one. So a flaymap can be a golden.
- **Nothing is read back, ever.** `Assets<Image>` is written by exactly one function (`FlayCanvas::flush`) and read by nobody.
- **Bone exposure is gameplay, and it comes back as a value.** Every paint returns a `Handoff`, and the call that first reaches the cortex is the only one that says so. So "the skin is off, here is the bone" is a `Handoff`/`BoneExposed` a fracture system can act on, rather than a texture somebody has to look at.

There is **no custom shader and no shipped asset.** The crate owns two `Image`s per actor and hands them to a plain `StandardMaterial`. The intact surface underneath is written *into* the canvas on the CPU, so there is nothing left to blend in WGSL.

## The layer model

Per texel the state is one number: **hundredths of a millimetre of tissue removed**, as a `u16`, row-major, saturating at the whole span the region describes. Depth is **monotone** — tissue does not grow back — so every pass either adds to a texel or leaves it alone, and the layer sequence is a one-way walk rather than a state machine.

Which tissue is showing at a texel is then a lookup, and the depths are not invented here: they come from [`bevy_carnage::cross_section`](https://github.com/Ladvien/bevy_cross_section)'s `Layers`, measured with ultrasound on living adults because needle lengths and body-composition estimates depend on them.

| Region | Skin | Fat | Muscle | Cortex | Bone starts at |
|---|---|---|---|---|---|
| Limb | 1.9 mm | 7.2 mm | 18.7 mm | 5.0 mm* | 27.8 mm |
| Torso | 2.2 mm | 16.0 mm | 7.3 mm | 2.0 mm* | 25.5 mm |
| Head | 2.0 mm | 2.6 mm | 2.2 mm | 6.0 mm* | 6.8 mm |

\* Cortical bone thickness is **not corpus-sourced** — see `cross_section`'s own note — and is that crate's stated own number. `FlayCanvas::new` takes the `Layers` by value so a caller overriding it overrides it in both places at once.

The colour and roughness of a peeled texel are `bevy_carnage::cross_section::texel_at` at that depth: fat is lobules ~2 mm across in a septal net, muscle is fascicles wrapped in pale perimysium, cortex is ivory pierced by Haversian canals, marrow opens through a trabecular lattice. **The identical per-texel rule that bakes a cut face's strip**, so a flayed patch and the stump beside it are one tissue at one physical grain rather than two authored palettes that drift.

**A hit peels a crater, not a cylinder.** `paint_uv` adds its full depth at the centre and smoothstep-falls to zero at the radius. A flat disc would stack into a bore with a vertical wall and the bands would never show; with a falloff, the rim *is* the cross-section — skin at the edge, then fat, then muscle, then bone in the middle.

## The handoff contract

```rust
# use bevy_carnage::flaymap::Handoff;
# fn f(handoff: &Handoff) {
// `deepest_layer` — what THIS hit reached. Per call, not per canvas.
// `bone_reached`  — true on exactly ONE call per canvas: the first one to cross the cortex.
// `first_bone_uv` — where, in the canvas's UVs, on that one call.
// `at` / `normal` — where in MESH space, and the plane it landed on: `Some` from `paint_world`,
//                   `None` from `paint_uv`, which was handed a texture coordinate and cannot
//                   invert one (an atlas seam maps one UV to several places on a body).
# }
```

Bone is exposed **once**. A flag that stayed true would make every later shot re-announce it, and a consumer that spawns a fracture proxy or a bone-scrape sound on the announcement would spawn one per shot for the rest of the fight. `BoneExposed::from_handoff` is the whole gate, so no caller has to remember which field guards which.

## The metallic-roughness image carries the depth, not just gloss

Bevy reads exactly two channels out of a metallic-roughness texture — **green** is roughness and **blue** is metallic (`bevy_pbr-0.19.0/src/pbr_material.rs:153-154`) — so red and alpha are free, and a flaymap that left them at a constant would be uploading two wasted channels per texel. It writes the depth buffer there instead:

| channel | what | range |
|---|---|---|
| R | `round(255 · min(depth_mm / Layers::span_mm(), 1))` — how deep the wound is here | `0..=255` |
| G | perceptual roughness of the exposed tissue, from `bevy_carnage::cross_section::texel_at` | `0..=255` |
| B | metallic — always `0`, because tissue is a dielectric | `0` |
| A | `255` where anything has been removed, `0` on intact surface | `0` or `255` |

So `R · span_mm / 255` is the millimetres of tissue gone and `A` is the wound's own mask — the two things a caller's shader would otherwise have to infer from the colour, which cannot be done, because flayed muscle and a red shirt are the same red. A subsurface term, a wetness boost or a rim darkening can be driven off the image the `StandardMaterial` is already sampling, with no second texture and no second upload. Both are **output**: the depth buffer stays the authority and nothing here reads a pixel back.

## The digest

`digest()` folds the depth buffer and **nothing else** — not the pixels. `shade` is a pure function of that buffer, the layer table and the settings, so folding the pixels as well would hash the same information twice and would make a palette tweak read as a simulation divergence. The frozen golden is `the_scripted_wound_is_frozen`, and moving that number is a deliberate act.

There is no RNG in this crate and no clock. Every random-looking value comes from `bloodstain::hash_f32` keyed by integer lattice coordinates and `FlaySettings::seed`, under `texel_at`. A wound is a pure function of the hits that made it.

`lib.rs` is `#![doc = include_str!("../README.md")]`, so this is a doctest and the compiler checks it:

```rust
use bevy::prelude::*;
use bevy::ecs::message::MessageWriter;
use bevy_carnage::flaymap::{BoneExposed, FlayCanvas, FlaySettings, Layers, Region};

// Once, when the actor spawns. `FlaymapPlugin` gives you `FlaySettings`, the `BoneExposed`
// registration and the upload budget.
fn wire_up(
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> (FlayCanvas, Handle<StandardMaterial>) {
    let region = Region::Limb;
    let canvas = FlayCanvas::new(
        images,
        128,
        region,
        Layers::for_region(region),
        [0.78, 0.66, 0.60], // intact skin
        0.55,
    );
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(canvas.albedo()),
        metallic_roughness_texture: Some(canvas.roughness()),
        // REQUIRED: Bevy multiplies this scalar by the texture, so the shipped value
        // would scale the map away. See `bevy_pbr-0.19.0/src/pbr_material.rs:157-163`.
        perceptual_roughness: 1.0,
        ..default()
    });
    (canvas, material)
}

// Per hit, from your own gunfire code. This crate has no clock, so the tick number comes
// from you — and so does the decision to forward the bone handoff.
fn on_hit(
    entity: Entity,
    canvas: &mut FlayCanvas,
    mesh: &Mesh,
    actor: &GlobalTransform,
    muzzle: Vec3,
    direction: Vec3,
    settings: &FlaySettings,
    tick: u32,
    bone: &mut MessageWriter<BoneExposed>,
) {
    // 3 mm of tissue off, over a 4 cm radius of the atlas.
    if let Some(handoff) = canvas.paint_world(mesh, actor, muzzle, direction, 0.04, 3.0, tick) {
        if let Some(msg) = BoneExposed::from_handoff(entity, &handoff) {
            bone.write(msg);
        }
    }
    // After the LAST paint of the tick, and before `Update`.
    canvas.shade(settings);
}
```


## What it exposes

| Kind | Name | What it is |
|---|---|---|
| Plugin | `FlaymapPlugin` | Adds `FlaySettings`, registers `BoneExposed`, runs the upload budget on `Update`. Registers nothing else. |
| `SystemSet` | `FlaymapSystems` | The one system above, so you can order against it. |
| Component | `FlayCanvas` | One actor's canvas. Holds the depth buffer and the two image handles. |
| Resource | `FlaySettings` | Four dials: `max_canvas_updates_per_tick`, `tile_mm`, `seed`, `scale`. |
| Message | `BoneExposed` | `{ entity, uv, at, normal }` — the one-shot handoff, addressed. Written by **you**, from a `Handoff`. |
| Value | `Handoff` | `{ deepest_layer, bone_reached, first_bone_uv, at, normal }` — what a paint call found. |
| Constant | `UV_SPAN_M` | How much world one UV unit is taken to be — one metre. |
| Re-export | `Layer`, `Layers`, `Region`, `Scale` | The tissue model, the blood model under it, and the four names in this crate's signatures. |

`FlayCanvas`: `new`, `albedo`, `roughness`, `paint_uv`, `paint_world`, `shade`, `flush`, `digest`, `exposed_area`, plus the read-only `size`, `region`, `layers`, `depth_at`, `bone_texels`, `is_dirty`, `dirty_since`.

## Three things the caller owns

**The schedule.** `paint_uv` takes the tick *number* and `shade` is called by you. Nothing here reads a clock, virtual or real, because a flaymap that read one could not be a golden. Peel, then `shade` once after the last hit of the tick, and the plugin uploads what changed.

**The message.** `BoneExposed` is registered here and written nowhere in this crate: only you know whether the thing you just peeled has a skeleton something else owns.

**One UV unit is one metre** (`UV_SPAN_M`). A texture coordinate carries no scale, so the bridge from a texel index to the millimetre position the tissue noise is a function of has to be stated somewhere; it is stated once, as a constant, rather than as a per-canvas dial every call site would have to agree on.

## The upload budget

A 128×128 canvas at `Rgba8UnormSrgb` is 64 KB, and this crate owns **two** images per actor, so one canvas is 128 KB of `Assets<Image>` writes each time it is flushed. At the shipped `max_canvas_updates_per_tick = 4` that is 512 KB per frame. An actor wearing this crate *and* `bevy_wetmap` pays it twice, which is the arithmetic behind keeping canvases small: **128 is the practical default and 256 the ceiling.**

`flush` uploads **only when dirty**, and the plugin picks the oldest-dirty canvases first by a stable `(dirty_since, Entity)` key, so a canvas that has been waiting cannot be starved by one that keeps being hit.

## References

- Kihl (DICE), *Destruction Masking in Frostbite 2 using Volume Distance Fields* — SIGGRAPH 2010 *Advances in Real-Time Rendering in 3D Graphics and Games* course. The idea this crate takes off the GPU: a texture-space damage mask that reveals a layered material underneath.
- Akkus, Oguz, Uzunlulu & Kizilgul, *Evaluation of skin and subcutaneous adipose tissue thickness for optimal insulin injection* — `doi:10.4172/2155-6156.1000216`. Skin and fat depths, ultrasound, 200 adults.
- Derraik et al., *Effects of age, gender, BMI, and anatomical site on skin thickness in children and adults with diabetes* — `doi:10.1371/journal.pone.0086637`. The second skin/subcutis series the table's means come from.
- Abe, Loenneke & Thiebaud, *Morphological and functional relationships with ultrasound measured muscle thickness of the lower extremity* — `doi:10.1177/1742271X14554678`. The cadaver-validated muscle thicknesses.
- Dimitrova et al., *Facial soft tissue thicknesses in Bulgarian adults* — `doi:10.5603/fm.a2017.0114`. The head row's 6.8 mm to bone.
- Möller & Trumbore, *Fast, minimum storage ray-triangle intersection*, J. Graphics Tools 2(1), 1997 — `doi:10.1080/10867651.1997.10487468`. The UV lookup, reimplemented here rather than borrowed from a sibling.
- Oum, Lieberman & Aylward, *A feel for disgust: tactile cues to pathogen presence* — `doi:10.1080/02699931.2010.496997`. Why roughness is a first-class channel and not a darker albedo.
- Fowler, Noll & Vo, FNV hash (1991) — the digest.

The four anatomical papers are reached through `bevy_carnage::cross_section`, which owns the table; they are cited here because this crate's depths, its bone threshold and its frozen digest are all functions of them.

## Examples

```sh
cargo run -p bevy_carnage --example flay_digest    # terminal: 30 hits, the layers coming off, and one number
cargo run -p bevy_carnage --example flaymap_peel   # a lit slab peeling itself to bone. Needs a GPU.
```

`flay_digest` is the crate's headline claim run as a program: a 64² canvas takes thirty hits at one spot with growing depth, and it prints the exposed area of each tissue after every one, the hit on which bone came through, and the digest. No window and no GPU, so it runs anywhere — including over ssh, which is how a claim about reproducibility ought to be checkable. Run it twice; the digest is the same.

`flaymap_peel` is the same model with a renderer attached: a slab is hit every 20 frames at a fixed spot while the camera orbits, so the crater opens through skin, fat and muscle and stops at bone while you watch the rim.

## Licence

MIT OR Apache-2.0, with the crate.
