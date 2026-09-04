# bevy_carnage — BACKLOG

**Updated:** 2026-09-01
**Companions:** `CLAUDE.md` (rules), `README.md` (what the crate promises),
`docs/research-brief.md` (the open problems), `docs/isomesh-upstream-asks.md` (what we need from the
validator).

**15 tickets archived, 10 landed in phase 3 (AG-015 … AG-024), 7 landed in phase 4 (AG-025 … AG-031), 0 open.**
The architectural change this backlog opened with has landed: the crate no longer cuts the triangle
soup. It cuts a caller-supplied convex proxy and carries the render triangles along as a payload.

**Phase 3 opened the crate to gameplay-driven fracture** — see "Phase 3" below. It answers
`docs/research-brief.md`'s P4, which had been ranked last and never worked.

**Phase 4 renamed the crate and built the layer the new name promises** — wounds, a literature-grounded
spatter model, a pulsatile bleed schedule, pure game-feel curves, GPU blood behind a feature, and a
headless recorder whose printed digest is the determinism check for all of it. See "Phase 4".

**What survives here is the reasoning, not a work queue.** The sections below — the architecture
argument, and the two corrections carried in from research — are kept because they explain *why* the
crate is shaped as it is, and because both corrections turned out to need corrections of their own.
Every ticket, with what it cost and what it falsified, is in `BACKLOG_ARCHIVE.md` (phases 1–2) or in
the phase sections below.

**One piece of history worth keeping at the top, and it has now needed two corrections.** **This
repository is the source of truth**: `foundation_vs_slop` is an ordinary consumer that depends on the
crate as a git dependency pinned to a rev, and there is no copy of it inside that checkout to edit.
Two earlier arrangements are recorded because each cost a session, and neither is live: the crate first
lived only here while the monorepo consumed it by rev; then it was vendored into the monorepo under
this crate's former name as a workspace member, with this repository re-derived by `git subtree split`
(`scripts/mirror_crates.sh`) and never pulled back; then the vendored copy was deleted and the git
dependency restored. An earlier revision of this section asserted the middle arrangement as permanent.
What survives from that note is the *reading* hazard, which is real and cost a session: a
`subtree split` carries only commits, so everything these tickets called "Stage 1" (the audit harness,
the `isomesh` dependency, both research docs, this file) was invisible on the far side for as long as it
lived untracked in a working tree — which is why nine of the original eleven tickets named files that
did not exist there. See `BACKLOG_ARCHIVE.md`, A-1. **Whichever direction is live, read the tree you
are about to change, not a copy of it.**

---

## How this backlog was worked

1. Take the **topmost unblocked, unchecked ticket**. The order encodes dependencies.
2. One ticket = one commit (or a short stack). Commit message starts with the ticket ID.
3. **Check the box in this file as part of that same commit.** This file is the state.
4. If a ticket can't be finished, leave it unchecked, add a `> BLOCKED:` line saying exactly what is in
   the way, and move to the next unblocked ticket. Do not half-finish and check the box.
5. On completion, move the row to `BACKLOG_ARCHIVE.md` with an indented annotation recording any
   amendment, deviation, or **falsified premise**. The annotation is the point; the checkmark is not.
6. A ticket with a **pre-registered prediction** records the outcome against it, including when the
   prediction was right. A prediction nobody wrote down before the run is not evidence.

### Definition of done — applies to every ticket

- `cargo test` green. `cargo clippy --all-targets` introduces no new warnings. **The baseline is 12, not
  the "three in `bake.rs`, `soup.rs` and `mesh.rs`" this line claimed until AG-021 measured it:** 6 ×
  `chunks_exact` with a constant chunk size (`audit.rs:134,632`, `mesh.rs:28,33,838`,
  `examples/fracture_cube.rs:51`), 2 × `too_many_arguments` (`bake.rs:256`, `examples/explode.rs:198`),
  and one each of `type_complexity` (`bake.rs:322`), `empty_line_after_doc_comments`
  (`audit.rs:425`), `items_after_test_module` (`mesh.rs:578`) and `unusual_byte_groupings`
  (`bond.rs:588`). Nothing in `soup.rs` at all. A stale baseline is where a real new warning hides.
  *The bare, `-p`-less form of every command in this section is the
  **mirror's**, where this crate is the whole repository. In a `foundation_vs_slop` checkout it is a
  workspace member, so each one takes `-p bevy_carnage`.*
- **`cargo build --release` passes.** Not redundant with `cargo test`: the dev-dependency pulls the full
  `bevy` umbrella and enables features the trimmed `[dependencies]` set does not. A missing feature is
  only visible in the release build.
- **`cargo build --examples` passes**, and any ticket that changes emitted geometry re-runs
  `cargo run --release --example capture` and regenerates its GIF through `tools/gif.sh`. A change to
  the fracture that nobody looked at is a change nobody checked.
- `tests/leaf.rs` green — the crate stays game-free and its dependency list stays closed. Widening
  `ALLOWED_DEPS` requires the justification in the same commit.
- No `unwrap()`, no `expect` on caller data, no panicking index. Malformed input is `warn!`-skipped.
- Determinism holds: `fracture_output_is_bit_identical_across_runs` must stay green. If a ticket
  legitimately changes emitted geometry, say so in the commit and re-bless deliberately.
- Anything with a sign convention, winding order, or coordinate order says so **in the doc comment**.

**Size key:** `S` ≈ one sitting · `M` ≈ a day · `L` ≈ multi-day, consider splitting.

---

## The architectural change, in one section

Everything in Phase 1 follows from one finding, so it is stated once here rather than repeated per
ticket.

**Production fracture does not cut the mesh.** Müller, Chentanez & Kim (`10.1145/2461912.2461934` — the
NVIDIA lineage behind PhysX Blast, already cited in our README) cut a **volumetric convex
decomposition** and carry the visual triangles as a payload uniquely assigned to a cell. Booleans
become convex ∩ convex, which is trivially robust.

The load-bearing consequence: **plane ∩ convex polyhedron = convex polygon.** Every cap is therefore a
convex cross-section, and the existing centroid fan is *provably correct* for all of them, unchanged.

**This explains our own measurement.** Stage 1 found a cuboid fractures 8/8 clean while the torso+head
fixture scores 7/12 watertight and 2/12 manifold. That is not luck and not two bugs — the cuboid is
**convex**, so every cross-section it can produce is convex, so the fan is valid. The capper was never
broken for convex input and is not fixable for non-convex input. Sellán et al. (`10.1145/3549540`,
*Breaking Good*) reach the same architecture independently, and their transfer step yields *"the
exterior surface of each fragment component is exactly a subset of the input mesh"* — which is the
property that keeps skin UVs.

**The shape:**

- **Tier A — the proxy.** Convex cells, per *connected shell*. Recursively plane-cut **only the cells**.
  A fragment is a set of cells on one side. Colliders, cut caps and fragment identity all come from here.
- **Tier B — the render mesh, never topologically cut.** Assign each input triangle to the fragment
  whose proxy cell contains its centroid. Split only *straddling* triangles against the plane — a
  triangle-plane split is exact and **needs no loop recovery, ever**.
