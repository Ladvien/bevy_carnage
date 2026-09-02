//! The bake: walk a subject's loaded scene once, fracture it, and keep the result forever.
//!
//! This is the half that touches the ECS and the asset arena. Everything here exists to answer one
//! question — *what does this asset look like broken?* — exactly once per source asset, and to answer
//! it the same way on every machine.

use std::collections::{HashMap, HashSet};

use bevy::asset::AssetPath;
use bevy::prelude::*;

use crate::bore::Bore;
use crate::FractureSettings;
use crate::bond::BondGraph;
use crate::mesh::{FragmentSolid, append_mesh, geometry_from_piece, geometry_from_soup};
use crate::order::sort_total_by_key_at;
use crate::proxy::ProxyCell;
use crate::soup::{Soup, fracture};
use crate::tree::{FragmentId, FragmentTree};

/// **Marks an entity whose descendants should be pre-fractured, and names the asset to key that bake
/// by.** Put it on the root the scene hangs under; the bake walks `Children` from there.
///
/// The handle is the cache key *and* the seed source, so two subjects sharing one asset share one
/// bake — and swapping the asset needs no code change.
#[derive(Component)]
pub struct FractureSubject(pub Handle<WorldAsset>);

/// **The caller's convex decomposition of the subject, in subject-local space.**
///
/// Required alongside [`FractureSubject`]: this crate cuts a proxy, and computing a convex
/// decomposition is not its job — a consumer already running V-HACD or CoACD for colliders has one,
/// and forcing a second, different decomposition would be the fracture disagreeing with the physics
/// about what the object is. A blocked-out subject can build cells with [`ProxyCell::from_box`].
///
/// A subject with this component missing is `error!`-refused and never baked. That is deliberate: the
/// alternative is synthesising a bounding box and silently fracturing the wrong shape.
#[derive(Component)]
pub struct FractureProxy(pub Vec<ProxyCell>);

/// **Channels bored through this subject's proxy before it is cut** — bullet holes, baked in.
///
/// Read once, at bake time, exactly like [`FractureProxy`]: the bake is cached per asset id, so
/// adding a bore after the first bake does not re-bore that subject. A subject whose holes change
/// during play is a fresh bake, not an edit — [`crate::fracture_mesh`] is the pure path for that, and
/// it costs about the 2 ms `AG-011` measured.
#[derive(Component, Default, Clone, Debug)]
pub struct FractureBores(pub Vec<Bore>);

/// **Marks a subtree to be pruned out of the body and baked as one intact chunk**, keeping its own
/// material — a carried weapon, a hat, a backpack. The walk does not descend past it.
///
/// Present-but-empty is treated as "still streaming", never as "this subject has no detached part":
/// see the bake gate below for what that distinction cost.
#[derive(Component)]
pub struct DetachedPart;

/// One baked body fragment, in subject-**local** units (render scale is applied at spawn). Both
/// meshes are recentered to `center_local` (their shared bounding-box center), so a physics body
/// placed at `origin + center_local*scale` with a `half_extents*scale` box collider lines up exactly
/// with the rendered chunk. Either mesh may be `None` (a fragment with no cut faces has no cap; a
/// pure-cap sliver has no outer skin).
pub struct Fragment {
    /// Which node of this source's [`FragmentTree`] this is — and its own index in the array
    /// [`FractureCache::fragments`] returned.
    pub id: FragmentId,
    pub outer_mesh: Option<Handle<Mesh>>,
    pub cap_mesh: Option<Handle<Mesh>>,
    /// **The fragment as a solid: one convex cell.** This is what a solver wants — a single convex
    /// collider, no decomposition at spawn time and no trimesh. See `AG-007`.
    pub cell: ProxyCell,
    pub center_local: Vec3,
    /// Half the bounding box per axis (local units). **A coarse bound, not the collider** —
    /// [`Self::cell`] is the collider.
    pub half_extents: Vec3,
}

/// The pruned [`DetachedPart`], flung intact as a single tumbling chunk (it kept its own material).
/// Baked in the same subject-local space as body fragments.
pub struct DetachedChunk {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    pub center_local: Vec3,
    pub half_extents: Vec3,
}

