//! **What a spilled strand looks like: a texture for the tube [`crate::viscera::tube_mesh`] sweeps.**
//!
//! Bowel is not the colour of the blood it lies in. Its outer coat is a pale, salmon-pink serosa,
//! glossy and a little translucent, with the mesentery's yellow fat along the side the strand was
//! tethered by and the straight vessels (vasa recta) branching off that side and round the wall, a
//! few to every centimetre or two. A strand drawn the red of the blood pool reads as red tubing in it,
//! which is what every spill in this crate looked like until 0.5.41.
//!
//! The image is laid out for the tube's own `UV_0`: `u` round the wall, with the mesenteric side at
//! `u = 0`, and `v` along the strand from the wound (`v = 0`) to its free end, over a strand of
//! [`crate::viscera::SPILL_SEGMENTS`] × [`crate::viscera::SPILL_REST_LEN`]. It repeats round the wall
//! and is sampled that way. The end that left the wound is bloodied, and patches along the rest are
//! smeared.
//!
//! Cosmetic: a pure function of nothing, built once by the caller and handed to whatever material it
//! likes, and outside every digest. The crate still never spawns and never writes an asset arena.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::viscera::hash::{hash_pair, hash_unit};

/// Texels round the wall. A 20 mm tube is 126 mm round, so 2 mm a texel.
const AROUND: u32 = 64;
/// Texels along the strand. 840 mm over 384, so 2.2 mm a texel.
const ALONG: u32 = 384;

/// The serosa, sRGB: a salmon pink. This crate's own number, chosen by eye against the blood colour
/// the wetmap draws; a paler `0.84, 0.60, 0.56` read as pink candy in the pool.
const SEROSA: [f32; 3] = [0.76, 0.48, 0.45];
/// The mesentery's fat along the tethered side, sRGB.
const FAT: [f32; 3] = [0.84, 0.68, 0.52];
/// A vessel in the wall, sRGB: venous, dark and a little purple.
const VESSEL: [f32; 3] = [0.50, 0.13, 0.20];
/// Blood smeared on the wall, sRGB, as the wetmap draws a thick venous film.
const BLOOD: [f32; 3] = [0.40, 0.05, 0.07];

/// Straight vessels along the strand, each leaving the mesenteric side for both sides of the wall.
const VESSELS: u32 = 56;

/// **How rough a spilled strand's wall is**: `0.3`, wet serosa — glossy, but a curved tube's highlight
/// stays a streak rather than a mirror. This crate's own number.
pub const GUT_ROUGHNESS: f32 = 0.3;

