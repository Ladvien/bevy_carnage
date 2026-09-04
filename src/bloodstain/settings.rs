//! **Every dial the blood model has, in one struct, with one source per value.**
//!
//! # Ticks, not seconds
//!
//! Nothing in this crate reads a clock; a caller supplies its own fixed-tick counter and its rate.
//! The shipped tick counts are derived for a **60 Hz** fixed tick — a game on 30 Hz re-derives
//! [`BloodSettings::spurt_ticks`], [`clot_ticks`](BloodSettings::clot_ticks) and
//! [`pressure_decay_ticks`](BloodSettings::pressure_decay_ticks) in its own config. That is one place
//! to change, in data, and it is stated here because a silent rate-dependence in a *duration* only
//! shows up on someone else's machine.
//!
//! # One value per dial
//!
//! Every field carries an explicit `serde` default, and **every default is the same function
//! [`Default`] itself calls** — so the pair cannot drift. The struct stays `deny_unknown_fields`: a
//! *missing* dial takes the shipped value, a *misspelled* one is still an error, and that combination
//! is what makes the default safe rather than a weakening.


/// **The blood dials.** How much leaves a wound, how fast, how it lands, how it pools, and how it
/// dries and clots.
///
/// Authored once per game and then left alone. Nothing here decides *when* blood happens — the caller
/// owns that, and this crate never registers a system.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct BloodSettings {
    /// Droplets a wound throws per square metre of wound area, at severity 1.
    ///
    /// **Count scales with area, not per hit**, because a wound is a surface and the amount of blood
    /// that leaves it is a property of how much of it is open — a graze and a bisection are the same
    /// event with two areas, and one dial covers both.
    #[cfg_attr(feature = "serde", serde(default = "shipped::droplets_per_m2"))]
    pub droplets_per_m2: f32,
    /// Hard ceiling on one wound's droplet count, so a huge cut cannot exceed a consumer's particle
    /// capacity in a single burst.
    #[cfg_attr(feature = "serde", serde(default = "shipped::max_droplets_per_wound"))]
    pub max_droplets_per_wound: u32,
    /// Scales the measured 8…40 m/s spatter span. **`1.0` is the paper's own numbers, and it is a
    /// physical measurement rather than a look.**
    ///
    /// Ship it at 1.0 because [`FORWARD_SPATTER_SPEED`](crate::bloodstain::FORWARD_SPATTER_SPEED) and
    /// [`BACK_SPATTER_SPEED`](crate::bloodstain::BACK_SPATTER_SPEED) are measurements, and a default that quietly
    /// divided them would make the constants lie about what they are.
    ///
    /// **Expect to lower it, and here is the arithmetic.** At 1.0 a droplet leaving straight up at
    /// 40 m/s under the shipped 18 m/s² gravity rises `40² / (2·18) ≈ 44` metres. Correct for a real
    /// gunshot and absurd on a 1.8 m subject. The reference demos set **0.25**, which puts the throw
    /// at roughly 1–3 metres.
    #[cfg_attr(feature = "serde", serde(default = "shipped::spatter_speed_scale"))]
    pub spatter_speed_scale: f32,
    /// Half-angle of the forward spray cone, degrees, about the wound normal.
    #[cfg_attr(feature = "serde", serde(default = "shipped::spatter_cone_deg"))]
    pub spatter_cone_deg: f32,
    /// Smallest droplet diameter, metres — the indivisible droplet end of the cluster span.
    #[cfg_attr(feature = "serde", serde(default = "shipped::droplet_size_min"))]
    pub droplet_size_min: f32,
    /// Largest droplet diameter, metres — the many-droplet-cluster end of the span.
    #[cfg_attr(feature = "serde", serde(default = "shipped::droplet_size_max"))]
    pub droplet_size_max: f32,
    /// Downward acceleration used to fly a droplet to its landing point, m/s².
    ///
    /// **Not 9.81, and deliberately.** It matches the reference demos' own integrator, because blood
    /// and gibs falling at different rates in one scene reads as blood floating.
    #[cfg_attr(feature = "serde", serde(default = "shipped::gravity"))]
    pub gravity: f32,
    /// Linear drag on a droplet, 1/s — the game-scale stand-in for the two-phase air entrainment the
    /// spatter paper models properly.
    #[cfg_attr(feature = "serde", serde(default = "shipped::drag"))]
    pub drag: f32,
    /// Heartbeat rate driving the pulse train, beats per minute.
    #[cfg_attr(feature = "serde", serde(default = "shipped::spurt_bpm"))]
    pub spurt_bpm: f32,
    /// Ticks of full-flow spurting before the taper starts. `210` is 3.5 s at 60 Hz.
    #[cfg_attr(feature = "serde", serde(default = "shipped::spurt_ticks"))]
    pub spurt_ticks: u32,
    /// Ticks from opening to a clot, where flow reaches exactly zero. `360` is 6.0 s at 60 Hz.
    ///
    /// Must be at least [`spurt_ticks`](Self::spurt_ticks) — see [`BloodSettings::validate`].
    #[cfg_attr(feature = "serde", serde(default = "shipped::clot_ticks"))]
    pub clot_ticks: u32,
    /// Smallest stain radius, metres.
    #[cfg_attr(feature = "serde", serde(default = "shipped::stain_radius_min"))]
    pub stain_radius_min: f32,
    /// Largest stain radius, metres.
    #[cfg_attr(feature = "serde", serde(default = "shipped::stain_radius_max"))]
    pub stain_radius_max: f32,
    /// Stains landing within this distance of a pool join it instead of starting their own, metres.
    #[cfg_attr(feature = "serde", serde(default = "shipped::pool_merge_radius"))]
    pub pool_merge_radius: f32,
    /// Multiplier from a pool's wetted-area-equivalent radius to its drawn radius.
    ///
    /// Above 1 because blood spreads thinner than the discs that fed it: the area a droplet *wets* on
    /// impact is measured at the moment of impact, and a slick keeps creeping outward after.
    #[cfg_attr(feature = "serde", serde(default = "shipped::pool_spread"))]
    pub pool_spread: f32,
    /// Fraction of the remaining gap between drawn and target radius a pool closes per tick.
    ///
    /// Must be in `(0, 1]` — see [`BloodSettings::validate`].
    #[cfg_attr(feature = "serde", serde(default = "shipped::pool_spread_rate"))]
    pub pool_spread_rate: f32,
    /// Hard ceiling on live pools. Past it a stain joins a nearby pool if it can and is dropped if it
    /// cannot — dropping is correct at the ceiling of a system whose whole job is to accumulate.
    #[cfg_attr(feature = "serde", serde(default = "shipped::max_pools"))]
    pub max_pools: u32,
    /// Exponent on the hematocrit viscosity scale.
    ///
    /// **A tuning dial, not a measured law** — see [`crate::bloodstain::rheo::viscosity`], which says why in as
    /// many words. Carreau–Yasuda's parameters were fitted at one hematocrit; this is the shape used
    /// to move off it, and it is honest about being a shape.
    #[cfg_attr(feature = "serde", serde(default = "shipped::hct_exponent"))]
    pub hct_exponent: f32,
    /// Packed cell volume fraction. `0.45` is the value Carreau–Yasuda was fitted at.
    #[cfg_attr(feature = "serde", serde(default = "shipped::hematocrit"))]
    pub hematocrit: f32,
    /// Yield stress at a full clot, Pa. [`crate::bloodstain::rheo::flows`] fails against it, which is what makes
    /// clotting a *material* state rather than a boolean beside one.
    #[cfg_attr(feature = "serde", serde(default = "shipped::clot_yield_pa"))]
    pub clot_yield_pa: f32,
    /// Large discrete stains one arterial systole places along its arc.
    #[cfg_attr(feature = "serde", serde(default = "shipped::arc_stains"))]
    pub arc_stains: u32,
    /// Ticks over which arterial reach decays to zero. `600` is 10 s at 60 Hz.
    #[cfg_attr(feature = "serde", serde(default = "shipped::pressure_decay_ticks"))]
    pub pressure_decay_ticks: u32,
    /// Cast-off droplet diameter at the reference tip speed, metres.
    #[cfg_attr(feature = "serde", serde(default = "shipped::cast_off_d_ref"))]
    pub cast_off_d_ref: f32,
    /// The tip speed [`cast_off_d_ref`](Self::cast_off_d_ref) was authored at, m/s.
    #[cfg_attr(feature = "serde", serde(default = "shipped::cast_off_v_ref"))]
    pub cast_off_v_ref: f32,
    /// Pendant volume a weapon tip can hold before it sheds, millilitres.
    ///
    /// `0.15` is 150 µL — Adam 2019 (`doi:10.1016/j.forsciint.2019.109934`) measures the cap on what
    /// a tip actually carries, and it is a cap rather than a rate.
    #[cfg_attr(feature = "serde", serde(default = "shipped::cast_off_max_ml"))]
    pub cast_off_max_ml: f32,
    /// Fraction of expirated patterns that show bubble rings.
    ///
    /// `0.2` because only about a fifth do (Donaldson et al. 2011,
    /// `doi:10.1007/s00414-010-0498-5`) — so [`crate::bloodstain::patterns::expirated`] usually returns zero
    /// rings, deliberately.
    #[cfg_attr(feature = "serde", serde(default = "shipped::expirated_ring_fraction"))]
    pub expirated_ring_fraction: f32,
    /// Smallest stain that can carry a bubble ring, millimetres. Rings occur only above this.
    #[cfg_attr(feature = "serde", serde(default = "shipped::expirated_ring_min_mm"))]
    pub expirated_ring_min_mm: f32,
    /// Metres between drips at 1 m/s. Spacing scales with speed, which is what makes a drip trail
    /// legible as a walk, a run, or a stagger.
    #[cfg_attr(feature = "serde", serde(default = "shipped::drip_spacing_ref"))]
    pub drip_spacing_ref: f32,
    /// Millilitres moved out of a carried load per contact tick by a transfer pattern.
    ///
    /// The reason a dragged body **runs out of blood**, which is the whole point of the conserved
    /// budget in [`crate::bloodstain::patterns::transfer`].
    #[cfg_attr(feature = "serde", serde(default = "shipped::transfer_rate"))]
    pub transfer_rate: f32,
    /// Relative humidity, `[0, 1]`. Above `0.5` a drying pool grows a serum halo outside itself
    /// (Laan et al. 2016, `doi:10.1016/j.forsciint.2016.08.005`); below it, none forms.
    #[cfg_attr(feature = "serde", serde(default = "shipped::humidity"))]
    pub humidity: f32,
    /// Perceptual roughness of fresh blood. Low, because **wet is the strongest disgust cue and it is
    /// not a colour** (Oum et al., `doi:10.1080/02699931.2010.496997`) — the wetness channel is
    /// specular.
    #[cfg_attr(feature = "serde", serde(default = "shipped::wet_roughness"))]
    pub wet_roughness: f32,
    /// Perceptual roughness of fully dried blood.
    #[cfg_attr(feature = "serde", serde(default = "shipped::dry_roughness"))]
    pub dry_roughness: f32,
    /// Default substrate roughness fed to [`crate::bloodstain::stain::stain_shape`]. A rough surface shortens a
    /// stain and merges its spines.
    #[cfg_attr(feature = "serde", serde(default = "shipped::substrate_roughness"))]
    pub substrate_roughness: f32,
}

