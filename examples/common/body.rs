//! **The subject `sever` takes apart, and the rules for taking it apart.**
//!
//! Shared by the interactive example and by the recorder that turns it into a GIF, so the animation
//! in the README is the same code you can play with rather than a re-implementation of it. A
//! recorder that reimplements its subject drifts from it silently, and the drift is invisible in
//! exactly the place you would look for it — the picture.
//!
//! Everything here works on `&mut World` rather than on `Commands`, because that is the shape both
//! callers can drive: a Bevy system can take `&mut World`, and a hand-pumped headless loop has
//! nothing else.
//!
//! **None of this is in the crate.** `bevy_carnage` hands out a reach — a severity per bond — and
//! everything below picks the threshold at which one gives way, decides which island is still "the
//! body", and throws the rest.

#![allow(dead_code)]

use std::collections::HashSet;

use bevy::prelude::*;
use bevy_carnage::{
    Bore, BondGraph, BondSet, CarnageSettings, CutSettings, FragmentId, FragmentTree, Pool,
    PoolDecal, ProxyCell, Reach, SplatTextures, Stain, absorb, capsule, directional, fracture_mesh,
    hash_f32, radial, spawn_pool, spread, spread_pools, swept_triangle, update_pool_decals,
};

use super::material;

/// Finest fragment count. Higher than `explode.rs`'s because a localised hit should be able to take
/// something *small* off — at a dozen pieces every hit removes a quarter of the body.
pub const TARGET: usize = 34;
/// Stop cutting a piece below this fraction of the whole solid's extent.
pub const MIN_FRACTION: f32 = 0.08;
/// How many cuts deep the hierarchy may go.
pub const MAX_DEPTH: u16 = 64;
/// The seed. Same seed, same pieces, every run — which is what makes the GIF diffable.
pub const SEED: u32 = 0x00C0_FFEE;

/// **The line between "reached" and "severed", and it lives here rather than in the crate.**
/// A game would scale each bond's severity by what the thing is made of and how much damage the blow
/// carried before comparing; these examples take the reach at face value.
pub const GIVES_WAY: f32 = 0.5;

/// Where the subject stands: feet on the floor. The lowest point is the leg bottom at `y = -0.92`.
pub const ORIGIN: Vec3 = Vec3::new(0.0, 0.92, 0.0);

/// The rounding strengths the `T` key cycles through. Changing this re-bakes, because the softening
/// happens when the drawn mesh is built — it is not something a shader can be toggled.
pub const SOFTENINGS: [f32; 4] = [0.0, 0.25, 0.5, 0.75];

/// The piece counts the granularity dial cycles through. One bake answers all of them.
///
/// **The first entry is the six body parts**, because `frontier_of(6)` on a six-cell proxy is the
/// roots — uncut. That is dismemberment; the last entry is gibs; the two in between are the range.
pub const GRANULARITIES: [usize; 4] = [6, 12, 20, TARGET];

/// **Where the demo's shots land**: `(frame, entry point, radius, shatter)`, subject-local.
///
/// Front to back along `-z`, because the camera sits at `+z` and the entry hole is the thing worth
/// seeing. Radii are about a fortieth of the subject's height — a bullet, not a cannon — and every
/// one is comfortably above `bevy_carnage`'s own minimum bore radius.
///
/// **The `shatter` column climbs on purpose**, 3 → 8 across the five shots. A plug is one convex
/// prism, so ejected whole it reads as a dowel — the channel was cut by something corer-shaped and
/// the material it removed looks cored. The clip walks up the dial so the difference between a few
/// big chunks and a spray is visible in one recording rather than described.
pub const SHOTS: [(u32, Vec3, f32, u32); 5] = [
    (12, Vec3::new(0.06, 0.14, 0.0), 0.035, 3),   // torso, high right
    (32, Vec3::new(-0.09, -0.05, 0.0), 0.035, 4), // torso, low left
    (52, Vec3::new(0.00, 0.46, 0.0), 0.030, 5),   // head
    (72, Vec3::new(0.14, -0.18, 0.0), 0.035, 6),  // torso, low right
    (92, Vec3::new(-0.32, 0.06, 0.0), 0.030, 8),  // through the left arm
];

/// One shot as a bore straight through the subject, entering at `+z`, its plug breaking into
/// `shatter` pieces.
///
/// The segment is longer than the subject is deep on purpose: a `Bore` *is* its segment, so a shot
/// that goes clean through is one that starts and ends outside the solid. Ending it inside would
/// make the far end a pit floor instead, which is the other thing the same type says.
pub fn bore_at(at: Vec3, radius: f32, shatter: u32) -> Bore {
    Bore { shatter, ..Bore::new(at + Vec3::Z * 0.6, at - Vec3::Z * 0.6, radius) }
}

pub const GRAVITY: f32 = 18.0;
pub const RESTITUTION: f32 = 0.3;
pub const GROUND_DRAG: f32 = 4.0;

