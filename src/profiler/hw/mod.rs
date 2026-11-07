//! Hardware counters (`--profile hw`): what the CPU counted while the run thread ran (instructions, cycles, the data
//! caches' misses, branch mispredictions), per run, thread, phase, frame and Luau stack.
//!
//! A process on Windows cannot read the CPU's counters itself (`rdpmc` faults in user mode), and a kernel trace session,
//! which can record them, needs a process with the system's profiling right, which an account's ordinary processes never
//! hold (UAC takes it out of their token). The counter service (tools/hw-counters, installed once from an elevated
//! shell) runs such a session for the asking process only: the CPU's counters ride on every context switch, and the
//! service hands back each of the process's threads' slices (protocol.rs), never another process's.
//!
//! The run thread takes a reading (client.rs [`client::Reader`]: it makes itself switch out) at the profile's start and
//! end, when the VM is installed, at each frame's start and end, at each `Context.ProfilePhase` mark, around each of the
//! engine's own steps (the Animator, joints, physics) and at the VM's safepoints when the 1 ms host-time ticker has
//! moved. The counts between two readings are exact (readings.rs). A safepoint's reading books what was counted since
//! the last sample to the Luau stack it finds, as host time books its ticks: a line's counts land on its next
//! safepoint, the engine's own steps get frames of their own, and what ran before the VM did (the mount) or after a
//! phase's last sample is booked to a frame of its own, so the stacks add up to the run. A reading costs a few
//! thousand cycles of the run thread's own, counted with the rest: the profile measures that cost at its start and
//! says it.
//!
//! `REE_HW_COUNTERS=mock` counts without the service (tests): each reading takes the thread's cycle count
//! (QueryThreadCycleTime) and the other counters are fixed shares of it. `REE_HW_COUNTERS=console` reads from
//! `ree-hw-counters serve` run in a console.

pub mod client;
pub mod protocol;
pub mod readings;

use rustc_hash::FxHashMap;

use super::memory;
use protocol::{MAX_COUNTERS, Slice};
use readings::Counts;

/// What a reading marks
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    /// Back-to-back readings before the start: what a reading itself costs
    Calibrate,
    Start,
    /// The VM was installed: what ran before it is the engine's start (the mount)
    Install,
    FrameBegin,
    /// A frame ended (its number)
    FrameEnd(u64),
    /// A phase mark (the reading's phase is the new one)
    Mark,
    /// A safepoint's sample (the stack's id)
    Sample(u32),
    /// One of the engine's own steps begins (its frame's function id), and ends
    StepBegin(i32),
    StepEnd,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// In the service's clock (QueryPerformanceCounter ticks)
    pub at: i64,
    pub what: What,
    /// The phase in effect from this reading on
    pub phase: u32,
}

/// Where the counts come from
enum Source {
    Service { client: client::Client, reader: client::Reader },
    /// The thread's cycle count at each reading
    Mock { cycles: Vec<u64> },
}

/// How many readings measure a reading's own cost
const CALIBRATION_READINGS: usize = 16;

/// A profile's hardware counters while it runs
pub struct Session {
    source: Option<Source>,
    /// The counters' names (the profile's) and the profile sources the service counts for them
    pub names: Vec<String>,
    sources: Vec<String>,
    error: Option<String>,
    notes: Vec<String>,
    readings: Vec<Reading>,
    /// The samples' stacks
    pub stacks: memory::Stacks,
    /// The ticker's ticks at the last sample
    pub seen: u64,
    thread: u32,
    frequency: i64,
    in_step: bool,
    ended: bool,
}

