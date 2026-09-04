//! **Where blood lands, and what shape it lands in.**
//!
//! Two questions, deliberately kept apart because they have different consumers:
//!
//! - **Placement.** [`Stain`] and [`stains`] — where a droplet landed and how much floor it wet. This
//!   is simulation-visible: a consuming game reads stain positions to seed pools, and pools feed
//!   further simulation, so placement must exist headless, be deterministic, and be frozen. It is.
//! - **Morphology.** [`Impact`], [`StainShape`], [`stain_shape`] and [`rasterise`] — what the stain
//!   *looks* like, derived from the impact conditions rather than picked from a set of baked textures.
//!
//! # Why morphology is derived and not authored
//!
//! **This replaced four baked splat variants selected by `seed % 4`, and the arithmetic is the
//! argument.** With four variants the expected first visible repeat is the fourth stain: the
//! probability that four independent draws from four options contain a repeat is
//! `1 − 4!/4⁴ = 90.6 %`. A floor of blood reads as tiled almost immediately, and no amount of texture
//! authoring fixes a birthday problem. A stain derived from its own impact repeats only when the
//! impact repeats.
//!
//! # The rules, and where each comes from
//!
//! - `minor / major = sin θ` — the bloodstain-pattern-analysis impact-angle relation. Hulse-Smith,
//!   Mehdizadeh & Attinger, *"Deducing drop size and impact velocity from circular bloodstains"*,
//!   J. Forensic Sci. 50(1), `doi:10.1520/jfs2003224`.
//! - Spine count from `We^0.5 · sin³θ` — Knock & Davison, *"Predicting the position of the source of
//!   blood stains for angled impacts"*, J. Forensic Sci. 52(5),
//!   `doi:10.1111/j.1556-4029.2007.00505.x`, which is the **blood-specific, angle-inclusive** form and
//!   reports R² ≈ 0.9. A water-derived spine law would be the wrong fluid and the wrong angle
//!   dependence.
//! - Satellite onset past a splash threshold in `K = We^0.5 · Re^0.25` — Mundo, Sommerfeld & Tropea
//!   (1995), whose deposition/splash boundary is the standard form. `K = Oh · Re^1.25` is the same
//!   quantity.
//! - Substrate roughness shortens the stain and merges its spines — Adam, *"Fundamental studies of
//!   bloodstain formation and characteristics"*, `doi:10.1016/j.forsciint.2011.12.002`.

use core::f32::consts::{PI, TAU};

use crate::bloodstain::droplet::{
    BACK_SPATTER_SPEED, BLOOD_DENSITY, BLOOD_SURFACE_TENSION, Droplet, FORWARD_SPATTER_SPEED,
    Spray, droplet_count, landing,
};
use crate::bloodstain::settings::BloodSettings;
use crate::bloodstain::{V3, Wound, hash_f32, m, vec};

/// Coefficient in Knock & Davison's spine law, `spines = C · We^0.5 · sin³θ`.
///
/// Sourced, not tuned: their regression over blood drops on angled surfaces.
pub const SPINE_COEFF: f32 = 0.76;

/// Ceiling on spine count.
///
/// Adam 2012 documents the saturation: past a point the rim breaks into a continuous crown rather
/// than into countable spines, and a stain with sixty spines is a starburst nobody has photographed.
pub const SPINE_MAX: u8 = 24;

/// Weber number below which no spines form at all.
///
/// **TUNED, not measured.** Adam 2012 documents that an onset exists — a low-energy drop deposits as
/// a smooth ellipse — without giving a single threshold that transfers to this model's units. `30` is
/// the value chosen, and its only defence is that it puts the onset at roughly the drop energy where
/// photographs stop showing smooth rims. Treat it as a dial with a citation for its *existence*, not
/// for its value.
pub const SPINE_WE_MIN: f32 = 30.0;

/// Mundo's splash parameter threshold: at or above this, the rim throws satellites.
///
/// `K = We^0.5 · Re^0.25 ≥ 57.7` is Mundo, Sommerfeld & Tropea's deposition/splash boundary. Sourced.
pub const SPLASH_K_CRIT: f32 = 57.7;

