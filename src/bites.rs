// Copyright (C) 2020-2023 Andy Kurnia.

pub struct Bites([u8; 16]);

impl Bites {
    #[inline(always)]
    fn new(given: &[u8]) -> Self {
        let mut ret = [0u8; 16];
        if given.len() < ret.len() {
            // Inline: 0..15 = data, 15 = length (max 15), msb is unset.
            ret[ret.len() - 1] = given.len() as u8;
            ret[0..given.len()].copy_from_slice(given);
        } else {
            // Heap: 0..8 = pointer, 8..16 = length (little endian), msb is set.
            ret[0..8].copy_from_slice(
                &(Box::leak(Box::<[u8]>::from(given)).as_ptr() as u64).to_le_bytes(),
            );
            ret[8..16].copy_from_slice(&(given.len() as u64 | !(!0 >> 1)).to_le_bytes());
        }
        Bites(ret)
    }
}

use std::convert::TryInto;

impl std::ops::Deref for Bites {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        if self.0[self.0.len() - 1] & 0x80 == 0 {
            // Inline: slice the backing slice.
            &self.0[0..self.0[self.0.len() - 1] as usize]
        } else {
            // Heap: turn off msb for actual length.
            unsafe {
                std::slice::from_raw_parts(
                    u64::from_le_bytes(self.0[0..8].try_into().unwrap()) as *const _,
                    (u64::from_le_bytes(self.0[8..16].try_into().unwrap()) & (!0 >> 1)) as usize,
                )
            }
        }
    }
}

impl Drop for Bites {
    // Do not inline, this code is big but the Heap case is rare.
    #[inline(never)]
    fn drop(&mut self) {
        if self.0[self.0.len() - 1] & 0x80 == 0 {
            // Inline: nothing to do.
        } else {
            // Heap: free the pointer.
            drop(unsafe {
                Box::from_raw(u64::from_le_bytes(self.0[0..8].try_into().unwrap()) as *mut u8)
            });
        }
    }
}

impl AsRef<[u8]> for Bites {
    #[inline(always)]
    fn as_ref(&self) -> &[u8] {
        &self[..]
    }
}

impl std::borrow::Borrow<[u8]> for Bites {
    #[inline(always)]
    fn borrow(&self) -> &[u8] {
        self
    }
}

impl Clone for Bites {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self::new(&self[..])
    }

    #[inline(always)]
    fn clone_from(&mut self, source: &Self) {
        if self.0[self.0.len() - 1] & 0x80 != 0 && self.len() == source.len() {
            // Heap, same length.
            unsafe {
                std::slice::from_raw_parts_mut(
                    u64::from_le_bytes(self.0[0..8].try_into().unwrap()) as *mut _,
                    (u64::from_le_bytes(self.0[8..16].try_into().unwrap()) & (!0 >> 1)) as usize,
                )
            }
            .clone_from_slice(source);
        } else {
            // Optimal for all other cases since boxed slices cannot be resized.
            *self = source.clone();
        }
    }
}

impl std::fmt::Debug for Bites {
    #[inline(always)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // caution: exactly as debug-formatting the underlying &[u8],
        // padding/alignment is applied to each element.
        self[..].fmt(f)
    }
}

impl From<&[u8]> for Bites {
    #[inline(always)]
    fn from(given: &[u8]) -> Self {
        Self::new(given)
    }
}

impl Eq for Bites {}

impl std::hash::Hash for Bites {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self[..].hash(state)
    }
}

impl Ord for Bites {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self[..].cmp(&other[..])
    }
}

impl PartialEq for Bites {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self[..].eq(&other[..])
    }
}

impl PartialOrd for Bites {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