/// **A plug a bore pushed out**, baked into mesh handles alongside the fragments.
///
/// The ECS mirror of [`crate::Ejecta`], and it exists so that setting [`FractureBores`] means the same
/// thing on both paths: a subject with baked-in channels also has the material those channels removed.
/// Two meshes, like a fragment, because the point is the contrast — the channel wall takes the
/// interior material and the two end patches keep the subject's own skin.
///
/// Spawn these at the same moment the body fragments are spawned; they are debris that already left,
/// so no frontier query returns one and no bond holds one on.
pub struct EjectaChunk {
    pub outer_mesh: Option<Handle<Mesh>>,
    pub cap_mesh: Option<Handle<Mesh>>,
    /// **The plug as a solid.** One convex cell, and its volume is exactly what the hole took.
    pub cell: ProxyCell,
    pub center_local: Vec3,
    pub half_extents: Vec3,
    /// Where the channel left the subject, subject-local.
    pub exit: Vec3,
    /// The channel's axis, unit — which way this was travelling. A geometric fact, not a velocity.
    pub direction: Vec3,
}

/// Baked fracture data, keyed by the subject's source scene asset id so multiple distinct subjects
/// each get their own bake and swapping the asset needs zero code change.
///
/// # Tier B is built on request, not at bake time
///
/// A fracture tree keeps every piece the cut loop split, so for `R` root cells and `T` leaves it
/// holds `2T − R` nodes and the interior ones are the upper levels — drawn only if someone asks for a
/// coarse frontier. Building their meshes eagerly uploaded a `Handle<Mesh>` per interior node into
/// `Assets<Mesh>` that nothing ever rendered, and cost about a sixth of the bake, because
/// [`crate::soup::soften`] scales with the triangle count it is handed.
///
/// So: [`solids`](Self::solids) is complete the moment the bake finishes, and a `Fragment` appears
/// only after [`request`](Self::request) has named its id and [`materialise_fragments`] has run.
/// `bake_fractures` requests the finest frontier itself, so the ordinary case needs no opt-in;
/// a caller wanting a coarser frontier asks a frame ahead.
///
/// **Materialising is a system, not a lazy read**, because `Assets<Mesh>` is only reachable from one.
#[derive(Resource, Default)]
pub struct FractureCache {
    body: HashMap<AssetId<WorldAsset>, Vec<Option<Fragment>>>,
    /// Tier A for every node, always complete after a bake.
    solids: HashMap<AssetId<WorldAsset>, Vec<FragmentSolid>>,
    /// Unmaterialised Tier B, index-parallel with `solids`. `None` once taken.
    pending: HashMap<AssetId<WorldAsset>, Vec<Option<crate::soup::Piece>>>,
    /// Ids a caller has asked to draw that are not built yet. Drained by [`materialise_fragments`].
    wanted: HashMap<AssetId<WorldAsset>, Vec<FragmentId>>,
    trees: HashMap<AssetId<WorldAsset>, FragmentTree>,
    graphs: HashMap<AssetId<WorldAsset>, BondGraph>,
    detached: HashMap<AssetId<WorldAsset>, DetachedChunk>,
    ejecta: HashMap<AssetId<WorldAsset>, Vec<EjectaChunk>>,
    baked: HashSet<AssetId<WorldAsset>>,
}

impl FractureCache {
    /// **Every** slot for a source, interior nodes of the hierarchy included, or `None` if that
    /// source hasn't been baked. A `None` *slot* means that node's Tier B has not been materialised.
    ///
    /// Index-parallel with [`tree`](Self::tree). Do not spawn this whole slice — it holds parents
    /// and their children both, and spawning both puts the same volume in the scene twice. Spawn a
    /// frontier: [`leaves`](Self::leaves) for the finest, [`frontier_of`](Self::frontier_of) for a
    /// chosen granularity.
    ///
    /// This is **not** a readiness test: the vector exists from the moment of the bake, before
    /// anything in it is drawn. Use [`is_baked`](Self::is_baked) for that question.
    pub fn fragments(&self, source: AssetId<WorldAsset>) -> Option<&[Option<Fragment>]> {
        self.body.get(&source).map(|v| v.as_slice())
    }

    /// **Tier A for every node** — the convex cells, present the instant the bake finishes and
    /// needing no mesh. `&[]` for a source that was never baked, matching
    /// [`ejecta`](Self::ejecta)'s never-`None` convention.
    pub fn solids(&self, source: AssetId<WorldAsset>) -> &[FragmentSolid] {
        self.solids.get(&source).map_or(&[], |v| v.as_slice())
    }

