//! **Expressive range: does the model actually produce different-looking blood?**
//!
//! A generator can be deterministic, physically sourced and fully tested and still produce one
//! pattern at six intensities. That failure is invisible to every other kind of test — each mechanism
//! passes its own unit tests while the *set* of outputs collapses onto a single blob — so it needs a
//! measurement of its own.
//!
//! The method is the one the procedural-generation literature calls an expressive-range analysis:
//! sample the generator across its input space, project each output onto a small number of
//! interpretable metrics, and histogram. Two failure modes then become numbers:
//!
//! - **Collapse** — too few occupied buckets. The generator has fewer distinct outputs than it has
//!   knobs.
//! - **Domination** — one bucket holding most of the samples. The generator has a favourite, and
//!   everything else is a rounding error a player will never see.

use bevy_carnage::bloodstain::patterns::{
    self, PatternClass, arterial_arc, cast_off, drip_trail, expirated, impact_spatter, transfer,
};
use bevy_carnage::bloodstain::{
    BloodSettings, Bleed, Droplet, Stain, Wound, WoundKind, bag, hash_f32, landing, stain,
};

/// Where every sample is thrown from — chest height, so the field lands on the floor plane.
const SOURCE: [f32; 3] = [0.0, 1.4, 0.0];

/// How many samples the sweep takes. The plan's number.
const SAMPLES: usize = 512;

/// Buckets per axis. `6 × 6 = 36` cells, of which the plan requires at least 12 occupied.
const BUCKETS: usize = 6;

/// Five interpretable numbers per pattern. Named rather than a tuple, because the whole point is that
/// a human can read the failure message.
#[derive(Clone, Copy, Debug, Default)]
struct Metrics {
    /// How many marks it left.
    stains: usize,
    /// Furthest mark from the pattern's own centroid, metres.
    spread: f32,
    /// How far the centroid sits from directly below the source, relative to the spread. A cone is
    /// symmetric about its axis; an arc, a cast-off line and a drip trail are not.
    asymmetry: f32,
    /// Dominant direction of the marks, as a unit `(x, z)`.
    direction: [f32; 2],
    /// Summed wetted area, m².
    area: f32,
}

fn land_all(drops: &[Droplet], s: &BloodSettings) -> Vec<Stain> {
    drops
        .iter()
        .enumerate()
        .filter_map(|(i, d)| {
            let at = landing(SOURCE, d, s.gravity, 0.0)?;
            let impact = stain::impact_at_plane(d, SOURCE, 0.0, s);
            Some(Stain {
                at,
                radius: stain::stain_radius(d, impact.speed, s),
                seed: i as u32,
            })
        })
        .collect()
}

fn measure(stains: &[Stain]) -> Metrics {
    if stains.is_empty() {
        return Metrics::default();
    }
    let n = stains.len() as f32;
    let cx = stains.iter().map(|s| s.at[0]).sum::<f32>() / n;
    let cz = stains.iter().map(|s| s.at[2]).sum::<f32>() / n;
    let spread = stains
        .iter()
        .map(|s| {
            let (dx, dz) = (s.at[0] - cx, s.at[2] - cz);
            (dx * dx + dz * dz).sqrt()
        })
        .fold(0.0f32, f32::max);
    let off = (cx * cx + cz * cz).sqrt();
    let len = (cx * cx + cz * cz).sqrt().max(f32::MIN_POSITIVE);
    Metrics {
        stains: stains.len(),
        spread,
        asymmetry: if spread > 0.0 { off / spread } else { 0.0 },
        direction: [cx / len, cz / len],
        area: stains.iter().map(|s| s.radius * s.radius * core::f32::consts::PI).sum(),
    }
}

