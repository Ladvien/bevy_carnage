//! **Every dial a flaymap has, in one struct, with the arithmetic behind each value.**
//!
//! # No clock, no rate
//!
//! Nothing here is per second and nothing here is per tick. A flaymap does not evolve on its own —
//! tissue that has been peeled off stays off — so the only schedule in the crate is the upload
//! budget, and the only time-shaped number is how many canvases may reach the GPU in one frame.
//! That is why this struct has four fields where `crate::wetmap::WetSettings` has eight.
//!
//! # The two that decide what the wound looks like
//!
//! [`FlaySettings::tile_mm`] and [`FlaySettings::seed`] are handed straight to
//! `crate::cross_section::texel_at`, so a flayed patch and a cut face on the same body are shaded by
//! **one** tissue model at **one** physical scale rather than by two that drift.

use bevy::prelude::Resource;
use crate::cross_section::Scale;

/// **The flaymap dials.** How many canvases may upload in a frame, and the three numbers the tissue
/// shading is a function of.
///
/// Authored once per game and then left alone. Nothing here decides *when* a canvas is painted or
/// shaded — the caller owns that, because the caller owns the hit.
#[derive(Resource, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FlaySettings {
    /// Canvases the plugin may upload in one frame. `4`.
    ///
    /// A 128×128 `Rgba8UnormSrgb` canvas is 64 KB, and this crate owns **two** images per canvas
    /// (albedo and metallic-roughness), so four canvases is 512 KB of `Assets<Image>` writes per
    /// frame. The same budget `bevy_wetmap` ships, for the same reason and at the same cost: an
    /// actor wearing both crates pays it twice, which is the arithmetic behind keeping canvases
    /// small.
    pub max_canvas_updates_per_tick: u32,
    /// The along-axis period of the tissue noise, millimetres. `20.0`.
    ///
    /// Forwarded to `crate::cross_section::texel_at` as its `tile_mm`, where it is the period the fat
    /// lobules, the muscle fascicles and the trabecular lattice repeat over. Twenty millimetres is
    /// two centimetres of body per repeat: long enough that a hand-sized wound never shows the same
    /// grain twice, short enough that the noise lattice stays cheap.
    ///
    /// **This is a physical length, not a texel count.** A texel's millimetre position comes from
    /// [`scale`](Self::scale) and the canvas size, so moving to a bigger canvas makes the grain
    /// finer in texels and leaves it exactly where it was on the body.
    pub tile_mm: f32,
    /// The one seed the tissue noise is drawn from.
    ///
    /// Every random-looking value in a flaymap comes from `crate::bloodstain::hash_f32` under
    /// `crate::cross_section::texel_at`, keyed by integer lattice coordinates and this number. There is
    /// no RNG in this crate and no clock, so a wound is a pure function of the hits that made it.
    pub seed: u32,
    /// Millimetres per mesh unit and the strip's along-axis tile, as `bevy_cross_section` states
    /// them. `Scale::default()` — `mm_per_unit: 1000.0`, i.e. a mesh authored in metres.
    ///
    /// Only `mm_per_unit` is read here, and it is read for exactly one thing: turning a texel index
    /// into a position in millimetres, `texel · UV_SPAN_M · mm_per_unit / size`. The whole field is
    /// carried rather than the one number so a game holds **one** `Scale` and hands the same value to
    /// the cross-section bake and to this crate — two copies of a scale that disagreed would put the
    /// grain of a cut face and the grain of the flayed skin beside it at two different sizes.
    #[cfg_attr(feature = "serde", serde(with = "ScaleDef"))]
    pub scale: Scale,
}

impl Default for FlaySettings {
    fn default() -> Self {
        Self {
            max_canvas_updates_per_tick: 4,
            tile_mm: 20.0,
            seed: 0xF1A9_5EED,
            scale: Scale::default(),
        }
    }
}

/// **`crate::cross_section::Scale`'s missing serde derives, supplied here.**
///
/// `Scale` carries no `serde` derive in `bevy_cross_section 0.1` and this crate is not allowed to
/// grow one there, so without this mirror the `serde` feature could not round-trip [`FlaySettings`] —
/// which is the one type in the crate a game authors in a config file rather than in code. Serde's
/// documented remote-derive pattern is exactly this shape: a mirror struct with the same fields, used
/// through `#[serde(with = ..)]`, and no `From` impl needed because both fields are public.
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(remote = "Scale")]
struct ScaleDef {
    mm_per_unit: f32,
    tile_units: f32,
}