/// **A blocked-out humanoid: one convex cell per body part.**
///
/// This replaced a torso box and a head box, and the reason is the whole point of the crate. Cutting
/// a limbless mass with pseudorandom planes produces wedges sliced diagonally out of a blob — which
/// reads as a frozen statue shattering, however good the fracture is, because none of the pieces is
/// a *part of a body*. Nothing tuned in the cutter fixes that; the subject had no anatomy to break
/// along.
///
/// **The joints come for free, and that is not a coincidence.** A joint is two body parts meeting
/// over a shared surface, and [`BondGraph::of`](bevy_carnage::BondGraph::of) already finds exactly
/// that — coplanar faces, opposite normals, positive overlap. Laid out this way the bond graph comes
/// back with one bond per joint, its area the joint's own cross-section:
///
/// ```text
/// torso <-> head    area 0.0676   the neck
/// torso <-> arm.L   area 0.1040   the shoulder
/// torso <-> leg.L   area 0.0528   the hip
/// ```
///
/// So a hit on the shoulder takes off the arm, at every granularity, with no code that knows what an
/// arm is. Read the bake at [`GRANULARITIES`]`[0]` and the pieces *are* the body parts; read it at
/// the finest and they are gibs.
///
/// # Two placements that are load-bearing
///
/// **Every part touches its neighbour exactly, never overlapping.** Cells that interpenetrate share
/// no coplanar face and get no bond, so an overlapping limb would hang off its own island and drop
/// at the first blow anywhere. Each part's face sits *on* the torso's, not inside it.
///
/// **The legs are held apart by 0.04.** Touching at `x = 0` they would share a coplanar face and the
/// graph would correctly — and uselessly — bond the legs to each other, so severing a hip would
/// leave a leg dangling from its twin. Anatomy is the caller's business, and this is what that costs.
pub fn subject() -> Vec<(Mesh, Mat4)> {
    parts()
        .into_iter()
        .map(|(_, c, h)| (Mesh::from(Cuboid::new(h.x * 2.0, h.y * 2.0, h.z * 2.0)), Mat4::from_translation(c)))
        .collect()
}

/// One convex cell per body part — the caller's decomposition, matching [`subject`] exactly.
pub fn proxy() -> Vec<ProxyCell> {
    parts().into_iter().map(|(_, c, h)| ProxyCell::from_box(c, h)).collect()
}

/// The blockout: `(name, centre, half-extents)`, in subject-local space. The name is for logging —
/// nothing in the crate ever sees it, and nothing here branches on it.
pub fn parts() -> Vec<(&'static str, Vec3, Vec3)> {
    vec![
        ("torso", Vec3::new(0.00, 0.00, 0.0), Vec3::new(0.22, 0.32, 0.14)),
        ("head", Vec3::new(0.00, 0.46, 0.0), Vec3::new(0.13, 0.14, 0.13)),
        ("arm.L", Vec3::new(-0.32, 0.06, 0.0), Vec3::new(0.10, 0.26, 0.10)),
        ("arm.R", Vec3::new(0.32, 0.06, 0.0), Vec3::new(0.10, 0.26, 0.10)),
        ("leg.L", Vec3::new(-0.13, -0.62, 0.0), Vec3::new(0.11, 0.30, 0.12)),
        ("leg.R", Vec3::new(0.13, -0.62, 0.0), Vec3::new(0.11, 0.30, 0.12)),
    ]
}

/// Which body part a fragment came out of, by walking the hierarchy back to its root cell.
pub fn part_of(id: FragmentId, tree: &FragmentTree) -> &'static str {
    let root = tree.root_of(id).unwrap_or(id);
    parts().get(root.index()).map(|(n, _, _)| *n).unwrap_or("?")
}

/// What one baked fragment needs to be spawned, resolved once so a reset costs no geometry work.
pub struct Part {
    pub outer: Option<Handle<Mesh>>,
    pub cap: Option<Handle<Mesh>>,
    pub center_local: Vec3,
    /// How far the piece's lowest point sits below its centre, from the cell rather than the bound.
    pub drop_to_rest: f32,
    /// **Kept, because adjacency is per frontier.** A game hands this to `Collider::convex_hull`;
    /// here it is also what `BondGraph::of` needs to bond whichever frontier is standing.
    pub cell: ProxyCell,
    /// The fragment's volume, which is its mass at uniform density. Drives how fast it is thrown.
    pub volume: f32,
}

/// **The bake, and it happens once.** Everything a blow does is a query against this.
#[derive(Resource)]
pub struct Baked {
    pub tree: FragmentTree,
    /// Indexed by [`FragmentId`], parallel to the tree. `None` for a node whose Tier B was never
    /// asked for — see [`Baked::bake`], which materialises only the granularities the demo offers.
    pub parts: Vec<Option<Part>>,
    /// **What the bores pushed out**, in the order the shots were fired. Empty with no bores.
    ///
    /// Off the tree and off the bond graph, because the crate keeps it off both — so nothing here has
    /// to remember that a plug is not a frontier piece.
    pub gore: Vec<GorePart>,
}

/// One ejected plug, resolved to handles at bake time exactly like a [`Part`].
///
/// **Not a fragment, and the crate refuses to let it be one** — see `bevy_carnage::Ejecta`. It is the
/// material a channel removed, so it is already outside the subject the moment it exists.
pub struct GorePart {
    pub outer: Option<Handle<Mesh>>,
    pub cap: Option<Handle<Mesh>>,
    pub center_local: Vec3,
    pub drop_to_rest: f32,
    /// The plug's volume — its mass at uniform density, and the size of the pool it leaves.
    pub volume: f32,
    pub exit: Vec3,
    /// The channel's axis, unit. The crate hands this over as geometry; turning it into a velocity
    /// is this file's job, like every other launch here.
    pub direction: Vec3,
    /// **The plug as a solid, kept for the same reason [`Part`] keeps its own.**
    ///
    /// A `Part` keeps its cell because adjacency is per frontier; this keeps its cell because the
    /// *wound* a channel leaves is measured off it — `wound_of_channel` sums the plug's raw-interior
    /// faces, which is the channel wall. Approximating that from `volume` was tried and is exactly
    /// the kind of invented number this repository refuses: the cell is right here.
    pub cell: ProxyCell,
}

