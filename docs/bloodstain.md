# `bevy_carnage::bloodstain`

Blood as a **material**, not a texture: shear-thinning viscosity with a yield stress that grows into a clot, percolation spatter, stain silhouettes derived from impact conditions, the six forensic pattern classes as generators, a drying timeline, an inverse solver that reads the wound back out of the stains — and the three injury kernels around it: a bruise's haemoglobin/bilirubin chemistry, a burn's bioheat and Arrhenius damage, and blood wicking into cloth.

> **A module of `bevy_carnage` since 0.5.0.** This was a crate of its own — the family below — until 2026-09-04, when the seven leaves were folded back into the one crate a game is meant to depend on. This page is its module documentation, kept whole; the paths in it are spelled the way a consumer reaches them now.

![Sixteen films of blood, thin to thick by arterial and venous, ageing from scarlet to brown over two minutes of drying](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/spectral.gif)

## The family

This module is one of seven kernels `bevy_carnage` composes; the umbrella README lists them all with the path each is reached by. Nothing here is a separate dependency any more.

## Why this exists

Games treat blood as a decal with a lifetime. It is a shear-thinning yield-stress fluid, and two things a player can actually see follow from that and from nothing else:

- A fast rivulet is **thin and races**; a slow one **thickens and beads**.
- A flow **stops where it is** when its yield stress overtakes the stress driving it. That is what clotting is — and modelling it as a material property rather than a boolean beside one is what makes a clot happen in the right place.

The forensic literature then supplies what a stain's *shape* means, which is the difference between blood that reads as evidence and blood that reads as a particle emitter.

```rust
use bevy_carnage::bloodstain::{BloodSettings, Wound, WoundKind};
use bevy_carnage::bloodstain::stain::{Impact, stain_shape, rasterise};

let s = BloodSettings::default();

// A wound, and the blood that leaves it.
let w = Wound { at: [0.0, 1.2, 0.0], normal: [1.0, 0.0, 0.0],
                area: 0.004, severity: 1.0, kind: WoundKind::Severance };
let spray = bevy_carnage::bloodstain::droplets(&w, &s);
let landed = bevy_carnage::bloodstain::stain::stains(&w, &s, 0.0);

// What one of those stains looks like, from its own impact conditions.
let shape = stain_shape(&Impact { speed: 6.0, diameter: 0.004,
                                  angle_rad: 30.0_f32.to_radians(), roughness: 0.2,
                                  travel: [1.0, 0.0] }, &s, 7);
assert!(shape.minor < shape.major);          // minor/major = sin θ
let mut mask = vec![0u8; 64 * 64];
rasterise(&shape, 64, &mut mask);            // one byte of coverage per texel
```

## What is in it

| Module | What it answers |
|---|---|
| `rheo` | Carreau–Yasuda viscosity with Cho & Kensey's constants; a Casson yield stress that ramps into a clot; `flows()` — the one predicate that decides whether anything moves |
| `droplet` | The Comiskey percolation spatter model: many small droplets fast, few large ones slow. Frozen by a golden |
| `stain` | Placement (frozen) and **morphology**: aspect from the impact angle, spine count from Knock & Davison, satellites past Mundo's splash threshold, rasterised to a coverage mask |
| `patterns` | The six SWGSTAIN / ASB TR-033 classes as six *mechanisms*: impact, arterial arc, cast-off, expirated, drip trail, transfer — the last three spending a conserved blood budget |
| `dry` | The coagulation timeline: oxyHb → metHb → hemichrome, gloss collapsing, a rim-first drying front, a serum halo above 50 % RH, and a late craquelure |
| `pool` | Stains merging into slicks that spread. Frozen by a golden |
| `bleed` | The integer-tick pulse train and the perfusion envelope |
| `bag` | Variety selection with a guaranteed minimum gap, and no mutable cursor |
| `origin` | The inverse solver: stains in, wound out |
| `bruise` | Stam's compartment model as a radial 1-D: Darcy convection out of the pool, Fick diffusion, Michaelis–Menten conversion to bilirubin, and a colour computed from the two concentrations. Frozen by a golden |
| `burn` | Layered Pennes conduction with an Arrhenius damage integral per node: `Ω`, the necrosis depth, and a degree. Frozen by a golden |
| `wick` | Lucas–Washburn imbibition into a porous sheet at the shear-thinning viscosity of blood. Frozen by a golden |

## Determinism

- One generator (`hash_f32`), hand-rolled and **frozen by a test**. No RNG crate, because a crate that may change its stream between minor versions cannot promise reproducibility.
- Integer ticks everywhere time appears, and nothing reads a clock. The three injury kernels are the one exception, and it is a unit rather than a clock: `bruise` steps in 0.1 h, `burn` in 20 ms substeps and `wick` answers for a time in seconds — each an integer count of fixed steps or a pure function of its argument, never an accumulated float.
- Seeds are hashes of **where** something is, quantised onto a weld lattice, never of history, an entity id or an accumulator.
- `libm` unconditionally rather than `std`'s math behind a feature, because a second math path is a second set of bits.
- Seven goldens are locks rather than snapshots: `hash_f32_is_frozen`, `the_spatter_model_is_frozen`, `the_stain_placement_is_frozen`, `the_pool_model_is_frozen`, and the three added with the injury kernels — `the_bruise_model_is_frozen`, `the_burn_model_is_frozen`, `the_wick_model_is_frozen`. If one moves while the build profile is held fixed, the model moved.

