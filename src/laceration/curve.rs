//! **How wide a cut opens, and when.** The time curve, the skin tension that scales it, and the
//! Langer-line anisotropy that decides how much a given direction of cut gapes at all.
//!
//! Engine-free apart from `f32`: a direction is `[f32; 3]`, so nothing here names a math library and
//! nothing here can perturb a float differently on two machines.
//!
//! # Why a wound gapes, and why it stays gaped
//!
//! Skin is not slack. It sits under a resting tension carried by a collagen network with a
//! preferred direction — the Langer lines — so cutting it releases that tension and the two lips
//! retract away from each other. Ní Annaidh et al., *"Characterization of the anisotropic mechanical
//! properties of excised human skin"*, J. Mech. Behav. Biomed. Mater. 5 (2012),
//! `doi:10.1016/j.jmbbm.2011.08.016`, measured the anisotropy directly on excised human back skin:
//! ultimate tensile strength **21.6 ± 8.4 MPa**, failure strain **54 ± 17 %**, and an elastic modulus
//! of **112.5 MPa parallel** to the Langer lines against **63.8 MPa perpendicular** to them (middle
//! back). A cut made *across* the lines severs the tensioned fibres and gapes most; a cut made
//! *along* them parts a network that is still holding hands across the wound, and gapes least. That
//! ratio, `63.8 / 112.5 = 0.567`, is [`ALONG_LANGER_FACTOR`].
//!
//! The gape does not spring back, and that is not laziness in the model. O'Brien, Bargteil &
//! Hodgins, *"Graphical modeling and animation of ductile fracture"*, SIGGRAPH 2002,
//! `doi:10.1145/566570.566579`, is the paper that put **plastic** — permanently retained —
//! deformation ahead of separation in a fracture model: material yields before it tears, and the
//! yielded part never returns. A laceration is the ductile case, so the curve here is monotone and
//! has no closing half.
//!
//! # What the crate made up, and says so
//!
//! **The time constant is this crate's own.** Nothing in the corpus above gives a *rate* at which a
//! wound opens — those are quasi-static tensile tests, not high-speed video of a laceration — so
//! [`Gape::open_ticks`] is an authoring dial with an exponential shape chosen because it is the
//! response of a first-order system relaxing to a new equilibrium, which is the honest reading of
//! elastic retraction against tissue viscosity. It reaches 95 % of the final width at `open_ticks`
//! because `1 - e^-3 = 0.9502`, and the `3` in [`gape`] is exactly that choice made visible.
//!
//! **[`ALONG_LANGER_FACTOR`] is a stiffness ratio used as a gape proxy.** The paper measures moduli,
//! not wound widths; using the ratio of the two moduli as the ratio of the two gapes assumes the
//! retraction is the released strain of a linear spring, which is a modelling step the paper does
//! not take for us.

/// **The skin's resting tension where the cut is**, and which way its collagen runs.
///
/// Both are authored per subject and per site, because both genuinely vary that way: the same blade
/// on the same person opens a different wound on a shin than on a cheek.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tension {
    /// How tensioned the skin is here, `[0, 1]`. `0` is a slack, undermined flap that barely parts;
    /// `1` is skin over a joint in extension. Values outside the range are clamped by [`gape`]
    /// rather than refused, because a caller driving this from gameplay state should not be able to
    /// author a negative width.
    pub skin: f32,
    /// Unit direction of the Langer lines **in mesh space**, or `None` when the site's collagen
    /// direction is not authored — in which case [`anisotropy`] is `1.0` and the cut gapes the same
    /// whichever way it runs.
    ///
    /// Need not be exactly unit: [`anisotropy`] normalises it and treats a degenerate vector as
    /// `None`, so a hand-typed `[1.0, 1.0, 0.0]` behaves.
    pub langer: Option<[f32; 3]>,
}

impl Default for Tension {
    /// Middling tension, no authored line direction — the isotropic default, so a caller who has not
    /// yet decided gets a wound that opens the same in every direction rather than one that silently
    /// prefers an axis.
    fn default() -> Self {
        Self { skin: 0.5, langer: None }
    }
}

/// **The gape of a cut along the Langer lines, as a fraction of one across them: `63.8 / 112.5`.**
///
/// The ratio of the perpendicular to the parallel elastic modulus of middle-back human skin — Ní
/// Annaidh et al. 2012, `doi:10.1016/j.jmbbm.2011.08.016`. It is used here as a **proxy**: the paper
/// reports stiffnesses, not wound widths, and equating the ratio of moduli with the ratio of
/// retractions assumes the lips are released linear springs. That step is this crate's, not the
/// paper's.
pub const ALONG_LANGER_FACTOR: f32 = 63.8 / 112.5;

