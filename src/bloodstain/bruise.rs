//! **A bruise, as the chemistry that makes one.** Haemoglobin leaks, is eaten, becomes bilirubin,
//! and the colour follows from the two concentrations rather than from a ramp between two swatches.
//!
//! # Why a kinetics rather than a colour walk
//!
//! [`crate::bloodstain::dry`] walks blood *outside* the body through a published sequence of oxidation stops.
//! A bruise is the opposite problem: the blood is *inside*, nothing evaporates, and the colours a
//! player recognises — red, then purple, then green-yellow, then gone — are the visible trace of
//! **two chromophores at different concentrations moving at different speeds**. A ramp cannot say
//! that, because the yellow is not a later shade of the red: it is a *different molecule*, produced
//! from the red one, diffusing four times faster, and outliving it. That is why a two-week-old
//! bruise has a yellow *halo wider than* its red core, and why the halo is what dates it.
//!
//! # The model: Stam et al. 2010, reduced to one radial dimension
//!
//! Stam, van Gemert, van Leeuwen & Aalders, *"3D finite compartment modeling of formation and
//! healing of bruises may identify methods for age determination of bruises"*, Med Biol Eng Comput
//! 48 (2010), `doi:10.1007/s11517-010-0647-5`. Their model is a 100 × 100 × 3 compartment grid
//! stepped at Δt = 0.1 h; four mechanisms move or consume the chromophores, and this module keeps
//! all four:
//!
//! 1. **Darcy convection** across the subcutis/dermis border, `v = K·Δp`, carrying whole blood out
//!    of the pool. The ruptured vessels close, so it **stops at 12 h** (their §"Simulations and
//!    parameters", citing their refs 18 and 30).
//! 2. **Fick diffusion** of both chromophores, horizontally *and* vertically — the addition that
//!    made their model spatial, and the reason a front exists to measure.
//! 3. **Michaelis–Menten** conversion of haemoglobin by heme oxygenase-1, `V_max·[HO]·c/(K_m + c)`.
//!    At bruise concentrations `c ≫ K_m`, so it runs saturated and is effectively zero-order until
//!    the pool is nearly exhausted — which is why the red fades at a near-constant rate.
//! 4. **Bilirubin production at 4 mol per mol of haemoglobin** (four hemes to a tetramer), then
//!    lymphatic clearance on a 240 h time constant.
//!
//! The reduction to **one radial dimension** is exact for the circularly symmetric bruise their
//! Fig. 3 discusses, which is the only case a caller can author without supplying a bruise-shaped
//! image. Their 100 × 100 lateral grid becomes [`SHELLS`] annuli over [`RADIUS_MM`], and their three
//! layers stay three: dermis top, dermis bottom, subcutis. Convection is `3 → 2` only; layer 1 is
//! reached by diffusion, exactly as their Eq. 1 specifies ("neglecting the convection from the
//! subcutaneous layer to the top layer of the dermis").
//!
//! # The colour
//!
//! The top 400 µm of dermis is the layer light comes back out of, so it is the layer whose
//! absorption decides the colour. Its `μa(λ)` is the haemoglobin absorption of [`crate::bloodstain::spectral`]
//! — Bosschaart et al. 2014, `doi:10.1007/s10103-013-1446-7`, mixed at venous saturation, because
//! extravasated blood is not being oxygenated — scaled by how much haemoglobin is actually there
//! against the whole-blood 150 g/l the table was measured at, **plus** a bilirubin band. That total
//! goes through the same Kubelka–Munk layer solution [`crate::bloodstain::spectral::kubelka_munk`] uses, over an
//! authored skin substrate, and out through the CIE observer.
//!
//! So an unbruised patch is the substrate, a fresh bruise is the substrate seen through blood, and
//! an old one is the substrate seen through bilirubin. Nothing is authored per age.
//!
//! # What is this crate's own
//!
//! Stam's Table 1 is the source for every kinetic constant but one and for the two dermal
//! thicknesses. Six numbers are **not** in it and say so here rather than hiding in a literal:
//!
//! - [`Params::clearance_h`] — 240 h is Randeberg's figure as Stam quote it in their discussion;
//!   their own Table 1 column is 150 h. Inside the paper's range, but not the paper's column.
//! - [`Params::subcutis_mm`] — their Table 1 gives dermal thicknesses only.
//! - [`DERMIS_MUSP_MM`] — Bosschaart's tables are *blood*, not dermis, so the scattering of the
//!   layer the blood sits in has no source in this crate's corpus.
//! - [`BILIRUBIN_EPS_PEAK`], [`BILIRUBIN_PEAK_NM`], [`BILIRUBIN_FWHM_NM`] — a Gaussian stand-in for
//!   a spectrum that is not tabulated here.
//! - [`Params::substrate`] — a skin tone is authored by a caller, and the default is deliberately
//!   neutral so `a*` and `b*` measure the *bruise*.
//! - [`Params::ho_induction_h`] — Stam fit a relaxation time this corpus cannot read; its own field
//!   carries the whole argument.
//!
//! # Two things this model does not do, because its source does not
//!
//! **A bruise is red the moment it is made, and this one is not.** Stam say why in as many words:
//! "although the dermis also contains small vessels, we assume that the blood comes from the
//! subcutaneous layer, therefore, the contribution of ruptured dermal vessels is neglected". So all
//! the haemoglobin the colour can see has to *arrive* — by Darcy convection and then by diffusion
//! across half a millimetre at `0.01 mm²/h` — and the reddest moment lands a day or two in, not at
//! `t = 0`. A caller who wants day-zero red draws the impact's own blood over the top of this;
//! that is [`crate::bloodstain::stain`]'s job and this module should not be guessing at it.
//!
//! **A fresh bruise is not blue here.** Deep blood reads blue-purple because *scattering* between
//! it and the eye is wavelength-dependent — the reason a vein looks blue — and that is a
//! multi-layer transport effect, not an absorption one. The single Kubelka–Munk layer here cannot
//! produce it, and authoring a blue would be a colour dressed as a computation.

use crate::bloodstain::m;
use crate::bloodstain::spectral::{self, SAMPLES, SO2_VENOUS, TABLE};

