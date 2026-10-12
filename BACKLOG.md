# bevy_carnage — BACKLOG

**Updated:** 2026-10-11
**Companions:** `CLAUDE.md` (rules), `README.md` (what the crate promises),
`docs/research-brief.md` (the open problems), `docs/isomesh-upstream-asks.md` (what we need from the
validator).

**15 tickets archived, 10 landed in phase 3 (AG-015 … AG-024), 7 landed in phase 4 (AG-025 … AG-031), 13 landed in phase 6 (AG-032, AG-034 … AG-045), 1 open (AG-033, blocked).**
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

**`0.3.1`, the same day, after one question — "is there a single crate that exposes all of this?"** There
was not, quite: `bevy_wetmap` and `bevy_viscera` were dev-dependencies, so the umbrella composed four
kernels and left two at the door. Both are now real dependencies and `bevy_carnage::{wetmap, viscera}`,
every README carries the same eight-row family table with its own row marked, and the three crates
without a clip got one — `capture_family` records the blood pair, `capture_entrails` the strands.
Recording the swatches falsified the ageing chain: an additive Bremmer shift on a near-black venous
pool dried it grey-teal. `bloodstain 0.2.1` scales the MET and HEMI stops to the film's own red
instead, so every film dries brown and the authored walk is the `k = 1` case exactly.

