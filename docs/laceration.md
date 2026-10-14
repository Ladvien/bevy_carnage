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
2. once the wound is open at all, remove any triangle whose three vertices are all inside `half_width`, any triangle the path passes through, and any triangle with a vertex inside the gape and vertices on both sides of the path. Kept, those last two would be stretched across the gap as bridges of skin, and a slash at an angle to the mesh opened as a row of pits until 0.5.3. Those two are removed only when all three vertices are within `influence`, since only those are moved onto the lip. A mesh coarser than that — a blockout's whole face — stays closed rather than losing the face, which 0.5.3 did;
3. move every vertex on the mouth sideways to the **lip**, which is the rail pushed by the falloff a vertex at the rail would get, and displace everything else within `influence` along `±(normal × segment_direction)` by `half_width · (1 - d/influence)²` — zero value *and* zero slope at the edge of the influence radius, so the surface does not crease where the displacement stops.

**The vertex buffer never changes length.** Only positions and indices are rewritten, so `ATTRIBUTE_JOINT_INDEX`, `ATTRIBUTE_JOINT_WEIGHT`, `UV_0`, normals, vertex colours and anything custom arrive untouched — a wound can be cut into a skinned character without invalidating its skinning.

**The bed of a skinned skin is skinned too** (0.5.49). The bed is a new mesh, so each of its vertices takes the joint indices and weights of the torn skin's vertex nearest it — a rail those of the lip it meets — and, when the source mesh has `SkinnedMeshBounds`, the bed gets its own. `LacerationPlugin` spawns it with the skin entity's `SkinnedMesh`, and with its `NoFrustumCulling` or `DynamicSkinnedMeshBounds`, so it bends and is culled as the limb is. Until then a skinned limb's bed was drawn in the bind pose: once the limb moved, it stood off the body as a pale fin across the cut.

**A cut is a spindle, not a slot.** A blade parts the skin widest where it went in deepest and not at all at either end, so the half-width at arc length `s` of a path `L` long is `half_width · sin(π·s/L)^GAPE_TAPER`, and the bed's floor shallows with it to the skin at both tips. Every pass above — the clip, the lip, the displacement, the walls — reads that local width, so the mouth, the trough and the floor close together. Until 0.5.33 the gape was one width from end to end with square ends, and on a limb it read as a ribbon laid on the skin rather than an opening in it.

**The gape does not close.** O'Brien, Bargteil & Hodgins (2002) put plastic — permanently retained — deformation ahead of separation in a ductile fracture model. A laceration is the ductile case, so the time curve is monotone and has no closing half:

```text
gape(t) = width_max · skin_tension · anisotropy · (1 − e^(−3t/open_ticks))
```

**The anisotropy is measured.** Ní Annaidh et al. (2012) tensile-tested excised human skin: ultimate tensile strength **21.6 ± 8.4 MPa**, failure strain **54 ± 17 %**, and an elastic modulus of **112.5 MPa parallel** to the Langer lines against **63.8 MPa perpendicular** (middle back). `anisotropy` interpolates between those two extremes on `sin²θ`, so a cut across the lines gapes at full width and one along them at `ALONG_LANGER_FACTOR = 63.8 / 112.5 = 0.567`.

**The bed is anatomy, not a dark colour.** Every bed vertex gets `UV_1` from `bevy_carnage::cross_section::uv1_at`, so the rails read depth 0 and the floor reads exactly `bed_depth_mm` in that region's strip — skin, fat and muscle at the depths they were measured at, painted by the material the atlas already baked.

**The bed is wet.** A fresh cut fills with its own blood, so every bed vertex carries `ATTRIBUTE_COLOR`, a ratio the material's colour is multiplied by: `BED_BLOOD`, a venous red, at 60 % at the lip and in full on the floor, in long patches along the cut by the same frozen noise the rail wanders by. Fat reads as fat seen through blood in places and is hidden in others; muscle darkens where the blood pools. Bevy's `StandardMaterial` multiplies by a mesh's vertex colour, so the atlas material shows it with no change; a custom bed material that ignores vertex colours shows the clean bands, as before 0.5.33.