/// The shipped [`BloodSettings`] values, one function per dial.
///
/// **These are the single source, and [`BloodSettings::default`] calls them.** The alternative —
/// literals in `Default` and a parallel set of `serde` default functions — is exactly the drift a
/// consumer of this family already had to write a test to catch. Here the two cannot disagree,
/// because there is only one of them.
pub mod shipped {
    // Count scales with wound area, not per hit.
    pub fn droplets_per_m2() -> f32 {
        2400.0
    }
    // Keeps one burst inside a consumer's effect capacity.
    pub fn max_droplets_per_wound() -> u32 {
        512
    }
    // Scales the measured 8…40 m/s span.
    pub fn spatter_speed_scale() -> f32 {
        1.0
    }
    // Forward spray half-angle.
    pub fn spatter_cone_deg() -> f32 {
        32.0
    }
    // Metres; the indivisible droplet.
    pub fn droplet_size_min() -> f32 {
        0.000_8
    }
    // Metres; the cluster span's far end.
    pub fn droplet_size_max() -> f32 {
        0.006
    }
    // Matches the reference demos' own integrator so blood and gibs fall in one world; 9.81 would
    // make blood float relative to the chunks.
    pub fn gravity() -> f32 {
        18.0
    }
    // Stands in for the paper's two-phase air entrainment.
    pub fn drag() -> f32 {
        1.6
    }
    // Pulse period is `60 / bpm`.
    pub fn spurt_bpm() -> f32 {
        96.0
    }
    // 3.5 s at 60 Hz.
    pub fn spurt_ticks() -> u32 {
        210
    }
    // 6.0 s at 60 Hz.
    pub fn clot_ticks() -> u32 {
        360
    }
    // Metres.
    pub fn stain_radius_min() -> f32 {
        0.02
    }
    // Metres.
    pub fn stain_radius_max() -> f32 {
        0.12
    }
    // Metres. About a hand's width — close enough that two spatter discs read as one wet patch.
    pub fn pool_merge_radius() -> f32 {
        0.10
    }
    // Blood creeps outward after the impact area was measured.
    pub fn pool_spread() -> f32 {
        1.35
    }
    // Fraction of the remaining gap per tick; ≈0.2 s to close half the distance at 60 Hz.
    pub fn pool_spread_rate() -> f32 {
        0.08
    }
    // Live slicks.
    pub fn max_pools() -> u32 {
        256
    }
    // TUNED, not measured — see `rheo::viscosity`.
    pub fn hct_exponent() -> f32 {
        2.5
    }
    // The hematocrit Carreau–Yasuda's constants were fitted at.
    pub fn hematocrit() -> f32 {
        0.45
    }
    // Pa at a full clot. Three orders above the fresh-blood Casson yield stress.
    pub fn clot_yield_pa() -> f32 {
        5.0
    }
    // Large discrete stains per systole.
    pub fn arc_stains() -> u32 {
        7
    }
    // 10 s at 60 Hz.
    pub fn pressure_decay_ticks() -> u32 {
        600
    }
    // Metres at the reference tip speed.
    pub fn cast_off_d_ref() -> f32 {
        0.003_5
    }
    // m/s.
    pub fn cast_off_v_ref() -> f32 {
        6.0
    }
    // 150 µL — Adam 2019's measured pendant cap.
    pub fn cast_off_max_ml() -> f32 {
        0.15
    }
    // Only ~20 % of expirated patterns show bubble rings (Donaldson 2011).
    pub fn expirated_ring_fraction() -> f32 {
        0.2
    }
    // Rings occur only in stains larger than this, mm.
    pub fn expirated_ring_min_mm() -> f32 {
        3.0
    }
    // Metres between drips at 1 m/s.
    pub fn drip_spacing_ref() -> f32 {
        0.25
    }
    // Millilitres per contact tick.
    pub fn transfer_rate() -> f32 {
        0.02
    }
    // Below the 0.5 serum-halo threshold by default.
    pub fn humidity() -> f32 {
        0.4
    }
    // Perceptual roughness, fresh.
    pub fn wet_roughness() -> f32 {
        0.12
    }
    // Perceptual roughness, fully dried.
    pub fn dry_roughness() -> f32 {
        0.85
    }
    // Default surface roughness fed to `stain_shape`.
    pub fn substrate_roughness() -> f32 {
        0.2
    }
}

