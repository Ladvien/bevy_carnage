//! **A burn, as the heat that made it.** One-dimensional layered bioheat conduction, and an
//! Arrhenius damage integral per node.
//!
//! # Why a heat equation rather than a depth
//!
//! A burn is the one injury in this crate whose severity is **not** a function of geometry. Nothing
//! is cut, nothing is displaced; what varies is how long tissue spent above a temperature, and the
//! depth of the injury is the *output* of that history rather than an input to it. So a "third-degree
//! burn" cannot be authored as a depth without discarding the only thing that decides it: a 200 °C
//! contact for 200 ms and a 55 °C contact for a minute are different injuries, and only a rate
//! process distinguishes them.
//!
//! That also makes the interesting behaviour free. **Damage keeps accruing after the heat is gone**,
//! because the tissue is still hot — which is why a burn worsens for a while after contact ends, and
//! why cooling it early matters. Here that is not a rule; it is what happens when [`Burn::expose`] is
//! called a second time at a lower temperature.
//!
//! # The model: Pennes conduction, Gowrishankar's layers, Henriques' rate process
//!
//! Gowrishankar, Stewart, Martin & Weaver, *"Transport lattice models of heat transport in skin with
//! spatially heterogeneous, temperature-dependent perfusion"*, BioMedical Engineering OnLine 3:42
//! (2004), `doi:10.1186/1475-925x-3-42`. Their three-layer skin — non-perfused viable epidermis over
//! perfused dermis and subcutis — its thermal properties, its surface-contact boundary condition
//! (33 °C at rest, stepped to the contact temperature at `t = 0`, core held at 37 °C), and its damage
//! estimate are what this module is:
//!
//! ```text
//! ρc ∂T/∂t = ∂/∂z(k ∂T/∂z) + ρ_b c_b ω (T_a − T)          (Pennes)
//! dΩ/dt    = A exp(−ΔE / ℜT),  integrated only where T ≥ 42 °C
//! ```
//!
//! with `A` = [`ARRHENIUS_A`], `ΔE` = [`ACTIVATION_J_MOL`] and `ℜ` = [`GAS_CONSTANT`], all three
//! quoted from their Eq. 7, and the 42 °C onset from the same place (their refs 57, 58). They state
//! the reading of the indicator this module's [`Degree`] rests on: "complete epidermal necrosis
//! corresponds to Ω = 1".
//!
//! # The thresholds are not in this crate's corpus, and that is said rather than hidden
//!
//! [`OMEGA_FIRST`] `= 0.53` and [`OMEGA_THIRD`] `= 1e4` are the classical burn-degree thresholds
//! attributed to Henriques' rate-process analysis. **Neither value appears in any paper in this
//! crate's local corpus** — searched: this paper, `doi:10.2174/1874120701105010047` and
//! `doi:10.3390/ma18153524`, both thermal-damage reviews. Only `Ω = 1` is sourced, above. So the
//! other two are cited by reference to Moritz & Henriques, *"Studies of thermal injury II: the
//! relative importance of time and surface temperature in the causation of cutaneous burns"*, Am J
//! Pathol 26 (1947) 695–720 — which is Gowrishankar's own reference 55 — and **flagged: threshold
//! values not in corpus**.
//!
//! One consequence is worth stating because it will look like a bug otherwise. `A` and `ΔE` come
//! from Weaver's and Lee's kinetics; the thresholds come from Henriques'. They are **different rate
//! processes**, so the thresholds land early against these constants: at exactly the 42 °C onset,
//! `A exp(−ΔE/ℜT)` is `4.5e-3 s⁻¹`, so `Ω = 1` accrues in about 222 s of contact. Clinically 42 °C
//! is tolerated far longer than that. The mismatch is in the literature, not in this code, and the
//! honest thing is to name it rather than to quietly retune someone's published constant.

use crate::bloodstain::m;

/// Nodes across the modelled depth: `0` at the surface, [`NODES`]` − 1` at [`DEPTH_MM`].
pub const NODES: usize = 51;
/// Node spacing, mm.
pub const DZ_MM: f32 = 0.1;
/// Modelled depth, mm.
pub const DEPTH_MM: f32 = DZ_MM * (NODES - 1) as f32;