/// Radial shells the bruise is resolved into.
pub const SHELLS: usize = 64;
/// Outer radius of the grid, mm. Far beyond any bruise's reach on the 240 h+ timescale, so the
/// outermost shell stays empty and the no-flux outer face never has to be argued about.
pub const RADIUS_MM: f32 = 60.0;
/// Shell width, mm.
pub const DR_MM: f32 = RADIUS_MM / SHELLS as f32;
/// **The fixed step, hours.** Stam's own: "for a time step of 0.1 h, a simulation of 400 h takes
/// 1 min". Nothing here reads a clock and nothing accumulates a float age — [`Bruise::steps`] is an
/// integer count and [`Bruise::hours`] multiplies it, which is the crate's tick rule in another
/// unit.
pub const STEP_H: f32 = 0.1;
/// Compartment layers: dermis top, dermis bottom, subcutis.
pub const LAYERS: usize = 3;

/// Starting haemoglobin concentration of whole blood, g/l (Stam Table 1, standard column).
pub const HB_START_G_L: f32 = 150.0;
/// Molar mass of haemoglobin, g/mol. Textbook; Stam's Eq. 1 needs it to turn the molar
/// Michaelis–Menten rate back into a mass.
pub const MW_HB_G_MOL: f32 = 64_500.0;
/// Moles of bilirubin produced per mole of haemoglobin — four hemes to a tetramer, and Stam's
/// stated reason for scaling `D_B` off `D_Hb`.
pub const BILIRUBIN_PER_HB: f32 = 4.0;
/// Hours after which the ruptured subcutaneous vessels have closed and convection is zero
/// (Stam §"Simulations and parameters", their refs 18 and 30).
pub const CONVECTION_STOP_H: f32 = 12.0;

/// Reduced scattering coefficient of dermis, mm⁻¹. **This crate's own.**
///
/// [`crate::bloodstain::spectral::TABLE`] carries Bosschaart's *whole blood*; the dermis the blood is sitting in
/// is a different medium and its scattering is not in this crate's corpus. `2.0 mm⁻¹` is flat across
/// the visible band — real dermis falls with wavelength — and it is a scale rather than a spectrum:
/// it sets how deep the 400 µm layer looks, not what colour it is, because the colour comes out of
/// `μa`.
pub const DERMIS_MUSP_MM: f32 = 2.0;

/// Peak molar absorptivity of bilirubin, M⁻¹cm⁻¹. **This crate's own** — see the module docs.
pub const BILIRUBIN_EPS_PEAK: f32 = 55_000.0;
/// Centre of the bilirubin absorption band, nm. **This crate's own.**
pub const BILIRUBIN_PEAK_NM: f32 = 460.0;
/// Full width at half maximum of that band, nm. **This crate's own.**
pub const BILIRUBIN_FWHM_NM: f32 = 60.0;

/// Which compartment a concentration is read from.
///
/// [`Compartment::DermisTop`] is the one that decides the colour, and the default every reader that
/// does not say otherwise takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Compartment {
    /// The top 400 µm of dermis — the layer light returns through.
    DermisTop,
    /// The rest of the dermis.
    DermisBottom,
    /// Subcutaneous fat: where the pool is, and where the convection starts.
    Subcutis,
}

impl Compartment {
    #[inline]
    const fn index(self) -> usize {
        match self {
            Self::DermisTop => 0,
            Self::DermisBottom => 1,
            Self::Subcutis => 2,
        }
    }
}

/// Which chromophore a front is measured on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Chromophore {
    /// Extravasated haemoglobin: the red core.
    Hemoglobin,
    /// Bilirubin: the yellow halo, wider because `D_B` is four times `D_Hb`.
    Bilirubin,
}

