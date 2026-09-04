//! **Every dial a wetmap has, in one struct, with the arithmetic behind each value.**
//!
//! # Ticks, not seconds
//!
//! Nothing here reads a clock. [`WetSettings::dry_ticks`] is a tick count quoted for a **60 Hz** fixed
//! tick, exactly as `bloodstain`'s durations are, and a caller on another rate re-derives it in its own
//! config. A float accumulator large enough stops advancing at all, which is a recorded failure in this
//! family of crates.
//!
//! # Two dials are rates, and a rate needs a denominator
//!
//! [`spread_rate`](WetSettings::spread_rate) is per tick, and says so plainly: 8 % of a texel's
//! coverage moves into its four neighbours each tick. [`absorbency`](WetSettings::absorbency) is
//! **not** per tick, and reading it as one is the interpretation this module exists to close — see its
//! doc comment for the arithmetic.

use bevy::prelude::Resource;
use crate::bloodstain::BloodSettings;

/// **The wetmap dials.** When blood runs, how far it creeps, how much the substrate keeps, how long it
/// takes to dry, and how many canvases may reach the GPU in one frame.
///
/// Authored once per game and then left alone. Nothing here decides *when* a canvas ticks — the caller
/// owns that, because the caller owns the tick counter.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct WetSettings {
    /// Normalised coverage above which a texel runs. `0.35`.
    ///
    /// A texel holding more than this sheds **the excess** one texel along gravity and keeps the rest,
    /// which is why a run leaves a trail rather than translating wholesale like a sprite — and it is
    /// what makes the drip pass conserve mass exactly (see [`crate::wetmap::WetCanvas::tick`]).
    pub drip_rate: f32,
    /// Fraction of a texel's coverage that diffuses into its 4-neighbourhood **per tick**. `0.08`.
    ///
    /// Split four ways, so the per-edge coefficient is `0.02` on the coverage *difference* across that
    /// edge. Writing it as a difference rather than as a give-away is what makes the pass exactly
    /// antisymmetric and therefore exactly mass-conserving.
    pub spread_rate: f32,
    /// Ticks from fresh to fully dry. `1800` — 30 s at 60 Hz.
    ///
    /// **The same reference `crate::bloodstain::dry::DRY_REF_TICKS` uses**, so blood on a wall and blood on a
    /// floor dry at one rate. It is the single authority: a texel's age is rescaled onto that
    /// reference before `crate::bloodstain::dry::appearance` is asked what the blood looks like, so moving
    /// this dial moves the whole timeline rather than only the wet/dry gate.
    ///
    /// Clamped into `1..=65535` internally, because a texel's age is a `u16`. 65 535 ticks is 18
    /// minutes at 60 Hz, past any drying anyone would author.
    pub dry_ticks: u32,
    /// Fraction of full coverage the substrate keeps **over the whole wet lifetime**. `0.15`.
    ///
    /// **Not per tick, and the arithmetic is the reason.** Read per tick, `0.15` leaves
    /// `0.85³⁰ ≈ 0.008` of the blood after half a second — every stain would vanish long before
    /// [`dry_ticks`](Self::dry_ticks) could dry it, and a wetmap would never show a dried stain at
    /// all. Read as a lifetime fraction it is exactly what its name says: a soaking substrate keeps
    /// 15 % of what lands on it, spread evenly across the drying.
    ///
    /// Applied as an integer schedule rather than an accumulator: at age `a` the cumulative loss is
    /// `a · round(255 · absorbency) / dry_ticks`, floored, and one tick applies the difference between
    /// two consecutive values. So the deltas sum to the cumulative exactly and there is no float
    /// residue to drift. Faint spatter soaks away to nothing; a pool loses 15 % and stays.
    pub absorbency: f32,
    /// Canvases the plugin may upload in one frame. `4`.
    ///
    /// A 128×128 `Rgba8UnormSrgb` canvas is 64 KB, and this crate uploads **two** images per canvas
    /// (albedo and metallic-roughness), so four canvases is 512 KB of `Assets<Image>` writes per frame.
    /// That budget is why the default canvas size is 128 rather than 512 — see [`crate::wetmap::WetCanvas::new`].
    pub max_canvas_updates_per_tick: u32,
    /// Relative humidity, `[0, 1]`. `0.4`.
    ///
    /// Forwarded straight into a [`BloodSettings`] for `crate::bloodstain::dry::appearance`, which grows a
    /// serum halo only at or above `0.5` (Laan et al. 2016, `doi:10.1016/j.forsciint.2016.08.005`). The
    /// shipped value sits below that threshold, so no halo by default.
    pub humidity: f32,
    /// Film thickness a texel at full coverage stands for, mm. `2.0`.
    ///
    /// **The coverage byte is a depth, and the colour is computed from it.** A texel's amount, scaled
    /// by this, is the thickness `crate::bloodstain::spectral` puts under the Kubelka–Munk two-flux model:
    /// a faint edge is a thin film that lets the substrate through and reads pink-scarlet, a full
    /// texel is a pool that converges on blood's own semi-infinite reflectance and reads near-black
    /// crimson. No blood colour is authored anywhere in this crate.
    pub film_depth_mm: f32,
    /// Oxygen saturation of the blood that lands, `[0, 1]`. [`crate::bloodstain::SO2_VENOUS`].
    ///
    /// What an ordinary wound bleeds. A caller painting an arterial spurt sets
    /// [`crate::bloodstain::SO2_ARTERIAL`] and gets a visibly brighter red from the same model.
    pub so2: f32,
    /// **Subsamples per texel axis when a stain is stamped.** `1` — one sample per texel, the
    /// shipped rasterisation to the byte.
    ///
    /// `crate::bloodstain::stain::rasterise` answers "how covered is this texel" once per texel, at the
    /// texel's own centre, so at the default 128-texel canvas a stain's rim is a staircase: a texel
    /// the silhouette crosses is either in or out. At `n` the mask is rasterised at `n` times the
    /// resolution and each texel takes the **mean of its `n × n` subsamples**, so a texel the edge
    /// only clips gets a proportional share of the coverage.
    ///
    /// One path, not two: the reduction is a box filter over `n²` samples, and at `n = 1` it is the
    /// identity — the same bytes the crate has always written, which is why the digests are
    /// untouched by this dial existing. `2` is the smallest useful value and `4` is what the shipped
    /// examples use; the scratch mask costs `(major · size · n)²` bytes, so `n` past 4 buys nothing a
    /// player can see for four times the rasterisation.
    ///
    /// Clamped into `1..=8` internally.
    pub edge_samples: u32,
}