/// One sample of one class. Every class is driven from the same seed stream, so the sweep covers each
/// mechanism across its own inputs rather than sampling one and repeating it.
fn sample(class: PatternClass, i: u32, s: &BloodSettings) -> Metrics {
    let severity = 0.2 + 0.8 * hash_f32(i ^ 0xA1);
    let area = 0.002 + 0.05 * hash_f32(i ^ 0xB2);
    let nx = hash_f32(i ^ 0xC3) * 2.0 - 1.0;
    let nz = hash_f32(i ^ 0xD4) * 2.0 - 1.0;
    let len = (nx * nx + nz * nz + 0.25f32).sqrt();
    let w = Wound {
        at: SOURCE,
        normal: [nx / len, 0.5 / len, nz / len],
        area,
        severity,
        kind: WoundKind::Severance,
    };
    let b = Bleed::new(0, &w);
    let hz = 60u32;

    match class {
        PatternClass::Impact => measure(&land_all(&impact_spatter(&w, s), s)),
        PatternClass::ArterialSpurt => {
            // Sampled across the pressure decay, so the class covers a fresh spurt and a spent one.
            let tick = bevy_carnage::bloodstain::bleed::pulse_period(hz, s) * (i % 12)
                + bevy_carnage::bloodstain::bleed::pulse_phase(&b, hz, s);
            measure(&land_all(&arterial_arc(&w, &b, tick, hz, s), s))
        }
        PatternClass::CastOff => {
            let mut load = 0.05 + 0.2 * hash_f32(i ^ 0xE5);
            let step = 0.04 + 0.30 * hash_f32(i ^ 0xF6);
            let drops = cast_off(SOURCE, [SOURCE[0], SOURCE[1], SOURCE[2] + step], &mut load, hz, s);
            measure(&land_all(&drops, s))
        }
        PatternClass::Expirated => {
            let volume = 0.05 + 1.5 * hash_f32(i ^ 0x17);
            let impulse = 0.5 + 6.0 * hash_f32(i ^ 0x28);
            let (mist, _) = expirated(w.normal, volume, impulse, i, s);
            measure(&land_all(&mist, s))
        }
        PatternClass::DripTrail => {
            let mut load = 0.2 + 3.0 * hash_f32(i ^ 0x39);
            let speed = 0.3 + 4.0 * hash_f32(i ^ 0x4A);
            let to = [3.0 * nx / len, 0.0, 3.0 * nz / len];
            measure(&drip_trail([0.0, 0.0, 0.0], to, speed, &mut load, s))
        }
        PatternClass::Transfer => {
            let mut load = s.transfer_rate * (0.5 + 6.0 * hash_f32(i ^ 0x5B));
            let mut out = Vec::new();
            for k in 0..12u32 {
                let at = [0.1 * k as f32 * nx / len, 0.0, 0.1 * k as f32 * nz / len];
                match transfer(at, [nx, 0.0, nz], &mut load, s) {
                    Some(st) => out.push(st),
                    None => break,
                }
            }
            measure(&out)
        }
    }
}

/// **The sweep does not collapse and nothing dominates it.**
#[test]
fn the_expressive_range_stays_open() {
    let s = BloodSettings { spatter_speed_scale: 0.25, ..Default::default() };
    let mut grid = [[0usize; BUCKETS]; BUCKETS];
    let mut all = Vec::with_capacity(SAMPLES);

    for i in 0..SAMPLES {
        let class = PatternClass::ALL[i % PatternClass::ALL.len()];
        let m = sample(class, i as u32, &s);
        all.push((class, m));
        // **The two axes are how big the pattern is and how directional it is**, because those are
        // what "looks different" means to the eye reading the floor. Mark COUNT is deliberately not
        // an axis: it is dominated by a single dial per class (`arc_stains`, a load, a path length),
        // so histogramming it measures the dials rather than the shapes — and it is checked
        // separately below, per class, where a collapsed count is unambiguous.
        let spread_bucket =
            ((m.spread / 3.0).clamp(0.0, 0.999) * BUCKETS as f32) as usize % BUCKETS;
        let asym_bucket =
            ((m.asymmetry / 2.0).clamp(0.0, 0.999) * BUCKETS as f32) as usize % BUCKETS;
        grid[spread_bucket][asym_bucket] += 1;
    }

    let occupied = grid.iter().flatten().filter(|&&c| c > 0).count();
    let worst = grid.iter().flatten().copied().max().unwrap_or(0);

    let render: String = grid
        .iter()
        .enumerate()
        .map(|(c, row)| format!("  spread[{c}] x asymmetry {row:?}\n"))
        .collect();

    assert!(
        occupied >= 12,
        "the model occupies only {occupied} of {} buckets, which is a collapsed expressive range — \
         six mechanisms producing one shape.\n{render}",
        BUCKETS * BUCKETS
    );
    assert!(
        worst * 4 <= SAMPLES,
        "one bucket holds {worst} of {SAMPLES} samples ({:.0} %), over the 25 % ceiling — the model \
         has a favourite and everything else is a rounding error.\n{render}",
        worst as f32 / SAMPLES as f32 * 100.0
    );

    // And each class must be individually alive: a class that produced nothing at all would still
    // let the aggregate pass on the strength of the other five.
    for class in PatternClass::ALL {
        let mine: Vec<&Metrics> =
            all.iter().filter(|(c, _)| *c == class).map(|(_, m)| m).collect();
        let marks: usize = mine.iter().map(|m| m.stains).sum();
        assert!(marks > 0, "{class:?} produced no marks at all across {} samples", mine.len());
        let spreads = mine.iter().map(|m| m.spread).fold(0.0f32, f32::max)
            - mine.iter().map(|m| m.spread).fold(f32::INFINITY, f32::min);
        assert!(
            spreads > 1.0e-4,
            "{class:?} produced the same spread every time, so its inputs do nothing"
        );
        // **And the mark count must move too.** A class that places exactly N marks for every input
        // is a fixed dial wearing a mechanism's name — which is what `arterial_arc` was until its
        // count was tied to the pressure envelope.
        // Wetted area and dominant direction are read here rather than histogrammed: area is a
        // monotone function of the marks already counted, and a direction is only meaningful for the
        // classes that have one — so what they are checked for is sanity, not range.
        let area: f32 = mine.iter().map(|m| m.area).sum();
        assert!(
            area > 0.0 && area.is_finite(),
            "{class:?} wet no floor at all, or wet a non-finite amount of it: {area}"
        );
        for m in &mine {
            if m.stains == 0 {
                continue;
            }
            let len = (m.direction[0] * m.direction[0] + m.direction[1] * m.direction[1]).sqrt();
            assert!(
                (len - 1.0).abs() < 1.0e-3 || m.spread == 0.0,
                "{class:?} reported a dominant direction of length {len}, which is not a direction"
            );
        }
        let lo = mine.iter().map(|m| m.stains).min().unwrap_or(0);
        let hi = mine.iter().map(|m| m.stains).max().unwrap_or(0);
        assert!(
            hi > lo,
            "{class:?} left exactly {hi} marks on every one of {} samples — that is a dial, not a \
             mechanism",
            mine.len()
        );
    }
}

