// Copyright (C) 2020-2021 Andy Kurnia.

// Fast insecure non-cryptographic hash.

pub struct MyHasher(u64);

impl std::hash::Hasher for MyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (std::num::Wrapping(self.0) * std::num::Wrapping(3467)).0 ^ (!b as u64);
        }
    }
}

impl Default for MyHasher {
    fn default() -> MyHasher {
        MyHasher(0)
    }
}

pub type MyHasherDefault = std::hash::BuildHasherDefault<MyHasher>;
pub type MyHashMap<K, V> = std::collections::HashMap<K, V, MyHasherDefault>;
pub type MyHashSet<T> = std::collections::HashSet<T, MyHasherDefault>;