- **Never union the shells.** Cut each independently; associate fragments by proxy-cell provenance, not
  by surface overlap. This is measured, not theoretical: beyond Takayama et al.'s objection to using
  generalized winding number for mesh *repair*, Sacht et al. ran exactly this experiment on
  interpenetrating character limbs and report the legs sticking together and the arms sticking to the
  belly and head. For gibs that is not a quality loss but a **correctness** loss — it destroys the
  ability to separate the head from the torso, which is the entire point of the crate.

**The proxy is supplied by the caller.** This is a deliberate boundary decision, and it makes AG-001
unblocked rather than gated on a convex decomposition nobody has written. It also dodges the
solver-dependency problem: our game can hand in parry's VHACD output (already in the tree via
`avian3d`), while a consumer on a different solver hands in something else. It adds a fourth entry to
**`CLAUDE.md`'s** "Where the boundary falls" — *not the README's, which has no such section; its
analogue is "What it deliberately does not do".*

> **Do NOT use Convex Primitive Decomposition** for the proxy. CPD (`10.48550/arXiv.2602.07369`) *wraps
> the outside* in overlapping primitives — it is a collision proxy, not a filling. Two disqualifiers:
> there is **no interior to cut**, and its enclosure guarantee makes the wrapper strictly *larger* than
> the shape, so every fragment comes out fat. "Guarantees enclosure" is a virtue for *"did I bump into
> this?"* and the wrong sign for *"cut this."* Use **V-HACD or CoACD** — genuinely volumetric.

---

## Two corrections carried in from research — do not re-derive these

Both were reported to us as fact and both are **false**. They failed the same way: **reading intent as
implementation**. Recorded here because the cost of re-checking is small and the cost of building on
them is not.

1. **"`isomesh` is not in `bevy_carnage`'s `Cargo.toml`; it appears in test usage only."** False as a
   statement about the crate, and **the reading that produced it was fair**. It is a real
   `[dependencies]` entry pinned to `rev = "4369e3c"`, with `ALLOWED_DEPS` widened in the same commit —
   but at the time that entry existed only in the monorepo's *working tree*. It was in no commit, in
   either repository, so an agent reading published history could not have found it. **This is now
   fixed at the root, though not the way this line used to claim:** the monorepo is the source of truth
   and everything is committed *there*, which the mirror then carries. AG-021 corrected the earlier
   wording, which had it backwards. See `BACKLOG_ARCHIVE.md` A-1.
2. **"`signed_distance_from_mesh_winding → SampledField::new → ManifoldDualContouring` works end to end
   today, roughly three lines."** False **at the rev we pin**, which is the only rev that can affect a
   build: none of those symbols exist at `4369e3c`.<br><br>
   **But this correction has itself gone stale, and re-checking it is why AG-013 exists.** isomesh's
   `HEAD` is no longer `4369e3c` — it is **229 commits ahead**, and `signed_distance_from_mesh_winding`,
   `SampledField` and `MeshField` all exist there now. So the claim was wrong about *when*, not about
   *what*. Two things stay true regardless: convex decomposition is still absent, and the third link in
   that chain is refuted by upstream's own source — Manifold Dual Contouring reads an eager N³ grid like
   every other extractor, so "queries where it needs to" was never right about it. See AG-010.

**The architecture argument in the research is unaffected by either.** Its claims about *existing
capability* should be independently verified before anything depends on them.

---

## Phase 3 — gameplay-driven fracture

**The problem, stated as it was reported:** the demo "looks like we took a guy, froze him, and then
shattered him." That is structural, not cosmetic, and it has three separate causes.

1. **One bake = one outcome.** Every death produced the same fragment set, all at once, all over the
   body, with no relation to what killed it. `examples/explode.rs` makes it literal: intact for
   2.5 s, then the whole subject is despawned and replaced.
2. **No fragment had an identity or a neighbour.** `Fragment` carried a centre, an extent and a
   cell. No id, no parent, no adjacency — so breaking off *one* piece was not expressible.
3. **The cut geometry produces uniform convex shards.** Always-split-the-largest-by-volume drives
   fragment volumes toward uniformity, and a plane through the centroid centres every piece.

**The answer the literature gives is the same in three places, and it is not "bake harder".** Müller,
Chentanez & Kim reject static pre-fracture precisely because "the number of hierarchical fracture
levels is fixed" and "there is no way to align fracture patterns with the impact location"; their fix
is a hierarchy plus runtime, impact-aligned *selection*, with island detection deciding what actually
separated. PhysX Blast generalises that to a chunk hierarchy, a support graph of bonds, and damage
programs mapping a geometric query onto which bonds break — its shader set (`ImpactSpread`,
`CapsuleFalloff`, `TriangleIntersection` "useful for sweeping-blade effects", `RadialFalloff`,
`Shear`) is one-for-one the behaviours asked for. Unreal's Chaos lands on a connection graph with
per-level damage thresholds independently.

**On the look specifically:** Sellán et al. (`10.1145/3549540`, already cited here) state that
Voronoi and plane-cut prefracture "results in recognizable, unrealistic pieces" because it is blind
to where a shape is weak — for a body, the thin cross-sections. DeepFracture
(`10.48550/arXiv.2310.13344`) gives the quantitative half: real fragment volumes follow Mott's
distribution, `P(V) = e^(−∛(ζV))` with `ζ = 6/V̄`. Many small, few large. Uniform sizes are the
signature of a geometric cutter.

**Where the boundary falls, decided before any code:** the crate gets *geometry only* — a hierarchy,
an adjacency graph, and pure functions from a geometric region to a fragment set. Health, damage
numbers, weapon identity, impulses and pooling stay with the caller. `CLAUDE.md`'s boundary list is
unchanged and `tests/leaf.rs` stays green; no new type is named for a weapon or a body part.

| | ticket | size |
|---|---|---|
| [x] | **AG-015 — the fracture hierarchy: one bake, every granularity.** Record the forest the cut loop already walks; keep parents instead of overwriting them. `FragmentTree`/`TreeNode`/`FragmentId`, frontier queries on `Fracture` and `FractureCache`, `FractureSettings::max_depth`. | M |
| [x] | **AG-016 — the bond graph: which fragments actually touch.** Parent–child bonds from the tree are free but insufficient — two leaves of a common ancestor need not touch. Müller's coplanar-face match (sort faces by \|d\|, match equal-\|d\| opposite normals, planar convex∩convex overlap for the area) is exact for convex cells. Plus stateless `islands(graph, broken)`. | M |
| [x] | **AG-017 — severance queries.** Five pure region→fragment-set functions: `spread` (nearest fragment then breadth-first along bonds with falloff — a bullet takes one chunk), `capsule`, `swept_triangle`, `radial`, `shear`. Falloff follows Blast: full inside `min_r`, linear to zero at `max_r`. | M |
| [x] | **AG-018 — the cheap look fixes, and delete `impact_dir`.** Offset the cut plane along its normal by a hashed fraction instead of always through the centroid; weight piece selection by `volume * (0.5 + hash)` on a stable node id so sizes spread Mott-ward. **This is the stage that moves emitted geometry** — regenerate `docs/fracture-tier-ab.gif`. `impact_dir` biased only the first two cut *normals*, never the plane position, was hardcoded `None` by the bake and passed `None` by every caller; the runtime queries supersede it, so it goes. | S |

### AG-016, as landed

**Pre-registered prediction: the coplanar match recovers every cut-adjacency exactly and finds
nothing between the caller's root cells, so the torso and the head come back as two islands.**