/// **The texture a spilled strand wears**: `64 × 384` `Rgba8UnormSrgb`, repeating round the wall,
/// for the tube's `UV_0`. Hand it to a material's base colour with a white factor.
///
/// The same bytes every call, so a caller builds it once and shares the handle.
pub fn gut_image() -> Image {
    let mut data = Vec::with_capacity((AROUND * ALONG * 4) as usize);
    for y in 0..ALONG {
        let v = (y as f32 + 0.5) / ALONG as f32;
        for x in 0..AROUND {
            let u = (x as f32 + 0.5) / AROUND as f32;
            // Distance round the wall from the mesenteric side, `0..=0.5`.
            let side = (u - u.round()).abs();
            let mut c = SEROSA;
            // A soft mottle, so the wall is not a flat fill.
            let shade = 0.94 + 0.10 * noise(x, y, 8, 12, 0x6E7A);
            c = c.map(|ch| ch * shade);
            // The mesentery's fat, a thin band along the side the strand hung from.
            c = mix(c, FAT, 0.8 * (1.0 - smooth(0.03, 0.06, side)));
            // The straight vessels, round the wall from that side, each its own length.
            let mut vessel = 0.0f32;
            let k0 = (v * VESSELS as f32).floor() as i64;
            for k in k0 - 1..=k0 + 1 {
                if k < 0 || k >= VESSELS as i64 {
                    continue;
                }
                let h = |salt: u32| hash_unit(hash_pair(0x7A53_11D4 ^ salt, k as u32));
                let centre = (k as f32 + 0.5 + 0.6 * (h(1) - 0.5)) / VESSELS as f32;
                let wiggle = 0.003 * (core::f32::consts::TAU * (3.0 * u + h(2))).sin();
                let reach = 0.18 + 0.22 * h(3);
                let d = (v - centre - wiggle).abs() * ALONG as f32;
                let width = (1.3 * (1.0 - 0.6 * side / reach)).max(0.4);
                let line = 1.0 - smooth(0.5 * width, width, d);
                vessel = vessel.max(line * (1.0 - smooth(0.6 * reach, reach, side)) * smooth(0.04, 0.16, side));
            }
            // Faint: drawn at full strength, every vessel's root lined up along the fat and read as a
            // row of stitches.
            c = mix(c, VESSEL, 0.35 * vessel);
            // Blood: a wash over all of it, heavy at the end that left the wound, and smeared along
            // the strand in patches four times as long as they are wide — a strand drags through the
            // pool, it is not spotted.
            let smear = smooth(0.48, 0.78, noise(x, y, 16, 64, 0xB100));
            let wound_end = 1.0 - smooth(0.04, 0.22, v);
            c = mix(c, BLOOD, (0.15 + 0.75 * smear).max(0.9 * wound_end));
            data.extend(c.map(byte));
            data.push(255);
        }
    }
    let mut image = Image::new(
        Extent3d { width: AROUND, height: ALONG, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// Value noise in `[0, 1)` at texel `(x, y)`, on a lattice `cu` texels round the wall and `cv` along
/// it, periodic round the wall so the seam at `u = 0` does not show. `cu` divides the texels round.
fn noise(x: u32, y: u32, cu: u32, cv: u32, salt: u32) -> f32 {
    let period = AROUND / cu;
    let (gx, gy) = (x / cu, y / cv);
    let (fx, fy) = ((x % cu) as f32 / cu as f32, (y % cv) as f32 / cv as f32);
    let corner = |i: u32, j: u32| hash_unit(hash_pair(salt ^ (i % period).wrapping_mul(0x9E37_79B9), j));
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = corner(gx, gy) + (corner(gx + 1, gy) - corner(gx, gy)) * sx;
    let bottom = corner(gx, gy + 1) + (corner(gx + 1, gy + 1) - corner(gx, gy + 1)) * sx;
    top + (bottom - top) * sy
}

/// The cubic smoothstep from `a` to `b`.
fn smooth(a: f32, b: f32, t: f32) -> f32 {
    let t = ((t - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `a` toward `b` by `t`.
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// A `[0, 1]` channel as a byte.
fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The texture wraps round the tube without a seam.** The wall's first and last columns meet at
    /// `u = 0`, and the step between them must be no larger than the steps between neighbouring
    /// columns anywhere else; the mottle and the smears are periodic round the wall for exactly that.
    #[test]
    fn the_gut_texture_wraps_round_the_wall_without_a_seam() {
        let image = gut_image();
        let px = image.data.unwrap_or_default();
        let w = AROUND as usize;
        assert_eq!(px.len(), w * ALONG as usize * 4);
        let step = |row: usize, a: usize, b: usize| -> f32 {
            let (i, j) = ((row * w + a) * 4, (row * w + b) * 4);
            (0..3).map(|c| (px[i + c] as f32 - px[j + c] as f32).abs()).sum()
        };
        let rows = ALONG as usize;
        let seam = (0..rows).map(|r| step(r, w - 1, 0)).sum::<f32>() / rows as f32;
        let inner = (0..rows).map(|r| (1..w).map(|x| step(r, x - 1, x)).sum::<f32>() / (w - 1) as f32).sum::<f32>() / rows as f32;
        assert!(seam <= 1.5 * inner, "the seam steps {seam} on average where neighbours step {inner}");
    }
}
