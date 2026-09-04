//! **Blood colour from physics, not from a swatch.**
//!
//! Every colour `bloodstain` used to know was a stop in sRGB — three authored triples walked by
//! age. That is fine for the *chemistry* of ageing, which is a sequence with published endpoints,
//! and wrong for the two things that decide what fresh blood looks like on a surface: **how thick
//! the film is** and **how much oxygen it carries**. A 50 µm smear is pink-scarlet and a 3 mm pool
//! is near-black crimson from the same fluid; arterial blood at 97 % saturation is visibly brighter
//! than venous at 75 %. No triple can say that, so this module computes it.
//!
//! # The model
//!
//! Whole blood is an absorbing, strongly forward-scattering medium. Its optical properties are
//! compiled per wavelength by Bosschaart, Edelman, Aalders, van Leeuwen & Faber, *"A literature
//! review and novel theoretical approach on the optical properties of whole blood"*, Lasers Med
//! Sci 29 (2014), `doi:10.1007/s10103-013-1446-7` — the absorption coefficient `μa` for
//! oxygenated (SO₂ > 98 %) and deoxygenated (SO₂ = 0 %) blood at haematocrit 45 %, the scattering
//! coefficient `μs` and the anisotropy `g`, all tabulated in their Appendix. [`TABLE`] carries
//! their 380–780 nm rows resampled to 5 nm, which is the visible range and the CIE grid.
//!
//! Intermediate saturation is the linear mix of the two endpoint absorptions, which is what the
//! two-chromophore Beer–Lambert model every pulse oximeter rests on assumes; Faber et al.,
//! *"Oxygen saturation-dependent absorption and scattering of blood"*, Phys. Rev. Lett. 93,
//! 028102 (2004), `doi:10.1103/PhysRevLett.93.028102`, measure only the two endpoints and note
//! that scattering also moves with SO₂ — by about 10 % above 600 nm — which is small against the
//! 10× swings in `μa` and is left on the oxygenated table.
//!
//! A film of thickness `d` over a substrate of reflectance `Rg` reflects by the Kubelka–Munk
//! two-flux solution for a finite layer:
//!
//! ```text
//! a = 1 + K/S,  b = √(a² − 1)
//! R = (1 − Rg·(a − b·coth(b·S·d))) / (a − Rg + b·coth(b·S·d))
//! ```
//!
//! with `K = 2·μa` and `S = ¾·μs·(1 − g)` — the usual identification of the two-flux constants with
//! the transport coefficients. Thin films transmit the substrate; thick ones converge on the
//! semi-infinite reflectance `R∞ = a − b`, which for blood is ≈ 0.44 at 650 nm and ≈ 0.02 at
//! 540 nm: opaque, and deep red, without a colour being authored anywhere.
//!
//! The reflectance spectrum is then integrated against the CIE 1931 2° colour-matching functions
//! under illuminant D65 and taken to linear sRGB by the standard matrix. Both tables are the CIE's
//! own, at 5 nm.
//!
//! # What this does not do
//!
//! **Ageing stays in [`crate::bloodstain::dry`].** The oxidation products — methaemoglobin and hemichrome — have
//! absorption spectra of their own, and those are not in the corpus this crate was written from
//! (they are in Prahl's compilation, which Bosschaart cites as their reference 5). So the age walk
//! keeps its published sRGB stops and [`crate::bloodstain::dry::appearance_of`] applies the stop-to-stop *shift*
//! to the physical fresh colour. The day those spectra are tabulated here, `μa` becomes a
//! three-chromophore mix and the shift goes away; nothing a caller holds changes.

use crate::bloodstain::m;

/// First tabulated wavelength, nm.
pub const LAMBDA_MIN_NM: u32 = 380;
/// Table step, nm.
pub const LAMBDA_STEP_NM: u32 = 5;
/// Rows in [`TABLE`]: 380, 385, …, 780.
pub const SAMPLES: usize = 81;

/// Arterial oxygen saturation of a healthy adult: above 95 % (the exercise-physiology corpus puts
/// resting values above 95 % and only high-intensity exercise pulls them down). `0.97` is the
/// centre of that band.
pub const SO2_ARTERIAL: f32 = 0.97;
/// Mixed venous saturation, the clinical `SvO₂` of about 70–75 %. Venous blood is what an ordinary
/// laceration bleeds; arterial is what a severed vessel sprays.
pub const SO2_VENOUS: f32 = 0.75;

