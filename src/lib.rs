mod alloc;
use std::ptr::NonNull;

pub struct Value {
    /// Number of bits represented by this value.
    ///
    /// If bits if <= 64, then there is just a u64 in `value`. Otherwise, the data is backed by an array of bytes
    /// on the heap that `value` points to.
    num_bits: u32,
    /// Value tracked by this struct.
    ///
    /// Can be either directly a value in a u64 or a pointer to u64s on the heap.
    value: Backing,
}

impl Value {
    #[inline]
    pub fn new_u8(val: u8) -> Self {
        Self {
            num_bits: 8,
            value: Backing { val: val as u64 },
        }
    }

    pub fn parse_from_words(data: &[u64], num_bits: u32) -> Self {
        let mut value = if num_bits <= 64 {
            Self {
                num_bits,
                value: Backing { val: data[0] },
            }
        } else {
            let num_words = num_bits.div_ceil(64);
            assert!(data.len() >= num_words as usize, "Not enough data to parse");
            let ptr = crate::alloc::alloc_bits(num_bits);
            // SAFETY: Requirements: source and destination must be valid for reads/writes of `num_words` u64s and they must be properly aligned.
            // A softer requirement is that they must not overlap in a way that could invalidate anything.
            // We know that both are valid for `num_words` reads and writes because we check the size of source and allocate that many for the
            // destination. They must be aligned properly because one came from a slice which guarantees that and our allocator guarantees that
            // the returned pointer is properly aligned for a u64. Lastly, we know there is no overlap since the destination is a new allocation.
            unsafe { std::ptr::copy(data.as_ptr(), ptr.as_ptr(), num_words as usize) };
            Self {
                num_bits,
                value: Backing { ptr },
            }
        };
        value.clear_unused_bits();
        value
    }

    /// Clear any high bits that are not used.
    fn clear_unused_bits(&mut self) {
        let zero_bits = 64 - (self.num_bits % 64);
        let mask = u64::MAX >> zero_bits;
        if self.interned() {
            unsafe {
                self.value.val &= mask;
            }
        } else {
            unsafe {
                let slice = self.as_slice_mut();
                slice[slice.len() - 1] &= mask;
            }
        }
    }

    /// Get the size of the represented value in bits.
    #[inline]
    pub fn size(&self) -> u32 {
        self.num_bits
    }

    /// Check if the value is stored directly in the associated union or not.
    ///
    /// If the represented value fits in a u64, then it is stored directly in this struct. Otherwise, this struct has
    /// a pointer to data on the heap where the data lives.
    #[inline]
    pub fn interned(&self) -> bool {
        self.num_bits <= 64
    }

    #[inline]
    pub fn byte_size(&self) -> u32 {
        self.num_bits.div_ceil(8)
    }

    #[inline]
    pub fn num_words(&self) -> usize {
        (self.num_bits as usize).div_ceil(64)
    }

    /// Get a slice to the bytes on the heap.
    ///
    /// # Safety
    /// Only safe if the backing data is actually stored on the heap. You can check that by calling [`Self::interned`].
    /// You may only call this method if that function would return false.
    pub unsafe fn as_slice(&self) -> &[u64] {
        // SAFETY: Safety contract of this function requires that the struct actually contains a pointer. So this is
        // safe as long as the function's safety contract is satisfied.
        let ptr = unsafe { self.value.ptr };
        let num_bytes = self.byte_size();
        // SAFETY: This is a pointer to `self.byte_size()` u64s on the heap. This slice is valid.
        unsafe { std::slice::from_raw_parts(ptr.as_ptr(), num_bytes as usize) }
    }

    /// Get a mutable slice to the bytes on the heap.
    ///
    /// # Safety
    /// Only safe if the backing data is actually stored on the heap. You can check that by calling [`Self::interned`].
    /// You may only call this method if that function would return false.
    pub unsafe fn as_slice_mut(&mut self) -> &mut [u64] {
        // SAFETY: Safety contract of this function requires that the struct actually contains a pointer. So this is
        // safe as long as the function's safety contract is satisfied.
        let ptr = unsafe { self.value.ptr };
        let num_bytes = self.byte_size();
        // SAFETY: This is a pointer to `self.byte_size()` u64s on the heap. This slice is valid.
        unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), num_bytes as usize) }
    }
}

impl Clone for Value {
    fn clone(&self) -> Self {
        if self.interned() {
            Self {
                num_bits: self.num_bits,
                value: Backing {
                    val: unsafe { self.value.val },
                },
            }
        } else {
            todo!()
        }
    }
}

impl Drop for Value {
    fn drop(&mut self) {
        if !self.interned() {
            // SAFETY: We know the union has a pointer because the data is not interned.
            let ptr = unsafe { self.value.ptr };
            crate::alloc::free_bits(ptr, self.num_bits);
        }
    }
}

impl From<&[u64]> for Value {
    fn from(value: &[u64]) -> Self {
        if value.len() == 1 {
            let val = value[0];
            Self {
                num_bits: 64,
                value: Backing { val },
            }
        } else if value.len() > 1 {
            let num_bits: u32 = (64usize.saturating_mul(value.len()))
                .try_into()
                .expect("Can't represent that many bits");

            let ptr = crate::alloc::alloc_bits(num_bits);
            Self {
                num_bits,
                value: Backing { ptr },
            }
        } else {
            panic!("Don't support zero sized integers");
        }
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut format = f.debug_struct("Value");
        format.field("bits", &self.num_bits);
        if self.interned() {
            // SAFETY: Checked that bit size is less than or equal to 64 so we know this is the correct field to use
            // from the union.
            let value = unsafe { self.value.val };
            format.field("value", &value).finish()
        } else {
            // SAFETY: Checked the bit size is greater than 64 so this is the correct field to use.
            let slice = unsafe { self.as_slice() };
            format.field("value", &slice).finish()
        }
    }
}

impl std::fmt::LowerHex for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut format = f.debug_struct("Value");
        format.field("bits", &self.num_bits);
        if self.interned() {
            // SAFETY: Checked the bit size so this is the correct field to use.
            let value = unsafe { self.value.val };
            let formatter = std::fmt::from_fn(|f| <u64 as std::fmt::LowerHex>::fmt(&value, f));
            format.field("value", &formatter).finish()
        } else {
            // SAFETY: Checked the bit size so this is the correct field to use.
            let slice = unsafe { self.as_slice() };
            let formatter = std::fmt::from_fn(|f| write!(f, "{:x?}", slice));
            format.field("value", &formatter).finish()
        }
    }
}

union Backing {
    val: u64,
    ptr: NonNull<u64>,
}

const _: () = assert!(std::mem::size_of::<Backing>() == 8);

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn interns_small_values() {
        let small = Value::new_u8(128);
        assert!(small.interned());

        let small_slice = Value::parse_from_words(&[4096], 48);
        assert!(small_slice.interned());
    }

    #[test]
    fn allocates_large_values() {
        let large = Value::parse_from_words(&[4096, 4096], 96);
        assert!(!large.interned());
    }
}