impl Baked {
    /// Fracture the subject and register every fragment's meshes.
    ///
    /// `bores` are channels subtracted from the proxy before any cut — see [`bore_at`]. Taking them
    /// here rather than in each caller is what keeps the windowed demo and the recorder honest: there
    /// is one bake definition, so a hole in the GIF is a hole you can reproduce by keypress.
    pub fn bake(world: &mut World, soften: f32, bores: &[Bore]) -> Baked {
        let owned = subject();
        let parts: Vec<(&Mesh, Mat4)> = owned.iter().map(|(m, x)| (m, *x)).collect();
        let cut = CutSettings {
            max_depth: MAX_DEPTH,
            soften,
            bores: bores.to_vec(),
            ..CutSettings::new(TARGET, MIN_FRACTION, SEED)
        };
        let mut baked = fracture_mesh(&parts, &proxy(), &cut);

        info!(
            "baked {} fragments ({} finest, {} cuts) with {} bonds, and {} ejected plug(s)",
            baked.len(),
            baked.tree.leaves().len(),
            baked.tree.cuts(),
            baked.bonds.len(),
            baked.ejecta.len()
        );
        info!("soften {soften:.2} — rounding the drawn surface only; the colliders are unchanged");

        // **Only the frontiers this demo can ever stand at get meshes.** The tree keeps every piece
        // the cut loop split, and the interior levels are pure waste unless something draws them — so
        // ask for exactly the four granularities the `G` key cycles plus the leaves, and leave the
        // rest unmaterialised. The union, not just the leaves: `frontier_of` legitimately returns
        // interior ids, which is the whole point of the granularity dial.
        let mut wanted: Vec<FragmentId> =
            GRANULARITIES.iter().flat_map(|g| baked.tree.frontier_of(*g)).collect();
        wanted.extend(baked.tree.leaves());
        // SORT-OK: by tree index, which is unique per node; the dedup below is the whole purpose.
        wanted.sort_unstable_by_key(|id| id.index());
        wanted.dedup();

        let node_count = baked.len();
        // Taken out before `into_pick` consumes the bake. Both are whole-bake facts that no frontier
        // query touches.
        let tree = std::mem::take(&mut baked.tree);
        let ejecta = std::mem::take(&mut baked.ejecta);

        let gore = ejecta
            .into_iter()
            .map(|e| {
                let lowest = e.cell.points().iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
                let meshes = &mut world.resource_mut::<Assets<Mesh>>();
                GorePart {
                    outer: e.outer.map(|m| meshes.add(m)),
                    cap: e.cap.map(|m| meshes.add(m)),
                    center_local: e.center_local,
                    drop_to_rest: (e.cell.center().y - lowest).max(0.0),
                    volume: e.cell.volume().max(1.0e-6),
                    exit: e.exit,
                    direction: e.direction,
                    cell: e.cell,
                }
            })
            .collect();

        // Index-parallel with the tree still, so `parts.get(id.index())` keeps working — a node
        // nobody asked to draw is simply `None`, exactly as `Part::outer` is already `None` for a
        // fragment with no skin.
        let mut parts: Vec<Option<Part>> = (0..node_count).map(|_| None).collect();
        for f in baked.into_pick(&wanted) {
            let lowest = f.cell.points().iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let meshes = &mut world.resource_mut::<Assets<Mesh>>();
            parts[f.id.index()] = Some(Part {
                outer: f.outer.map(|m| meshes.add(m)),
                cap: f.cap.map(|m| meshes.add(m)),
                center_local: f.center_local,
                drop_to_rest: (f.cell.center().y - lowest).max(0.0),
                volume: f.cell.volume().max(1.0e-6),
                cell: f.cell,
            });
        }
        Baked { tree, parts, gore }
    }

    /// The adjacency for one frontier.
    ///
    /// **Not the leaf graph.** A fragment off a graph's frontier has no incident bonds, so reading a
    /// coarse frontier against `Fracture::bonds` reports every piece as its own island and the
    /// subject falls apart on the first blow. Rebuilt per frontier instead — cheap, because the
    /// match is over a few dozen convex cells.
    pub fn graph_for(&self, ids: &[FragmentId]) -> BondGraph {
        let members: Vec<(FragmentId, &ProxyCell)> = ids
            .iter()
            .filter_map(|&id| self.parts.get(id.index())?.as_ref().map(|p| (id, &p.cell)))
            .collect();
        BondGraph::of(&members, self.tree.len())
    }
}

/// The caller's accumulated damage — the graph of whatever frontier is standing, which of its bonds
/// have gone, and which fragments have already left.
///
/// The graph lives here rather than beside the bake because it is per frontier, and so is the
/// `BondSet`: `BondId`s are positions in one graph, so changing granularity means starting both over.
#[derive(Resource)]
pub struct Damage {
    pub bonds: BondGraph,
    pub broken: BondSet,
    pub gone: HashSet<FragmentId>,
}