**`0.3.2`, from the review of the whole release.** Five defects and a handful of nits, each with a
test that failed before its fix: the laceration bed's two end caps were wound against their normals
(back-face-culled from inside the trough, where a wound is looked at — `every_bed_face_is_wound_to_its_normal`);
the cross-section strip baked its along axis at the depth axis' 5.1 mm while `annotate_cap` mapped one
repeat onto the scale's 50 mm, stretching every lobule tenfold, and its noise was periodic in rounded
cells rather than in the tile, so it seamed (`the_along_axis_tiles_without_a_seam` now measures the
claim it is named for; the strips are re-frozen and the plugin's default height is 512 rows);
`bake_strips` took a bare `Assets<Image>` in the headless configuration its own doc advertised, which
Bevy 0.19 panics on (`tests/plugin.rs`); a black fresh colour fell back to the authored walk and made
red rise (`k` is now `max(0, r / r_oxy)`); the wetmap rebuilt its sixteen-entry spectral table on every
tick of every canvas. Also: the flaymap ray test now rejects NaN by negation, an off-atlas UV returns
`None` through the per-mesh memo, the four kernels joined `tests/determinism_lint.rs` (three sorts
annotated), the leaf ratchets read target-gated tables, and `bevy_carnage::{wetmap, viscera}` lost a
`default-features = false` that turned nothing off.

---

## Phase 6 — the visual pass (2026-10)

Dependencies first, then every visual surface of the crate — 24 windowed examples, 11 headless
recorders, 10 terminal examples, 15 committed GIFs, 15 web demos — looked at round by round, each
defect a ticket and each ticket a commit. The registry release is cut once, after the last fix.

| | ticket | size |
|---|---|---|
| [x] | **AG-032 — dependencies to the newest the ecosystem allows.** `bevy` 0.19.0 → 0.19.1, `serde` 1.0.228 → 1.0.229 and `wasm-bindgen` 0.2.126 → 0.2.129, with the transitive closure under them, in both lockfiles: the workspace's, moved by `cargo update --recursive` on this crate's own packages so game-only dependencies stayed put, and the standalone one the mirror CI builds from, moved by a plain `cargo update` on a copy of the crate. Manifests untouched: every requirement already admits the newest version, and raising a library's minimum only restricts its consumers. `CLAUDE.md`'s vendored-source path follows to `bevy-0.19.1`. | S |
| [ ] | **AG-033 — Bevy 0.20.** Out on 2026-10-08, and the crate stays on the 0.19 line until its two neighbours move. | L |
| [x] | **AG-034 — nine example headers told you to run crates that no longer exist.** `cargo run -p bevy_wetmap`, `-p bevy_flaymap` and `-p bevy_viscera` in seven headers, `-p bloodstain` in two more, all folded into `bevy_carnage` at 0.5.0. Each is now `cargo run -p bevy_carnage --release --example <name>`. | S |
| [x] | **AG-035 — `cross_section` promised a mouse wheel it does not read.** The header said "Orbit with the mouse wheel"; the camera orbits on its own, off the clock, and `web/play.html` already said so. The comment now says what the code does. | S |
| [x] | **AG-036 — the demo counts were stale in five places.** `docs/DEMOS.md` said "All seven run" and listed seven commands for a crate with twenty-four windowed examples; `scripts/build_web.sh`, the CI `site` job (its comment and its name) and `web/play.html` said "ten" for a roster of fifteen. `DEMOS.md` now lists every windowed example, with the feature the six that need one require, and the rest say fifteen. | S |
| [x] | **AG-037 — two web demos never found their canvas.** `flesh` and `carnage_web` asked the `WindowPlugin` for `"#bevy"`; `web/play.html` has only `<canvas id="carnage-canvas">`, so on the site both rendered nothing. Both now name `#carnage-canvas`, as the other thirteen do. | S |
| [x] | **AG-038 — the examples drive themselves for screenshots, and `tools/qa.sh` runs a whole round.** Nothing could screenshot a windowed example or send it a key: this process has no Screen Recording or Accessibility permission, `capture_*` is headless and scene-specific, and a path dev-dependency on `bevy_devshot` would break the standalone mirror build. `examples/common/autopilot.rs` reads `CARNAGE_AUTOPILOT` (`<frame>:shot`, `key=<Name>`, `hold=<Name>*<n>`, `exit`), writes the keys as `KeyboardInput` messages and the shots through `Screenshot`, and is inert without the variable; all 24 windowed examples register it. `tools/qa.sh <out-dir> [name …]` builds, runs the 12 recorders, the 9 terminal examples and the 24 windowed ones, and writes contact sheets, `warnings.txt`, `digests.txt` and `missing.txt`. | M |
| [x] | **AG-039 — `bench_carnage` refused to report a time on every machine at hand, because its golden was one platform's.** It pinned `0xfc71_5deb_a0ea_5cf8` and 544 fragments. An Apple-silicon Mac gives `0x4366_a8b6_4352_c85a` and 554 fragments, and an x86_64 Linux host gives `0x99ca_3d14_78d2_58e3`, one bond and one pulse fewer. The golden is now pinned per `target_arch`, and its doc comment says why. | S |
| [x] | **AG-040 — the bake welded each fragment's mesh with 27 hash lookups per corner, most of them for a corner it had already emitted.** `AttributeWeld` keeps an exact-repeat index (recentred position bits, normal bucket, UV bucket), so a bit-for-bit repeat returns its vertex in one lookup. The proof that this returns what the probe would have is in the field's doc. The bake on `bench_carnage` drops from 55.1 to 50.3 ms (−8.7 %), with the digest unchanged. | S |
| [x] | **AG-041 — the bake recomputed every cell's face planes for every triangle it placed.** `ProxyCell::contains` ran Newell's method over every face on each call, and assigning the render skin calls it once per triangle per candidate cell. `ProxyCell::halfspaces` computes each cell's planes once, and `Halfspaces::contains` is now the only containment test. The bake on `bench_carnage` drops from 50.2 to 41.4 ms (−17.7 %), with the digest unchanged. | S |
| [x] | **AG-042 — the visual pass could not see 35 of its own windowed shots.** Round 1 lost them to pure black (`sever` 13, `ribbons` 7, `pooling` 5, `preset` 4, `rheology` 3, `stain_morphology` 3), because a macOS window that is covered, on another Space, or behind a locked screen stops presenting. With a script, the autopilot now opens no window at all: `common::autopilot::plugins` swaps winit for a plain loop, every camera draws into an off-screen image at 2× the window's logical size, time steps a fixed 1/60 s per frame, `exit` waits for every shot in flight, and each frame waits for the GPU. All 24 windowed examples are registered through it. `qa.sh`'s digest summary also caught `capture_gore` and `capture_family` for the first time. | M |
| [x] | **AG-043 — once the legs went, the torso hung at standing height for the rest of the clip (R1-01, high).** In six examples and three layers, the biggest island was held in place whether or not anything touched the floor beneath it. Now it stays standing only while it holds a piece the body stood on. Otherwise it comes down with the rest, slumping into a heap rather than being thrown, since nothing hit it. Fixed in `examples/common/body.rs` (`sever`, `carnage`, `entrails`, `ribbons` and their recorders, with "nothing holds it up now" in the HUDs), in `capture_gore`'s fracture-modes clip, and in the crate's own `preset`, whose `thrown_by` judges footing by the subject's lowest extent since the crate knows no floor. | M |
| [x] | **AG-044 — gut spilled as rigid red rods (R1-02, high).** `viscera::spill` laid every strand straight along its heading, 84 cm of ruler-straight tube. For the first frames after a blow that is a fan of poles, and on the floor a straight stick. Each strand is now laid with `Strand::curved`, which is new: it leaves the wound along its heading, turns toward the floor within about a quarter of its length, and meanders in a slow S with a phase of its own. Every segment is still laid at its rest length, so the solver starts it at rest. | S |
| [x] | **AG-045 — a run that hangs once in about a hundred left nothing to diagnose it with.** `spill`, `carnage` and `rheology` each once saved every shot and then never exited. `spill` then ran clean four times in a row on its own, and all 24 examples ran clean through 72 more scripted runs. `qa.sh`'s watchdog now runs macOS `sample` on a run it is about to kill, into `<out>/<name>.hang.txt`, so the next occurrence leaves a stack for every thread. Not fixed: no hang has occurred since the sampler went in, so its cause is unknown. | S |
| [x] | **AG-046 — the GPU spray was pink square confetti (R1-03, high).** Two separate faults made one look. Each droplet was a bare billboard, a square quad, and now carries Hanabi's `RoundModifier::ellipse`, so it is a round drop. And the colour every Hanabi effect drew with was `spectral::srgb`, the gamma-encoded triple the crate's *materials* take, while a particle gradient takes linear values. Encoded values read as linear display lighter and greyer, which is the dusty pink. The droplet and ribbon colours are now `spectral::linear_srgb` of the same two films, so the spray shows the blood the film model computes. | S |
| [x] | **AG-047 — repeat shots never moved, so pools never merged (R1-05, high).** `qa.sh` fired `pooling`'s four shots and `bullet_holes`' five at one aim. A second identical bore reaches no proxy cell and changes nothing, so `pooling` showed one shot's six slicks four times over and never the merge it exists to show. `bullet_holes`' second and fourth shots were exact repeats, and no scripted blow ever reached a limb. The key lists now step the aim between shots, and `sever` gains a blade at the right arm and one at the right leg. The examples were right; the script was not. | S |
| [x] | **AG-048 — a slash opened as a dashed row of pits (R1-04, high).** Three faults in `laceration::tear`, measured from above with a throwaway rasteriser. The clip removed only triangles with all three vertices inside the gape. A cut at an angle to the mesh crosses triangles that keep a vertex outside, and each was stretched across the gap as a bridge of skin. At a gape narrower than a cell, nothing was removed at all and the cut was invisible. The bed's walls stopped at the rail, while the falloff had pushed the lips past it, which left a strip of nothing down both sides. And the bed ended at the path's ends, while the clip took skin up to `half_width` past them. Now a triangle the path enters, or one with a vertex in the gape and vertices on both lips, is clipped too. Every vertex on the mouth lands on one lip curve, and the bed rises to that curve every 2 mm, runs as far past each end as the clip measurably did, and fills the outside of each bend. | M |
| [x] | **AG-049 — blood read as pink paint because the shared scene was overexposed (R1-06, colour half).** Bevy multiplies by `2^-ev100 / 1.2` and lights a matte surface by `albedo · lux · cos / π`. At the default exposure, EV100 9.7, a white surface facing `light_and_floor`'s 9 000 lux key rendered at 2.9, not 1, so the scene was lifted about threefold. The floor had to be authored at 2 % albedo, darker than fresh blood, to come out mid-grey, and every colour the crate computes (stains, pools, spray, cut faces, gut) was lifted with it into pink. `light_and_floor` now sets `SCENE_EXPOSURE`, EV100 11.2, on the scene's cameras, where that white surface renders at 1.04. The floor and the blockout's skin (now one constant, `body::SKIN`) are authored brighter to render as they did, and the gore keeps its own albedo. Thirteen example files share the scene. The examples' scenes only; the crate is untouched. | S |
| [x] | **AG-050 — dried blood drew salmon in `drying` (R1-07, its first half).** The same fault as `AG-049` in a scene of its own. A key at n·l 0.69 plus a fill light a white floor at about 10 600 lux, which the default exposure renders at 3.4, so the hemichrome crust the HUD reports as 0.26 0.11 0.08 drew as (121, 88, 84). The camera is now exposed at EV100 11.45 and the floor authored to match. | S |
| [x] | **AG-051 — a slash across a blockout limb removed the limb's whole face (0.5.3 regression).** `AG-048` clips every triangle the cut enters. On a cuboid leg a face is two triangles whose corners lie far beyond the influence radius, so the preset's slash deleted the face. With back faces culled the leg looked severed, and the bed showed as a pale line. The two new clip rules now apply only to a triangle whose three vertices are within `influence`, the ones moved onto the lip the bed meets. A mesh coarser than that keeps its skin and its wound stays closed, as in `0.5.2`. | S |
| [x] | **AG-052 — the preset's stains faded to pink and its skin came out nearly white (R1-07, its second half).** The preset scene's 11 000 lux sun rendered a white surface at 3.6 at the default exposure, so the skin, authored as a plausible 0.80 0.62 0.53, drew nearly white, and every wound and stain the preset paints went pink. `preset_scene` now carries `EXPOSURE` (EV100 11.5) and the `AMBIENT` that goes with it (260 raised by the same 3.5, so the shaded side is unchanged), used by `preset`, `capture_preset` and `carnage_web`. The props keep their albedos and render as authored. The slab rises from 9 % to a concrete's albedo, so blood has something light to read against. | S |
| [x] | **AG-053 — gib ribbons read as fishing line (R1-23).** `vfx::gib_ribbon` drew each strand 12 mm wide at its head and tapered it to nothing over the strand's 0.9 s, so most of every strand was thinner than a finger. From the examples' camera that was one to three pixels beside 60-pixel chunks, and once `AG-046` gave it blood's own dark red it was harder still to see. A strand is now 20 mm, a dragged stream rather than a thread, and holds that width over its first half before tapering. | S |
| [x] | **AG-054 — every reset logged "Entity despawned" twice (round-2 candidate).** `CarnageVfxPlugin`'s ribbon and effect systems queued plain `with_child`, `insert` and `despawn` commands. Commands apply at the next sync point, and a consumer that despawns its chunks from a system the crate cannot be ordered against despawns them in between. In `ribbons` that is the reset, two warnings each, ten a run. The four commands now use `try_insert`, `try_despawn`, and a silenced `with_child` that does nothing to an entity that is gone. | S |
| [x] | **AG-055 — three more scenes lit three times too bright for their exposure (R2-01, R2-13).** `explode` builds the shared scene by hand with `Commands`, with copies of its colours, so `AG-049` never reached it, and its cut faces stayed cherry-red plastic. `spill` (8 000 lux) and `viscera_spill` (9 000) were never exposed for their keys, so their gut, the same authored red that reads maroon in `entrails`, drew coral. `explode` now takes the shared scene's constants: `SCENE_EXPOSURE`, the new `FLOOR`, and `body::SKIN`. `viscera_spill` takes `SCENE_EXPOSURE` with its ambient raised to match. `spill` is exposed for its own key. The floors and torsos are authored brighter to render as before. | S |
| [x] | **AG-056 — the hang sampler left nothing when a hang came back.** Round 2's spot runs of `spill` and `viscera_spill` saved every shot and then hung, the first recurrence since `AG-045`. Neither left a `.hang.txt`: `sample`'s output, error included, went to `/dev/null`. The watchdog now writes the process state and `sample`'s own output to `<name>.hang.txt`, and the stacks to `<name>.hang.txt.sample`. Not fixed: six direct runs of `spill` straight after exited cleanly in 2 to 4 s, so the cause is still unknown. | S |
| [x] | **AG-057 — floor stains painted the legs and chunks standing in them (R2-03).** Bevy's forward decal fades as `1 − distance / depth_fade_factor`, the distance from the decal's plane to the surface behind it, in metres. `decal.rs` set the factor to 1.0, so a floor stain still painted a leg point half a metre up at half opacity. A body standing in blood grew vertical streaks along its feet and a pale wash up its shins, and chunks lying in a pool turned into translucent pink ghosts. The factor is now `STAIN_DEPTH_FADE_M`, 4 cm, which still covers a floor that is not quite where the stain was placed and the sole of a foot. | S |
| [x] | **AG-058 — the far side of the preset's orbit was one flat brown (R1-12).** The scene had one light, the sun, so once the orbit passed behind the body every face it saw was lit by ambient alone. The torso, arms and legs merged into a single dark silhouette, and none of the demo's wounds or shading read. `preset_scene` now adds an unshadowed fill opposite the sun at a quarter of its strength, marked `Sun` so a reset despawns it with the scene. `preset`, `capture_preset` and `carnage_web` share it. | S |
| [x] | **AG-059 — three of the fifteen web demos showed a black canvas (web pass).** `cross_section`, `fracture_modes` and `laceration_open` never named the page's canvas. Bevy therefore created its own, appended it to the page body below the fold, and drew there, while the `#carnage-canvas` the page shows stayed black. `AG-037` renamed the selector in the twelve demos that set one and missed these three, since they set none. All three now set it, and `scripts/demo_facts.sh` fails a roster demo whose source does not. | S |
| [x] | **AG-060 — window titles and messages still named the folded crates (R1-46).** `bevy_cross_section`, `bevy_fracture_modes`, `bevy_laceration`, `bevy_flaymap`, `bevy_wetmap` and `bevy_viscera` have been modules of this crate since 0.5.0, but 61 places still named them. That was three window titles, three printed banners, 17 `warn!` prefixes and the rest doc comments, across 23 files. Titles now read `bevy_carnage - …` like the others, banners and warnings name the module, and doc comments name the module, or "the former `cross_section` crate" where they record that crate's own version history. | S |
| [x] | **AG-061 — the "exit hangs" were the watchdog lying (closes `AG-045`, `AG-056`).** `qa.sh`'s `run_limited` stops its watchdog by killing the watchdog's `sleep` first, because bash will not act on the subshell's own `TERM` until that child returns. With the `sleep` gone, the subshell ran its next line, `echo timeout >"$flag"`, before the parent's `kill` landed. The parent then reported a run that had exited in seconds as having hit the 180 s timeout, and the subshell died before sampling anything. That is every hang `AG-045` and `AG-056` recorded: every shot saved, a timeout, no `.hang.txt`. The parent now marks a run finished before it touches the watchdog, and the watchdog checks the mark after its sleep. | S |
| [x] | **AG-062 — on the site, the legends printed over each other (web pass).** Every legend is placed in logical pixels for the default 1280-wide window. The page's canvas is as wide as its column, 950 logical pixels on a desktop, so in `drying` and `rheology` the top-left and top-right blocks ran into each other, and `drying`'s two pool panels did too. The 15 web demos now run `common::fit_hud`, which scales the UI by the window's width over 1280, capped at 1, so the site draws the layout the native window does. | S |
| [x] | **AG-063 — two bottom panels were drawn through their own status lines (R1-36, R1-40).** `gore_tier`'s flash meter was pinned 112 px up and `fragment_energy`'s histogram 96 px up, each a guess at how tall the status text below would be. Both statuses are taller (six lines and five), so the meter struck through "tier BloodAndGore" and the bars ran through the tissue line. Each panel now shares a bottom column with its status, above it, so the status pushes it up. `fragment_energy`'s fourth status line ran off the window's right edge and wrapped; its parenthetical is now its own line. | S |
| [x] | **AG-064 — two subjects stood behind their own legends (R1-18, R1-41).** `gore_tier`'s camera stood 2.35 m from the body, so the eleven-line legend ran across the head and the feet were below the window. `fragment_energy`'s bone reached up behind its eight-line legend, which printed across its top fragments. Both cameras now stand farther back, 3.6 m and about 1.22 times as far, and aim so that each subject fills the band between legend and status. | S |
| [x] | **AG-065 — a cut face at rest could read as a pink tile (R3-01).** In `capture_carnage` one settled chunk's broad flat cut face lay at the sun's mirror angle. At roughness 0.42 and the default reflectance, the specular lobe covered the whole face, which washed out to a flat salmon-grey for the last 160 frames. Six examples each built their own copy of the interior material. They now share `common::interior()`, with water's reflectance (F0 0.02) and roughness 0.5. `docs/carnage.gif` is regenerated. | S |
| [x] | **AG-066 — blood fell on the subject after a reset (R1-08).** Two causes, each found by counting entities frame by frame around an `R`. First, a reset despawned the stains, pools and chunks but none of the sprays, so up to 94 effects still in the air kept falling. Second, `carnage`'s systems were not ordered against the `vfx` plugin's, which therefore read each heartbeat a frame late. The chunks' last heartbeat before the reset was read and sprayed one frame after it, from where the chunks had lain: the cluster of drops `R1-08` saw on the floor 30 frames after every reset. `body::wipe` now despawns every `EffectTtl` spray, and `carnage` runs its chain before `CarnageVfxSystems`. | S |
| [x] | **AG-067 — `pooling` cropped its head and half its slick (R1-14).** Both `pooling` and `capture_pooling` stood the camera 2.6 m off and aimed it at the feet. The head left the top of every frame, and the body stood between the camera and the slick that forms behind and beside it, where a channel throws its plugs. Both now stand back and up, aimed between the body and the slick. | S |
| [x] | **AG-068 — the preset scene hid its own wounds (R1-12, R1-13).** `preset` and `carnage_web` orbited the body once every 42 and 52 s. Half of each lap looked at the side the sun does not light and the hits do not land on, so the body read as one flat brown shape, and for seconds of each lap the sheet stood between the camera and the body. `capture_preset`, and so `docs/preset.gif`, stood still in a spot where the sheet hid the right arm and the scalded thigh for the whole clip. The windowed demos now sweep across the front instead, from 70° left to 5° right, on `preset_scene::sweep`. `capture_preset` stands on that arc, nearly straight ahead, and all three aim low enough to keep the floor spatter in frame. `docs/preset.gif` is regenerated. | S |
| [x] | **AG-069 — screen blood was six red pills (R1-39).** `gore_tier` drew each screen mark as a UI box with fully rounded corners, sized in percent of the window on both axes. A disc in normalised device coordinates is wider than tall on a 16:9 window, so every mark came out a flat capsule, and the six read as UI chips rather than blood on the lens. Each mark is now an image of the crate's own stain silhouette (`rasterise`), with 2-4 spines thrown outward from the screen centre. It is round on screen, a viewport-height across, so it stays inside the disc `occludes_aim` judged, and the radii are larger, since a silhouette's body fills only part of its mask. | S |
| [x] | **AG-070 — `gore_tier`'s floor blood was drawn as outlines (R1-37, in part).** The 48 landed stains were gizmo ellipses, a one-pixel outline each, so blood read as thin rings, and at `Stylised`, in the spark palette, as specks too small to make out. Each stain is now a quad on the floor carrying the crate's own silhouette for it, filled and faded, sized and tinted per tier as before. | S |

> BLOCKED: bevy_hanabi has no bevy-0.20 release (crates.io 0.19.0; its main still declares bevy 0.19), and the game's avian3d 0.7 is on bevy 0.19 with no 0.20 release; the game consumes this crate by path, so the crate cannot move alone.

### Phase 6, as landed

- **AG-032.** Held against a baseline taken on the same host before the bump. The crate's 287-test suite and every other test binary pass with identical counts, so `hash_f32`, `the_spatter_model_is_frozen`, the seven bloodstain goldens and `the_measured_rows_are_the_papers` did not move and nothing was re-blessed. The whole workspace's `cargo test --exclude bevy_debugger_mcp` gives the same 103 result lines before and after, including one failure that was already there (`emerge-mapper`'s `no_two_leaders_cross`). The release, lean and example builds, the 15-demo wasm check and `scripts/demo_facts.sh` pass, and clippy reports the same 338 warning lines with the same text. The standalone tree's `cargo test` passes on Linux x86_64, which is what the mirror CI runs. `wasm-bindgen-cli` has to move to 0.2.129 to match, since `build_web.sh` reads its version out of the lock. Three things the plan assumed were not so, each equally true before the bump: `cargo tree --no-default-features --features serde -e normal | grep -c bevy_render` prints 4, not 0, in the workspace and in the standalone tree; `cargo check --workspace --all-targets` fails in four `bevy_debugger_mcp` benches, and that crate's `stdio_integration_test` hangs `cargo test --workspace`; and a standalone `cargo test` on an Apple-silicon Mac, at the default opt-level 0, fails `preset::tests::a_wound_stains_the_floor_beneath_it` ("nothing landed on the floor") on the old lock as well as the new, while it passes at opt-level 1, which is the workspace's, and on Linux. That last one is a codegen-sensitive result on one platform, recorded here and not fixed.
- **AG-034.** The grep the ticket named, `cargo run -p bevy_(wetmap|flaymap|viscera|cross_section|laceration|fracture_modes)|-p bloodstain`, found nine files, not the seven the plan counted: `stain_sweep` and `dry_timeline` still said `-p bloodstain`. None of the nine needs `--features`. The same grep over `docs/`, `README.md`, `tools/`, `web/` and `scripts/` is empty.
- **AG-035.** The example reads no `KeyCode`, no `MouseButton` and no `MouseWheel` at all; `orbit` moves the camera from `time.elapsed_secs()`. Nothing else in the file made a wheel claim.
- **AG-036.** The twenty-four are the examples that build a `DefaultPlugins` window; `rod_determinism` only mentions it in a comment and is not one. The six `required-features` entries in `Cargo.toml` are `carnage`, `ribbons` and `pooling` (`vfx`) and `flesh`, `preset` and `carnage_web` (`flesh`); both features are on by default, so the flag is there for a `--no-default-features` build. The roster of fifteen was counted from `DEMOS` in `build_web.sh` and in `web/play.html`, and `scripts/demo_facts.sh` still passes. The CI job's `name:` changed with its comment; `main` has no branch protection or ruleset, so no required status check names it.
- **AG-037.** One string per file; `fit_canvas_to_parent` stays. Native runs are unaffected, since `canvas` is read only on wasm32. What proves it in a browser is the web pass of the round that follows; the fallback proof is `grep -n canvas examples/{flesh,carnage_web}.rs`, which now shows `#carnage-canvas` and no `#bevy`.
- **AG-038.** Measured, on the first run of `sever` with `30:shot 40:key=Digit1 160:shot 200:exit`: it exits by itself with status 0 and writes both PNGs, and the bottom line reads "projectile: severed 28 bond(s), 6 fragment(s) came off". **The plan's expectation that an arm is gone was not observed**: the aim starts at `(0, 0.25, 0)`, 0.32 m from either shoulder, so the blow lands on the upper torso and neck. Reaching an arm means holding an arrow key first, which a later round can script if it matters. Without the variable `sever` stays open (alive after 12 s); with `CARNAGE_AUTOPILOT=x` it logs "autopilot: `x`: expected <frame>:<action>" at error level and runs normally.
- **AG-038, the key lists.** Three read-only passes over the 24 examples' own key reads changed the plan's table for twelve of them. Polled `pressed()` keys need holds (`rheology`'s arrows and brackets, `viscera_spill`'s `G`, `gore_tier`'s `F`): a one-frame press moved `rheology`'s age dial about 1.8 ticks. The strike demos (`carnage`, `entrails`, `ribbons`, `sever`) reset between blows, because damage is cumulative and a later blow reports "nothing left to break here". `wetmap_paint`'s mouse painting cannot be driven, so its list uses `Space`'s three fixed bursts. `drying`'s second hold shrank to 110 frames so it ends before the next press.
- **AG-038, a window needs a display.** On a Mac with no live display a window presents black: the same release binary, run on the build host, wrote two byte-identical black 1920×1360 PNGs, and run on a laptop with a screen it wrote the scene. So `qa.sh` has phases: `QA_PHASES` picks from `build recorders terminal windowed sheets`, `QA_BIN` points at prebuilt binaries, and `QA_KEEP_MISSING=1` appends to `missing.txt`. Build and record on the fast host, `rsync` the out-dir and the binaries (portable between Macs of one OS) to a machine with a screen, and run `windowed sheets` there.
- **AG-039.** A `git bisect` over the commits since the golden was blessed tested four of them, and a direct run tested `438908b`, the blessing commit itself. All five produced the arm64 value. So the pin was never this host's, and the bench's own output did not move. **The cause is inferred, not isolated:** `std`'s `sin`, `cos`, `atan2` and `powf` call the platform maths library, which can differ in the last bit between Apple's libm and glibc, and a cut plane derived from one of them would partition differently. The crate promises that two runs of the same build agree, and each pin is that promise kept on one platform. It does not promise that two platforms agree. Recorded rather than fixed: if a game ever needs cross-platform bit-identity, the first thing to test is moving those functions onto `libm`, which would change emitted geometry on every platform.
- **AG-040, measured.** `sample` on a line-table build of `bench_carnage` put `AttributeWeld::insert` first, at 24 % of self time. The single hottest line was the probe's `HashMap::get` at `mesh.rs:353`, at 16 %. After the change the bench reports `bake_ms` 55.1 → 50.3, `carnage_total_ms` 70.1 → 65.4 and the worst per-body spike 4.14 → 3.78 ms, three runs each on an M4 Pro, best-3 spread under 1 %, digest `0x4366_a8b6_4352_c85a` unchanged. The re-profile puts the weld at about 13 % and `ProxyCell::contains` (via `face_plane`, from `soup.rs:613`) first at about 23 %. That is the next lever.
- **AG-041, measured and looked at.** The planes are a function of the cell alone, and the test is the same expression, so every answer is unchanged. Three call sites in `soup.rs`: the shell and triangle assignment, and the plug's skin. The cells there do not change between building the planes and using them. `bench_carnage` on an M4 Pro, three runs: `bake_ms` 50.2 → 41.4, `carnage_total_ms` 65.4 → 56.5, worst per-body spike 3.77 → 2.99 ms, digest `0x4366_a8b6_4352_c85a` unchanged. Against `0.5.0` the two tickets together take the bake from 55.1 to 41.4 ms (−25 %) and the worst hitch from 4.14 to 2.99 ms (−28 %). Every recorder was re-rendered and byte-compared with the round-1 frames from before both tickets. Eleven of twelve are identical in all 2,758 of their frames. `capture_carnage` differs from frame 19 on, and so do two runs of one unchanged binary: its GPU particles are not pixel-reproducible, while its CPU digest `0f45a783feb6bb85` is unchanged. Looked at by eye as well: `capture_holes` at its front, profile and back orbit angles, bullet channels and fragment seams cropped to about 4×, `capture_carnage` before and after at frames 40, 100, 200 and 380, and the orbiting windowed `cross_section` and `preset` across their autopilot frames.
- **AG-042, and the four turns that did not work.** Measured on the build host, with its screen locked: all 24 windowed examples, 324 shots, none black, none missing, in 153 s, against round 1's 35 black shots on a laptop with a live screen. The turns worth keeping:
  - **Pinning the window on top** (`WindowLevel::AlwaysOnTop`) made things worse on the laptop: 15 to 23 of each run's shots came back black.
  - **Rendering off screen while keeping the window** produced pure black too, for a reason worth knowing about Bevy 0.19. A screenshot's attachment is keyed by the whole render target, and `ImageRenderTarget` compares its scale factor. So `Screenshot::image(handle)`, which is scale 1.0, names a different target from a camera drawing into the same image at 2.0, and it reads back a texture nothing drew into.
  - **With that fixed, the window still existed.** On a locked screen Bevy blocked acquiring its surface every frame until wgpu panicked: "We timed out while waiting on the last successful submission to complete". That is why a scripted run opens no window.
  - **Without vsync the CPU ran ahead of the GPU.** That lost each run's final shots and hung `carnage` and `rheology` past their timeout, hence the `exit` hold and the per-frame wait.

  The off-screen render is the scene, cameras and HUD the window shows. Side by side, round 1's `sever` window shot and the off-screen one share HUD size, layout and lighting at 1920×1360.
- **AG-043, measured and looked at.** Byte-compared against round 1, each change starts exactly where the body loses its legs and nowhere earlier:
  - `capture_sever` is identical through frame 111 and changes from 112, two frames after the waist blade.
  - `capture_ribbons` changes from 118.
  - `capture_preset` changes from 120, the moment the body breaks.
  - `capture_gore` changes only in its fracture-modes clip, frames 90–179.
  - `capture_demo`, `capture_holes`, `capture_pooling`, `capture_entrails` and `capture_flesh` are untouched, and so are the other three `capture_gore` clips.

  Digests moved where they should: `capture_carnage` 220 wounds / 21 546 stains / `0f45a783feb6bb85` → 289 / 32 043 / `b540ac712d86e37a`, because the fallen torso's pieces bleed too, and `capture_gore`'s fracture-modes hash `5676116bc7e015de` → `860dd78ff64d9c47`. `capture_preset`'s digest does not cover flying pieces and stayed `af15ebc6c1c8033d`. Looked at frame by frame:
  - `capture_sever` at 108, 112, 116, 124, 140 and 177: standing, legs gone, the torso coming down as one mass, then a heap with red cut faces.
  - `capture_carnage` at 200 and 380 against round 1: a bleeding heap where a hovering block was.
  - `capture_gore` at 125, 150 and 179: a heap of banded pieces, zoomed to show fat, muscle and bone in the gaps.
  - `capture_preset` at 130, 200 and 359.
  - Windowed `entrails` blow 5: 34 chunks and 8 of 8 strands streaming, where round 1 showed 15 chunks and no guts at all.

  `sever.gif`, `carnage.gif`, `preset.gif` and `fracture_modes.gif` are regenerated, and the last now has a recipe in `docs/DEMOS.md`. `fracture_modes.md`'s alt text described a necked bar, but the GIF has always been this humanoid; it says so now. Looked at and kept: the new `fracture_modes.gif` shows faint Bayer-dither speckle where the torso used to be. Measured, the floor's mean colour equals the source frame's (99,96,93) and the background's standard deviation is 0.0066, so the encode is faithful. Found along the way and filed for round 2: a red cut face turned toward the sun washes out to pale pink (`capture_carnage` frame 380), and `ribbons` logs ten despawned-entity command warnings where round 1 logged six.
- **AG-044, measured and looked at.** The 19 viscera tests pass unchanged. None pins a spill's layout; they compare a spill with itself across runs, orders and seeds, and `rod_determinism` still prints one digest twice — `cea7d2e3a3eb8b5c` now, `3287f1ba16201294` before, because it hashes six strands laid by `spill`. `capture_entrails` moves from `0xca5e2f523e020bf8` to `0x1905015392eac10c`. Before and after, side by side:
  - `capture_entrails` frames 10 and 14: a fan of six rigid rods is now drooping loops hanging down the front of the body.
  - `capture_preset` frames 120, 130 and 359: a fan of sticks is now hanging loops, ending coiled on the floor.
  - `viscera_spill` frames 70 and 160: the ruler-straight strand on the floor is now a coil.

  `viscera.gif` (whose recipe is now in `docs/DEMOS.md`) and `preset.gif` are regenerated and looked at: `viscera.gif` frames 5, 20, 60 and 129, and `preset.gif` frames 60, 70, 100 and 179. In those, gut hangs in loops from the opened body, then the body comes down and the gut coils with the heap. `docs/viscera.md` names the new constructor and says why the spill drapes.
- **AG-046, measured and looked at.** The decals were checked for the same colour fault and do not have it: `floor_colour()` passes the encoded triple to `Color::srgb`, which converts it. The four `vfx` unit tests pass, and `capture_carnage` prints the same 289 wounds / 32 043 stains / `b540ac712d86e37a`, because particles reach no digest. Looked at against round 1:
  - `carnage` blast (f1110), cropped to the cloud at 160 %: hard-edged rotated pink squares are now round drops of varied size, deep red, the same family as the cut faces.
  - `carnage` slash (f590), whole frame: the drops stand out against the blue body and grey floor, and against the dark backdrop they read as darker spatter rather than vanishing.
  - `capture_carnage` 200: a deep-red round cloud over the heap where a pink square one was.
  - `capture_ribbons` 130 and `ribbons` f70: the strands draw dark red and are still visible.
  - `pooling` f200 is unchanged, as it should be.

  `carnage.gif` is regenerated from those frames and looked at in frames 30, 70, 100, 150 and 190. The floor stains stay lighter than the spray. Their colour path converts correctly, as above, so this ticket leaves them; why they read lighter is not established.
- **AG-047, measured and looked at.** A key list only; no example changed. The three examples re-run on the build host, and none logs "reached no proxy cell", where round 1 logged it on every repeat.
  - `pooling`, after each shot settled (f160, f420, f680, f940): 6, 15, 17 and 21 slicks, widest 0.115, 0.185, 0.208 and 0.239 m. The pools on the floor run together, and the entry wound widens shot by shot.
  - `bullet_holes` (f160 … f1720): five distinct holes, the proxy growing 16, 32, 52, 69 and 100 cells, each shot with the calibre, shatter, jaggedness and flare its key had just set. `R` returns it to an unbored subject.
  - `sever`'s new blows (f1760 to f2500): the marker sits on the right shoulder, then on the right thigh, as the arithmetic in `qa.sh` says it should.

  Reaching the limbs found two things for round 2, both at the coarsest granularity, where the body is its six parts. **A blade aimed at the right shoulder takes the head off and leaves the arm on.** `swept_triangle` breaks a bond when the segment joining the two fragments' centres crosses the blade. The torso-to-head segment runs straight up the middle and crosses a horizontal sweep at shoulder height, and the torso-to-arm segment sits below it. That reads wrong, and it is the crate's bond model, not the script. The blade at the thigh severs both legs and the body comes down, which is what a 1.8 m horizontal sweep at that height would do. Separately, `pooling`'s HUD counts 6, 36, 66 and 132 plugs thrown after its four shots, more than one shot's worth each time. `spawn_gore` throws only the tail past `Thrown`, so whether earlier plugs are being thrown again is not established.
- **AG-048, measured and looked at.** The see-through area, from above at 0.1 mm a pixel, with nothing removed rendering at 0.4 mm² of the rasteriser's own crack noise:

  | case | before | after |
  |---|---|---|
  | `laceration_open`'s 22 cm slash, 5 mm cells, tick 160 | 1 397 mm² | 4.9 mm² |
  | the same, ticks 3 to 30 (gape 0.5 to 4 mm) | 0 triangles removed: no opening at all | 99 removed, open from the first tick |
  | `capture_gore`'s thigh, 24 cells, tick 160 | 69 082 mm² | 602 mm² |

  What is left: under 3 mm² at the inside corner of the bend early in the opening, where the skin's edge between a vertex pushed along one segment and one pushed along the other leaves both troughs. A triangle between the two walls cannot cover that, which was tried and measured unchanged. Closing it would take a mitred bed. On the coarse patch it is hairline slivers where the skin's straight edge between two lip vertices cuts inside the walls' curve.

  `the_tear_is_frozen` is re-blessed, both halves, with the reason beside the constants. `laceration_curve` prints the new pair: skin `d8dff15d9bd83f00` (was `afb3d3cbbc85b028`) and bed `7baacbd6447efeb3` (was `18da2f8586e79831`), with 72 faces removed where it was 66. The new test `no_skin_is_left_across_a_diagonal_cut` fails on the old kernel ("a kept triangle … lies over the cut") and passes on this one. Every other tear and plugin test passes unchanged: the gap is empty, the tear is local, the floor is at depth, every bed face is wound to its normal, and removal is monotone in the gape. `capture_gore`'s laceration digest hashes transforms and stays `0534427ef2693376`. Looked at against round 1:
  - `laceration_open` f30 and f70: nothing visible before, a slit now. At f160, pits bridged by skin before, a continuous trough with a skin band at the lip and fat to the floor now. The f160 crop shows a closed tip and a seamless bend.
  - `capture_gore` 310, 340 and 419: the thigh opened as the same dashed row of pits at 310, which round 1 never noticed. Now it is a slit, then a full gash with ragged, rounded lips.

  `laceration.gif` is regenerated from those frames (recipe added to `docs/DEMOS.md`) and looked at in frames 5, 30, 70 and 119. `docs/laceration.md` describes the new clip and lip.
- **AG-049, measured and looked at.** Rendering only: every recorder digest is unchanged (`capture_carnage` 289 / 32 043 / `b540ac712d86e37a`, `capture_entrails` `0x1905015392eac10c`, `capture_pooling` `9df16f03c2bb2e0f`). The props render as before. The floor's mean in `capture_carnage` frame 200 is (89, 89, 97) before and (87, 87, 95) after, and (86, 87, 94) against (85, 85, 93) in `pooling`. The first try scaled the floor's linear albedo by exactly 2.83 and came out 13 % darker, because the constant specular share of a matte floor drops with the exposure and does not scale with albedo. Floor and skin are raised a little past 2.83 for it. Looked at against round 1, all eight recorder runs and six windowed examples on the shared scene:
  - `capture_carnage` 200 and 380, cropped to the floor: crimson stains on grey where there were pink ones. The cut faces are deep wet red, not bright red.
  - `pooling` f940: a dark red pool where there was pink paint.
  - `capture_demo` and `capture_audit` 70: the blues and greens are unchanged, the cut faces deeper.
  - `capture_holes` 150: dark holes and a crimson pool.
  - `capture_entrails` 14 and 200: deep red gut.
  - `capture_pooling` 230, `capture_ribbons` 130 and `capture_sever` 170.
  - Windowed `carnage` f590, `entrails` f330, `sever` f1980 and `bullet_holes` f1720.

  `explode.gif`, `fracture-tier-ab.gif`, `sever.gif`, `holes.gif`, `carnage.gif` and `viscera.gif` are regenerated from those frames and looked at, coalesced, at two frames each.

  Not fixed here. R1-06's shape half: the pools are still near-identical round blots with no darker wet core. The pale cut face in `capture_carnage` frame 380 is still pale, so it is the specular highlight of a glossy face turned to the sun, not exposure. Scenes that do not use `light_and_floor` keep their own lighting, among them `drying` (R1-07) and the preset scenes. And `capture_ribbons` frame 130 shows one strand standing straight up from a chunk, found while looking.
- **AG-050, measured and looked at.** The floor's mean is (106, 105, 111) before and (103, 103, 113) after. The first render, at 3.4 times the old linear albedo alone, came out at 95, short by the specular share. Pool A, fully dried at f460: (121, 88, 84) salmon before, (76, 53, 50) after, a dark brown crust under its craquelure. Pool A fresh at f70: (205, 127, 125) before, (141, 82, 81) after, a dark glossy red. Side by side at f70 and f460, oxyhaemoglobin red, then methaemoglobin, then hemichrome brown now read as three different colours. Before, they read as one pink going paler. The preset scenes, the other half of R1-07, are still lit as they were.
- **AG-051, measured and looked at.** Found while checking `AG-052`'s preset frames: at frame 60 the left leg was missing, and a pale line ran from the slab to the hip. **How it got past `AG-048`:** that ticket was looked at in `laceration_open` and `capture_gore`, both fine meshes, and never in the preset, the one caller that slashes a blockout. The new test `a_cut_across_a_face_coarser_than_the_influence_removes_nothing` cuts a one-cell patch. It fails on the `0.5.3` kernel ("a face with every corner beyond `influence` was clipped") and passes now. `the_tear_is_frozen` holds without a re-bless, because every triangle the frozen cut crosses is within its influence. So does every fine-mesh frame: `capture_gore`'s laceration clip is byte-identical to `AG-048`'s in all 120 frames, and so are `laceration_open`'s three shots. In `capture_preset` frames 60 and 110 the leg stands again, with its slash a dark red slit.
- **AG-052, measured and looked at.** `capture_preset` prints the same `af15ebc6c1c8033d`. Frames 60, 110, 120, 200 and 359: tan skin where it was nearly white, dark red wounds and a hanging gut, and dark crimson stains on a concrete slab where they were pink on pale grey. The first render kept the slab's 0.34 albedo, and the slab came out dark enough to flatten the stains. Windowed `preset` f330 and f720 and `carnage_web` f720 read the same way. R1-12 is not touched: the far side of the orbit is as dark as before, by construction, since the ambient was raised exactly as much as the exposure lowered it. `preset.gif` is regenerated and looked at, coalesced, at frames 15, 35, 60, 100 and 179.
- **AG-053, measured and looked at.** The four `vfx` unit tests pass. No recorder digest covers particles. Against `AG-049`'s frames, `ribbons` f330 and `capture_ribbons` 70: the same arcs, now read as blood trailing each chunk rather than hairlines, and still well under a chunk's size. A 3× crop of `capture_ribbons` 70 shows smooth arcs tapering to their tails, with no twisting. Only the two ribbon examples use the effect, and neither has a GIF. The long vertical strand in `capture_ribbons`, noted under `AG-049`, is a chunk thrown straight up, so its trail is vertical. It is not a defect.
- **AG-054, measured and looked at.** `ribbons` logs no WARN, against 10 in round 2's render and 7 in round 1's. `capture_ribbons` logs none either. All 21 shots render, and f330 and f590 match round 2's: the same strands on the same chunks, "24 of 24 ribbons live" and "11 of 24". The four `vfx` unit tests pass.
- **AG-055, measured and looked at.** Rendering only. Floors against round 2: `explode` (89, 89, 96) → (87, 87, 95), `spill` (69, 69, 74) → (66, 66, 73), `viscera_spill` (75, 75, 79) → (73, 73, 80). The first `spill` render, at the exposure factor alone, came out at 60 and 68 and was raised by the measured shortfall. `spill`'s gut, sampled at f200: (118, 65, 64) coral before, (86, 46, 46) blood red after. Cropped, `explode`'s large cut face at f160 goes from cherry red to deep wet red while the blue skin beside it reads the same, (123, 154, 183) against (121, 155, 185). `viscera_spill`'s gut darkens the same way at f200.
- **AG-056, measured.** With `QA_TIMEOUT=2` forcing the watchdog on `spill`, the run leaves `spill.hang.txt` (the `ps` line, state `R` at 2 s, and `sample`'s progress lines) and a 1.7 MB `spill.hang.txt.sample` with 31 thread stacks. Sampling a live `spill` directly over `ssh` works too. But `sample -file` writes its file whenever it succeeds, so the two real hangs leaving none means `sample` failed on them. A process stuck while exiting is one way that happens [INFERENCE]. The next hang records why, in `.hang.txt`.
- **AG-057, measured and looked at.** The six `decal` unit tests pass, and every digest prints as before (`capture_carnage` `b540ac712d86e37a`, `capture_pooling` `9df16f03c2bb2e0f`). The cost is 5 % of a stain's opacity on the floor itself: lifted 2 mm to clear z-fighting, it now paints at 95 % rather than 99.8 %. Looked at against round 2:
  - `carnage` f680, cropped to the feet: the sawtooth streaks along the foot edge are gone, leaving a few pixels of blood at the sole.
  - `carnage` f1200: the chunks lying in the pool show their own blue and red where they were washed pink, and the standing leg's lower third is solid.
  - `capture_carnage` 380: no pink wash on the pieces in the pool.
  - `pooling` f940: the pool is unchanged.

  `capture_holes` changes in 128 of its 160 frames. `capture_sever`, `capture_entrails` and `capture` draw no stains. `carnage.gif` and `holes.gif` are regenerated and looked at, coalesced, at frames 100 and 180 and at 80 and 150.
- **AG-058, measured and looked at.** `capture_preset` prints the same `af15ebc6c1c8033d`. At windowed `preset` f850, the orbit's back, the torso's two visible faces sample (81, 64, 55) and (84, 68, 60) before, three levels apart, and (104, 82, 71) and (123, 98, 85) after. The legs separate the same way. f550 lights the sheet's far side, and the front at f200 is unchanged. Every `capture_preset` frame differs in bytes, since the fill adds a little light even from the front. `preset.gif` is regenerated and looked at, coalesced, at frames 15, 60, 100 and 179. Not addressed: R1-13's sheet across the body, or R2-17's sheet that never catches blood. Moving the sheet front-left, where the spatter lands on the slab, was tried and reverted. It turned edge-on to the camera and still caught nothing. Where the spatter actually flies was not measured, so placing the sheet in it needs that first.
- **AG-059, measured and looked at.** Found in step 4.6's web pass, the first time the built site was opened in a browser. Served locally and loaded in headless Chromium, `carnage_web` and `viscera_spill` drew and ran with no console errors. `carnage_web` took three shots, a blow, an iron, a slash and an arterial, and came down into a banded heap. `laceration_open` drew nothing. Its page held two canvases: the black `#carnage-canvas` at y 125, and Bevy's own 1280 × 720 canvas at y 868, a child of `<body>`. `demo_facts.sh` passes with the fix and fails, naming the file, with `laceration_open`'s reverted. Rebuilt and reloaded, all fifteen demos have exactly one canvas, the page's, with no console errors, and every one draws its scene (a contact sheet of the fifteen, 12 s after load). Found on the way, for round 3: at the web page's 1280 × 720, `drying`'s and `rheology`'s top HUD blocks overlap each other into unreadable text, since they were laid out for the native windows.
- **AG-060, measured.** `grep -rnE 'bevy_(cross_section|fracture_modes|laceration|flaymap|wetmap|viscera)\b'` over `examples/` and `src/` prints nothing. No test matches on the warning text. `bloodstain` is still a module and keeps its name.
- **AG-061, measured.** `run_limited`, taken from the old and the new `qa.sh` and called 300 times on a command that exits in 0.05 s under a 30 s watchdog, reports **276** false timeouts with the old code and **0** with the new. A genuine timeout (`sleep 30` under 2 s) still returns 124 and leaves both `.hang.txt`, the `ps` line, and the stack sample. The timings agree: round 2's two "hangs" came from a job that ran two examples and a 2 m 34 s build in 190 s, and this ticket's from a `carnage` run inside a 274 s job that spent 246 s building. Neither ran anywhere near 180 s. Real windowed runs tripped the race about once in a hundred, not 276 times in 300; why the rate differs is not established. After the fix, six windowed examples ran with no timeout. Found while chasing R1-08, which stays open. A reset now clearing the plugin's `EffectTtl` sprays and pending `Wounded` messages was tried and reverted. Neither showed any measured effect: fresh drops still fell on the reset subject eight frames later, and the HUD counted six new slicks, from a source not identified.
- **AG-062, measured and looked at.** The 15 demos were rebuilt for wasm, served, and screenshot in headless Chromium 12 s after load. At a 1280-wide page the canvas is 950 × 534, and at 760 it is 718 × 404. At both widths `drying`'s and `rheology`'s blocks stand apart as in the native window, and each demo still has one canvas. Natively the scale stays 1: `drying`, `rheology` and `fragment_energy` give pixel-identical f30 and f160 to round 3's (`compare -metric AE` 0). At 718 the text is about 8 logical pixels, small but legible. That is the cost of keeping the layout; a phone-width page would need a layout of its own. Fitting makes the site match the native window, overlaps included: `fragment_energy`'s histogram over its readout (R1-40) and `gore_tier`'s flash meter over its status line (R1-36) appear in both, and stay open.
- **AG-063, looked at.** `gore_tier` and `fragment_energy` were rendered windowed on the same key lists as round 3, at f70 and f1460, and at f30 and f720. Cropped at full resolution, the three meter slots sit clear above "tier GrossViolence", separated by the column's 8 px gap, and fill red as before. The histogram's caption and bars sit clear of "tissue Cortical (G_c 2500 J/m2, rho 1900 kg/m3)", and every status line ends inside the window. At f720, after `H`, neither caption nor bars show, since both are children of the one `HistoPanel`. A follow-up caught one more overrun at f460: with `T` on trabecular, the crate's ceiling note was spliced into the target line, which then ran past the right edge and wrapped "bake produced 3 leaves" onto a line of its own. The note is now a line of its own, below the target line.
- **AG-064, looked at.** Rendered windowed on round 3's key lists. In `gore_tier` at f30 the whole figure, head to feet, stands between the legend's last line and the flash meter. At f70 every chunk of the kill is in frame and clear of both blocks, and at f160 the heap lies on the floor above the status, with each piece still reading as cut, a red face against blue. In `fragment_energy` at f30, f200, f460 and f720 the bone's top fragment sits below the legend and its foot above the status. The pieces are drawn about a fifth smaller than before. The screen marks and the aim cone are placed in NDC, so the camera does not move them.
- **AG-065, measured and looked at.** On a 40 px patch of that face at `capture_carnage` frame 260, the mean is (158, 132, 133) before, (114, 22, 22) with the specular switched off entirely, and (131, 80, 81) after. Two variants were tried on the way: water's reflectance at roughness 0.42 gave (141, 101, 102), and a clearcoat layer (1.0, roughness 0.12) over roughness 0.65 gave (151, 120, 121), with a bright band across the face. Re-rendered with `capture_demo`, `capture_audit`, `capture_sever`, `capture_holes`, `explode`, `fault_modes`, `fragment_energy`, `gore_tier`, `sever`, `bullet_holes` and `carnage`, at the frames where cut faces show most (`capture_sever` frame 120, `capture_demo` 49, `explode` f200), the faces keep a travelling highlight and read wet, and are a shade darker. `capture_carnage` still prints `b540ac712d86e37a`. `carnage.gif` is rebuilt from the new frames by the recipe; its GIF frames 130 and 149, counted from 0 (recorder frames 260 and 298), show the face dusky red.
- **AG-066, measured and looked at.** A temporary probe printed, every frame, the counts of sprays, decals, stains, pools, chunks and bleeding chunks, and the pending `Wounded` messages; a slash at f40 and `R` at f170 were scripted. Before, the reset frame left 0 decals, chunks and stains, and 93 sprays; one frame later a new spray appeared. Its drops were in the air at f172-f184 and lay on the floor at f200. The `AG-061` trial cleared the sprays and the messages but not the ordering, which is why it saw no change. After the fix there is no spray after the reset and no drop in f172, f176, f184 or f200. In the `qa.sh` run the f720 and f1240 floor is clean, against the cluster in round 3's frames. The blows at f330, f590 and f1110 spray as before. `pooling` and `ribbons`, which share `wipe`, ran their key lists without a warning beyond the known `R1-27` line.
- **AG-067, looked at.** Rendered windowed on round 3's key list, and `capture_pooling`. In `pooling` at f30, f160, f590 and f940, and in the recorder at frames 10, 100, 150 and 281, the whole figure stands in frame with the entry hole and the aim marker facing the camera. The plume leaves past the body's right side, and the slick lies beside the legs, unoccluded. A side view from (2.8, 1.6, 0.9) was tried first. It showed the plume and the slick as well, but put the entry hole edge-on, and the hole is the wound the demo makes. `capture_pooling` still prints `9df16f03c2bb2e0f`.
- **AG-068, looked at.** `preset` and `carnage_web` were rendered windowed on round 3's key lists, `capture_preset` as a clip, and the GIF frames checked. In `preset` at f200, f420, f550 and f720, and in `carnage_web` at f420 and f810, the body is lit on every visible face and the torso slit, the bruise, the burn and the thigh slash all face the camera. The sheet stands beside the body at both ends of the sweep, and the floor spatter is in frame. An earlier swing reaching 20° right put the sheet over the right arm again at f330 and f420, which is why the sweep stops at 5°. The first recorder angle tried, the sweep's start at 33° left, kept the body clear but cut most of the spatter off at the bottom, which is what the lower aim is for. `capture_preset` still prints `af15ebc6c1c8033d`. In `preset.gif` frames 25, 100 and 179, counted from 0, the wounded body, the heap and the spatter are all in frame, and the sheet never covers the body. Unchanged: the spatter still never reaches the sheet (`R2-17`).
- **AG-069, looked at.** Rendered windowed on round 3's key list. At f70, f160, f590 and f1200 the marks are irregular dark-red drops, each with one or two short tails pointing away from the centre, at 20-40 logical px; the HUD still reads "screen marks refused 8 of 14" at every tier. Two variants were tried on the way: keeping the old radii drew a speck of about 8 px, and 5-9 spines read as small spiky stars. The marks stay on screen after the body is gone, which is what screen blood on the lens is.
- **AG-070, looked at.** Rendered windowed on round 3's key list. At f160 and f1200 the stains in front of the body are filled dark-red drops, and at `Stylised` (f330) filled pale-yellow specks rather than rings. **`R1-37` stays open on size.** The stains are the spatter model's own, millimetres to a few centimetres across, and they land in front of the body: from the camera `AG-064` placed, that is under the status block and 2-8 px wide. So the substitute palette is visible at `Stylised` only on close inspection, and making it legible is a camera or a spray question, not this one.

### Rounds

| round | examples rendered | new defects | fixed | blocked | open |
|---|---|---|---|---|---|
| iter-01 | 12 recorders, 9 terminal, 24 windowed | 83 candidates, not yet re-inspected | 1 (R1-01, `AG-043`) | 0 | 82 |
| iter-02 (partial) | 24 windowed off screen; the recorders `AG-043`, `AG-044`, `AG-046` and `AG-048` touched; `pooling`, `bullet_holes` and `sever` rescripted; the 14 shared-scene examples after `AG-049`; `drying`, the three preset scenes and the two ribbon examples | 4: 2 found while fixing R1-01, 2 once `AG-047` reached the limbs (a fifth, the vertical strand under `AG-049`, was dismissed under `AG-053`) | R1-02 (`AG-044`), R1-03 (`AG-046`), R1-05 (`AG-047`), R1-04 (`AG-048`), R1-07 (`AG-050`, `AG-052`), R1-23 (`AG-053`); R1-06's colour half (`AG-049`); `AG-051`, a regression `AG-048` shipped in `0.5.3` | 0 | 80 |
| iter-03 (round 2) | all 45 surfaces at `f370414` (`AG-053`): 12 recorders, 9 terminal, 24 windowed; 0 missing, no hang | 27 candidates, 0 high, 9 medium, 18 low (`docs/visual-pass-round-2.md`); R2-05 and R2-09 are two of iter-02's four | 11 round-1 rows confirmed fixed and 2 partly by eleven inspection agents; R2-05 by `AG-054`, after the render | 0 | 100: 72 round-1 rows, 26 of round 2's, and iter-02's pale cut face and shoulder blade |
| iter-04 (round 3) | all 45 surfaces at `e137d9b` (`AG-064`): 12 recorders, 9 terminal, 24 windowed; 0 missing, no timeout | 9 candidates, 0 high, 1 medium, 8 low (`docs/visual-pass-round-3.md`) | 9 open rows confirmed fixed: R1-07, R1-18, R1-21, R1-36, R1-38, R1-61, R2-01, R2-05, R2-13; 14 partly | 0 | 102: 92 re-judged rows still open, R2-23 not re-judged, and the 9 new |
| iter-05 (round 4) | all 45 surfaces at `c77d32b` (`AG-070`): 12 recorders, 9 terminal, 24 windowed; 0 missing, no timeout | 5 candidates, 0 high, 1 medium, 4 low (`docs/visual-pass-round-4.md`) | 3 open rows confirmed fixed by every agent that judged them: R1-08 (`AG-066`), R1-14 (`AG-067`), R1-26; 14 partly, among them R1-12 and R1-13 (`AG-068`, fixed in `preset`, partly in `carnage_web`) and R3-01 (`AG-065`) | 0 | 104: 99 re-judged rows still open, and the 5 new |

**Round 1 found 83 candidates.** The work then moved to performance (`AG-039` … `AG-041`,
released as `0.5.1` and `0.5.2`, neither of which changed a pixel) and to the harness (`AG-042`).
The deduplicated list is `docs/visual-pass-round-1.md`. All five of its high-severity rows are
fixed:

- ~~the torso staying at standing height after the legs go (six examples)~~, fixed by `AG-043`;
- ~~viscera and blood strands launched as rigid body-length rods~~, fixed by `AG-044`;
- ~~`carnage`'s GPU spray drawn as pink square confetti~~, fixed by `AG-046`;
- ~~`laceration_open`'s cut opening as a dashed row of pits rather than a trough~~, fixed by `AG-048`;
- ~~`pooling`'s repeat shots never merging, because the autopilot never moves the aim~~, fixed by `AG-047`.

The harness defect that made all of this hard to see, windows presenting black, is gone with
`AG-042`: a round now runs on the build host with its screen locked.

**Round 2 found nothing high.** Its list is `docs/visual-pass-round-2.md`. The nine medium rows are
two scenes still lit three times too bright for their exposure (`explode`'s cut faces, the gut in
`spill` and `viscera_spill`), stains biting the feet of a body standing in them, the chunks thrown
highest trailing no ribbon, pools that stay separate where `DEMOS.md` promises one slick, a sheet the
preset says is spattered and never is, `rheology`'s later rods drawn thinner, a fracture-modes clip that
ends on a long still of a small heap, and the ribbons warnings `AG-054` has since removed.

**Round 3 found nothing high either.** Its list is `docs/visual-pass-round-3.md`. Of round 2's nine
medium rows, three are confirmed fixed (`explode`'s cut faces and the gut colour by `AG-055`, the
ribbons warnings by `AG-054`) and the stains at the feet partly (`AG-057`). The one new medium row is a
settled chunk in `capture_carnage` whose broad flat cut face catches the sun at the mirror angle and
reads as a pink tile for 160 frames.

**Round 4 found nothing high.** Its list is `docs/visual-pass-round-4.md`. Most open rows are unchanged
because no ticket touched them. It confirmed `AG-066` and `AG-067`, and `AG-068` in `preset`; it judged
`AG-065`'s face salmon rather than pink (partly), `AG-069`'s marks changed but still on screen, and
`R1-37` still tiny after `AG-070`. Of its five new rows the one medium is a `capture_ribbons` strand
hanging straight from the top edge after its chunk left the frame.

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