/// **The six classes are distinguishable from each other**, not merely varied within themselves. An
/// analyst reads a scene by telling classes apart, and so does a player.
#[test]
fn the_classes_do_not_look_like_each_other() {
    let s = BloodSettings { spatter_speed_scale: 0.25, ..Default::default() };
    let mean = |class: PatternClass| {
        let n = 60u32;
        let mut count = 0.0f32;
        let mut spread = 0.0f32;
        let mut asym = 0.0f32;
        for i in 0..n {
            let m = sample(class, i * 7 + 3, &s);
            count += m.stains as f32;
            spread += m.spread;
            asym += m.asymmetry;
        }
        [count / n as f32, spread / n as f32, asym / n as f32]
    };
    let profiles: Vec<(PatternClass, [f32; 3])> =
        PatternClass::ALL.iter().map(|c| (*c, mean(*c))).collect();

    for (i, (ca, a)) in profiles.iter().enumerate() {
        for (cb, b) in profiles.iter().skip(i + 1) {
            // Normalised so a difference in mark count is comparable to one in metres.
            let d = ((a[0] - b[0]) / 40.0).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs();
            assert!(
                d > 0.05,
                "{ca:?} and {cb:?} are indistinguishable: {a:?} vs {b:?}. Two classes that measure \
                 the same are one class with two names."
            );
        }
    }
}

/// **The variety bag never repeats early**, over a run long enough that a birthday problem would
/// have shown up thousands of times.
#[test]
fn the_variety_bag_holds_its_gap_over_ten_thousand_draws() {
    let n = 4u32;
    let avoid = 2u32;
    let seq: Vec<u32> = (0..10_000u32).map(|e| bag::pick(e, 0xC0FFEE, n, avoid)).collect();
    for (i, v) in seq.iter().enumerate() {
        for (k, w) in seq.iter().enumerate().skip(i + 1).take(avoid as usize) {
            assert_ne!(v, w, "variant {v} repeated at draws {i} and {k}, a gap of {}", k - i);
        }
    }
    // And it is not degenerate: all four variants actually appear.
    let mut seen = [false; 4];
    for v in seq.iter().take(64) {
        seen[*v as usize % 4] = true;
    }
    assert!(seen.iter().all(|b| *b), "the bag never dealt some variants: {seen:?}");
    let _ = patterns::PatternClass::Impact;
}
