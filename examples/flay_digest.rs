//! **The crate's headline claim, run as a program.** No window, no GPU, no `App`.
//!
//! A damage mask that lives on the GPU cannot be hashed — that is why every other texture-space
//! destruction system is a picture rather than a state. This one takes thirty hits at one spot on a
//! limb, prints how many texels of each tissue are showing after every one, names the hit on which
//! bone came through, and prints the digest. Run it twice: the number is the same.
//!
//! Run it over ssh if you like. That is rather the point.
//!
//! ```sh
//! cargo run -p bevy_flaymap --example flay_digest
//! ```

use bevy::asset::Assets;
use bevy::image::Image;
use bevy::math::Vec2;
use bevy_carnage::flaymap::{FlayCanvas, FlaySettings, Layer, Layers, Region};

/// Edge length of the canvas, in texels. Small on purpose: a terminal example should cost nothing,
/// and the depth buffer is the only thing being demonstrated.
const SIZE: u32 = 64;
/// Hits the script lands, all at the same spot.
const HITS: u32 = 30;
/// Where they land, in the canvas's own UVs.
const SPOT: Vec2 = Vec2::new(0.5, 0.5);
/// Radius of each hit, in UV units — a tenth of the atlas, so the crater has a rim to read.
const RADIUS: f32 = 0.10;

fn main() {
    let settings = FlaySettings::default();
    let region = Region::Limb;
    let layers = Layers::for_region(region);
    let starts = layers.starts_mm();

    println!("bevy_flaymap — the wound is CPU state, so it has a number under it.\n");
    println!(
        "  region {region:?}: skin 0-{:.1} mm, fat -{:.1}, muscle -{:.1}, cortex -{:.1}, then marrow",
        starts[1], starts[2], starts[3], starts[4]
    );
    println!("  canvas {SIZE}x{SIZE} texels, {HITS} hits at ({}, {})\n", SPOT.x, SPOT.y);
    println!("  hit   depth mm   removed mm       skin    fat  muscle  cortex  marrow   bone?");

    let (digest, bone_on) = run(&settings, region, layers, true);

    match bone_on {
        Some(hit) => println!("\n  bone came through on hit {hit}, and the handoff fired once."),
        None => println!("\n  bone was never reached — thirty hits did not get through the muscle."),
    }
    println!("  digest 0x{digest:016x}");

    // The same script again, in a fresh arena with a fresh canvas: the claim, checked in the program
    // that makes it rather than only in the test suite.
    let (again, _) = run(&settings, region, layers, false);
    if again == digest {
        println!("  a second run of the same script agrees to the bit.");
    } else {
        println!("  a second run DISAGREED (0x{again:016x}) — something here is reading a clock.");
    }

    // And one hit moved by a single texel must be visible in the fold: equality alone would pass on a
    // function that returned a constant.
    let nudged = nudge(&settings, region, layers);
    if nudged == digest {
        println!("  a one-texel move gave the SAME digest, which is wrong.");
    } else {
        println!("  a one-texel move gives 0x{nudged:016x} — the digest sees the wound, not a summary.");
    }
}

/// Peel the script, optionally printing a row per hit, and return `(digest, hit bone came through on)`.
fn run(
    settings: &FlaySettings,
    region: Region,
    layers: Layers,
    print: bool,
) -> (u64, Option<u32>) {
    // A scratch arena: this example never renders, so the two images it makes are simply never read.
    let mut images = Assets::<Image>::default();
    let mut canvas = FlayCanvas::new(&mut images, SIZE, region, layers, [0.78, 0.66, 0.60], 0.55);
    let mut bone_on = None;
    let centre = SIZE / 2;

    for hit in 0..HITS {
        // Growing depth: a first graze takes half a millimetre, a thirtieth hit takes over three, so
        // the crater deepens the way repeated fire does rather than linearly.
        let depth_mm = 0.5 + 0.1 * hit as f32;
        let handoff = canvas.paint_uv(SPOT, RADIUS, depth_mm, hit);
        if handoff.bone_reached {
            bone_on = Some(hit);
        }
        if print {
            let removed = canvas.depth_at(centre, centre).unwrap_or(0.0);
            print!(
                "  {hit:>3}   {depth_mm:>8.2}   {removed:>10.2}   {:>6} {:>6} {:>7} {:>7} {:>7}",
                canvas.exposed_area(Layer::Skin),
                canvas.exposed_area(Layer::Fat),
                canvas.exposed_area(Layer::Muscle),
                canvas.exposed_area(Layer::Cortex),
                canvas.exposed_area(Layer::Marrow),
            );
            // The handoff is once per canvas, so at most one row in this table is marked.
            if handoff.bone_reached {
                println!("   BONE  <- handoff, at {:?}", handoff.first_bone_uv);
            } else {
                println!("   {:?}", handoff.deepest_layer);
            }
        }
    }

    // Shading is what a renderer would sample, and it is deliberately outside the digest: it is a
    // pure function of the depth buffer, so folding it would hash the same information twice.
    canvas.shade(settings);
    (canvas.digest(), bone_on)
}

/// The same script with the spot moved by exactly one texel.
fn nudge(settings: &FlaySettings, region: Region, layers: Layers) -> u64 {
    let mut images = Assets::<Image>::default();
    let mut canvas = FlayCanvas::new(&mut images, SIZE, region, layers, [0.78, 0.66, 0.60], 0.55);
    let spot = Vec2::new(SPOT.x + 1.0 / SIZE as f32, SPOT.y);
    for hit in 0..HITS {
        canvas.paint_uv(spot, RADIUS, 0.5 + 0.1 * hit as f32, hit);
    }
    canvas.shade(settings);
    canvas.digest()
}
