//! Coarse, exclusive wall-time buckets for one live simulation (port of
//! `runtime/engine/profile.luau`). Virtual clocks are untouched.
//!
//! A live session begins a profile around a frame command's input dispatch and again around its
//! simulation ([`begin`] / [`finish`]); instrumented phases ([`enter`] / [`leave`]) charge the wall
//! time since the previous checkpoint to the innermost open bucket ("other" at the bottom), minus
//! the layout time accumulated meanwhile (layout is reported on its own). [`count`] adds to named
//! counters. The stack is 32 deep; deeper enters are counted and their leaves ignored.
//!
//! With `RUR_PROFILE_CALLBACKS=1`, every 7th signal handler (at most 16 per profile) is sampled:
//! its source, line, event name and duration excluding layout ([`begin_callback`] /
//! [`end_callback`], called by the signal dispatch in scheduler.rs / lua/instance.rs).
//!
//! The state is per thread (the Luau module's globals). Idle cost: a thread-local flag read per
//! call; no allocation per enter / leave / count while a profile runs (buckets and counters are
//! small vectors of static names, reserved once per profile).

use std::{
    cell::{Cell, RefCell},
    sync::OnceLock,
    time::Instant,
};

use serde_json::{Map, Value as Json, json};

/// A time source in seconds (the profile's clock, or the layout's accumulated time).
pub type Reader = Box<dyn Fn() -> f64>;

const DEPTH: usize = 32;
const SAMPLE_EVERY: u64 = 7;
const SAMPLE_LIMIT: usize = 16;

/// os.clock: seconds since the first read in this process.
pub fn clock() -> f64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Profile.SampleCallbacks: RUR_PROFILE_CALLBACKS=1 (read once).
pub fn sample_callbacks() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("RUR_PROFILE_CALLBACKS").as_deref() == Ok("1"))
}

/// One sampled handler run.
#[derive(Clone, Debug)]
pub struct CallbackSample {
    pub source: String,
    pub line: Option<usize>,
    pub name: String,
    pub duration: f64,
}

