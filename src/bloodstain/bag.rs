//! **Variety selection that cannot repeat early, and cannot be perturbed by iteration order.**
//!
//! # The arithmetic this module exists to fix
//!
//! Picking a variant with `seed % n` is a fresh uniform draw every time, so it has a birthday
//! problem. With four variants the probability that four consecutive stains contain a repeat is
//!
//! > `1 − 4! / 4⁴ = 1 − 24/256 = 90.6 %`
//!
//! so the *expected* first visible repeat is the fourth stain. A floor of blood reads as tiled almost
//! immediately, and authoring more textures moves the number without fixing the shape of the problem.
//!
//! # Why not a shuffle bag with a cursor
//!
//! The usual fix is a bag you draw from and refill, which is a **mutable cursor**. Two problems, both
//! recorded failures in this family of crates: a cursor advanced from inside an ECS system is advanced
//! in query order, which is not stable across `App` instances; and a cursor is state a replay has to
//! carry and restore, so a single missed increment desynchronises everything after it permanently.
//!
//! So this bag has no state. It is a **pure function of a draw ordinal**, which means a replay
//! reproduces it by construction and nothing about the order things were visited can reach it.

use crate::bloodstain::{hash_f32, m};

/// The largest `n` this module will build a permutation for.
///
/// A bag of more than this is not a variety problem, it is a texture atlas, and the fixed-size buffer
/// below is what keeps the whole module allocation-free.
pub const MAX_VARIANTS: u32 = 64;

/// **Pick a variant for one draw.** `bag_epoch` is the draw's ordinal, `site` keys an independent
/// sequence per place, `n` is how many variants exist, `avoid_last` is the caller's requirement.
///
/// # The construction, and the gap it guarantees
///
/// Draws are grouped into blocks of `n`. Block `b` deals the site's own base permutation rotated by
/// `b`, so **every block is a permutation** (no repeat inside a block) and the boundary between two
/// blocks is a rotation by one (no repeat across it either). Written out, the index dealt at ordinal
/// `e` is
///
/// > `idx = (e / n + e % n) mod n`
///
/// and the closest two equal indices can be is `n − 1` draws apart: the same index recurs only when
/// the slot within the block falls by one as the block rises by one. So **the minimum gap between
/// repeats is `n − 1`**, exactly, for every `n` and every site.
///
/// `avoid_last` is the caller stating the gap it needs. This construction meets it whenever
/// `n >= avoid_last + 2`; at four variants and `avoid_last = 2` that is `4 >= 4`, so the minimum gap
/// is 3 and the requirement is met with nothing to spare. **When `n` is too small the requirement is
/// arithmetically unsatisfiable** — you cannot deal four variants with a gap of five — and this
/// function still returns the best available spacing rather than looping, panicking, or inventing a
/// variant that does not exist. That is not a fallback: there is one code path, and `avoid_last`'s job
/// is to make the caller's requirement legible at the call site and checkable in a test.
///
/// A rotation of one is optimal, not arbitrary: rotating by `d` gives a minimum gap of `n − d`, so
/// every other stride is strictly worse.
pub fn pick(bag_epoch: u32, site: u32, n: u32, avoid_last: u32) -> u32 {
    let _ = avoid_last;
    if n <= 1 {
        return 0;
    }
    let n = n.min(MAX_VARIANTS);
    let block = bag_epoch / n;
    let slot = bag_epoch % n;
    let idx = (block.wrapping_add(slot)) % n;
    base_permutation(site, n, idx)
}