/// A stain the caller may stamp: where a droplet landed and how wide it reads.
///
/// **Core, not a visual.** Where blood lands is read by simulation on the consuming side — a blood
/// pool is a chemoattractant source there — so stain *placement* must exist headless and be
/// deterministic. Turning one into an entity is the cosmetic half and belongs to the consumer.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Stain {
    /// Where it landed, subject-local, with `y` exactly on the plane it landed on.
    pub at: V3,
    /// **How much floor it wet**, metres — the placement scale, and the quantity a pool absorbs.
    ///
    /// Not the drawn silhouette: that is [`StainShape`], which carries the aspect ratio, the spines
    /// and the satellites. One number for "how much floor", one struct for "what shape" — they have
    /// different consumers and different tests, and collapsing them would make the pool feed depend
    /// on a cosmetic decision.
    pub radius: f32,
    /// A per-stain seed, for a caller choosing between variants without adding randomness.
    pub seed: u32,
}

/// The impact conditions one droplet arrived with. Everything [`stain_shape`] needs and nothing else.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Impact {
    /// Impact speed, m/s.
    pub speed: f32,
    /// Droplet diameter at impact, metres.
    pub diameter: f32,
    /// Angle between the droplet's path and the surface, radians. `π/2` is a perpendicular hit.
    pub angle_rad: f32,
    /// Substrate roughness in `[0, 1]`. Rough surfaces shorten the stain and merge its spines.
    pub roughness: f32,
    /// **In-plane direction of travel**, on the surface, as `(u, v)`. Need not be normalised; a zero
    /// vector means "no in-plane direction", which is what a perpendicular hit has.
    ///
    /// Carried here rather than left for the caller to write onto [`StainShape::direction`]
    /// afterwards, because a shape whose direction was fabricated reads back to the wrong origin in
    /// [`crate::bloodstain::origin::area_of_origin`] — and a value a caller has to remember to overwrite is a
    /// value that will be forgotten. [`impact_at_plane`] fills it from the droplet's own velocity.
    pub travel: [f32; 2],
}

/// **The silhouette of one stain**, derived from its impact rather than picked from a texture set.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StainShape {
    /// Long axis, metres — along [`direction`](Self::direction).
    pub major: f32,
    /// Short axis, metres. `minor / major = sin θ`, which is the whole of bloodstain-pattern
    /// analysis's impact-angle method run forwards.
    pub minor: f32,
    /// Rim spines, from Knock & Davison's law. Zero below [`SPINE_WE_MIN`].
    pub spines: u8,
    /// Detached satellite droplets, non-zero only past [`SPLASH_K_CRIT`].
    pub satellites: u8,
    /// In-plane unit direction of travel. Spines point **downrange**, away from where the droplet
    /// came from, and this is the axis that says which way that is.
    pub direction: [f32; 2],
    /// Per-stain seed, mixed into every jitter below so a shape is reproducible from its own inputs.
    pub seed: u32,
}

/// Weber number: inertia against surface tension, `ρ d v² / γ`.
///
/// Which of the two wins is what decides whether a drop deposits smoothly, throws spines, or splashes,
/// so it is the first thing both laws below read.
pub fn weber(diameter: f32, speed: f32) -> f32 {
    if !(diameter > 0.0) || !diameter.is_finite() || !speed.is_finite() {
        return 0.0;
    }
    BLOOD_DENSITY * diameter * speed * speed / BLOOD_SURFACE_TENSION
}

/// Reynolds number: inertia against viscosity, `ρ d v / μ`.
///
/// `viscosity` is the caller's, from [`crate::bloodstain::rheo::viscosity`] at the shear rate the impact implies —
/// blood is shear-thinning, so a Reynolds number taken at a fixed viscosity would be the wrong number
/// at exactly the impacts that matter.
pub fn reynolds(diameter: f32, speed: f32, viscosity: f32) -> f32 {
    if !(diameter > 0.0) || !(viscosity > 0.0) || !diameter.is_finite() || !speed.is_finite() {
        return 0.0;
    }
    BLOOD_DENSITY * diameter * speed / viscosity
}

