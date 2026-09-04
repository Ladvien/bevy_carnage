//! The two components: a [`Strand`] of bowel and the [`Mesentery`] that tethers it.

use bevy::log::warn;
use bevy::math::Vec3;
use bevy::prelude::Component;

use crate::viscera::hash::{fnv1a_u32, FNV_SEED};

/// The largest node count a strand may hold.
///
/// Not a memory budget — a *solver* budget. The XPBD multipliers are carried in fixed-size stack
/// arrays sized by this constant (see `crate::viscera::solver`), which is what keeps a step allocation-free,
/// and an eight-sided tube over this many nodes is under 512 triangles.
pub const MAX_NODES: usize = 32;

/// The largest segment count [`Strand::new`] will build: one fewer than [`MAX_NODES`].
pub const MAX_SEGMENTS: u32 = MAX_NODES as u32 - 1;

/// The shortest rest length a segment may have.
///
/// A zero rest length gives a strand with no length scale: strain is `(len - rest) / rest`, and the
/// mesenteric tear threshold is measured in it. Clamped rather than rejected because [`Strand::new`]
/// must return a usable strand.
pub const MIN_REST_LEN: f32 = 1.0e-4;

/// The engineering strain at which a *bowel* segment itself parts.
///
/// **TUNED, not measured.** It is set above [`DEFAULT_TEAR_STRAIN`] on purpose, and the ordering is the
/// anatomical one: the mesentery is a thin double fold of peritoneum and gives way before the bowel
/// wall does. With `compliance_stretch` at its shipped `1e-6` the stretch constraint resolves a
/// gravity load inside the iteration budget, so a strand does not part under its own weight — only
/// under a load the solver cannot take up. Like the mesentery's, this tear is monotone.
pub const STRAND_TEAR_STRAIN: f32 = 0.6;

/// The strain at which a mesenteric link parts, and [`Mesentery`]'s `Default`.
pub const DEFAULT_TEAR_STRAIN: f32 = 0.35;

/// **A Cosserat-style strand of bowel: a polyline of nodes with unit mass, solved by XPBD.**
///
/// Positions are world space and are the whole of the state — velocity is implicit in `prev`, the
/// Verlet way, so there is no second integrator to keep in step with the first. The node array is the
/// canonical order for every pass in the crate: constraints project along it ascending, and
/// [`Strand::digest`] hashes it in the same order.
///
/// Formulation: Deul, Charrier & Bender, *Direct position-based solver for stiff rods*, Computer
/// Graphics Forum 37(6) (`doi:10.1111/cgf.13326`); Bergou, Wardetzky, Robinson, Audoly & Grinspun,
/// *Discrete elastic rods*, ACM TOG 27(3) (`doi:10.1145/1399504.1360662`).
#[derive(Component, Clone, Debug)]
pub struct Strand {
    /// Current node positions, world space. Length `segments + 1`, never more than [`MAX_NODES`].
    pos: Vec<Vec3>,
    /// Node positions at the start of the current substep. `(pos - prev) / dt` is the velocity.
    prev: Vec<Vec3>,
    /// The rest length of every segment, and the length scale strain is measured in.
    rest_len: f32,
    /// Tube radius. The floor is a clamp at `floor_y + radius`, so a strand rests *on* the floor.
    radius: f32,
    /// One flag per segment. Set when the segment's strain passes [`STRAND_TEAR_STRAIN`]; never
    /// cleared, so a tear cannot heal and the sim cannot oscillate between torn and whole.
    torn: Vec<bool>,
}

impl Strand {
    /// Lay a straight strand of `segments` segments from `from`, along `dir`.
    ///
    /// Every argument is clamped rather than rejected, because this returns a `Strand` and a caller
    /// that has already decided to spill guts is not helped by a `Result`:
    ///
    /// * `segments` into `1..=`[`MAX_SEGMENTS`],
    /// * `rest_len` up to at least [`MIN_REST_LEN`],
    /// * `radius` up to at least `0.0`,
    /// * `dir` normalised — and a direction that cannot be normalised is reported and replaced by
    ///   straight down. That is not a second code path: a strand of coincident nodes has no stretch
    ///   gradient, so it would be stuck at a point forever, which is a worse failure than a loud one.
    pub fn new(from: Vec3, dir: Vec3, segments: u32, rest_len: f32, radius: f32) -> Self {
        let segments = segments.clamp(1, MAX_SEGMENTS) as usize;
        let rest_len = if rest_len.is_finite() { rest_len.max(MIN_REST_LEN) } else { MIN_REST_LEN };
        let radius = if radius.is_finite() { radius.max(0.0) } else { 0.0 };
        let from = if from.is_finite() { from } else { Vec3::ZERO };
        let axis = axis_or_down(dir);

        let mut pos = Vec::with_capacity(segments + 1);
        for i in 0..=segments {
            pos.push(from + axis * (rest_len * i as f32));
        }
        Self { prev: pos.clone(), pos, rest_len, radius, torn: vec![false; segments] }
    }