impl Default for BloodSettings {
    fn default() -> Self {
        BloodSettings {
            droplets_per_m2: shipped::droplets_per_m2(),
            max_droplets_per_wound: shipped::max_droplets_per_wound(),
            spatter_speed_scale: shipped::spatter_speed_scale(),
            spatter_cone_deg: shipped::spatter_cone_deg(),
            droplet_size_min: shipped::droplet_size_min(),
            droplet_size_max: shipped::droplet_size_max(),
            gravity: shipped::gravity(),
            drag: shipped::drag(),
            spurt_bpm: shipped::spurt_bpm(),
            spurt_ticks: shipped::spurt_ticks(),
            clot_ticks: shipped::clot_ticks(),
            stain_radius_min: shipped::stain_radius_min(),
            stain_radius_max: shipped::stain_radius_max(),
            pool_merge_radius: shipped::pool_merge_radius(),
            pool_spread: shipped::pool_spread(),
            pool_spread_rate: shipped::pool_spread_rate(),
            max_pools: shipped::max_pools(),
            hct_exponent: shipped::hct_exponent(),
            hematocrit: shipped::hematocrit(),
            clot_yield_pa: shipped::clot_yield_pa(),
            arc_stains: shipped::arc_stains(),
            pressure_decay_ticks: shipped::pressure_decay_ticks(),
            cast_off_d_ref: shipped::cast_off_d_ref(),
            cast_off_v_ref: shipped::cast_off_v_ref(),
            cast_off_max_ml: shipped::cast_off_max_ml(),
            expirated_ring_fraction: shipped::expirated_ring_fraction(),
            expirated_ring_min_mm: shipped::expirated_ring_min_mm(),
            drip_spacing_ref: shipped::drip_spacing_ref(),
            transfer_rate: shipped::transfer_rate(),
            humidity: shipped::humidity(),
            wet_roughness: shipped::wet_roughness(),
            dry_roughness: shipped::dry_roughness(),
            substrate_roughness: shipped::substrate_roughness(),
        }
    }
}

