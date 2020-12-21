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

impl Clone for Bites {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self::new(&*self)
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
    fn drop(&mut self) {
        if self.0[self.0.len() - 1] & 0x80 == 0 {
            // Inline: nothing to do.
        } else {
            // Heap: free the pointer.
            unsafe {
                Box::from_raw(u64::from_le_bytes(self.0[0..8].try_into().unwrap()) as *mut u8);
            }
        }
    }
}

impl From<&[u8]> for Bites {
    #[inline(always)]
    fn from(given: &[u8]) -> Self {
        Self::new(given)
    }
}