/// **Stam Table 1's standard column**, plus the four numbers that column does not contain.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Params {
    /// Diameter of the initial subcutaneous pool, mm. Table 1: 10 mm standard, 2–100 mm biological.
    pub pool_diameter_mm: f32,
    /// Haemoglobin concentration of that pool, g/l. Table 1: 150 g/l.
    pub hb_start_g_l: f32,
    /// Diffusivity of haemoglobin, mm²/h. Table 1: `1 × 10⁻⁸ m²/h` = `0.01 mm²/h`, inside a
    /// biological range of `1 × 10⁻⁷`–`1 × 10⁻⁹`.
    pub d_hb_mm2_h: f32,
    /// Diffusivity of bilirubin, mm²/h. Table 1: `4 × 10⁻⁸ m²/h` = `0.04 mm²/h` — four times
    /// haemoglobin's, scaled by Stam off the 4:1 molar production.
    pub d_bil_mm2_h: f32,
    /// Hydraulic conductivity across the subcutis/cutis border, m³/(N·h). Table 1: `5 × 10⁻⁹`.
    pub hydraulic_conductivity: f32,
    /// Pressure difference driving the leak, N/m². Table 1: `2.6 × 10²`.
    pub pressure_difference_pa: f32,
    /// Michaelis affinity `K_m`, µmol/l. Table 1: 0.24 µM.
    pub km_umol_l: f32,
    /// Speed of conversion `V_max`, µmol/(h·mg HO). Table 1: 3.4.
    pub v_max_umol_h_mg: f32,
    /// Heme oxygenase-1 concentration, mg/l. Table 1: 5 mg/l standard, 0.1–10 range — which Stam
    /// sets at 10–100× the normal serum value, because HO-1 is upregulated in a wound.
    pub ho_mg_l: f32,
    /// Bilirubin clearance time constant, hours.
    ///
    /// **240 h, and it is first-order.** Stam quote Randeberg's clearance time as "in the order of
    /// 240 h" and their own Table 1 standard column runs 150 h inside a 50–400 h range, so 240 h is
    /// a value from the paper inside the paper's range. It is applied as a relaxation, `−[B]/τ`,
    /// which is what a *clearance time* means; a constant-rate sink with a time constant attached
    /// would drive the concentration negative and need a clamp to hide it.
    pub clearance_h: f32,
    /// **How long HO-1 takes to be there**, hours — the time constant of an exponential ramp on
    /// [`Params::ho_mg_l`], `1 − exp(−t/τ)`.
    ///
    /// **The mechanism is Stam's; the number is this crate's own.** Their Table 1 carries a *serum*
    /// HO-1 concentration and sets the bruise's range at "10–100 times this normal value, because
    /// of the upregulation of HO-1 in wounds" — so the enzyme concentration in their standard column
    /// is the *upregulated* one, reached at some point after the injury rather than present at
    /// `t = 0`. They fit that transient: "Diffusivity, relaxation time, and concentration of HO were
    /// varied until the simulated bruise at various time points resembled the natural bruise". The
    /// relaxation time they fitted is not legible in this crate's corpus extraction of their Table 1,
    /// and the time course it stands for is measured in a paper this corpus does not hold —
    /// Nakajima et al., *"Time-course changes in the expression of heme oxygenase-1 in human
    /// subcutaneous hemorrhage"*, Forensic Sci Int 158 (2006) 157–163, their reference 27. **Flagged:
    /// value not in corpus.**
    ///
    /// Without it the model is not just uncalibrated, it is **wrong in a way anyone can see**: the
    /// enzyme runs at full speed from the first step, so bilirubin — which nothing consumes and
    /// which clears only on a 240 h constant — outweighs haemoglobin optically within hours, and a
    /// three-hour-old bruise renders yellow. `48 h` puts HO-1 at half strength at 33 h and nearly
    /// all of it by five days, which is the ordering the forensic literature reports and the one
    /// number in it that is dated: Langlois & Gresham, *"The ageing of bruises: a review and study
    /// of the colour changes with time"*, Forensic Sci Int 50 (1991) 227–238 — Stam's reference 18 —
    /// find no yellow in bruises younger than about a day. **That paper is not in this corpus
    /// either**, so it is cited by reference; the consequence is pinned by
    /// [`tests::the_red_peaks_before_the_yellow`] rather than left as a comment.
    pub ho_induction_h: f32,
    /// Top dermal layer thickness, mm. Table 1: 400 µm.
    pub dermis_top_mm: f32,
    /// Bottom dermal layer thickness, mm. Table 1: 600 µm.
    pub dermis_bottom_mm: f32,
    /// Subcutis layer thickness, mm. **This crate's own** — Stam's Table 1 gives dermal thicknesses
    /// only, and this one sets the pool's volume, so it is the single most load-bearing authored
    /// number here. 3 mm is an ordinary subcutaneous fat thickness over a limb.
    pub subcutis_mm: f32,
    /// Diffuse reflectance of the tissue beneath the top dermis, `[0, 1]`, grey. **This crate's
    /// own**, and deliberately **neutral**: a skin tone belongs to a caller, and a neutral substrate
    /// is what makes `a*` and `b*` read the bruise rather than the skin.
    pub substrate: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            pool_diameter_mm: 10.0,
            hb_start_g_l: HB_START_G_L,
            d_hb_mm2_h: 0.01,
            d_bil_mm2_h: 0.04,
            hydraulic_conductivity: 5.0e-9,
            pressure_difference_pa: 2.6e2,
            km_umol_l: 0.24,
            v_max_umol_h_mg: 3.4,
            ho_mg_l: 5.0,
            clearance_h: 240.0,
            ho_induction_h: 48.0,
            dermis_top_mm: 0.4,
            dermis_bottom_mm: 0.6,
            subcutis_mm: 3.0,
            substrate: 0.55,
        }
    }
}

/// **A bruise, mid-flight.** Two chromophore fields over three layers and [`SHELLS`] shells, and an
/// integer count of the [`STEP_H`] steps taken.
///
/// Amounts are stored, not concentrations, because the transport is a finite-volume flux between
/// compartments of **different volumes** — an annulus at 50 mm holds fifty times what the middle one
/// does — and a scheme that moved concentrations directly would invent or destroy chromophore at
/// every face.
#[derive(Clone, Debug, PartialEq)]
pub struct Bruise {
    params: Params,
    /// Haemoglobin, mg per compartment.
    hb: [[f32; SHELLS]; LAYERS],
    /// Bilirubin, µmol per compartment.
    bil: [[f32; SHELLS]; LAYERS],
    steps: u32,
}

/// Area of shell `i`, mm² — the annulus between `i·dr` and `(i+1)·dr`.
#[inline]
fn shell_area_mm2(i: usize) -> f32 {
    let r0 = i as f32 * DR_MM;
    let r1 = r0 + DR_MM;
    core::f32::consts::PI * (r1 * r1 - r0 * r0)
}

/// Which shell a radius falls in, saturating at the outermost rather than escaping the grid.
#[inline]
fn shell_of(r_mm: f32) -> usize {
    if !r_mm.is_finite() || r_mm <= 0.0 {
        return 0;
    }
    let i = (r_mm / DR_MM) as usize;
    if i >= SHELLS { SHELLS - 1 } else { i }
}

/// Read one cell of a field, with out-of-range reading as absent rather than panicking.
#[inline]
fn at(field: &[[f32; SHELLS]; LAYERS], z: usize, i: usize) -> f32 {
    match field.get(z) {
        Some(row) => match row.get(i) {
            Some(v) => *v,
            None => 0.0,
        },
        None => 0.0,
    }
}

/// Accumulate into one cell, ignoring an out-of-range write for the same reason.
#[inline]
fn add(field: &mut [[f32; SHELLS]; LAYERS], z: usize, i: usize, v: f32) {
    if let Some(row) = field.get_mut(z) {
        if let Some(cell) = row.get_mut(i) {
            *cell += v;
        }
    }
}