## The three injury kernels

**A bruise ages by a chemistry, not by a colour ramp.** `bruise` is Stam et al.'s compartment model (`10.1007/s11517-010-0647-5`) reduced to one radial dimension over three layers — dermis top, dermis bottom, subcutis — stepped at their own 0.1 h. Blood leaks out of the subcutaneous pool by Darcy convection until the vessels close at 12 h, both chromophores diffuse, heme oxygenase-1 converts haemoglobin to bilirubin under Michaelis–Menten kinetics at 4 mol per mol, and the lymph drains the bilirubin on a 240 h constant. The colour is then *computed*: the top 400 µm of dermis as a Kubelka–Munk layer whose absorption is Bosschaart's whole-blood spectrum scaled by how much haemoglobin is actually there, plus a bilirubin band, over an authored substrate. So the yellow halo is wider than the red core because bilirubin diffuses four times faster, and the red peaks before the yellow because one is the substrate of the other — neither is authored anywhere.

**A burn is the heat that made it.** `burn` is one-dimensional layered Pennes conduction on Gowrishankar et al.'s three-layer skin (`10.1186/1475-925x-3-42`) at 100 µm nodes and a 0.02 s substep, with their Arrhenius damage integral — `A = 2.9e37 s⁻¹`, `ΔE = 2.4e5 J/mol` — accumulated per node wherever the tissue is above 42 °C. `Ω` gives the necrosis depth and a degree, so 200 ms against 200 °C and a minute against 55 °C are different injuries, and **the damage keeps accruing after contact ends** because the tissue is still hot. The two outer burn-degree thresholds are not in this crate's corpus and the module says so.

**Blood soaks into cloth as √t.** `wick` is Lucas–Washburn imbibition, with the twist Steinik et al. (`10.1103/physrevfluids.9.023305`) establish for shear-thinning fluids: the ½ exponent survives only after the effective viscosity is scaled out, so the raw front rises measurably *slower* than √t while the rescaled one is √t to within 2 %. `μ_eff` comes from this crate's own Carreau–Yasuda at the front's wall shear rate — which is why blood wicks faster than its resting viscosity predicts — and the front carries a smooth saturation profile a renderer can read.

## The literature this is a reduction of

| What | Source |
|---|---|
| Carreau–Yasuda constants for whole blood | Cho & Kensey (1991) |
| Percolation spatter, forward and back | Comiskey, Yarin & Attinger, `10.1103/PhysRevFluids.3.063901`, `10.1103/physrevfluids.2.073906` |
| `minor/major = sin θ`, drop size and impact velocity | Hulse-Smith et al., `10.1520/jfs2003224` |
| Spine count, angle-inclusive | Knock & Davison, `10.1111/j.1556-4029.2007.00505.x` |
| Spine onset and saturation, substrate roughness | Adam, `10.1016/j.forsciint.2011.12.002` |
| Splash threshold `K = We^0.5 Re^0.25` | Mundo, Sommerfeld & Tropea (1995) |
| Cast-off is tangential; pendant volume ≤ 150 µL | Williams et al., `10.1111/1556-4029.13855`; Adam, `10.1016/j.forsciint.2019.109934` |
| Expirated bubble rings: > 3 mm, ~20 % of patterns | Donaldson et al., `10.1007/s00414-010-0498-5` |
| Drying pools collapse onto one curve; rim-first front | Laan et al., `10.1016/j.forsciint.2016.08.005`; Smith, Nicloux & Brutin, `10.1038/s41598-020-65465-4` |
| Colour walk oxyHb → metHb → hemichrome | Bremmer et al., `10.1016/j.forsciint.2011.07.027` |
| Wetness, not colour, is the disgust cue | Oum, Lieberman & Aylward, `10.1080/02699931.2010.496997` |
| Whole-blood absorption and scattering, 380–780 nm | Bosschaart et al., `10.1007/s10103-013-1446-7` |
| Bruise kinetics: convection, diffusion, Michaelis–Menten, 4:1 bilirubin | Stam, van Gemert, van Leeuwen & Aalders, `10.1007/s11517-010-0647-5` |
| Layered skin bioheat and the Arrhenius damage integral | Gowrishankar, Stewart, Martin & Weaver, `10.1186/1475-925x-3-42` |
| Burn-degree `Ω` thresholds — **not in the local corpus**, cited by reference | Moritz & Henriques, Am J Pathol 26 (1947) 695–720 |
| Lucas–Washburn survives shear thinning only when rescaled | Steinik, Picchi, Lavalle & Poesio, `10.1103/physrevfluids.9.023305` |

Constants that are **tuned rather than measured** say so in their own doc comments, and `docs/citations.md` lists every citation with whether it resolved in the local corpus. A tuned constant that says so is honest; one dressed as a measurement is not.

## Examples

Both are terminal-only — no window, no GPU — so they run anywhere.

```sh
cargo run -p bevy_carnage --example stain_sweep    # ASCII stain silhouettes, 15° to 90°
cargo run -p bevy_carnage --example dry_timeline   # colour, gloss, rim, halo and cracks over the drying age
```

## Licence

MIT OR Apache-2.0, with the crate.