#[cfg(test)]
thread_local! {
    /// A test's `REE_HW_COUNTERS` for this thread (the variable is the process's: tests run side by side)
    pub static TEST_MODE: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// `REE_HW_COUNTERS`: where the counts come from (the installed service unless it names `mock` or `console`)
fn mode() -> String {
    #[cfg(test)]
    if let Some(mode) = TEST_MODE.with(std::cell::Cell::get) {
        return mode.into();
    }
    std::env::var("REE_HW_COUNTERS").unwrap_or_default().trim().to_ascii_lowercase()
}

/// The names of a `--profile-counters` value: at most [`MAX_COUNTERS`], none twice
pub fn parse_counters(text: &str) -> Result<Vec<String>, String> {
    let names: Vec<String> = text.split(',').map(str::trim).filter(|name| !name.is_empty()).map(str::to_string).collect();
    if names.is_empty() {
        return Err("--profile-counters: no counter named".into());
    }
    if names.len() > MAX_COUNTERS {
        return Err(format!("--profile-counters: {} counters, at most {MAX_COUNTERS} (the CPU counts six at once)", names.len()));
    }
    for (index, name) in names.iter().enumerate() {
        if names[..index].contains(name) {
            return Err(format!("--profile-counters: {name} twice"));
        }
    }
    Ok(names)
}

impl Session {
    /// A session that counts nothing yet
    fn new(names: &[String]) -> Session {
        Session {
            source: None,
            names: names.to_vec(),
            sources: names.iter().map(|name| protocol::source_of(name).to_string()).collect(),
            error: None,
            notes: Vec::new(),
            readings: Vec::new(),
            stacks: memory::Stacks::default(),
            seen: 0,
            thread: client::current_thread_id(),
            frequency: client::qpc_frequency(),
            in_step: false,
            ended: false,
        }
    }

    /// Starts counting `names` on this thread (the run thread); what failed is reported, never fatal
    pub fn begin(names: &[String]) -> Session {
        let mut session = Session::new(names);
        let started = match mode().as_str() {
            "mock" => Ok(Source::Mock { cycles: Vec::new() }),
            mode => {
                let endpoint = if mode == "console" { client::Endpoint::Console } else { client::Endpoint::Service };
                // (the reader's thread first: the service counts every thread the process has from its start on)
                client::Reader::new().and_then(|reader| {
                    let mut client = client::Client::connect(endpoint)?;
                    let started = client.start(&session.sources, &[session.thread])?;
                    session.sources = started.sources;
                    session.notes = started.notes;
                    session.frequency = (started.qpc_frequency as i64).max(1);
                    Ok(Source::Service { client, reader })
                })
            }
        };
        match started {
            Ok(source) => session.source = Some(source),
            Err(error) => session.error = Some(error),
        }
        for _ in 0..CALIBRATION_READINGS {
            session.read(What::Calibrate, 0);
        }
        session.read(What::Start, 0);
        session
    }

    /// Whether counting runs
    pub fn counting(&self) -> bool {
        self.source.is_some() && !self.ended
    }

    /// Why counting did not start
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Takes a reading (nothing when counting did not start or has ended)
    #[inline]
    pub fn read(&mut self, what: What, phase: u32) {
        if self.ended {
            return;
        }
        let at = match self.source.as_mut() {
            Some(Source::Service { reader, .. }) => reader.read(),
            Some(Source::Mock { cycles }) => {
                cycles.push(super::cpu::thread_cycles());
                // (the mock's slices end a tick after their reading: each reading gets a tick of its own)
                let last = self.readings.last().map_or(i64::MIN, |reading| reading.at);
                client::qpc().max(last.saturating_add(2))
            }
            None => return,
        };
        self.readings.push(Reading { at, what, phase });
    }

    /// One of the engine's own steps begins (`Some(frame id)`) or ends
    pub fn step(&mut self, step: Option<i32>, phase: u32) {
        match step {
            Some(id) => {
                self.in_step = true;
                self.read(What::StepBegin(id), phase);
            }
            None if self.in_step => {
                self.in_step = false;
                self.read(What::StepEnd, phase);
            }
            None => {}
        }
    }

    /// The profile's last reading (once)
    pub fn end(&mut self, phase: u32) {
        self.read(What::End, phase);
        self.ended = true;
    }

    /// Stops counting and books what was counted: the report
    pub fn finish(mut self, phase: u32) -> Report {
        if !self.ended {
            self.end(phase);
        }
        let mut report = Report {
            source: match self.source {
                Some(Source::Mock { .. }) => "mock",
                Some(Source::Service { .. }) => "service",
                None => "none",
            },
            names: self.names.clone(),
            sources: self.sources.clone(),
            error: self.error.take(),
            notes: std::mem::take(&mut self.notes),
            readings: self.readings.len(),
            // (the samples' stacks: the booking adds its own frames to them)
            stacks: std::mem::take(&mut self.stacks),
            ..Report::default()
        };
        let results = match self.source.take() {
            Some(Source::Service { client, reader }) => {
                drop(reader);
                client.stop()
            }
            Some(Source::Mock { cycles }) => Ok(mock_results(&self.names, &self.readings, &cycles, self.thread)),
            None => return report,
        };
        match results {
            Ok(results) => book(&mut report, &self, results),
            Err(error) => report.error = Some(format!("the counter service's results: {error}")),
        }
        report
    }
}

/// The mock's counters for a window of `cycles` of the thread's cycles: each a fixed share of them
pub fn mock_count(name: &str, cycles: u64) -> u64 {
    match name {
        "cycles" => cycles,
        "instructions" => cycles * 2,
        "l1d-accesses" => cycles / 2,
        "l1d-misses" => cycles / 50,
        "l2-misses" => cycles / 400,
        "llc-misses" => cycles / 3200,
        "branches" => cycles / 8,
        "branch-misses" => cycles / 800,
        _ => cycles / 100,
    }
}

/// The mock's results: the run thread's slices end a tick after each reading, as a reading's switch would
fn mock_results(names: &[String], readings: &[Reading], cycles: &[u64], thread: u32) -> protocol::Results {
    let mut slices = Vec::with_capacity(readings.len());
    for index in 1..readings.len().min(cycles.len()) {
        let spent = cycles[index].saturating_sub(cycles[index - 1]);
        let mut counts = [0u64; MAX_COUNTERS];
        for (count, name) in counts.iter_mut().zip(names) {
            *count = mock_count(name, spent);
        }
        slices.push(Slice { cpu: 0, start: readings[index - 1].at + 1, end: readings[index].at + 1, counts });
    }
    protocol::Results { counters: names.len() as u8, threads: vec![protocol::Thread { id: thread, registered: true, slices }], ..protocol::Results::default() }
}

/// A thread's counts over the profile
#[derive(Clone, Debug, Default)]
pub struct ThreadCounts {
    pub id: u32,
    pub name: Option<String>,
    pub run_thread: bool,
    pub slices: usize,
    pub counts: Counts,
}

/// A frame's counts, from its start to its end
#[derive(Clone, Debug, Default)]
pub struct FrameCounts {
    pub frame: u64,
    pub phase: u32,
    pub host_ms: f64,
    pub counts: Counts,
}

/// What the counters counted, booked
#[derive(Default)]
pub struct Report {
    /// `service`, `mock`, or `none` when counting did not start
    pub source: &'static str,
    pub names: Vec<String>,
    pub sources: Vec<String>,
    /// Why there are no counts
    pub error: Option<String>,
    pub notes: Vec<String>,
    /// The run thread from the start reading to the end one, and the host time between them
    pub run: Counts,
    pub run_seconds: f64,
    /// A reading's own counts (the median of back-to-back ones)
    pub per_reading: Option<Counts>,
    pub readings: usize,
    /// Readings the run thread switched out for (their counts split exactly there)
    pub matched: usize,
    pub samples: usize,
    pub threads: Vec<ThreadCounts>,
    /// By phase id
    pub phases: Vec<(u32, Counts)>,
    pub frames: Vec<FrameCounts>,
    /// (phase, stack) -> counts: every count of the run, booked to the stack of the sample that followed it
    pub by_stack: FxHashMap<(u32, u32), Counts>,
    pub stacks: memory::Stacks,
    pub switches: u64,
    pub without_counters: u64,
    pub lost_events: u32,
    pub lost_buffers: u32,
}

/// The engine's start before the VM ran, and what ran after a phase's last sample: frames of their own
pub const BEFORE_VM: &str = "ree before the VM: the mount and the engine's start";
pub const NO_SAMPLE: &str = "ree outside Luau: after the phase's last sample";

/// Books the service's slices to the run, its threads, phases, frames and stacks
fn book(report: &mut Report, session: &Session, results: protocol::Results) {
    let readings = &session.readings;
    report.switches = results.switches;
    report.without_counters = results.without_counters;
    report.lost_events = results.lost_events;
    report.lost_buffers = results.lost_buffers;
    report.notes.extend(results.notes);
    let Some(run) = results.threads.iter().find(|thread| thread.id == session.thread) else {
        report.error = Some("the counter service saw no slice of the run thread".into());
        return;
    };
    let at: Vec<i64> = readings.iter().map(|reading| reading.at).collect();
    // (a reading's switch comes microseconds after it; a millisecond is far)
    let split = readings::split(&at, &run.slices, session.frequency / 1000);
    report.matched = split.matched;
    let Some(start) = readings.iter().position(|reading| reading.what == What::Start) else { return };
    let end = readings.iter().rposition(|reading| reading.what == What::End).unwrap_or(readings.len().saturating_sub(1)).max(start);
    report.run_seconds = (readings[end].at - readings[start].at) as f64 / session.frequency as f64;
    // a reading's own cost: the windows between the back-to-back readings
    let calibration: Vec<&Counts> = (1..start).map(|index| &split.windows[index]).collect();
    if !calibration.is_empty() {
        let mut median = [0u64; MAX_COUNTERS];
        for (counter, value) in median.iter_mut().enumerate() {
            let mut values: Vec<u64> = calibration.iter().map(|counts| counts[counter]).collect();
            values.sort_unstable();
            *value = values[values.len() / 2];
        }
        report.per_reading = Some(median);
    }
    // the run's other threads: what ended between the run thread's start and end switches
    let (from, to) = (readings[start].at, readings[end].at);
    for thread in &results.threads {
        let mut counts = [0u64; MAX_COUNTERS];
        let mut slices = 0;
        if thread.id == session.thread {
            for window in &split.windows[start + 1..=end] {
                readings::add(&mut counts, window);
            }
            slices = run.slices.iter().filter(|slice| slice.end > from && slice.end <= to + session.frequency / 1000).count();
        } else {
            for slice in thread.slices.iter().filter(|slice| slice.end > from && slice.end <= to) {
                readings::add(&mut counts, &slice.counts);
                slices += 1;
            }
        }
        report.threads.push(ThreadCounts { id: thread.id, name: client::thread_name(thread.id), run_thread: thread.id == session.thread, slices, counts });
    }
    report.threads.sort_by(|a, b| b.run_thread.cmp(&a.run_thread).then(b.counts[0].cmp(&a.counts[0])));
    if let Some(run_thread) = report.threads.iter().find(|thread| thread.run_thread) {
        report.run = run_thread.counts;
    }
    // phases, frames and stacks: each window between the start and the end readings, in order
    let mut phases: FxHashMap<u32, Counts> = FxHashMap::default();
    let mut frame: Option<(i64, u32, Counts)> = None;
    let mut pending = [0u64; MAX_COUNTERS];
    let mut step: Option<(i32, Counts)> = None;
    let mut stacks = std::mem::take(&mut report.stacks);
    let book_stack = |report: &mut Report, phase: u32, stack: u32, counts: &mut Counts| {
        if counts.iter().any(|&count| count > 0) {
            readings::add(report.by_stack.entry((phase, stack)).or_insert([0; MAX_COUNTERS]), counts);
        }
        *counts = [0; MAX_COUNTERS];
    };
    let mut synthetic: FxHashMap<i32, u32> = FxHashMap::default();
    let mut frame_of = |stacks: &mut memory::Stacks, id: i32| *synthetic.entry(id).or_insert_with(|| stacks.intern(&[(id, -1)]));
    for index in start + 1..=end {
        let (opened, here) = (&readings[index - 1], &readings[index]);
        let window = &split.windows[index];
        let phase = opened.phase;
        readings::add(phases.entry(phase).or_insert([0; MAX_COUNTERS]), window);
        if opened.what == What::FrameBegin {
            frame = Some((opened.at, opened.phase, [0; MAX_COUNTERS]));
        }
        if let Some((_, _, counts)) = frame.as_mut() {
            readings::add(counts, window);
        }
        if let What::FrameEnd(number) = here.what
            && let Some((began, phase, counts)) = frame.take()
        {
            report.frames.push(FrameCounts { frame: number, phase, host_ms: (here.at - began) as f64 * 1e3 / session.frequency as f64, counts });
        }
        // the stacks: inside an engine step its counts are the step's until a sample in it takes them
        if let What::StepBegin(id) = opened.what {
            if let Some((outer, mut counts)) = step.take() {
                let stack = frame_of(&mut stacks, outer);
                book_stack(report, phase, stack, &mut counts);
            }
            step = Some((id, [0; MAX_COUNTERS]));
        }
        match step.as_mut() {
            Some((_, counts)) => readings::add(counts, window),
            None => readings::add(&mut pending, window),
        }
        match here.what {
            What::Sample(stack) => {
                report.samples += 1;
                match step.as_mut() {
                    Some((_, counts)) => book_stack(report, phase, stack, counts),
                    None => book_stack(report, phase, stack, &mut pending),
                }
            }
            What::StepEnd => {
                if let Some((id, mut counts)) = step.take() {
                    let stack = frame_of(&mut stacks, id);
                    book_stack(report, phase, stack, &mut counts);
                }
            }
            What::Install => {
                let stack = frame_of(&mut stacks, super::report::outside_function(BEFORE_VM));
                book_stack(report, phase, stack, &mut pending);
            }
            What::Mark | What::End => {
                if let Some((id, mut counts)) = step.take() {
                    let stack = frame_of(&mut stacks, id);
                    book_stack(report, phase, stack, &mut counts);
                }
                let stack = frame_of(&mut stacks, super::report::outside_function(NO_SAMPLE));
                book_stack(report, phase, stack, &mut pending);
            }
            _ => {}
        }
    }
    report.stacks = stacks;
    let mut phases: Vec<(u32, Counts)> = phases.into_iter().collect();
    phases.sort_by_key(|(phase, _)| *phase);
    report.phases = phases;
}

/// The derived figures of a set of counts: IPC, misses per thousand instructions, miss rates (label, value, how it
/// prints)
pub fn rates(names: &[String], counts: &Counts) -> Vec<(String, f64, String)> {
    let get = |name: &str| names.iter().position(|known| known == name).map(|index| counts[index] as f64);
    let mut rates = Vec::new();
    if let (Some(instructions), Some(cycles)) = (get("instructions"), get("cycles"))
        && cycles > 0.0
    {
        rates.push(("IPC".to_string(), instructions / cycles, format!("{:.2}", instructions / cycles)));
    }
    let ratio = |rates: &mut Vec<(String, f64, String)>, label: &str, part: Option<f64>, whole: Option<f64>| {
        if let (Some(part), Some(whole)) = (part, whole)
            && whole > 0.0
        {
            rates.push((label.to_string(), part / whole, format!("{:.1} %", part / whole * 100.0)));
        }
    };
    ratio(&mut rates, "l1d miss rate", get("l1d-misses"), get("l1d-accesses"));
    ratio(&mut rates, "l2 miss rate (of l1d misses)", get("l2-misses"), get("l1d-misses"));
    ratio(&mut rates, "llc miss rate (of l2 misses)", get("llc-misses"), get("l2-misses"));
    ratio(&mut rates, "branch miss rate", get("branch-misses"), get("branches"));
    if let Some(instructions) = get("instructions").filter(|&instructions| instructions > 0.0) {
        for (index, name) in names.iter().enumerate() {
            if name.ends_with("-misses") {
                let per = counts[index] as f64 / instructions * 1000.0;
                rates.push((format!("{name} per 1000 instructions"), per, format!("{per:.2}")));
            }
        }
    }
    rates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(at: i64, what: What, phase: u32) -> Reading {
        Reading { at, what, phase }
    }

    fn counts(value: u64) -> Counts {
        let mut counts = [0; MAX_COUNTERS];
        counts[0] = value;
        counts
    }

    /// The bookkeeping, with slices made up: every count of the run lands once in the run, its phase, its frame and a
    /// stack; a sample takes what came since the last one; an engine step's counts are its own; what came before the
    /// VM and after a phase's last sample get frames of their own; a reading's cost is the back-to-back windows' median
    #[test]
    fn counts_are_booked_once_to_the_run_its_phases_frames_and_stacks() {
        // (no source: the test gives the readings and the slices itself)
        let mut session = Session::new(&["cycles".into()]);
        let sequence = [
            (What::Calibrate, 0),
            (What::Calibrate, 0),
            (What::Calibrate, 0),
            (What::Start, 0),
            (What::Install, 0),
            (What::Sample(7), 0),
            (What::Mark, 1),
            (What::FrameBegin, 1),
            (What::Sample(8), 1),
            (What::StepBegin(-1_000_000_000), 1),
            (What::StepEnd, 1),
            (What::Sample(8), 1),
            (What::FrameEnd(1), 1),
            (What::End, 1),
        ];
        session.readings = sequence.iter().enumerate().map(|(index, &(what, phase))| reading(100 * (index as i64 + 1), what, phase)).collect();
        // window k (reading k-1 to k) counts 1 << k, so each booking says which windows it holds
        let slices: Vec<Slice> = (0..session.readings.len()).map(|k| Slice { cpu: 0, start: 100 * k as i64 + 2, end: 100 * (k as i64 + 1) + 1, counts: counts(1 << k) }).collect();
        let thread = session.thread;
        let results = protocol::Results { counters: 1, threads: vec![protocol::Thread { id: thread, registered: true, slices }], ..Default::default() };
        let mut report = Report { names: session.names.clone(), ..Report::default() };
        report.stacks = std::mem::take(&mut session.stacks);
        book(&mut report, &session, results);
        let window = |k: u32| 1u64 << k;
        // the run: windows 4..=13 (start is reading 3)
        let run: u64 = (4..=13).map(window).sum();
        assert_eq!(report.run[0], run);
        assert_eq!(report.per_reading.unwrap()[0], window(2), "the median of windows 1 and 2");
        assert_eq!(report.matched, 14);
        // phases: 0 holds windows 4..=6 (opened by readings 3..=5), 1 the rest
        assert_eq!(report.phases, vec![(0, counts(window(4) + window(5) + window(6))), (1, counts((7..=13).map(window).sum()))]);
        // the frame: from its begin (reading 7) to its end (reading 12): windows 8..=12
        assert_eq!(report.frames.len(), 1);
        assert_eq!(report.frames[0].counts[0], (8..=12).map(window).sum::<u64>());
        assert_eq!((report.frames[0].frame, report.frames[0].phase), (1, 1));
        // the stacks
        let stack_of = |report: &Report, id: i32| report.stacks.stacks.iter().position(|frames| frames.as_ref() == [(id, -1)]).map(|index| index as u32);
        let before = stack_of(&report, super::super::report::outside_function(BEFORE_VM)).unwrap();
        let after = stack_of(&report, super::super::report::outside_function(NO_SAMPLE)).unwrap();
        let step = stack_of(&report, -1_000_000_000).unwrap();
        let booked = |phase: u32, stack: u32| report.by_stack.get(&(phase, stack)).map_or(0, |counts| counts[0]);
        assert_eq!(booked(0, before), window(4), "before the VM: the start to the install");
        assert_eq!(booked(0, 7), window(5), "the first sample");
        assert_eq!(booked(0, after), window(6), "the phase's end after its last sample");
        assert_eq!(booked(1, 8), window(7) + window(8) + window(9) + window(11), "samples take what came since the last, steps aside");
        assert_eq!(booked(1, step), window(10), "the engine step's own");
        assert_eq!(booked(1, after), window(12) + window(13), "after the last sample");
        let total: u64 = report.by_stack.values().map(|counts| counts[0]).sum();
        assert_eq!(total, run, "the stacks add up to the run");
        assert_eq!(report.samples, 3);
    }

    /// The derived figures need both their counts, and say nothing over a zero
    #[test]
    fn rates_are_derived_from_the_counts_they_need() {
        let names: Vec<String> = ["instructions", "cycles", "l1d-misses", "l2-misses", "llc-misses", "branch-misses"].iter().map(|s| s.to_string()).collect();
        let mut values = [0u64; MAX_COUNTERS];
        values[..6].copy_from_slice(&[3000, 1000, 30, 3, 1, 6]);
        let rates = rates(&names, &values);
        let get = |label: &str| rates.iter().find(|rate| rate.0 == label).map(|rate| rate.1);
        assert_eq!(get("IPC"), Some(3.0));
        assert_eq!(get("l2 miss rate (of l1d misses)"), Some(0.1));
        assert_eq!(get("l1d-misses per 1000 instructions"), Some(10.0));
        assert_eq!(get("branch-misses per 1000 instructions"), Some(2.0));
        assert_eq!(get("l1d miss rate"), None, "no accesses counted");
        assert!(super::rates(&names, &[0; MAX_COUNTERS]).is_empty());
    }

    /// `--profile-counters`: names, at most eight, none twice
    #[test]
    fn counter_lists_parse() {
        assert_eq!(parse_counters("cycles, instructions").unwrap(), vec!["cycles".to_string(), "instructions".to_string()]);
        assert!(parse_counters(" , ").is_err());
        assert!(parse_counters("a,b,c,d,e,f,g,h,i").is_err());
        assert!(parse_counters("cycles,cycles").unwrap_err().contains("twice"));
    }
}