impl Damage {
    /// An undamaged state for one frontier: its own graph, and an empty set over it.
    pub fn fresh(baked: &Baked, granularity: usize) -> Damage {
        let ids = baked.tree.frontier_of(GRANULARITIES[granularity]);
        let bonds = baked.graph_for(&ids);
        let broken = BondSet::new(&bonds);
        info!("standing at {} pieces, held together by {} bonds", ids.len(), bonds.len());
        Damage { bonds, broken, gone: HashSet::new() }
    }
}

/// A fragment still attached to the body.
#[derive(Component)]
pub struct Attached(pub FragmentId);

/// A fragment that came loose and is now the example's problem rather than the crate's.
#[derive(Component)]
pub struct Chunk {
    pub velocity: Vec3,
    pub spin: Vec3,
    pub drop_to_rest: f32,
    /// **Which fragment this was**, kept so a caller can find the piece's own geometry after it has
    /// left the frontier.
    ///
    /// `Attached` carries the same id; a `Chunk` did not, and the cost showed up immediately: a demo
    /// that wants the wound a gib bled from had to average the cut-face area over *every* part and
    /// call the result typical. That is exactly the invented number this repository refuses, and the
    /// id is one field. `None` for a plug, which was never a fragment — see [`Gore`].
    pub fragment: Option<FragmentId>,
}

/// **A plug in flight** — the material a channel pushed out, on its way to becoming a pool.
///
/// Carries a `Chunk` too, so it falls and tumbles through the same integrator every gib uses. What
/// this adds is the *end* of that life: once it has stopped moving it stops being a mesh and becomes
/// a flat stain on the floor.
#[derive(Component)]
pub struct Gore {
    /// The plug's volume — how big the pool it leaves will be.
    pub volume: f32,
    /// The channel's axis in the plug's **own** local frame, which is the plug's long dimension: the
    /// mesh is recentred but never rotated, so the crate's `direction` is still the rod's own axis.
    /// Kept so a landed plug can be laid flat instead of freezing wherever it happened to be pointing.
    pub axis: Vec3,
    /// Consecutive frames it has been slow and on the floor. A pool forms past [`GORE_SETTLE`].
    ///
    /// Counted in frames rather than seconds deliberately: the recorder runs a fixed 30 Hz and the
    /// windowed demo runs at whatever the display does, and frames are the one thing both have. It is
    /// a cosmetic threshold on an object that has already stopped, so the difference does not show.
    pub settled: u32,
}

/// **Every slick on the floor.** The crate's [`bevy_carnage::Pool`] model, held by the demo.
///
/// This replaced a hand-rolled example-local `Pool` component that spawned one scaled `Circle` disc
/// per landed plug and **never merged** — no proximity test, no spatial hash, no query over existing
/// pools — so a dozen plugs landing together left a dozen coincident discs stacked at `y = 0.006`.
/// Two paths for one feature is what that was; there is now one, and it lives in the crate where the
/// consuming game reads it too.
#[derive(Resource, Default)]
pub struct Pools(pub Vec<bevy_carnage::Pool>);

/// Frames a plug stays a mesh after touching down, before it becomes a pool. Short on purpose: it is
/// the beat between "it landed" and "it spread", and any longer reads as debris that forgot to melt.
pub const GORE_SETTLE: u32 = 3;

/// **How fast a plug leaves.** Faster than a mid-sized gib, because a plug was *pushed* by the thing
/// that made the channel rather than shaken loose by it — but chosen for the frame, not from
/// ballistics. Every plug is tiny enough that [`heft`] saturates at its 2.2 clamp, so the effective
/// launch speed is 2.2x this.
///
/// **Measured, because the first number was wrong by a factor of four.** At 6.5 the plugs left at an
/// effective 14.3 and travelled roughly 8 units before landing — off the far edge of the visible
/// floor, so the pools formed correctly and were simply never on camera, and the flight itself was two
/// frames of a streak. At 1.6 the effective 3.5 puts the landing about 1.2 units out, which is on the
/// floor the camera can see and about a dozen frames of visible arc.
pub const GORE_SPEED: f32 = 1.6;
/// How wide the ejection cone is, as a fraction of the axial speed. Zero would send every plug down
/// the same line, which reads as a mechanism rather than as a spray.
pub const GORE_SPREAD: f32 = 0.30;
/// How much bigger the *stain* a landed plug makes is than the cube root of the volume that made it —
/// spilled material wets far more floor than the lump it came from, and without this a 0.0007-volume
/// plug leaves a mark you cannot see.
///
/// This is the radius handed to [`bevy_carnage::absorb`] as one stain. How the resulting slick then
/// *spreads* is the crate's `pool_spread`/`pool_spread_rate`, not this.
pub const GORE_STAIN_SPREAD: f32 = 1.9;

/// Thrown out along the channel, with a hashed cone and spin — deterministic, no RNG dependency.
///
/// `direction` is what the crate handed over: the bore's own axis. Everything else here is this
/// file's opinion, exactly as [`launch`] is for a fragment.
pub fn launch_gore(seed: u32, direction: Vec3, volume: f32) -> (Vec3, Vec3) {
    let h = |n: u32| hash_f32(seed.wrapping_mul(2_654_435_761).wrapping_add(n));
    // A basis across the channel, so the cone is measured off the axis rather than off the world.
    let aside = if direction.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = direction.cross(aside).normalize_or_zero();
    let v = direction.cross(u);
    let cone = (u * (h(1) - 0.5) + v * (h(2) - 0.5)) * 2.0 * GORE_SPREAD;
    // A little lift, so a horizontal shot arcs instead of skidding along the floor.
    let dir = (direction + cone + Vec3::Y * 0.22).normalize_or_zero();
    let heft = heft(volume);
    let spin = Vec3::new(h(3) - 0.5, h(4) - 0.5, h(5) - 0.5).normalize_or_zero() * (12.0 + 10.0 * h(6));
    (dir * GORE_SPEED * heft, spin * heft)
}