    /// **Ask for these nodes' drawn meshes.** They appear after [`materialise_fragments`] next runs,
    /// which is the same frame when the caller ran before it. A no-op for an unbaked source and for
    /// an id that is already drawn; duplicates are collapsed.
    pub fn request(&mut self, source: AssetId<WorldAsset>, ids: &[FragmentId]) {
        let Some(body) = self.body.get(&source) else { return };
        let queue = self.wanted.entry(source).or_default();
        for id in ids {
            if body.get(id.index()).is_none_or(Option::is_some) {
                continue; // out of range, or already drawn
            }
            if !queue.contains(id) {
                queue.push(*id);
            }
        }
    }

    /// Whether every one of these ids has a drawn mesh right now.
    pub fn ready(&self, source: AssetId<WorldAsset>, ids: &[FragmentId]) -> bool {
        let Some(body) = self.body.get(&source) else { return false };
        ids.iter().all(|id| body.get(id.index()).is_some_and(Option::is_some))
    }

    /// The fracture hierarchy for a source: which fragments nest inside which, and the frontier
    /// queries that read one bake at any granularity from the proxy cells up to the finest cut.
    pub fn tree(&self, source: AssetId<WorldAsset>) -> Option<&FragmentTree> {
        self.trees.get(&source)
    }

    /// Which of a source's finest fragments **touch** which, over how much shared face.
    ///
    /// Nesting and neighbouring are different questions: [`tree`](Self::tree) answers the first and
    /// this answers the second. Pair it with a [`BondSet`](crate::BondSet) the caller owns and
    /// [`BondGraph::islands`] to take one piece off and leave the rest standing.
    pub fn bonds(&self, source: AssetId<WorldAsset>) -> Option<&BondGraph> {
        self.graphs.get(&source)
    }

    /// The finest granularity — every fragment that was never cut further. **This is the set the
    /// cache handed out before it kept a hierarchy**, so a caller that just wants the old behaviour
    /// wants this.
    pub fn leaves(&self, source: AssetId<WorldAsset>) -> Vec<&Fragment> {
        self.pick(source, |t| t.leaves())
    }

    /// The frontier holding roughly `count` fragments, clamped to what this bake can offer — the
    /// granularity dial. Three pieces for a cleaving blow, all of them for a blast, from one bake.
    pub fn frontier_of(&self, source: AssetId<WorldAsset>, count: usize) -> Vec<&Fragment> {
        self.pick(source, |t| t.frontier_of(count))
    }

    /// The frontier at most `depth` cuts from the caller's proxy cells.
    pub fn at_depth(&self, source: AssetId<WorldAsset>, depth: u16) -> Vec<&Fragment> {
        self.pick(source, |t| t.at_depth(depth))
    }

    /// Resolve the ids a frontier query chose against this source's fragment array. An id outside
    /// the array is skipped rather than fatal, and so is one whose Tier B has not been built — ask
    /// for it with [`request`](Self::request) a frame ahead, or check [`ready`](Self::ready).
    fn pick<F>(&self, source: AssetId<WorldAsset>, choose: F) -> Vec<&Fragment>
    where
        F: FnOnce(&FragmentTree) -> Vec<FragmentId>,
    {
        let (Some(frags), Some(tree)) = (self.body.get(&source), self.trees.get(&source)) else {
            return Vec::new();
        };
        choose(tree).into_iter().filter_map(|id| frags.get(id.index())?.as_ref()).collect()
    }

    /// The baked [`DetachedPart`] chunk for a source, if any.
    pub fn detached_chunk(&self, source: AssetId<WorldAsset>) -> Option<&DetachedChunk> {
        self.detached.get(&source)
    }

    /// The plugs this source's baked-in [`FractureBores`] pushed out, if any.
    ///
    /// Empty rather than `None` for a subject with no bores: "this subject ejected nothing" and "this
    /// subject was never bored" are the same fact from a spawner's point of view, and both mean it has
    /// no debris to place.
    pub fn ejecta(&self, source: AssetId<WorldAsset>) -> &[EjectaChunk] {
        self.ejecta.get(&source).map_or(&[], |v| v.as_slice())
    }