impl Default for WetSettings {
    fn default() -> Self {
        Self {
            drip_rate: 0.35,
            spread_rate: 0.08,
            dry_ticks: 1800,
            absorbency: 0.15,
            max_canvas_updates_per_tick: 4,
            humidity: 0.4,
            film_depth_mm: 2.0,
            so2: crate::bloodstain::SO2_VENOUS,
            edge_samples: 1,
        }
    }
}

impl WetSettings {
    /// [`dry_ticks`](Self::dry_ticks) clamped into the range a `u16` age can actually reach.
    ///
    /// One place, so the wet/dry gate, the age ceiling and the appearance rescale cannot disagree about
    /// where dry is.
    pub(crate) fn dry_span(&self) -> u32 {
        self.dry_ticks.clamp(1, u16::MAX as u32)
    }

    /// [`edge_samples`](Self::edge_samples) clamped into `1..=8`.
    ///
    /// One place, so the scratch buffer's size and the box filter's divisor cannot disagree about how
    /// many samples a texel got.
    pub(crate) fn edge_span(&self) -> u32 {
        self.edge_samples.clamp(1, 8)
    }

    /// Coverage byte above which a texel runs.
    ///
    /// At least 1: a threshold of 0 would make every texel with any blood in it shed all of it, which
    /// is not a drip but a teleport.
    pub(crate) fn drip_threshold(&self) -> u8 {
        let t = (self.drip_rate.clamp(0.0, 1.0) * 255.0).round();
        (t as u32).clamp(1, 255) as u8
    }

    /// Coverage bytes the substrate has taken by the time a texel reaches `age`.
    ///
    /// Integer and cumulative — see [`absorbency`](Self::absorbency) for why it is a schedule rather
    /// than a per-tick multiply.
    pub(crate) fn absorbed_by(&self, age: u32) -> u32 {
        let span = self.dry_span();
        let total = (self.absorbency.clamp(0.0, 1.0) * 255.0).round() as u32;
        let a = age.min(span);
        ((a as u64 * total as u64) / span as u64) as u32
    }

    /// The [`BloodSettings`] this crate hands `crate::bloodstain::dry::appearance`.
    ///
    /// Shipped values with [`humidity`](Self::humidity) substituted. Every other blood dial the drying
    /// timeline reads — `wet_roughness`, `dry_roughness`, the three haemoglobin stops — is
    /// `bloodstain`'s, because a second copy here is how the two would stop agreeing.
    pub fn blood(&self) -> BloodSettings {
        BloodSettings { humidity: self.humidity, ..BloodSettings::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_dials_are_the_contract() {
        let s = WetSettings::default();
        assert_eq!(s.drip_rate, 0.35);
        assert_eq!(s.spread_rate, 0.08);
        assert_eq!(s.dry_ticks, 1800);
        assert_eq!(s.absorbency, 0.15);
        assert_eq!(s.max_canvas_updates_per_tick, 4);
        assert_eq!(s.humidity, 0.4);
        // One sample per texel: the shipped rasterisation, so every digest in the crate is the one
        // it was frozen at. An example that wants a smoother rim opts in.
        assert_eq!(s.edge_samples, 1);
        assert_eq!(s.edge_span(), 1);
    }

    #[test]
    fn absorption_is_cumulative_and_lands_on_its_stated_total() {
        let s = WetSettings::default();
        assert_eq!(s.absorbed_by(0), 0);
        // 15 % of full coverage, taken by the time the texel is dry.
        assert_eq!(s.absorbed_by(s.dry_ticks), 38);
        // Monotone, and the per-tick deltas sum to the cumulative rather than drifting from it.
        let mut sum = 0;
        for age in 0..s.dry_ticks {
            let d = s.absorbed_by(age + 1) - s.absorbed_by(age);
            sum += d;
        }
        assert_eq!(sum, s.absorbed_by(s.dry_ticks));
    }

    #[test]
    fn the_drip_threshold_is_never_zero() {
        let mut s = WetSettings::default();
        assert_eq!(s.drip_threshold(), 89);
        s.drip_rate = 0.0;
        assert_eq!(s.drip_threshold(), 1);
        s.drip_rate = 5.0;
        assert_eq!(s.drip_threshold(), 255);
    }

    #[test]
    fn humidity_reaches_the_blood_model_and_nothing_else_is_invented() {
        let s = WetSettings { humidity: 0.8, ..Default::default() };
        let b = s.blood();
        assert_eq!(b.humidity, 0.8);
        assert_eq!(b.wet_roughness, BloodSettings::default().wet_roughness);
        assert_eq!(b.dry_roughness, BloodSettings::default().dry_roughness);
    }
}