impl Bruise {
    /// **A fresh bruise**: a pool of whole blood in the subcutis, nothing anywhere else.
    ///
    /// The pool is laid down by **area overlap** rather than by which shell centres fall inside it,
    /// so a 10 mm pool on a 0.9375 mm grid starts with the right amount of haemoglobin instead of
    /// the nearest stair-step, and a caller who moves `pool_diameter_mm` by less than a shell width
    /// sees the change.
    pub fn new(params: Params) -> Self {
        let mut me = Self { params, hb: [[0.0; SHELLS]; LAYERS], bil: [[0.0; SHELLS]; LAYERS], steps: 0 };
        let pool_r = (me.params.pool_diameter_mm * 0.5).max(0.0);
        // mg per mm³: 1 mg/mm³ is 1 g/ml, which is 1000 g/l.
        let conc = me.params.hb_start_g_l.max(0.0) * 1.0e-3;
        let z = Compartment::Subcutis.index();
        for i in 0..SHELLS {
            let r0 = i as f32 * DR_MM;
            let r1 = r0 + DR_MM;
            let inner = r0.min(pool_r);
            let outer = r1.min(pool_r);
            let covered = (outer * outer - inner * inner).max(0.0);
            let whole = r1 * r1 - r0 * r0;
            let fraction = if whole > 0.0 { covered / whole } else { 0.0 };
            let volume = shell_area_mm2(i) * me.params.subcutis_mm.max(1.0e-6);
            add(&mut me.hb, z, i, conc * volume * fraction);
        }
        me
    }

    /// The parameters this bruise is running under.
    pub fn params(&self) -> &Params {
        &self.params
    }

    /// Steps taken. **The age is an integer**, per the crate's no-clocks rule.
    pub fn steps(&self) -> u32 {
        self.steps
    }

    /// Age in hours: [`Bruise::steps`] × [`STEP_H`], derived rather than accumulated.
    pub fn hours(&self) -> f32 {
        self.steps as f32 * STEP_H
    }

    /// Thickness of a layer, mm.
    fn thickness(&self, z: usize) -> f32 {
        let t = match z {
            0 => self.params.dermis_top_mm,
            1 => self.params.dermis_bottom_mm,
            _ => self.params.subcutis_mm,
        };
        if t.is_finite() && t > 0.0 { t } else { 1.0e-6 }
    }

    /// Volume of one compartment, mm³.
    fn volume(&self, z: usize, i: usize) -> f32 {
        shell_area_mm2(i) * self.thickness(z)
    }

    /// Haemoglobin concentration of one compartment, mg/mm³ — the unit the fluxes are taken in.
    fn hb_dens(&self, z: usize, i: usize) -> f32 {
        let v = self.volume(z, i);
        if v > 0.0 { at(&self.hb, z, i) / v } else { 0.0 }
    }

    /// Bilirubin concentration of one compartment, µmol/mm³.
    fn bil_dens(&self, z: usize, i: usize) -> f32 {
        let v = self.volume(z, i);
        if v > 0.0 { at(&self.bil, z, i) / v } else { 0.0 }
    }

    /// **One [`STEP_H`] step**: convection, diffusion, conversion, clearance, in that order.
    ///
    /// Explicit forward difference, as Stam's own solver is ("a forward difference method in
    /// space"). The stability margin is wide and it is worth stating because it is what licenses the
    /// explicit scheme at a step this large: the diffusion number is `D·Δt/Δx²` = `0.04 × 0.1 /
    /// 0.9375²` ≈ `0.0046` radially and ≈ `0.016` vertically across the two dermal layers, against a
    /// bound of `0.5`. The mechanism that would break first is not diffusion at all.
    pub fn step(&mut self) {
        let mut d_hb = [[0.0f32; SHELLS]; LAYERS];
        let mut d_bil = [[0.0f32; SHELLS]; LAYERS];

        self.convect(&mut d_hb, &mut d_bil);
        self.diffuse(&mut d_hb, &mut d_bil);
        self.react(&mut d_hb, &mut d_bil);

        for z in 0..LAYERS {
            for i in 0..SHELLS {
                let hb = (at(&self.hb, z, i) + at(&d_hb, z, i) * STEP_H).max(0.0);
                let bil = (at(&self.bil, z, i) + at(&d_bil, z, i) * STEP_H).max(0.0);
                if let Some(row) = self.hb.get_mut(z) {
                    if let Some(cell) = row.get_mut(i) {
                        *cell = hb;
                    }
                }
                if let Some(row) = self.bil.get_mut(z) {
                    if let Some(cell) = row.get_mut(i) {
                        *cell = bil;
                    }
                }
            }
        }
        self.steps = self.steps.saturating_add(1);
    }

    /// `steps` steps. An integer count, so a caller cannot accumulate a fractional age.
    pub fn advance(&mut self, steps: u32) {
        for _ in 0..steps {
            self.step();
        }
    }

    /// Darcy convection out of the pool and into the bottom of the dermis, while it lasts.
    ///
    /// `v = K·Δp` is a velocity (m/h from m³/(N·h) × N/m²), so `v·A` is a volumetric flow and
    /// `c·v·A` is an amount per hour — Stam's Eq. 1 first term, whose `[Hb]_{z+1}/V_{z+1}` is
    /// exactly the density this reads. It is `3 → 2` only, and it carries **both** chromophores,
    /// because what moves is blood rather than haemoglobin.
    fn convect(&self, d_hb: &mut [[f32; SHELLS]; LAYERS], d_bil: &mut [[f32; SHELLS]; LAYERS]) {
        if self.hours() >= CONVECTION_STOP_H {
            return;
        }
        // m/h → mm/h.
        let v_mm_h = self.params.hydraulic_conductivity * self.params.pressure_difference_pa * 1.0e3;
        if !v_mm_h.is_finite() || v_mm_h <= 0.0 {
            return;
        }
        let (from, to) = (Compartment::Subcutis.index(), Compartment::DermisBottom.index());
        for i in 0..SHELLS {
            let flow = v_mm_h * shell_area_mm2(i);
            let hb = self.hb_dens(from, i) * flow;
            let bil = self.bil_dens(from, i) * flow;
            add(d_hb, from, i, -hb);
            add(d_hb, to, i, hb);
            add(d_bil, from, i, -bil);
            add(d_bil, to, i, bil);
        }
    }

