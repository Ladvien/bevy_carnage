# `bevy_carnage::wetmap`

Texture-space blood accumulation for Bevy: paint a world-space hit into a mesh's own UVs, then let it run downhill, creep into its neighbours, soak into the substrate and dry in place — **on the CPU, on an integer tick, so you can hash it.**

> **A module of `bevy_carnage` since 0.5.0.** This was a crate of its own — the family below — until 2026-09-04, when the seven leaves were folded back into the one crate a game is meant to depend on. This page is its module documentation, kept whole; the paths in it are spelled the way a consumer reaches them now.

![A pale sphere shot six times; each stain runs under gravity, spreads, and dries to brown in place](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/wetmap.gif)

## The family

This module is one of seven kernels `bevy_carnage` composes; the umbrella README lists them all with the path each is reached by. Nothing here is a separate dependency any more.

## Why CPU-authoritative is the whole point

Texture-space wound and blood accumulation exists in shipped AAA — *The Last of Us Part II* keeps blood and wound render targets per character — and everywhere it exists it is a **GPU render target**. That is why nobody can hash one. A render target is written by a fragment shader and read back, if at all, through an asynchronous copy that arrives some frames later; its contents are a function of driver rounding, of raster order, and of when the readback landed. It is a picture, not a state.

This one keeps the state on the CPU in integers and treats the GPU as **output only** — the rule `bevy_carnage` states at `src/vfx.rs:6-19` and this crate inherits. Two consequences, and they are the product:

- **`digest()` is a fold over the blood, not over a screenshot.** FNV-1a over `(amount, age)` in row-major order. Two runs of the same scripted hits give the same `u64`; one hit moved by a single texel gives a different one. So a wetmap can be a golden, which as far as we can tell has not been true of one before.
- **Nothing is read back, ever.** `Assets<Image>` is written by exactly one function (`WetCanvas::flush`) and read by nobody. A future idea of the form "sample the wetmap in a shader and feed it to gameplay" has to be refused — the CPU buffer is already there and is already the authority.

There is **no custom shader and no shipped asset.** The crate owns two `Image`s per actor and hands them to a plain `StandardMaterial`. The dry surface underneath is composited *into* the canvas on the CPU, so there is nothing left to blend in WGSL.

## The model

State per texel is `amount: u8` (normalised coverage) and `age: u16` (ticks since the youngest blood there landed) — **one `Vec<(u8, u16)>` in row-major order**, which *is* the canonical order the tick pass walks. No sort is needed and none may be added.

`tick` runs four passes, in exactly this order:

1. **Drip.** A wet texel holding more than `drip_rate` sheds the **excess** one texel along `gravity_uv` and keeps the rest, so a run leaves a trail instead of translating wholesale like a sprite. The step is quantised to the dominant axis: a fractional step would need interpolation, which is a second movement model. What leaves a texel arrives in exactly one other, or is lost at the border — nothing else, because the step is a translation and at most one texel can step into another. It is **conserved to the byte wherever the destination is also wet**: a wet destination sheds down to the threshold, so its residue is at most `threshold` and the parcel is at most `255 − threshold`, which sums to exactly 255 and needs no clamp. The one lossy case is blood running onto a dry crust that is *already saturated*, and there the loss is the model rather than a rounding error — `amount` is normalised coverage, so a texel at 255 is fully covered and more blood on it is not representable because it would not be visible.
2. **Spread.** `spread_rate` diffuses into the 4-neighbourhood, written as an **antisymmetric flux** on each edge's coverage difference, `round(spread_rate/4 · (aᵢ − aⱼ))`. `f32::round` is odd, so the flux one way is exactly minus the flux the other: conserved to the byte again.
3. **Time.** `age` increments and the substrate takes its cut. Age stops at `dry_ticks`, which makes a fully dried canvas a **fixed point** — nothing moves, no byte changes, and no upload is requested.
4. **Shade.** `bloodstain::dry::appearance` writes the sRGB and roughness bytes: oxyhaemoglobin → methaemoglobin → hemichrome for colour, and `wet_roughness` → `dry_roughness` for gloss.

Both moving passes read a snapshot taken at their own top and write disjoint slots, so **traversal order cannot change the answer.** That is the property that makes the digest worth taking.