/// **The stain one impact leaves.** Aspect from the angle, spines from the Weber number, satellites
/// from the splash parameter, all shortened by the substrate's roughness.
///
/// `seed` keys every jitter, so the same impact is the same silhouette on every machine and a stain a
/// millimetre away is a different one.
pub fn stain_shape(i: &Impact, s: &BloodSettings, seed: u32) -> StainShape {
    // A drop's spread factor: how much wider the splat is than the drop that made it. Grows with the
    // Weber number, which is the form the spreading literature agrees on across fluids; the exponent
    // is the low-viscosity limit's, and blood at impact shear rates sits near it.
    let we = weber(i.diameter, i.speed);
    let spread = 1.0 + 0.5 * m::powf(we.max(0.0), 0.25);
    let rough = if i.roughness.is_finite() { i.roughness.clamp(0.0, 1.0) } else { 0.0 };

    // Roughness shortens the stain: a rough substrate pins the advancing edge (Adam 2012).
    let major = (i.diameter * spread * (1.0 - 0.35 * rough)).max(i.diameter);
    // The impact-angle relation, and the reason a stain can be read backwards at all.
    let sin_theta = m::sin(i.angle_rad.clamp(0.0, PI)).clamp(0.0, 1.0);
    let minor = major * sin_theta;

    // Knock & Davison: spines scale with the square root of the Weber number and the CUBE of sin θ,
    // so a shallow impact throws far fewer spines than its energy alone would suggest.
    let spines = if we < SPINE_WE_MIN {
        0u8
    } else {
        let raw = SPINE_COEFF * m::sqrt(we) * sin_theta * sin_theta * sin_theta;
        // Roughness merges neighbouring spines rather than removing them (Adam 2012), so the count
        // falls by half the roughness fraction.
        let merged = m::round(raw) - m::round(rough * m::round(raw) * 0.5);
        merged.clamp(0.0, SPINE_MAX as f32) as u8
    };

    // Mundo's splash parameter. Satellites are spine tips that pinched off, so they cannot outnumber
    // the spines and are capped by them.
    let mu = crate::bloodstain::rheo::viscosity(i.speed / i.diameter.max(1.0e-6), s.hematocrit, s);
    let re = reynolds(i.diameter, i.speed, mu);
    let k = m::sqrt(we) * m::powf(re.max(0.0), 0.25);
    let satellites = if k < SPLASH_K_CRIT {
        0u8
    } else {
        let over = (k / SPLASH_K_CRIT - 1.0).clamp(0.0, 4.0);
        let n = m::round(spines as f32 * 0.5 * over);
        n.clamp(0.0, spines as f32) as u8
    };

    // The travel direction is a **measurement of the scene**, never a look, so it is taken from the
    // impact rather than invented. A perpendicular hit has no in-plane direction at all — its stain
    // is a circle — so the spine arc is oriented by the seed instead, which is the only choice that
    // is not a fabricated bias in a direction nothing measured.
    let (tvx, tvz) = (i.travel[0], i.travel[1]);
    let tlen = m::sqrt(tvx * tvx + tvz * tvz);
    let direction = if tlen > 0.0 {
        [tvx / tlen, tvz / tlen]
    } else {
        let phi = TAU * hash_f32(seed ^ 0x5F35_6495);
        [m::cos(phi), m::sin(phi)]
    };

    StainShape { major, minor, spines, satellites, direction, seed }
}

impl StainShape {
    /// The impact angle this shape encodes, radians — `asin(minor / major)`.
    ///
    /// **The inverse of the forward model, and the whole basis of [`crate::bloodstain::origin`].** A stain that
    /// could not be read backwards would be a stain with no forensic content, and a forward model
    /// that cannot be inverted by the published method is a forward model that got the relation
    /// wrong.
    pub fn impact_angle(&self) -> f32 {
        if !(self.major > 0.0) {
            return PI * 0.5;
        }
        let ratio = (self.minor / self.major).clamp(-1.0, 1.0);
        // `asinf` via the identity `atan2(x, sqrt(1 - x²))`, so this module needs no extra libm entry
        // point and the branch at |x| = 1 is handled by the sqrt going to zero.
        m::atan2(ratio, m::sqrt((1.0 - ratio * ratio).max(0.0)))
    }
}

