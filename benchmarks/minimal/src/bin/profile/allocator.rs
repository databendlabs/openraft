//! Count allocation events and requested bytes across all runtime threads.
//! These counts include the benchmark driver and exclude allocator bookkeeping.
//! Updating the counters adds overhead to latency and throughput measurements.

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::new();

struct CountingAllocator {
    allocations: AtomicUsize,
    allocated_bytes: AtomicUsize,
    live_bytes: AtomicUsize,
    peak_live_bytes: AtomicUsize,
    track_peak: AtomicBool,
}

impl CountingAllocator {
    const fn new() -> Self {
        Self {
            allocations: AtomicUsize::new(0),
            allocated_bytes: AtomicUsize::new(0),
            live_bytes: AtomicUsize::new(0),
            peak_live_bytes: AtomicUsize::new(0),
            track_peak: AtomicBool::new(false),
        }
    }

    fn record(&self, size: usize) {
        self.allocations.fetch_add(1, Ordering::Relaxed);
        self.allocated_bytes.fetch_add(size, Ordering::Relaxed);
        let previous = self.live_bytes.fetch_add(size, Ordering::Relaxed);
        if self.track_peak.load(Ordering::Relaxed) {
            let live = previous + size;
            self.peak_live_bytes.fetch_max(live, Ordering::Relaxed);
        }
    }

    #[cfg(any(test, feature = "realistic-profiles"))]
    fn begin_peak(&self, live: usize) {
        let active = self.track_peak.load(Ordering::Relaxed);
        assert!(!active, "allocation peak phases must not overlap");
        self.peak_live_bytes.store(live, Ordering::Relaxed);
        self.track_peak.store(true, Ordering::Relaxed);
    }

    fn end_peak(&self) -> usize {
        self.track_peak.store(false, Ordering::Relaxed);
        self.peak_live_bytes.load(Ordering::Relaxed)
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            self.record(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            self.record(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        self.live_bytes.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            self.live_bytes.fetch_sub(layout.size(), Ordering::Relaxed);
            self.record(size);
        }
        result
    }
}

pub(super) fn allocations() -> usize {
    ALLOCATOR.allocations.load(Ordering::Relaxed)
}

pub(super) fn allocated_bytes() -> usize {
    ALLOCATOR.allocated_bytes.load(Ordering::Relaxed)
}

pub(super) fn live_bytes() -> usize {
    ALLOCATOR.live_bytes.load(Ordering::Relaxed)
}

#[cfg(feature = "realistic-profiles")]
pub(super) fn begin_peak(live: usize) {
    ALLOCATOR.begin_peak(live);
}

pub(super) fn end_peak() -> usize {
    ALLOCATOR.end_peak()
}

#[cfg(test)]
mod tests {
    use std::alloc::GlobalAlloc;
    use std::alloc::Layout;
    use std::sync::atomic::Ordering;

    use super::CountingAllocator;

    #[test]
    fn phase_peak_resets_and_tracks_live_requested_bytes() {
        let allocator = CountingAllocator::new();
        let old_layout = Layout::from_size_align(4096, 8).unwrap();
        allocator.begin_peak(0);
        let old = unsafe { allocator.alloc(old_layout) };
        let allocated = !old.is_null();
        assert!(allocated);
        let old_peak = allocator.end_peak();
        assert_eq!(4096, old_peak);
        unsafe { allocator.dealloc(old, old_layout) };

        let base_layout = Layout::from_size_align(64, 8).unwrap();
        let base = unsafe { allocator.alloc(base_layout) };
        let allocated = !base.is_null();
        assert!(allocated);
        let before = allocator.live_bytes.load(Ordering::Relaxed);
        assert_eq!(64, before);
        allocator.begin_peak(before);

        let layout = Layout::from_size_align(128, 8).unwrap();
        let pointer = unsafe { allocator.alloc_zeroed(layout) };
        let allocated = !pointer.is_null();
        assert!(allocated);
        let resized = unsafe { allocator.realloc(pointer, layout, 256) };
        let allocated = !resized.is_null();
        assert!(allocated);
        let resized_layout = Layout::from_size_align(256, 8).unwrap();
        unsafe { allocator.dealloc(resized, resized_layout) };

        let peak = allocator.end_peak();
        assert_eq!(320, peak);
        let after = allocator.live_bytes.load(Ordering::Relaxed);
        assert_eq!(64, after);
        let events = allocator.allocations.load(Ordering::Relaxed);
        assert_eq!(4, events);
        let requested = allocator.allocated_bytes.load(Ordering::Relaxed);
        assert_eq!(4544, requested);

        allocator.begin_peak(after);
        let empty_peak = allocator.end_peak();
        assert_eq!(64, empty_peak);
        unsafe { allocator.dealloc(base, base_layout) };
        let live = allocator.live_bytes.load(Ordering::Relaxed);
        assert_eq!(0, live);
    }
}
