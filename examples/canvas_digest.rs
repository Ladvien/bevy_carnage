//! **The crate's headline claim, run as a program.** No window, no GPU, no `App`.
//!
//! A wetmap that lives on the GPU cannot be hashed — that is why every other texture-space blood
//! system is a picture rather than a state. This one paints the same scripted sequence twice and prints
//! the same `u64`, then moves **one hit by one texel** and prints a different one. Both halves matter:
//! equality alone would pass on a function that returned a constant.
//!
//! Run it over ssh if you like. That is rather the point.
//!
//! ```sh
//! cargo run -p bevy_wetmap --example canvas_digest
//! ```

use bevy::asset::Assets;
use bevy::image::Image;
use bevy::math::Vec2;
use bevy_carnage::wetmap::{StainShape, WetCanvas, WetSettings};

/// Edge length of the canvas, in texels. The crate's shipped default.
const SIZE: u32 = 128;
/// Ticks the script runs for. 90 at 60 Hz is a second and a half of a run developing.
const TICKS: u32 = 90;

/// Where the shots land, and on which tick: `(u, v, tick)`.
///
/// Three hits rather than one, so the drip, spread, age and shade passes have all run against blood of
/// two different ages by the end — a script with a single stamp would not notice a pass that only
/// worked on fresh paint.
const HITS: [(f32, f32, u32); 3] = [(0.34, 0.18, 0), (0.58, 0.30, 12), (0.36, 0.44, 30)];

fn main() {
    let settings = WetSettings::default();

    println!("bevy_wetmap — the canvas is CPU state, so it has a number under it.\n");
    println!(
        "  canvas {SIZE}x{SIZE}, {TICKS} ticks, gravity +v, \
         drip {:.2} / spread {:.2} / dry {} ticks / absorb {:.2}\n",
        settings.drip_rate, settings.spread_rate, settings.dry_ticks, settings.absorbency
    );

    // Two runs of the identical script. Separate scratch arenas, separate canvases, separate handles.
    let (first, area) = run(&settings, 0);
    let (second, _) = run(&settings, 0);

    println!("  run 1                     digest 0x{first:016x}");
    println!("  run 2 (identical script)  digest 0x{second:016x}");
    if first == second {
        println!("  -> EQUAL. The wetmap is reproducible.\n");
    } else {
        println!("  -> DIFFERENT. Something in this crate is reading something it should not.\n");
    }

    // One hit moved by a single texel: 1/128 of a UV unit on the first shot only.
    let (nudged, nudged_area) = run(&settings, 1);
    println!("  run 3 (first hit moved by ONE texel)");
    println!("                            digest 0x{nudged:016x}");
    if nudged == first {
        println!("  -> EQUAL, which is wrong: a one-texel move must be visible in the fold.\n");
    } else {
        println!("  -> DIFFERENT, as it must be. The digest sees the blood, not a summary of it.\n");
    }

    println!("  wetted area, run 1: {:.6} m^2", area);
    println!("  wetted area, run 3: {:.6} m^2", nudged_area);
    println!(
        "\n  (Area is quoted under bevy_carnage::wetmap::UV_SPAN_M = {:.1} m per UV unit; a texel is \
         {:.3} mm across.)",
        bevy_carnage::wetmap::UV_SPAN_M,
        bevy_carnage::wetmap::UV_SPAN_M / SIZE as f32 * 1000.0
    );
}

/// Paint and tick the script, returning the digest and the wetted area at the end.
///
/// `nudge_texels` shifts the **first** hit only, which is how run 3 differs from runs 1 and 2.
fn run(settings: &WetSettings, nudge_texels: i32) -> (u64, f32) {
    // A scratch arena: this example never renders, so nothing ever reads these images. `flush` is
    // still called, because "the upload is the only thing that touches `Assets<Image>`" is a property
    // worth exercising even when nobody is looking at the result.
    let mut images = Assets::<Image>::default();
    let mut canvas = WetCanvas::new(&mut images, SIZE, [0.78, 0.66, 0.60], 0.55);

    let nudge = nudge_texels as f32 / SIZE as f32;
    let mut next = 0;
    for tick in 0..TICKS {
        while next < HITS.len() && HITS[next].2 == tick {
            let (u, v, _) = HITS[next];
            let u = if next == 0 { u + nudge } else { u };
            canvas.paint_uv(Vec2::new(u, v), &shot(next as u32), tick);
            next += 1;
        }
        // Gravity is `+v`: down the texture. Which way that is on an actor is the caller's to know.
        canvas.tick(tick, Vec2::new(0.0, 1.0), settings);
        canvas.flush(&mut images);
    }
    (canvas.digest(), canvas.wetted_area())
}

/// One shot's stain silhouette.
///
/// Built by hand rather than through `bevy_carnage::bloodstain::stain::stain_shape` so this example has exactly one
/// subject: the wetmap. The morphology model has its own examples in `bloodstain`.
fn shot(index: u32) -> StainShape {
    StainShape {
        // ~7 cm long, which is nine texels at this canvas size — big enough to run.
        major: 0.070,
        minor: 0.045,
        spines: 6,
        satellites: 2,
        direction: [0.0, 1.0],
        seed: 0x51ED_0000 ^ index,
    }
}
