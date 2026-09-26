//! Count allocation events and requested bytes across all runtime threads.
//! These counts include the benchmark driver and exclude allocator bookkeeping.
//! Updating the counters adds overhead to latency and throughput measurements.

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct CountingAllocator;

fn record(size: usize) {
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    ALLOCATED_BYTES.fetch_add(size, Ordering::Relaxed);
    LIVE_BYTES.fetch_add(size, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
            record(size);
        }
        result
    }
}

pub(super) fn allocations() -> usize {
    ALLOCATIONS.load(Ordering::Relaxed)
}

pub(super) fn allocated_bytes() -> usize {
    ALLOCATED_BYTES.load(Ordering::Relaxed)
}

pub(super) fn live_bytes() -> usize {
    LIVE_BYTES.load(Ordering::Relaxed)
}