**Half confirmed, half falsified — and the falsified half was ours, not the algorithm's.** Cut
adjacency is recovered exactly, as predicted: all seven bond tests passed on the first run, including
connectivity and the localised break. But the two-shell fixture comes back as **one** island, not
two. The head cell's underside sits at `y = 0.5` and the torso cell's top face sits at `y = 0.5` —
they are exactly coplanar, so there is a real shared face and the match correctly finds it. The
example had been written asserting the opposite before it was run.

That is worth keeping because the prediction was not idle: it was reasoning about interpenetrating
shells, which is a real case (`BACKLOG.md`'s Sacht et al. note), and this fixture simply is not one.
The two boxes *abut*. The refusal being tested is still pinned, by
`cells_that_do_not_share_a_face_are_not_bonded`, on cells that genuinely interpenetrate.

**Measured on the standard fixture:** 36 bonds over 12 finest fragments; intact, one island;
severing one fragment's 3 bonds leaves islands of sizes `[11, 1]` — the localised break-off working
on real baked geometry.

**No proximity fallback was added, deliberately.** Cells that touch without agreeing on a face get no
bond, which is the normal case between V-HACD or CoACD root cells. Approximating there would weld a
head to a torso with a tolerance no caller could tune, so it is refused and documented instead.

| [x] | **AG-019 — a face too small to draw must not be shipped as a face.** Found while working AG-018, and it is a *pre-existing* defect: sweeping seeds showed **1 fragment in 320** coming back with `boundary_edges != 0`. Not caused by the jitter — jitter roughly doubled it, which is what made it visible. | S |

### AG-018 and AG-019, as landed

**AG-018's pre-registered prediction: the look dials move emitted geometry, so the GIF is
regenerated and some pinned counts may need re-blessing.**

**Falsified in the most useful direction — the geometry change did not need re-blessing, it needed a
bug fixed.** Turning the jitter on turned `every_proxy_fragment_of_a_closed_solid_is_closed` red.
The reflex reading is "geometry moved, re-bless it". Probing instead found the defect was **already
there at `plane_jitter = 0`**: one fragment in 320, seed-dependent, and the pinned seeds simply
missed it. The crate's central watertightness promise was seed-lucky, not true.

**The mechanism, measured rather than guessed.** Repeated cutting leaves near-degenerate faces on a
cell — a plane passing close to an existing vertex produces vertices `1.3e-4` apart, just past the
`1e-4` weld. That face is real, but every triangle of its fan falls under the emitter's zero-area
filter, so `append_cut_faces` and `soup_to_mesh` both drop it — and dropping a face from a closed
cell opens it. The dumped fragment showed it exactly: face `[6,7,8]` over three near-collinear
points, area `≈4e-7`, `boundary_edges: 3`, `χ = 1`.

**One hypothesis was wrong and is recorded because it cost time.** `convex_ring` dedupes by snapping
to a `WELD` lattice, which is a known-wrong idiom for coincidence — two points a nanometre apart on
opposite sides of a grid line both survive. Replacing it with a distance test changed the defect
count by **zero**, so it was reverted rather than shipped: a change that moves geometry and fixes
nothing measured is exactly what this repo's norms warn against. (`CellBuilder::weld` still uses a
bare lattice, while `mesh.rs`'s `AttributeWeld` uses a 27-cell probe for precisely this reason. Left
alone deliberately — 0 defects in 9000 fragments after the real fix, so there is nothing to justify
touching it.)

**The fix:** `CellBuilder::build` now collapses any face whose Newell area falls under the shared
`MIN_CROSS2`, merging its vertices transitively via union-find. A face that cannot be drawn is a
vertex, not a face; merging closes the gap for free, because the sliver's two long edges become the
same edge. `MIN_CROSS2` is now one constant shared by the three sites that apply it, so the cell's
"will not build" and the emitter's "will not draw" cannot drift apart again.

**Measured after: 0 defective fragments in 9000**, across 300 seeds × three jitter levels including
`plane_jitter = 0.6`. The regression test is a **seed sweep**, not a pinned seed, because a pinned
seed is what missed it.

**AG-018's own measurement.** Largest/smallest fragment volume, median over 200 seeds:

| `plane_jitter` | `size_spread` | ratio |
|---|---|---|
| 0.0 | 0.0 | ~2.5 |
| 0.35 | 0.5 (shipped defaults) | ~4.1 |
| 0.6 | 0.8 | ~10.6 |

The test pins the *ordering* rather than those numbers, since pinning the ratios would re-bless on
any change to the cut sequence — which is not the claim being made.

**Two things grew that the ticket did not name.** `fracture_mesh` would have reached eight positional
arguments, so the geometry dials became `CutSettings` — which also fixes the readability of a call
that already read `(&parts, &proxy, 12, 0.15, 64, 0xC0FFEE)`. And `fracture_cube`'s size bar was
keyed on max half-extent, which reads every slab as large and hides the size distribution entirely —
the one thing these dials exist to change. It reads volume now.

**`impact_dir` is gone**, as planned — though it went in AG-015 rather than here, because that ticket
was already rewriting every call site.

| [x] | **AG-020 — `examples/sever.rs`, and `BondGraph::of`.** The demo the phase exists to produce: the subject stays standing and you take pieces off it. Building it found that the bond graph was leaf-only, so any coarser frontier read as fully disconnected. | M |

### AG-020, as landed

**Two defects, both found by building the thing rather than by reasoning about it.**

**One: adjacency is per frontier, and the crate only shipped the leaf graph.** `Fracture::bonds`
covers the finest frontier; a fragment off a graph's frontier has no incident bonds at all. So
standing the subject at `frontier_of(8)` and running `islands` against the leaf graph reported every
piece as its own island — the granularity dial and localised damage did not compose, and the subject
would have fallen apart on the first blow. `BondGraph::build` is now public as `BondGraph::of`, and a
coarse frontier is not a special case: two frontier cells that touch were separated by a cut at their
common ancestor, so the faces they present each other are exactly coplanar however deep either sits.
`every_frontier_has_its_own_connected_graph` pins both halves — that every frontier is connected, and
that reading a coarse one against the leaf graph *does* look disconnected, so the trap stays visible.

**Two: the example's own subject was unbondable.** `explode.rs` puts the head at `y = 0.74`,
overlapping the torso by a centimetre. That is harmless when the whole subject bursts at once, and
wrong here — cells that overlap rather than share a plane get no bond, so the head was its own island
from the start and would drop off at the first blow anywhere. Caught by
`a_hit_takes_part_of_the_subject_and_leaves_the_rest_standing`, which is a headless replay of what
the example does on screen. `sever.rs` seats the head at `0.75` so the two cells meet exactly.

**One assertion was written too strong and corrected rather than propped up.** The end-to-end test
first asserted that a second blow elsewhere must break the subject further. It need not: a fragment
can lose bonds and still be held on by the ones the region missed, which is the behaviour that makes
repeated damage read as wearing a thing down instead of as a switch. The test now pins what is
actually true — a blow never re-joins anything, and a sequence of blows does progress.

> **Weak-axis bias is deliberately deferred.** Choosing each cut normal to minimise cross-section
> area — a cheap stand-in for Sellán's fracture modes, and what would make a character come apart at
> neck and wrist rather than into shards — is the obvious next step after AG-018 and was scoped out
> of this phase on purpose.

