//! **Blood that accumulates.** Stains land, merge into slicks, and the slicks spread.
//!
//! [`crate::spatter::stains`] returns independent [`Stain`]s that never interact, so a hundred
//! droplets landing in one place leave a hundred separate discs rather than a wet patch. This is the
//! fold that turns them into one.
//!
//! # Core, not `vfx`
//!
//! Deterministic, integer-ticked, and on this side of the feature gate — not because pooling is
//! cheap, but because **where blood pools is simulation-visible**. The consuming game reads pool
//! positions as a chemoattractant, so two runs that kill the same body must pool in the same places,
//! bit for bit. Drawing them is the cosmetic half and lives in `decal`, behind `vfx`.
//!
//! # The plane is the scope
//!
//! Pools form on the **single horizontal plane** the spatter model already solves against
//! ([`crate::spatter::landing`]). Flowing downhill to the lowest reachable point needs a heightfield
//! this crate does not have, and growing one here would be a second world model beside the proxy
//! cells.

use bevy::math::Vec3;

use crate::CarnageSettings;
use crate::spatter::Stain;

/// **How much further than [`CarnageSettings::pool_merge_radius`] a stain may reach once the pool
/// list is full.**
///
/// At the ceiling the choice is "join something further away" or "throw this blood away", and joining
/// is the better answer up to a point — past three merge radii the stain would be joining a slick it
/// visibly is not touching, which reads worse than not drawing it at all.
const POOL_CAP_SLACK: f32 = 3.0;

/// **A slick on the floor.** Grows as blood arrives, merges with its neighbours, and stops.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Pool {
    /// On the plane, in world units. `y` is exactly the plane the stains landed on.
    pub at: Vec3,
    /// Accumulated blood, in the same units as [`Stain::radius`] squared — an **area**, not a volume,
    /// because the spatter model measures what a droplet *wets*, never how deep it is.
    pub wetted: f32,
    /// Current drawn radius, which lags [`Self::wetted`] so a pool visibly spreads rather than
    /// snapping to its final size the frame the blood arrives.
    pub radius: f32,
    /// Ticks since this pool was created. Integer, like every clock in this crate.
    pub age: u32,
    /// Carried from the first stain that formed it, so a decal can pick a variant without inventing
    /// randomness of its own.
    pub seed: u32,
}

/// **Fold fresh stains into an existing set of pools, merging by proximity.**
///
/// Stains are consumed in the order given, which is already total — [`crate::spatter::stains`] emits
/// in droplet-ordinal order and says why it needs no sort.
///
/// **Takes no clock.** A fresh pool starts at age 0 and [`spread_pools`] is what advances it, so the
/// caller's tick number would be a parameter this function reads and ignores. That is the same
/// one-path rule the rest of the crate keeps: the tick lives with the system that calls
/// `spread_pools` once per tick.
///
/// # First match, not nearest
///
/// A nearest-pool search needs a float comparison that can tie, and a tie would be broken by whatever
/// order the vector happened to be in — which is exactly the class of instability
/// [`crate::order::sort_total_by_key_at`] exists to remove. First-match is a total function of the
/// input order, so it is the rule.
///
/// # The centre never moves
///
/// A hit adds area and leaves [`Pool::at`] alone. Averaging the centroid would make the drawn decal
/// slide across the floor as more blood lands on one side of it, which reads as a bug rather than as
/// spreading.
pub fn absorb(pools: &mut Vec<Pool>, stains: &[Stain], s: &CarnageSettings) {
    let merge_r2 = s.pool_merge_radius * s.pool_merge_radius;
    let slack_r2 = merge_r2 * POOL_CAP_SLACK * POOL_CAP_SLACK;
    for stain in stains {
        let full = pools.len() >= s.max_pools as usize;
        let reach2 = if full { slack_r2 } else { merge_r2 };
        let hit = pools.iter_mut().find(|p| p.at.distance_squared(stain.at) <= reach2);
        if let Some(p) = hit {
            p.wetted += stain.radius * stain.radius;
            continue;
        }
        // **At the ceiling a stain out of reach is dropped, and that is the correct answer.** The
        // alternative is unbounded growth in a system whose whole job is to accumulate. An existing
        // pool is never evicted to make room — same first-come-first-served rule as the ribbon cap,
        // and for the same reason: a slick that vanishes reads as a glitch.
        if full {
            continue;
        }
        pools.push(Pool {
            at: stain.at,
            wetted: stain.radius * stain.radius,
            radius: stain.radius,
            age: 0,
            seed: stain.seed,
        });
    }
}