    /// Fick diffusion, radially inside each layer and vertically between them.
    ///
    /// Finite volume: one flux per face, added to one side and subtracted from the other, so the
    /// scheme conserves mass exactly whatever the volumes are. The outermost radial face is skipped
    /// rather than opened onto a zero — a leaking boundary is a silent mass sink, and this grid is
    /// wide enough that nothing ever reaches it.
    fn diffuse(&self, d_hb: &mut [[f32; SHELLS]; LAYERS], d_bil: &mut [[f32; SHELLS]; LAYERS]) {
        let d_h = self.params.d_hb_mm2_h.max(0.0);
        let d_b = self.params.d_bil_mm2_h.max(0.0);

        for z in 0..LAYERS {
            let t = self.thickness(z);
            for i in 0..SHELLS.saturating_sub(1) {
                let face = core::f32::consts::TAU * ((i + 1) as f32 * DR_MM) * t;
                let g_hb = (self.hb_dens(z, i + 1) - self.hb_dens(z, i)) / DR_MM;
                let g_bil = (self.bil_dens(z, i + 1) - self.bil_dens(z, i)) / DR_MM;
                let f_hb = d_h * g_hb * face;
                let f_bil = d_b * g_bil * face;
                add(d_hb, z, i, f_hb);
                add(d_hb, z, i + 1, -f_hb);
                add(d_bil, z, i, f_bil);
                add(d_bil, z, i + 1, -f_bil);
            }
        }

        for z in 0..LAYERS.saturating_sub(1) {
            let span = 0.5 * (self.thickness(z) + self.thickness(z + 1));
            if span <= 0.0 {
                continue;
            }
            for i in 0..SHELLS {
                let face = shell_area_mm2(i);
                let f_hb = d_h * (self.hb_dens(z + 1, i) - self.hb_dens(z, i)) / span * face;
                let f_bil = d_b * (self.bil_dens(z + 1, i) - self.bil_dens(z, i)) / span * face;
                add(d_hb, z, i, f_hb);
                add(d_hb, z + 1, i, -f_hb);
                add(d_bil, z, i, f_bil);
                add(d_bil, z + 1, i, -f_bil);
            }
        }
    }

    /// **How much of the HO-1 is there yet**, `[0, 1]` — `1 − exp(−t/τ)` on
    /// [`Params::ho_induction_h`].
    ///
    /// Exposed because it is the one authored curve in the module and a caller comparing a rendered
    /// bruise against a photograph will want to see it rather than infer it. `1.0` when the time
    /// constant is zero or non-finite, which is the "enzyme already present" model Stam's Table 1
    /// reads as on its own.
    pub fn ho_fraction(&self) -> f32 {
        let tau = self.params.ho_induction_h;
        if !tau.is_finite() || tau <= 0.0 {
            return 1.0;
        }
        1.0 - m::exp(-self.hours() / tau)
    }

    /// Michaelis–Menten conversion, 4:1 bilirubin production, first-order clearance.
    ///
    /// # The unit chain, written out because it is the part a reader cannot check by eye
    ///
    /// `c` is mg/mm³, and `1 mg/mm³ = 1 g/ml = 1000 g/l`, so the molar haemoglobin concentration is
    /// `c · 1000 / 64500` mol/l, i.e. `c · 1e9 / 64500` µmol/l. `V_max·[HO]` is then µmol/(l·h) and
    /// the saturation fraction is dimensionless, so the rate is µmol of **haemoglobin** per litre
    /// per hour. Back to a mass density per hour: `rate · 64500 · 1e-6 g/(l·h)`, and `1 g/l =
    /// 1e-3 mg/mm³`. Bilirubin takes the same rate times [`BILIRUBIN_PER_HB`], in µmol/(l·h), which
    /// is `1e-6 µmol/(mm³·h)`.
    ///
    /// The whole rate is scaled by [`Bruise::ho_fraction`], which is the enzyme arriving.
    fn react(&self, d_hb: &mut [[f32; SHELLS]; LAYERS], d_bil: &mut [[f32; SHELLS]; LAYERS]) {
        let vmax_ho =
            self.params.v_max_umol_h_mg.max(0.0) * self.params.ho_mg_l.max(0.0) * self.ho_fraction();
        let km = self.params.km_umol_l.max(0.0);
        let tau = self.params.clearance_h;
        for z in 0..LAYERS {
            for i in 0..SHELLS {
                let volume = self.volume(z, i);
                let hb = self.hb_dens(z, i);
                if hb > 0.0 && vmax_ho > 0.0 {
                    let molar = hb * 1.0e9 / MW_HB_G_MOL;
                    let rate = vmax_ho * molar / (km + molar);
                    // **Bounded by what is there.** Michaelis–Menten is zero-order once `molar ≫ km`,
                    // so on the step that exhausts a compartment the rate would take more haemoglobin
                    // than it holds. The store's `max(0.0)` used to truncate that loss while the
                    // bilirubin was still minted at the full rate — four moles for every mole that
                    // was never converted. The conversion is capped at the density present and the
                    // bilirubin follows the capped amount, so the two stay 1 : 4 to the last step.
                    let lost = (rate * MW_HB_G_MOL * 1.0e-6 * 1.0e-3).min(hb / STEP_H);
                    let converted = lost / (MW_HB_G_MOL * 1.0e-6 * 1.0e-3);
                    add(d_hb, z, i, -lost * volume);
                    add(d_bil, z, i, converted * BILIRUBIN_PER_HB * 1.0e-6 * volume);
                }
                if tau > 0.0 {
                    add(d_bil, z, i, -at(&self.bil, z, i) / tau);
                }
            }
        }
    }

    /// **Haemoglobin at a radius**, g/l, in the top dermis — the layer the colour comes from.
    pub fn hb_at(&self, r_mm: f32) -> f32 {
        self.hb_in(Compartment::DermisTop, r_mm)
    }

    /// **Bilirubin at a radius**, µmol/l, in the top dermis.
    pub fn bilirubin_at(&self, r_mm: f32) -> f32 {
        self.bilirubin_in(Compartment::DermisTop, r_mm)
    }

    /// Haemoglobin at a radius in a named compartment, g/l.
    pub fn hb_in(&self, layer: Compartment, r_mm: f32) -> f32 {
        // mg/mm³ → g/l is × 1000.
        self.hb_dens(layer.index(), shell_of(r_mm)) * 1.0e3
    }