/// One row of the optical table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Wavelength, nm.
    pub nm: u16,
    /// CIE 1931 2° colour-matching functions `(x̄, ȳ, z̄)`.
    pub cie: [f32; 3],
    /// CIE illuminant D65 relative spectral power.
    pub d65: f32,
    /// Absorption coefficient of oxygenated whole blood (SO₂ > 98 %, hct 45 %), mm⁻¹.
    pub mua_oxy: f32,
    /// Absorption coefficient of deoxygenated whole blood (SO₂ = 0 %, hct 45 %), mm⁻¹.
    pub mua_deoxy: f32,
    /// Scattering coefficient, compiled literature spectrum at hct 45 %, mm⁻¹.
    pub mus: f32,
    /// Scattering anisotropy `g`, compiled literature spectrum.
    pub g: f32,
}

/// **The optical table**, 380–780 nm at 5 nm.
///
/// `cie` and `d65` are the CIE 1931 2° observer and the D65 illuminant at 5 nm. `mua_*`, `mus`
/// and `g` are Bosschaart et al. 2014's Appendix, linearly resampled from their 2 nm grid; the
/// three columns are the compiled literature spectra (their Fig. 1a/1b), not the Kramers–Kronig
/// calculation.
pub const TABLE: [Sample; SAMPLES] = [
    Sample { nm: 380, cie: [1.368000e-03, 3.900000e-05, 6.450001e-03], d65: 49.9755, mua_oxy: 47.860, mua_deoxy: 61.280, mus: 49.79, g: 0.8798 },
    Sample { nm: 385, cie: [2.236000e-03, 6.400000e-05, 1.054999e-02], d65: 52.3118, mua_oxy: 56.015, mua_deoxy: 63.545, mus: 46.75, g: 0.8639 },
    Sample { nm: 390, cie: [4.243000e-03, 1.200000e-04, 2.005001e-02], d65: 54.6482, mua_oxy: 68.330, mua_deoxy: 67.750, mus: 43.94, g: 0.8379 },
    Sample { nm: 395, cie: [7.650000e-03, 2.170000e-04, 3.621000e-02], d65: 68.7015, mua_oxy: 81.535, mua_deoxy: 75.520, mus: 42.05, g: 0.8047 },
    Sample { nm: 400, cie: [1.431000e-02, 3.960000e-04, 6.785001e-02], d65: 82.7549, mua_oxy: 93.070, mua_deoxy: 82.540, mus: 40.12, g: 0.7734 },
    Sample { nm: 405, cie: [2.319000e-02, 6.400000e-04, 1.102000e-01], d65: 87.1204, mua_oxy: 103.990, mua_deoxy: 90.970, mus: 39.87, g: 0.7748 },
    Sample { nm: 410, cie: [4.351000e-02, 1.210000e-03, 2.074000e-01], d65: 91.4860, mua_oxy: 113.810, mua_deoxy: 99.870, mus: 38.60, g: 0.7604 },
    Sample { nm: 415, cie: [7.763000e-02, 2.180000e-03, 3.713000e-01], d65: 92.4589, mua_oxy: 118.005, mua_deoxy: 111.475, mus: 40.48, g: 0.7595 },
    Sample { nm: 420, cie: [1.343800e-01, 4.000000e-03, 6.456000e-01], d65: 93.4318, mua_oxy: 115.390, mua_deoxy: 125.100, mus: 43.91, g: 0.7742 },
    Sample { nm: 425, cie: [2.147700e-01, 7.300000e-03, 1.039050e+00], d65: 90.0570, mua_oxy: 103.455, mua_deoxy: 142.335, mus: 51.32, g: 0.8081 },
    Sample { nm: 430, cie: [2.839000e-01, 1.160000e-02, 1.385600e+00], d65: 86.6823, mua_oxy: 89.180, mua_deoxy: 155.620, mus: 57.55, g: 0.8390 },
    Sample { nm: 435, cie: [3.285000e-01, 1.684000e-02, 1.622960e+00], d65: 95.7736, mua_oxy: 70.780, mua_deoxy: 157.615, mus: 63.69, g: 0.8765 },
    Sample { nm: 440, cie: [3.482800e-01, 2.300000e-02, 1.747060e+00], d65: 104.8650, mua_oxy: 51.070, mua_deoxy: 118.330, mus: 68.83, g: 0.9026 },
    Sample { nm: 445, cie: [3.480600e-01, 2.980000e-02, 1.782600e+00], d65: 110.9360, mua_oxy: 40.225, mua_deoxy: 81.570, mus: 73.31, g: 0.9323 },
    Sample { nm: 450, cie: [3.362000e-01, 3.800000e-02, 1.772110e+00], d65: 117.0080, mua_oxy: 31.840, mua_deoxy: 40.340, mus: 76.53, g: 0.9440 },
    Sample { nm: 455, cie: [3.187000e-01, 4.800000e-02, 1.744100e+00], d65: 117.4100, mua_oxy: 26.830, mua_deoxy: 18.430, mus: 79.12, g: 0.9532 },
    Sample { nm: 460, cie: [2.908000e-01, 6.000000e-02, 1.669200e+00], d65: 117.8120, mua_oxy: 22.820, mua_deoxy: 11.450, mus: 80.31, g: 0.9587 },
    Sample { nm: 465, cie: [2.511000e-01, 7.390000e-02, 1.528100e+00], d65: 116.3360, mua_oxy: 19.535, mua_deoxy: 9.380, mus: 81.29, g: 0.9643 },
    Sample { nm: 470, cie: [1.953600e-01, 9.098000e-02, 1.287640e+00], d65: 114.8610, mua_oxy: 17.050, mua_deoxy: 8.290, mus: 82.61, g: 0.9677 },
    Sample { nm: 475, cie: [1.421000e-01, 1.126000e-01, 1.041900e+00], d65: 115.3920, mua_oxy: 15.435, mua_deoxy: 7.695, mus: 83.66, g: 0.9692 },
    Sample { nm: 480, cie: [9.564000e-02, 1.390200e-01, 8.129501e-01], d65: 115.9230, mua_oxy: 14.150, mua_deoxy: 7.310, mus: 86.13, g: 0.9705 },
    Sample { nm: 485, cie: [5.795001e-02, 1.693000e-01, 6.162000e-01], d65: 112.3670, mua_oxy: 13.110, mua_deoxy: 7.625, mus: 86.78, g: 0.9727 },
    Sample { nm: 490, cie: [3.201000e-02, 2.080200e-01, 4.651800e-01], d65: 108.8110, mua_oxy: 12.230, mua_deoxy: 8.100, mus: 87.02, g: 0.9743 },
    Sample { nm: 495, cie: [1.470000e-02, 2.586000e-01, 3.533000e-01], d65: 109.0820, mua_oxy: 11.510, mua_deoxy: 8.735, mus: 88.53, g: 0.9754 },
    Sample { nm: 500, cie: [4.900000e-03, 3.230000e-01, 2.720000e-01], d65: 109.3540, mua_oxy: 11.050, mua_deoxy: 9.500, mus: 88.63, g: 0.9761 },
    Sample { nm: 505, cie: [2.400000e-03, 4.073000e-01, 2.123000e-01], d65: 108.5780, mua_oxy: 10.665, mua_deoxy: 10.445, mus: 87.05, g: 0.9762 },
    Sample { nm: 510, cie: [9.300000e-03, 5.030000e-01, 1.582000e-01], d65: 107.8020, mua_oxy: 10.550, mua_deoxy: 11.410, mus: 85.43, g: 0.9763 },
    Sample { nm: 515, cie: [2.910000e-02, 6.082000e-01, 1.117000e-01], d65: 106.2960, mua_oxy: 11.340, mua_deoxy: 12.670, mus: 84.14, g: 0.9765 },
    Sample { nm: 520, cie: [6.327000e-02, 7.100000e-01, 7.824999e-02], d65: 104.7900, mua_oxy: 13.260, mua_deoxy: 13.980, mus: 81.42, g: 0.9760 },
    Sample { nm: 525, cie: [1.096000e-01, 7.932000e-01, 5.725001e-02], d65: 106.2390, mua_oxy: 16.395, mua_deoxy: 15.630, mus: 77.38, g: 0.9748 },
    Sample { nm: 530, cie: [1.655000e-01, 8.620000e-01, 4.216000e-02], d65: 107.6890, mua_oxy: 20.910, mua_deoxy: 17.820, mus: 73.83, g: 0.9707 },
    Sample { nm: 535, cie: [2.257499e-01, 9.148501e-01, 2.984000e-02], d65: 106.0470, mua_oxy: 24.540, mua_deoxy: 19.940, mus: 71.47, g: 0.9659 },
    Sample { nm: 540, cie: [2.904000e-01, 9.540000e-01, 2.030000e-02], d65: 104.4050, mua_oxy: 27.130, mua_deoxy: 22.130, mus: 70.14, g: 0.9627 },
    Sample { nm: 545, cie: [3.597000e-01, 9.803000e-01, 1.340000e-02], d65: 104.2250, mua_oxy: 26.425, mua_deoxy: 24.390, mus: 70.12, g: 0.9626 },
    Sample { nm: 550, cie: [4.334499e-01, 9.949501e-01, 8.749999e-03], d65: 104.0460, mua_oxy: 23.860, mua_deoxy: 25.990, mus: 71.49, g: 0.9642 },
    Sample { nm: 555, cie: [5.120501e-01, 1.000000e+00, 5.749999e-03], d65: 102.0230, mua_oxy: 20.852, mua_deoxy: 26.695, mus: 73.34, g: 0.9671 },
    Sample { nm: 560, cie: [5.945000e-01, 9.950000e-01, 3.900000e-03], d65: 100.0000, mua_oxy: 19.120, mua_deoxy: 26.560, mus: 74.13, g: 0.9694 },
    Sample { nm: 565, cie: [6.784000e-01, 9.786000e-01, 2.749999e-03], d65: 98.1671, mua_oxy: 20.590, mua_deoxy: 25.060, mus: 72.47, g: 0.9690 },
    Sample { nm: 570, cie: [7.621000e-01, 9.520000e-01, 2.100000e-03], d65: 96.3342, mua_oxy: 24.380, mua_deoxy: 23.290, mus: 70.02, g: 0.9677 },
    Sample { nm: 575, cie: [8.425000e-01, 9.154000e-01, 1.800000e-03], d65: 96.0611, mua_oxy: 26.950, mua_deoxy: 20.960, mus: 68.83, g: 0.9668 },
    Sample { nm: 580, cie: [9.163000e-01, 8.700000e-01, 1.650001e-03], d65: 95.7880, mua_oxy: 25.750, mua_deoxy: 18.870, mus: 70.07, g: 0.9662 },
    Sample { nm: 585, cie: [9.786000e-01, 8.163000e-01, 1.400000e-03], d65: 92.2368, mua_oxy: 18.285, mua_deoxy: 16.800, mus: 76.05, g: 0.9686 },
    Sample { nm: 590, cie: [1.026300e+00, 7.570000e-01, 1.100000e-03], d65: 88.6856, mua_oxy: 9.720, mua_deoxy: 14.210, mus: 81.67, g: 0.9724 },
    Sample { nm: 595, cie: [1.056700e+00, 6.949000e-01, 1.000000e-03], d65: 89.3459, mua_oxy: 5.070, mua_deoxy: 11.260, mus: 85.12, g: 0.9767 },
    Sample { nm: 600, cie: [1.062200e+00, 6.310000e-01, 8.000000e-04], d65: 90.0062, mua_oxy: 2.620, mua_deoxy: 7.530, mus: 86.88, g: 0.9794 },
    Sample { nm: 605, cie: [1.045600e+00, 5.668000e-01, 6.000000e-04], d65: 89.8026, mua_oxy: 1.510, mua_deoxy: 5.550, mus: 87.89, g: 0.9809 },
    Sample { nm: 610, cie: [1.002600e+00, 5.030000e-01, 3.400000e-04], d65: 89.5991, mua_oxy: 0.880, mua_deoxy: 4.070, mus: 88.09, g: 0.9815 },
    Sample { nm: 615, cie: [9.384000e-01, 4.412000e-01, 2.400000e-04], d65: 88.6489, mua_oxy: 0.630, mua_deoxy: 3.270, mus: 88.09, g: 0.9820 },
    Sample { nm: 620, cie: [8.544499e-01, 3.810000e-01, 1.900000e-04], d65: 87.6987, mua_oxy: 0.460, mua_deoxy: 2.820, mus: 88.28, g: 0.9823 },
    Sample { nm: 625, cie: [7.514000e-01, 3.210000e-01, 1.000000e-04], d65: 85.4936, mua_oxy: 0.350, mua_deoxy: 2.480, mus: 88.50, g: 0.9824 },
    Sample { nm: 630, cie: [6.424000e-01, 2.650000e-01, 4.999999e-05], d65: 83.2886, mua_oxy: 0.280, mua_deoxy: 2.270, mus: 88.55, g: 0.9826 },
    Sample { nm: 635, cie: [5.419000e-01, 2.170000e-01, 3.000000e-05], d65: 83.4939, mua_oxy: 0.240, mua_deoxy: 2.100, mus: 88.63, g: 0.9827 },
    Sample { nm: 640, cie: [4.479000e-01, 1.750000e-01, 2.000000e-05], d65: 83.6992, mua_oxy: 0.200, mua_deoxy: 1.980, mus: 88.84, g: 0.9827 },
    Sample { nm: 645, cie: [3.608000e-01, 1.382000e-01, 1.000000e-05], d65: 81.8630, mua_oxy: 0.170, mua_deoxy: 1.890, mus: 88.55, g: 0.9826 },
    Sample { nm: 650, cie: [2.835000e-01, 1.070000e-01, 0.000000e+00], d65: 80.0268, mua_oxy: 0.160, mua_deoxy: 1.800, mus: 88.01, g: 0.9825 },
    Sample { nm: 655, cie: [2.187000e-01, 8.160000e-02, 0.000000e+00], d65: 80.1207, mua_oxy: 0.160, mua_deoxy: 1.710, mus: 87.72, g: 0.9825 },
    Sample { nm: 660, cie: [1.649000e-01, 6.100000e-02, 0.000000e+00], d65: 80.2146, mua_oxy: 0.150, mua_deoxy: 1.640, mus: 87.61, g: 0.9826 },
    Sample { nm: 665, cie: [1.212000e-01, 4.458000e-02, 0.000000e+00], d65: 81.2462, mua_oxy: 0.140, mua_deoxy: 1.580, mus: 87.51, g: 0.9828 },
    Sample { nm: 670, cie: [8.740000e-02, 3.200000e-02, 0.000000e+00], d65: 82.2778, mua_oxy: 0.140, mua_deoxy: 1.510, mus: 87.25, g: 0.9832 },
    Sample { nm: 675, cie: [6.360000e-02, 2.320000e-02, 0.000000e+00], d65: 80.2810, mua_oxy: 0.140, mua_deoxy: 1.430, mus: 86.82, g: 0.9830 },
    Sample { nm: 680, cie: [4.677000e-02, 1.700000e-02, 0.000000e+00], d65: 78.2842, mua_oxy: 0.140, mua_deoxy: 1.350, mus: 86.61, g: 0.9831 },
    Sample { nm: 685, cie: [3.290000e-02, 1.192000e-02, 0.000000e+00], d65: 74.0027, mua_oxy: 0.140, mua_deoxy: 1.260, mus: 86.57, g: 0.9834 },
    Sample { nm: 690, cie: [2.270000e-02, 8.210000e-03, 0.000000e+00], d65: 69.7213, mua_oxy: 0.130, mua_deoxy: 1.170, mus: 86.35, g: 0.9835 },
    Sample { nm: 695, cie: [1.584000e-02, 5.723000e-03, 0.000000e+00], d65: 70.6652, mua_oxy: 0.130, mua_deoxy: 1.100, mus: 86.18, g: 0.9835 },
    Sample { nm: 700, cie: [1.135916e-02, 4.102000e-03, 0.000000e+00], d65: 71.6091, mua_oxy: 0.140, mua_deoxy: 1.000, mus: 85.70, g: 0.9836 },
    Sample { nm: 705, cie: [8.110916e-03, 2.929000e-03, 0.000000e+00], d65: 72.9790, mua_oxy: 0.140, mua_deoxy: 0.930, mus: 83.70, g: 0.9837 },
    Sample { nm: 710, cie: [5.790346e-03, 2.091000e-03, 0.000000e+00], d65: 74.3490, mua_oxy: 0.170, mua_deoxy: 0.870, mus: 83.33, g: 0.9841 },
    Sample { nm: 715, cie: [4.109457e-03, 1.484000e-03, 0.000000e+00], d65: 67.9765, mua_oxy: 0.170, mua_deoxy: 0.800, mus: 82.99, g: 0.9840 },
    Sample { nm: 720, cie: [2.899327e-03, 1.047000e-03, 0.000000e+00], d65: 61.6040, mua_oxy: 0.180, mua_deoxy: 0.750, mus: 82.57, g: 0.9839 },
    Sample { nm: 725, cie: [2.049190e-03, 7.400000e-04, 0.000000e+00], d65: 65.7448, mua_oxy: 0.190, mua_deoxy: 0.720, mus: 82.18, g: 0.9839 },
    Sample { nm: 730, cie: [1.439971e-03, 5.200000e-04, 0.000000e+00], d65: 69.8856, mua_oxy: 0.200, mua_deoxy: 0.700, mus: 81.64, g: 0.9839 },
    Sample { nm: 735, cie: [9.999493e-04, 3.611000e-04, 0.000000e+00], d65: 72.4863, mua_oxy: 0.210, mua_deoxy: 0.700, mus: 81.09, g: 0.9839 },
    Sample { nm: 740, cie: [6.900786e-04, 2.492000e-04, 0.000000e+00], d65: 75.0870, mua_oxy: 0.220, mua_deoxy: 0.720, mus: 80.66, g: 0.9839 },
    Sample { nm: 745, cie: [4.760213e-04, 1.719000e-04, 0.000000e+00], d65: 69.3398, mua_oxy: 0.230, mua_deoxy: 0.765, mus: 80.44, g: 0.9838 },
    Sample { nm: 750, cie: [3.323011e-04, 1.200000e-04, 0.000000e+00], d65: 63.5927, mua_oxy: 0.240, mua_deoxy: 0.810, mus: 80.22, g: 0.9837 },
    Sample { nm: 755, cie: [2.348261e-04, 8.480000e-05, 0.000000e+00], d65: 55.0054, mua_oxy: 0.260, mua_deoxy: 0.850, mus: 79.85, g: 0.9836 },
    Sample { nm: 760, cie: [1.661505e-04, 6.000000e-05, 0.000000e+00], d65: 46.4182, mua_oxy: 0.270, mua_deoxy: 0.840, mus: 79.41, g: 0.9835 },
    Sample { nm: 765, cie: [1.174130e-04, 4.240000e-05, 0.000000e+00], d65: 56.6118, mua_oxy: 0.290, mua_deoxy: 0.800, mus: 78.93, g: 0.9835 },
    Sample { nm: 770, cie: [8.307527e-05, 3.000000e-05, 0.000000e+00], d65: 66.8054, mua_oxy: 0.300, mua_deoxy: 0.730, mus: 78.42, g: 0.9833 },
    Sample { nm: 775, cie: [5.870652e-05, 2.120000e-05, 0.000000e+00], d65: 65.0941, mua_oxy: 0.310, mua_deoxy: 0.660, mus: 78.00, g: 0.9832 },
    Sample { nm: 780, cie: [4.150994e-05, 1.499000e-05, 0.000000e+00], d65: 63.3828, mua_oxy: 0.330, mua_deoxy: 0.590, mus: 77.61, g: 0.9832 },
];

