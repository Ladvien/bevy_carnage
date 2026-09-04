//! **Terminal only.** Bakes the three region strips and prints where every band landed, the
//! thickness it stands for, and the digest a golden freezes.
//!
//! ```sh
//! cargo run --example cross_section_strip
//! cargo run --example cross_section_strip -- --ppm strips.ppm   # also dump the three strips stacked
//! ```
//!
//! Run it twice: the digests match, which is the property the whole crate rests on.

use bevy_carnage::cross_section::{Layers, Region, strip};

fn main() {
    let mut ppm: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--ppm" {
            ppm = args.next();
        }
    }
    let mut stacked: Vec<u8> = Vec::new();
    let mut stacked_h = 0u32;
    let mut stacked_w = 0u32;
    for region in Region::ALL {
        let layers = Layers::for_region(region);
        let s = strip(&layers, 512, 512, 50.0, 0xC0FF_EE00);
        let ppm_per_mm = s.px_per_mm(&layers);
        println!("{region:?}: {}x{} texels, {ppm_per_mm:.1} px/mm, span {:.1} mm", s.width, s.height, layers.span_mm());
        for band in &s.bands {
            let drawn = (band.x1 - band.x0) as f32 / ppm_per_mm;
            println!(
                "  {:<7} columns {:>3}..{:<3}  {:>5.1} mm drawn  {:>5.1} mm measured",
                format!("{:?}", band.layer),
                band.x0,
                band.x1,
                drawn,
                layers.thickness_mm(band.layer)
            );
        }
        println!("  digest {:016x}", s.digest());
        stacked_w = s.width;
        stacked_h += s.height;
        for px in s.albedo.chunks_exact(4) {
            stacked.extend_from_slice(&px[..3]);
        }
    }
    if let Some(path) = ppm {
        let mut bytes = format!("P6\n{stacked_w} {stacked_h}\n255\n").into_bytes();
        bytes.extend_from_slice(&stacked);
        match std::fs::write(&path, bytes) {
            Ok(()) => println!("wrote {path}"),
            Err(e) => eprintln!("could not write {path}: {e}"),
        }
    }
}