**Dry paint does not move.** Wetness gates every change to `amount` in passes 1–3, which is what makes a run stop where it stopped rather than creeping forever, and what stops a dried crust from soaking away.

**A tick costs what the blood covers, not what the canvas covers.** A canvas keeps the smallest rectangle holding every texel with blood in it. Since one tick moves blood at most two texels (drip one, then spread one more), the passes run over that rectangle grown by two, and every other texel is zero and would stay zero. A canvas whose blood has all set, or that has none, is a fixed point, so `tick` returns at once until a paint or a change of settings wakes it, and `is_still` reports it. Both are exact: the canvas's buffer, images and digest are what four passes over every texel produce, and `the_live_rectangle_changes_nothing_but_the_cost` checks that tick for tick. Measured on 40 canvases of 128² on an x86_64 Linux host: idle, 17.7 ms a tick became effectively nothing; each shot once, 18.0 ms became 0.47 ms.

`lib.rs` is `#![doc = include_str!("../README.md")]`, so this is a doctest and the compiler checks it:

```rust
use bevy::prelude::*;
use bevy_carnage::wetmap::{StainShape, WetCanvas, WetSettings};

// Once, when the actor spawns. `WetmapPlugin` gives you `WetSettings` and the upload budget.
fn wire_up(
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> (WetCanvas, Handle<StandardMaterial>) {
    let canvas = WetCanvas::new(images, 128, [0.78, 0.66, 0.60], 0.55);
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(canvas.albedo()),
        metallic_roughness_texture: Some(canvas.roughness()),
        // REQUIRED: Bevy multiplies these scalars by the texture, so the shipped values
        // would scale the map away. See `bevy_pbr-0.19.0/src/pbr_material.rs:157-163`.
        perceptual_roughness: 1.0,
        metallic: 1.0,
        ..default()
    });
    (canvas, material)
}

// Per hit, from your own gunfire code; then once per fixed tick, from your own schedule.
// This crate has no clock, so the tick number comes from you.
fn on_hit(
    canvas: &mut WetCanvas,
    mesh: &Mesh,
    actor: &GlobalTransform,
    muzzle: Vec3,
    direction: Vec3,
    shape: &StainShape,
    settings: &WetSettings,
    tick: u32,
) {
    // `paint_world` returns the UV it painted. Keep it to paint the same spot again with
    // `paint_uv_with` (a bleed, a soak) after the actor has moved and the ray no longer lands there.
    let _wound_uv: Option<Vec2> = canvas.paint_world(mesh, actor, muzzle, direction, shape, tick);
    canvas.tick(tick, Vec2::new(0.0, 1.0), settings);
}
```


## What it exposes

| Kind | Name | What it is |
|---|---|---|
| Plugin | `WetmapPlugin` | Adds `WetSettings` and the upload budget on `Update`. Registers nothing else. |
| `SystemSet` | `WetmapSystems` | The one system above, so you can order against it. |
| Component | `WetCanvas` | One actor's canvas. Holds the buffer and the two image handles. |
| Resource | `WetSettings` | Nine dials: `drip_rate`, `spread_rate`, `dry_ticks`, `absorbency`, `max_canvas_updates_per_tick`, `humidity`, `film_depth_mm`, `so2`, `edge_samples`. |
| Constant | `UV_SPAN_M` | How much world one UV unit is taken to be — see below. |
| Constant | `MAX_DETAIL`, `DETAIL_EDGE` | The finest `new_detailed` draws (4), and where it draws a stain's edge (4 % of full coverage). |
| Re-export | `StainShape`, `bevy_carnage::bloodstain` | The silhouette `paint_*` takes, and the blood model it comes from. |

`WetCanvas`: `new`, `new_detailed`, `albedo`, `roughness`, `paint_uv`, `paint_uv_with`, `paint_world`, `paint_world_with`, `tick`, `flush`, `digest`, `wetted_area`, plus the read-only `size`, `detail`, `amount_at`, `age_at`, `is_dirty`, `dirty_since`, `is_still`, `heap_bytes`, `image_bytes`.

## The metallic-roughness image carries data, not just gloss