### AG-015, as landed

**Pre-registered prediction: keeping the parents changes no emitted geometry, and every currently
green test stays green without re-blessing.**

**Confirmed.** The cut sequence is preserved by keeping selection and the seed mix on *frontier
slots* rather than node ids — a cut still reuses its own slot for the `above` half and pushes a new
slot for `below`, exactly as the flat loop did, so the fact that ids no longer coincide with frontier
positions is invisible to the plane sequence. All 36 unit tests, `tests/leaf.rs` and the doctests
passed unchanged, including `fracture_output_is_bit_identical_across_runs`, `hash_f32_is_frozen`,
and both `== 12` fragment-count assertions.

**One thing was falsified, and it was a number we had been quoting.** `bake.rs` recorded the bake at
**0.33 ms**. Re-measured on this machine, the *pre-change* code takes **~1.4 ms** and the hierarchy
takes **~2.2 ms** — the ratio tracks node count (23 built instead of 12), which is the honest cost of
keeping every piece the loop split. Both are far under AG-011's 50 ms threshold so the
main-thread conclusion is unchanged, but the 0.33 ms figure was being repeated as though it had been
re-checked, and it had not.

**Two shapes had to change to keep the array index-parallel with the tree.** `geometry_from_piece`
was fallible and its caller dropped the `None`s; dropping an entry would slide every id after it onto
the wrong node, so it is now total — a piece that draws nothing keeps its slot with both meshes
`None`, bounded by its cell, which is still a perfectly good convex collider. And `fracture_mesh`
now returns `Fracture { fragments, tree }` rather than a flat `Vec`, because the flat `Vec` no longer
has a single meaning: `into_leaves()` is the old one.

| [x] | **AG-021 — move the `isomesh` pin to `aa82b0b`, re-record the GIFs, resync the mirror.** 291 commits, `0.0.7` → `0.0.10` plus the unreleased `mass` work. Both lockfiles, the pin-rationale comment, the `fracture_cube` transcript, a re-triage of `docs/isomesh-upstream-asks.md` against the four modules that landed, all three demo GIFs re-recorded on Linux/Vulkan, `tools/gif.sh` freed of its macOS-only font defaults, and two false claims this file, `CLAUDE.md` and `README.md` all carried — which repository owns the crate, and how many clippy warnings pre-exist. | M |

### AG-021, as landed

**Pre-registered prediction: zero numbers move and no test needs re-blessing** — because `weld.rs` and
`mesh.rs` are byte-identical across the two revs, every counter-producing pass in `validate.rs` is
line-for-line identical, `collider.rs`'s three predicates and `from_report` are byte-identical, and the
one behaviour change `0.0.10` advertises loudly (vertex placement on the default extraction path, 135 of
216 golden hashes rebaselined) sits on a code path this crate never calls — it has no extractor. Contrast
AG-013, which cost one re-blessed number.

**Confirmed, digit for digit.** `fracture_cube` at `aa82b0b` reproduces the committed transcript
exactly: `12 of 12` watertight, manifold, χ = 2 and collider-ready; `volume enclosed 0.2493`; `total
volume 0.2493` at all five granularities; the whole `soften` table unmoved; `adjacency — 31 bonds over
12 finest fragments`; `2 island(s) of sizes [11, 1]`; `bit-identical: true`. Not one line of
`docs/DEMOS.md`'s fenced block needed editing. It gained only a provenance stamp naming the rev it was
captured against — which is what makes the claim re-checkable next time rather than argued.

**Second pre-registered prediction: the re-recorded GIFs differ from the committed ones in pixels but
not in verdict. Confirmed, and measured.** Frame for frame against the committed
`fracture-tier-ab.gif`, 0.8% of pixels differ and the tint census does not: zero amber and zero magenta
pixels outside the burnt-in legend, in the old clip and the new one alike, so every fragment is still
green. The pixel difference has two host causes and no code cause — the committed clips were encoded on
macOS against Metal and these on Linux against Vulkan, where anti-aliasing and gamma differ; and the
captions are set in Liberation Sans, metrically compatible with Arial but not the same glyph outlines,
so the text fills the same box with different edges. Read that diff as a change of machine, not a change
of geometry.

**`docs/fracture-baseline.gif` was deliberately not re-recorded.** It is the *before* picture from the
soup cutter that predated the Tier A/B split, and that code no longer exists, so there is nothing left
to record it from. Regenerating it would have replaced a historical contrast with a duplicate of the
current clip.

**`tools/gif.sh` had a second path, and it was deleted rather than doubled.** `FONT` and `BOLD`
defaulted to `/System/Library/Fonts/Supplemental/Arial.ttf` and its bold twin — paths that exist on one
of the machines this repo is built on, with the failure landing in `magick` *after* ffmpeg had spent the
entire two-pass encode. A Linux default would have been two paths to one output; instead both variables
are now `${…:?}` with an explicit existence check placed before ffmpeg runs, and `docs/DEMOS.md` carries
the paths for the host it was last run on.

**Nothing new was adopted from upstream, and every refusal is written down** rather than left as an
omission: `validate::sealing` (there is no field to pass it — this crate plane-cuts a caller-supplied
proxy — and it does not answer ask 5 either, since it never looks at triangle pairs, so **ask 5 stays
open**), `validate::mesh_hash` (`f64`-only against an `f32` buffer, and
`fracture_output_is_bit_identical_across_runs` already compares the fragments themselves, which is
strictly stronger than comparing two hashes of them), `connectivity` (incremental components over a
voxel lattice, not a bond graph of convex cells — same word, different problem), and
`mass::mass_properties` plus `MeshReport`'s `mean_ratio` and `irregular_vertices`, all three applicable
and all three public API additions that deserve their own ticket. `mass_properties` in particular is
**not** a replacement for `SolidAudit::signed_volume`: it returns `Err(MassPropertiesUndefined)` on a
non-positive volume, which is exactly the inconsistently-oriented fragment that field exists to report.
`docs/isomesh-upstream-asks.md` carries the full triage and a third rev column.

**Two stale claims were corrected while the files were open, both of which actively misdirected.**

*Which repository owns the crate.* This file, `CLAUDE.md` and `README.md` each asserted that
`Ladvien/bevy_carnage` is the source of truth and that a `crates/bevy_carnage/` in a monorepo checkout
"is a corpse". The tooling says otherwise: `scripts/mirror_crates.sh` states "the monorepo is the source
of truth; nothing is ever edited on the far side and nothing is ever pulled back", `bevy_carnage` is in
its `CRATES` and `PUBLIC_CRATES` lists, the root `Cargo.toml` lists `crates/bevy_carnage` as a workspace
member and depends on it by `path`, and there is no standalone checkout of the mirror on this machine.
The corpse was the live copy. All three notes now say: the monorepo is the source of truth, the mirror is
a `git subtree split` of it, and the bare `-p`-less build commands are the *mirror's* form — in a
monorepo checkout every one of them takes `-p bevy_carnage`. The *reading* hazard the old note was
reaching for survives, because it is real: a `subtree split` carries only commits, so anything
uncommitted in the monorepo working tree cannot appear on the mirror at all.

