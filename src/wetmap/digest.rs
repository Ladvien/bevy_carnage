//! **FNV-1a over the wet buffer, and nothing else.**
//!
//! # Why the CPU buffer and not the images
//!
//! The two `Image`s a canvas owns are **output**. `crates/bevy_carnage/src/vfx.rs:6-19` states the rule
//! this crate inherits: GPU output is write-only, nothing reads it back, and a value that cannot reach
//! a hash cannot be the authority for one. So the digest is taken over `(amount, age)` in row-major
//! order — the CPU state that *decides* what the images will say — and the uploaded pixels are a
//! derived, cosmetic account of it.
//!
//! That is also what makes this crate's headline claim checkable. Texture-space blood accumulation
//! elsewhere is a GPU render target, which is why nobody can hash it; hashing this one is a two-line
//! fold because the state never left the CPU.
//!
//! FNV-1a (Fowler–Noll–Vo, 1991) rather than a cryptographic hash: 64 bits over 3 bytes per texel, one
//! multiply and one xor per byte, and no allocation. It is a fingerprint for equality, not a signature.

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A running FNV-1a 64 fold.
///
/// Kept as a type rather than a free function over a slice because a texel's state is a `(u8, u16)`
/// tuple, not bytes — building a byte buffer to hash it would allocate a second copy of the canvas
/// every time anyone asked for a digest.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fnv1a(u64);

impl Fnv1a {
    /// A fresh fold at the offset basis.
    pub(crate) const fn new() -> Self {
        Self(FNV_OFFSET)
    }

    /// Fold one byte in.
    #[inline]
    pub(crate) fn byte(&mut self, b: u8) {
        self.0 = (self.0 ^ b as u64).wrapping_mul(FNV_PRIME);
    }

    /// Fold a `u16` in, **little-endian**, because a byte order that varied by target would make the
    /// digest a property of the machine rather than of the blood.
    #[inline]
    pub(crate) fn u16(&mut self, v: u16) {
        self.byte((v & 0xff) as u8);
        self.byte((v >> 8) as u8);
    }

    /// The fold so far.
    pub(crate) const fn finish(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published FNV-1a 64 test vector for `"a"`, so a future refactor of the fold is checked
    /// against the algorithm rather than against itself.
    #[test]
    fn the_fold_is_fnv_1a() {
        let mut f = Fnv1a::new();
        f.byte(b'a');
        assert_eq!(f.finish(), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn a_u16_folds_little_endian() {
        let mut a = Fnv1a::new();
        a.u16(0x0102);
        let mut b = Fnv1a::new();
        b.byte(0x02);
        b.byte(0x01);
        assert_eq!(a.finish(), b.finish());
    }
}