/// **The fixed substep, seconds.**
///
/// # The stability bound, stated because it is what licenses an explicit scheme
///
/// Explicit forward-difference conduction is stable while `α Δt / Δz² ≤ ½`. The fastest layer here
/// is the dermis, `α = k/ρc = 0.45 / (1200 × 3300) = 1.14 × 10⁻⁷ m²/s`, so with `Δz = 100 µm` the
/// bound is `Δt ≤ 0.044 s`. At `0.02 s` the diffusion number is `0.23` — inside the bound by a
/// factor of two, which leaves room for a caller to author a thinner epidermis or a different
/// conductivity without silently crossing it. The Pennes perfusion term adds
/// `ρ_b c_b ω / ρc ≈ 1.3 × 10⁻³ s⁻¹` to the same denominator, four orders below the conduction term,
/// so it does not move the bound.
///
/// Fifty substeps to the second, and an exposure's step count is an integer — the crate's no-clocks
/// rule in seconds rather than in ticks.
pub const DT_S: f32 = 0.02;

/// Resting skin surface temperature, °C (Gowrishankar: "the surface is initially set to 33 °C").
pub const SURFACE_REST_C: f32 = 33.0;
/// Core and arterial temperature, °C — the deep Dirichlet boundary and the Pennes sink.
pub const CORE_C: f32 = 37.0;
/// Onset temperature for thermal damage, °C (ibid., after Lee and co-workers).
pub const ONSET_C: f32 = 42.0;

/// Arrhenius attempt rate `A`, s⁻¹ (ibid., Eq. 7).
pub const ARRHENIUS_A: f32 = 2.9e37;
/// `ln(A)`, carried as a constant so the rate is one exponential and never a subnormal.
///
/// `exp(−ΔE/ℜT)` at these constants is around `1e-40`, which in `f32` is **subnormal** — 17 bits of
/// mantissa instead of 24 — and only recovers its precision after being multiplied by `A`. Folding
/// the multiply into the exponent, `exp(ln A − ΔE/ℜT)`, keeps every intermediate normal. Checked
/// against [`ARRHENIUS_A`] by [`tests::the_attempt_rate_constant_round_trips`].
pub const LN_ARRHENIUS_A: f32 = 86.260_36;
/// Effective activation energy `ΔE`, J/mol (ibid., Eq. 7).
pub const ACTIVATION_J_MOL: f32 = 2.4e5;
/// Universal gas constant `ℜ` as that paper quotes it, J/(mol·K).
pub const GAS_CONSTANT: f32 = 8.31;

/// `Ω` at which a burn reads as first-degree. **Not in this crate's corpus** — see the module docs.
pub const OMEGA_FIRST: f32 = 0.53;
/// `Ω` at which a burn reads as second-degree: complete epidermal necrosis, which *is* sourced
/// (Gowrishankar, "complete epidermal necrosis corresponds to Ω = 1").
pub const OMEGA_SECOND: f32 = 1.0;
/// `Ω` at which a burn reads as third-degree. **Not in this crate's corpus** — see the module docs.
pub const OMEGA_THIRD: f32 = 1.0e4;

/// Blood specific heat, J/(kg·°C) (Gowrishankar Table 1).
pub const BLOOD_SPECIFIC_HEAT: f32 = 3770.0;
/// Blood density, kg/m³ (ibid.). The same 1060 [`crate::bloodstain::droplet::BLOOD_DENSITY`] carries, from a
/// different paper — which is a small piece of evidence that both tables are quoting the same fluid.
pub const BLOOD_DENSITY: f32 = 1060.0;

/// Thickness of the epidermis, mm. **This crate's own.**
///
/// Gowrishankar's Table 1 has a row for it, but that row is not legible in this crate's corpus
/// extraction of the paper (the table is truncated above the epidermal conductivity). 100 µm is an
/// ordinary interfollicular epidermis and lands on exactly one node at [`DZ_MM`], so the basal layer
/// — where a burn's degree is read — is a node rather than an interpolation.
pub const EPIDERMIS_MM: f32 = 0.1;
/// Thickness of the dermis, mm (Gowrishankar Table 1: `t_d` = 2000 µm).
pub const DERMIS_MM: f32 = 2.0;

/// Thermal properties of one layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tissue {
    /// Thermal conductivity, W/(m·°C).
    pub k: f32,
    /// Density, kg/m³.
    pub rho: f32,
    /// Specific heat, J/(kg·°C).
    pub c: f32,
    /// Volumetric perfusion rate, s⁻¹ — blood volume per tissue volume per second.
    pub perfusion: f32,
}