*How many clippy warnings pre-exist.* The definition of done said "three, in `bake.rs`, `soup.rs` and
`mesh.rs`". Measured: **12**, in six files, and **none in `soup.rs`** — six of `chunks_exact` with a
constant chunk size, two of `too_many_arguments`, and one each of `type_complexity`,
`empty_line_after_doc_comments`, `items_after_test_module` and `unusual_byte_groupings`. The list is now
enumerated by file and line, because "no new warnings" is unfalsifiable against a baseline that is
wrong: a real regression hides in the gap. AG-021 itself adds none — it changes no `.rs` file, and every
warning sits in source it never touched.

| [x] | **AG-022 — bullet holes that go through: a `Bore` subtracted from the proxy.** `Bore { from, to, radius, sides, jaggedness, flare }` carried in `CutSettings::bores` and in a `FractureBores` component; `bore::apply` subtracts each channel from the proxy cells before assignment and before the cut loop, and carves it out of the render skin per closed shell. `FaceKind` replaces `ProxyCell`'s `face_cut: Vec<bool>` so a channel wall is emitted flat. Plus `examples/bullet_holes.rs`, `examples/capture_holes.rs`, `docs/holes.gif`, a bore census in `fracture_cube`'s transcript, and ten tests. | M |

### AG-022, as landed

**Pre-registered prediction, written before the run: no existing number moves** — every bore is
opt-in and empty by default, so `fracture_cube`'s committed transcript, `mesh.rs`'s volume
conservation and exact-area accounting, and all three existing GIFs are untouched — **and a bored
subject stays one island**, because the shards' radial faces are bit-identical coplanar pairs.

**Both confirmed.** `fracture_cube` reproduces the committed transcript digit for digit: `12 of 12`
watertight, manifold, χ = 2 and collider-ready; `volume enclosed 0.2493`; `total volume 0.2493` at all
five granularities; the whole `soften` table unmoved; `adjacency — 31 bonds over 12 finest fragments`;
`2 island(s) of sizes [11, 1]`; `bit-identical: true`. All 71 tests green with none re-blessed, and
`git status` shows `docs/holes.gif` added with no existing GIF modified. `a_bored_cell_is_still_one_island`
finds exactly one island over the eight shards of a bored cell, with no fracture cut holding them
together.

**Three premises the plan stated were falsified by building it, and the second one was the expensive
one.**

*One: the barrel planes at distance `radius` from the axis describe the wrong polygon.* The plan's
design put each of the `sides` planes at `radius`, and asserted the channel was therefore the
*inscribed* polygon. It is the **circumscribed** one: a plane at distance `radius` is the apothem, so
the polygon's corners reach `radius / cos(π/n)` and the hole is 8.2% wider than asked for at 8 sides.
Measured on the 1×2×1 fixture, an 8-gon bore of radius 0.1 removed `0.066274` — exactly
`8 · 0.1² · tan(22.5°) · 2`, the circumscribed area — where the inscribed channel is `0.056569`; at 24
sides `0.063193` against `0.062117`, which is how it first showed up, as a 1.7% miss that looked like
float noise until the 8-sided case made it 17%. The fix is one `cos(π/n)` factor, chosen over
re-documenting `radius` as the apothem because two public doc sentences promise `radius` bounds the
entry hole, and `jaggedness`'s inward-only bite is meaningless against a bound that is already
exceeded.

*Two: the skin cannot be carved before the shells are classified.* The plan had `bore::apply` take the
whole render soup and hand back a carved one, with `fracture` classifying shells afterwards as it
always had. A carved skin has boundary edges at every hole rim, so `Shell::open` reads a bored solid
as a **sheet** — AG-003's protection for capes and hair cards — and a sheet is carried whole to the one
cell containing its centroid. For a bored box that centroid is *inside the channel*, so it belongs to
no cell: measured, all 10.0 of the fixture's skin area came back homeless and every fragment drew
nothing. The fix keeps one path rather than adding an exception: `bore::apply` returns the prisms that
landed, `fracture` decides open-versus-closed on the artist's own geometry, and `bore::carve` runs per
**closed** shell. A sheet is carried unbored, which is the same answer from the other direction — a
bore is a subtraction from the proxy, and a sheet is not in the proxy. `carve` with no prisms is the
identity, so an unbored bake stays byte-identical.

*Three: softening is not a minor perturbation of skin area once there are shards.* The plan's skin
test allowed the measured loss to sit within 1.5–2.5× the channel cross-section, "since the softening
and the rim slivers move it". At the shipped `soften = 0.5` the 24-shard bored fixture's skin came
back **3.1 against the unbored 10.0** — because the relaxation runs per fragment and 24 shards shrink
24 times. The test measures at `soften = 0.0` and says so; softening has its own tests. The same
measurement is why `capture_holes` and `bullet_holes` render at `soften = 0.0`: independent relaxation
pulls two shards' shared boundary apart, and a hairline along every wedge boundary radiating from a
hole is, in a clip about holes, read as a crack.

**One assumption was carried through unmeasured, and that is worth flagging.** The plan predicted that
`cap_relief` on a bore wall would fold the wall through the channel axis — a 0.04 bore through the
0.28-deep torso gives a wall face of radius ≈ 0.176, which `cap_relief = 0.30` displaces by up to
0.053, larger than the hole — and specified `FaceKind::Bore` to emit walls flat. That is what shipped,
and the arithmetic is in `FaceKind`'s doc comment, but **the defect was never rendered**: no clip was
taken with `FaceKind::Bore` treated as `Cut`. The dial is off by construction, so nothing is wrong;
what is missing is the picture proving it had to be.

**A tenth test was added beyond the nine the plan listed.** `flare` is a public dial whose whole claim
is "the exit radius is `radius * (1 + flare)`", and nothing tested it — the GIF's third visual claim
rested on eyeballing an obliquely-viewed face. `flare_widens_the_exit_and_leaves_the_entry_where_it_was`
solves for where each barrel plane crosses the ray `axis(h) + dir·t`, which is the channel's radius
along that facet's own outward direction rather than the plane's perpendicular distance, and pins entry
= apothem and exit = 1.6 × apothem to within `1e-6`.

**Two shapes grew, both as forcing functions rather than conveniences.** `FractureSettings::cut_for`
takes `bores` as a parameter instead of defaulting it, so the ECS path cannot silently drop a subject's
channels; and `examples/common/body.rs`'s `Baked::bake` takes the bore list, so the windowed demo and
the recorder cannot diverge about what was fired. `CutSettings::bores` carries
`#[cfg_attr(feature = "serde", serde(default))]` — `CutSettings` has no `deny_unknown_fields`, so a
missing field is the only compatibility risk, and the default is the empty list that means "as before".

**Measured, from `capture_holes`'s own log:** each shot subtracts from the original six cells and adds
exactly seven — 13, 20, 27, 34, 41 cells for one through five holes — one cell becoming its eight
shards every time, with volume removed 0.00079 → 0.00367. No bore reported reaching no cell, and no
triangle came back homeless.

| [x] | **AG-023 — the plug comes out: a bullet hole and its gore are the same subtraction.** `bore::subtract` kept the plug instead of dropping it; it comes back as `Ejecta` (pure path) and `EjectaChunk` (ECS path) — a convex cell, the channel wall as interior material, a patch of skin at each end, plus the exit point and the channel axis. `audit_cell` extracted so a plug can be audited without a `FragmentGeometry` it has no id for. The demo throws each plug down the channel, lands it, and replaces it with a flat pool. Six tests. | M |