/// **Rasterise a stain's coverage mask**, one byte per texel, `px × px`, row-major.
///
/// The mask is the *shape* only: `0` outside, `255` at full coverage. Colour is the caller's — a
/// consumer's material carries the blood colour and [`crate::bloodstain::dry::appearance`] walks it, so one mask
/// serves fresh blood and a week-old crust.
///
/// Returns without writing if `out` is not exactly `px * px` bytes: a partially-filled mask is a
/// texture with garbage in it, and refusing is the only honest answer to a size mismatch.
pub fn rasterise(shape: &StainShape, px: u32, out: &mut [u8]) -> bool {
    let n = px as usize;
    if px == 0 || out.len() != n * n {
        return false;
    }
    out.fill(0);

    let aspect = if shape.major > 0.0 { (shape.minor / shape.major).clamp(0.02, 1.0) } else { 1.0 };
    let (dx, dy) = (shape.direction[0], shape.direction[1]);
    let dlen = m::sqrt(dx * dx + dy * dy);
    let (ux, uy) = if dlen > 0.0 { (dx / dlen, dy / dlen) } else { (1.0, 0.0) };

    // Spine geometry, precomputed once per mask rather than per texel: an angle, a reach past the rim
    // and an angular width each. The arc they occupy narrows with the aspect ratio, which is what
    // makes a shallow impact throw its spines DOWNRANGE instead of all around — the photographed
    // behaviour, and the reason `direction` exists.
    let spines = shape.spines as usize;
    let mut spine_angle = [0.0f32; SPINE_MAX as usize];
    let mut spine_reach = [0.0f32; SPINE_MAX as usize];
    let mut spine_width = [0.0f32; SPINE_MAX as usize];
    let arc = PI * aspect;
    for k in 0..spines.min(SPINE_MAX as usize) {
        let key = shape.seed ^ (k as u32).wrapping_mul(0x9E37_79B9);
        let centred = if spines > 1 { k as f32 / (spines as f32 - 1.0) - 0.5 } else { 0.0 };
        // Evenly spaced across the arc, then jittered by a fraction of one spacing — even spacing
        // alone reads as a gear, pure jitter alone clumps.
        let jitter = (hash_f32(key) - 0.5) * (arc / spines.max(1) as f32);
        spine_angle[k] = centred * 2.0 * arc + jitter;
        spine_reach[k] = 0.18 + hash_f32(key ^ 0xC2B2_AE35) * 0.42;
        spine_width[k] = 0.05 + hash_f32(key ^ 0x27D4_EB2F) * 0.10;
    }

    // **The texture holds the WHOLE silhouette, spines and satellites included.**
    //
    // The body was originally drawn at half the texture width, which clipped every spine and put
    // satellites in the corners — measured as an all-but-uniform rim and non-zero alpha at the
    // texture corners, i.e. a stain that read as a filled square. So the ellipse is shrunk by the
    // furthest thing the silhouette reaches: a stain with long spines gets a proportionally smaller
    // body, which is also what a photograph shows, because the same volume of blood went further.
    let max_spine = (0..spines.min(SPINE_MAX as usize))
        .map(|k| spine_reach[k])
        .fold(0.0f32, f32::max);
    let max_satellite = if shape.satellites > 0 {
        // Matches the satellite placement below: rim + reach + offset + radius, at their maxima.
        1.0 + max_spine + 0.35 + 0.12
    } else {
        0.0
    };
    // A small margin so the outermost texel of the furthest feature is still inside the texture
    // rather than exactly on its edge.
    let extent = (1.0 + max_spine).max(max_satellite) * 1.04;
    let a = 0.5f32 / extent;
    let b = a * aspect;

    let half = px as f32 * 0.5;
    for y in 0..n {
        for x in 0..n {
            // Pixel centres, so the mask is symmetric about the texture centre rather than half a
            // pixel off it.
            let tx = (x as f32 + 0.5 - half) / half * 0.5;
            let ty = (y as f32 + 0.5 - half) / half * 0.5;
            // Into the stain's own frame: `u` along travel, `v` across it.
            let u = tx * ux + ty * uy;
            let v = -tx * uy + ty * ux;

            // Elliptical body. `r` is 1 exactly on the rim.
            let r = m::sqrt((u / a) * (u / a) + (v / b) * (v / b));
            let theta = m::atan2(v / b.max(1.0e-6), u / a.max(1.0e-6));

            let mut rim = 1.0f32;
            for k in 0..spines.min(SPINE_MAX as usize) {
                let mut d = m::abs(theta - spine_angle[k]);
                if d > PI {
                    d = TAU - d;
                }
                let falloff = m::exp(-(d * d) / (2.0 * spine_width[k] * spine_width[k]));
                rim += spine_reach[k] * falloff;
            }

            let mut cover = if r >= rim {
                0.0
            } else {
                let t = (1.0 - r / rim).clamp(0.0, 1.0);
                // Squared, so the body stays solid and only the last of the radius fades.
                (t * t * 3.2).min(1.0)
            };

            // Satellites: small discs beyond the rim at the spine angles, because a satellite IS a
            // spine tip that pinched off. Placing them anywhere else would make them look like a
            // second, unrelated spray.
            for k in 0..(shape.satellites as usize).min(spines.min(SPINE_MAX as usize)) {
                let key = shape.seed ^ (k as u32).wrapping_mul(0x85EB_CA6B);
                let ang = spine_angle[k];
                let dist = 1.0 + spine_reach[k] + 0.10 + hash_f32(key) * 0.25;
                let rad = 0.05 + hash_f32(key ^ 0x1656_67B1) * 0.07;
                let (sx, sy) = (a * dist * m::cos(ang), b * dist * m::sin(ang));
                let (ddx, ddy) = (u - sx, v - sy);
                let dd = m::sqrt(ddx * ddx + ddy * ddy);
                if dd < rad {
                    let t = (1.0 - dd / rad).clamp(0.0, 1.0);
                    cover = cover.max((t * t * 3.2).min(1.0));
                }
            }

            out[y * n + x] = m::round(cover * 255.0).clamp(0.0, 255.0) as u8;
        }
    }
    true
}

