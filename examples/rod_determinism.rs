//! **The proof, in a terminal.**
//!
//! Spill six strands, tether them, step 600 fixed ticks, print the digest. Throw the whole thing away
//! and do it again. The two lines must match — that is the crate's product, and this is the cheapest
//! possible way to see it fail.
//!
//! There is no window, no `App`, no GPU and no `DefaultPlugins` here on purpose: [`bevy_carnage::viscera::step`]
//! is a plain function over plain data, so a build machine that cannot open a window can still run the
//! check. `cargo run -p bevy_viscera --example rod_determinism`.

use bevy::math::Vec3;
use bevy_carnage::viscera::{
    spill, step, Mesentery, Strand, ViscSettings, DEFAULT_TEAR_STRAIN, FIXED_HZ, SPILL_SEGMENTS,
};

/// Ten seconds at the crate's fixed 60 Hz.
const TICKS: u32 = 600;
const WOUND: Vec3 = Vec3::new(0.1, 1.4, -0.2);
const EXIT: Vec3 = Vec3::new(0.35, 0.2, 1.0);
const SEED: u32 = 0x5EED_1234;

/// One complete run, from an empty world to a folded digest.
fn run(settings: &ViscSettings) -> Run {
    let mut strands = spill(WOUND, EXIT, 6, SEED, settings);

    // Tether every strand back to where it left the body, at two different densities. A mesenteric
    // link supports roughly nine nodes of hanging weight before its strain passes `tear_strain`, so
    // the strands anchored every fourth node hold and the ones anchored every twelfth do not — and
    // the digest below covers both the intact and the torn path.
    let mut mesentery: Vec<Mesentery> = strands
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let stride = if i % 2 == 0 { 4 } else { 12 };
            let anchors: Vec<(u32, Vec3)> = s
                .nodes()
                .iter()
                .enumerate()
                .filter(|(n, _)| n % stride == 0)
                .map(|(n, p)| (n as u32, *p))
                .collect();
            let torn = vec![false; anchors.len()];
            Mesentery { anchors, tear_strain: DEFAULT_TEAR_STRAIN, torn }
        })
        .collect();

    for _ in 0..TICKS {
        step(&mut strands, &mut mesentery, settings);
    }

    // Fold the per-strand digests into one number, in slice order, with the same FNV-1a mixing the
    // strands use internally — printed as one line so a diff is a diff.
    let mut folded: u64 = 0xcbf2_9ce4_8422_2325;
    for strand in &strands {
        for byte in strand.digest().to_le_bytes() {
            folded ^= u64::from(byte);
            folded = folded.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    let count = |take_dense: bool| -> (usize, usize) {
        mesentery
            .iter()
            .enumerate()
            .filter(|(i, _)| (i % 2 == 0) == take_dense)
            .fold((0, 0), |(torn, links), (_, m)| {
                (torn + m.torn.iter().filter(|t| **t).count(), links + m.torn.len())
            })
    };
    let lowest = strands.iter().flat_map(|s| s.nodes()).map(|p| p.y).fold(f32::MAX, f32::min);

    Run {
        folded,
        per_strand: strands.iter().map(Strand::digest).collect(),
        dense: count(true),
        sparse: count(false),
        lowest,
    }
}

struct Run {
    folded: u64,
    per_strand: Vec<u64>,
    /// `(torn, links)` for the strands tethered every fourth node.
    dense: (usize, usize),
    /// `(torn, links)` for the strands tethered every twelfth node.
    sparse: (usize, usize),
    lowest: f32,
}

fn main() {
    let settings = ViscSettings::default();

    println!("bevy_viscera — deterministic rod solver");
    println!(
        "  {} substeps x {} iterations at {FIXED_HZ} Hz, {TICKS} ticks, 6 strands of {SPILL_SEGMENTS} segments",
        settings.substeps, settings.iterations
    );
    println!(
        "  gravity {}  damping {}  stretch {:e}  bend {:e}  floor {}",
        settings.gravity,
        settings.damping,
        settings.compliance_stretch,
        settings.compliance_bend,
        settings.floor_y
    );
    println!();

    let first = run(&settings);
    let second = run(&settings);

    println!("  run 1 digest : {:016x}", first.folded);
    println!("  run 2 digest : {:016x}", second.folded);
    println!(
        "  mesentery    : {}/{} links torn where the tether is every 4th node (it holds)",
        first.dense.0, first.dense.1
    );
    println!(
        "                 {}/{} links torn where it is every 12th (it parts)",
        first.sparse.0, first.sparse.1
    );
    println!("  lowest node  : y = {:.4}", first.lowest);
    println!();

    let matched = first.folded == second.folded && first.per_strand == second.per_strand;
    if matched {
        println!("  MATCH — the solver is reproducible.");
    } else {
        println!("  MISMATCH — the solver is NOT reproducible.");
        for (i, (a, b)) in first.per_strand.iter().zip(second.per_strand.iter()).enumerate() {
            let mark = if a == b { ' ' } else { '!' };
            println!("   {mark} strand {i}: {a:016x} vs {b:016x}");
        }
    }
}