/// **A film of blood on a surface**, which is the whole of what decides its fresh colour.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Film {
    /// Film thickness, mm. A smear is ~0.05, a drip ~0.5, a pool 2–5. Clamped at zero.
    pub thickness_mm: f32,
    /// Oxygen saturation, `[0, 1]`. See [`SO2_ARTERIAL`] and [`SO2_VENOUS`].
    pub so2: f32,
    /// Diffuse reflectance of whatever the film lies on, `[0, 1]`, grey. Thin films let it through.
    pub substrate: f32,
}

impl Film {
    /// A film of `thickness_mm` of arterial blood on a mid-grey substrate.
    pub const fn arterial(thickness_mm: f32) -> Self {
        Self { thickness_mm, so2: SO2_ARTERIAL, substrate: 0.5 }
    }
    /// A film of `thickness_mm` of venous blood on a mid-grey substrate.
    pub const fn venous(thickness_mm: f32) -> Self {
        Self { thickness_mm, so2: SO2_VENOUS, substrate: 0.5 }
    }
}

/// Kubelka–Munk reflectance of one layer over a substrate — the formula in the module docs.
///
/// `k` and `s` are the two-flux constants, `d` the thickness, `rg` the substrate reflectance. A
/// zero-thickness film returns `rg` exactly; a film thick enough that `coth` saturates returns the
/// semi-infinite `a − b`; a film that scatters but does not absorb (`k = 0`, so `b = 0` and the
/// general form is `0/0`) takes the two-flux limit `R = (Rg + S·d·(1 − Rg)) / (1 + S·d·(1 − Rg))`
/// — a non-absorbing layer still *whitens* what is under it, which is what a millimetre of dermis
/// does to a pool of blood beneath it. Until 0.3.0 this branch returned `Rg`, as though a clear
/// scattering layer were glass; nothing that had blood in it ever reached the branch.
pub fn kubelka_munk(k: f32, s: f32, d: f32, rg: f32) -> f32 {
    let d = if d.is_finite() && d > 0.0 { d } else { return rg };
    let s = s.max(1.0e-6);
    let a = 1.0 + k / s;
    let b = m::sqrt((a * a - 1.0).max(0.0));
    let x = b * s * d;
    // coth(x) = (e^{2x} + 1) / (e^{2x} − 1); past x ≈ 20 it is 1 to f32 precision and the exponent
    // would overflow, so it is pinned there rather than evaluated.
    let coth = if x >= 20.0 {
        1.0
    } else if x <= 1.0e-6 {
        let g = s * d * (1.0 - rg);
        return ((rg + g) / (1.0 + g)).clamp(0.0, 1.0);
    } else {
        let e = m::exp(2.0 * x);
        (e + 1.0) / (e - 1.0)
    };
    let r = (1.0 - rg * (a - b * coth)) / (a - rg + b * coth);
    r.clamp(0.0, 1.0)
}

