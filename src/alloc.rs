//! Long-lived allocations: what a process keeps across runs (a `ree serve` worker's converted
//! bundles, compiled modules and parsed snapshots) is allocated in a heap of its own while
//! [`long_lived`] runs, when the program's allocator honours it (`ree`'s does: src/main.rs).
//! The service's other requests of the allocator go through [`Controls`], which the program
//! installs: the library itself calls no allocator's functions, so a program links it with any
//! global allocator.
//!
//! The data a run keeps is made in the middle of that run, among the run's own short-lived blocks
//! (a decoded bundle, the compiler's trees, the engine): once those are freed, each page that also
//! holds a kept block stays committed though nearly empty. In a heap of their own, kept blocks pack
//! a few pages and every page of the run's heap is handed back when the run ends.

use std::{
    cell::Cell,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
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

/// What `ree serve` asks of the program's allocator: hand a thread's freed pages back to the system when a worker is idle,
/// and keep freed pages committed longer while jobs come in bursts. `ree`'s binary installs mimalloc's (src/main.rs); a
/// program that installs none, with whatever global allocator it has, gets neither, and nothing else changes.
#[derive(Clone, Copy, Debug)]
pub struct Controls {
    /// Return this thread's freed pages to the system now (mimalloc's `mi_collect(true)`)
    pub collect: fn(),
    /// Keep freed pages committed this many milliseconds before returning them (mimalloc's purge delay option)
    pub set_purge_delay: fn(milliseconds: i64),
}

static CONTROLS: OnceLock<Controls> = OnceLock::new();

/// Installs the program's allocator controls, once (a second call is refused and gets its controls back).
pub fn set_controls(controls: Controls) -> Result<(), Controls> {
    CONTROLS.set(controls)
}

/// Hands this thread's freed pages back to the system, if the program installed [`Controls`].
pub fn collect() {
    if let Some(controls) = CONTROLS.get() {
        (controls.collect)();
    }
}

/// Sets how long freed pages stay committed, if the program installed [`Controls`].
pub fn set_purge_delay(milliseconds: i64) {
    if let Some(controls) = CONTROLS.get() {
        (controls.set_purge_delay)(milliseconds);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use super::*;

    static COLLECTS: AtomicUsize = AtomicUsize::new(0);
    static PURGE_DELAY: AtomicI64 = AtomicI64::new(0);

    /// The service's requests reach the controls a program installed (and only those: a second install is refused).
    #[test]
    fn the_services_allocator_requests_go_to_the_installed_controls() {
        let controls = Controls {
            collect: || {
                COLLECTS.fetch_add(1, Ordering::SeqCst);
            },
            set_purge_delay: |milliseconds| PURGE_DELAY.store(milliseconds, Ordering::SeqCst),
        };
        set_controls(controls).unwrap();
        collect();
        collect();
        set_purge_delay(1500);
        assert_eq!((COLLECTS.load(Ordering::SeqCst), PURGE_DELAY.load(Ordering::SeqCst)), (2, 1500));
        assert!(set_controls(Controls { collect: || {}, set_purge_delay: |_| {} }).is_err());
        collect();
        assert_eq!(COLLECTS.load(Ordering::SeqCst), 3);
    }
}