    /// Has this source been baked at all? True even when the bake produced no fragments (a degenerate
    /// mesh), because "baked and empty" and "not yet baked" are different states and a caller waiting
    /// on the bake must be able to stop waiting.
    pub fn is_baked(&self, source: AssetId<WorldAsset>) -> bool {
        self.baked.contains(&source)
    }
}

/// Derive the per-source fracture seed from the asset's **path**.
///
/// **This used to hash the `AssetId`, and that was a real, expensive bug.**
///
/// ```ignore
/// fn seed_from(id: AssetId<WorldAsset>) -> u32 {
///     let mut h = DefaultHasher::new();
///     id.hash(&mut h);
///     h.finish() as u32
/// }
/// ```
///
/// An `AssetId` is a **slot index in the asset arena**, assigned by async load order — so the same
/// asset gets a different id run to run, hashes to a different seed, and `fracture` slices the body
/// along **completely different planes**. Measured before the fix: two same-seed builds produced
/// **23 of 23 fragments differing**, in `half_extents` as well as `center_local` — the
/// mesh was being partitioned differently, not merely rounded differently. Every downstream symptom
/// (chunk positions differing by ULPs, the load-dependence that made it look like a race) follows from
/// the fracture planes moving.
///
/// The old doc comment said "deterministic **within a run**", which was true and was the tell — nothing
/// compared two runs' bakes until it did.
///
/// The asset **path** is the stable identity: it is authored, not allocated, and identical across runs,
/// processes and machines. Hashed with a hand-rolled FNV-1a because `DefaultHasher` is not guaranteed
/// stable across toolchains, so it has no business seeding anything whose output is compared between
/// builds.
fn seed_from_path(path: &AssetPath) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in path.to_string().as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Turn one finished piece into cached mesh handles. Total, like [`geometry_from_piece`]: the
/// fragment array is index-parallel with the hierarchy, so a piece that draws nothing still occupies
/// its slot — with no meshes, and still a usable convex collider.
fn build_fragment(id: FragmentId, piece: crate::soup::Piece, meshes: &mut Assets<Mesh>) -> Fragment {
    let g = geometry_from_piece(id, piece);
    Fragment {
        id: g.id,
        outer_mesh: g.outer.map(|m| meshes.add(m)),
        cap_mesh: g.cap.map(|m| meshes.add(m)),
        cell: g.cell,
        center_local: g.center_local,
        half_extents: g.half_extents,
    }
}

/// Turn one ejected plug into cached mesh handles — the same shape as [`build_fragment`], minus the
/// tree id it deliberately does not have.
fn build_ejecta(e: crate::soup::Ejected, meshes: &mut Assets<Mesh>) -> EjectaChunk {
    let g = crate::mesh::ejecta_from_piece(e);
    EjectaChunk {
        outer_mesh: g.outer.map(|m| meshes.add(m)),
        cap_mesh: g.cap.map(|m| meshes.add(m)),
        cell: g.cell,
        center_local: g.center_local,
        half_extents: g.half_extents,
        exit: g.exit,
        direction: g.direction,
    }
}

/// Bake the pruned part into a single intact chunk (no fracture), keeping its own material.
/// `None` if its soup is empty or it had no material.
fn bake_detached(
    part: &Soup,
    material: Option<Handle<StandardMaterial>>,
    meshes: &mut Assets<Mesh>,
) -> Option<DetachedChunk> {
    let g = geometry_from_soup(part)?;
    let material = material?;
    let mesh = g.outer.or(g.cap).map(|m| meshes.add(m))?;
    Some(DetachedChunk { mesh, material, center_local: g.center_local, half_extents: g.half_extents })
}