/// **How much of the full gape a cut in this direction earns**, `[ALONG_LANGER_FACTOR, 1]`.
///
/// `lerp(ALONG_LANGER_FACTOR, 1.0, sin²θ)` where θ is the angle between the tear and the Langer
/// line. `sin²θ` rather than `|θ|` because the quantity that varies is the component of the cut that
/// crosses fibres, and that component goes as the sine; squaring it keeps the function smooth at
/// both ends and needs no trigonometry, since `sin²θ = 1 - cos²θ` and `cos θ` is a dot product of
/// unit vectors.
///
/// A cut **along** the lines (θ = 0) returns [`ALONG_LANGER_FACTOR`]; **across** them (θ = 90°)
/// returns `1.0`. No authored line direction, a degenerate direction, or a non-finite one returns
/// `1.0` — the isotropic answer, which is the one that cannot silently shrink a wound a caller
/// asked for.
pub fn anisotropy(tear_dir: [f32; 3], tension: &Tension) -> f32 {
    let Some(langer) = tension.langer else {
        return 1.0;
    };
    let Some(l) = unit(langer) else {
        return 1.0;
    };
    let Some(t) = unit(tear_dir) else {
        return 1.0;
    };
    let cos = (t[0] * l[0] + t[1] * l[1] + t[2] * l[2]).clamp(-1.0, 1.0);
    let sin2 = 1.0 - cos * cos;
    ALONG_LANGER_FACTOR + (1.0 - ALONG_LANGER_FACTOR) * sin2
}

/// **How wide the wound ends up, and how long it takes to get there.**
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Gape {
    /// Full-open width in **mesh units** — the separation between the two lips at a tension of `1`
    /// on a cut straight across the Langer lines. Every other combination is a fraction of it.
    pub width_max: f32,
    /// Ticks to reach **95 %** of that width. The time curve's only scale, and **this crate's own
    /// number**: see the module docs for why no paper supplies one. Zero reads as one tick.
    pub open_ticks: u32,
}

impl Default for Gape {
    /// A 12 mm mouth at metre scale, opening over 60 ticks — one second at 60 Hz. Both are authoring
    /// defaults, not measurements.
    fn default() -> Self {
        Self { width_max: 0.012, open_ticks: 60 }
    }
}

/// **The wound's width right now**, in mesh units.
///
/// `width_max · skin · anisotropy · (1 - e^(-3t/open_ticks))`. Monotone non-decreasing in `t` and in
/// `tension.skin`, and exactly `0` at `t = 0` — a wound that has not started opening has no width,
/// which is what lets a caller spawn the component and let the clock do the rest.
///
/// Refuses to produce nonsense rather than propagating it: a non-finite or negative `width_max` is
/// `0`, `open_ticks == 0` is read as one tick (the alternative is a division by zero, and a
/// same-tick wound is what the caller meant), and `tension.skin` is clamped into `[0, 1]`.
pub fn gape(ticks_since_open: u32, g: &Gape, tension: &Tension, tear_dir: [f32; 3]) -> f32 {
    if !g.width_max.is_finite() || g.width_max <= 0.0 {
        return 0.0;
    }
    let skin = if tension.skin.is_finite() { tension.skin.clamp(0.0, 1.0) } else { 0.0 };
    if skin <= 0.0 || ticks_since_open == 0 {
        return 0.0;
    }
    // `max(1)` rather than a refusal: an author who writes `0` means "already open", and reading it
    // as one tick gets them 95 % on the first tick instead of a NaN.
    let span = g.open_ticks.max(1) as f32;
    let t = ticks_since_open as f32 / span;
    let opened = 1.0 - (-3.0 * t).exp();
    g.width_max * skin * anisotropy(tear_dir, tension) * opened
}