/// **The reflectance spectrum of a film**, one value per [`TABLE`] row.
pub fn reflectance(film: &Film) -> [f32; SAMPLES] {
    let so2 = film.so2.clamp(0.0, 1.0);
    let rg = film.substrate.clamp(0.0, 1.0);
    let d = film.thickness_mm.max(0.0);
    let mut out = [0.0f32; SAMPLES];
    for (o, row) in out.iter_mut().zip(TABLE.iter()) {
        let mua = so2 * row.mua_oxy + (1.0 - so2) * row.mua_deoxy;
        let k = 2.0 * mua;
        let s = 0.75 * row.mus * (1.0 - row.g);
        *o = kubelka_munk(k, s, d, rg);
    }
    out
}

/// CIE XYZ of a reflectance spectrum under D65, normalised so a perfect white is `Y = 1`.
pub fn xyz(refl: &[f32; SAMPLES]) -> [f32; 3] {
    let mut acc = [0.0f32; 3];
    let mut norm = 0.0f32;
    for (r, row) in refl.iter().zip(TABLE.iter()) {
        let w = row.d65;
        norm += w * row.cie[1];
        acc[0] += w * row.cie[0] * r;
        acc[1] += w * row.cie[1] * r;
        acc[2] += w * row.cie[2] * r;
    }
    let inv = if norm > 0.0 { 1.0 / norm } else { 0.0 };
    [acc[0] * inv, acc[1] * inv, acc[2] * inv]
}

