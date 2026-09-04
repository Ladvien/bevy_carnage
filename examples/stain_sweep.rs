//! **Stain silhouettes across the impact angle, in ASCII.**
//!
//! Terminal only — no window, no GPU, no asset — so it runs anywhere, including over ssh on a box
//! with no display. What it shows is the one relation the whole of bloodstain-pattern analysis rests
//! on: `minor / major = sin θ`. At 90° the stain is a disc; at 15° it is a lance with its spines
//! thrown downrange.
//!
//! ```sh
//! cargo run -p bloodstain --example stain_sweep
//! ```

use bevy_carnage::bloodstain::stain::{Impact, rasterise, stain_shape};
use bevy_carnage::bloodstain::{BloodSettings, weber};

/// Texels per side of the rasterised mask. 33 is odd, so the stain has an exact centre row.
const PX: u32 = 33;

/// Coverage ramp, darkest last. Read a column of these as a density.
const RAMP: [char; 5] = [' ', '.', ':', '#', '@'];

fn main() {
    let s = BloodSettings::default();
    println!(
        "bloodstain: stain silhouette vs impact angle\n\
         \n\
         droplet 4.0 mm at 6.0 m/s, substrate roughness {:.2}\n\
         aspect is sin θ (Hulse-Smith, doi:10.1520/jfs2003224); spines are\n\
         0.76 · We^0.5 · sin³θ (Knock & Davison, doi:10.1111/j.1556-4029.2007.00505.x)\n",
        s.substrate_roughness
    );

    let mut mask = vec![0u8; (PX * PX) as usize];
    for deg in [90.0f32, 75.0, 60.0, 45.0, 30.0, 15.0] {
        let impact = Impact {
            speed: 6.0,
            diameter: 0.004,
            angle_rad: deg.to_radians(),
            roughness: s.substrate_roughness,
            // Travel along +x, so the long axis runs left-to-right in the print below.
            travel: [1.0, 0.0],
        };
        let shape = stain_shape(&impact, &s, 0x5EED);
        if !rasterise(&shape, PX, &mut mask) {
            eprintln!("rasterise refused a {PX}x{PX} buffer — that is a bug, not a setting");
            return;
        }

        println!(
            "── {deg:>4.0}°   major {:.1} mm   minor {:.1} mm   aspect {:.3}   spines {:>2}   \
             satellites {}   We {:.0}",
            shape.major * 1000.0,
            shape.minor * 1000.0,
            shape.minor / shape.major,
            shape.spines,
            shape.satellites,
            weber(impact.diameter, impact.speed),
        );

        for row in 0..PX as usize {
            let line: String = (0..PX as usize)
                .map(|col| {
                    let v = mask[row * PX as usize + col];
                    // Five levels, so a soft rim reads as a gradient rather than a hard edge.
                    RAMP[(v as usize * (RAMP.len() - 1) + 127) / 255]
                })
                // Two characters per texel: a terminal cell is about twice as tall as it is wide, so
                // one character per texel would squash the aspect ratio this example exists to show.
                .flat_map(|c| [c, c])
                .collect();
            if line.trim().is_empty() {
                continue;
            }
            println!("  {line}");
        }
        println!();
    }

    println!(
        "The aspect ratio is the measurement: `origin::area_of_origin` reads it back to find\n\
         where the blood came from, and `tests/origin.rs` proves it recovers the wound."
    );
}
