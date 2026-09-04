# `bevy_carnage::fracture_modes`

Fracture modes on a convex decomposition: precompute, once, the sparse mass-orthonormal modes of a cell graph — the few ways a shape *wants* to come apart — then project an impact onto them at runtime so a body breaks across its real weaknesses instead of along whatever a Voronoi diagram happened to draw. Sellán et al.'s *Breaking Good* (2022), reduced to the one place a runtime fracture system already has a null space of rigid motions: its cells.

> **A module of `bevy_carnage` since 0.5.0.** This was a crate of its own — the family below — until 2026-09-04, when the seven leaves were folded back into the one crate a game is meant to depend on. This page is its module documentation, kept whole; the paths in it are spelled the way a consumer reaches them now.

![A necked bar breaking at its neck under a blow at either end](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/fracture_modes.gif)

## The family

This module is one of seven kernels `bevy_carnage` composes; the umbrella README lists them all with the path each is reached by. Nothing here is a separate dependency any more.

## Why this exists

Every realtime destruction system precomputes fragments and swaps them in on impact, and the paper names the fault of every one of them: Voronoi and level-set prefracture *"often miss obvious structural weaknesses"* and produce *"recognizable, unrealistic pieces"*, because they look at the shape's volume and never at where it is thin. A fracture mode is the opposite: a displacement field that pays for the *number* of faces it opens, not for how far, so its minimisers are piecewise-constant with jumps on a sparse set of faults — the thin neck, the narrow wrist, the seam. Compute a few, and any impact is a linear combination of them.

Everything here is deterministic to the bit. The eigen-initialisation is a fixed-sweep Jacobi decomposition, the solver runs a fixed number of ADMM steps, and the partition is a union-find whose unions are order-independent; a mode set is a golden and two machines bake the same one.

## The model

The paper's mode problem (their Eq. 15):

```text
argmin_{UᵀMU = I}   ½ trace(UᵀQU) + ω Σ_i E_D(U_i),      E_D(u) = Σ_faces √( ∫ ‖D(u)‖² )
```

`Q` is a strain Hessian whose only job is its null space (their §3.7: *"only its null space matters"*); `E_D` is a group-ℓ1 over fault patches, which is what makes the modes sparse. On a convex decomposition every cell already moves as one piece, so the unknowns collapse to one translation per cell, the fault patches are the shared faces, and — because `E_D` cannot tell directions apart — the modes are **scalar** functions on the cell graph with `M = diag(cell masses)` and an area-weighted graph Laplacian for `Q`.

At runtime (their §3.4) an impact is blurred one implicit step along the graph, `g = (M + τL)⁻¹ M δ_p`, projected onto the modes, and every face whose two cells' responses differ by less than `σ = 10⁻³` is **glued** back; what is left apart is the fracture. The projection is precomputed into one row per mode (their `A_i`), so an impact costs `k × cells` multiplications. Because the projection is linear, *"scaling σ and scaling the magnitude of the impact are equivalent"* — the impulse is the dial.

**One departure from the paper, stated.** The paper projects with every mode weighted equally. This crate divides each mode's contribution by its discontinuity energy — the area-weighted sum of the jumps it opens, which is the work its fault set costs — so a weak fault opens under a small blow and a strong one needs a large one. On a graph small enough that a few modes span most of it, equal weighting lets an impulse excite whichever modes are large *at the impact* and a thin neck elsewhere never opens first; with the weighting, the neck is the first face to give, which `the_neck_is_the_first_face_to_break` pins.

What this crate does not do, and the paper does not either: secondary fractures of the pieces, and directional loading (their §6). Both belong to the crate that owns the cells.


## What it exposes

- `CellGraph`, `Face` — cells with masses and centres, faces with areas; `CellGraph::bar` is the necked test shape.
- `ModeSettings` (a `Resource`), `ModeSet::bake(&graph, &settings)` → `ModeSet { modes: Vec<Mode> }` with energies ascending and the precomputed impact rows.
- `Impact { cell, magnitude }`, `ModeSet::response`, `ModeSet::partition` → `Partition { groups, broken, group_of }`. `SIGMA` is the paper's tolerance.
- `FractureModesPlugin` — adds `ModeSettings` and `FractureModeCache` (keyed by a caller `u64`). **Registers no systems**: only the crate that owns a decomposition knows when a graph is complete.

No components, no system sets.

## References

- Sellán, S., Ni, J., Stein, O., Jacobson, A. et al., *Breaking Good: Fracture Modes for Realtime Destruction*, ACM Transactions on Graphics 42(1), 2022. `doi:10.1145/3549540`
- Brandt, C. & Hildebrandt, K., *Compressed vibration modes of elastic bodies*, Computer Aided Geometric Design 52–53, 2017 — the ICCM scheme the paper adapts and this crate's ADMM stands in for.

## Examples

```sh
cargo run --example fracture_modes_bar   # terminal only — the necked bar's modes, and a blow at each end
cargo run --example fracture_modes       # a wall of cells with a weak seam, broken by clicks  (needs a GPU)
```

## Licence

MIT OR Apache-2.0, with the crate.