Bevy reads exactly two channels out of a metallic-roughness texture — **green** is roughness and **blue** is metallic (`bevy_pbr-0.19.0/src/pbr_material.rs:153-154`) — so red and alpha are free, and a wetmap that left them at a constant would be uploading two wasted channels per texel. It writes the buffer's own two numbers there instead:

| channel | what | range |
|---|---|---|
| R | coverage — the film depth byte, `amount` | `0..=255` |
| G | perceptual roughness, wet blood composited over the dry surface | `0..=255` |
| B | metallic — always `0`, because blood is a dielectric | `0` |
| A | wetness, `round(255 · (1 − age / dry_ticks))`, and `0` where there is no blood | `0..=255` |

So `R · film_depth_mm / 255` is the millimetres of blood on a texel and `A` is how far it is from set: a caller's own shader can drive a specular boost, a normal-map ripple or a subsurface term off the same image the `StandardMaterial` is already sampling, without a second texture and without inferring either quantity from the colour. Nothing in the crate reads them back — they are output, like every other byte here.

## Anti-aliased stain edges: `edge_samples`

`bloodstain::stain::rasterise` answers *how covered is this texel* once per texel, at the texel's own centre, so at the shipped 128-texel canvas a stain's rim is a staircase. `WetSettings::edge_samples = n` rasterises the mask at `n` times the resolution and box-filters each `n × n` block down, so a texel the silhouette only clips receives the share of it that is actually inside.

It ships at **`1`** and that is not a fallback: the box filter over one tap is the identity — divisor one, rounding term zero — so the default writes the bytes the crate has always written and **every frozen digest is untouched by the dial existing**. `4` is what the examples use; the scratch mask costs `(major · size · n)²` bytes, so past 4 it buys nothing a player can see for four times the rasterisation. Reach it through `paint_uv_with` / `paint_world_with`, which take the settings the way `tick` does — `paint_uv` and `paint_world` are the same stamp at one sample.

## Smooth edges on a plain `StandardMaterial`: `new_detailed`

A canvas's images are its texels, and a sampler magnifies them bilinearly, so close up a stain's rim is the texels' staircase drawn soft. `WetCanvas::new_detailed(images, size, detail, base_srgb, base_roughness)` keeps the canvas — the same buffer, the same passes and the same `digest` as `new` at `size` — and draws its two images `detail` times finer, up to `MAX_DETAIL` (4). On `flush`, every 32-texel tile whose texels changed is redrawn: the coverage is read as a cubic B-spline (Sigg & Hadwiger, *GPU Gems 2*, ch. 20), the same reading the `flesh` feature's shader makes of the map, and the blood is drawn where that crosses `DETAIL_EDGE`, 4 % of full coverage, anti-aliased over one pixel. Inside the edge the colour is the film's own, weighted by how much blood each texel holds, so a pool keeps its colour up to its rim instead of fading into the surface. R and A of the metallic-roughness image are the smoothed coverage and wetness under that edge. `wetmap.gif` is drawn this way, at detail 4.

What it cannot do, and what it costs:

- **A long, shallow staircase stays a gentle wave.** A straight edge at a slant, drawn in whole texels, comes out with its corners gone and its steps softened, not straight: the spline is four texels wide and does not know the line was meant to be straight. `a_detailed_image_draws_an_edge_crisp_and_red_to_its_rim` measures a slope of 0.3.
- **The edge sits where the faintest blood is.** At 4 % of full coverage a stain is drawn out to the edge of its texels, so its size matches the canvas's; a threshold of a quarter was tried and lost most of the clip's stains, whose coverage is mostly below that. The other side of it: a run one texel wide at full coverage is drawn close to three texels wide, by the spline's arithmetic.
- **`detail²` the bytes.** Both images and their CPU copies grow by it, so at 128 a detail of 2 uploads what a 256 canvas does and 4 what a 512 does — see the budget below.
- **A redraw per flush.** Measured on an x86_64 Linux host in release, at 128 and detail 4: 1.7 ms to redraw six drying stains of 10×10 texels, and 5.9 ms when a third of the canvas changed at once; at detail 2, 0.5 ms and 1.7 ms. The plugin's budget counts canvases, not milliseconds, so a game with many detailed canvases should lower `max_canvas_updates_per_tick`.

A material that reads the canvas in its own shader, as the `flesh` feature's does, gains nothing from it and should keep `new`.

