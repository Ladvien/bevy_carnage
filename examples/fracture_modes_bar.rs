//! **Terminal only.** The necked bar's modes, then a blow at each end: which faces give, and in
//! what order as the blow grows.
//!
//! ```sh
//! cargo run --example fracture_modes_bar
//! ```
//!
//! Run it twice: the digest matches, which is the property the whole crate rests on.

use bevy_carnage::fracture_modes::{CellGraph, Impact, ModeSet, ModeSettings};

fn main() {
    let graph = CellGraph::bar(10, 4, 0.05);
    let settings = ModeSettings { k: 3, ..Default::default() };
    let set = match ModeSet::bake(&graph, &settings) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bake refused: {e:?}");
            return;
        }
    };
    println!("bar of {} cells, neck after cell 4 at 5 % area; {} modes", graph.len(), set.modes.len());
    for (i, m) in set.modes.iter().enumerate() {
        let phi: Vec<String> = m.phi.iter().map(|v| format!("{v:+.2}")).collect();
        let fault = set.strongest_fault(i).map(|(f, j)| format!("face {f} jumps {j:.3}")).unwrap_or_default();
        println!("  mode {i}  E_D = {:.4}  [{}]  {fault}", m.energy, phi.join(" "));
    }

    for cell in [0usize, 9] {
        println!("blow at cell {cell}, impulse rising:");
        let mut last = 1;
        for i in 0..400 {
            let magnitude = 1.0e-4 * 1.05f32.powi(i);
            let p = set.partition(&Impact { cell, magnitude });
            if p.fragment_count() != last {
                last = p.fragment_count();
                println!("  impulse {magnitude:>9.5}: {} piece(s), broken faces {:?}", last, p.broken);
            }
            if last >= 4 {
                break;
            }
        }
    }

    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for m in &set.modes {
        for v in m.phi.iter().chain(m.impact_row.iter()).chain(std::iter::once(&m.energy)) {
            for byte in v.to_bits().to_le_bytes() {
                h ^= byte as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    println!("digest {h:016x}");
}
