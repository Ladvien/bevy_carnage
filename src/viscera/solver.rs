//! **The XPBD step: a fixed number of substeps, a fixed number of iterations, a fixed sequence.**
//!
//! Extended position-based dynamics (Macklin, Müller & Chentanez, *XPBD: position-based simulation of
//! compliant constrained dynamics*, MIG 2016) replaces PBD's iteration-count-dependent stiffness with a
//! physical **compliance** `α`, entering the projection as `α̃ = α / Δt²`:
//!
//! ```text
//! Δλ  = (−C − α̃ λ) / (Σ wᵢ |∇ᵢC|² + α̃)
//! Δxᵢ = wᵢ ∇ᵢC Δλ
//! ```
//!
//! Deul, Charrier & Bender (*Direct position-based solver for stiff rods*, CGF 37(6),
//! `doi:10.1111/cgf.13326`) is the rod formulation this follows — their zero-stretch/bend-twist
//! decomposition is what the two compliances here stand in for. The bending term is a *position-only*
//! surrogate: Bergou et al. (*Discrete elastic rods*, ACM TOG 27(3), `doi:10.1145/1399504.1360662`)
//! write bending against the curvature binormal of a material frame, and a frame is state this crate
//! deliberately does not carry, so the resistance to turning is expressed instead as a distance
//! constraint across the triple `(i−1, i+1)` at twice the rest length. It resists coiling and never
//! resists straightening — a chain of rest-length segments cannot exceed that span.
//!
//! **Nothing here reads a clock and nothing here converges early.** The tick length is
//! [`FIXED_DT`], the counts come from [`crate::viscera::ViscSettings`], and each iteration walks the whole
//! sequence whether or not the residual is already zero. Constraints that are *skipped* — torn,
//! degenerate, or slack — are skipped by data, not by convergence, so the pass count is the same for
//! every strand on every run.

use bevy::math::Vec3;

use crate::viscera::settings::ViscSettings;
use crate::viscera::strand::{Mesentery, Strand, DEFAULT_TEAR_STRAIN, MAX_NODES, STRAND_TEAR_STRAIN};

/// The tick rate the solver assumes, in hertz.
///
/// **The crate is integer-tick and never reads a clock.** Bevy's `Time<Fixed>` defaults to 64 Hz
/// (`bevy_time-0.19.0/src/fixed.rs:76`), so an app that wants viscera to fall at wall-clock speed
/// should say `Time::<Fixed>::from_hz(60.0)`. Reading the app's timestep instead would make the digest
/// a function of a runtime setting, which is exactly the property this crate sells.
pub const FIXED_HZ: f32 = 60.0;

/// The length of one fixed tick, seconds. See [`FIXED_HZ`].
pub const FIXED_DT: f32 = 1.0 / FIXED_HZ;

/// XPBD compliance of one mesenteric link, m/N.
///
/// **TUNED, not measured, and load-bearing** — but tuned from an equation rather than by eye, and the
/// equation is worth keeping because both ways of getting it wrong produce a flag that means nothing.
///
/// A compliant XPBD constraint does not converge to `C = 0`. Accumulating the multiplier to its fixed
/// point leaves `C = C₀·α̃/(1+α̃)`, so each substep removes only `1/(1+α̃)` of the error while gravity
/// adds `g·Δt²` per node of hanging weight. A link therefore settles at
///
/// ```text
/// C = (1 + α̃) · g · Δt² · N        for N nodes hanging below it
/// ```
///
/// and it parts once `C` passes `tear_strain · rest_len`. At the shipped defaults — `g = 18`,
/// `Δt = 1/240`, `rest_len = 35 mm`, `tear_strain = 0.35`, so a 12.2 mm threshold — `6e-5` puts
/// `α̃ ≈ 3.46` and the capacity at **about nine nodes**. That is the whole design: a strand tethered
/// every fourth node holds, one tethered every twelfth does not.
///
/// Both neighbourhoods of this value are dead. Far softer (`5e-4`, `α̃ ≈ 29`) supports barely one node
/// and *everything* tears; far stiffer (`compliance_stretch`, `α̃ ≈ 0.06`) takes up any load the
/// iteration budget can resolve and *nothing* ever tears. A membrane that stretches is a membrane
/// that can be torn.
pub const COMPLIANCE_MESENTERY: f32 = 6.0e-5;

/// The largest number of mesenteric links one strand's solve will project.
///
/// The Lagrange multipliers live in a stack array of this size, which is what keeps a step
/// allocation-free. A strand holds at most [`MAX_NODES`] nodes, so more tethers than this is not a
/// configuration the model describes; the surplus is ignored rather than reallocated for.
pub const MAX_ANCHORS: usize = MAX_NODES;