### AG-023, as landed

**Two pre-registered predictions, written before the run.** *One: keeping the plug moves no existing
number and adds no new failure mode*, because it is material the cut already computed in order to
remove it — nothing new is generated, something is simply no longer thrown away. *Two: the bore then
conserves volume exactly*, `shards + plug == the cell`, which it could not previously state at all.

**Both confirmed.** All 70 pre-existing tests stayed green with none re-blessed, `fracture_cube`
reproduces the committed transcript digit for digit, and clippy stayed at exactly the 12-warning
baseline. The conservation law is now printed in the transcript rather than argued:
`shards + plugs 0.2493 = the subject 0.2493`. Before this ticket the bore was the one operation in the
crate that destroyed solid.

**The plug must not be a `ProxyCell` in the proxy, and that is the whole reason `Ejecta` is a separate
type.** A plug's barrel faces are the *same* rings `clip` handed the shards, reversed — bit-identically
coplanar, which is exactly what `BondGraph::of` matches on. Returned as a cell it would be bonded to
every shard around it by a match working perfectly, the plug would join the body island, and the hole
would be *filled by a piece welded across it*. `a_plug_is_absent_from_the_tree_and_from_the_bonds`
pins all three halves: not in `fragments`, not a leaf, and the shards still one island without it.

**Six premises were wrong, and five of them were only findable by looking at the picture.**

*One: `audit_proxy` was accidentally coupled to `FragmentGeometry`.* It reads nothing but `.cell`, so
auditing a plug meant either inventing a `FragmentId` it does not have or copying the function. Split
into `audit_cell`, which is what it always was.

*Two: the shared integrator treats every chunk as a pebble, and a plug is not one.* Restitution 0.3
with a drag that only bites during contact meant a landed plug still carried **0.99 of its speed
sixteen frames after touchdown**, so it skidded and the pool formed the better part of a unit from
where it came down. A wet lump neither bounces nor skids: it is stopped dead on first contact, in the
gore system rather than in `integrate`, so there is still one integrator and one chunk type.

*Three: `GORE_SPEED` was wrong by a factor of four, and the failure was invisible in the logs.* Every
plug is small enough that `heft` saturates at its 2.2 clamp, so 6.5 meant an effective 14.3 and a
flight of roughly **eight units** — off the far edge of the 14×14 floor. The pools formed correctly
every time and were simply never on camera, which is the kind of bug a green build cannot report. At
1.6 the effective 3.5 lands about 1.2 units out.

*Four: a rod freezes on its end.* A plug is as long as the subject is deep and as wide as the calibre,
so stopping it mid-tumble left it standing upright like a bollard — the single most artificial thing in
the clip. Its own long axis is rotated onto the horizontal on landing.

*Five: a third of a turn puts the camera on the same side as the gore.* The plugs fly out the exit
side, so the AG-022 orbit ended with the pools a few tenths of a unit in front of the lens. The orbit
now backs off to 1.45x and rises while aiming lower, so the exit wounds and the stains they left are
in the same shot.

*Six: softening **grows** a plug's end discs, where it shrinks a fragment's skin.* Measured 0.244
against the carve's own 0.089 at the shipped `soften = 0.5` — a disc welded to a barrel ring bulges
outward when it relaxes, the opposite direction to the effect AG-022 recorded for shards. The test
measures at 0.0 and says why.

**What is deliberately not here.** The plug is one chunk, not a spray of several: that is the honest
geometry of the subtraction, and splitting it would mean choosing a fracture for it that the shot did
not describe. The pools are flat discs of one shared unit-radius circle asset scaled per pool, not
projected decals — this crate has no floor and no decal pipeline, and the example should not grow one
to prove a point about geometry. And nothing in the crate moves a plug: `direction` and `exit` are
facts about the `Bore`, and turning them into a velocity is the caller's, exactly as it is for a
fragment.

| [x] | **AG-024 — the plug breaks up, because one convex prism looks like one convex prism.** Reported as "the plugs look like someone used an apple coring cutter", and that is literally what the geometry was. `Bore::shatter` (1..=12, clamped) runs the plug through `soup::choose_plane` — extracted verbatim from `fracture`'s loop so there is still exactly one cut policy — and `CutSettings::ejecta_soften` rounds the debris without touching the body. Per-shot `shatter` in the demo, a `K` key in the windowed one, two tests. | M |

### AG-024, as landed

**Pre-registered prediction: extracting `choose_plane` out of the cut loop moves nothing**, because it
is a verbatim lift of a pure function of `(cell, mixed seed, weak_axis, plane_jitter)` and the seed
mixing — which folds in the live frontier size and is load-bearing for every bake this crate has ever
produced — stays with the caller.

**Confirmed, and checked before anything was built on it.** 76 tests green, `31 bonds over 12 finest
fragments`, `volume enclosed 0.2493`, `bit-identical: true`, and the bore census unchanged. The
extraction was verified on its own commit-worth of work precisely because a silent drift there would
have re-partitioned every asset and been attributed to the shatter.

**Sharing the policy rather than copying it is the whole point.** The *look* of every broken thing in
this crate comes from those twenty lines. Two copies would drift the first time either was tuned, and
gore that came apart along a different rule than the body it came out of would be a second answer to
one question.

**Three things were tried and rejected on evidence, not taste.**

*Calling `soup::fracture` recursively on the plug.* The obvious reuse, and wrong for AG-003's exact
reason: a plug's skin is the two **disconnected** patches where the channel crossed the surface, so
`Shell::open` reads each as a *sheet* — the cape protection — and carries it whole to one piece instead
of clipping it. What the two loops can share is `choose_plane`; what they cannot share is shell
classification, which a plug needs none of.

*Cutting the plug along a random direction instead of its weak axis.* The theory was sound — a plug is
blown apart by an impact rather than failing along its own weak cross-section, so `weak_axis` is
importing the wrong physical story. Recorded both: random planes through a thin rod produce flat flakes
with visibly less mass, while the weak-axis cuts give chunkier segments that read better. Reverted to
the shared dial. The theory was right and the picture disagreed.

*Raising `soften` so the gore rounds.* This is where the ticket found a defect worse than the one AG-022
recorded. At `soften = 0.40` the demo body does not merely show hairlines — the eight shards of every
hole **separate outright**, red gaps radiate from each entry wound, and the subject reads as
disassembled rather than shot. Cause: `soften` relaxes each drawn piece independently and never pins
the boundary it shares with its neighbour. Compact fracture fragments barely show it; a bore's shards
are long thin wedges meeting over large faces through the middle of the cell, so the shrink is obvious.
Fixing `soften` to pin shared boundaries would need per-vertex neighbour knowledge and would re-bless
every bake in the repo — out of scope, recorded here. `ejecta_soften` is the narrow fix: **ejecta share
a boundary with nothing**, so they can be rounded freely, and that is most of the difference between
sharp coins and lumps of meat. Twelve dials now, not eleven.

**One test earned its keep immediately.**
`a_plug_carries_the_wall_and_the_skin_the_channel_tore_out` went red at 0.256 the moment
`ejecta_soften` gained its 0.55 default — which is exactly the evidence that the new dial reaches
ejecta rather than being silently ignored. It now pins both softening dials and says why.

