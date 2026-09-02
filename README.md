# bevy_carnage

> ⚠️ **Vibe Coded** — written by an AI agent working from a human's direction. It is used in a shipping game and covered by tests, but it has had no line-by-line human audit. Read it before you trust it.

**Deterministic runtime gore for Bevy.** Plane-cut a character's own meshes into watertight-capped chunks, bore bullet channels through them, and drive blood, spatter and impact feel off the wounds that result.

![A blue blocked-out humanoid standing intact, then bursting into tumbling rounded chunks whose cut faces are raw red while their outer surfaces stay blue](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/explode.gif)

That is `examples/explode.rs` at its own 0.4× playback. The subject is intact, then it *is* its own fragments — the "break" is one despawn and a spawn, because the fracture was computed long before. **The red is not a colour choice, it is the whole idea:** every fragment comes back as two meshes, the subject's own surface and the faces this cut just created, so the inside can take a different material. Render both with the skin material and the same fragments stop looking broken and start looking disassembled.

> **This repository is the source of truth.** Work is done here, verified here against this lockfile, and pushed here; [`Ladvien/foundation_vs_slop`](https://github.com/Ladvien/foundation_vs_slop) is an ordinary consumer that depends on the crate as a git dependency pinned to a rev. Two earlier arrangements are recorded in `BACKLOG.md` because each cost a session, and neither is live — at one point the crate was vendored into that monorepo as a workspace member with this repository re-derived by `git subtree split`, and an earlier revision of this banner asserted that arrangement as permanent. **The stale-read hazard it named is real in either direction:** a `subtree split` carries only *commits*, so anything living uncommitted in a working tree cannot arrive on the far side at all — which is exactly how a research agent once read a copy, found no `isomesh` and no audit harness, and reported both as missing when both existed. Read the tree you are about to change, not a copy of it.

## Features

- **One bake, every granularity.** A bake keeps the whole hierarchy it cut through, so the same cached asset answers "break this into three" and "break this into forty" without cutting twice.
- **Localised damage.** A bond graph records which fragments actually share a face. Shoot a shoulder and the arm comes off while the body stays standing — the joints are found by geometry, not authored.
- **Five region queries** — projectile, slash, swept blade, blast, directional pull — each a pure function of the bake plus some geometry.
- **Bullet holes that go through, and gore that comes out the far side.** A segment plus a radius subtracts a convex prism from the proxy, in closed form and as plane cuts only, so the channel is real geometry with a red interior, the pieces around it stay bonded, and every shard is still a convex-hull collider. The material the channel removed comes back as `Ejecta` — a spawnable convex chunk with the wall's raw interior and a patch of skin at each end — so the hole and the gore are the *same subtraction*, and the bore conserves volume. Dials for radius, barrel sides, raggedness, exit flare, and how many pieces the plug breaks into — because one convex prism ejected whole reads as a dowel, which is exactly what an apple corer leaves.
- **Progressive destruction.** Hit it again and it comes apart further. Island detection is stateless; you own the damage state.
- **Two meshes per fragment** — the subject's own skin and the newly-cut faces, separately, so the inside can take a different material.
- **Solver-ready colliders.** Every fragment is one convex cell. `Collider::convex_hull(frag.cell.points())` and you are done — no decomposition at spawn, no trimesh.
- **Shape and look dials** — off-centre cuts, size spread, weak-axis bias, crumpled cut faces, rounding — so the output reads as torn rather than as shattered ice.
- **Reproducible.** Two runs of the same build on the same asset produce bit-identical fragments, and a test enforces it.
- **Wounds, as values.** `Wound { at, normal, area, severity, kind }` — subject-local, derived only from baked geometry. A severed bond *is* a wound (its centroid, normal and area, with no arithmetic); a bore's channel wall is one too, because `face_is_cut` already answers `true` for it. So a bullet hole bleeds through exactly the same code a bisection does, with no second path.
- **A spatter model with a citation, not a preset.** Comiskey, Yarin & Attinger (*Phys. Rev. Fluids* 3, 063901, 2018) show blood disintegrating by *percolation*, which makes droplet size and initial speed **inversely** correlated — many small ones fast, few large ones slow, bracketed by their measured 40 m/s forward and 8 m/s back spatter. One draw sets size and its inverse sets speed, on the CPU and in the shader both, and a test asserts the correlation (Pearson `r < -0.9`) rather than a comment claiming it. Landing points are solved in closed form, so **where blood lands exists with the render feature off** — which matters when a pool's position feeds simulation.
- **A pulsatile bleed schedule in integer ticks.** One state machine over `tick - opened_at`: integer modulo for the heartbeat, so a pulse train cannot drift or depend on frame rate, and a monotone taper to *exactly* zero at the clot. `pulse_wound` scales the wound's severity, so the first arterial jet and the last seep are the same model at two numbers.
- **Impact feel as numbers you apply.** Trauma, hit-stop ticks and a tick-indexed shake offset eased along the wound normal — grounded in Pichlmair & Johansen's game-feel survey, which says in as many words that shake should *not* be random. The crate never writes `Time<Virtual>` and never touches a camera: it returns values and you own both.
- **GPU blood behind a feature.** `bevy_hanabi` is optional and gated on `vfx`, so the default-off build resolves no particle system and no render stack. It is admitted on the terms that keep the boundary meaningful, and one of them is sharp: Hanabi 0.19 has **no GPU→CPU readback at all**, so a particle cannot reach a golden even by mistake.
- **No physics dependency, no RNG dependency, no game logic.** `bevy`, optional `serde`, `isomesh` for validation, and optional `bevy_hanabi` behind `vfx`. `hash_f32` is the only source of randomness in the crate, and it is frozen by a test.

## Install

Not on crates.io (`publish = false`). Depend on it by git, pinned to a rev:

```toml
[dependencies]
bevy_carnage = { git = "https://github.com/Ladvien/bevy_carnage", rev = "..." }
```

Requires **Bevy 0.19** and a Rust toolchain with **edition 2024**.

| feature | default | what it does |
|---|---|---|
| `serde` | ✅ | `Serialize`/`Deserialize` on the settings and hierarchy types, so a game can author dials in RON |
| `strict-order` | — | Turns on the vertex-soup sort's runtime tie check in release. On automatically under `debug_assertions`; this is for a harness that builds in release and still wants it |
| `vfx` | ✅ | GPU blood (`bevy_hanabi`) and stain decals. Turn it **off** for a headless or server build: the deterministic half — wounds, spatter, stains, bleed schedule, feel — is entirely outside it |

## Quick start

Mark what should break, then read the bake back when it dies. The launch is yours, and so is the solver.

```rust
use bevy::prelude::*;
use bevy_carnage::{CarnagePlugin, CarnageSystems, DetachedPart, FractureCache, FractureSubject};

#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
enum GameState { #[default] Playing, Paused }

fn wire(app: &mut App) {
    app.add_plugins(CarnagePlugin)
        // The crate configures no run condition — when the bake runs is yours.
        .configure_sets(Update, CarnageSystems.run_if(in_state(GameState::Playing)));
}

// Mark what should break, and what should come off intact.
fn spawn_enemy(mut commands: Commands, assets: Res<AssetServer>) {
    let scene: Handle<WorldAsset> = assets.load("enemy.glb#Scene0");
    let enemy = commands.spawn((FractureSubject(scene.clone()), WorldAssetRoot(scene))).id();
    // ...once the scene streams in, tag whatever should detach intact:
    commands.entity(enemy).insert(DetachedPart);
}

// At the moment of death, read the bake at whatever granularity this death deserves.
fn on_death(cache: Res<FractureCache>, subject: &FractureSubject) {
    for frag in cache.leaves(subject.0.id()) {          // finest
        let _ = (&frag.outer_mesh, &frag.cap_mesh, frag.center_local, frag.half_extents);
    }
    for frag in cache.frontier_of(subject.0.id(), 3) {  // the same bake, as three big chunks
        let _ = frag.id;
    }
}
```

You do not need an `App` to use the fracture itself. `fracture_mesh` is the whole pipeline with no assets and no ECS — meshes in, meshes out:

```rust
use bevy::math::{Mat4, Vec3, primitives::Cuboid};
use bevy::mesh::Mesh;
use bevy_carnage::{CutSettings, ProxyCell, fracture_mesh};

let body = Mesh::from(Cuboid::new(1.0, 2.0, 1.0));

// **The proxy is yours.** This crate cuts a convex decomposition and carries your triangles along
// as a payload — it never cuts the triangle soup. One cell per connected shell; a consumer already
// running V-HACD or CoACD for colliders has these, and a blocked-out subject can use `from_box`.
let proxy = vec![ProxyCell::from_box(Vec3::ZERO, Vec3::new(0.5, 1.0, 0.5))];

let cut = CutSettings::new(
    12,          // finest fragment count
    0.15,        // stop cutting below this fraction of the subject's size
    0xC0FFEE,    // seed — same seed, same pieces, every run
);
// Tier A — the convex cells — exists for every node the instant this returns. The drawn meshes are
// built for the nodes you ask for, which is why the accessors take `&mut`.
let mut baked = fracture_mesh(&[(&body, Mat4::IDENTITY)], &proxy, &cut);

assert_eq!(baked.frontier_of(3).len(), 3);   // one bake, read back as three pieces
let pieces = baked.leaves();                 // ...or as all of them
assert!(pieces.len() > 1);
assert!(pieces.iter().all(|p| p.outer.is_some() || p.cap.is_some()));
```

## Demos

**[docs/DEMOS.md](docs/DEMOS.md) — all six examples, with recordings**, what each is for, and how to regenerate the clips.

![A blue blocked-out humanoid standing; a projectile takes off its arm, another its head, a slash takes the other arm, a blade through the waist takes both legs and a blast finishes the torso](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/sever.gif)

That is `examples/sever.rs`, on a fixed script. The subject **stays standing** between blows and what comes off depends on where you hit it. Run it and you aim it yourself:

```sh
cargo run --release --example carnage          # needs a GPU
cargo run --release --example sever            # needs a GPU
cargo run --release --example explode          # needs a GPU
cargo run --release --example bullet_holes     # needs a GPU
cargo run --release --example fracture_cube    # terminal only — no window, no GPU
```

```text
  arrows / WASD   move the aim marker
  1 … 5           projectile · slash · swept blade · blast · pull
  G               granularity — cycle which frontier of the bake is standing
  T               soften — cycle how hard the drawn fragments are rounded
  R               reset
```

![A blue blocked-out humanoid standing still while five shots punch through it one at a time, each leaving a small dark-red hole in the blue skin and throwing a handful of rounded red chunks out the far side that arc down, land and spread into overlapping dark pools on the floor, then the camera orbits a third of a turn to show the wider exit wounds and the spatter together](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/holes.gif)

That is `examples/bullet_holes.rs`, on a fixed script. Each shot is a `Bore` — a segment, a radius and three look dials — subtracted from the proxy before any cut, so the hole has a wall rather than being painted on a surface, and the plug it removed is thrown out the exit side as a chunk of gore that lands and becomes a flat stain. The subject keeps standing because the shards around a channel share their radial faces bit-for-bit and the bond graph reads them as one island. Run it and you aim it yourself, with `[`/`]` for calibre, `J` for raggedness, `F` for exit flare and `K` for how many pieces the plug breaks into.

![A blue blocked-out humanoid; a shot punches a channel through its chest which mists blood from the hole, then four blows in turn take an arm, the head, the other arm and both legs, each cut throwing a red spray outward along the face it opened while dark stains pile up on the floor beneath; the severed pieces keep pulsing blood at a heartbeat's rate as they lie there, the pulses weaken, and by the end the floor is soaked and the bleeding has stopped](https://raw.githubusercontent.com/Ladvien/bevy_carnage/main/docs/carnage.gif)

That is `examples/carnage.rs`, on a fixed script — and it is the one clip that shows the whole crate at once. **A bullet hole and a severance are geometrically different openings, and they bleed through the same code**: frame 18 is a bore, the rest are severances, and nothing downstream can tell them apart beyond a `WoundKind` mixed into a seed. The spray leaves each cut along that cut's own normal, the floor stains are solved on the CPU and would exist with the render feature off, and every severed piece keeps pulsing at its own heartbeat until it clots. Run it and you aim it yourself, with `1`–`5` for the five blows and `6` to shoot a channel through.

Its recorder, `capture_carnage`, prints one line that two runs must agree on — `carnage: frames=… wounds=… stains=… digest=…` — which is the determinism check for the whole layer: the bake, the bond graph, wound extraction and its canonical sort, the wound seed, the droplet draws, the ballistic solve and the pulse schedule. A digest that moves means something read a clock, an `Entity`, or an unsorted iteration order.

## Why: break the asset once, not the frame

The tempting version of this feature computes a fracture at the moment of impact. The shipped version, in more or less every game that has one, does not — it **pre-fractures the asset and swaps it in at runtime**, which is exactly what Müller, Chentanez & Kim document as the practical norm (ACM TOG 2013). Sellán et al.'s *Breaking Good* (ACM TOG 2022) is the most physically honest option available, and its fragments are precomputed **offline** through tetrahedralization and a conic solve in a Python/libigl toolchain — not something a minimal-dependency Rust crate can embed.

**But a static pre-fracture cannot answer where it was hit**, and Müller says so plainly: *"When a gamer shoots at a glass window, she expects the spider-web-shaped fracture pattern to be centered around the location where the bullet hit the glass. Anything else clearly destroys the illusion."* So the bake is not a fragment *set*. It is a hierarchy plus an adjacency graph, and every blow is a query against it. That is the shape PhysX Blast and Unreal's Chaos arrived at independently.

**The cut runs on a convex proxy, never on the triangle soup.** Müller's load-bearing observation is that `plane ∩ convex polyhedron = convex polygon`, which makes every cut face convex by construction and a centroid fan over it provably valid. There is no boundary-loop recovery in this crate because there is no input that needs one. The render triangles ride along as a payload, assigned to whichever cell bounds them and split only where they straddle a plane.

The cost of that choice is the proxy itself: **you supply it.** See "What it deliberately does not do".

## The API

### Baking, in the ECS

| item | kind | what it is for |
|---|---|---|
| `CarnagePlugin` | `Plugin` | Registers the cache, the settings, and the bake |
| `CarnageSystems` | `SystemSet` | On `Update`. Gate and order against this, not the system |
| `FractureSubject(Handle<WorldAsset>)` | `Component` | What to break. The cache key *and* the seed source |
| `FractureProxy(Vec<ProxyCell>)` | `Component` | Your convex decomposition, subject-local. Required |
| `DetachedPart` | `Component` | A subtree pruned out and baked as one intact chunk — a carried weapon, a hat |
| `FractureSettings` | `Resource` | Twelve bake dials. `init_resource`d, so yours wins if inserted first |
| `bake_fractures` / `materialise_fragments` | fn | The two systems, chained. Public so they can be named in an ordering constraint |

### Reading a bake back

| item | what it is for |
|---|---|
| `FractureCache::leaves(source)` | The finest granularity — every fragment that was never cut further |
| `FractureCache::frontier_of(source, n)` | The same bake as roughly `n` pieces. **The granularity dial** |
| `FractureCache::at_depth(source, d)` | Every branch cut to the same level |
| `FractureCache::tree(source)` | The hierarchy itself: `FragmentTree`, `TreeNode`, `FragmentId` |
| `FractureCache::bonds(source)` | The `BondGraph` for the finest frontier |
| `FractureCache::detached_chunk(source)` / `is_baked(source)` | The pruned part; whether the bake has happened |
| `FractureCache::solids(source)` | Tier A for **every** node — the convex cells, present the instant the bake finishes |
| `FractureCache::request(source, ids)` / `ready(source, ids)` | Ask for a frontier's drawn meshes a frame ahead; check they arrived |
| `Fragment` | `id`, two mesh handles, the convex `cell`, `center_local`, `half_extents` |

### Where the blow landed

Each query returns a `Reach` — a severity in `[0, 1]` per bond — and **you** pick the threshold at which one gives way.

| item | models |
|---|---|
| `spread(graph, at, min, max)` | a projectile — nearest fragment, then outward **along the bonds** |
| `capsule(graph, a, b, min, max)` | a swung edge — falloff from the segment a blade travelled |
| `swept_triangle(graph, a, b, c)` | a swept blade — every bond the swing passed *through*, no falloff |
| `radial(graph, at, min, max)` | a blast — falloff from a point in open space |
| `directional(graph, at, dir, min, max)` | a pull — weighted by how squarely each face meets it |
| `Reach::above(threshold)` | the bonds that give way. Scale severity by material first if you like |
| `BondSet` | your accumulated damage. `sever_all`, `is_broken`, `severed` |
| `BondGraph::islands(members, broken)` | what is still connected. Stateless — call it after every blow |
| `BondGraph::of(members, capacity)` | build a graph for a frontier other than the finest |

### Geometry, with no ECS

| item | what it is for |
|---|---|
| `fracture_mesh(parts, proxy, &CutSettings)` | The whole pipeline. Meshes in, `Fracture` out |
| `Fracture` | `solids()` + `tree` + `bonds`, with `leaves()`, `frontier_of()`, `at_depth()` (all `&mut`: Tier B is built on request) |
| `CutSettings` | The geometry dials for one bake, without the ECS sizing policy |
| `ProxyCell` | One convex cell. `from_box`, `points()` (→ your collider), `volume()` (→ mass) |
| `audit_proxy` / `audit_render` / `SolidAudit` / `SurfaceReport` | Measure what the fracture produced |
| `hash_f32` | The frozen integer hash the fracture seeds from |

### The carnage layer

All of it pure functions over a `Wound`, all of it available with `vfx` off. Nothing here reads a clock, spawns anything, or writes a `Transform`.

| item | what it is for |
|---|---|
| `Wound` / `WoundKind` | The one value everything downstream reads: `at`, `normal`, `area`, `severity`, and which of the two openings it was |
| `wounds_from_reach(graph, reach, threshold)` | The wounds a blow opened, sorted by `BondId` so the order is a function of the graph |
| `wounds_from_bonds(graph, broken)` | The wounds a set of already-severed bonds represents |
| `cap_faces(cell)` / `largest_cap(cell)` | Every raw-interior face of a convex cell, and the widest one — "the wound on this chunk" |
| `wound_of_channel(cell, exit, direction)` / `wound_from_ejecta(chunk)` | A bore's channel, as a wound. Its area is the channel *wall*, not the entry disc |
| `Wound::to_world(&GlobalTransform)` → `Wounded` | Subject-local to world. A normal goes through the affine's linear part, never `transform_point` |
| `Wounded` | The crate's one message. `CarnagePlugin` registers it; you write it |
| `droplet(w, i, s)` / `droplets` / `droplet_count` | The spray. Indexed, so any subset recomputes without the rest |
| `landing(from, &droplet, gravity, plane_y)` | Closed-form ballistic landing. `None` when it never crosses — no invented answer |
| `stains(w, s, plane_y)` / `Stain` / `stain_radius` | Where blood lands and how wide it reads. **Core, not cosmetic** |
| `Bleed` | `Component`. `opened_at` and `area`; everything else is derived from `tick - opened_at` |
| `pulses_on` / `flow` / `clotted` / `pulse_wound` / `pulse_period` | The heartbeat, the taper, and the clot — integer ticks throughout |
| `trauma_for` / `hitstop_ticks` / `shake_offset` | Impact feel, as numbers. **You** apply them |
| `CarnageSettings` | Eighteen carnage dials, `deny_unknown_fields` with a serde default per field |
| `BLOOD_DENSITY` / `BLOOD_SURFACE_TENSION` / `FORWARD_SPATTER_SPEED` / `BACK_SPATTER_SPEED` | The measured constants, each with its citation |

Behind `vfx` only: `CarnageVfxPlugin`, `CarnageVfxSystems`, `CarnageEffects`, `EffectTtl`, the five effect builders, and `SplatTextures` / `spawn_stain` / `splat_image` / `build_splats` for stain decals. A camera rendering a stain **must** carry `DepthPrepass`.

### The dials

`FractureSettings` sizes the bake per asset; `CutSettings` is what a single cut actually needs.

| dial | default | effect |
|---|---|---|
| `pieces_base` / `ref_extent` / `min_pieces` / `max_pieces` | 14 / 0.5 / 6 / 40 | Fragment count, scaled by the mesh's own size |
| `min_fraction` | 0.18 | Stop cutting a piece below this fraction of the subject's size |
| `max_depth` | 12 | Cuts from a proxy cell to the finest fragment — the hierarchy's memory bound |
| `plane_jitter` | 0.35 | How far a cut plane slides off centre. `0.0` halves every piece and reads as uniform shards |
| `size_spread` | 0.5 | How much the largest-first cut order may be nudged, widening the size distribution |
| `weak_axis` | 0.75 | How hard to cut across a piece's *narrow* dimension — where a real thing comes apart |
| `cap_relief` | 0.30 | How much the drawn cut face is crumpled |
| `soften` | 0.5 | How much the drawn fragment is rounded. **Tier B only** — the collider never changes |

The last four are look dials, and the last two touch only the *drawn* mesh: the proxy cell stays exactly planar and convex, so colliders and every watertightness guarantee are untouched. `fracture_cube` prints the proof — cell volume identical at every `soften`, drawn area falling away.

## What it deliberately does not do

**It does not compute a convex decomposition.** You supply the proxy cells; the crate cuts them. A consumer already running V-HACD or CoACD for colliders has a decomposition, and forcing a second, different one would be the fracture disagreeing with the physics about what the object is. `ProxyCell::from_box` covers a blocked-out subject. Boring is not an exception to this: a `Bore` subtracts a prism *you* described from a cell *you* supplied, by plane splits with a closed-form decomposition, so what comes back is still a set of cells whose union is the shape you handed in — and a concave cell is still refused at the door.

**It does not move anything.** No rigid bodies, no velocities, no physics dependency. The bake hands you a mesh and a convex cell per piece; spawning them, building a collider and throwing them is your game's decision and your solver's job. `examples/explode.rs` integrates its own ballistics in thirty lines to make the point.

**It does not know what died.** No health, no factions, no damage types. It hands back a *reach* — geometry — and your game decides what an area is worth and what threshold means "this gives way".

**It does not own your schedule.** The plugin adds one system to `Update` in one public set. Whether that runs while a menu is open is yours to configure.

**It does not bond cells that merely touch.** Two cells are neighbours only when they share a *coplanar* face. V-HACD and CoACD produce cells that abut without their boundary polygons agreeing, so each root's subtree comes out as its own island unless your decomposition shares faces. Closing that with a proximity heuristic would silently weld a head to a torso.

**It fractures the bind pose.** Geometry is read from `Mesh3d` as authored, so a skinned character breaks from its rest pose, not its death pose. A death-pose snapshot is the proper upgrade; per the fracture literature the gap is not visible when the chunks are flung fast.

## Determinism

Two runs of the same build, on the same asset, must produce bit-identical fragments. Two things make that true, and both were learned the hard way:

**The seed comes from the asset PATH, never its `AssetId`.** An `AssetId` is a slot index in the asset arena, handed out by async load order — so the same file gets a different id run to run, hashes to a different seed, and the mesh is partitioned along completely different planes. Measured, before the fix: 23 of 23 fragments differed, in half-extents as well as centres.

**The vertex soup is assembled in a canonical order.** `Children` order for a glTF scene is whatever order async instantiation happened to add nodes in. Fragment centroids are float sums over the merged soup, float addition is not associative, and so an unsorted soup moves every centroid by a few ULPs — same fragment count, positions off in the last bits. Sub-meshes are sorted by `(mesh asset path, world-matrix bits)` before a single vertex is appended, and the key is checked at runtime for uniqueness under `debug_assertions` or `strict-order`.

`hash_f32` is a hand-rolled integer hash with its output frozen in a test, for the same reason: there is no RNG dependency here, because a stream that may change between minor versions cannot underwrite any of the above.

Note what this does *not* claim. Fragment geometry is `f32` arithmetic, so cross-architecture bit-identity is not promised — only same-build, same-machine reproducibility, which is what a replay or a regression golden actually needs.

## Status

**0.1.0, pre-release, and not published to crates.io.** It is used in one shipping game, which consumes it by rev.

| area | state |
|---|---|
| Cutting, capping, colliders | Stable. Every fragment audits as a closed convex solid, swept across seeds and shape dials |
| Hierarchy, bond graph, region queries | Landed and tested; the API may still move before 0.2 |
| Look dials (`plane_jitter`, `size_spread`, `weak_axis`, `cap_relief`, `soften`) | Newest surface here. Defaults are tuned by eye on a blockout, not measured against real assets |
| Skinned meshes | Bind pose only — see above |
| Cross-architecture reproducibility | Not claimed |

The API is **not** stable before 0.2. Pin a rev.

Known limitations are listed under "What it deliberately does not do" rather than hidden; `BACKLOG.md` carries the reasoning behind each decision, including the predictions that turned out wrong.

## Contributing

This repository is a **read-only history mirror**, re-derived by `git subtree split` from
`crates/bevy_carnage/` in [`Ladvien/foundation_vs_slop`](https://github.com/Ladvien/foundation_vs_slop),
which is where the crate is developed. Nothing is ever edited here and nothing is pulled back, so a PR
opened against this repo cannot be merged — open issues and PRs on the monorepo instead.

Before opening a PR:

```sh
cargo test                  # unit + tests/leaf.rs + doctests
cargo build --release       # NOT redundant with `cargo test` — see below
cargo build --examples
cargo clippy --all-targets  # 12 warnings pre-exist; add none. See BACKLOG.md's definition of done
```

**`cargo build --release` is load-bearing.** `cargo test` compiles dev-dependencies, and the dev-dependency here is the *full* `bevy` umbrella. Cargo unifies features, so under `cargo test` this crate silently gets every `bevy` feature there is and a missing entry in its own `[dependencies]` cannot fail. That is not hypothetical — `WorldAsset` was reached for while only `bevy_scene` was declared, and every test passed on a crate that did not build.

Two house rules worth knowing before you write code:

- **One path per feature.** No fallbacks, no legacy shims, no stub placeholders. Where the primary path cannot produce a usable result, it fails loudly. `seed_from_path` refusing to bake an unpathed handle *is* the rule.
- **This crate never learns what died.** No health, no factions, no damage, no physics. `tests/leaf.rs` enforces the dependency half of that; the naming half is on you.

Anything that changes emitted geometry regenerates the clips in `docs/` — see [docs/DEMOS.md](docs/DEMOS.md#regenerating-these).

## References

- Müller, Chentanez & Kim, "Real-Time Dynamic Fracture with Volumetric Approximate Convex Decompositions", ACM TOG 32(4), 2013. DOI [10.1145/2461912.2461934](https://doi.org/10.1145/2461912.2461934)
- Sellán, Luong, Mattos Da Silva, Ramakrishnan, Yang & Jacobson, "Breaking Good: Fracture Modes for Realtime Destruction", ACM TOG 41(4), 2022. DOI [10.1145/3549540](https://doi.org/10.1145/3549540)
- Huang & Kanai, "DeepFracture: A Generative Approach for Predicting Brittle Fractures", 2023. arXiv [2310.13344](https://arxiv.org/abs/2310.13344) — Mott's fragment-size distribution
- Schvartzman & Otaduy, "Fracture Animation Based on High-Dimensional Voronoi Diagrams", I3D 2014.
- Sutherland & Hodgman, "Reentrant Polygon Clipping", CACM 17(1), 1974.
- [NVIDIA Blast](https://nvidia-omniverse.github.io/PhysX/blast/) — the chunk-hierarchy / support-graph / damage-shader vocabulary this crate's runtime half borrows.

## License

MIT OR Apache-2.0, at your option.
