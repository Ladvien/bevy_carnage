//! **Terminal only.** Prints the gape curve at three skin tensions, in both Langer directions, then
//! tears a grid and prints the digest a golden freezes.
//!
//! ```sh
//! cargo run --example laceration_curve
//! ```
//!
//! Run it twice: the digest line is identical, which is the property the whole crate rests on. No
//! window, no GPU, no clock, no filesystem — the numbers are a pure function of the constants below.

use bevy::math::Vec3;
use bevy_carnage::laceration::{
    ALONG_LANGER_FACTOR, Gape, Layers, Region, Scale, TearShape, Tension, anisotropy, digest, gape,
    skin_patch, tear,
};

/// The Langer lines run along `x` in this example's mesh space.
const LANGER: [f32; 3] = [1.0, 0.0, 0.0];
/// A cut straight across them — the one that gapes.
const ACROSS: [f32; 3] = [0.0, 0.0, 1.0];
/// A cut along them — the one that barely parts.
const ALONG: [f32; 3] = [1.0, 0.0, 0.0];

/// The grid the frozen digest is taken over: 21 × 21 vertices across one metre, cut with a gape
/// 1.2 cells wide so whole triangles fall inside it. **Exactly the case `tests/tear.rs` freezes** —
/// the half-width is spelled `CELL * 1.2` rather than `0.06` because the two are different `f32`
/// bit patterns, and a digest is a digest of bits.
const CELLS: u32 = 20;
const SIZE: f32 = 1.0;
const CELL: f32 = SIZE / CELLS as f32;
const HALF: f32 = CELL * 1.2;

fn main() {
    let g = Gape { width_max: 0.020, open_ticks: 60 };
    println!(
        "gape: width_max {:.0} mm, 95 % at {} ticks (this crate's own number — no paper gives a rate)",
        g.width_max * 1000.0,
        g.open_ticks
    );
    println!(
        "Langer anisotropy: along {:.3}, across {:.3} — the 63.8/112.5 MPa stiffness ratio of Ni Annaidh et al. 2012,",
        anisotropy(ALONG, &Tension { skin: 1.0, langer: Some(LANGER) }),
        anisotropy(ACROSS, &Tension { skin: 1.0, langer: Some(LANGER) })
    );
    println!("                   doi:10.1016/j.jmbbm.2011.08.016, used as a gape proxy (ALONG_LANGER_FACTOR = {ALONG_LANGER_FACTOR:.4})");
    println!();

    println!("      tick   |  across the lines, by skin tension  |  along the lines, by skin tension");
    println!("             |    0.3       0.6       1.0          |    0.3       0.6       1.0");
    println!("  -----------+------------------------------------+-----------------------------------");
    for tick in [0u32, 5, 10, 20, 30, 45, 60, 90, 120, 180] {
        let mm = |dir: [f32; 3], skin: f32| gape(tick, &g, &Tension { skin, langer: Some(LANGER) }, dir) * 1000.0;
        println!(
            "  {tick:>9}  | {:>6.2}mm {:>6.2}mm {:>6.2}mm       | {:>6.2}mm {:>6.2}mm {:>6.2}mm",
            mm(ACROSS, 0.3),
            mm(ACROSS, 0.6),
            mm(ACROSS, 1.0),
            mm(ALONG, 0.3),
            mm(ALONG, 0.6),
            mm(ALONG, 1.0),
        );
    }
    println!();

    // The tear itself, on a patch of skin this crate ships so the digest is over geometry it owns.
    // A metre-wide grid is a coarse stand-in for a limb, so the widths here are far larger than the
    // 20 mm of the table above; what matters is that the bits are the frozen ones.
    let patch = skin_patch(CELLS, SIZE);
    let layers = Layers::for_region(Region::Limb);
    let path = [Vec3::new(-0.4, 0.0, 0.0), Vec3::new(0.4, 0.0, 0.0)];
    let shape = TearShape { half_width: HALF, influence: 0.15, bed_depth_mm: 6.0 };
    let Some(torn) = tear(
        &patch,
        &path,
        Vec3::Y,
        &shape,
        Region::Limb,
        &layers,
        &Scale::default(),
    ) else {
        eprintln!("the tear was refused — this example's inputs are wrong, which is a bug");
        return;
    };
    let (layer, into) = layers.at(shape.bed_depth_mm);
    println!(
        "tear: half_width {:.0} mm, influence {:.0} mm, bed {:.0} mm deep -> {layer:?} at {:.0} % into the band",
        shape.half_width * 1000.0,
        shape.influence * 1000.0,
        shape.bed_depth_mm,
        into * 100.0
    );
    println!(
        "      {} faces removed, {} vertices displaced, limb span {:.1} mm",
        torn.removed_faces,
        torn.displaced_vertices,
        layers.span_mm()
    );
    println!("skin digest {:016x}", digest(&torn.skin));
    println!("bed  digest {:016x}", digest(&torn.bed));
}