    /// Bilirubin at a radius in a named compartment, µmol/l.
    pub fn bilirubin_in(&self, layer: Compartment, r_mm: f32) -> f32 {
        // µmol/mm³ → µmol/l is × 1e6.
        self.bil_dens(layer.index(), shell_of(r_mm)) * 1.0e6
    }

    /// **How wide the bruise reads on one chromophore**, mm: the outer edge of the outermost shell
    /// still above `frac` of that chromophore's own current peak.
    ///
    /// Stam define the simulated bruise's area by counting compartments above a **detection
    /// threshold**, which is what makes an area comparable to a photograph at all. The threshold is
    /// expressed here as a fraction of the channel's own peak rather than as an absolute
    /// concentration, for one reason: the two chromophores are in different units and orders of
    /// magnitude, and the interesting claim — that the yellow spreads further than the red — is
    /// about *shape*. Zero when the channel is empty; never a fabricated width.
    pub fn front_mm(&self, chromophore: Chromophore, frac: f32) -> f32 {
        let read = |i: usize| -> f32 {
            let r = (i as f32 + 0.5) * DR_MM;
            match chromophore {
                Chromophore::Hemoglobin => self.hb_at(r),
                Chromophore::Bilirubin => self.bilirubin_at(r),
            }
        };
        let mut peak = 0.0f32;
        for i in 0..SHELLS {
            peak = peak.max(read(i));
        }
        if peak <= 0.0 {
            return 0.0;
        }
        let cut = peak * frac.clamp(0.0, 1.0);
        let mut edge = 0.0f32;
        for i in 0..SHELLS {
            if read(i) >= cut {
                edge = (i + 1) as f32 * DR_MM;
            }
        }
        edge
    }

    /// Absorption coefficient of the top dermis at a radius, mm⁻¹, one value per
    /// [`crate::bloodstain::spectral::TABLE`] row.
    ///
    /// Two chromophores, both in the same unit:
    ///
    /// - **Haemoglobin.** [`TABLE`]'s `μa` is whole blood at haematocrit 45 %, which is
    ///   [`HB_START_G_L`] of haemoglobin, so a dermis holding `c` g/l of it absorbs
    ///   `μa_blood · c / 150`. Mixed at [`SO2_VENOUS`], because extravasated blood is not being
    ///   re-oxygenated.
    /// - **Bilirubin.** `μa = ln(10) · ε · [B]`, with `ε` in M⁻¹cm⁻¹ and `[B]` in M giving cm⁻¹, so
    ///   the `0.1` is cm⁻¹ → mm⁻¹. `[B]` arrives in µmol/l, hence the `1e-6`.
    pub fn mua_at(&self, r_mm: f32) -> [f32; SAMPLES] {
        mua_from(self.hb_at(r_mm), self.bilirubin_at(r_mm))
    }

    /// Absorption coefficient of one compartment at a radius, mm⁻¹ — [`mua_at`](Self::mua_at) for
    /// the layer a caller names rather than the top dermis alone.
    pub fn mua_in(&self, layer: Compartment, r_mm: f32) -> [f32; SAMPLES] {
        mua_from(self.hb_in(layer, r_mm), self.bilirubin_in(layer, r_mm))
    }

    /// **Reflectance through all three layers** at a radius: the top dermis as a Kubelka–Munk layer
    /// over the bottom dermis over the subcutis over [`Params::substrate`], each with its own
    /// chromophores. This is what a fresh bruise looks like from outside — the pool still sits in the
    /// subcutis on the first day and shows through a millimetre of dermis dark and dull, which the
    /// top-layer-only [`reflectance_at`](Self::reflectance_at) cannot say. Kubelka–Munk chained
    /// layer over layer is the standard multi-layer two-flux composition (each layer sees the one
    /// beneath as its substrate); the same identification `K = 2μa`, `S = ¾μs'` as everywhere else.
    pub fn reflectance_through_at(&self, r_mm: f32) -> [f32; SAMPLES] {
        let s = 0.75 * DERMIS_MUSP_MM;
        let stack = [
            (Compartment::DermisTop, self.params.dermis_top_mm.max(0.0)),
            (Compartment::DermisBottom, self.params.dermis_bottom_mm.max(0.0)),
            (Compartment::Subcutis, self.params.subcutis_mm.max(0.0)),
        ];
        let mua: [[f32; SAMPLES]; LAYERS] = [
            self.mua_in(stack[0].0, r_mm),
            self.mua_in(stack[1].0, r_mm),
            self.mua_in(stack[2].0, r_mm),
        ];
        let mut out = [self.params.substrate.clamp(0.0, 1.0); SAMPLES];
        // Bottom up: each layer's reflectance is the substrate of the one above it.
        for (layer, (_, d)) in stack.iter().enumerate().rev() {
            let Some(k) = mua.get(layer) else { continue };
            for (o, k) in out.iter_mut().zip(k.iter()) {
                *o = spectral::kubelka_munk(2.0 * k, s, *d, *o);
            }
        }
        out
    }

    /// **The colour of the bruise through the whole skin** at a radius, encoded sRGB in `[0, 1]` —
    /// [`srgb_at`](Self::srgb_at) with the deeper layers showing through.
    pub fn srgb_through_at(&self, r_mm: f32) -> [f32; 3] {
        let [r, g, b] = spectral::xyz_to_linear_srgb(spectral::xyz(&self.reflectance_through_at(r_mm)));
        [spectral::encode(r), spectral::encode(g), spectral::encode(b)]
    }

    /// CIE L\*a\*b\* of the bruise through the whole skin at a radius.
    pub fn lab_through_at(&self, r_mm: f32) -> [f32; 3] {
        spectral::lab(&self.reflectance_through_at(r_mm))
    }
}