impl BloodSettings {
    /// Reject a settings block that cannot produce a sane schedule.
    ///
    /// **Two of these are real crashes, not hypotheticals.** `spurt_bpm` at zero divides by zero
    /// deriving the pulse period; `pool_spread_rate` above 1 makes a pool oscillate rather than
    /// spread. The remaining checks catch inverted ranges, which do not panic but silently invert the
    /// model — a `clot_ticks` below `spurt_ticks` would make flow rise before it fell.
    ///
    /// Call it at load. Failing loudly at the door is the one path; clamping a bad pair here would be
    /// a second, quieter one.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.spurt_bpm > 0.0) || !self.spurt_bpm.is_finite() {
            return Err(format!(
                "blood: spurt_bpm is {} — the pulse period is `60 / bpm`, so this must be finite \
                 and positive.",
                self.spurt_bpm
            ));
        }
        if self.clot_ticks < self.spurt_ticks {
            return Err(format!(
                "blood: clot_ticks ({}) < spurt_ticks ({}) — flow is full until `spurt_ticks` and \
                 zero at `clot_ticks`, so an inverted pair would have it rise before it fell.",
                self.clot_ticks, self.spurt_ticks
            ));
        }
        for (name, lo, hi) in [
            ("droplet_size", self.droplet_size_min, self.droplet_size_max),
            ("stain_radius", self.stain_radius_min, self.stain_radius_max),
        ] {
            if !(lo > 0.0) || !(hi >= lo) || !lo.is_finite() || !hi.is_finite() {
                return Err(format!(
                    "blood: {name}_min ({lo}) and {name}_max ({hi}) must be finite with \
                     0 < min <= max — the pair is lerped, and an inverted one reverses the model."
                ));
            }
        }
        if !(0.0..=180.0).contains(&self.spatter_cone_deg) {
            return Err(format!(
                "blood: spatter_cone_deg is {} — it is a half-angle about the wound normal, so it \
                 must be in [0, 180].",
                self.spatter_cone_deg
            ));
        }
        if self.max_pools == 0 {
            return Err("blood: max_pools is 0 — every stain would be dropped and blood would \
                        never accumulate, which is the whole feature switched off by a ceiling."
                .to_string());
        }
        for (name, v) in
            [("pool_merge_radius", self.pool_merge_radius), ("pool_spread", self.pool_spread)]
        {
            if !(v > 0.0) || !v.is_finite() {
                return Err(format!(
                    "blood: {name} is {v} — it scales a radius, so it must be finite and positive."
                ));
            }
        }
        if !(self.pool_spread_rate > 0.0 && self.pool_spread_rate <= 1.0) {
            return Err(format!(
                "blood: pool_spread_rate is {} — it is the fraction of the remaining gap closed \
                 per tick, so it must be in (0, 1]. At 0 a pool never spreads; above 1 it \
                 overshoots and oscillates.",
                self.pool_spread_rate
            ));
        }
        if !(0.0..1.0).contains(&self.hematocrit) {
            return Err(format!(
                "blood: hematocrit is {} — it is a volume fraction, so it must be in [0, 1). At 1 \
                 the viscosity scale divides by zero.",
                self.hematocrit
            ));
        }
        if !(0.0..=1.0).contains(&self.humidity) {
            return Err(format!(
                "blood: humidity is {} — it is a relative humidity in [0, 1].",
                self.humidity
            ));
        }
        if self.pressure_decay_ticks == 0 {
            return Err("blood: pressure_decay_ticks is 0 — arterial reach would be zero on the \
                        first systole, which is an arterial wound that never spurts."
                .to_string());
        }
        if !(self.cast_off_v_ref > 0.0) || !self.cast_off_v_ref.is_finite() {
            return Err(format!(
                "blood: cast_off_v_ref is {} — it is the denominator of the cast-off size law, so \
                 it must be finite and positive.",
                self.cast_off_v_ref
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped defaults are a working settings block. A crate whose own `Default` fails its own
    /// validator would be shipping a broken door.
    #[test]
    fn the_shipped_defaults_validate() {
        BloodSettings::default().validate().expect("the shipped dials must be valid");
    }

    /// Each refusal names the dial and fires on the value that breaks it — the checks are the door,
    /// so an unreachable check is a door that does not close.
    #[test]
    fn every_refusal_fires() {
        let cases: [(&str, BloodSettings); 7] = [
            ("spurt_bpm", BloodSettings { spurt_bpm: 0.0, ..Default::default() }),
            (
                "clot_ticks",
                BloodSettings { clot_ticks: 10, spurt_ticks: 20, ..Default::default() },
            ),
            ("droplet_size", BloodSettings { droplet_size_max: 0.0, ..Default::default() }),
            ("spatter_cone_deg", BloodSettings { spatter_cone_deg: 200.0, ..Default::default() }),
            ("max_pools", BloodSettings { max_pools: 0, ..Default::default() }),
            ("pool_spread_rate", BloodSettings { pool_spread_rate: 1.5, ..Default::default() }),
            ("hematocrit", BloodSettings { hematocrit: 1.0, ..Default::default() }),
        ];
        for (dial, s) in cases {
            let err = s.validate().expect_err("this block must be refused");
            assert!(err.contains(dial), "the refusal for {dial} must name it, got: {err}");
        }
    }
}
