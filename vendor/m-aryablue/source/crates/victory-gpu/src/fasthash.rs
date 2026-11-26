//! a multiply-fold hasher for maps keyed by our own addresses and counts (not one to face keys anyone chooses)

use std::hash::{BuildHasherDefault, Hasher};

const K: u64 = 0x9e37_79b9_7f4a_7c15;

#[derive(Clone, Copy)]
pub(crate) struct Fold(u64);

impl Default for Fold {
    fn default() -> Self {
        Fold(K)
    }
}

impl Fold {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(K);
    }
}

impl Hasher for Fold {
    fn write(&mut self, bytes: &[u8]) {
        let mut words = bytes.chunks_exact(8);
        for word in &mut words {
            self.add(u64::from_le_bytes(word.try_into().unwrap_or([0; 8])));
        }
        let rest = words.remainder();
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(last) ^ (rest.len() as u64) << 56);
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.add(u64::from(n));
    }

    fn write_u16(&mut self, n: u16) {
        self.add(u64::from(n));
    }

    fn write_u32(&mut self, n: u32) {
        self.add(u64::from(n));
    }

    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }

    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }

    // the low bits pick the bucket: the high ones, where the multiplying mixed, are folded in
    fn finish(&self) -> u64 {
        let h = self.0;
        (h ^ (h >> 32)).wrapping_mul(K) ^ (h >> 29)
    }
}

pub(crate) type Build = BuildHasherDefault<Fold>;
pub(crate) type Map<K, V> = std::collections::HashMap<K, V, Build>;
pub(crate) type Set<T> = std::collections::HashSet<T, Build>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    fn of<T: Hash>(t: T) -> u64 {
        let mut h = Fold::default();
        t.hash(&mut h);
        h.finish()
    }

    #[test]
    fn aligned_addresses_spread_over_the_buckets() {
        // 4,096 addresses 64 KiB apart, as the copies' keys are: the low 12 bits take nearly as many values
        let low: std::collections::HashSet<u64> = (0..4096u64).map(|i| of((0x1_c000_0000usize + (i as usize) << 16, 64usize)) & 0xfff).collect();
        assert!(low.len() > 2400, "{}", low.len());
        let top: std::collections::HashSet<u64> = (0..4096u64).map(|i| of((0x1_c000_0000usize + (i as usize) << 16, 64usize)) >> 57).collect();
        assert_eq!(top.len(), 128);
    }

    #[test]
    fn keys_that_differ_in_one_field_or_one_byte_differ() {
        assert_ne!(of((1u64, 2u32, 3u64)), of((1u64, 3u32, 3u64)));
        assert_ne!(of((1u64, 2u32, 3u64)), of((2u64, 1u32, 3u64)));
        assert_ne!(of([1u32, 2, 3]), of([1u32, 2, 4]));
        assert_ne!(of(&b"abc"[..]), of(&b"abd"[..]));
        assert_ne!(of(&b"abc\0"[..]), of(&b"abc"[..]));
    }
}
