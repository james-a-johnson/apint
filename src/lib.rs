//! Arbitrary precision integer arithmetic library.
//!
//! This library is based off of [LLVM's APInt](https://github.com/llvm/llvm-project/blob/main/llvm/include/llvm/ADT/APInt.h)
//! class. It offers the ability to emulate operations on integers with an arbitrary number of bits.
//!
//! This specific implementation is written expressly for the needs of [Emil](https://github.com/james-a-johnson/emil). So some
//! implementation choices and decision decisions may differ from the LLVM one slightly.
//!
//! The main export of this library is the [`Value`] struct. That represents signed or unsigned integers with an arbitrary number of bits.
//! It supports all of the operations that are required by Emil.

mod alloc;
use std::hint::cold_path;
use std::ops::{Add, BitOr};
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

// SAFETY: Value will not automatically be marked as Send because it contains a `NonNull` in the `Backing` field.
// However, the data that points to is owned heap memory that is not aliased by anything else. That means this struct
// is safe to send to another thread.
unsafe impl Send for Value {}
// SAFETY: Not automatically Sync because of the `NonNull` in the `Backing` field. However, that points to owned
// heap allocated data. It is safe to share references to taht data across threads.
unsafe impl Sync for Value {}

/// Integer value with an arbitrary number of bits.
///
/// This struct is used to represent signed or unsigned integers with an arbitrary number of bits.
///
/// You can create one from any of the basic integer types (`u8`, `u16`, `u32`, `u64`). Constructors only accept the unsigned version
/// of types. You can cast a signed value to the unsigned version and then construct it if you want to represent a signed value. Each
/// operation that this struct supports will either implement signed or unsigned behavior.
///
/// Each of the operations from [`std::ops`] implement unsigned operations. Only methods that explicitly say they are signed operations
/// emulate a signed operation.
///
/// # Implementation
/// `Value` is a manually implemented tagged union. The `num_bits` field serves as the tag and indicats now many bits are in the value
/// the instance represents. 64 or fewer bits and the value is stored in a single `u64` word. More than 64 bits and the value is stored
/// as a number of `u64` words that are allocated on the heap.
///
/// The tagged union comes from the pointer and interned value being the same data held in a C style union. This keeps the struct as
/// small as possible.
///
/// # Examples
///
/// ```
/// use apint::Value;
///
/// let val1 = Value::new_u8(123);
/// let val2 = Value::new_u8(111);
/// let result = val1 + val2;
/// assert_eq!(result.get_word(), 234);
/// ```
impl Value {
    #[inline]
    pub fn new_u8(val: u8) -> Self {
        Self {
            num_bits: 8,
            value: Backing { val: val as u64 },
        }
    }

    #[inline]
    pub fn new_u16(val: u16) -> Self {
        Self {
            num_bits: 16,
            value: Backing { val: val as u64 },
        }
    }

    #[inline]
    pub fn new_u32(val: u32) -> Self {
        Self {
            num_bits: 32,
            value: Backing { val: val as u64 },
        }
    }

    #[inline]
    pub fn new_u64(val: u64) -> Self {
        Self {
            num_bits: 64,
            value: Backing { val },
        }
    }

    pub fn parse_from_words(data: &[u64], num_bits: u32) -> Self {
        let mut value = if num_bits <= 64 {
            Self {
                num_bits,
                value: Backing { val: data[0] },
            }
        } else {
            cold_path();
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
            cold_path();
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

    pub fn get_word(&self) -> u64 {
        if self.interned() {
            unsafe { self.value.val }
        } else {
            panic!("Value is not interned")
        }
    }

    pub fn get_slice(&self) -> &[u64] {
        if self.interned() {
            panic!("Value is interned")
        } else {
            unsafe { self.as_slice() }
        }
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
        let num_words = self.num_words();
        // SAFETY: This is a pointer to `self.byte_size()` u64s on the heap. This slice is valid.
        unsafe { std::slice::from_raw_parts(ptr.as_ptr(), num_words as usize) }
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
        let num_words = self.num_words();
        // SAFETY: This is a pointer to `self.byte_size()` u64s on the heap. This slice is valid.
        unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), num_words as usize) }
    }

    fn big_add(&self, rhs: &Self, mut carry: bool) -> Self {
        assert!(!self.interned());
        assert_eq!(self.num_bits, rhs.num_bits);
        let num_words = self.num_words();
        let ptr = crate::alloc::alloc_bits(self.num_bits);
        // SAFETY: We know both of them are not interned.
        let lhs = unsafe { self.as_slice() };
        let rhs = unsafe { rhs.as_slice() };
        for i in 0..num_words {
            let (val, new_carry) = lhs[i].carrying_add(rhs[i], carry);
            unsafe { ptr.add(i).write(val) };
            carry = new_carry;
        }
        Self {
            num_bits: self.num_bits,
            value: Backing { ptr },
        }
    }
}

