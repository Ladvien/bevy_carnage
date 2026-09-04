# bevy_carnage — notes for agents

Deterministic runtime gore: take whatever meshes an entity actually loaded, plane-cut them into watertight-capped chunks, bore bullet channels through them, and drive blood, spatter and impact feel off the wounds that result.

## Source of truth

**The monorepo is the source of truth.** The crate lives at `crates/bevy_carnage/` in [`Ladvien/foundation_vs_slop`](https://github.com/Ladvien/foundation_vs_slop) as a workspace member; work is done there, verified there against that lockfile, and `scripts/mirror_crates.sh` re-derives [`Ladvien/bevy_carnage`](https://github.com/Ladvien/bevy_carnage) from it by `git subtree split`. Nothing is ever edited on the mirror and nothing is pulled back. The game consumes the crate **by path**, deliberately — a rev pin is what made this crate unfixable during sustained work. The registry release is the third stop and is cut from the monorepo too: `cargo publish -p bevy_carnage`. So the direction is **monorepo → mirror → crates.io**, and it never runs backwards.

**Three earlier arrangements are recorded here because each cost a session, and none is live.** The crate first lived only in its own repository while the monorepo consumed it by rev; then it was vendored into the monorepo under this crate's former name as a workspace member, mirrored out and never pulled back; then the vendored copy was deleted and the rev pin restored, which made every fix a sibling clone plus a push plus a bump; and on 2026-09-01 it was vendored again with `git subtree add`, history intact, which is where it stands. A fourth claim was recorded and was simply **wrong**: that the two histories were unrelated, so a split push could never fast-forward. `eacb160` carries `git-subtree-split: 1434a39` *and* has that commit as its second parent, so the mirror's `main` is an ancestor of the monorepo's history and the unforced push fast-forwards — measured, then done. (The former name is deliberately not spelled anywhere outside `BACKLOG_ARCHIVE.md`, so that a grep for it stays a completeness check on the AG-025 rename rather than a list of prose exceptions.) **The hazard the mirror era was reaching for is still real, and it is about reading:** a `subtree split` carries only *commits*, so anything living uncommitted in a working tree cannot arrive on the far side at all — a research agent once read the far side, found no audit harness and no `isomesh`, and reported both as missing when both existed. Read the tree you are about to change, not a copy of it.

## Build and test

A leaf — `bevy` with defaults off, optional `serde`, `isomesh` for validation, and optional `bevy_hanabi` behind the `vfx` feature — so it builds and tests on its own. In the monorepo it is a workspace member, so reach it with `-p bevy_carnage`; the mirrored copy has no workspace above it and takes no flag.

```
cargo test              # 16 unit + leaf.rs + doctests
cargo build --release   # NOT redundant: see below
cargo build --examples
```

**`cargo build --release` is the load-bearing one.** `cargo test` compiles dev-dependencies, and the dev-dependency here is the *full* `bevy` umbrella (the windowed example needs winit and the render pipeline). Cargo unifies features, so under `cargo test` this crate silently gets every `bevy` feature there is and a missing entry in its own `[dependencies]` list cannot fail. That is not hypothetical — `WorldAsset` was reached for while only `bevy_scene` was declared, not `bevy_world_serialization`, and every test passed on a crate that did not build.

## The non-negotiable: the bake is reproducible, or it does not happen

Two runs of the same build on the same asset must produce bit-identical fragments. Everything below follows from that, and every line of it was paid for by a bug that took two sessions to find.

**Never seed from an `AssetId`.** An `AssetId` is a slot index in the asset arena, assigned by async load order and subject to slot recycling. The same file gets a different id run to run. When `seed_from_path` hashed the id instead of the path, two same-seed builds produced **23 of 23 fragments differing** — in `half_extents` as well as `center_local`, meaning the mesh was being *partitioned* differently, not merely rounded differently. The asset path is authored rather than allocated; that is the whole reason it is the key.

**Never append to the vertex soup during the `Children` walk.** glTF scene-child order is the order async instantiation happened to add nodes, which is wall-clock dependent. Fragment centroids are float sums over the merged soup, float addition is not associative, and so an unsorted soup moves every centroid by a few ULPs — identical fragment count, positions off in the last bits. Collect first, sort by `(mesh asset path, world-matrix bits)`, then append. The mesh's own `AssetId` was tried as that key and was the *same bug over again*, on a subset of the soup.

**`sort_total_by_key_at` earns its runtime check — do not downgrade it to a comment.** Its one call site is the one place in this crate whose input is an ECS query, and query order is not stable across `App` instances. A comment asserting the key is total cannot fail; the check can, and it is what caught the soup-order bug above. It compiles out in release and is turned back on by the `strict-order` feature, for a harness that builds in release and still wants it.

**`hash_f32` is frozen by a test.** It seeds every cut plane's direction, so a one-constant edit re-partitions every mesh this crate has ever fractured. Treat that test as a lock, not a snapshot to re-bless. There is deliberately no RNG dependency — a stream that may change between minor versions cannot underwrite any of this.

**The bake gate treats an empty detached part as "still streaming", never as "absent".** When the held item is a separate scene from the body it streams on its own schedule: the `DetachedPart` entity exists immediately but has no `Mesh3d` descendants yet, so `all_loaded` stays true and the bake would cache a source with an empty chunk and mark it baked **permanently**. Measured: 11 of 12 runs produced the chunk and 1 did not. If anything downstream is a fixed-size pool or a numbered sequence, losing one chunk shifts every later one, so the race does not stay cosmetic. If a subject with genuinely no detached part is ever supported, this gate must learn to tell "absent" from "not yet" — do not relax it back.

## Rules

- **Bevy 0.19 is pinned.** Read the vendored source (`~/.cargo/registry/src/index.crates.io-*/bevy-0.19.0/`, and its `examples/`), not bevy.org — that documents `main` and has been wrong for this pin more than once.
- Consult Bevy documentation often. It can be found at codex_fs/offline_reference_docs/bevy-0.19-book/
- **A missing `Res<T>` panics its system in 0.19**; it does not skip. Both resources this reads are `init_resource`d by the plugin, which is what keeps that true — a caller supplying `FractureSettings` inserts it *before* adding the plugin, and `init_resource` then no-ops.
- **All run conditions are evaluated — there is no short-circuit.** A bare `Res<T>` in a `.run_if(..)` closure panics whenever that resource is absent, even behind an earlier condition that returned false.
- **The caller owns the schedule.** The plugin adds one system to `Update` in `CarnageSystems` and configures no run condition. Anything inserting `DetachedPart` on a streamed-in subtree must run `.before(CarnageSystems)`.
- **No `unwrap()`**, no `expect` on caller data, no panicking index. Malformed input is `warn!`-skipped: a mesh with no `Float32x3` positions, a non-`TriangleList` topology, an unclosed cut boundary, an out-of-range index. A handle with no asset path is `error!`-refused rather than baked unreproducibly.
- **One path per feature.** No fallbacks, no legacy shims, no stub placeholders. `seed_from_path` refusing to bake an unpathed handle *is* the rule: falling back to the `AssetId` for one sub-mesh would reintroduce the instability intermittently, which is worse than not baking.
- **This crate never learns what died.** No health, no factions, no damage, no physics. `tests/leaf.rs` enforces the dependency half of that; the naming half is on you — nothing here should say "gun", "figurine", or "unit".

## The pixel half and the preset (0.4.0)

- **`flesh` is cosmetic and must stay that way.** `FleshMaterial` reads canvases the CPU owns and never writes one; its two tables are baked with `libm` and frozen by `the_flesh_tables_are_frozen`. A change to a profile or the integrator moves that golden and is a deliberate, documented re-bless — never a "fix". Nothing in the shader may be needed by a hash.
- **WebGL2 is a requirement, not a hope.** Sixteen-byte-aligned uniforms, no storage buffers, one sampler for the extension's five textures, forward path only. It was verified in headless Chromium over SwiftShader with `bevy`'s `webgl2` feature swapped in for the site's `webgpu`; do that again after touching `flesh.wgsl`.
- **`preset` is the one place in the family that owns a schedule.** `GoreClock` is one `u32` per `FixedUpdate` and nothing in the preset reads `Time` for anything a canvas or a kernel sees. A headless clip drives it with `TimeUpdateStrategy::ManualDuration` so ticks equal frames; `capture_preset` prints a digest two runs must reproduce.
- **A hit that removes nothing peels nothing.** A blow bruises the dermis map, a burn chars it (an eschar stays on the surface), a slash opens geometry; only an impact writes the flaymap. The dermis map is a *ratio* under the skin the caller authored, so a skin tone stays the caller's.
- **`Gore::uv_per_metre` is not optional in practice.** The canvases take radii in UV space at one UV per metre; a body atlas is nothing like that, and a wound the preset cannot size is a dot. State it from the asset.

## Where the boundary falls

Four things belong to the caller, not here, and each has bitten someone who assumed otherwise:

- **The convex decomposition.** This crate cuts a proxy — `ProxyCell` per connected shell — and carries the render triangles along as a payload. Computing that decomposition is not its job: a consumer already running V-HACD or CoACD for colliders has one, and forcing a second, different decomposition would be the fracture disagreeing with the physics about what the object is. A subject with no `FractureProxy` is `error!`-refused rather than given a synthesised bounding box. `Bore` is not an exception: it subtracts a prism *you* described from a cell *you* supplied, by plane splits with a closed-form decomposition, so the union of what comes back is still the shape you handed in — and a concave cell is still refused at the door.
- **Naming the part that detaches.** Finding a weapon node by name is content, not fracture. Tag it `DetachedPart` from your own system, `.before(CarnageSystems)`.
- **Deciding when the bake may run.** The plugin sets no run condition. Gate `CarnageSystems` on your own state.
- **Everything after the fragment exists.** Rigid bodies, colliders, launch impulses, pooling, despawn. This crate hands out a mesh, a local centre and a half-extent, and stops.

If a change here would need to know what died, or which solver you use, it belongs on your side of that line.

## The modules' own non-negotiables

Each of the seven kernels was a crate with a `CLAUDE.md` of its own until 0.5.0 (2026-09-04). Those rules did not stop applying when the crate boundary became a module boundary; they are carried here, with the mirror preamble, the mirror-furniture sections and the per-crate dependency ratchets dropped (there is one ratchet now, `tests/leaf.rs`, and `bloodstain_is_still_engine_free` mechanises the first rule below). Headings are demoted two levels.

### `bloodstain`

#### No math library. Ever.

The public API is `[f32; 3]`. The workspace this crate lives in resolves **two** `glam` versions at
once, so a leaf naming either could collide with a consumer naming the other — naming neither is the
only choice that cannot. `tests/leaf.rs` forbids `glam`, `nalgebra` and every `bevy*` crate by name,
and `crate::vec` mirrors `glam::Vec3` **operation for operation** because its output feeds a frozen
golden that was blessed against glam. If you add a vector helper, name the glam function it mirrors.

Conversion is the consumer's job and it has exactly one home: `bevy_carnage/src/v3.rs`. Per wound,
not per vertex.

#### One math path: `libm`, unconditionally

Not `std`'s math behind a feature. A second math path is a second set of bits, and this crate's
product is a frozen model. This was **measured before it was adopted**: at the spatter golden's own
inputs, `libm::{sinf, cosf, sqrtf}` are bit-identical to the platform libm the model was blessed
against. `powf`/`expf` differ by one ULP at some inputs and are read only by code written here, never
by a moved golden. Every transcendental goes through `crate::m`.

#### Seven goldens are locks, not snapshots

`hash_f32_is_frozen`, `the_spatter_model_is_frozen`, `the_stain_placement_is_frozen`,
`the_pool_model_is_frozen`, and the three the injury kernels brought with them —
`the_bruise_model_is_frozen`, `the_burn_model_is_frozen`, `the_wick_model_is_frozen`. If one goes red
while the build profile is held fixed, **the model moved** — that is the finding, not a table to
update. Re-bless only for a profile change, and say which profile in the doc comment (both spatter
tables record the one re-blessing they have had).

`the_spatter_model_is_frozen` and `the_stain_placement_is_frozen` came out of `bevy_carnage::spatter`
**with their bits unchanged**. That is the evidence the extraction was a move rather than a rewrite,
and it is why a future refactor must not touch them.

#### Ticks, not seconds. No clocks. And the one exception, which is a unit rather than a clock.

Every function that involves *game* time takes `tick: u32` and `hz: u32`. Nothing reads a clock,
virtual or real. A float accumulator large enough stops advancing at all, which is a recorded failure
in this family of crates. Tick counts are quoted for 60 Hz; a caller on another rate re-derives them
in data.

**`bruise`, `burn` and `wick` are in physical time, and that is not a loosening of the rule.** Each
is a reduction of a published numerical model whose step size *is* one of its constants — Stam's
0.1 h, the CFL-bounded 0.02 s of an explicit heat solver — so a tick count would be a second,
hidden, rate-dependent discretisation on top of the paper's own. The rule the exception keeps is the
one that mattered: **the state is advanced by an integer count of fixed steps and never by an
accumulated float.** `Bruise::steps` and `Burn::steps` are `u32`, the elapsed time is *derived* by
multiplying, and a `seconds` argument is rounded to a step count once at the door. `wick` holds no
state at all — its front is a closed form of `t`.

#### Seeds are places, never histories

`wound_seed` and `patterns::site_seed` hash a position **quantised onto `WELD`**, mixed with an enum
discriminant. Never an accumulator, an entity id, an asset id or a clock: an arena slot is assigned by
load order, and a drain counter desynchronises permanently after any single difference.

**`WoundKind` and `PatternClass` are append-only.** Their discriminants are mixed into seeds and
travel in saved data. A new variant goes on the end with the next number; nothing is ever renumbered.

#### No fallbacks, and no fabricated answers

A function that cannot answer returns `None`, `0`, or `false` — never a plausible-looking guess.
`landing` refuses a droplet that never crosses the plane; `area_of_origin` refuses an
underdetermined scene rather than averaging it into a point; `rasterise` refuses a wrong-sized buffer
rather than half-filling it; a zero normal gets a zero basis rather than "straight up".

No `unwrap`, no `expect`, no panicking index in library code. `#![forbid(unsafe_code)]`.

#### Cessation has one predicate

`rheo::flows(driving, yield)`. A clot, a rivulet arresting on a wall, and a pool that stopped creeping
are **one mechanism at three ages**. There is deliberately no `clotted` boolean beside it and no
second `f <= 0.0` guard in `pulse_wound` — a second guard would be a second answer to the same
question.

#### Tuned constants say so

`SPINE_WE_MIN`, `PERFUSION_STRESS_PA`, `hct_exponent`, `DRY_REF_TICKS` are tuned or compressed rather
than measured, and each says so in its own doc comment **and** in `docs/citations.md`. A tuned
constant that admits it is honest; one dressed as a measurement is not. When adding a constant: name
the paper, or name the fact that there isn't one.

### `wetmap` (was `bevy_wetmap`)

#### The CPU is the authority. The GPU is write-only.

This is the crate's entire reason to exist. Texture-space blood accumulation elsewhere is a GPU render
target, which is why nobody can hash one; this one keeps `(amount, age)` on the CPU in integers and
treats the two `Image`s as output. `crates/bevy_carnage/src/vfx.rs:6-19` states the rule and the
reason.

So: **nothing reads `Assets<Image>`.** A future idea of the form "sample the canvas in a shader and
feed the result to gameplay", or "read the uploaded pixels back to place decals", must be refused —
the CPU buffer is already there and is already the authority. `digest()` folds the buffer, never the
images, and that is not an implementation detail: it is the claim.

#### Row-major is the canonical order

One `Vec<(u8, u16)>`, row-major. That order **is** the order the tick pass walks, so no sort is needed
and none may be added. A sort here would be a second answer to a question the layout already answers,
and the digest would then depend on which answer ran.

#### The moving passes read a snapshot and write disjoint slots

Both the drip and the spread pass copy `wet` into `prev`, then compute each texel's new value from
`prev` alone. That is what makes the result independent of traversal order, and it is what makes the
digest mean anything. Two unit tests defend it directly —
`the_drip_pass_moves_a_parcel_exactly_one_texel` and
`the_spread_pass_reaches_only_the_four_neighbourhood_in_one_call` — because a pass that read its own
writes would cascade in one call and both would go red.

**Both passes conserve mass to the byte, and each has a separate argument for it.** The drip leaves the
threshold behind, so a wet destination's residue is at most `threshold` while a parcel is at most
`255 − threshold`: they sum to exactly 255, so an arrival between two wet texels cannot overflow and
there is no clamp on that path. The spread is an antisymmetric flux on the coverage *difference*, and
`f32::round` is odd, so `flux(i→j) == −flux(j→i)` exactly. If you rewrite either as a "give a fraction
away" loop, both arguments die and the conservation tests go red — that is the tests doing their job,
not a table to update.

The single lossy path is blood running onto a **saturated dry crust**, which does not shed and so has
no room. That loss is the model: `amount` is normalised coverage, a texel at 255 is fully covered, and
more blood on it would not be visible. Do not "fix" it by letting the crust move.

#### Dry paint does not move

Wetness gates every change to `amount`. A texel past `dry_ticks` neither drips, nor spreads, nor soaks,
and its age has stopped — so a dried canvas is a **fixed point** of `tick`, and asks for no upload.
That is what makes a run stop where it stopped. Do not add a "dry blood slowly fades" path; a second
rule for the same quantity is how a run would start creeping again three months later.

#### `flush` is the only place `Assets<Image>` is touched

One function, and it uploads only when dirty. The per-frame budget across canvases is the plugin's, and
it is ordered `(dirty_since, Entity)` — an integer tick first, so it is reproducible, with `Entity`
breaking a tie only between canvases that went dirty on the same tick, where the only thing at stake is
which write-only image gets its bytes first.

#### Ticks, not seconds. No clocks.

`tick` takes the tick *number*. Nothing here reads a clock, virtual or real, and nothing may. A float
accumulator large enough stops advancing at all, which is a recorded failure in this family of crates.
`dry_ticks` is quoted at 60 Hz; a caller on another rate re-derives it in data.

`dry_ticks` is also the **single authority** for the drying timeline: a texel's age is rescaled onto
`bloodstain::dry::DRY_REF_TICKS` before `appearance` is asked anything, so moving the dial moves the
whole curve rather than only the wet/dry gate.

#### The caller owns the schedule

`WetmapPlugin` registers exactly one system, the upload budget, because uploading is the only part with
no gameplay opinion in it. It does **not** tick canvases: a tick number is gameplay state, and
inventing one would mean reading a clock.

Both of that system's resources are `Option`, deliberately. Bevy 0.19 *panics* a system with a missing
`Res<T>` rather than skipping it, and `Assets<Image>` belongs to a plugin this crate does not add.

#### No shader, no asset, no material

The crate hands back two `Handle<Image>` and never names a material — which is why it does not depend
on `bevy_pbr`. The dry surface is composited *into* the canvas on the CPU, so there is nothing left to
blend in WGSL. If a caller's blood looks flat, the cause is almost always that their
`StandardMaterial` left `perceptual_roughness` and `metallic` at the shipped scalars, which Bevy
multiplies by the texture (`bevy_pbr-0.19.0/src/pbr_material.rs:157-163`); both must be `1.0`.

Roughness is the **green** channel and metallic the **blue** one, stated at
`bevy_pbr-0.19.0/src/pbr_material.rs:153-154`. The metallic-roughness image is `Rgba8Unorm`, **not**
sRGB: it carries material data, not colour.

#### Rules

- **Bevy 0.19 is pinned.** Read the vendored source, not bevy.org — that documents `main` and has been
  wrong for this pin more than once. `bevy_render` is in the feature list for exactly three type names
  (`Extent3d`, `TextureDimension`, `TextureFormat`); see the comment in `Cargo.toml`.
- **No `unwrap()`, no `expect`, no `panic!`, no panicking index in library code.** The hot loops use
  `get`/`get_mut` with a `continue` guard even where the index is in bounds by construction, so the
  crate contains none at all. A mesh that cannot carry a wetmap gets `false` and one `warn!`.
- **One path per feature.** No fallbacks, no legacy shims, no stub placeholders, no setting that picks
  between two implementations of one thing.

### `viscera` (was `bevy_viscera`)

XPBD Cosserat-style strands with a tearing mesenteric membrane: guts that spill, fall with weight, coil on the floor, stay tethered, and tear loose.

#### The non-negotiable: the answer must not depend on how hard the machine was working

This crate's product is not viscera. It is *reproducible* viscera, and every rule below exists to keep one number — `Strand::digest()` — a function of the inputs and nothing else.

- **The substep count, the iteration count and the constraint sequence are fixed.** Four substeps, eight passes, stretch → bend → mesentery → floor, ascending index order throughout. They are named constants (`DEFAULT_SUBSTEPS`, `DEFAULT_ITERATIONS`) and they are not tuned per call: a solver that spent more iterations on a nearby strand than on a distant one would give a different answer for the same input.
- **There is no early-out.** Every iteration walks the whole sequence whether or not the residual is already zero. Constraints are *skipped* only by data — torn, degenerate, or slack — never by convergence, so the pass count is identical on every run. If you find yourself adding `if residual < eps { break }`, you are deleting the crate's reason to exist.
- **Nothing reads a clock.** `FIXED_DT` is a constant. Bevy's `Time<Fixed>` defaults to 64 Hz, which means the shipped plugin runs 60 Hz physics on a 64 Hz tick unless the app says `Time::<Fixed>::from_hz(60.0)`; that is a documented caveat, not a bug to fix by reading `Time`. Reading it would make the digest a function of a runtime setting.
- **Nothing draws from an RNG.** `spill` derives every per-strand quantity from its `seed` through `src/hash.rs`. Both hashes there are frozen; changing either moves every digest the crate has ever printed.
- **Tearing is monotone.** A `torn` flag is set and never cleared, in `Strand` and in `Mesentery` alike. That is what makes a tear a state change rather than a threshold the sim can chatter across. The mesentery's `torn` is parallel to its `anchors` and the solver's `canonicalise` sorts them *together*; a change that reorders one without the other migrates a tear to a different link, which is the same bug as clearing it.

#### Rules

- **Bevy 0.19 is pinned.** Read the vendored `bevy-0.19.0` source, not bevy.org — that documents `main` and has been wrong for this pin more than once. Three traps already paid for: a missing `Res<T>` **panics** its system rather than skipping it, `Resource` is a subtrait of `Component` so you cannot derive both, and `add_plugins` tuples cap at 15.
- **No `unwrap()`**, no `expect`, no panicking index. Every index in `src/solver.rs` is proved in range by the single `let n = …min()…` line at the top of `solve_one`; keep it that way rather than adding a guard per access.
- **One path per feature.** No fallbacks, no legacy shims, no stub placeholders. Degenerate input is *clamped and reported*, which is one path with a guard, not two paths.
- **The crate never spawns.** `tube_mesh` returns a `Mesh`. Choosing the material, the handle, the parent and the schedule slot is the caller's job, and a crate that chose them would be unusable in any game that wanted different ones.

#### Where the boundary falls

Four things belong to the caller, and each is a thing this crate deliberately cannot do:

- **Rendering.** A `Mesh` comes back; nothing is spawned, no material is chosen, no asset arena is written.
- **The wound.** Where guts come from, when, and how many, is gameplay. `spill` takes a point, a direction, a count and a seed.
- **Collision beyond the floor.** Strands do not see each other and do not see the world. Self-collision on a coiling rope needs a broadphase the caller already owns.
- **Blood.** `tests/leaf.rs` allows exactly one dependency, `bevy`. A rope solver has no business knowing about rheology, and widening that list should cost a deliberate edit rather than a passing build.

#### Interpretations recorded at the time

Two numbers in the contract needed a reading, and both are written down at the constant rather than buried in behaviour:

- **A mesenteric link is a pin — rest length zero — and its strain is `|node − anchor| / rest_len`**, because `Mesentery` carries no length of its own and the strand's segment rest length is the only length scale in the data. The other reading, a link whose rest length is one segment, was tried and is measurably wrong: it leaves the link slack for 35 mm and then gives it 12 mm of working range, which a node already at terminal velocity crosses inside one substep, so every tether tore, always, and the flag stopped meaning anything.
- **`COMPLIANCE_MESENTERY` is a crate constant, not a `ViscSettings` dial**, and its value is derived rather than eyeballed — the derivation is in its doc comment. A compliant XPBD constraint settles at `C = (1 + α̃)·g·Δt²·N` for `N` nodes of hanging weight, so the compliance is what decides the capacity, and both neighbourhoods of the shipped `6e-5` are dead: far softer and everything tears, far stiffer and nothing ever does.

### `cross_section` (was `bevy_cross_section`)

#### The table has sources, and a number without one says so

`Layers::for_region` is the crate. Every value in it is either a number from a paper cited in `src/layers.rs`'s module docs, a stated derivation from one (the head split), or flagged as this crate's own (cortical bone). A new number goes in with its DOI or with the flag; never silently. The test `the_measured_rows_are_the_papers` pins the sourced ones to the tenth of a millimetre they were reported at, so retuning a band for looks fails a test — which is the point.

#### Depth is measured, never guessed

`depth_below_skin` is exact for a point inside a convex cell with the cell's supplied faces as planes. Do not replace it with a distance to the mesh, a raycast, or a per-vertex "inset" — those are approximations of a quantity this crate can compute exactly, and they would cost a mesh query where this costs a dot product.

#### `UV_0` is not this crate's to touch

Caps arrive with planar cross-section UVs in `UV_0` that other crates and their goldens depend on. This crate writes `UV_1` and nothing else. A material that wants the bands samples through `UvChannel::Uv1`.

#### The strip is a pure function

`strip(layers, width, height, seed)` reads nothing but its arguments and `bloodstain::hash_f32`. No clock, no RNG crate, no global. `the_strips_are_frozen` pins the digest of all three regions; if a change moves it, that change re-blesses the golden deliberately, in the same commit, with the reason.


### `flaymap` (was `bevy_flaymap`)

#### The CPU is the authority. The GPU is write-only.

This is the crate's entire reason to exist. Texture-space damage masking is shipped technology and
everywhere it exists it is a GPU render target — Frostbite 2 revealed a layered material out of one
(Kihl, SIGGRAPH 2010 Advances course) — which is why nobody can hash one. This crate keeps the removed
depth on the CPU in integers and treats the two `Image`s as output.
`crates/bevy_carnage/src/vfx.rs:6-19` states the rule and the reason.

So: **nothing reads `Assets<Image>`.** A future idea of the form "sample the flaymap in a shader and
feed the result to gameplay", or "read the uploaded pixels back to place a decal", must be refused —
the depth buffer is already there and is already the authority. `digest()` folds that buffer, never the
images, and that is not an implementation detail: it is the claim.

#### Depth is one `u16` per texel, monotone, and row-major

Hundredths of a millimetre removed, saturating at `Layers::span_mm()`. Row-major **is** the canonical
order every pass walks, so no sort is needed and none may be added: a sort would be a second answer to
a question the layout already answers, and the digest would then depend on which answer ran.

**Tissue does not grow back.** Every write either adds to a texel or leaves it alone. Do not add a
"wounds close over time" path — a second rule for the same quantity is how a monotone buffer starts
oscillating three months later, and it would put a clock in a crate that has none.

Integers rather than an `f32` because the buffer is the thing the digest folds: a float accumulation
would make the wound depend on the order the hits were summed in.

#### `shade` is derived, and deliberately outside the digest

`shade` is a pure function of the depth buffer, the layer table and `FlaySettings`. That is why
`digest()` does not fold the pixels: it would hash the same information twice, and a palette tweak
would then read as a simulation divergence. If you ever need the pixels in a hash, the depth buffer is
what you actually want.

Every peeled texel's colour and roughness is `bevy_cross_section::texel_at` — **the identical
per-texel rule that bakes a cut face's strip**. Do not author a flaymap palette here. A flayed patch
and the stump beside it drifting apart is exactly what one shared function prevents.

#### A hit peels a crater, not a cylinder

`paint_uv` adds its full depth at the centre and smoothstep-falls to zero at the radius, quantised to a
byte so the depth a texel receives is an integer multiply. A flat disc would stack into a bore with a
vertical wall and the layer bands would never show; the falloff is what makes the rim readable, and it
is the crate's whole visual. The stamp's edge length is forced **odd** so the footprint is symmetric
about its centre texel — an even one would drift a crater by half a texel per hit.

#### The bone handoff fires exactly once

`Handoff::bone_reached` is true on the first paint call in which any texel crosses
`Layers::starts_mm()[3]`, and false forever after; `bone_handed_off` is the whole mechanism and
`BoneExposed::from_handoff` is the whole gate. A flag that stayed true would make a consumer spawn a
fracture proxy, or a bone-scrape sound, once per shot for the rest of the fight.

`at` and `normal` are `Some` from `paint_world` and `None` from `paint_uv`, and that asymmetry is not
an oversight to tidy up: a UV names a point on an *atlas*, and a seam maps one UV to several places on
a body, so a `paint_uv` that invented a mesh position would be guessing.

#### A UV off the canvas is refused, not clamped

This is the one place the crate deliberately differs from `bevy_wetmap`, which clamps to the edge.
Blood on the wrong texel is cosmetic; peeling a body's edge texels to bone because a ray came back with
a UV of `1.4` is a gameplay error, and the caller would then get a bone handoff for a hit that never
landed. One `warn!` per canvas, then silence.

#### No clock, no RNG

`paint_uv` takes the tick *number* and `shade` is called by the caller. Nothing here reads a clock,
virtual or real, and nothing may. The only random source anywhere in the crate is
`bloodstain::hash_f32`, reached through `texel_at`, keyed by integer lattice coordinates and
`FlaySettings::seed` — so a wound is a pure function of the hits that made it, and
`the_scripted_wound_is_frozen` can pin a digest.

#### The caller owns the schedule and the message

`FlaymapPlugin` registers exactly one system, the upload budget, because uploading is the only part
with no gameplay opinion in it. It registers `BoneExposed` and **writes it nowhere**: only the caller
knows whether the thing it peeled has a skeleton something else owns.

Both of that system's resources are `Option`, deliberately. Bevy 0.19 *panics* a system with a missing
`Res<T>` rather than skipping it, and `Assets<Image>` belongs to a plugin this crate does not add.

#### No shader, no asset, no material

The crate hands back two `Handle<Image>` and never names a material — which is why it does not depend
on `bevy_pbr`. The intact surface is written *into* the canvas on the CPU, so there is nothing left to
blend in WGSL. If a caller's wound looks flat, the cause is almost always that their `StandardMaterial`
left `perceptual_roughness` at the shipped scalar, which Bevy multiplies by the texture
(`bevy_pbr-0.19.0/src/pbr_material.rs:157-163`); it must be `1.0`.

Roughness is the **green** channel and metallic the **blue** one, stated at
`bevy_pbr-0.19.0/src/pbr_material.rs:153-154`. The metallic-roughness image is `Rgba8Unorm`, **not**
sRGB: it carries material data, not colour.

#### Rules

- **Bevy 0.19 is pinned.** Read the vendored source, not bevy.org — that documents `main` and has been
  wrong for this pin more than once. `bevy_render` is in the feature list for exactly three type names
  (`Extent3d`, `TextureDimension`, `TextureFormat`); see the comment in `Cargo.toml`.
- **No `unwrap()`, no `expect`, no `panic!`, no panicking index in library code.** The hot loops use
  `get`/`get_mut` with a `continue` guard even where the index is in bounds by construction, so the
  crate contains none at all — and `tests/leaf.rs::the_library_holds_no_panicking_call` sweeps `src/`
  for them. Tests may panic; that is what an assertion is.
- **One path per feature.** No fallbacks, no legacy shims, no stub placeholders, no setting that picks
  between two implementations of one thing. A non-finite `scale.mm_per_unit` collapses the noise onto
  one phase rather than selecting a second scale.
- **`bevy_wetmap` is a sibling, not a base.** `src/uv.rs` reimplements Möller–Trumbore rather than
  importing its copy, so an actor that only ever gets flayed does not resolve a blood-drip model it
  never calls. `tests/leaf.rs` is the ratchet: allowed dependencies are `bevy`, `bevy_cross_section`,
  `bloodstain` and `serde`. Widening it is a design decision and should cost a deliberate edit there.

### `laceration` (was `bevy_laceration`)

#### The tear is a pure function, and `digest` is how we know

`tear(mesh, path, normal, shape, region, layers, scale)` reads nothing but its arguments and `bloodstain::hash_f32`. No clock, no RNG crate, no global, no query order. `the_tear_is_frozen` pins both output digests; if a change moves either one, that change re-blesses the constants deliberately, in the same commit, with the reason — and `examples/laceration_curve.rs` prints the same two numbers, so a reader can check the claim without the suite.

The example's half-width is spelled `CELL * 1.2`, not `0.06`, because those are different `f32` bit patterns and a digest is a digest of bits. Keep it that way.

#### The gape only ever opens

The time curve is monotone by construction and there is no closing half. That is not a simplification — it is O'Brien, Bargteil & Hodgins (2002), `doi:10.1145/566570.566579`: plastic deformation is *retained* ahead of separation, so a laceration cannot spring back. Adding a heal, a close or an elastic recoil is a different model and needs a different name.

#### Every retear starts from the intact source

`Laceration::source` is never written. The gape is a function of `(clock - opened_at)` and nothing else, so re-running the system twice on the same tick produces the same mesh — an accumulating edit would drift, and a drifting wound cannot be hashed or rewound. The plugin refuses, with one warning, when the entity's own `Mesh3d` handle *is* the source, because that would destroy the intact copy on the first frame; `a_laceration_that_draws_its_own_source_is_refused_rather_than_destroying_it` pins it.

#### The vertex buffer keeps its length

Only positions and indices are rewritten. Everything else — normals, `UV_0`, `ATTRIBUTE_JOINT_INDEX`, `ATTRIBUTE_JOINT_WEIGHT`, vertex colours, anything custom — arrives untouched because the skin mesh is a clone of the input, not a rebuild. Do not "optimise" this by compacting the buffer: a re-index is a copy per attribute per retear, and getting it wrong is a limb following the wrong bone.

#### No number without a source, and a made-up number says so

Three papers carry this crate, cited in `src/curve.rs` and `src/tear.rs` module docs with their DOIs. Everything else is flagged in the doc comment as this crate's own: `Gape::open_ticks` (no paper gives a *rate*), the `3` in the exponent (the 95 %-at-`open_ticks` choice), `RAIL_WANDER` and `WANDER_MM` (nothing tabulates the raggedness of a wound margin), and `ALONG_LANGER_FACTOR`, which is a **stiffness** ratio used as a **gape** proxy. A new number goes in with its DOI or with the flag; never silently.

#### Nothing panics, and refusals are loud once

No `unwrap`, no `expect`, no indexing that can fail, anywhere in `src/`. `tear` returns `Option` and every refusal is a `warn_once!` naming what was wrong. Read meshes through `try_attribute_option` / `try_indices_option`, **never** `attribute()` / `indices()`: Bevy 0.19's plain accessors `expect` when a mesh's vertex data has been extracted to the render world, and that is reachable from a caller who authored `RenderAssetUsages::RENDER_WORLD`.

Every resource in a system signature is `Option<Res<..>>` or `Option<ResMut<..>>`, including ones this plugin inits, because in 0.19 a missing `Res<T>` *panics* the system rather than skipping it — and `CrossSectionSettings` being absent is a supported configuration, not an error.

#### Depth belongs to `bevy_cross_section`

The bed's `UV_1` comes from `uv1_at`, never from a fraction computed here. One definition of depth-below-skin, shared with every cut face in the family; `the_bed_floor_sits_at_the_authored_depth` pins that a rail reads 0 and the floor reads `bed_depth_mm / span_mm`.


### `fracture_modes` (was `bevy_fracture_modes`)

#### Every numerical routine is a fixed schedule

The eigen-initialisation is cyclic Jacobi for a fixed number of sweeps; the solver is a fixed number of ADMM steps; the Cholesky has no pivoting. None of them has a convergence test, and that is the design, not an omission: a convergence test is a branch on floating-point data, and a branch is where two machines part company. `a_bake_is_a_pure_function_of_its_inputs` asserts bit equality between two bakes. If you need more accuracy, raise `iterations` or `eigen_sweeps` — never add an early exit.

#### The modes are scalar, and the direction comes back at impact

The paper's modes are vector fields. On a cell graph with one translation per cell, `E_D` cannot distinguish directions, so a vector mode is a scalar mode times a direction and the direction factors out of the gluing norm. Do not "upgrade" the unknowns to three per cell: it triples the work to recover a degeneracy, and it moves every golden.

#### No linear-algebra dependency

`src/linalg.rs` exists so the solver's every operation is readable and fixed. `tests/leaf.rs` refuses `nalgebra`, `faer`, `ndarray` and `rand` by name.

#### The plugin registers no systems

A bake needs a `CellGraph`, and only the crate that owns a decomposition knows when it is complete. The plugin adds two resources and stops. A system that polled for graphs would be a schedule this crate does not own.