/// How many plugs have already been thrown, so a re-bake does not throw them a second time.
///
/// **Every shot re-bakes from scratch**, because a bore is a bake input — so the plug list is rebuilt
/// each time. It only ever grows by appending, in shot order, so everything past this index is new.
#[derive(Resource, Default)]
pub struct Thrown(pub usize);

#[derive(Resource)]
pub struct BodyMaterials {
    pub skin: Handle<StandardMaterial>,
    pub interior: Handle<StandardMaterial>,
    pub aim: Handle<StandardMaterial>,
    // **No pool material and no disc mesh, deliberately.** A slick is a `bevy_carnage` forward decal
    // now (`decal::spawn_pool`), which shares the crate's four generated splat textures — so this
    // file no longer owns a second, plainer way to draw blood on a floor.
}

impl BodyMaterials {
    pub fn new(world: &mut World) -> BodyMaterials {
        let aim = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.85, 0.25),
            emissive: LinearRgba::rgb(0.6, 0.5, 0.1),
            ..default()
        });
        BodyMaterials {
            skin: material(world, Color::srgb(0.30, 0.42, 0.52), 0.85),
            // **Darker and much glossier than a "red material".** At roughness 0.55 a flat cut face
            // is lit evenly across its whole area, which is what stone and ice look like; dropping it
            // to 0.25 puts a moving highlight on each face instead, and that specular travel is most
            // of what reads as wet. The colour goes down, not up: bright red on a flat plane reads as
            // paint.
            interior: material(world, Color::srgb(0.46, 0.07, 0.07), 0.42),
            aim,
        }
    }
}

/// **The five regions, as this example aims them.** The crate has no idea which weapon any of them
/// is; the names here are the example's reading, and the radii are its tuning.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Blow {
    Projectile,
    Slash,
    SweptBlade,
    Blast,
    Pull,
}

impl Blow {
    pub fn reach(self, graph: &BondGraph, at: Vec3) -> Reach {
        match self {
            // **0.6, and it was measured rather than picked.** At 0.34 a projectile freed a piece
            // only where the body is thin — the head — because an interior fragment is held by six
            // or eight bonds and the region has to reach past all of them. Freed pieces per hit at
            // head / shoulder / flank / hip: 0.34 -> [1,0,0,0], 0.45 -> [1,0,0,1], 0.60 -> [2,1,1,1].
            Blow::Projectile => spread(graph, at, 0.08, 0.60),
            // **The radius has to beat the subject's depth, not look reasonable.** At `max = 0.16`
            // the effective cut is about 0.105 at the shipped threshold and the body is 0.4 deep, so
            // the bonds at the front and back survived, the halves stayed joined around them, and a
            // slash severed eleven bonds while detaching nothing at all. Freed pieces per slash at
            // four heights: 0.30 -> [0,0,1,1], 0.36 -> [1,14,2,2], 0.42 -> [4,15,16,9]. Cleaving is
            // what a slash is for, so 0.42.
            Blow::Slash => capsule(graph, at - Vec3::X * 0.5, at + Vec3::X * 0.5, 0.10, 0.42),
            // A blade sweeping down and across through the aim point: two corners above, one below.
            Blow::SweptBlade => swept_triangle(
                graph,
                at + Vec3::new(-0.9, 0.35, -0.9),
                at + Vec3::new(0.9, 0.35, -0.9),
                at + Vec3::new(0.0, -0.35, 0.9),
            ),
            Blow::Blast => radial(graph, at, 0.15, 1.10),
            Blow::Pull => directional(graph, at, Vec3::Y, 0.20, 0.85),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Blow::Projectile => "projectile",
            Blow::Slash => "slash",
            Blow::SweptBlade => "swept blade",
            Blow::Blast => "blast",
            Blow::Pull => "pull",
        }
    }
}

/// Stand the subject up at one frontier of the hierarchy.
pub fn stand(world: &mut World, granularity: usize) {
    let ids = {
        let baked = world.resource::<Baked>();
        baked.tree.frontier_of(GRANULARITIES[granularity])
    };
    for id in ids {
        spawn_fragment(world, id, None);
    }
}

/// Despawn everything the **body** is currently made of, attached or flying.
///
/// **Gore and pools are deliberately exempt.** A shot re-bakes the subject, and re-baking must not
/// recall debris that has already left it: material in the air belongs to the world now, not to the
/// body it came out of. Without the exemption every new hole would teleport the previous shot's gore
/// back into the subject and throw it again.
pub fn clear(world: &mut World) {
    let doomed: Vec<Entity> = world
        .query_filtered::<Entity, (Or<(With<Attached>, With<Chunk>)>, Without<Gore>, Without<PoolDecal>)>()
        .iter(world)
        .collect();
    for e in doomed {
        world.entity_mut(e).despawn();
    }
}