/// Normalise, or `None` for a zero-length or non-finite vector — the two cases where a direction
/// does not exist and pretending otherwise would divide by zero.
fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if !len2.is_finite() || len2 <= 1.0e-24 {
        return None;
    }
    let inv = len2.sqrt().recip();
    if !inv.is_finite() {
        return None;
    }
    Some([v[0] * inv, v[1] * inv, v[2] * inv])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two directions the anisotropy argument is about, in a mesh whose Langer lines run along `x`.
    const LANGER: [f32; 3] = [1.0, 0.0, 0.0];
    const ALONG: [f32; 3] = [1.0, 0.0, 0.0];
    const ACROSS: [f32; 3] = [0.0, 0.0, 1.0];

    fn tensioned(skin: f32) -> Tension {
        Tension { skin, langer: Some(LANGER) }
    }

    #[test]
    fn gape_is_zero_at_zero_and_monotone_in_time_and_tension() {
        let g = Gape { width_max: 0.02, open_ticks: 90 };
        for skin in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let tension = tensioned(skin);
            assert_eq!(gape(0, &g, &tension, ACROSS), 0.0, "a wound with no elapsed time has no width");
            let mut prev = 0.0;
            for t in 0..=600u32 {
                let w = gape(t, &g, &tension, ACROSS);
                assert!(w.is_finite(), "gape went non-finite at t={t}, skin={skin}");
                assert!(w >= prev - 1.0e-7, "gape shrank between t={} and t={t}: {prev} -> {w}", t.saturating_sub(1));
                assert!(w <= g.width_max + 1.0e-6, "gape exceeded width_max at t={t}: {w}");
                prev = w;
            }
        }
        // Monotone in tension at every sampled time, not just at the end.
        for t in [1u32, 10, 45, 90, 600] {
            let mut prev = -1.0;
            for skin in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
                let w = gape(t, &g, &tensioned(skin), ACROSS);
                assert!(w > prev || (skin == 0.0 && w == 0.0), "gape not monotone in tension at t={t}: {prev} -> {w}");
                prev = w;
            }
        }
        // 95 % at `open_ticks` is the whole claim of the `3` in the exponent.
        let at_span = gape(g.open_ticks, &g, &tensioned(1.0), ACROSS);
        assert!((at_span / g.width_max - 0.9502).abs() < 1.0e-3, "open_ticks must be the 95 % point, got {at_span}");
    }

    #[test]
    fn a_cut_across_the_langer_lines_gapes_more_than_one_along_them() {
        let g = Gape { width_max: 0.02, open_ticks: 60 };
        let tension = tensioned(1.0);
        let across = gape(60, &g, &tension, ACROSS);
        let along = gape(60, &g, &tension, ALONG);
        assert!(across > along, "a cut across the lines must gape more: across {across}, along {along}");
        assert!(
            (along / across - ALONG_LANGER_FACTOR).abs() < 1.0e-5,
            "the ratio must be the measured stiffness ratio {ALONG_LANGER_FACTOR}, got {}",
            along / across
        );
        // 45° sits between the two, at the sin² midpoint.
        let diagonal = gape(60, &g, &tension, [1.0, 0.0, 1.0]);
        assert!(diagonal > along && diagonal < across, "45° must land between the extremes: {diagonal}");
    }

    #[test]
    fn anisotropy_is_one_without_an_authored_line() {
        let none = Tension { skin: 1.0, langer: None };
        assert_eq!(anisotropy(ACROSS, &none), 1.0);
        assert_eq!(anisotropy(ALONG, &none), 1.0);
        // A degenerate or non-finite direction is the isotropic case, not a NaN.
        let degenerate = Tension { skin: 1.0, langer: Some([0.0, 0.0, 0.0]) };
        assert_eq!(anisotropy(ACROSS, &degenerate), 1.0);
        let nan = Tension { skin: 1.0, langer: Some([f32::NAN, 0.0, 0.0]) };
        assert_eq!(anisotropy(ACROSS, &nan), 1.0);
        assert_eq!(anisotropy([f32::INFINITY, 0.0, 0.0], &tensioned(1.0)), 1.0);
        // Sign does not matter: a line has no direction, only an orientation.
        assert_eq!(anisotropy(ALONG, &tensioned(1.0)), anisotropy([-1.0, 0.0, 0.0], &tensioned(1.0)));
    }

    #[test]
    fn bad_dials_produce_zero_rather_than_nonsense() {
        let tension = tensioned(1.0);
        assert_eq!(gape(10, &Gape { width_max: f32::NAN, open_ticks: 60 }, &tension, ACROSS), 0.0);
        assert_eq!(gape(10, &Gape { width_max: -1.0, open_ticks: 60 }, &tension, ACROSS), 0.0);
        // Zero ticks reads as one tick: 95 % immediately, and finite.
        let instant = gape(1, &Gape { width_max: 0.02, open_ticks: 0 }, &tension, ACROSS);
        assert!(instant.is_finite() && instant > 0.018, "open_ticks == 0 must open at once, got {instant}");
        // Out-of-range tension is clamped, not refused.
        let over = Tension { skin: 5.0, langer: Some(LANGER) };
        assert_eq!(gape(60, &Gape::default(), &over, ACROSS), gape(60, &Gape::default(), &tensioned(1.0), ACROSS));
        let under = Tension { skin: -5.0, langer: Some(LANGER) };
        assert_eq!(gape(60, &Gape::default(), &under, ACROSS), 0.0);
        assert_eq!(gape(60, &Gape::default(), &Tension { skin: f32::NAN, langer: None }, ACROSS), 0.0);
    }
}
