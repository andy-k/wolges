// Copyright (C) 2020-2026 Andy Kurnia.

// Fast insecure non-cryptographic hash.

#[derive(Default)]
pub struct MyHasher(u64);

impl std::hash::Hasher for MyHasher {
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.wrapping_mul(3467) ^ (!b as u64);
        }
    }
}

pub type MyHasherDefault = std::hash::BuildHasherDefault<MyHasher>;
pub type MyHashMap<K, V> = std::collections::HashMap<K, V, MyHasherDefault>;
pub type MyHashSet<T> = std::collections::HashSet<T, MyHasherDefault>;