## Two things the caller owns, and one convention

**The schedule.** `tick` takes the tick *number*. Nothing here reads a clock, virtual or real, because a wetmap that read one could not be a golden. Call it from your own `FixedUpdate` before `Update`, and the plugin uploads what changed.

**Which way is down.** `gravity_uv` is in UV space. Whether `+v` runs down an actor's chest or across it is a property of the atlas, and only you know it.

**One UV unit is one metre** (`UV_SPAN_M`). A texture coordinate carries no scale, so the bridge from `StainShape::major` in metres to texels has to be stated somewhere; it is stated once, as a constant, rather than as a per-canvas dial every call site would have to agree on.

## The upload budget, and why the default size is 128

A 128×128 canvas at `Rgba8UnormSrgb` is `128 · 128 · 4 = 65 536` bytes — **64 KB per upload** — and this crate owns **two** images per actor (albedo and metallic-roughness), so one canvas is 128 KB of `Assets<Image>` writes each time it is flushed. At the shipped `max_canvas_updates_per_tick = 4` that is **512 KB per frame**. At 256 it is 2 MB/frame and at 512 it is 8 MB/frame, which is a bandwidth budget rather than a texture.

So **128 is the default and 256 is the practical ceiling.** Blood reads fine at 128 because a stain is a blob, not text.

`flush` uploads **only when dirty**, and the plugin picks the oldest-dirty canvases first by a stable key, so a canvas that has been waiting cannot be starved by one that keeps being repainted.

## One interpretation worth knowing about

**`absorbency` is a fraction of the wet *lifetime*, not a fraction per tick**, and the arithmetic is why. Read per tick, the shipped `0.15` leaves `0.85³⁰ ≈ 0.008` of the blood after half a second: every stain would vanish long before `dry_ticks = 1800` could dry it, and a wetmap would never show a dried stain at all. Read as a lifetime fraction it is exactly what its name says — a soaking substrate keeps 15 % of what lands on it, spread evenly across the drying. It is applied as an integer schedule (`floor(age · round(255 · absorbency) / dry_ticks)`, differenced), so the per-tick deltas sum to the cumulative exactly and there is no float residue to drift. Faint spatter soaks away to nothing; a pool loses 15 % and stays.

## References

- Möller & Trumbore, *Fast, minimum storage ray-triangle intersection*, J. Graphics Tools 2(1), 1997 — `doi:10.1080/10867651.1997.10487468`. The UV lookup, reimplemented here rather than borrowed from the gore crate that composes this one.
- Fowler, Noll & Vo, FNV hash (1991) — the digest.
- Oum, Lieberman & Aylward, *A feel for disgust: tactile cues to pathogen presence* — `doi:10.1080/02699931.2010.496997`. Why roughness is a first-class channel rather than a darker albedo.
- Laan et al., *Morphology of drying blood pools* — `doi:10.1016/j.forsciint.2016.08.005`, and Smith, Nicloux & Brutin — `doi:10.1038/s41598-020-65465-4`. Reached through `bloodstain::dry`, which owns the timeline.

## Examples

```sh
cargo run -p bevy_carnage --example canvas_digest   # terminal: two identical runs, two equal digests, and one that differs
cargo run -p bevy_carnage --example wetmap_paint    # click-paint a sphere and watch it run and dry. Needs a GPU.
```

`canvas_digest` is the crate's headline claim, run as a program: it paints a scripted sequence into a scratch `Assets<Image>` twice, prints both digests, then moves a single hit by **one texel** and prints the third. No window and no GPU, so it runs anywhere — including over ssh, which is how a claim about reproducibility ought to be checkable.

`wetmap_paint` is the same model with a renderer attached, and it is also **demo 8** on the project's demo site. Click to shoot the subject, `Space` for a burst, `G` to turn gravity on the texture, `D` to hide the digest. Watch blood run down the geometry, thin as the substrate takes its cut, and set in place. The digest is on screen because the interesting thing about this crate is that a picture of blood has a number under it.

That key legend is duplicated verbatim into the site page's `#notes-wetmap_paint` block, and **the page is the spec** — if the two ever disagree, the page is right and the example is wrong.

## Licence

MIT OR Apache-2.0, with the crate.
