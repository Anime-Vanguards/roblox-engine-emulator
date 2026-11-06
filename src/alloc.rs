//! Long-lived allocations: what a process keeps across runs (a `ree serve` worker's converted
//! bundles, compiled modules and parsed snapshots) is allocated in a heap of its own while
//! [`long_lived`] runs, when the program's allocator honours it (`ree`'s does: src/main.rs).
//!
//! The data a run keeps is made in the middle of that run, among the run's own short-lived blocks
//! (a decoded bundle, the compiler's trees, the engine): once those are freed, each page that also
//! holds a kept block stays committed though nearly empty. In a heap of their own, kept blocks pack
//! a few pages and every page of the run's heap is handed back when the run ends.

use std::{
    cell::Cell,
    sync::atomic::{AtomicUsize, Ordering},
};

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// How many threads are in [`long_lived`] now: while none is (nearly always), an allocation is told so
/// without reading its thread's depth.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// Runs `f` with this thread's allocations going to the long-lived heap.
pub fn long_lived<T>(f: impl FnOnce() -> T) -> T {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            DEPTH.with(|depth| depth.set(depth.get() - 1));
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _restore = Restore;
    f()
}

/// Whether this thread's allocations go to the long-lived heap now (for a global allocator).
#[inline]
pub fn in_long_lived() -> bool {
    ACTIVE.load(Ordering::Relaxed) != 0 && DEPTH.try_with(|depth| depth.get() > 0).unwrap_or(false)
}