/// Below this length a difference vector has no usable direction, so the constraint is skipped.
const EPS: f32 = 1.0e-9;

/// Everything a substep needs, computed once for the whole batch.
#[derive(Clone, Copy)]
struct Pass {
    substeps: u32,
    iterations: u32,
    dt: f32,
    inv_dt: f32,
    keep: f32,
    fall: Vec3,
    alpha_stretch: f32,
    alpha_bend: f32,
    alpha_tether: f32,
    floor_y: f32,
}

/// **Advance every strand by one fixed tick.**
///
/// `mesentery[i]` tethers `strands[i]`; a strand with no matching entry is simply untethered, and a
/// mesentery with no matching strand is untouched. Strands do not see each other — there is no
/// strand-strand collision and no shared accumulator — so the slice order changes nothing, and the
/// per-entity system in [`crate::viscera::VisceraPlugin`] is free of ECS query order for the same reason.
pub fn step(strands: &mut [Strand], mesentery: &mut [Mesentery], s: &ViscSettings) {
    let substeps = s.substeps.max(1);
    let iterations = s.iterations.max(1);
    let dt = FIXED_DT / substeps as f32;
    let inv_dt = 1.0 / dt;
    let inv_dt2 = inv_dt * inv_dt;
    let gravity = if s.gravity.is_finite() { s.gravity } else { 0.0 };
    let damping = if s.damping.is_finite() { s.damping } else { 0.0 };
    let pass = Pass {
        substeps,
        iterations,
        dt,
        inv_dt,
        // Damping is a velocity multiplier applied once per substep, before the prediction that uses
        // that velocity — so it is a property of the motion, not of the constraint residual.
        keep: (1.0 - damping).clamp(0.0, 1.0),
        fall: Vec3::new(0.0, -gravity * dt * dt, 0.0),
        alpha_stretch: sane_compliance(s.compliance_stretch) * inv_dt2,
        alpha_bend: sane_compliance(s.compliance_bend) * inv_dt2,
        alpha_tether: COMPLIANCE_MESENTERY * inv_dt2,
        floor_y: if s.floor_y.is_finite() { s.floor_y } else { 0.0 },
    };

    for (i, strand) in strands.iter_mut().enumerate() {
        solve_one(strand, mesentery.get_mut(i), &pass);
    }
}

/// A negative or non-finite compliance is a caller slip, and a rigid constraint is the safe reading.
#[inline]
fn sane_compliance(alpha: f32) -> f32 {
    if alpha.is_finite() { alpha.max(0.0) } else { 0.0 }
}