/// **Despawn the debris too** — every plug in flight and every pool on the floor.
///
/// Separate from [`clear`] because they answer different questions. `clear` rebuilds the subject and
/// must leave debris alone; this is "start over", which means the floor too. A reset that left the
/// blood behind would say the subject had been shot when it had not.
pub fn wipe(world: &mut World) {
    let doomed: Vec<Entity> = world
        .query_filtered::<Entity, Or<(With<Gore>, With<PoolDecal>)>>()
        .iter(world)
        .collect();
    for e in doomed {
        world.entity_mut(e).despawn();
    }
    // **And the model, not just the decals.** `PoolDecal` holds an index into [`Pools`]; leaving the
    // list behind would have the next slick spawn with an index into stale entries and refresh the
    // wrong radius — and `absorb` would keep merging fresh blood into pools nobody can see.
    if let Some(mut pools) = world.get_resource_mut::<Pools>() {
        pools.0.clear();
    }
}

/// **Throw whatever the latest bake ejected and has not been thrown yet.**
///
/// Idempotent across re-bakes by construction: [`Thrown`] records how many plugs have gone, the plug
/// list only grows by appending in shot order, so this spawns exactly the tail. Call it right after
/// [`stand`], with the fresh [`Baked`] already inserted.
pub fn spawn_gore(world: &mut World) {
    let already = world.get_resource::<Thrown>().map_or(0, |t| t.0);
    let Some(fresh) = world.get_resource::<Baked>().map(|b| {
        b.gore
            .iter()
            .skip(already)
            .map(|g| {
                (
                    g.outer.clone(),
                    g.cap.clone(),
                    g.center_local,
                    g.drop_to_rest,
                    g.volume,
                    g.direction,
                )
            })
            .collect::<Vec<_>>()
    }) else {
        return;
    };
    if fresh.is_empty() {
        return;
    }
    let Some(mats) =
        world.get_resource::<BodyMaterials>().map(|m| (m.skin.clone(), m.interior.clone()))
    else {
        return;
    };

    for (i, (outer, cap, center, rest, volume, direction)) in fresh.into_iter().enumerate() {
        // Seeded on the plug's index within the whole run, so the same shot sequence throws the same
        // way on every run — the property the GIF rests on.
        let (velocity, spin) = launch_gore((already + i) as u32, direction, volume);
        let entity = world
            .spawn((
                Transform::from_translation(ORIGIN + center),
                Visibility::default(),
                // A plug was never a fragment, so it has no id to carry.
                Chunk { velocity, spin, drop_to_rest: rest, fragment: None },
                Gore { volume, axis: direction, settled: 0 },
            ))
            .id();
        world.entity_mut(entity).with_children(|parent| {
            if let Some(outer) = outer {
                parent.spawn((Mesh3d(outer), MeshMaterial3d(mats.0.clone())));
            }
            if let Some(cap) = cap {
                parent.spawn((Mesh3d(cap), MeshMaterial3d(mats.1.clone())));
            }
        });
    }
    let total = world.get_resource::<Baked>().map_or(already, |b| b.gore.len());
    if let Some(mut thrown) = world.get_resource_mut::<Thrown>() {
        thrown.0 = total;
    }
}