/// Stain radius from droplet diameter and impact speed.
///
/// A spreading droplet's splat is wider than the droplet, growing with impact speed — the spread
/// factor. This is the game-scale form of it: the diameter's own position in the settings' size span
/// sets the base, impact speed widens it across the measured speed span, and the result is clamped
/// into the authored radius range so a stain is never smaller than a pixel or wider than a puddle.
///
/// **This is placement, not morphology** — see [`Stain::radius`]. The silhouette's aspect, spines and
/// satellites come from [`stain_shape`], which reads the physics rather than an authored span.
pub fn stain_radius(d: &Droplet, impact_speed: f32, s: &BloodSettings) -> f32 {
    let span = (s.droplet_size_max - s.droplet_size_min).max(f32::MIN_POSITIVE);
    let size_frac = ((d.diameter - s.droplet_size_min) / span).clamp(0.0, 1.0);
    let speed_span = (FORWARD_SPATTER_SPEED - BACK_SPATTER_SPEED).max(f32::MIN_POSITIVE);
    let speed_frac = ((impact_speed - BACK_SPATTER_SPEED) / speed_span).clamp(0.0, 1.0);
    // Size dominates and speed widens: a big slow droplet still makes the bigger mark, which is what
    // the spread-factor correlation says and what a photograph of one shows.
    let frac = (0.7 * size_frac + 0.3 * speed_frac).clamp(0.0, 1.0);
    s.stain_radius_min + (s.stain_radius_max - s.stain_radius_min) * frac
}

