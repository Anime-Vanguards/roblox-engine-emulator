//! Type-erased stable sort for cold paths (reporting, setup, CLI, tests).
//!
//! Rust's `slice::sort_by` / `slice::sort_by_key` monomorphizes the entire driftsort
//! implementation (~4,000 IR lines) for every `(T, comparator)` pair. Cold callers route
//! through [`by`], [`by_key`], and [`ord`], which instantiate driftsort once (for `usize`)
//! and permute the slice in place via a single non-generic byte-swap routine.

use std::cmp::Ordering;

/// Sorts `slice` stably using `cmp` without monomorphizing `slice::sort_by` per `(T, F)`.
pub(crate) fn by<T>(slice: &mut [T], mut cmp: impl FnMut(&T, &T) -> Ordering) {
    if slice.len() <= 1 {
        return;
    }
    let order = sort_indices(slice.len(), &mut |a, b| cmp(&slice[a], &slice[b]));
    // SAFETY: `order` is a permutation of `0..slice.len()` and `slice` holds `order.len()`
    // contiguous initialized values of size `size_of::<T>()`, which are bitwise movable.
    unsafe {
        apply_permutation_raw(slice.as_mut_ptr().cast::<u8>(), std::mem::size_of::<T>(), order);
    }
}

/// Sorts `slice` stably by `key` without monomorphizing `slice::sort_by_key` per `(T, F)`.
pub(crate) fn by_key<T, K: Ord>(slice: &mut [T], mut key: impl FnMut(&T) -> K) {
    if slice.len() <= 1 {
        return;
    }
    let order = sort_indices(slice.len(), &mut |a, b| key(&slice[a]).cmp(&key(&slice[b])));
    // SAFETY: same as [`by`].
    unsafe {
        apply_permutation_raw(slice.as_mut_ptr().cast::<u8>(), std::mem::size_of::<T>(), order);
    }
}

/// Sorts `slice` stably in ascending order without monomorphizing `slice::sort` per `T`.
pub(crate) fn ord<T: Ord>(slice: &mut [T]) {
    by(slice, Ord::cmp);
}

#[inline(never)]
fn sort_indices(len: usize, cmp: &mut dyn FnMut(usize, usize) -> Ordering) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    order.sort_by(|&a, &b| cmp(a, b));
    order
}

/// Applies `order` in place in O(n) swaps so `slot[i]` receives the element originally at
/// `slot[order[i]]`.
#[inline(never)]
unsafe fn apply_permutation_raw(base: *mut u8, elem_size: usize, mut order: Vec<usize>) {
    if elem_size == 0 {
        return;
    }
    for start in 0..order.len() {
        let mut cur = start;
        let mut next = order[cur];
        if next == cur {
            continue;
        }
        while next != start {
            // SAFETY: `cur != next` inside a cycle of a valid permutation, so the two element
            // slots within `base` are non-overlapping and aligned for `T`.
            unsafe {
                std::ptr::swap_nonoverlapping(base.add(cur * elem_size), base.add(next * elem_size), elem_size);
            }
            order[cur] = cur;
            cur = next;
            next = order[cur];
        }
        order[cur] = cur;
    }
}

/// Type-erased channels sharing one `mpsc::channel<Box<dyn Any + Send>>` monomorphization.
pub(crate) mod channel {
    use std::{
        any::Any,
        marker::PhantomData,
        sync::mpsc::{self, RecvError, RecvTimeoutError, SendError, TryRecvError},
        time::Duration,
    };

    pub(crate) struct Sender<T: Send + 'static> {
        inner: mpsc::Sender<Box<dyn Any + Send>>,
        _marker: PhantomData<fn(T)>,
    }

    pub(crate) struct Receiver<T: Send + 'static> {
        inner: mpsc::Receiver<Box<dyn Any + Send>>,
        _marker: PhantomData<T>,
    }

    pub(crate) fn channel<T: Send + 'static>() -> (Sender<T>, Receiver<T>) {
        let (tx, rx) = raw_channel();
        (Sender { inner: tx, _marker: PhantomData }, Receiver { inner: rx, _marker: PhantomData })
    }

    #[inline(never)]
    fn raw_channel() -> (mpsc::Sender<Box<dyn Any + Send>>, mpsc::Receiver<Box<dyn Any + Send>>) {
        mpsc::channel()
    }

    #[inline(never)]
    fn raw_send(tx: &mpsc::Sender<Box<dyn Any + Send>>, value: Box<dyn Any + Send>) -> Result<(), Box<dyn Any + Send>> {
        tx.send(value).map_err(|err| err.0)
    }

    #[inline(never)]
    fn raw_recv(rx: &mpsc::Receiver<Box<dyn Any + Send>>) -> Result<Box<dyn Any + Send>, RecvError> {
        rx.recv()
    }

    #[inline(never)]
    fn raw_try_recv(rx: &mpsc::Receiver<Box<dyn Any + Send>>) -> Result<Box<dyn Any + Send>, TryRecvError> {
        rx.try_recv()
    }

    #[inline(never)]
    fn raw_recv_timeout(rx: &mpsc::Receiver<Box<dyn Any + Send>>, timeout: Duration) -> Result<Box<dyn Any + Send>, RecvTimeoutError> {
        rx.recv_timeout(timeout)
    }

    impl<T: Send + 'static> Sender<T> {
        pub(crate) fn send(&self, value: T) -> Result<(), SendError<T>> {
            raw_send(&self.inner, Box::new(value)).map_err(|boxed| SendError(*boxed.downcast::<T>().expect("channel type")))
        }
    }

    impl<T: Send + 'static> Receiver<T> {
        pub(crate) fn recv(&self) -> Result<T, RecvError> {
            raw_recv(&self.inner).map(|boxed| *boxed.downcast::<T>().expect("channel type"))
        }

        pub(crate) fn try_recv(&self) -> Result<T, TryRecvError> {
            raw_try_recv(&self.inner).map(|boxed| *boxed.downcast::<T>().expect("channel type"))
        }

        pub(crate) fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
            raw_recv_timeout(&self.inner, timeout).map(|boxed| *boxed.downcast::<T>().expect("channel type"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_handles_edge_cases_stability_drop_types_and_zst() {
        let mut empty: Vec<i32> = Vec::new();
        ord(&mut empty);
        assert!(empty.is_empty());

        let mut single = vec![42];
        ord(&mut single);
        assert_eq!(single, [42]);

        let mut pairs: Vec<(i32, String)> = vec![
            (3, "a".into()),
            (1, "b".into()),
            (2, "c".into()),
            (1, "d".into()),
            (3, "e".into()),
            (2, "f".into()),
            (0, "g".into()),
            (1, "h".into()),
        ];
        by_key(&mut pairs, |(k, _)| *k);
        assert_eq!(
            pairs,
            vec![
                (0, "g".into()),
                (1, "b".into()),
                (1, "d".into()),
                (1, "h".into()),
                (2, "c".into()),
                (2, "f".into()),
                (3, "a".into()),
                (3, "e".into()),
            ]
        );

        let mut floats = vec![3.0_f64, -1.0, 2.5, 0.0, -0.0, 2.5];
        by(&mut floats, f64::total_cmp);
        assert_eq!(floats.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), [-1.0_f64, -0.0, 0.0, 2.5, 2.5, 3.0].iter().map(|v| v.to_bits()).collect::<Vec<_>>());

        let mut zsts = vec![(), (), ()];
        ord(&mut zsts);
        assert_eq!(zsts.len(), 3);
    }
}