/// Linear sRGB (Rec. 709 primaries, D65) from XYZ, clamped to `[0, 1]`.
pub fn xyz_to_linear_srgb(c: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = c;
    let r = 3.240_6 * x - 1.537_2 * y - 0.498_6 * z;
    let g = -0.968_9 * x + 1.875_8 * y + 0.041_5 * z;
    let b = 0.055_7 * x - 0.204_0 * y + 1.057_0 * z;
    [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)]
}

/// The sRGB transfer function, linear → encoded.
pub fn encode(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * m::powf(c, 1.0 / 2.4) - 0.055 }
}

/// **The colour of a film**, linear sRGB in `[0, 1]`.
pub fn linear_srgb(film: &Film) -> [f32; 3] {
    xyz_to_linear_srgb(xyz(&reflectance(film)))
}

/// **The colour of a film**, encoded sRGB in `[0, 1]` — the form [`crate::bloodstain::dry::Appearance::srgb`]
/// and every other colour in this crate is in.
pub fn srgb(film: &Film) -> [f32; 3] {
    let [r, g, b] = linear_srgb(film);
    [encode(r), encode(g), encode(b)]
}

/// CIE L* of a film — perceptual lightness, `0` black to `100` white — from its `Y`.
///
/// This is the quantity a test should compare, because a colour that is "darker" is one whose L*
/// is lower, and nothing about an RGB triple says that directly.
pub fn lightness(film: &Film) -> f32 {
    let y = xyz(&reflectance(film))[1];
    let f = if y > 0.008_856 { m::powf(y, 1.0 / 3.0) } else { 7.787 * y + 16.0 / 116.0 };
    116.0 * f - 16.0
}