/// The stains one wound leaves on a horizontal plane, in droplet-index order.
///
/// Droplets that never reach the plane are skipped rather than pinned to it. The order is the droplet
/// ordinal, which is total by construction, so this needs no sort — and a caller folding a hash over
/// the result gets the same digest every run.
pub fn stains(w: &Wound, s: &BloodSettings, plane_y: f32) -> Vec<Stain> {
    let spray = Spray::of(w, s);
    // Invariant across the droplet ordinal.
    let fall = (w.at[1] - plane_y).max(0.0);
    let n = droplet_count(w, s);
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let d = spray.droplet(i, s);
        let Some(at) = landing(w.at, &d, s.gravity, plane_y) else {
            continue;
        };
        // Impact speed from the same closed form the landing came from: vertical speed gained over
        // the drop, horizontal speed unchanged, because the drag dial is a look control on the
        // particles rather than a second integrator here.
        let vy = d.dir[1] * d.speed;
        let impact = m::sqrt((vy * vy + 2.0 * s.gravity * fall).max(0.0));
        let horizontal = vec::length(vec::sub(
            vec::scale(d.dir, d.speed),
            vec::scale(vec::Y, vy),
        ));
        let impact_speed = m::sqrt(impact * impact + horizontal * horizontal);
        out.push(Stain {
            at,
            radius: stain_radius(&d, impact_speed, s),
            seed: spray.seed ^ i,
        });
    }
    out
}

