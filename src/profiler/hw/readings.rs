//! Splits a thread's slices (protocol.rs: each stretch it ran on a CPU, with what the counters counted meanwhile) at the
//! readings it took. A reading is a moment the thread made itself switch out (client.rs [`super::client::Reader`]): the
//! slice that ends right after it ends at the reading, so the counts between two readings are exactly the slices that
//! ended between their switches. Shared with tools/hw-counters (its probe), so it uses nothing but std.

use super::protocol::{MAX_COUNTERS, Slice};

pub type Counts = [u64; MAX_COUNTERS];

pub fn add(total: &mut Counts, counts: &Counts) {
    for (total, count) in total.iter_mut().zip(counts) {
        *total = total.wrapping_add(*count);
    }
}

/// What a thread counted between its readings
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
    /// One entry per reading: `windows[k]` is what the thread counted from reading k-1's switch to reading k's
    /// (`windows[0]`: before the first reading)
    pub windows: Vec<Counts>,
    /// What it counted after the last reading's switch
    pub after: Counts,
    /// Readings whose switch came within the tolerance (a reading that found no switch of its own shares the next one's)
    pub matched: usize,
}

/// Splits `slices` (any order) at `readings` (increasing, in the slices' clock). A reading's switch is the first slice
/// end at or after it; it matched when that came within `tolerance` ticks.
pub fn split(readings: &[i64], slices: &[Slice], tolerance: i64) -> Split {
    let mut ends: Vec<(i64, usize)> = slices.iter().enumerate().map(|(index, slice)| (slice.end, index)).collect();
    ends.sort_unstable();
    // each reading's switch: the first slice end at or after it
    let mut boundaries = Vec::with_capacity(readings.len());
    let mut matched = 0;
    let mut next = 0;
    for &reading in readings {
        while next < ends.len() && ends[next].0 < reading {
            next += 1;
        }
        let boundary = ends.get(next).map_or(i64::MAX, |end| end.0);
        if boundary != i64::MAX && boundary - reading <= tolerance {
            matched += 1;
        }
        boundaries.push(boundary);
    }
    let mut split = Split { windows: vec![[0; MAX_COUNTERS]; readings.len()], after: [0; MAX_COUNTERS], matched };
    for &(end, index) in &ends {
        // the first reading whose switch is at or after the slice's end; when later readings share that switch (theirs
        // was not seen), the last of them: the slice is known to have ended before that one's reading, not the first's
        let mut window = boundaries.partition_point(|&boundary| boundary < end);
        if let Some(&boundary) = boundaries.get(window) {
            window = boundaries.partition_point(|&other| other <= boundary) - 1;
        }
        match split.windows.get_mut(window) {
            Some(counts) => add(counts, &slices[index].counts),
            None => add(&mut split.after, &slices[index].counts),
        }
    }
    split
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(start: i64, end: i64, count: u64) -> Slice {
        let mut counts = [0; MAX_COUNTERS];
        counts[0] = count;
        counts[1] = count * 10;
        Slice { cpu: 0, start, end, counts }
    }

    /// A slice that ends at a reading's switch is the window's that reading ends; one the thread was preempted out of
    /// before the reading is the same window's; what came before the first reading and after the last stay apart.
    #[test]
    fn slices_split_at_their_readings_switches() {
        let readings = [100, 200, 300];
        // (out of order: the service gives them by CPU)
        let slices = [slice(205, 301, 4), slice(50, 101, 1), slice(310, 400, 5), slice(155, 201, 3), slice(105, 150, 2)];
        let split = split(&readings, &slices, 5);
        assert_eq!(split.windows.len(), 3);
        assert_eq!(split.windows[0][0], 1, "the slice the first reading ended");
        assert_eq!(split.windows[1][0], 2 + 3, "preempted once, then the reading's own switch");
        assert_eq!(split.windows[2][0], 4);
        assert_eq!(split.windows[2][1], 40, "every counter splits the same way");
        assert_eq!(split.after[0], 5);
        assert_eq!(split.matched, 3);
    }

    /// A reading that found no switch of its own (its events lost) leaves its window empty and the next one takes both;
    /// nothing is lost or counted twice, and the unmatched reading is told apart.
    #[test]
    fn a_reading_without_its_switch_shares_the_next_ones() {
        let readings = [100, 200, 300];
        let slices = [slice(0, 101, 1), slice(105, 301, 7)];
        let split = split(&readings, &slices, 5);
        assert_eq!(split.windows.iter().map(|w| w[0]).collect::<Vec<_>>(), [1, 0, 7]);
        assert_eq!(split.matched, 2, "the reading at 200 had no switch within 5 ticks");
        let total: u64 = split.windows.iter().map(|w| w[0]).sum::<u64>() + split.after[0];
        assert_eq!(total, 8);
    }

    /// No slices: every window is empty; no readings: everything is after
    #[test]
    fn empty_inputs() {
        let none = split(&[1, 2], &[], 5);
        assert_eq!((none.windows.len(), none.matched), (2, 0));
        let all_after = split(&[], &[slice(0, 10, 3)], 5);
        assert_eq!(all_after.after[0], 3);
    }
}
