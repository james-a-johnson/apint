use std::alloc::{GlobalAlloc, Layout, System};
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
    pub fn new_u8(val: u8) -> Self {
        Self {
            num_bits: 8,
            value: Backing { val: val as u64 },
        }
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
}

impl Drop for Value {
    fn drop(&mut self) {
        if !self.interned() {
            // SAFETY: We know the union has a pointer because the data is not interned.
            let ptr = unsafe { self.value.ptr };
            let num_allocated = self.num_words();
            let layout = Layout::array::<u64>(num_allocated)
                .expect("Should be able to create layout for something we allocated");
            // SAFETY: The layout has nonzero size since we don't support nonzero sizes.
            unsafe { System.dealloc(ptr.as_ptr() as *mut _, layout) };
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
            let layout =
                Layout::array::<u64>(value.len()).expect("Can't create layout for backing memory");
            // SAFETY: The size can't be zero.
            let ptr = unsafe { System.alloc(layout) };
            let ptr = NonNull::new(ptr as *mut u64).expect("Allocation failed");
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