/// Profile.Finish's result.
#[derive(Debug, Default)]
pub struct Report {
    /// Exclusive seconds per bucket, in first-charged order
    pub parts: Vec<(&'static str, f64)>,
    pub counts: Vec<(&'static str, u64)>,
    /// Layout seconds during the profile (excluded from the parts)
    pub layout: f64,
    /// Measured wall seconds (the parts partition it)
    pub wall: f64,
    pub callbacks: Option<Vec<CallbackSample>>,
}

impl Report {
    pub fn part(&self, name: &str) -> Option<f64> {
        self.parts.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }

    pub fn count(&self, name: &str) -> Option<u64> {
        self.counts.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }

    /// Adds to a bucket (the session's `other` fill-up).
    pub fn add_part(&mut self, name: &'static str, seconds: f64) {
        add(&mut self.parts, name, seconds);
    }

    pub fn parts_json(&self) -> Json {
        Json::Object(self.parts.iter().map(|(name, value)| (name.to_string(), json!(value))).collect::<Map<_, _>>())
    }

    pub fn counts_json(&self) -> Json {
        Json::Object(self.counts.iter().map(|(name, value)| (name.to_string(), json!(value))).collect::<Map<_, _>>())
    }

    /// The sampled callbacks (`{source, line, name, duration}` each), None when nothing was sampled
    pub fn callbacks_json(&self) -> Option<Json> {
        let samples = self.callbacks.as_ref()?;
        Some(Json::Array(
            samples
                .iter()
                .map(|s| {
                    let mut sample = Map::new();
                    sample.insert("source".into(), json!(s.source));
                    if let Some(line) = s.line {
                        sample.insert("line".into(), json!(line));
                    }
                    sample.insert("name".into(), json!(s.name));
                    sample.insert("duration".into(), json!(s.duration));
                    Json::Object(sample)
                })
                .collect(),
        ))
    }
}

fn add<T: Copy + std::ops::Add<Output = T>>(list: &mut Vec<(&'static str, T)>, name: &'static str, amount: T) {
    match list.iter_mut().find(|(n, _)| *n == name) {
        Some((_, value)) => *value = *value + amount,
        None => list.push((name, amount)),
    }
}

struct State {
    clock: Reader,
    layout: Reader,
    last: f64,
    last_layout: f64,
    start: f64,
    start_layout: f64,
    parts: Vec<(&'static str, f64)>,
    counts: Vec<(&'static str, u64)>,
    stack: [&'static str; DEPTH],
    depth: usize,
    overflow: u32,
    generation: u64,
    sample_count: usize,
    callbacks: Option<Vec<CallbackSample>>,
}

impl State {
    /// Charges the time since the last checkpoint (minus layout) to the innermost bucket.
    fn checkpoint(&mut self) {
        let (now, layout) = ((self.clock)(), (self.layout)());
        let name = self.stack[self.depth - 1];
        add(&mut self.parts, name, (now - self.last - (layout - self.last_layout)).max(0.0));
        self.last = now;
        self.last_layout = layout;
    }
}

thread_local! {
    /// Whether a profile is active (the idle fast path reads only this)
    static ON: Cell<bool> = const { Cell::new(false) };
    static ACTIVE: RefCell<Option<State>> = const { RefCell::new(None) };
    static SAMPLE_INDEX: Cell<u64> = const { Cell::new(0) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

#[inline]
fn on() -> bool {
    ON.with(Cell::get)
}

/// Whether a profile is active (callers skip work that only feeds counters).
#[inline]
pub fn active() -> bool {
    on()
}

fn with_active<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    ACTIVE.with(|active| active.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Profile.Begin(LayoutTime, Clock?): starts a profile (replacing any active one).
pub fn begin(layout: Reader, clock: Option<Reader>) {
    let clock = clock.unwrap_or_else(|| Box::new(self::clock));
    let (now, layout_now) = (clock(), layout());
    let generation = GENERATION.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let mut stack = [""; DEPTH];
    stack[0] = "other";
    let state = State {
        clock,
        layout,
        last: now,
        last_layout: layout_now,
        start: now,
        start_layout: layout_now,
        parts: Vec::with_capacity(24),
        counts: Vec::with_capacity(24),
        stack,
        depth: 1,
        overflow: 0,
        generation,
        sample_count: 0,
        callbacks: None,
    };
    ACTIVE.with(|active| *active.borrow_mut() = Some(state));
    ON.with(|on| on.set(true));
}

thread_local! {
    /// The buckets open on this thread, mirrored as MicroProfiler scopes (whether or not a profile
    /// is active)
    static TIMELINE: std::cell::RefCell<Vec<(&'static str, Option<(u64, u16)>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Profile.Enter: opens bucket `name` (time from here on is charged to it).
#[inline]
pub fn enter(name: &'static str) {
    if crate::microprofile::enabled() || TIMELINE.with(|stack| !stack.borrow().is_empty()) {
        let opened = crate::microprofile::enter(name);
        TIMELINE.with(|stack| stack.borrow_mut().push((name, opened)));
    }
    if !on() {
        return;
    }
    with_active(|state| {
        if state.depth >= DEPTH {
            state.overflow += 1;
            return;
        }
        state.checkpoint();
        state.stack[state.depth] = name;
        state.depth += 1;
    });
}

/// Profile.Leave: closes the innermost bucket.
#[inline]
pub fn leave() {
    if let Some((name, opened)) = TIMELINE.with(|stack| stack.borrow_mut().pop()) {
        crate::microprofile::leave(name, opened);
    }
    if !on() {
        return;
    }
    with_active(|state| {
        if state.overflow > 0 {
            state.overflow -= 1;
            return;
        }
        state.checkpoint();
        if state.depth > 1 {
            state.depth -= 1;
        }
    });
}

/// Profile.Count(Name, Amount?)
#[inline]
pub fn count(name: &'static str, amount: u64) {
    if !on() {
        return;
    }
    with_active(|state| add(&mut state.counts, name, amount));
}

/// A sampled handler run in progress (Profile.BeginCallback's result).
pub struct Sample {
    generation: u64,
    started: f64,
    layout: f64,
    source: String,
    line: Option<usize>,
    name: String,
}

/// Profile.BeginCallback: every 7th handler run while a profile is active, at most 16 per
/// profile, is sampled. `name` is the signal's name. (Callers check [`sample_callbacks`] first.)
pub fn begin_callback(callback: &mlua::Function, name: impl FnOnce() -> String) -> Option<Sample> {
    if !on() {
        return None;
    }
    let (generation, layout) = with_active(|state| {
        let index = SAMPLE_INDEX.with(|i| {
            i.set(i.get() + 1);
            i.get()
        });
        if index % SAMPLE_EVERY != 0 || state.sample_count >= SAMPLE_LIMIT {
            return None;
        }
        state.sample_count += 1;
        Some((state.generation, (state.layout)()))
    })??;
    // debug.info(Callback, "sl")
    let info = callback.info();
    let source = info.short_src.or(info.source).unwrap_or_default();
    let line = info.line_defined;
    Some(Sample { generation, started: clock(), layout, source, line, name: name() })
}

/// Profile.EndCallback: records the sample's duration (excluding layout) if its profile is still
/// the active one.
pub fn end_callback(sample: Option<Sample>) {
    let Some(sample) = sample else { return };
    if !on() {
        return;
    }
    with_active(|state| {
        if sample.generation != state.generation {
            return;
        }
        let duration = (clock() - sample.started - ((state.layout)() - sample.layout)).max(0.0);
        state.callbacks.get_or_insert_with(Vec::new).push(CallbackSample {
            source: sample.source,
            line: sample.line,
            name: sample.name,
            duration,
        });
    });
}

/// Profile.Finish: the active profile's buckets, counters, layout and wall time; None when no
/// profile is active.
pub fn finish() -> Option<Report> {
    if !on() {
        return None;
    }
    ON.with(|on| on.set(false));
    let mut state = ACTIVE.with(|active| active.borrow_mut().take())?;
    state.checkpoint();
    Some(Report {
        parts: state.parts,
        counts: state.counts,
        layout: state.last_layout - state.start_layout,
        wall: state.last - state.start,
        callbacks: state.callbacks,
    })
}

/// Profile.Cancel
pub fn cancel() {
    ON.with(|on| on.set(false));
    ACTIVE.with(|active| active.borrow_mut().take());
}

#[cfg(test)]
mod tests {
    //! runtime/tests/profile.luau
    use std::rc::Rc;

    use mlua::prelude::*;

    use super::*;
    use crate::{Engine, Options, lua as engine_lua, scheduler};

    fn fake() -> (Rc<Cell<f64>>, Rc<Cell<f64>>) {
        (Rc::new(Cell::new(0.0)), Rc::new(Cell::new(0.0)))
    }

    fn begin_fake(now: &Rc<Cell<f64>>, layout: &Rc<Cell<f64>>) {
        let (n, l) = (now.clone(), layout.clone());
        begin(Box::new(move || l.get()), Some(Box::new(move || n.get())));
    }

    fn zero() -> Reader {
        Box::new(|| 0.0)
    }

    #[test]
    fn exclusive_buckets_layout_stack_and_cancel() {
        let (now, layout) = fake();
        begin_fake(&now, &layout);
        now.set(1.0);
        enter("renderEvents");
        now.set(3.0);
        enter("deferred");
        now.set(8.0);
        layout.set(2.0);
        count("deferredJobs", 4);
        leave();
        now.set(9.0);
        leave();
        now.set(10.0);
        let result = finish().unwrap();
        assert_eq!(result.part("other"), Some(2.0));
        assert_eq!(result.part("renderEvents"), Some(3.0));
        assert_eq!(result.part("deferred"), Some(3.0));
        assert!(result.layout == 2.0 && result.wall == 10.0 && result.count("deferredJobs") == Some(4));

        enter("disabled");
        count("disabled", 1);
        assert!(finish().is_none());

        begin(zero(), None);
        for _ in 0..100 {
            enter("nested");
        }
        for _ in 0..100 {
            leave();
        }
        assert!(finish().unwrap().part("nested").unwrap() >= 0.0);

        begin(zero(), None);
        cancel();
        assert!(finish().is_none());
    }

    #[test]
    fn scheduler_buckets_partition_wall_time() {
        let engine = Engine::new(Options::default()).expect("engine");
        let lua = Lua::new();
        engine_lua::start(&lua, &engine).expect("start");
        engine.state.borrow_mut().sched.set_main_thread(&lua.current_thread());
        let events: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        let log = events.clone();
        engine.state.borrow_mut().sched.add_hook(
            scheduler::Phase::RenderStep,
            Rc::new(move |_, _, _| {
                log.borrow_mut().push("render");
                Ok(())
            }),
        );
        let log = events.clone();
        engine.state.borrow_mut().sched.add_hook(
            scheduler::Phase::PreRender,
            Rc::new(move |lua, engine, _| {
                let log = log.clone();
                let job = lua.create_function(move |_, ()| {
                    log.borrow_mut().push("deferred");
                    Ok(())
                })?;
                let thread = lua.create_thread(job)?;
                engine.state.borrow_mut().sched.defer(scheduler::JobTarget::Thread(thread), Vec::new());
                Ok(())
            }),
        );
        begin(zero(), None);
        scheduler::step(&lua, &engine, None).unwrap();
        let actual = finish().unwrap();
        assert_eq!(events.borrow().join(","), "render,deferred");
        assert_eq!(actual.count("renderSteps"), Some(1));
        assert!(actual.count("heartbeats").unwrap() > 0);
        assert_eq!(actual.count("deferredJobs"), Some(1));
        let mut total = 0.0;
        for (_, duration) in &actual.parts {
            assert!(*duration >= 0.0);
            total += duration;
        }
        assert!((total - actual.wall).abs() < 1e-8, "exclusive simulation buckets must partition measured wall time");

        // Timer entries count discarded cancelled entries, without claiming a resumption
        let ran: Rc<Cell<bool>> = Rc::default();
        let flag = ran.clone();
        let callback = lua.create_function(move |_, ()| {
            flag.set(true);
            Ok(())
        });
        let thread = lua.create_thread(callback.unwrap()).unwrap();
        {
            let mut state = engine.state.borrow_mut();
            state.sched.schedule_task(scheduler::TimerKind::Delay, 0.0, thread.clone(), Vec::new());
            state.sched.cancel(&thread);
        }
        begin(zero(), None);
        scheduler::step(&lua, &engine, None).unwrap();
        let cancelled = finish().unwrap();
        assert!(!ran.get() && cancelled.count("timerEntries") == Some(1));
    }

    #[test]
    fn callback_sampling_is_bounded_and_profiles_are_separate() {
        let lua = Lua::new();
        let sampled = lua.load("return function() end").set_name("=Sampled").eval::<LuaFunction>().unwrap();
        begin(zero(), None);
        for _ in 0..224 {
            let sample = begin_callback(&sampled, || "Changed".into());
            end_callback(sample);
        }
        let samples = finish().unwrap().callbacks.unwrap();
        assert_eq!(samples.len(), 16, "optional callback sampling must stay bounded");
        for sample in &samples {
            assert!(sample.name == "Changed" && sample.duration >= 0.0 && !sample.source.is_empty());
        }

        // Input and simulation are separate profiles: counters / samples cannot leak across them.
        let (now, layout) = fake();
        begin_fake(&now, &layout);
        enter("dispatch");
        now.set(now.get() + 1.0);
        enter("hitTesting");
        now.set(now.get() + 2.0);
        leave();
        enter("deferred");
        now.set(now.get() + 4.0);
        layout.set(layout.get() + 1.0);
        leave();
        leave();
        let input = finish().unwrap();
        assert!(input.part("dispatch") == Some(1.0) && input.part("hitTesting") == Some(2.0) && input.part("deferred") == Some(3.0));
        assert!(input.layout == 1.0 && input.wall == 7.0);
        begin_fake(&now, &layout);
        now.set(now.get() + 1.0);
        let simulation = finish().unwrap();
        assert!(simulation.part("dispatch").is_none() && simulation.callbacks.is_none());
        assert_eq!(simulation.part("other"), Some(1.0));
    }
}