/// **CIE L\*a\*b\* of a reflectance spectrum**, against the same D65 white [`xyz`] normalises to.
///
/// `[L*, a*, b*]`: lightness `0`–`100`, then the two opponent axes — `+a*` red, `−a*` green, `+b*`
/// yellow, `−b*` blue. [`lightness`] answers the first of those from a film and is left alone; this
/// exists because **a hue claim needs two axes**. "This bruise went from red to yellow" is a
/// statement about the sign of `a*` giving way to the sign of `b*`, and no RGB triple and no
/// lightness says it — which is exactly the claim [`crate::bloodstain::bruise`]'s trajectory test has to make.
///
/// The same `f` companding and the same `0.008856` break the CIE puts on `Y/Yn`, applied to all
/// three ratios rather than to `Y` alone.
pub fn lab(refl: &[f32; SAMPLES]) -> [f32; 3] {
    let white = xyz(&[1.0f32; SAMPLES]);
    let c = xyz(refl);
    let ratio = |v: f32, n: f32| if n > 0.0 { v / n } else { 0.0 };
    let f = |t: f32| if t > 0.008_856 { m::powf(t, 1.0 / 3.0) } else { 7.787 * t + 16.0 / 116.0 };
    let fx = f(ratio(c[0], white[0]));
    let fy = f(ratio(c[1], white[1]));
    let fz = f(ratio(c[2], white[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables integrate to the D65 white point — which is the check that the CMF and
    /// illuminant columns are the CIE's and were not transposed.
    #[test]
    fn a_perfect_white_is_d65() {
        let white = [1.0f32; SAMPLES];
        let [x, y, z] = xyz(&white);
        let sum = x + y + z;
        let (cx, cy) = (x / sum, y / sum);
        assert!((y - 1.0).abs() < 1.0e-5, "Y of white must be 1, got {y}");
        assert!((cx - 0.3127).abs() < 1.0e-3 && (cy - 0.3290).abs() < 1.0e-3, "white point drifted to ({cx}, {cy})");
    }

    /// **Thicker is darker, strictly.** The claim the whole module exists to make checkable.
    #[test]
    fn lightness_falls_strictly_with_thickness() {
        let mut last = f32::INFINITY;
        for i in 1..=60u32 {
            let d = i as f32 * 0.05;
            let l = lightness(&Film::arterial(d));
            assert!(l < last, "L* rose or stalled at {d} mm: {l} vs {last}");
            last = l;
        }
    }

    /// **Arterial is redder than venous at every thickness.** Hue is measured as the share of red
    /// in the linear triple, which is the quantity a saturation difference moves.
    #[test]
    fn arterial_is_redder_than_venous() {
        for i in 1..=30u32 {
            let d = i as f32 * 0.1;
            let a = linear_srgb(&Film::arterial(d));
            let v = linear_srgb(&Film::venous(d));
            let share = |c: [f32; 3]| c[0] / (c[0] + c[1] + c[2]).max(1.0e-6);
            assert!(share(a) > share(v), "at {d} mm arterial {a:?} was not redder than venous {v:?}");
            assert!(lightness(&Film::arterial(d)) > lightness(&Film::venous(d)));
        }
    }

    /// A zero film is the substrate, and a film thick beyond `coth` saturation is `a − b`.
    #[test]
    fn the_two_limits_are_exact() {
        assert_eq!(kubelka_munk(1.0, 1.0, 0.0, 0.37), 0.37);
        let r_inf = |k: f32, s: f32| {
            let a = 1.0 + k / s;
            a - (a * a - 1.0).sqrt()
        };
        let got = kubelka_munk(0.5, 1.0, 1.0e6, 0.9);
        assert!((got - r_inf(0.5, 1.0)).abs() < 1.0e-5, "{got}");
    }

    /// **Frozen.** Six colours a renderer will see; a moved bit here is a moved table or a moved
    /// formula, and either is a deliberate re-bless with the paper open.
    #[test]
    fn the_spectral_model_is_frozen() {
        let films = [
            Film::arterial(0.05),
            Film::arterial(0.2),
            Film::arterial(1.0),
            Film::venous(0.05),
            Film::venous(1.0),
            Film { thickness_mm: 3.0, so2: 0.5, substrate: 0.9 },
        ];
        let got: std::vec::Vec<[u32; 3]> = films
            .iter()
            .map(|f| {
                let c = srgb(f);
                [c[0].to_bits(), c[1].to_bits(), c[2].to_bits()]
            })
            .collect();
        std::println!("{got:?}");
        let want: std::vec::Vec<[u32; 3]> = std::vec![[0x3f2bc26d, 0x3e3297e2, 0x3e346386], [0x3f1a0eee, 0x3d9cbda9, 0x3e1ecda0], [0x3f0b8fe8, 0x3dc08347, 0x3e22e3e2], [0x3f229c37, 0x3e367342, 0x3e41766e], [0x3ee3e214, 0x3e02fe90, 0x3e284e1a], [0x3ec2583b, 0x3e120d18, 0x3e2f61ea]];
        assert_eq!(got, want);
    }
}