/// Once a subject's whole scene has streamed in, bake its fracture set (and its detached chunk)
/// exactly once per source. Walk the subject's descendants, prune the [`DetachedPart`] subtree into a
/// separate chunk, merge the rest into one soup in subject-local space, and fracture. Self-gates on
/// all sub-meshes being present in `Assets<Mesh>`.
pub fn bake_fractures(
    mut cache: ResMut<FractureCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    settings: Res<FractureSettings>,
    subjects: Query<(&FractureSubject, &FractureProxy, &Children, Option<&FractureBores>)>,
    children_q: Query<&Children>,
    transforms: Query<&Transform>,
    mesh_q: Query<&Mesh3d>,
    mat_q: Query<&MeshMaterial3d<StandardMaterial>>,
    is_detached: Query<(), With<DetachedPart>>,
) {
    for (subject, proxy, children, bores) in &subjects {
        let source = subject.0.id();
        if cache.baked.contains(&source) {
            continue;
        }
        // The fracture seed comes from the asset PATH, never the `AssetId` — see `seed_from_path`.
        // One path, no fallback: a handle with no asset path cannot be baked reproducibly, so it is
        // not baked at all, loudly.
        let Some(asset_path) = subject.0.path().map(|p| p.clone_owned()) else {
            error!(
                "carnage: a FractureSubject handle has no asset path — refusing to bake a fracture \
                 whose seed would depend on asset load order. No fragments for this source."
            );
            continue;
        };

        let mut body = Soup::default();
        let mut part = Soup::default();
        let mut part_material: Option<Handle<StandardMaterial>> = None;
        let mut all_loaded = true;

        // DFS stack of (entity, transform-relative-to-subject-root, inside-detached-subtree).
        let mut stack: Vec<(Entity, Mat4, bool)> = Vec::new();
        for child in children.iter() {
            let m = transforms.get(child).map(|t| t.to_matrix()).unwrap_or(Mat4::IDENTITY);
            stack.push((child, m, is_detached.get(child).is_ok()));
        }

        // Collect the meshes first, then append them in a CANONICAL order.
        //
        // Appending during the walk was a real determinism bug, and a subtle one because nothing
        // downstream hashed it directly. `Children` order for a glTF scene is the order the async
        // instantiation happened to add nodes, which is wall-clock dependent — so the vertex soup was
        // assembled in a different order between two same-seed runs. `fracture` then computes fragment
        // centroids as float sums over that soup, and float addition is not associative, so
        // `Fragment::center_local` came out a few ULPs apart. Every chunk spawns at
        // `origin + center_local * scale`, so the *positions* of an otherwise identical chunk set
        // diverged: same count, same order, coordinates off in the last few bits.
        //
        // The key is `(mesh asset PATH, world-matrix bits)`; the matrix disambiguates two entities that
        // share one mesh datablock at different transforms. Deliberately NOT the `Entity` id — id
        // allocation order is the instability being erased here.
        //
        // **It used to be the mesh's `AssetId`, and that was the same bug as `seed_from_path`'s, ninety
        // lines above.** The comment there even stated the assumption — "the asset id is stable across
        // same-seed runs (measured)" — and an `AssetId` is an *arena slot assigned by async load order
        // and slot recycling*, which is precisely what the seed was condemned for hashing. The
        // measurement behind that claim was taken idle; the residual only reproduced under heavy load.
        //
        // Note the tie check proves the key is *unique*, which is not the same as **stable** — a unique
        // key drawn from a load-order-dependent allocator still permutes the list. Uniqueness was never
        // the property this needed.
        //
        // A path is authored rather than allocated, so it is identical across runs, processes and
        // machines. glTF sub-meshes are path-backed (`enemy.glb#Mesh0/Primitive0`).
        let mut parts: Vec<(String, [u32; 16], Mat4, bool, Entity, Handle<Mesh>)> = Vec::new();
        let mut unpathed_mesh = false;
        while let Some((e, mat, in_part)) = stack.pop() {
            if let Ok(mesh3d) = mesh_q.get(e) {
                if meshes.get(&mesh3d.0).is_some() {
                    let mut bits = [0u32; 16];
                    for (i, v) in mat.to_cols_array().iter().enumerate() {
                        bits[i] = v.to_bits();
                    }
                    match mesh3d.0.path() {
                        Some(path) => {
                            parts.push((path.to_string(), bits, mat, in_part, e, mesh3d.0.clone()))
                        }
                        // One path, no fallback. Falling back to the `AssetId` for this one mesh would
                        // reintroduce exactly the instability this key exists to remove, on a subset of
                        // the soup — which is worse than not baking, because it would be intermittent.
                        None => unpathed_mesh = true,
                    }
                } else {
                    all_loaded = false; // sub-mesh still streaming
                }
            }
            if let Ok(ch) = children_q.get(e) {
                for child in ch.iter() {
                    let ct = transforms.get(child).map(|t| t.to_matrix()).unwrap_or(Mat4::IDENTITY);
                    let child_part = in_part || is_detached.get(child).is_ok();
                    stack.push((child, mat * ct, child_part));
                }
            }
        }
        if unpathed_mesh {
            error!(
                "carnage: a sub-mesh of {asset_path} has no asset path — refusing to assemble a vertex \
                 soup whose order would depend on asset load order. No fragments for this source."
            );
            continue;
        }
        sort_total_by_key_at(
            concat!(file!(), ":", line!()),
            &mut parts,
            |p: &(String, [u32; 16], Mat4, bool, Entity, Handle<Mesh>)| (p.0.clone(), p.1),
        );
        for (_, _, mat, in_part, e, mesh_handle) in parts {
            let Some(m) = meshes.get(&mesh_handle) else { continue };
            if in_part {
                append_mesh(&mut part, m, mat, false);
                if part_material.is_none() {
                    part_material = mat_q.get(e).ok().map(|mm| mm.0.clone());
                }
            } else {
                append_mesh(&mut body, m, mat, false);
            }
        }

        // Wait until the async scene has actually instantiated its body meshes AND they're loaded into
        // `Assets<Mesh>`. Before a glTF scene spawns its descendants there are simply no body `Mesh3d`
        // entities to find, so an empty body here means "still streaming", not "no geometry" — retry
        // next frame rather than caching an empty fracture set.
        //
        // **`part.is_empty()` is part of that gate, and leaving it out was a determinism bug.** When the
        // held item is a SEPARATE scene from the body, it streams on its own schedule. The
        // `DetachedPart` entity exists immediately, but until its scene instantiates it has no `Mesh3d`
        // descendants to find — so `all_loaded` stays true (nothing unloaded was *seen*) and `body` is
        // non-empty, and this baked a source with an EMPTY detached chunk and then marked it `baked`
        // **permanently**. Whether that race was won decided, for the whole run, whether a death flung
        // the weapon at all. If anything downstream is a fixed-size pool or a numbered sequence, losing
        // one chunk shifts every later one. Measured: 11 of 12 runs had the chunk and 1 did not.
        //
        // Empty part ⇒ "still streaming", never "this subject has no detached part". If a subject
        // without one is ever supported, this gate must learn to tell "absent" from "not yet", not be
        // relaxed back.
        if !all_loaded || body.is_empty() || part.is_empty() {
            continue;
        }

        let ext = body.extent();
        if ext <= 1.0e-5 {
            warn!("carnage: source body is degenerate (zero extent); marking baked with no fragments");
            cache.body.insert(source, Vec::new());
            cache.baked.insert(source);
            continue;
        }
        // **Refused, not substituted.** Synthesising a bounding box here would fracture a shape the
        // subject is not, and would do it silently. `baked` is left unset so a caller that adds the
        // component later still gets a bake.
        if proxy.0.is_empty() {
            error!("carnage: {asset_path} has no FractureProxy cells; refusing to bake");
            continue;
        }

        // Bounding-box-driven sizing: bigger/denser meshes yield more, appropriately-sized pieces.
        let ref_ext = settings.ref_extent.max(1.0e-4);
        let raw = (settings.pieces_base as f32 * (ext / ref_ext)).round() as i32;
        let target = raw.clamp(settings.min_pieces, settings.max_pieces).max(1) as usize;

        // **The bake runs here, on the main thread, and `AG-011` settled that by measuring rather than
        // arguing.** The torso-and-head fixture at its finest 12 fragments measures **~2.2 ms**
        // (release, `cargo run --release --example fracture_cube`), up from ~1.4 ms before the bake
        // kept its hierarchy — the ratio tracks node count, 23 nodes built instead of 12, which is
        // what keeping every piece the loop split costs. The ticket's own threshold was "a fix is
        // warranted at 50 ms and not at 5 ms", so this stays well the safe side of it.
        //
        // The figure recorded here was **0.33 ms**, and re-measuring found that stale on this
        // machine even before the change: the pre-hierarchy code measures ~1.4 ms today. Both
        // numbers are below the threshold, so the conclusion is unchanged — but the old one was
        // being quoted as if it had been re-checked, and it had not.
        //
        // Recording the alternative so nobody re-derives it: moving this to `AsyncComputeTaskPool`
        // would need `bevy/multi_threaded`, which this crate deliberately does not declare. Without it
        // that pool is single-threaded and `spawn` runs the work inline anyway — so the "async" bake
        // would be async only in builds where some *other* crate happened to turn the feature on, via
        // feature unification. One code path that is concurrent in some consumers' builds and not
        // others is exactly the ambiguity `CLAUDE.md`'s one-path rule exists to prevent, and buying it
        // for 0.33 ms would be a bad trade twice over.
        let bores = bores.map(|b| b.0.clone()).unwrap_or_default();
        let (pieces, tree, ejected) =
            fracture(body, &proxy.0, &settings.cut_for(target, seed_from_path(&asset_path), bores));
        let graph = crate::mesh::bond_graph(&pieces, &tree);
        // **Tier A for every node, Tier B for none — yet.** The interior nodes of the hierarchy are
        // only drawn if a caller asks for a coarse frontier, so building their meshes here uploaded
        // handles nothing rendered. See [`FractureCache`].
        let solids: Vec<FragmentSolid> = pieces
            .iter()
            .enumerate()
            .map(|(i, p)| FragmentSolid { id: FragmentId(i as u32), cell: p.cell.clone() })
            .collect();
        let node_count = solids.len();
        let plugs: Vec<EjectaChunk> =
            ejected.into_iter().map(|e| build_ejecta(e, &mut meshes)).collect();
        info!(
            "carnage: baked {} fragments for {asset_path} ({} in the finest frontier, {} cuts, \
             {} bonds, {} ejected plug(s))",
            node_count,
            tree.leaves().len(),
            tree.cuts(),
            graph.len(),
            plugs.len()
        );
        let leaves = tree.leaves();
        cache.body.insert(source, (0..node_count).map(|_| None).collect());
        cache.solids.insert(source, solids);
        cache.pending.insert(source, pieces.into_iter().map(Some).collect());
        cache.trees.insert(source, tree);
        cache.graphs.insert(source, graph);
        cache.ejecta.insert(source, plugs);
        // The common case, satisfied without anyone opting in: the finest frontier is what a death
        // spawns, and `materialise_fragments` is chained straight after this system, so it is drawn
        // on this same frame.
        cache.request(source, &leaves);

        // The detached chunk (single intact piece, keeps its own material).
        if let Some(chunk) = bake_detached(&part, part_material, &mut meshes) {
            cache.detached.insert(source, chunk);
        }

        cache.baked.insert(source);
    }
}