/// Epidermis (Gowrishankar Table 1: `ρ_e` = 1200, `c_e` = 3590, `ω_e` = 0 — it is not perfused).
///
/// **`k` is this crate's own**, for the same reason [`EPIDERMIS_MM`] is: the conductivity row is
/// truncated in this crate's corpus extraction. `0.21 W m⁻¹ °C⁻¹` is the value the burn literature
/// uses for a keratinised, avascular layer, and it is below the dermis' 0.45, which is the property
/// that matters here — the epidermis is the slower conductor of the two.
pub const EPIDERMIS: Tissue = Tissue { k: 0.21, rho: 1200.0, c: 3590.0, perfusion: 0.0 };
/// Dermis (ibid.: `k_d` = 0.45, `ρ_d` = 1200, `c_d` = 3300, `ω_d` = 1.25 × 10⁻³).
pub const DERMIS: Tissue = Tissue { k: 0.45, rho: 1200.0, c: 3300.0, perfusion: 1.25e-3 };
/// Subcutaneous tissue (ibid.: `k_f` = 0.19, `ρ_f` = 1000, `c_f` = 2675, `ω_f` = 1.25 × 10⁻³).
pub const SUBCUTIS: Tissue = Tissue { k: 0.19, rho: 1000.0, c: 2675.0, perfusion: 1.25e-3 };

/// **How bad the burn reads.** Four states at three thresholds on `Ω` at the basal layer.
///
/// The basal layer — the epidermal/dermal junction — is the site the clinical grades are stated
/// for, and it is the node [`EPIDERMIS_MM`] lands on. See the module docs for which of the three
/// thresholds is sourced and which two are not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Degree {
    /// Below [`OMEGA_FIRST`]: warm, perhaps painful, not injured.
    None,
    /// Erythema without necrosis.
    First,
    /// Complete epidermal necrosis — blistering. The one sourced threshold.
    Second,
    /// Full-thickness destruction.
    Third,
}

/// **Skin under heat.** A temperature profile, a damage integral per node, and an integer substep
/// count.
#[derive(Clone, Debug, PartialEq)]
pub struct Burn {
    temp_c: [f32; NODES],
    omega: [f32; NODES],
    steps: u32,
}

impl Default for Burn {
    fn default() -> Self {
        Self::new()
    }
}

/// The tissue at a depth.
///
/// A `match` on two thicknesses rather than a stored per-node table: the layer boundaries are
/// authored constants, so a table would be a second copy of them.
#[inline]
pub fn tissue_at(depth_mm: f32) -> Tissue {
    if depth_mm < EPIDERMIS_MM {
        EPIDERMIS
    } else if depth_mm < EPIDERMIS_MM + DERMIS_MM {
        DERMIS
    } else {
        SUBCUTIS
    }
}

/// **The Arrhenius damage rate at a temperature, s⁻¹.** Exactly zero below [`ONSET_C`].
///
/// The onset is a hard gate rather than a soft one because that is how Gowrishankar state it — "for
/// times with T ≥ 42 °C" — and because a rate process with no floor accrues a little damage at body
/// temperature forever, which would make `Ω` a clock instead of an injury.
#[inline]
pub fn damage_rate(temp_c: f32) -> f32 {
    if !temp_c.is_finite() || temp_c < ONSET_C {
        return 0.0;
    }
    let kelvin = temp_c + 273.15;
    if kelvin <= 0.0 {
        return 0.0;
    }
    m::exp(LN_ARRHENIUS_A - ACTIVATION_J_MOL / (GAS_CONSTANT * kelvin))
}

/// Which node a depth reads from, saturating at the deepest rather than escaping the grid.
#[inline]
fn node_of(depth_mm: f32) -> usize {
    if !depth_mm.is_finite() || depth_mm <= 0.0 {
        return 0;
    }
    let i = m::round(depth_mm / DZ_MM) as usize;
    if i >= NODES { NODES - 1 } else { i }
}