/// **A plug that has stopped stops being a mesh.** Each settled [`Gore`] chunk becomes one stain, the
/// stains are folded into [`Pools`], and every pool spreads one tick.
///
/// One system for all three because they are one transition: a gib whose geometry persists forever is
/// what makes a floor read as a bin of debris, and the fix is not smaller gibs but material that
/// stops being an object once it has spilled.
///
/// **The merge is the crate's**, [`bevy_carnage::absorb`] and [`bevy_carnage::spread_pools`]. The
/// version this replaced spawned one disc per plug and never merged, so a cluster of plugs landing
/// together left a stack of coincident circles instead of a slick.
pub fn bleed(world: &mut World) {
    // Which chunks have come to rest? Read first, mutate after — a plug is despawned in the same
    // frame it becomes a stain, so the two cannot share a query.
    let mut landed: Vec<(Entity, bevy_carnage::Stain)> = Vec::new();
    {
        let mut q = world.query::<(Entity, &mut Gore, &mut Chunk, &mut Transform)>();
        for (e, mut gore, mut chunk, mut transform) in q.iter_mut(world) {
            let grounded = transform.translation.y <= chunk.drop_to_rest + 1.0e-3;
            if grounded {
                // **A plug does not bounce and does not skid: the first touch is where it stays.**
                // The shared integrator treats every chunk as a pebble — restitution 0.3 and a drag
                // that only bites while it is in contact — which for a wet lump of material is wrong
                // twice over. Measured with it: a plug still carried 0.99 of speed sixteen frames
                // after landing, so it slid across the floor and the pool formed a metre from where
                // it came down. Stopping it dead is both the honest reading and what puts the stain
                // where the shot pointed. Written here rather than in `integrate` so there is still
                // one integrator, with one chunk type, differing only in what this rule does after it.
                chunk.velocity = Vec3::ZERO;
                chunk.spin = Vec3::ZERO;
                // **And it lies down.** A plug is a rod — as long as the subject is deep and as wide
                // as the calibre — so freezing it mid-tumble left it standing on one end like a
                // bollard, which is the single most artificial thing in the clip. Rotate its own long
                // axis onto the horizontal plane and it flops the way a wet lump does.
                let world_axis = transform.rotation * gore.axis;
                let flat = Vec3::new(world_axis.x, 0.0, world_axis.z).normalize_or_zero();
                if flat != Vec3::ZERO {
                    transform.rotation =
                        Quat::from_rotation_arc(world_axis.normalize_or_zero(), flat)
                            * transform.rotation;
                }
                gore.settled += 1;
            } else {
                gore.settled = 0;
            }
            if gore.settled >= GORE_SETTLE {
                // Sized by the cube root of the volume, because that is the plug's linear dimension,
                // times a spread factor: liquid wets far more floor than the lump it came from.
                let at = Vec3::new(transform.translation.x, 0.0, transform.translation.z);
                landed.push((
                    e,
                    Stain {
                        at,
                        radius: gore.volume.cbrt() * GORE_STAIN_SPREAD,
                        // From the plug's own landing point, never from its `Entity` — an entity id
                        // is a slot index assigned by allocation order, which is the one thing this
                        // crate refuses to seed from anywhere.
                        seed: at.x.to_bits() ^ at.z.to_bits().rotate_left(16),
                    },
                ));
            }
        }
    }

    let settings = world.get_resource::<CarnageSettings>().cloned().unwrap_or_default();

    // Fold the fresh stains in. A new pool gets a decal; a stain that merged into an existing pool
    // just grows the one already drawn, which is the whole point of the model.
    if !landed.is_empty() {
        for (entity, _) in &landed {
            world.entity_mut(*entity).despawn();
        }
        let stains: Vec<Stain> = landed.into_iter().map(|(_, s)| s).collect();
        let mut pools = world.get_resource_or_insert_with(Pools::default);
        let before = pools.0.len();
        absorb(&mut pools.0, &stains, &settings);
        // Only the pools `absorb` appended are new; everything before `before` already has a decal.
        // Indices are stable because `absorb` only ever pushes.
        let fresh: Vec<(usize, Pool)> =
            pools.0.iter().enumerate().skip(before).map(|(i, p)| (i, *p)).collect();

        if !fresh.is_empty()
            && let Some(splats) = world.get_resource::<SplatTextures>().cloned()
        {
            // `commands` is scoped and dropped before `flush`, matching `examples/carnage.rs`'s
            // stain-stamping block — a `Commands` holds the world mutably, and flushing while it is
            // still live is the one thing that shape gets wrong.
            {
                let mut commands = world.commands();
                for (index, pool) in fresh {
                    spawn_pool(&mut commands, &splats, index, &pool);
                }
            }
            world.flush();
        }
    }

    // Spread every pool one tick and push the new radii onto their decals. A slick that appeared at
    // full size would read as a decal being switched on.
    let Some(mut pools) = world.get_resource_mut::<Pools>() else { return };
    spread_pools(&mut pools.0, &settings);
    let snapshot = std::mem::take(&mut pools.0);
    let mut q = world.query::<(&PoolDecal, &mut Transform)>();
    update_pool_decals(&snapshot, q.iter_mut(world));
    world.resource_mut::<Pools>().0 = snapshot;
}

/// One fragment, attached if `launch` is `None` and flying if it is.
///
/// Both meshes are already recentred on the fragment's own centre, so a chunk spins about itself
/// rather than orbiting the origin.
pub fn spawn_fragment(world: &mut World, id: FragmentId, launch: Option<(Vec3, Vec3)>) {
    let Some((outer, cap, center, rest)) = world.get_resource::<Baked>().and_then(|b| {
        b.parts
            .get(id.index())?
            .as_ref()
            .map(|p| (p.outer.clone(), p.cap.clone(), p.center_local, p.drop_to_rest))
    }) else {
        return;
    };
    let Some(mats) = world.get_resource::<BodyMaterials>().map(|m| (m.skin.clone(), m.interior.clone()))
    else {
        return;
    };

    let mut e = world.spawn((Transform::from_translation(ORIGIN + center), Visibility::default()));
    match launch {
        Some((velocity, spin)) => {
            e.insert(Chunk { velocity, spin, drop_to_rest: rest, fragment: Some(id) });
        }
        None => {
            e.insert(Attached(id));
        }
    }
    let entity = e.id();
    world.entity_mut(entity).with_children(|parent| {
        if let Some(outer) = outer {
            parent.spawn((Mesh3d(outer), MeshMaterial3d(mats.0)));
        }
        if let Some(cap) = cap {
            parent.spawn((Mesh3d(cap), MeshMaterial3d(mats.1)));
        }
    });
}

/// What one blow actually did — enough for a caller to tell "nothing happened" from "nothing
/// happened *yet*", which are different and look identical on screen without it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Outcome {
    /// Bonds the region touched at all.
    pub reached: usize,
    /// Bonds whose severity cleared the threshold.
    pub gave_way: usize,
    /// Of those, how many were still intact before this blow.
    pub newly: usize,
    /// Fragments that stopped being connected to the body.
    pub off: usize,
}

