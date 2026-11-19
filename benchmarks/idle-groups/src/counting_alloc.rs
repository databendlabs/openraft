use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// Wraps the system allocator. Once [`Self::start()`] is called, it counts the allocations and
/// the heap bytes requested and not yet freed.
///
/// Counting costs CPU: every allocation and every free then updates counters that all threads
/// share.
pub(crate) struct CountingAlloc {
    counting: AtomicBool,

    /// The allocations so far, including reallocations.
    allocs: AtomicU64,

    /// The bytes requested and not yet freed. It is signed because it also subtracts the frees of
    /// memory allocated before counting started, and because one thread may count a free before
    /// another thread counts the allocation it frees.
    live_bytes: AtomicI64,
}

impl CountingAlloc {
    pub(crate) const fn new() -> Self {
        Self {
            counting: AtomicBool::new(false),
            allocs: AtomicU64::new(0),
            live_bytes: AtomicI64::new(0),
        }
    }

    /// Start counting.
    pub(crate) fn start(&self) {
        self.counting.store(true, Ordering::Relaxed);
    }

    /// The allocations so far, including reallocations, or `None` before counting starts.
    pub(crate) fn allocs(&self) -> Option<u64> {
        self.is_counting().then(|| self.allocs.load(Ordering::Relaxed))
    }

    /// The heap bytes requested and not yet freed, or `None` before counting starts.
    pub(crate) fn live_bytes(&self) -> Option<u64> {
        self.is_counting().then(|| self.live_bytes.load(Ordering::Relaxed).max(0) as u64)
    }

    fn is_counting(&self) -> bool {
        self.counting.load(Ordering::Relaxed)
    }

    fn count(&self, allocated: usize, freed: usize) {
        if self.is_counting() {
            self.allocs.fetch_add(1, Ordering::Relaxed);
            self.live_bytes.fetch_add(allocated as i64 - freed as i64, Ordering::Relaxed);
        }
    }
}

// SAFETY: every method passes its arguments to `System` unchanged and returns what `System`
// returns, so `System` upholds the `GlobalAlloc` contract.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds the contract of `GlobalAlloc::alloc()`.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            self.count(layout.size(), 0);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds the contract of `GlobalAlloc::alloc_zeroed()`.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            self.count(layout.size(), 0);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller upholds the contract of `GlobalAlloc::dealloc()`.
        unsafe { System.dealloc(ptr, layout) };
        if self.is_counting() {
            self.live_bytes.fetch_sub(layout.size() as i64, Ordering::Relaxed);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds the contract of `GlobalAlloc::realloc()`.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            self.count(new_size, layout.size());
        }
        new_ptr
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::GlobalAlloc;
    use std::alloc::Layout;

    use super::CountingAlloc;

    #[test]
    fn test_counts_allocations_and_the_bytes_still_allocated_once_started() {
        let counting = CountingAlloc::new();
        let layout = Layout::from_size_align(1000, 8).unwrap();

        // SAFETY: `layout` has a nonzero size, and each pointer is freed once, with the layout it
        // was last allocated with.
        unsafe {
            let ptr = counting.alloc(layout);
            assert!(!ptr.is_null());
            assert_eq!((None, None), (counting.allocs(), counting.live_bytes()));
            counting.dealloc(ptr, layout);

            counting.start();

            let ptr = counting.alloc(layout);
            assert!(!ptr.is_null());
            assert_eq!((Some(1), Some(1000)), (counting.allocs(), counting.live_bytes()));

            let ptr = counting.realloc(ptr, layout, 3000);
            assert!(!ptr.is_null());
            assert_eq!((Some(2), Some(3000)), (counting.allocs(), counting.live_bytes()));

            counting.dealloc(ptr, Layout::from_size_align(3000, 8).unwrap());
            assert_eq!((Some(2), Some(0)), (counting.allocs(), counting.live_bytes()));
        }
    }
}