impl Burn {
    /// **Unburnt skin**: 33 °C at the surface, 37 °C at depth, linear between, no damage anywhere.
    ///
    /// The two endpoints are Gowrishankar's pre-contact condition. The linear interior is this
    /// crate's own and it is the honest one: their resting profile is set by a surface heat-transfer
    /// coefficient this module does not model, and any curve fitted to look like theirs would be a
    /// guess dressed as a solution. It relaxes within a few seconds of simulated time anyway, and it
    /// is everywhere below [`ONSET_C`], so it contributes no damage while it does.
    pub fn new() -> Self {
        let mut temp_c = [CORE_C; NODES];
        let last = (NODES - 1) as f32;
        for (i, t) in temp_c.iter_mut().enumerate() {
            let f = i as f32 / last;
            *t = SURFACE_REST_C + (CORE_C - SURFACE_REST_C) * f;
        }
        Self { temp_c, omega: [0.0; NODES], steps: 0 }
    }

    /// Substeps taken.
    pub fn steps(&self) -> u32 {
        self.steps
    }

    /// Elapsed simulated time, seconds — derived from the integer substep count.
    pub fn seconds(&self) -> f32 {
        self.steps as f32 * DT_S
    }

    /// Temperature at a node, with an out-of-range read answering the core temperature.
    #[inline]
    fn t(&self, i: usize) -> f32 {
        match self.temp_c.get(i) {
            Some(v) => *v,
            None => CORE_C,
        }
    }

    /// **Hold the surface at `temp_c` for `seconds`.**
    ///
    /// `seconds` becomes an integer count of [`DT_S`] substeps, rounded once at the door; nothing
    /// accumulates a float clock. A non-finite argument is refused outright rather than turned into
    /// a plausible-looking exposure.
    ///
    /// **Cooling is the same call.** `expose(33.0, 20.0)` after a contact is twenty seconds with the
    /// skin back against room-temperature air, and the damage that accrues during it — because the
    /// dermis is still above 42 °C — is the reason a burn is worse than it looked at the moment
    /// contact ended.
    ///
    /// **A contact shorter than half a substep does nothing, and says so here rather than in a
    /// log.** `seconds` is rounded to a whole count of [`DT_S`] steps; below `DT_S / 2` that count
    /// is zero and the model is unchanged — an exposure the scheme cannot represent is refused, not
    /// rounded up to a whole step it never had, which would be a fabricated injury in the other
    /// direction. A caller with a 5 ms flash accumulates it into a longer exposure itself.
    pub fn expose(&mut self, temp_c: f32, seconds: f32) {
        if !temp_c.is_finite() || !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        let steps = m::round(seconds / DT_S);
        if !steps.is_finite() || steps < 1.0 {
            return;
        }
        let steps = steps as u32;
        for _ in 0..steps {
            self.substep(temp_c);
        }
    }

    /// One [`DT_S`] substep: conduction and perfusion, then damage on the temperatures that came
    /// out of it.
    ///
    /// The interface conductivity is the **harmonic** mean of the two nodes', which is the one that
    /// gets a layer boundary right: two slabs in series add their thermal *resistances*, and an
    /// arithmetic mean of the conductivities would make the epidermis/dermis interface conduct like
    /// neither of them.
    fn substep(&mut self, surface_c: f32) {
        let mut next = self.temp_c;
        for i in 1..NODES.saturating_sub(1) {
            let depth = i as f32 * DZ_MM;
            let here = tissue_at(depth);
            let above = tissue_at(depth - DZ_MM);
            let below = tissue_at(depth + DZ_MM);
            let face = |a: f32, b: f32| {
                let s = a + b;
                if s > 0.0 { 2.0 * a * b / s } else { 0.0 }
            };
            let k_up = face(here.k, above.k);
            let k_dn = face(here.k, below.k);
            let dz_m = DZ_MM * 1.0e-3;
            let t_here = self.t(i);
            let flux = (k_dn * (self.t(i + 1) - t_here) - k_up * (t_here - self.t(i - 1)))
                / (dz_m * dz_m);
            let perfusion =
                BLOOD_DENSITY * BLOOD_SPECIFIC_HEAT * here.perfusion * (CORE_C - t_here);
            let capacity = here.rho * here.c;
            if capacity > 0.0 {
                if let Some(cell) = next.get_mut(i) {
                    *cell = t_here + DT_S * (flux + perfusion) / capacity;
                }
            }
        }
        if let Some(cell) = next.get_mut(0) {
            *cell = surface_c;
        }
        if let Some(cell) = next.get_mut(NODES - 1) {
            *cell = CORE_C;
        }
        self.temp_c = next;

        for (o, t) in self.omega.iter_mut().zip(self.temp_c.iter()) {
            *o += damage_rate(*t) * DT_S;
        }
        self.steps = self.steps.saturating_add(1);
    }

