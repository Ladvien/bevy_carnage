# `bevy_carnage::laceration`

Progressive lacerations: a cut along a mesh surface whose gape widens on a time curve scaled by an authored skin tension, opening onto a wound bed banded by anatomical depth. Cut across the Langer lines and it yawns; cut along them and it barely parts, in the measured ratio. CPU-side, deterministic, hashable.

> **A module of `bevy_carnage` since 0.5.0.** This was a crate of its own — the family below — until 2026-09-04, when the seven leaves were folded back into the one crate a game is meant to depend on. This page is its module documentation, kept whole; the paths in it are spelled the way a consumer reaches them now.

![A slash across a thigh opening onto a banded wound bed](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/laceration.gif)

## The family

This module is one of seven kernels `bevy_carnage` composes; the umbrella README lists them all with the path each is reached by. Nothing here is a separate dependency any more.

## Why this exists

A knife wound in a game is a decal. A decal cannot gape, so it reads as paint on a surface, and the moment the camera gets close the illusion is that the character has been drawn on rather than opened. What is actually missing is not resolution — it is that a real laceration is a *hole with sides*, it takes about a second to reach its width, and the width depends on which way the blade ran relative to the collagen underneath.

All three of those are cheap. This crate does them on the CPU, from an intact source mesh, as a pure function of an integer tick — so the wound is the same on every machine and a test can freeze its digest.

## The model

**A tear is authored, then opened.** Kamarianakis, Protopsaltis et al. (2022) describe progressive tearing as a user-defined width along a sampled path, faces in the gap clipped, and auxiliary particles displaced *normal to and away from* the tear segments so the wound opens over time. That is exactly the three passes of `tear`:

1. measure every vertex's in-surface distance to the polyline;
2. remove any triangle whose three vertices are all inside `half_width`;
3. snap the surviving inside vertices out to the rail, and displace everything within `influence` along `±(normal × segment_direction)` by `half_width · (1 - d/influence)²` — zero value *and* zero slope at the edge of the influence radius, so the surface does not crease where the displacement stops.

**The vertex buffer never changes length.** Only positions and indices are rewritten, so `ATTRIBUTE_JOINT_INDEX`, `ATTRIBUTE_JOINT_WEIGHT`, `UV_0`, normals, vertex colours and anything custom arrive untouched — a wound can be cut into a skinned character without invalidating its skinning.

**The gape does not close.** O'Brien, Bargteil & Hodgins (2002) put plastic — permanently retained — deformation ahead of separation in a ductile fracture model. A laceration is the ductile case, so the time curve is monotone and has no closing half:

```text
gape(t) = width_max · skin_tension · anisotropy · (1 − e^(−3t/open_ticks))
```

**The anisotropy is measured.** Ní Annaidh et al. (2012) tensile-tested excised human skin: ultimate tensile strength **21.6 ± 8.4 MPa**, failure strain **54 ± 17 %**, and an elastic modulus of **112.5 MPa parallel** to the Langer lines against **63.8 MPa perpendicular** (middle back). `anisotropy` interpolates between those two extremes on `sin²θ`, so a cut across the lines gapes at full width and one along them at `ALONG_LANGER_FACTOR = 63.8 / 112.5 = 0.567`.

**The bed is anatomy, not a dark colour.** Every bed vertex gets `UV_1` from `bevy_carnage::cross_section::uv1_at`, so the rails read depth 0 and the floor reads exactly `bed_depth_mm` in that region's strip — skin, fat and muscle at the depths they were measured at, painted by the material the atlas already baked.

## What the crate made up, and says so

- **`Gape::open_ticks` is this crate's own number.** No paper in the corpus gives a *rate* at which a wound opens — those are quasi-static tensile tests. The exponential shape is the response of a first-order system relaxing to a new equilibrium, and the `3` in the exponent is the choice that puts 95 % of the width at `open_ticks` (`1 − e⁻³ = 0.9502`).
- **`ALONG_LANGER_FACTOR` is a stiffness ratio used as a gape proxy.** Ní Annaidh et al. report *moduli*, not wound widths. Equating the ratio of the two moduli with the ratio of the two gapes assumes the lips are released linear springs; that step is this crate's, not the paper's.
- **`RAIL_WANDER` (15 % of `half_width`) and `WANDER_MM` (8 mm between samples)** are this crate's own: nothing in the corpus tabulates the raggedness of a wound margin. The wander is outward-only, so it can never put a vertex back inside the gap.


## What it exposes

- `LacerationPlugin` — adds `LacerationClock` and two chained `Update` systems: the clock tick, then the retear. System set: `LacerationSystems`.
- `Laceration` — the component. Path, normal, `Gape`, `Tension`, influence, bed depth, `Region`, `opened_at`, and the **intact** `source` mesh every retear cuts from.
- `LacerationBed` — marker on the child entity carrying the wound bed's mesh, spawned by the plugin.
- `LacerationClock` — the integer tick every gape is measured against. Drive it yourself from a fixed step if the wound has to be part of a hashed simulation.
- `Tension`, `Gape`, `anisotropy`, `gape`, `ALONG_LANGER_FACTOR` — the curve, engine-free apart from `f32`.
- `tear`, `TearShape`, `Torn`, `tear_direction` — the geometry kernel, a pure function of its arguments.
- `skin_patch`, `digest` — a flat patch of skin to cut, and FNV-1a over a mesh's positions, which is how the goldens are frozen.
- `Layers`, `Region` and `Scale` re-exported from `cross_section`, which appear in this crate's own signatures.

The entity's `Mesh3d` handle and `Laceration::source` **must be different handles** — the plugin writes the first and reads the second, and refuses (once, loudly) rather than destroying the intact copy.

## References

- Kamarianakis, Protopsaltis, Angelis, Tamiolakis & Papagiannakis, *Progressive tearing and cutting of soft-bodies in high-performance virtual reality*, ICAT-EGVE 2022. `doi:10.48550/arXiv.2209.08531`
- O'Brien, Bargteil & Hodgins, *Graphical modeling and animation of ductile fracture*, SIGGRAPH 2002. `doi:10.1145/566570.566579`
- Ní Annaidh, Bruyère, Destrade, Gilchrist & Otténio, *Characterization of the anisotropic mechanical properties of excised human skin*, J. Mech. Behav. Biomed. Mater. 5 (2012). `doi:10.1016/j.jmbbm.2011.08.016`
- The layer thicknesses the bed's bands come from are `cross_section`'s, with their own sources.

## Examples

```sh
cargo run --example laceration_curve   # terminal only — gape vs ticks, both Langer directions, and the frozen digest
cargo run --example laceration_open    # a slash across a patch of skin, opening over ~3 s  (needs a GPU)
```

`laceration_curve` prints a digest; run it twice and the two lines match, which is the property the whole crate rests on.

## Licence

MIT OR Apache-2.0, with the crate.