**Measured, from the recorder's own log.** The five shots ask for 3, 4, 5, 6 and 8 pieces and the
cumulative ejecta count runs 3, 7, 12, 18, 26 — every plug divided into exactly what was asked. A
pleasant second-order effect: many small plugs leave many small pools, which overlap into irregular
spatter rather than the single tidy disc one plug left.

**A defect this ticket nearly shipped, caught by writing the claim down before trusting it.**
`ejecta_soften` is the twelfth field on `FractureSettings`, which is `serde(deny_unknown_fields)` with
**no struct-level default** — so every field is required on deserialize, and adding one refuses any
authored file that enumerated the others. At *load* time, which no build catches. The consuming game's
`config.ron` lists all eleven previous dials exhaustively, so the bump would have taken its release
build green and then refused the config at startup. Found while writing the pin-bump comment, which
asserted the opposite; checking the file was what disproved it.

The fix is the pairing `CutSettings::bores` already used: `#[serde(default = "default_ejecta_soften")]`
plus the existing `deny_unknown_fields`. **Missing takes the shipped value; unknown is still an
error** — a default that also swallowed typos would be a fallback, and this is not that. Three tests
now hold it: the serde default and the `Default` impl must agree (or a config that *omits* the dial
renders differently from one that never had it, which nobody would look for), the shipped settings
must pass `validate`, and — the one that would actually have caught this — the exact eleven-field
block from the game's own `config.ron`, copied rather than paraphrased, must still deserialize. That
last test needs a format, so `ron` joins `[dev-dependencies]`; it is already in the lockfile via
`bevy`, and `tests/leaf.rs` scopes its closed-dependency ratchet to `[dependencies]` for this case.

**Any dial added to `FractureSettings` from now on needs the same treatment**, and the note is on the
field rather than only here.

### AG-024 — and then the windowed example was run for the first time