    /// **Accumulated damage at a depth**, dimensionless `Ω`.
    pub fn omega_at(&self, depth_mm: f32) -> f32 {
        match self.omega.get(node_of(depth_mm)) {
            Some(v) => *v,
            None => 0.0,
        }
    }

    /// Temperature at a depth, °C.
    pub fn temperature_at(&self, depth_mm: f32) -> f32 {
        self.t(node_of(depth_mm))
    }

    /// **How deep the damage reaches**, mm: the deepest node whose `Ω` is at or above `threshold`.
    ///
    /// `0.0` when nothing reaches it — the surface node is not "damaged to zero depth", it is
    /// undamaged, and both read as zero because a burn with no depth is what that is. Monotone
    /// non-decreasing in exposure, because `Ω` is.
    pub fn depth_of(&self, threshold: f32) -> f32 {
        let mut deepest = 0.0f32;
        for (i, o) in self.omega.iter().enumerate() {
            if *o >= threshold {
                deepest = i as f32 * DZ_MM;
            }
        }
        deepest
    }

    /// **The burn's degree**, read at the basal layer. See [`Degree`].
    pub fn degree(&self) -> Degree {
        let omega = self.omega_at(EPIDERMIS_MM);
        if omega >= OMEGA_THIRD {
            Degree::Third
        } else if omega >= OMEGA_SECOND {
            Degree::Second
        } else if omega >= OMEGA_FIRST {
            Degree::First
        } else {
            Degree::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// The folded constant is the paper's constant. If this drifts, every `Ω` in the module is
    /// scaled by the same wrong factor and nothing else would say so.
    #[test]
    fn the_attempt_rate_constant_round_trips() {
        let got = m::exp(LN_ARRHENIUS_A);
        let rel = (got - ARRHENIUS_A) / ARRHENIUS_A;
        assert!(rel.abs() < 1.0e-3, "exp(ln A) = {got}, A = {ARRHENIUS_A}, relative error {rel}");
    }

    /// **Damage never unhappens.** `Ω` is an integral of a positive rate, so it is non-decreasing in
    /// exposure at every depth — and a model where a burn could heal by being held longer would be
    /// indistinguishable from a bug at exactly the moment a player is looking at it.
    #[test]
    fn omega_is_monotone_in_exposure_time() {
        let mut b = Burn::new();
        let depths = [0.0f32, 0.1, 0.5, 1.0, 2.0, 3.0];
        let mut last = [0.0f32; 6];
        for _ in 0..20 {
            b.expose(70.0, 0.5);
            for (d, prev) in depths.iter().zip(last.iter_mut()) {
                let now = b.omega_at(*d);
                assert!(now >= *prev, "Ω fell at {d} mm: {now} after {prev}");
                *prev = now;
            }
        }
        assert!(last[0] > 0.0, "ten seconds against 70 °C must damage the surface");
    }

    /// Heat arrives from the surface, so the damage cannot be worse deeper down. This is the claim
    /// that makes [`Burn::depth_of`] meaningful: a front, not a scatter.
    #[test]
    fn omega_falls_with_depth() {
        let mut b = Burn::new();
        b.expose(120.0, 4.0);
        let mut last = f32::INFINITY;
        for i in 0..NODES {
            let o = b.omega_at(i as f32 * DZ_MM);
            assert!(o <= last + 1.0e-6, "Ω rose with depth at node {i}: {o} after {last}");
            last = o;
        }
    }

    /// The burn's *depth* is monotone too, and it is the quantity a caller renders. Held against a
    /// hot surface, the necrosis front advances and never retreats.
    #[test]
    fn the_necrosis_depth_is_monotone_in_time() {
        let mut b = Burn::new();
        let mut last = 0.0f32;
        for _ in 0..30 {
            b.expose(150.0, 0.5);
            let d = b.depth_of(OMEGA_SECOND);
            assert!(d >= last, "the necrosis front retreated: {d} mm after {last} mm");
            last = d;
        }
        assert!(last > 0.0, "fifteen seconds against 150 °C must necrose something");
    }

    /// **Hotter and longer is worse, and the four grades come out in order.** A rate process is what
    /// makes that automatic: nothing here maps a temperature to a degree, and the four exposures
    /// below are ordered by *dose* rather than by either variable alone — 30 s at 55 °C outranks
    /// 30 s at 48 °C, and 1 s at 150 °C outranks both, which is the time/temperature trade the whole
    /// Arrhenius integral exists to express.
    #[test]
    fn the_degree_climbs_with_the_exposure() {
        let after = |temp: f32, seconds: f32| {
            let mut b = Burn::new();
            b.expose(temp, seconds);
            b.degree()
        };
        let none = after(44.0, 20.0);
        let first = after(48.0, 30.0);
        let second = after(55.0, 30.0);
        let third = after(150.0, 1.0);
        assert_eq!(none, Degree::None, "20 s at 44 °C should not read as a burn");
        assert_eq!(first, Degree::First, "30 s at 48 °C should be erythema, got {first:?}");
        assert_eq!(second, Degree::Second, "30 s at 55 °C should blister, got {second:?}");
        assert_eq!(third, Degree::Third, "1 s at 150 °C should be full thickness, got {third:?}");
        assert!(none < first && first < second && second < third, "the grades must be ordered");
    }

    /// **A burn keeps burning after contact ends** — the behaviour the whole heat equation is here
    /// for. Twenty seconds of room-temperature air after the contact still deepens the injury,
    /// because the dermis is still above the onset temperature.
    #[test]
    fn damage_accrues_after_the_heat_is_removed() {
        let mut b = Burn::new();
        b.expose(200.0, 1.0);
        let at_contact = b.omega_at(1.0);
        b.expose(SURFACE_REST_C, 20.0);
        let after = b.omega_at(1.0);
        assert!(
            after > at_contact,
            "Ω at 1 mm was {at_contact} when contact ended and {after} twenty seconds later — the \
             stored heat did nothing"
        );
    }

    /// Body temperature is not an injury. The onset gate is what keeps `Ω` from being a clock.
    #[test]
    fn resting_skin_accrues_nothing() {
        let mut b = Burn::new();
        b.expose(SURFACE_REST_C, 600.0);
        assert_eq!(b.omega_at(0.0), 0.0, "ten minutes of resting skin must accrue no damage");
        assert_eq!(b.degree(), Degree::None);
        for i in 0..NODES {
            let t = b.temperature_at(i as f32 * DZ_MM);
            assert!(t <= CORE_C + 1.0e-3, "node {i} warmed itself to {t} °C with no heat source");
        }
    }

    /// A refused argument changes nothing, rather than being turned into a plausible exposure.
    #[test]
    fn a_nonsense_exposure_is_refused() {
        let mut b = Burn::new();
        let before = b.clone();
        for (t, s) in [(f32::NAN, 1.0), (100.0, f32::INFINITY), (100.0, -1.0), (100.0, 0.0)] {
            b.expose(t, s);
        }
        assert_eq!(b, before, "a non-finite or empty exposure must leave the model untouched");
    }

    /// **Frozen.** A two-phase history — one second against 200 °C, then five seconds of cooling —
    /// sampled as temperature and `Ω` at four depths.
    ///
    /// A lock rather than a snapshot. Gowrishankar's layer properties, the harmonic interface, the
    /// substep, the boundary conditions, the folded `ln A` and the onset gate are all upstream of
    /// these bits.
    #[test]
    fn the_burn_model_is_frozen() {
        let mut b = Burn::new();
        let mut got: Vec<u32> = Vec::new();
        b.expose(200.0, 1.0);
        for d in [0.0f32, 0.1, 0.5, 1.5] {
            got.push(b.temperature_at(d).to_bits());
            got.push(b.omega_at(d).to_bits());
        }
        b.expose(SURFACE_REST_C, 5.0);
        for d in [0.0f32, 0.1, 0.5, 1.5] {
            got.push(b.temperature_at(d).to_bits());
            got.push(b.omega_at(d).to_bits());
        }
        got.push(b.depth_of(OMEGA_SECOND).to_bits());
        std::println!("{got:?}");
        let want: Vec<u32> = std::vec![
            0x43480000, 0x51a7420b, 0x431dd376, 0x4c4f3664, 0x4295d43d, 0x4038b11f, 0x42097d31,
            0x00000000, 0x42040000, 0x51a7420b, 0x420b2b65, 0x4c6223f6, 0x421afcaa, 0x420cf2e2,
            0x4223c680, 0x00000000, 0x3f19999a,
        ];
        assert_eq!(got, want);
    }
}
