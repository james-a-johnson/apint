use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr::NonNull;

/// Allocate a block of memory to hold `num_bits` bits.
///
/// Returns a pointer to the allocated memory, or panics if the allocation fails.
///
/// Backing memory is a number of `u64` values allocated on the heap. `num_bits` is rounded up to the nearest multiple of 64 bits, so `num` is the number of `u64` values needed.
///
/// # Panics
/// Panics if `num_bits` is less than 64. This allocator is specifically for the APInt use case which will not allocate for anything less than that.
///
/// Also panics if the allocation fails.
pub fn alloc_bits(num_bits: u32) -> NonNull<u64> {
    assert!(num_bits > 64, "Allocating a size that should be interned");
    let num = num_bits.div_ceil(64);
    let num = num as usize;
    // SAFETY: Requirements of this function are that align is not zero, align is a power of two, and size rounded up to the nearest multiple of align does not overflow isize.
    // The alignment is calculated by std::mem::align_of::<u64>() which should just be 8. That is non-zero and a power of two. The size will be some multiple
    // of the size of u64 which is also 8 bytes. We limit to allocating at most u32 u64s. That means the largest value will be u32::MAX * 8 bytes which will not
    // overflow an isize on a 64 bit system.
    let layout = unsafe {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "The multiplication here can never overflow"
        )]
        Layout::from_size_align_unchecked(
            num * std::mem::size_of::<u64>(),
            std::mem::align_of::<u64>(),
        )
    };
    // SAFETY: This method requires that size must be non-zero. We assert that at the entry to the function so we know that to be the case.
    let ptr = unsafe { System.alloc_zeroed(layout) };
    if ptr.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    // SAFETY: Checked for the pointer being null above so we know it's valid here.
    unsafe { NonNull::new_unchecked(ptr.cast()) }
}

/// Free the memory used by a value with `num_bits` bits.
///
/// # Panics
/// Panics if `num_bits` is less than 64. This allocator is specifically for the APInt use case which will not allocate for anything less than that.
pub fn free_bits(ptr: NonNull<u64>, num_bits: u32) {
    assert!(
        num_bits > 64,
        "Freeing a size that should have been interned"
    );
    let num = num_bits.div_ceil(64);
    let num = num as usize;
    // SAFETY: Requirements of this function are that align is not zero, align is a power of two, and size rounded up to the nearest multiple of align does not overflow isize.
    let layout = unsafe {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "The multiplication here can never overflow"
        )]
        Layout::from_size_align_unchecked(
            num * std::mem::size_of::<u64>(),
            std::mem::align_of::<u64>(),
        )
    };
    // SAFETY: This method requires that size must be non-zero. We assert that at the entry to the function so we know that to be the case.
    unsafe { System.dealloc(ptr.as_ptr().cast(), layout) };
}