    /// The node positions, in the crate's canonical order.
    #[inline]
    pub fn nodes(&self) -> &[Vec3] {
        &self.pos
    }

    /// The tube radius, in metres.
    #[inline]
    pub fn radius(&self) -> f32 {
        self.radius
    }

    /// **FNV-1a over the node positions' `f32::to_bits()`, in node order.**
    ///
    /// The crate's product. Two runs of the same spill through the same number of [`crate::viscera::step`]
    /// calls print the same number, on any machine, at any thread count — there is no thread count to
    /// vary, because a strand never reads another strand.
    pub fn digest(&self) -> u64 {
        let mut h = FNV_SEED;
        for p in &self.pos {
            h = fnv1a_u32(h, p.x.to_bits());
            h = fnv1a_u32(h, p.y.to_bits());
            h = fnv1a_u32(h, p.z.to_bits());
        }
        h
    }

    /// The rest length of every segment.
    #[inline]
    pub(crate) fn rest_len(&self) -> f32 {
        self.rest_len
    }

    /// Whether segment `i` has parted. Out of range reads as parted, so no pass walks off the end.
    #[inline]
    pub(crate) fn segment_torn(&self, i: usize) -> bool {
        self.torn.get(i).copied().unwrap_or(true)
    }

    /// The solver's view: positions, previous positions and the tear flags at once, so the borrow
    /// checker sees three disjoint fields rather than one `&mut self`.
    #[inline]
    pub(crate) fn state_mut(&mut self) -> (&mut [Vec3], &mut [Vec3], &mut [bool]) {
        (&mut self.pos, &mut self.prev, &mut self.torn)
    }
}

/// **The mesenteric membrane: a fan of tethers from fixed world points to nodes of one strand.**
///
/// Each anchor is `(node index, world point)` and each link is a **pin** — a compliant distance
/// constraint of rest length zero, so it is engaged from the first tick and there is no slack phase
/// to fall through. Its strain is the displacement measured in segment rest lengths,
/// `|node − anchor| / rest_len`, because `Mesentery` carries no length of its own and that is the only
/// length scale in the data. At the shipped defaults [`DEFAULT_TEAR_STRAIN`] is therefore a 12 mm
/// leash, and [`crate::viscera::COMPLIANCE_MESENTERY`] is sized so one link supports about nine nodes of
/// hanging weight before it reaches it.
///
/// A link whose strain passes [`Mesentery::tear_strain`] sets its flag in `torn` and is skipped
/// forever after. **The tear is monotone, like clotting**: nothing in this crate clears a flag, so a
/// tether cannot heal and the sim cannot oscillate between held and free.
#[derive(Component, Clone, Debug)]
pub struct Mesentery {
    /// `(node index into the strand, fixed world point)`. Projected in **ascending index order**, not
    /// in the order they were pushed, so two callers that build the same set in different orders get
    /// the same simulation. Anchors past [`MAX_NODES`] slots are ignored.
    pub anchors: Vec<(u32, Vec3)>,
    /// The engineering strain at which a link parts. Defaults to [`DEFAULT_TEAR_STRAIN`].
    pub tear_strain: f32,
    /// One flag per anchor, parallel to `anchors`. Grown to match by the solver if it is short; never
    /// cleared by anything in this crate.
    pub torn: Vec<bool>,
}

impl Default for Mesentery {
    fn default() -> Self {
        Self { anchors: Vec::new(), tear_strain: DEFAULT_TEAR_STRAIN, torn: Vec::new() }
    }
}

/// Normalise, or report the caller's bug and lay the strand straight down.
///
/// Shared by [`Strand::new`] and [`crate::viscera::spill`] so that a degenerate direction has exactly one
/// meaning in the crate rather than one per entry point.
pub(crate) fn axis_or_down(dir: Vec3) -> Vec3 {
    match dir.try_normalize() {
        Some(axis) => axis,
        None => {
            warn!(
                "bevy_viscera: a strand direction of {dir:?} cannot be normalised; laying it straight \
                 down instead. Coincident nodes have no stretch gradient and would never separate."
            );
            Vec3::NEG_Y
        }
    }
}