`examples/bullet_holes.rs` shipped in AG-022, AG-023 and AG-024 **without ever being executed.** This
host runs no desktop (`seat0`'s active session is the SDDM greeter), so three tickets' worth of
verification stopped at "it compiles, and every function it calls is exercised by `capture_holes`".
With `Xvfb` installed it ran under `:99` against the real GPU — NVIDIA/Vulkan, window created,
62 fragments baked — and produced **four defects in the first two screenshots**, none of which a test
or a headless recorder could have found:

1. **`·` rendered as a missing-glyph box** in the status line. Bevy's default font atlas has no
   U+00B7. The legend above it survived only because it was already ASCII.
2. **`—` did the same**, in every message the dial keys produce — found on the second screenshot,
   after fixing the first. `sever.rs` uses plain hyphens throughout, so the original author already
   knew; the note now lives on `HudStatus` so the next person does too.
3. **The aim marker was invisible.** `Aim` is a point on the bore's *axis*, so drawn at the aim point
   the marker sits inside the torso. The first fix pushed it 0.42 forward, which was visible and wrong
   in a subtler way — the camera is off-axis, so it parallaxed away from the hole it predicts and
   stopped being an aiming aid. 0.20 clears the skin by more than the marker's radius while reading as
   the same place as the wound.
4. **The subject's feet ran off the bottom of the window.** The camera aimed at `ORIGIN`, which is the
   feet-on-floor anchor rather than the subject's middle.

**Input was synthesised without another dependency.** No `xdotool`, but `libX11` and `libXtst` are
present, so XTEST goes through `ctypes` in about thirty lines. One catch worth writing down: with no
window manager there is no input focus, so the first injected keypress was silently swallowed —
`XSetInputFocus` on the app window is required before anything is delivered.

**Verified end to end after the fixes**: `K` twice moved the shatter dial 4 → 6 → 8, `Space` punched a
hole, the status line read `fired: 1 hole(s), the proxy is now 13 cells`, and eight pools landed on the
floor. That is the whole feature driven by real X key events through the real app.

**Still deliberately absent.** The pools are flat discs of one shared unit-radius circle asset scaled
per pool, not projected decals; and nothing in the crate moves a plug or its pieces.


---

## Phase 4 — the crate became `bevy_carnage`, and grew the layer the name promises

**AG-025 … AG-031, all landed.** The crate could already cut a subject apart, sever it by region
query, bond the pieces, bore channels through it and eject the plugs. What it could not do was say
anything about what came *out*, which is most of what a gore crate is for. Renamed, then extended in
place.

### AG-025, as landed — the rename

150 occurrences of the old name across 23 tracked files, plus the GitHub repo and `.serena/project.yml`.
Mechanical, and the completeness check is not a count: `src/lib.rs` opens with
`#![doc = include_str!("../README.md")]`, so every README code fence is a compiled doctest and a fence
still naming the old crate fails `cargo test`. It passes.

**`BACKLOG_ARCHIVE.md` is the one deliberate exception**, with a single line added at its top saying
so. Rewriting the history of what happened under the old name would make the record lie. **Nothing
else spells the old name** — including this file and `CLAUDE.md`, whose prose about the vendored era
says "this crate's former name" rather than writing it out. That is what keeps a case-insensitive
recursive grep for the former name, with `BACKLOG_ARCHIVE.md` excluded, a *ratchet* that returns
nothing rather than a list of exceptions to read past — and it is why the check is described here
instead of quoted, since a quoted one would trip on itself.

### AG-026 … AG-029, as landed — wounds, spatter, bleed, feel

Four modules, 33 tests, one golden. `src/order.rs` was promoted out of `bake.rs` first, because the
new folds need the same checked total-order sort the vertex soup does and two copies of it would drift.

**The spatter model is a reduction of a measurement.** Comiskey, Yarin & Attinger 2018 show blood
disintegrating by percolation, so droplet size and initial speed are **inversely** correlated. One
random draw sets the size fraction and the speed is the inverse of the same number, on the CPU and in
the shader both, and a test asserts the correlation (Pearson `r < -0.9`) instead of a comment claiming
it. The first GPU pass omitted it — all droplets one size — and looked exactly like confetti, which is
the failure the paper's own framing predicts.

### AG-030, as landed — GPU blood behind a feature

`bevy_hanabi 0.19` resolves against `bevy 0.19.1` with no `wgpu` conflict, and
`cargo tree --no-default-features --features serde | grep -c hanabi` prints `0`. `tests/leaf.rs`'s
`ALLOWED_DEPS` was widened in the same commit with the review its own assertion message demands, on
two terms: it is optional, and **it cannot report** — Hanabi 0.19 has no public GPU→CPU readback path
at all, so the "cosmetic output never re-enters the deterministic half" rule is enforced by the library
rather than by anyone remembering it.

### AG-031, as landed — the demos, and the digest

`examples/carnage.rs` (interactive) and `examples/capture_carnage.rs` (headless). The recorder prints
one line two runs must agree on:

```text
carnage: frames=382 wounds=253 stains=26892 digest=c7fde149e80f1b13
```

FNV-1a over every stain position in placement order, so it covers the bake, the bond graph, wound
extraction and its sort, the wound seed, the droplet draws, the ballistic solve and the pulse schedule.
Measured: identical twice, and the 382 rendered PNGs are byte-identical too.

### What this phase falsified, and it was mostly the plan's own premises

| prediction | outcome |
|---|---|
| the two local commits carry work the publish lacks, so replay them onto it | **falsified** — the publish had already absorbed both under different hashes. All 139 local-only lines were *older* variants (`bake(world, soften)` vs `bake(world, soften, &[])`, `face_cut: Vec<bool>` vs `FaceKind`, isomesh `22c3b35` vs `aa82b0b`, "Eleven dials" vs twelve). The cherry-pick conflicted in `src/lib.rs`, `mesh.rs`, `proxy.rs` and `soup.rs` — exactly the fracture-geometry merge that must never be guessed at. Took the publish verbatim; nothing was lost, and that was checked rather than assumed. |
| `EffectTtl` is a Hanabi type | **falsified** — it does not exist in 0.19. It is this crate's own component, and it has to be: `EffectSpawner::has_completed()` reports the *spawner* finished, which for a one-shot burst is almost immediately, so despawning on it alone cuts the spray off mid-flight. Both conditions are required. |
| `spatter_speed_scale = 1.0` is a sensible shipped default | **falsified by arithmetic.** At 1.0 the paper's measured 40 m/s under the examples' 18 m/s² gravity throws a droplet `40²/(2·18) ≈ 44` metres. Correct for a gunshot, a fountain leaving frame on a 1.8 m subject. The default stayed at 1.0 — the constants are *measurements* and a default that quietly divided them would make them lie — and both examples set 0.25, which is where a look decision belongs. |
| a wound's area and normal can be approximated where the harness has dropped the cell | **falsified twice, visibly.** Averaging the cut-face area over every part made a fingertip bleed like a torso; a `Vec3::Y` pulse normal made every resting gib fountain straight up. Both were invented numbers, and the real ones were one field away — `Chunk` now carries its `FragmentId`, and `GorePart` its `ProxyCell`. |
| the examples were verified by compiling and by the recorder | **falsified on first run.** `examples/carnage.rs` panicked on keys `6` and `R`: `body::Thrown` is not `init_resource`d by `common::body` (a plug counter is the caller's bookkeeping) and nothing had run those paths. The recorder could not have found it — it initialises the resource itself. Fixed, then re-run: key `6` bores a channel and ejects a plug, `R` resets, no panic. **Compiling an example is not running one**, which is the same lesson AG-024's followup recorded and the second time it has been paid for. |

### Still deliberately absent

Nothing in this crate applies trauma, hit stop or shake; `feel.rs` returns numbers and the caller moves
its own camera. Nothing writes `Time<Virtual>`. Nothing reads a particle back.


---

## Phase 5 — the four kernels, as landed on 2026-09-04

`0.3.0`. Five features, one release, each a crate this one composes and none depending on it back;
`bloodstain` went to `0.2` under them. The predictions were pre-registered in each crate's tests
before the numbers were known, which is the only way a golden can be frozen honestly.

| feature | where | prediction | outcome |
|---|---|---|---|
| spectral blood | `bloodstain::spectral` | L* falls strictly with film thickness; arterial reads redder than venous at every thickness | **held** — 60 thicknesses and 30 pairs, plus six colours frozen to the bit |
| cross-section caps | `bevy_cross_section`, `FragmentGeometry::annotate_cap` | every band's drawn width is within 10 % of the sourced table | **held at every region** — and the head's split of a 6.8 mm total into three layers is the crate's own, said so |
| flaymap | `bevy_flaymap` | the bone handoff fires exactly once, on the paint that crosses the cortex start | **held**; on the flagship's torso the cortex shows on the third scripted hit |
| laceration | `bevy_laceration` | gape zero at `t = 0`, monotone in time and tension; a cut across the Langer lines gapes more | **held** — and the blockout's cuboid could not open at all, because the kernel works with the vertices a mesh has; both demos lay a 24×24 skin patch on the thigh |
| fracture modes | `bevy_fracture_modes`, `modal.rs` | the first mode's only jump is the neck; the neck is the first face to give under a growing blow | **held only after a stated departure** — with every mode weighted equally an impulse at a bar's end excited the end modes and the neck never opened first; dividing each mode by its discontinuity energy is what makes a weak fault a weak fault |

What was falsified along the way: the first ADMM formulation converged to the zero vector (the
sphere constraint has to be *in* the sub-problem, which is exactly Sellán's linearised `cᵀMφ = 1`);
the one-luminance test of "a thin film is lighter than a pool" was wrong at a 0.19 mm film, because
blood at 540 nm is opaque past 50 µm — the test now compares the thinnest level, and the physics was
right. Cortical bone thickness is the one number in the release with no source; four DOIs were tried
and none had an open copy.

---

## The rest of the backlog is clear

All fifteen tickets are in `BACKLOG_ARCHIVE.md`, each with what it cost and what it falsified. Six
predictions were pre-registered; **five came back different from what was predicted**, and those
differences are the most useful thing this backlog produced:

| ticket | prediction | outcome |
|---|---|---|
| AG-001 | 12/12 proxy fragments closed, manifold, χ = 2, 0 open cut edges | **confirmed exactly** |
| AG-002 | χ and manifoldness conserved; only volume notices a filled bore | **falsified** — χ moves, orientation moves, volume is the field that *misses* it |
| AG-006 | fold ⟺ inconsistent orientation | **narrowed** — sufficient, not necessary; a doubly-wound fan folds with every counter at zero |
| AG-013 | falsified if the bump moves geometry | **held** — only a reported number moved, and it was ours being wrong |
| AG-011 | the async bake runs inline on the main thread | **falsified** — there is no async bake |
| AG-008 | a CDT is needed as the safety net | **falsified by AG-001** — refuse concave input instead of surviving it |

Two false claims were found in this crate's own source (`signed_volume`'s translation invariance, and
the fold equivalence we had already sent upstream), and one in the backlog's own corrections section.

### What is deliberately not here

- **A convex decomposition.** The proxy is the caller's; see `CLAUDE.md`'s boundary list.
- **A constrained Delaunay triangulator.** AG-008 explains why refusing concave cells beats surviving
  them. Reopen it if a caller genuinely needs concave support; `isomesh`'s `predicates` module is the
  exact-arithmetic floor it would stand on.
- **An async bake.** Measured at 0.33 ms; see AG-011.
- **Full closure of the render mesh.** Open edges fell from 13–19 to 3–9 per fragment with the emit-time
  seam weave, and the remainder is `convex_ring` deduping seam points within `WELD` of a corner. It is
  recorded rather than asserted, because a surface subset has a boundary by definition.

---

## Reading order

1. **Müller, Chentanez & Kim 2013** — `10.1145/2461912.2461934`. §1–2 and the VACD section. The
   production answer; dissolves the capper problem as a side effect.
2. **Shewchuk 1996, *Triangle*** — `10.1007/bfb0014497`. The PSLG definition and the hole/concavity
   flood fill.
3. **Diazzi & Attene 2021** — `10.1145/3478513.3480564` (impl at `github.com/MarcoAttene/VolumeMesher`).
   §2 on why CDT-based methods fail on defective input, and the cell classification. Probably not usable
   as a dependency (C++), but it is the one method whose *stated* input tolerance matches a real glTF
   character — self-intersecting, non-manifold, disconnected, holes and gaps — so it tells you what the
   tidy-up step in V-HACD/CoACD is actually costing you.

Ten-minute runner-up: **Sacht et al.**, *Consistent Volumetric Discretizations Inside Self-Intersecting
Surfaces*, Figs. 10–11 — the picture of a generalized-winding-number union welding a character's limbs
to its torso. That figure is the whole argument for never unioning the shells.
