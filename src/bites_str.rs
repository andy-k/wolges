// Copyright (C) 2020-2024 Andy Kurnia.

pub struct BitesStr([u8; 16]);

impl BitesStr {
    #[inline(always)]
    fn new(given: &str) -> Self {
        let mut ret = [0u8; 16];
        if given.len() < ret.len() {
            // Inline: 0..15 = data, 15 = length (max 15), msb is unset.
            ret[ret.len() - 1] = given.len() as u8;
            ret[0..given.len()].copy_from_slice(given.as_bytes());
        } else {
            // Heap: 0..8 = pointer, 8..16 = length (little endian), msb is set.
            ret[0..8].copy_from_slice(
                &(Box::leak(Box::<str>::from(given)).as_ptr() as u64).to_le_bytes(),
            );
            ret[8..16].copy_from_slice(&(given.len() as u64 | !(!0 >> 1)).to_le_bytes());
        }
        BitesStr(ret)
    }
}

impl std::ops::Deref for BitesStr {
    type Target = str;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        if self.0[self.0.len() - 1] & 0x80 == 0 {
            // Inline: slice the backing slice.
            unsafe { std::str::from_utf8_unchecked(&self.0[0..self.0[self.0.len() - 1] as usize]) }
        } else {
            // Heap: turn off msb for actual length.
            unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                    u64::from_le_bytes(self.0[0..8].try_into().unwrap()) as *const _,
                    (u64::from_le_bytes(self.0[8..16].try_into().unwrap()) & (!0 >> 1)) as usize,
                ))
            }
        }
    }
}

impl Drop for BitesStr {
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

impl AsRef<str> for BitesStr {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        &self[..]
    }
}

impl std::borrow::Borrow<str> for BitesStr {
    #[inline(always)]
    fn borrow(&self) -> &str {
        self
    }
}

impl Clone for BitesStr {
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
            .clone_from_slice(source.as_bytes());
        } else {
            // Optimal for all other cases since boxed slices cannot be resized.
            *self = source.clone() as _;
        }
    }
}

impl std::fmt::Debug for BitesStr {
    #[inline(always)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // caution: exactly as debug-formatting the underlying &str,
        // padding/alignment is applied to each element.
        self[..].fmt(f)
    }
}

impl From<&str> for BitesStr {
    #[inline(always)]
    fn from(given: &str) -> Self {
        Self::new(given)
    }
}

impl Eq for BitesStr {}

impl std::hash::Hash for BitesStr {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self[..].hash(state)
    }
}

impl Ord for BitesStr {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self[..].cmp(&other[..])
    }
}

impl PartialEq for BitesStr {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        PartialEq::eq(&self[..], &other[..])
    }
}

impl PartialOrd for BitesStr {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