/// **The whole feature in one function.** Pick a region, threshold the reach it comes back with,
/// sever what gave way, and re-run island detection to see what is no longer holding on.
pub fn strike(world: &mut World, blow: Blow, at: Vec3) -> Outcome {
    let standing: Vec<(Entity, FragmentId)> =
        world.query::<(Entity, &Attached)>().iter(world).map(|(e, a)| (e, a.0)).collect();
    if standing.is_empty() {
        return Outcome::default();
    }
    let mut outcome = Outcome::default();

    // Which fragments stopped being connected to the body. Scoped so every borrow of `Damage` and
    // `Baked` has ended before anything is spawned.
    let leaving: Vec<(Entity, FragmentId)> = {
        let Some(mut damage) = world.remove_resource::<Damage>() else { return Outcome::default() };
        let reach = blow.reach(&damage.bonds, at);
        let gave_way = reach.above(GIVES_WAY);
        let newly = damage.broken.sever_all(&gave_way);
        outcome.reached = reach.len();
        outcome.gave_way = gave_way.len();
        outcome.newly = newly;
        info!(
            "{} at {:.2},{:.2},{:.2} — reached {} bonds, {} gave way ({newly} newly)",
            blow.label(),
            at.x,
            at.y,
            at.z,
            reach.len(),
            gave_way.len()
        );

        let ids: Vec<FragmentId> = standing.iter().map(|(_, id)| *id).collect();
        let islands = damage.bonds.islands(&ids, &damage.broken);
        // **The body is the biggest island.** That is this example's rule, not the crate's — a game
        // with a floor would more likely keep whichever island is still standing on it.
        let body = islands.iter().enumerate().max_by_key(|(_, i)| i.len()).map(|(k, _)| k);
        for (k, island) in islands.iter().enumerate() {
            if Some(k) != body {
                damage.gone.extend(island.iter().copied());
            }
        }
        let gone = damage.gone.clone();
        world.insert_resource(damage);
        standing.into_iter().filter(|(_, id)| gone.contains(id)).collect()
    };

    for (entity, id) in &leaving {
        world.entity_mut(*entity).despawn();
        let center = world
            .get_resource::<Baked>()
            .and_then(|b| b.parts.get(id.index())?.as_ref().map(|p| p.center_local))
            .unwrap_or(Vec3::ZERO);
        let volume = world
            .get_resource::<Baked>()
            .and_then(|b| b.parts.get(id.index())?.as_ref().map(|p| p.volume))
            .unwrap_or(REFERENCE_VOLUME);
        let (velocity, spin) = launch(*id, center, at, volume);
        spawn_fragment(world, *id, Some((velocity, spin)));
    }
    if !leaving.is_empty() {
        info!("  {} fragment(s) came off", leaving.len());
    }
    outcome.off = leaving.len();
    outcome
}

/// Thrown away from where the blow landed, with deterministic variation from the crate's own frozen
/// hash — no RNG dependency in an example either.
///
/// **Speed and spin scale down with mass, and that is most of the difference between "gibs" and
/// "shrapnel".** Throwing every piece at the same speed is what made the old burst read as an
/// explosion in a quarry: a severed arm and a fingernail left at identical velocity. A blow delivers
/// roughly an impulse, so light pieces should leave fast and heavy ones should barely move and flop.
pub fn launch(id: FragmentId, center: Vec3, at: Vec3, volume: f32) -> (Vec3, Vec3) {
    let h = |n: u32| hash_f32(id.0.wrapping_mul(2_654_435_761).wrapping_add(n));
    let away = (center - at).normalize_or_zero();
    let dir = (away + Vec3::Y * (0.35 + 0.5 * h(3))).normalize_or_zero();
    // Cube root, because velocity from a fixed impulse goes as 1/mass and mass goes as the cube of
    // size — so the linear dimension is the honest scale, and it keeps the spread narrow enough that
    // a heavy piece still moves.
    let heft = heft(volume);
    let spin = Vec3::new(h(1) - 0.5, h(2) - 0.5, h(3) - 0.5).normalize_or_zero() * (7.0 + 7.0 * h(2));
    (dir * (2.4 + 2.0 * h(4)) * heft, spin * heft)
}

/// The fragment size that leaves at unmodified speed — roughly a mid-sized chunk of this subject.
/// Everything smaller goes faster, everything larger slower.
pub const REFERENCE_VOLUME: f32 = 0.012;

/// How much faster than a reference-sized chunk a fragment of this volume leaves.
///
/// Cube root, because velocity from a fixed impulse goes as `1/mass` and mass goes as the cube of
/// size — so the linear dimension is the honest scale, and it keeps the spread narrow enough that a
/// heavy piece still moves rather than sitting perfectly still.
pub fn heft(volume: f32) -> f32 {
    (REFERENCE_VOLUME / volume.max(1.0e-6)).cbrt().clamp(0.45, 2.2)
}

/// The examples' whole solver — the crate names none. Gravity, a ground bounce, and tumbling.
pub fn integrate(chunk: &mut Chunk, transform: &mut Transform, dt: f32) {
    chunk.velocity.y -= GRAVITY * dt;
    transform.translation += chunk.velocity * dt;
    transform.rotate_local_x(chunk.spin.x * dt);
    transform.rotate_local_y(chunk.spin.y * dt);
    transform.rotate_local_z(chunk.spin.z * dt);

    let floor = chunk.drop_to_rest;
    if transform.translation.y < floor {
        transform.translation.y = floor;
        if chunk.velocity.y < 0.0 {
            chunk.velocity.y = -chunk.velocity.y * RESTITUTION;
            let damp = (1.0 - GROUND_DRAG * dt).max(0.0);
            chunk.velocity.x *= damp;
            chunk.velocity.z *= damp;
            chunk.spin *= damp;
            if chunk.velocity.y.abs() < 0.4 {
                chunk.velocity.y = 0.0;
            }
        }
    }
}
