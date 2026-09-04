//! **Two integer hashes, and no floating-point randomness anywhere.**
//!
//! A solver whose product is a reproducible digest cannot draw from a general-purpose RNG: the crate
//! would then be reproducible only for as long as that RNG's algorithm is. Both functions here are
//! written out in full, take `u32` in and give `u32`/`u64` out, and are frozen — changing either moves
//! every digest this crate has ever printed, so a change is a deliberate re-blessing, never a tidy-up.

/// The FNV-1a 64-bit offset basis and prime (Fowler, Noll & Vo).
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The starting state of a digest.
pub(crate) const FNV_SEED: u64 = FNV_OFFSET;

/// Fold one 32-bit word into an FNV-1a state, little-endian, one byte at a time.
///
/// Little-endian is named rather than borrowed from the host: `to_le_bytes` gives the same four bytes
/// on every target, so a digest printed on an Apple M-series machine matches one printed on x86.
#[inline]
pub(crate) fn fnv1a_u32(state: u64, word: u32) -> u64 {
    let mut h = state;
    let bytes = word.to_le_bytes();
    let mut i = 0;
    while i < bytes.len() {
        h ^= u64::from(bytes[i]);
        h = h.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    h
}

/// A 32-bit integer bit-mixer — Wellons' `lowbias32`, found by his hash-prospector search.
///
/// Used only to fan [`crate::viscera::spill`]'s strands apart. It is an integer function of an integer, so it
/// has no rounding behaviour to differ between machines, and it is `const` so a caller can build a
/// compile-time table from it.
#[inline]
pub(crate) const fn hash_u32(x: u32) -> u32 {
    let mut x = x;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// A hash in `[0, 1)`, exactly representable.
///
/// The top 24 bits divided by `2^24`: every value is an exact `f32` (a 24-bit significand holds them
/// all), so the division is exact and cannot round differently under a different optimiser.
#[inline]
pub(crate) fn hash_unit(x: u32) -> f32 {
    const SCALE: f32 = 1.0 / 16_777_216.0; // 2^-24
    (hash_u32(x) >> 8) as f32 * SCALE
}

/// Two independent hashes of the same pair, so a caller does not have to invent salt constants.
#[inline]
pub(crate) fn hash_pair(seed: u32, index: u32) -> u32 {
    hash_u32(seed ^ hash_u32(index.wrapping_mul(0x9e37_79b9)))
}