/// Absorption coefficient, mm⁻¹ per [`TABLE`] row, of a tissue holding `hb_g_l` of haemoglobin and
/// `bil_umol_l` of bilirubin — the two-chromophore rule [`Bruise::mua_at`] documents.
fn mua_from(hb_g_l: f32, bil_umol_l: f32) -> [f32; SAMPLES] {
    {
        let frac = (hb_g_l / HB_START_G_L).max(0.0);
        let bil_molar = (bil_umol_l * 1.0e-6).max(0.0);
        let sigma = BILIRUBIN_FWHM_NM / 2.354_820_0;
        let mut out = [0.0f32; SAMPLES];
        for (o, row) in out.iter_mut().zip(TABLE.iter()) {
            let blood = SO2_VENOUS * row.mua_oxy + (1.0 - SO2_VENOUS) * row.mua_deoxy;
            let d = (row.nm as f32 - BILIRUBIN_PEAK_NM) / sigma;
            let eps = BILIRUBIN_EPS_PEAK * m::exp(-0.5 * d * d);
            let bil = core::f32::consts::LN_10 * eps * bil_molar * 0.1;
            *o = blood * frac + bil;
        }
        out
    }
}

impl Bruise {
    /// Reflectance spectrum of the bruise at a radius — the top dermis as a Kubelka–Munk layer over
    /// [`Params::substrate`], with `K = 2μa` and `S = ¾μs'`, the identification
    /// [`crate::bloodstain::spectral::reflectance`] already makes.
    pub fn reflectance_at(&self, r_mm: f32) -> [f32; SAMPLES] {
        let mua = self.mua_at(r_mm);
        let s = 0.75 * DERMIS_MUSP_MM;
        let d = self.params.dermis_top_mm.max(0.0);
        let rg = self.params.substrate.clamp(0.0, 1.0);
        let mut out = [0.0f32; SAMPLES];
        for (o, k) in out.iter_mut().zip(mua.iter()) {
            *o = spectral::kubelka_munk(2.0 * k, s, d, rg);
        }
        out
    }

    /// **The colour of the bruise at a radius**, encoded sRGB in `[0, 1]`.
    pub fn srgb_at(&self, r_mm: f32) -> [f32; 3] {
        let [r, g, b] = spectral::xyz_to_linear_srgb(spectral::xyz(&self.reflectance_at(r_mm)));
        [spectral::encode(r), spectral::encode(g), spectral::encode(b)]
    }