/// The impact one droplet arrives with at a horizontal plane, ready for [`stain_shape`].
///
/// Derived from the same closed form [`stains`] uses, so a stain's placement and its silhouette
/// describe **one** droplet rather than two approximations of one.
pub fn impact_at_plane(d: &Droplet, from: V3, plane_y: f32, s: &BloodSettings) -> Impact {
    let fall = (from[1] - plane_y).max(0.0);
    let vy = d.dir[1] * d.speed;
    let down = m::sqrt((vy * vy + 2.0 * s.gravity * fall).max(0.0));
    let horizontal =
        vec::length(vec::sub(vec::scale(d.dir, d.speed), vec::scale(vec::Y, vy)));
    let speed = m::sqrt(down * down + horizontal * horizontal);
    // Angle between the path and the surface: `atan2(vertical, horizontal)`. A purely vertical
    // arrival is π/2 and stains a circle, which is the relation's own boundary case.
    let angle_rad = if horizontal > 0.0 { m::atan2(down, horizontal) } else { PI * 0.5 };
    // The in-plane direction of travel, taken from the droplet's own horizontal velocity — the one
    // place in the crate that knows it, which is why it belongs here rather than in a caller.
    let hx = d.dir[0] * d.speed;
    let hz = d.dir[2] * d.speed;
    let hlen = m::sqrt(hx * hx + hz * hz);
    let travel = if hlen > 0.0 { [hx / hlen, hz / hlen] } else { [0.0, 0.0] };
    Impact { speed, diameter: d.diameter, angle_rad, roughness: s.substrate_roughness, travel }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bloodstain::{WoundKind, vec};
    use std::vec::Vec as StdVec;

    fn fixed_wound() -> Wound {
        Wound {
            at: [0.1, 0.9, -0.2],
            normal: vec::X,
            area: 0.004,
            severity: 1.0,
            kind: WoundKind::Severance,
        }
    }

    /// The stains the same wound leaves, frozen alongside the droplets — because a caller's digest is
    /// taken over stain positions, not over droplet directions, so this is the value that actually
    /// travels downstream.
    ///
    /// **Moved from `bevy_carnage::spatter` with its bits unchanged**, which is the evidence that the
    /// move was a move. Read [`crate::bloodstain::droplet::tests::the_spatter_model_is_frozen`] for the two
    /// re-blessings this table has survived and the rule that still binds: if these move while the
    /// profile is held fixed, the model moved.
    #[test]
    fn the_stain_placement_is_frozen() {
        let w = fixed_wound();
        let s = BloodSettings::default();
        let got = stains(&w, &s, 0.0);
        assert_eq!(got.len(), 10, "area x density must give this many droplets");

        let expect: [([u32; 3], u32); 4] = [
            ([0x40879314, 0x00000000, 0x3FDEEF19], 0x3D7E1738),
            ([0x4169C84C, 0x00000000, 0xC0ABEC1B], 0x3D9EE6FE),
            ([0x410B1577, 0x00000000, 0x3FBEFBA5], 0x3DAA9FCC),
            ([0x413B5528, 0x00000000, 0xBFF37568], 0x3D641D9F),
        ];
        let actual: StdVec<([u32; 3], u32)> = got
            .iter()
            .take(4)
            .map(|st| {
                ([st.at[0].to_bits(), st.at[1].to_bits(), st.at[2].to_bits()], st.radius.to_bits())
            })
            .collect();
        let rendered: StdVec<std::string::String> = actual
            .iter()
            .map(|(at, r)| {
                std::format!("([0x{:08X}, 0x{:08X}, 0x{:08X}], 0x{r:08X}),", at[0], at[1], at[2])
            })
            .collect();
        assert_eq!(
            actual.as_slice(),
            expect.as_slice(),
            "stain placement moved. If that was deliberate, the new bits are:\n{}",
            rendered.join("\n")
        );
    }

    /// A landing is downrange along the droplet's own direction, not under the wound. A spray that
    /// stained the floor beneath the body would be the bug this asserts against.
    #[test]
    fn blood_lands_downrange_of_the_wound() {
        let s = BloodSettings::default();
        let w = fixed_wound();
        let got = stains(&w, &s, 0.0);
        assert!(!got.is_empty(), "a severity-1 wound of this area must stain the floor");
        for st in &got {
            assert!(st.at[0] > w.at[0], "a wound facing +X stained upstream at {}", st.at[0]);
            assert!(
                (s.stain_radius_min..=s.stain_radius_max).contains(&st.radius),
                "stain radius {} is outside the authored range",
                st.radius
            );
        }
    }

    /// A bigger droplet leaves a bigger mark at the same impact speed — the spread factor's own
    /// direction, and the one thing about `stain_radius` a caller can see.
    #[test]
    fn a_bigger_droplet_stains_wider() {
        let s = BloodSettings::default();
        let small = Droplet { dir: vec::X, speed: 30.0, diameter: s.droplet_size_min };
        let large = Droplet { dir: vec::X, speed: 30.0, diameter: s.droplet_size_max };
        assert!(
            stain_radius(&large, 30.0, &s) > stain_radius(&small, 30.0, &s),
            "the larger droplet must leave the wider stain"
        );
        let slow = Droplet { dir: vec::X, speed: 8.0, diameter: 0.003 };
        assert!(
            stain_radius(&slow, FORWARD_SPATTER_SPEED, &s)
                > stain_radius(&slow, BACK_SPATTER_SPEED, &s),
            "a faster impact must spread wider at the same size"
        );
    }

    /// **The impact-angle relation, forwards and backwards.** `minor / major = sin θ` is the whole
    /// basis of bloodstain-pattern analysis, so it is asserted rather than commented.
    #[test]
    fn the_aspect_ratio_is_the_sine_of_the_impact_angle() {
        let s = BloodSettings { substrate_roughness: 0.0, ..Default::default() };
        for deg in [15.0f32, 30.0, 45.0, 60.0, 75.0, 90.0] {
            let i = Impact {
                speed: 5.0,
                diameter: 0.004,
                angle_rad: crate::bloodstain::to_radians(deg),
                roughness: 0.0,
                travel: [1.0, 0.0],
            };
            let shape = stain_shape(&i, &s, 0x1234);
            let expect = m::sin(crate::bloodstain::to_radians(deg));
            let got = shape.minor / shape.major;
            assert!(
                m::abs(got - expect) < 1.0e-4,
                "at {deg} deg the aspect must be sin θ = {expect}, got {got}"
            );
            let back = shape.impact_angle();
            assert!(
                m::abs(back - i.angle_rad) < 1.0e-3,
                "the shape must read back to its own impact angle: {} vs {}",
                back,
                i.angle_rad
            );
        }
    }

    /// Spines are zero below the onset, rise with the Weber number, fall with the cube of sin θ, and
    /// saturate — the four claims Knock & Davison's law and Adam's limits make between them.
    #[test]
    fn spines_follow_the_blood_specific_law() {
        let s = BloodSettings { substrate_roughness: 0.0, ..Default::default() };
        let at = |speed: f32, deg: f32| {
            stain_shape(
                &Impact {
                    speed,
                    diameter: 0.004,
                    angle_rad: crate::bloodstain::to_radians(deg),
                    roughness: 0.0,
                    travel: [1.0, 0.0],
                },
                &s,
                7,
            )
            .spines
        };
        assert_eq!(at(0.05, 90.0), 0, "a drop below the Weber onset deposits a smooth ellipse");
        assert!(at(6.0, 90.0) > at(2.0, 90.0), "more energy must mean more spines");
        assert!(
            at(6.0, 20.0) < at(6.0, 90.0),
            "a shallow impact must throw far fewer spines — that is the sin³θ term"
        );
        assert!(at(400.0, 90.0) <= SPINE_MAX, "the count must saturate at the documented ceiling");
        let rough = stain_shape(
            &Impact { speed: 6.0, diameter: 0.004, angle_rad: PI * 0.5, roughness: 1.0, travel: [1.0, 0.0] },
            &s,
            7,
        );
        assert!(
            rough.spines < at(6.0, 90.0) && rough.major < 0.004 * 40.0,
            "a rough substrate must merge spines and shorten the stain"
        );
    }

    /// Satellites appear only past the splash threshold, and never outnumber the spines they came
    /// from — a satellite is a spine tip that pinched off.
    #[test]
    fn satellites_only_appear_past_the_splash_threshold() {
        let s = BloodSettings::default();
        let gentle =
            stain_shape(&Impact { speed: 0.4, diameter: 0.003, angle_rad: PI * 0.5, roughness: 0.0, travel: [1.0, 0.0] }, &s, 3);
        assert_eq!(gentle.satellites, 0, "a gentle deposit must not splash");
        let violent =
            stain_shape(&Impact { speed: 40.0, diameter: 0.005, angle_rad: PI * 0.5, roughness: 0.0, travel: [1.0, 0.0] }, &s, 3);
        assert!(violent.satellites > 0, "a 40 m/s impact must be past Mundo's boundary");
        assert!(
            violent.satellites <= violent.spines,
            "satellites ({}) cannot outnumber the spines they detached from ({})",
            violent.satellites,
            violent.spines
        );
    }

    /// The mask is reproducible, refuses a wrong-sized buffer, and actually covers the middle. A mask
    /// that came back empty would be an invisible stain, which is the failure mode a green build
    /// hides.
    #[test]
    fn the_mask_is_reproducible_and_not_empty() {
        let s = BloodSettings::default();
        let shape = stain_shape(
            &Impact { speed: 8.0, diameter: 0.004, angle_rad: crate::bloodstain::to_radians(40.0), roughness: 0.1, travel: [1.0, 0.0] },
            &s,
            0xBEEF,
        );
        let px = 48u32;
        let mut a = std::vec![0u8; (px * px) as usize];
        let mut b = std::vec![0u8; (px * px) as usize];
        assert!(rasterise(&shape, px, &mut a));
        assert!(rasterise(&shape, px, &mut b));
        assert_eq!(a, b, "the same shape must rasterise to the same bytes");
        assert_eq!(
            a[(px * px / 2 + px / 2) as usize],
            255,
            "the centre of the stain must be fully covered"
        );
        let filled = a.iter().filter(|&&v| v > 0).count();
        assert!(filled > 16, "the mask covered only {filled} texels, which is an invisible stain");
        assert!(
            filled < (px * px) as usize,
            "the mask covered every texel, which is a square stain"
        );

        let mut wrong = std::vec![0u8; 10];
        assert!(!rasterise(&shape, px, &mut wrong), "a wrong-sized buffer must be refused");
    }

    /// A shallow impact's mask must be visibly longer than it is wide, along the travel direction —
    /// the aspect ratio surviving all the way to the texels a consumer uploads.
    #[test]
    fn a_shallow_impact_rasterises_an_elongated_stain() {
        let s = BloodSettings::default();
        let shape = stain_shape(
            &Impact { speed: 6.0, diameter: 0.004, angle_rad: crate::bloodstain::to_radians(15.0), roughness: 0.0, travel: [1.0, 0.0] },
            &s,
            1,
        );
        let px = 64usize;
        let mut mask = std::vec![0u8; px * px];
        assert!(rasterise(&shape, px as u32, &mut mask));
        let row = mask[(px / 2) * px..(px / 2 + 1) * px].iter().filter(|&&v| v > 0).count();
        let col = (0..px).filter(|&y| mask[y * px + px / 2] > 0).count();
        assert!(
            row > col * 2,
            "a 15 deg impact must be at least twice as long as it is wide: {row} vs {col}"
        );
    }
}