/// The `i`th element of the site's base permutation of `0..n`.
///
/// Fisher–Yates over a fixed-size buffer, keyed by the site alone — **not by the epoch**, because the
/// gap guarantee above is a property of dealing one permutation in rotations, and a permutation that
/// changed per block would put the boundary back in play.
fn base_permutation(site: u32, n: u32, i: u32) -> u32 {
    let len = n.min(MAX_VARIANTS) as usize;
    let mut perm = [0u8; MAX_VARIANTS as usize];
    for (k, slot) in perm.iter_mut().enumerate().take(len) {
        *slot = k as u8;
    }
    // Descending Fisher–Yates, one hash per step, so the permutation is a pure function of `site`.
    let mut k = len;
    while k > 1 {
        k -= 1;
        let key = site.wrapping_mul(0x9E37_79B9) ^ (k as u32).wrapping_mul(0x85EB_CA6B);
        let j = (m::round(hash_f32(key) * k as f32) as usize).min(k);
        perm.swap(k, j);
    }
    let idx = (i as usize).min(len.saturating_sub(1));
    perm[idx] as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec as StdVec;

    /// **The gap requirement, over a long run.** This is the property `seed % n` fails at the fourth
    /// stain, and it is the whole reason this module exists.
    #[test]
    fn four_variants_never_repeat_within_three_draws() {
        let n = 4u32;
        let avoid = 2u32;
        for site in [0u32, 1, 7, 0xDEAD_BEEF] {
            let seq: StdVec<u32> = (0..10_000u32).map(|e| pick(e, site, n, avoid)).collect();
            for (i, v) in seq.iter().enumerate() {
                for (k, w) in seq.iter().enumerate().skip(i + 1).take(avoid as usize) {
                    assert_ne!(
                        v, w,
                        "site {site} repeated variant {v} at draws {i} and {k}, a gap of {}",
                        k - i
                    );
                }
            }
        }
    }

    /// **Every block deals every variant exactly once.** Blocks, not sliding windows, and the
    /// distinction is the construction rather than a weaker claim: a block is a permutation by
    /// definition, while a sliding window straddling two blocks legitimately holds a repeat at the
    /// guaranteed gap — which is what `four_variants_never_repeat_within_three_draws` measures.
    ///
    /// Coverage is what stops one variant being rare, and the gap is what stops two being adjacent.
    /// They are separate properties and this is the first one.
    #[test]
    fn every_block_deals_every_variant_exactly_once() {
        for n in [2u32, 3, 4, 5, 8] {
            for block in 0..64u32 {
                let mut counts = [0u32; MAX_VARIANTS as usize];
                for slot in 0..n {
                    let v = pick(block * n + slot, 3, n, 1);
                    assert!(v < n, "pick returned {v} for n = {n}");
                    counts[v as usize] += 1;
                }
                for (v, c) in counts.iter().enumerate().take(n as usize) {
                    assert_eq!(
                        *c, 1,
                        "block {block} of {n} dealt variant {v} {c} times, not once"
                    );
                }
            }
        }
    }

    /// Different sites deal different sequences, or the "independent sequence per place" claim is
    /// false and every wall in a level would stain identically.
    #[test]
    fn different_sites_deal_different_sequences() {
        let a: StdVec<u32> = (0..16u32).map(|e| pick(e, 1, 8, 2)).collect();
        let b: StdVec<u32> = (0..16u32).map(|e| pick(e, 2, 8, 2)).collect();
        assert_ne!(a, b, "two sites dealt the same sequence, so the site key does nothing");
    }

    /// Stateless means reproducible: the same ordinal is the same answer, in any order, any number of
    /// times. A cursor could not promise this and that is why there isn't one.
    #[test]
    fn a_draw_is_a_pure_function_of_its_ordinal() {
        let forward: StdVec<u32> = (0..64u32).map(|e| pick(e, 9, 5, 2)).collect();
        let backward: StdVec<u32> = (0..64u32).rev().map(|e| pick(e, 9, 5, 2)).collect();
        for (i, v) in forward.iter().enumerate() {
            assert_eq!(*v, backward[63 - i], "draw {i} depended on the order it was asked in");
        }
    }

    /// Degenerate inputs answer honestly instead of panicking: one variant is always variant zero,
    /// and a bag larger than the ceiling is clamped rather than overflowing the buffer.
    #[test]
    fn degenerate_bags_do_not_panic() {
        assert_eq!(pick(7, 3, 1, 2), 0, "with one variant there is nothing to choose");
        assert_eq!(pick(7, 3, 0, 2), 0, "an empty bag cannot deal anything but zero");
        let v = pick(u32::MAX, 5, MAX_VARIANTS + 100, 2);
        assert!(v < MAX_VARIANTS, "an oversized bag must clamp, got {v}");
    }
}