/// **Advance every pool one tick toward the radius its wetted area implies.**
///
/// The target is the radius of a disc of area `wetted`, scaled by
/// [`CarnageSettings::pool_spread`] — blood creeps outward after the impact area was measured.
///
/// Multiplicative approach rather than a lerp on [`Pool::age`], deliberately: a pool that keeps
/// receiving blood keeps spreading, instead of freezing the moment some timer expires.
pub fn spread_pools(pools: &mut [Pool], s: &CarnageSettings) {
    for p in pools.iter_mut() {
        let target = (p.wetted / std::f32::consts::PI).sqrt() * s.pool_spread;
        p.radius += (target - p.radius) * s.pool_spread_rate;
        p.age = p.age.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wound::{Wound, WoundKind};

    /// The same wound `spatter::tests::fixed_wound` freezes, so the two tables describe one geometry.
    fn fixed_wound() -> Wound {
        Wound {
            at: Vec3::new(0.1, 0.9, -0.2),
            normal: Vec3::X,
            area: 0.004,
            severity: 1.0,
            kind: WoundKind::Severance,
        }
    }

    /// One wound's stains, absorbed and then spread for a second — the input the property tests use.
    fn settled() -> Vec<Pool> {
        let s = CarnageSettings::default();
        let stains = crate::spatter::stains(&fixed_wound(), &s, 0.0);
        let mut pools = Vec::new();
        absorb(&mut pools, &stains, &s);
        for _ in 0..60 {
            spread_pools(&mut pools, &s);
        }
        pools
    }

    /// **These bits are the API**, on exactly the terms `spatter::tests::the_spatter_model_is_frozen`
    /// sets out. Pool placement is read by the consuming simulation as a chemoattractant source, so a
    /// caller's replay is defined against these values. A change to the merge rule, the first-match
    /// choice, the area accumulation or the spread integrator moves every one of them.
    ///
    /// **This is not a snapshot to re-bless.** If it fails, the model moved, and the question is
    /// whether that was intended — not whether the numbers should be updated to match.
    ///
    /// # Why the fixture is three hand-written stains and not `spatter::stains`
    ///
    /// Deliberate, and it is what makes the table *checkable* rather than merely recorded. Every
    /// number below is a short chain of exactly-representable `f32` operations over exact binary
    /// fractions — `0.25`, `0.125`, `0.5` — with **no transcendental anywhere**: the only non-trivial
    /// step is `sqrt`, which IEEE-754 specifies as correctly rounded. So the expected values can be
    /// derived independently and confirmed against the closed form
    /// `r_n = target + (r_0 − target)·(1 − rate)^n`, which is exactly what was done. Driving it from
    /// `spatter::stains` instead would route the fixture through the spray cone's `sin`/`cos`, and a
    /// libm that differs by one ULP would move this table for a reason that has nothing to do with
    /// pooling. `settled()` above still exercises the spatter path — in the property tests, where a
    /// ULP cannot matter.
    ///
    /// Both `sqrt` results were checked to sit **~0.2 ULP from the nearest f32 rounding midpoint**, so
    /// the correctly-rounded answer is unambiguous and no double-rounding through a wider intermediate
    /// can change it.
    ///
    /// **The one thing that could still move these bits without the model moving is FP contraction.**
    /// `p.radius += (target - p.radius) * rate` is exactly the shape an FMA fuses, and a fused
    /// multiply-add rounds once where this rounds twice. Rust compiles with `fp-contract` off, and
    /// `spatter.rs`'s own investigation confirmed contraction was not what moved its table — but if
    /// these values ever differ by a single ULP on a new target, check that before concluding the
    /// arithmetic changed.
    ///
    /// # What the three stains are for
    ///
    /// `s0` starts a pool. `s1` lands 0.03 away, inside the 0.10 merge radius, so it **joins** — the
    /// centre stays `s0`'s, the area is the sum of both squared radii, and the seed stays `s0`'s. `s2`
    /// is a metre off, so it starts its own. That is the whole model in three inputs, and each of the
    /// three assertions below fails for a different edit.
    #[test]
    fn the_pool_model_is_frozen() {
        let s = CarnageSettings::default();
        let stains = [
            Stain { at: Vec3::new(0.5, 0.0, -0.25), radius: 0.25, seed: 0x1111_1111 },
            Stain { at: Vec3::new(0.53, 0.0, -0.25), radius: 0.125, seed: 0x2222_2222 },
            Stain { at: Vec3::new(1.5, 0.0, 0.75), radius: 0.5, seed: 0x3333_3333 },
        ];
        let mut pools = Vec::new();
        absorb(&mut pools, &stains, &s);
        assert_eq!(pools.len(), 2, "the middle stain must have merged into the first pool");
        assert_eq!(pools[0].seed, 0x1111_1111, "a pool keeps the seed of the stain that formed it");
        for _ in 0..60 {
            spread_pools(&mut pools, &s);
        }

        // (centre bits, radius bits, wetted bits). `wetted` is 0.25² + 0.125² = 0.078125 for the
        // merged pool and 0.5² = 0.25 for the lone one, both exact.
        let expect: [([u32; 3], u32, u32); 2] = [
            ([0x3F000000, 0x00000000, 0xBE800000], 0x3E5A4104, 0x3DA00000),
            ([0x3FC00000, 0x00000000, 0x3F400000], 0x3EC364D3, 0x3E800000),
        ];

        let actual: Vec<([u32; 3], u32, u32)> = pools
            .iter()
            .map(|p| {
                (
                    [p.at.x.to_bits(), p.at.y.to_bits(), p.at.z.to_bits()],
                    p.radius.to_bits(),
                    p.wetted.to_bits(),
                )
            })
            .collect();
        let rendered: Vec<String> = actual
            .iter()
            .map(|(at, r, w)| {
                format!("([0x{:08X}, 0x{:08X}, 0x{:08X}], 0x{r:08X}, 0x{w:08X}),", at[0], at[1], at[2])
            })
            .collect();
        assert_eq!(
            actual.as_slice(),
            expect.as_slice(),
            "the pool model moved. If that was deliberate, the new bits are:\n{}",
            rendered.join("\n")
        );

        // And the ages, which the bit table cannot carry: `spread_pools` is the only clock.
        assert!(pools.iter().all(|p| p.age == 60), "one `spread_pools` is one tick of age");
    }

    /// **Merging is a total function of the input order**, which a golden cannot express: the golden
    /// pins one run's output, this pins that the same input twice cannot disagree.
    ///
    /// This is the property a future "merge with the *nearest* pool" edit would break — a float
    /// distance comparison can tie, and a tie would be resolved by whatever order the vector happened
    /// to be in.
    #[test]
    fn absorbing_the_same_stains_twice_gives_bit_identical_pools() {
        let a = settled();
        let b = settled();
        assert!(!a.is_empty(), "precondition: the wound pooled at all");
        assert_eq!(a.len(), b.len(), "two folds of one stain list produced different pool counts");
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(
                (x.at.x.to_bits(), x.at.y.to_bits(), x.at.z.to_bits(), x.radius.to_bits(), x.wetted.to_bits()),
                (y.at.x.to_bits(), y.at.y.to_bits(), y.at.z.to_bits(), y.radius.to_bits(), y.wetted.to_bits()),
                "pool {i} differs between two identical folds"
            );
        }
    }

    /// **The cap holds and nothing panics**, over far more scattered stains than any death produces.
    ///
    /// Scattered deliberately: 10,000 stains over 100 m² are mostly further apart than
    /// `pool_merge_radius`, so this exercises the push path and then the at-capacity path, which is
    /// the one that would otherwise grow without bound.
    #[test]
    fn the_pool_cap_holds_under_ten_thousand_scattered_stains() {
        let s = CarnageSettings::default();
        let stains: Vec<Stain> = (0..10_000u32)
            .map(|i| {
                // A deterministic scatter over a 10 m × 10 m patch, from the crate's own frozen hash.
                let x = crate::soup::hash_f32(i.wrapping_mul(2_654_435_761)) * 10.0 - 5.0;
                let z = crate::soup::hash_f32(i.wrapping_mul(2_246_822_519) ^ 0x5EED) * 10.0 - 5.0;
                Stain { at: Vec3::new(x, 0.0, z), radius: 0.03, seed: i }
            })
            .collect();
        let mut pools = Vec::new();
        absorb(&mut pools, &stains, &s);
        assert_eq!(
            pools.len(),
            s.max_pools as usize,
            "a scatter this wide must fill the cap exactly, and never exceed it"
        );
        // And the blood that could not be placed was dropped, not lost track of: every pool still
        // holds a finite, positive area.
        for (i, p) in pools.iter().enumerate() {
            assert!(p.wetted > 0.0 && p.wetted.is_finite(), "pool {i} has a broken area {}", p.wetted);
        }
    }

    /// Blood arriving in one place makes **one** growing slick, not a pile of discs — the whole point
    /// of the module, asserted as the input-to-output statement it is.
    #[test]
    fn repeated_stains_in_one_place_become_one_growing_slick() {
        let s = CarnageSettings::default();
        let at = Vec3::new(0.4, 0.0, -0.25);
        let batch: Vec<Stain> =
            (0..40u32).map(|i| Stain { at, radius: 0.03, seed: i }).collect();
        let mut pools = Vec::new();
        absorb(&mut pools, &batch, &s);
        assert_eq!(pools.len(), 1, "forty stains on one point must be one pool");

        let before = pools[0].radius;
        for _ in 0..60 {
            spread_pools(&mut pools, &s);
        }
        let after = pools[0].radius;
        assert!(after > before, "the slick did not spread: {before} -> {after}");
        assert_eq!(pools[0].age, 60, "age is the caller's tick count, one per `spread_pools`");
    }
}