impl Add for Value {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        assert!(
            self.num_bits == rhs.num_bits,
            "Adding values with different bit widths"
        );
        let mut new_value = if self.interned() {
            Self {
                num_bits: self.num_bits,
                value: Backing {
                    val: unsafe { self.value.val + rhs.value.val },
                },
            }
        } else {
            cold_path();
            self.big_add(&rhs, false)
        };
        new_value.clear_unused_bits();
        new_value
    }
}

impl BitOr for Value {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        assert!(
            self.num_bits == rhs.num_bits,
            "Bitwise OR of values with different bit widths"
        );
        // Don't need to clear unused bits here because they can't get set via a bitwise or
        if self.interned() {
            let val = unsafe { self.value.val | rhs.value.val };
            Self {
                num_bits: self.num_bits,
                value: Backing { val },
            }
        } else {
            cold_path();
            let num_words = self.num_words();
            let ptr = crate::alloc::alloc_bits(self.num_bits);
            // SAFETY: We know both of them are not interned.
            let lhs = unsafe { self.as_slice() };
            let rhs = unsafe { rhs.as_slice() };
            for i in 0..num_words {
                let val = lhs[i] | rhs[i];
                unsafe { ptr.add(i).write(val) };
            }
            Self {
                num_bits: self.num_bits,
                value: Backing { ptr },
            }
        }
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
            cold_path();
            let num_words = self.num_words();
            let ptr = crate::alloc::alloc_bits(self.num_bits);
            // SAFETY: We know both of them are not interned.
            unsafe {
                std::ptr::copy(self.value.ptr.as_ptr(), ptr.as_ptr(), num_words);
            }
            Self {
                num_bits: self.num_bits,
                value: Backing { ptr },
            }
        }
    }
}

impl Drop for Value {
    fn drop(&mut self) {
        if !self.interned() {
            cold_path();
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
            cold_path();
            let num_bits: u32 = (64usize.saturating_mul(value.len()))
                .try_into()
                .expect("Can't represent that many bits");

            let ptr = crate::alloc::alloc_bits(num_bits);
            Self {
                num_bits,
                value: Backing { ptr },
            }
        } else {
            cold_path();
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

    #[test]
    fn adding_values() {
        let small_a = Value::new_u32(u32::MAX);
        let small_b = Value::new_u32(2);
        let small_c = small_a + small_b;
        assert_eq!(small_c.get_word(), 0x1);

        let big_a = Value::parse_from_words(&[u64::MAX, 2], 96);
        let big_b = Value::parse_from_words(&[1, 3], 96);
        let big_c = big_a + big_b;
        assert_eq!(big_c.get_slice(), &[0, 6]);
    }

    #[test]
    fn bitwise_or_values() {
        let small_a = Value::new_u64(0b10101010101010101010);
        let small_b = Value::new_u64(0b01010101010101010101);
        let small_c = small_a | small_b;
        assert_eq!(small_c.get_word(), 0b11111111111111111111);

        let big_a = Value::parse_from_words(&[0b1010, 0b1100], 67);
        let big_b = Value::parse_from_words(&[0b0101, 0b0011], 67);
        let big_c = big_a | big_b;
        assert_eq!(big_c.get_slice(), &[0b1111, 0b0111]);
    }

    #[test]
    fn different_sized_ops_panic() {
        let a = Value::new_u8(12);
        let b = Value::new_u16(100);
        let result = std::panic::catch_unwind(|| a.clone() + b.clone());
        assert!(result.is_err());
        let result = std::panic::catch_unwind(|| a.clone() | b.clone());
        assert!(result.is_err());
    }
}