**The bed lies in a trough, lit through its mouth** (0.5.50). The same colour is multiplied by the share of the room's light that reaches each vertex down the trough. In a section across the cut the bed is a V of half-width `w` and depth `d`; a point on a wall sees out only through the mouth, and the cosine-weighted fraction of its view through it is `(sin φ₂ − sin φ₁)/2` over the directions `φ₁..φ₂` from its normal (Hottel's crossed strings, for a strip). With `sin α = w/√(w² + d²)` that is `(1 + sin α)/2` at a lip and `sin² α` at the floor, the walls in between interpolated. It is a closed form of the geometry, not a tuning, and it leaves out the light the walls bounce to each other. Before, the walls were lit as open skin, so a narrow cut's far wall was brighter than the blood round it and the slit read as a pale ridge; a 6 mm cut 6 mm deep now shows 72 % at its lips and 20 % on its floor.

**The bed meets the skin.** Its walls rise to the same lip curve the mouth's vertices land on, sampled every 2 mm so they follow the wander. It runs past each end of the path as far as the clip took skin, and a wedge fills the outside of each bend. Until 0.5.3 the walls stopped at the rail while the lips had been pushed past it, so a strip of nothing showed down both sides of the trough. What is left is measured from above: under 5 mm² on a 22 cm slash in 5 mm cells, at the inside corner of its bend, and hairline slivers on a coarse 24-cell patch where the skin's straight edge between two lip vertices cuts inside the walls' curve.

## What the crate made up, and says so

- **`Gape::open_ticks` is this crate's own number.** No paper in the corpus gives a *rate* at which a wound opens — those are quasi-static tensile tests. The exponential shape is the response of a first-order system relaxing to a new equilibrium, and the `3` in the exponent is the choice that puts 95 % of the width at `open_ticks` (`1 − e⁻³ = 0.9502`).
- **`ALONG_LANGER_FACTOR` is a stiffness ratio used as a gape proxy.** Ní Annaidh et al. report *moduli*, not wound widths. Equating the ratio of the two moduli with the ratio of the two gapes assumes the lips are released linear springs; that step is this crate's, not the paper's.
- **`RAIL_WANDER` (15 % of `half_width`) and `WANDER_MM` (8 mm between samples)** are this crate's own: nothing in the corpus tabulates the raggedness of a wound margin. The wander is outward-only, so it can never put a vertex back inside the gap.
- **`GAPE_TAPER` (0.6) is this crate's own.** Nothing in the corpus measures a wound's width along its length. `sin(π·u)` closes both ends to a point; the power flattens the middle, so most of the cut is near its full width and it narrows over the last fifth at each end, which is how a slash photographs. At `1` the cut is a lens, pointed for half its length.
- **`BED_BLOOD` and how much of it the bed shows are this crate's own**, picked against the strip's bands so that fat still reads under the film.


## What it exposes

- `LacerationPlugin` — adds `LacerationClock` and two chained `Update` systems: the clock tick, then the retear. System set: `LacerationSystems`.
- `Laceration` — the component. Path, normal, `Gape`, `Tension`, influence, bed depth, `Region`, `opened_at`, and the **intact** `source` mesh every retear cuts from.
- `LacerationBed` — marker on the child entity carrying the wound bed's mesh, spawned by the plugin.
- `LacerationClock` — the integer tick every gape is measured against. Drive it yourself from a fixed step if the wound has to be part of a hashed simulation.
- `Tension`, `Gape`, `anisotropy`, `gape`, `ALONG_LANGER_FACTOR` — the curve, engine-free apart from `f32`.
- `tear`, `TearShape`, `Torn`, `tear_direction` — the geometry kernel, a pure function of its arguments.
- `GAPE_TAPER`, `BED_BLOOD` — the cut's profile along its length, and the colour its bed is wet with.
- `skin_patch`, `digest` — a flat patch of skin to cut, and FNV-1a over a mesh's positions, which is how the goldens are frozen. The bed's colours are not in it: they are cosmetic.
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