/// One strand and its tether, start to finish. Nothing outside these two objects is read or written.
fn solve_one(strand: &mut Strand, tether: Option<&mut Mesentery>, k: &Pass) {
    let rest = strand.rest_len();
    let radius = strand.radius();
    let bend_rest = rest * 2.0;
    let strand_tear = rest * STRAND_TEAR_STRAIN;
    let floor = k.floor_y + radius;

    let mut tether = tether;
    let tether_tear = match tether.as_deref_mut() {
        Some(m) => {
            // The flags are parallel to the anchors and must stay that way through the sort below, or
            // a tear would migrate to a different link — which is how a monotone flag turns into an
            // oscillation. Grown, never shrunk: shrinking is the one edit that could clear a tear.
            if m.torn.len() < m.anchors.len() {
                m.torn.resize(m.anchors.len(), false);
            }
            canonicalise(m);
            let strain =
                if m.tear_strain.is_finite() { m.tear_strain.max(0.0) } else { DEFAULT_TEAR_STRAIN };
            rest * strain
        }
        None => 0.0,
    };

    let (pos, prev, torn) = strand.state_mut();
    // Every index below is proved in range by this one line: the three arrays are built together and
    // never resized, but taking the minimum costs nothing and removes the last panicking index.
    let n = pos
        .len()
        .min(prev.len())
        .min(torn.len().saturating_add(1))
        .min(MAX_NODES);
    let last_seg = n.saturating_sub(1);

    // XPBD multipliers, zeroed at the start of every substep (Macklin et al. 2016, §3.3) and
    // accumulated across the iterations within it. Stack arrays, so a step allocates nothing.
    let mut lam_stretch = [0.0f32; MAX_NODES];
    let mut lam_bend = [0.0f32; MAX_NODES];
    let mut lam_tether = [0.0f32; MAX_ANCHORS];

    for _ in 0..k.substeps {
        // --- integrate ------------------------------------------------------------------------
        // `prev` is rewritten to the pre-prediction position here rather than after the solve. That
        // is the same thing: the projection passes below touch `pos` only, so the implicit velocity
        // `(pos - prev) / dt` read next substep is identical either way — and this ordering needs no
        // scratch buffer.
        for i in 0..n {
            let v = (pos[i] - prev[i]) * k.inv_dt * k.keep;
            prev[i] = pos[i];
            pos[i] += v * k.dt + k.fall;
        }

        lam_stretch.fill(0.0);
        lam_bend.fill(0.0);
        lam_tether.fill(0.0);

        for _ in 0..k.iterations {
            // --- 1. stretch: node i → i+1, ascending --------------------------------------------
            for i in 0..last_seg {
                if torn[i] {
                    continue;
                }
                let d = pos[i + 1] - pos[i];
                let len = d.length();
                if len <= EPS {
                    continue;
                }
                let c = len - rest;
                if c > strand_tear {
                    // Monotone, like clotting: set once, skipped forever after.
                    torn[i] = true;
                    continue;
                }
                let dir = d / len;
                let dl = (-c - k.alpha_stretch * lam_stretch[i]) / (2.0 + k.alpha_stretch);
                lam_stretch[i] += dl;
                pos[i] -= dir * dl;
                pos[i + 1] += dir * dl;
            }

            // --- 2. bend: triples (i-1, i, i+1), ascending ---------------------------------------
            for i in 1..last_seg {
                // A triple straddles two segments. If either has parted there is no rod left to bend.
                if torn[i - 1] || torn[i] {
                    continue;
                }
                let d = pos[i + 1] - pos[i - 1];
                let len = d.length();
                if len <= EPS {
                    continue;
                }
                let c = len - bend_rest;
                let dir = d / len;
                let dl = (-c - k.alpha_bend * lam_bend[i]) / (2.0 + k.alpha_bend);
                lam_bend[i] += dl;
                pos[i - 1] -= dir * dl;
                pos[i + 1] += dir * dl;
            }

            // --- 3. mesentery anchors, ascending by node index -----------------------------------
            if let Some(m) = tether.as_deref_mut() {
                let count = m.anchors.len().min(m.torn.len()).min(MAX_ANCHORS);
                for slot in 0..count {
                    if m.torn[slot] {
                        continue;
                    }
                    let (node, point) = m.anchors[slot];
                    let idx = node as usize;
                    if idx >= n {
                        continue;
                    }
                    let d = pos[idx] - point;
                    let len = d.length();
                    if len <= EPS {
                        continue;
                    }
                    // **The link is a pin: its rest length is zero.** `Mesentery` carries no length
                    // of its own, so the only length scale in the data is the strand's segment rest
                    // length, and that is what strain is measured in. A rest length of one segment
                    // was the other reading and it is measurably wrong: it leaves the link slack for
                    // 35 mm and then gives it 12 mm of working range, which a node already at
                    // terminal velocity crosses inside one substep — so every tether tore, always,
                    // and the flag stopped meaning anything.
                    let c = len;
                    if c > tether_tear {
                        m.torn[slot] = true;
                        continue;
                    }
                    let dir = d / len;
                    let dl = (-c - k.alpha_tether * lam_tether[slot]) / (1.0 + k.alpha_tether);
                    lam_tether[slot] += dl;
                    pos[idx] += dir * dl;
                }
            }

            // --- 4. floor ------------------------------------------------------------------------
            // A positional clamp at `floor_y + radius`, so the tube rests on the plane rather than
            // through it. It is last in the sequence, so the substep always ends above the floor.
            // Clamping position without touching `prev` bleeds the downward velocity away instead of
            // reflecting it: viscera land, they do not bounce.
            for i in 0..n {
                if pos[i].y < floor {
                    pos[i].y = floor;
                }
            }
        }
    }
}

/// Sort a mesentery's anchors into ascending node order, carrying the tear flags with them.
///
/// The projection order is stated as "ascending by node index", so it has to be a property of the
/// data rather than of the order a caller happened to `push`. Insertion sort because the list is at
/// most [`MAX_ANCHORS`] long and is almost always already sorted, which this walks in one pass; it is
/// stable, so two anchors on the same node keep their relative order and the result is total.
fn canonicalise(m: &mut Mesentery) {
    let n = m.anchors.len().min(m.torn.len());
    for i in 1..n {
        let mut j = i;
        while j > 0 && m.anchors[j - 1].0 > m.anchors[j].0 {
            m.anchors.swap(j - 1, j);
            m.torn.swap(j - 1, j);
            j -= 1;
        }
    }
}