    /// CIE L\*a\*b\* of the bruise at a radius — the form a **hue** claim can be made in.
    pub fn lab_at(&self, r_mm: f32) -> [f32; 3] {
        spectral::lab(&self.reflectance_at(r_mm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn fresh() -> Bruise {
        Bruise::new(Params::default())
    }

    /// **Under the blow, the deeper layers can only darken.** At the centre, where the pool sits, the
    /// through-skin reflectance is a stack whose every added layer absorbs, so its L\* is never
    /// above the top-dermis-only L\* at any hour of the first day, and a pool six hours into its
    /// leak already shows through the skin as a visible darkening. **Away from the pool the
    /// opposite holds, and it is not a defect**: tissue with no chromophore in it is a pure
    /// scattering stack over the neutral substrate, and a scattering layer whitens what is under it
    /// (the two-flux limit `kubelka_munk` takes at `k = 0`), so a chromophore-free shell reads
    /// *lighter* through three layers than through one. The claim below is therefore the centre's,
    /// and the far shell's lightening is pinned beside it so nobody widens the claim by accident.
    #[test]
    fn the_layers_beneath_only_darken_and_show_early() {
        let mut b = fresh();
        for _ in 0..24 {
            b.advance(10);
            let [top_l, ..] = b.lab_at(0.0);
            let [through_l, ..] = b.lab_through_at(0.0);
            assert!(through_l <= top_l + 1.0e-3, "at {} h through {through_l} > top {top_l}", b.hours());
        }
        let mut early = fresh();
        let blank = early.lab_through_at(0.0)[0];
        early.advance(60);
        let six = early.lab_through_at(0.0)[0];
        assert!(six < blank - 2.0, "a six-hour pool shows through the skin: L* {six} against {blank}");
        // The far shell holds nothing at six hours, and there the scattering stack is lighter than
        // the single layer — the property the doc comment bounds the claim against.
        let far_top = early.lab_at(RADIUS_MM)[0];
        let far_through = early.lab_through_at(RADIUS_MM)[0];
        assert!(far_through > far_top, "a chromophore-free stack whitens: through {far_through} vs top {far_top}");
    }

    /// Hue angle in the `a*`/`b*` plane, degrees. `0°` is red, `90°` is yellow.
    fn hue(b: &Bruise, r: f32) -> f32 {
        let [_, a, bb] = b.lab_at(r);
        m::atan2(bb, a) * (180.0 / core::f32::consts::PI)
    }

    /// **The trajectory a bruise is recognised by**: reddest first, yellowest later.
    ///
    /// Measured in CIE `a*`/`b*` rather than in an RGB triple, because "it went yellow" is a claim
    /// about `+a*` giving way to `+b*` and no channel of an sRGB triple says it. The claim is an
    /// **ordering**, which is the form Stam's own result takes — the peak of the haemoglobin comes
    /// before the peak of the bilirubin, everywhere, because one is the substrate of the other:
    ///
    /// 1. `a*` peaks strictly before `b*` does.
    /// 2. There is a genuinely red phase: `a*` gets well clear of zero.
    /// 3. The hue at the reddest moment is red-ward of the hue at 240 h.
    /// 4. By 240 h the red is spent — `a*` has fallen back — while `b*` is still high.
    ///
    /// None of the four is a threshold anyone authored; all four are consequences of a haemoglobin
    /// that is consumed and a bilirubin that is only drained.
    #[test]
    fn the_red_peaks_before_the_yellow() {
        let mut b = fresh();
        let mut best_a = (0.0f32, 0u32);
        let mut best_b = (0.0f32, 0u32);
        let mut redest_hue = 0.0f32;
        // 500 h at a 5 h stride: past the point where both chromophores are gone.
        for hour in (5..=500u32).step_by(5) {
            b.advance(50);
            let [_, a, bb] = b.lab_at(0.0);
            if a > best_a.0 {
                best_a = (a, hour);
                redest_hue = hue(&b, 0.0);
            }
            if bb > best_b.0 {
                best_b = (bb, hour);
            }
        }
        let [_, late_a, late_b] = b.lab_at(0.0);
        assert!(
            best_a.1 < best_b.1,
            "a* peaked at {} h and b* at {} h — the red must come first",
            best_a.1,
            best_b.1
        );
        assert!(best_a.0 > 10.0, "there was never a red phase: a* peaked at only {}", best_a.0);
        let late_hue = {
            let mut end = fresh();
            end.advance(2400); // 240 h
            hue(&end, 0.0)
        };
        assert!(
            redest_hue < late_hue,
            "the hue must swing toward yellow: {redest_hue}° at the reddest moment, {late_hue}° at \
             240 h"
        );
        assert!(
            late_a < 0.5 * best_a.0 && late_b > 50.0,
            "at 500 h the red should be spent and the yellow still standing, got a*={late_a} \
             b*={late_b}"
        );
    }

    /// **The yellow is wider than the red, always.** `D_B` is four times `D_Hb` and bilirubin
    /// outlives the haemoglobin that made it, so its front cannot be the narrower one — and this is
    /// the spatial signature Stam's Fig. 3 exists to show, the one an averaged spectroscopic model
    /// cannot produce.
    #[test]
    fn the_bilirubin_front_is_wider_than_the_hemoglobin_front() {
        let mut b = fresh();
        b.advance(240); // 24 h
        for hour in 25..=240u32 {
            b.advance(10);
            let hb = b.front_mm(Chromophore::Hemoglobin, 0.25);
            let bil = b.front_mm(Chromophore::Bilirubin, 0.25);
            assert!(
                bil > hb,
                "at {hour} h the bilirubin front ({bil} mm) was not wider than the haemoglobin \
                 front ({hb} mm)"
            );
        }
    }

    /// Convection is a mechanism with an end, and 12 h is where Stam put it. After that the only
    /// thing that can raise the dermis' haemoglobin is diffusion, and the pool is being eaten — so
    /// the boundary is observable: the pool's own loss rate drops when the leak stops.
    #[test]
    fn convection_stops_at_twelve_hours() {
        let mut b = fresh();
        let pool = |b: &Bruise| b.hb_in(Compartment::Subcutis, 0.0);
        b.advance(100); // 10 h
        let before = pool(&b);
        b.advance(10); // 11 h
        let during = before - pool(&b);
        b.advance(30); // 14 h — past the stop
        let after_start = pool(&b);
        b.advance(10);
        let after = after_start - pool(&b);
        assert!(during > 0.0, "the pool must be draining while the vessels are open");
        assert!(
            after < during,
            "the pool lost {after} g/l per hour after the leak closed and {during} before — the \
             12 h stop did nothing"
        );
    }

    /// A bruise resolves. Every chromophore is eventually gone, and neither can go negative on the
    /// way — a clamp that hid a negative concentration would read as a colour nobody can explain.
    #[test]
    fn nothing_goes_negative_and_the_bruise_resolves() {
        let mut b = fresh();
        for _ in 0..80 {
            b.advance(100); // 800 h in total
            for i in 0..SHELLS {
                let r = (i as f32 + 0.5) * DR_MM;
                for c in [Compartment::DermisTop, Compartment::DermisBottom, Compartment::Subcutis] {
                    assert!(b.hb_in(c, r) >= 0.0, "haemoglobin went negative at {r} mm");
                    assert!(b.bilirubin_in(c, r) >= 0.0, "bilirubin went negative at {r} mm");
                }
            }
        }
        assert!(
            b.hb_in(Compartment::Subcutis, 0.0) < 0.01 * HB_START_G_L,
            "the pool must be all but gone after 800 h, and holds {} g/l",
            b.hb_in(Compartment::Subcutis, 0.0)
        );
    }

    /// **Frozen.** A four-point trajectory through the whole model: the two concentrations at the
    /// centre and at 8 mm, and the colour at the centre, at 6 h, 24 h, 96 h and 240 h.
    ///
    /// A lock rather than a snapshot. Every constant in Stam's Table 1, the layer thicknesses, the
    /// flux scheme, the reaction order and the whole colour path are upstream of these bits; if one
    /// moves, the model moved. Re-frozen twice. Before 0.3.0 shipped: `kubelka_munk`'s
    /// non-absorbing limit was corrected (a clear scattering layer whitened nothing), which moved
    /// the four colour bytes at 96 h and 240 h where the top dermis holds no haemoglobin. After
    /// 0.3.0, on review: `react` minted bilirubin at the full Michaelis–Menten rate on the step that
    /// exhausted a compartment while the store clamped the haemoglobin loss, so the two drifted
    /// past 1 : 4 at the end of a bruise; the conversion is now bounded by what is present. A
    /// compartment that read exactly `0` after that overshoot now reads a residual of `1e-10` g/l,
    /// and the late colour moved by a few bits where that compartment fed it.
    #[test]
    fn the_bruise_model_is_frozen() {
        let mut b = fresh();
        let mut got: Vec<u32> = Vec::new();
        let mut last = 0u32;
        for hours in [6u32, 24, 96, 240] {
            b.advance(hours * 10 - last);
            last = hours * 10;
            for v in [b.hb_at(0.0), b.bilirubin_at(0.0), b.hb_at(8.0), b.bilirubin_at(8.0)] {
                got.push(v.to_bits());
            }
            for c in b.srgb_at(0.0) {
                got.push(c.to_bits());
            }
        }
        std::println!("{got:?}");
        let want: Vec<u32> = std::vec![
            0x3f673c96, 0x41b797d4, 0x30525422, 0x3cce3c54, 0x3f50a59f, 0x3f46c4c2, 0x3f2464ea,
            0x40cdec14, 0x43a6ed22, 0x2f84a806, 0x40cfe2f7, 0x3f4d76f5, 0x3f08d4aa, 0x3e65396c,
            0x3c333930, 0x452d914f, 0x29ba9dca, 0x435f16ae, 0x3f6c4122, 0x3f2c4c28, 0x00000000,
            0x00000000, 0x45082fce, 0x00000000, 0x43db8f7e, 0x3f6b118c, 0x3f2fed92, 0x00000000,
        ];
        assert_eq!(got, want);
    }
}
