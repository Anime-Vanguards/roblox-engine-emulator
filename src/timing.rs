//! Where a command's wall time goes (`ree record --profile`, `ree run --profile`): named phases in
//! the order they ran and costs summed over the run (module compiles, source reads, bundle loads,
//! frames), printed on stderr at the end. Off unless a command begins it: every entry point is
//! then one thread-local read.

use std::{borrow::Cow, cell::RefCell, time::Instant};

struct Profile {
    started: Instant,
    /// (name, seconds, count) in first-seen order
    entries: Vec<(Cow<'static, str>, f64, u64)>,
}

thread_local! {
    static PROFILE: RefCell<Option<Profile>> = const { RefCell::new(None) };
}

/// Starts profiling this thread's command; `started` is when the process began its work.
pub fn begin(started: Instant) {
    PROFILE.with(|profile| *profile.borrow_mut() = Some(Profile { started, entries: Vec::new() }));
}

pub fn enabled() -> bool {
    PROFILE.with(|profile| profile.borrow().is_some())
}

/// Adds `seconds` (one occurrence) to `name`.
pub fn add(name: impl Into<Cow<'static, str>>, seconds: f64) {
    PROFILE.with(|profile| {
        let Ok(mut profile) = profile.try_borrow_mut() else { return };
        let Some(profile) = profile.as_mut() else { return };
        let name = name.into();
        match profile.entries.iter_mut().find(|entry| entry.0 == name) {
            Some(entry) => {
                entry.1 += seconds;
                entry.2 += 1;
            }
            None => profile.entries.push((name, seconds, 1)),
        }
    });
}

/// Adds `count` occurrences (no time) to `name`: a tally the profile prints with the phases.
pub fn tally(name: impl Into<Cow<'static, str>>, count: u64) {
    PROFILE.with(|profile| {
        let Ok(mut profile) = profile.try_borrow_mut() else { return };
        let Some(profile) = profile.as_mut() else { return };
        let name = name.into();
        match profile.entries.iter_mut().find(|entry| entry.0 == name) {
            Some(entry) => entry.2 += count,
            None => profile.entries.push((name, 0.0, count)),
        }
    });
}

/// Runs `f`, adding its wall time to `name` when profiling.
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    add(name, start.elapsed().as_secs_f64());
    result
}

/// A timer started now when profiling ([`Timer::stop`] adds it).
pub struct Timer(Option<(&'static str, Instant)>);

pub fn start(name: &'static str) -> Timer {
    Timer(enabled().then(|| (name, Instant::now())))
}

impl Timer {
    pub fn stop(self) {
        if let Some((name, start)) = self.0 {
            add(name, start.elapsed().as_secs_f64());
        }
    }
}

/// The profile so far as lines (`name  ms  [count]`), with the total since [`begin`]; ends it.
pub fn finish() -> Option<String> {
    let profile = PROFILE.with(|profile| profile.borrow_mut().take())?;
    let total = profile.started.elapsed().as_secs_f64();
    let mut lines = vec![format!("profile: {:.1} ms", total * 1000.0)];
    for (name, seconds, count) in &profile.entries {
        let count = if *count > 1 { format!("  x{count}") } else { String::new() };
        lines.push(format!("  {name:<28} {:>9.1} ms{count}", seconds * 1000.0));
    }
    Some(lines.join("\n"))
}