/// **Build the Tier B meshes somebody asked for.** Chained straight after [`bake_fractures`], so a
/// bake and a request that happen on the same frame are drawn on that frame — otherwise the first
/// frame after a death would spawn nothing.
///
/// A system rather than a lazy read inside [`FractureCache`], and that is forced rather than chosen:
/// `Assets<Mesh>` is only reachable through `ResMut`, so a fragment reached from a `&FractureCache`
/// could not add a mesh asset at all. `OnceCell` is out for the same family of reasons — the cache is
/// a `Resource` and therefore `Send + Sync`, and a `Mutex` would put a lock in the bake's hot path.
pub fn materialise_fragments(mut cache: ResMut<FractureCache>, mut meshes: ResMut<Assets<Mesh>>) {
    let sources: Vec<AssetId<WorldAsset>> =
        cache.wanted.iter().filter(|(_, ids)| !ids.is_empty()).map(|(s, _)| *s).collect();
    for source in sources {
        let Some(ids) = cache.wanted.remove(&source) else { continue };
        // **Both maps are taken out and both are put back, unconditionally.** They have to leave the
        // resource so `build_fragment` can hold `&mut Assets<Mesh>` at the same time; taking one and
        // bailing on the other missing would delete a source's whole Tier B and leave it
        // permanently unmaterialisable, with `is_baked` still true. A bake writes both together, so
        // the pair is always present in practice — the point is that this cannot be the thing that
        // breaks it if that ever stops being so.
        let mut pending = cache.pending.remove(&source).unwrap_or_default();
        let mut body = cache.body.remove(&source).unwrap_or_default();
        for id in ids {
            let i = id.index();
            if body.get(i).is_none_or(Option::is_some) {
                continue; // out of range, or drawn since the request
            }
            let Some(piece) = pending.get_mut(i).and_then(Option::take) else { continue };
            body[i] = Some(build_fragment(id, piece, &mut meshes));
        }
        cache.pending.insert(source, pending);
        cache.body.insert(source, body);
    }
}
