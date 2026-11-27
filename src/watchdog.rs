//! A run that cannot outlive its job. On 2026-10-24 eleven `ree run`s of automatically mutated scripts spun one core
//! each for 7-8 hours: a mutation deleted the only yield of a loop (a `runOnRunner(coroutine.yield())` in
//! `while true do`, a `counter += Heartbeat:Wait()` in a particle tween), Roblox ends such a script with its execution
//! timeout, and ree had none, while the parent processes waited on it with no timeout of their own. Three guards:
//!
//! - [`ScriptTimeout`]: Roblox's script execution timeout. A resumption the engine makes (a thread resumed, a handler, a
//!   harness) that runs [`DEFAULT_SCRIPT_TIMEOUT`] seconds without yielding errors with Roblox's [`TIMEOUT_MESSAGE`] at its
//!   next safepoint in a script's own code, and again at every later one until it yields or ends, so a `pcall` cannot
//!   swallow it. The seconds are the work clock's (what the VM's instructions, calls and allocations and the engine calls a
//!   script makes cost Studio: cost.rs), not the virtual clock's, so the timeout fires with `REE_FROZEN_CLOCK=1` too and does
//!   not depend on how busy the machine is; host time past [`HOST_BACKSTOP`] times the limit also fires it (work the clock
//!   does not count: natively compiled code, the engine's own Luau). Roblox goes on after a timeout; a run stops after [`DEFAULT_STOP_AFTER`] of them
//!   ([`ScriptTimeout::stopped`]: every script safepoint raises, the frame loop ends the run, exit [`EXIT_SCRIPT_TIMEOUTS`]),
//!   because a script that never yields is usually resumed again each frame.
//! - [`cap_run`]: a wall-clock cap per process, from `--seconds` with a generous margin; ree exits with [`EXIT_CAP`].
//! - [`watch_parent`]: ree exits with [`EXIT_ORPHANED`] when the process that started it is gone.
//!
//! The timeout costs little while scripts yield: the engine bumps an atomic epoch when a top-level resumption begins and
//! ends ([`enter`], from `scheduler::resume`) and copies the VM's work counters as the resumption begins, and a watchdog
//! thread that finds one resumption still running after a tick (0.1 s of host time, less for a limit under 0.8 s: [`tick`])
//! installs the VM's interrupt (Luau's REPL sets it from a signal handler the same way); an engine charge of an eighth of the
//! limit or more arms it at once ([`ScriptTimeout::charged`]). The interrupt counts the resumption's whole work, from that
//! copy on (the work before the watchdog came too: a long call at its start, or the script work before one), and removes
//! itself once the resumption is over. It looks at the work clock at the first safepoint after the arming, at every
//! [`LOOK_STEPS`]th script call or loop step, and at the first safepoint after a call whose size work or engine charges passed
//! a multiple of [`LOOK_UNITS`] or [`LOOK_ENGINE`]: all counted by the VM and the engine alike in every run, so a timeout fires
//! at the same safepoint in every run of a script, with a profile's interrupt or without, once its resumption has run past
//! the limit (never before), and a single call that takes the resumption past it is followed by the timeout at the next
//! safepoint, as a client's script that ran too long in one call is (roblox-sim, Studio 0.747). Every [`HOST_LOOK_EVERY`]th
//! safepoint looks at host time alone, for natively compiled code, which moves no counts.

use std::{
    cell::Cell,
    rc::Rc,
    ffi::{CStr, c_int},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use mlua::ffi::{lua_Callbacks, lua_State};

/// The error a script gets when its resumption runs past the timeout without yielding (a playing client's and a server's text,
/// roblox-sim 0.747, ScriptContext:SetTimeout: tools/parity/probes/timeout_points.luau)
pub const TIMEOUT_MESSAGE: &str = "Script timeout: exhausted allowed execution time";

/// The error a resumption gets when its resumption point ([`point`]) runs past the timeout while the resumption itself has not
/// (the same sessions: the second of two deferred threads of 0.3 s each under a 0.5 s limit)
pub const POINT_TIMEOUT_MESSAGE: &str = "Script timeout: exhausted allowed execution time for the current resumption point";

/// ree's default limit, in seconds: Studio's default Script Timeout Length (Studio settings, Roblox's documentation). A playing
/// client and a server set none (roblox-sim 0.747, which loads no Studio settings: a deferred thread busy for 12 s ran to its end on
/// both, tools/parity/probes/timeout_points.luau notes); ree keeps this one as its guard against a script that never yields
/// (`--script-timeout 0` turns it off)
pub const DEFAULT_SCRIPT_TIMEOUT: f64 = 10.0;

/// Host time that fires the timeout as well, in multiples of the limit: a loop the work clock prices below its host cost
/// still ends (a mutated effect script rebuilt NumberSequences at ~10 host seconds a work second; natively compiled code)
pub const HOST_BACKSTOP: f64 = 6.0;

/// The exit code of a run that reached its wall-clock cap (GNU timeout's)
pub const EXIT_CAP: i32 = 124;

/// The exit code of a run whose parent process is gone
pub const EXIT_ORPHANED: i32 = 125;

/// The exit code of a run stopped by its script timeouts ([`DEFAULT_STOP_AFTER`])
pub const EXIT_SCRIPT_TIMEOUTS: i32 = 126;

/// Script timeouts after which the run stops. Roblox goes on after one, and ree does too, but a script that never yields
/// is usually resumed again (a frame's handler, a pooled runner): each further one costs the full limit, so one mutated
/// script ran 10 s of every frame and its 80 s scene would have taken hours.
pub const DEFAULT_STOP_AFTER: u32 = 3;

/// How often the watchdog thread looks at the running resumptions (a shorter limit's [`tick`] is shorter)
const TICK: f64 = 0.1;

/// The shortest tick: a limit of a few milliseconds still lets the thread sleep
const MIN_TICK: f64 = 0.005;

/// A VM's tick, in host seconds, for its `limit`: [`TICK`], or an eighth of a shorter limit, down to [`MIN_TICK`]. The watchdog arms
/// a resumption that has run a tick, so a short limit's is armed within a quarter of the limit in host time, before a call that
/// charges the whole limit ends.
fn tick_of(limit: f64) -> f64 {
    (limit / 8.0).clamp(MIN_TICK, TICK)
}

/// How long the watchdog thread sleeps: the shortest tick of the running VMs'
fn tick(limits: impl Iterator<Item = f64>) -> Duration {
    Duration::from_secs_f64(limits.map(tick_of).fold(TICK, f64::min))
}

/// A charge of the engine's of at least this share of the limit arms the running resumption's timeout at once ([`ScriptTimeout::charged`]),
/// so the resumption's next safepoint weighs it however briefly the call ran in host time
const ARM_SHARE: f64 = 1.0 / 8.0;

/// Luau's CALL, JUMPBACK, FORNLOOP and FORGLOOP: one of them begins every call and ends every turn of a loop. The VM counts them
/// for scripts (`VmCounters::ops`, `nativeops`), the same in every run, so they time the check's regular looks
const STEP_OPS: [usize; 4] = [21, 24, 57, 58];

/// Script steps ([`STEP_OPS`]) between two regular looks at a long resumption (its work clock, whether it has ended)
const LOOK_STEPS: u64 = 1024;

/// The check also looks at the first safepoint after a call once the counts of a call's size work (elements, bytes, steps,
/// objects: [`size_units`]) pass a multiple of this, so a long library call (a sort of millions) is followed by a look
const LOOK_UNITS: u64 = 65536;

/// The same for the engine calls' charges, in seconds of the work clock
const LOOK_ENGINE: f64 = 1e-4;

/// Safepoints between two looks at host time alone ([`HOST_BACKSTOP`]) and at whether the resumption has ended: natively compiled
/// code moves no step counts (the VM counts no instructions there, nor does the work clock charge them), yet a loop of it must
/// end by host time. These looks weigh no work, so where a timeout of the work clock falls stays the counts' to say.
const HOST_LOOK_EVERY: u32 = 1024;

/// The timeout's state for one VM, shared by its thread and the watchdog thread.
pub struct Watch {
    /// This watch among all the process has made (a later VM's callbacks and Arc may reuse an earlier one's addresses)
    serial: u64,
    /// Odd while the engine is inside a top-level resumption: bumped when one begins and when it ends
    epoch: AtomicU64,
    /// Nested resumptions inside the current top-level one (the VM's thread only)
    depth: AtomicU32,
    /// The epoch of the resumption the watchdog found running long (0: none)
    due: AtomicU64,
    /// Bumped when the first resumption of a resumption point begins and when the point closes ([`point`]): odd while one runs
    point_epoch: AtomicU64,
    /// Resumption points open (the VM's thread only; one opened inside another counts with the outermost)
    point_depth: AtomicU32,
    /// The point epoch the watchdog armed a resumption of: each later resumption of that point is armed as it begins
    point_armed: AtomicU64,
    /// The VM's callbacks (`lua_callbacks`), null once the VM is about to go: the watchdog writes their interrupt
    callbacks: AtomicPtr<lua_Callbacks>,
    /// Serializes writes of the interrupt slot
    slot: Mutex<()>,
    /// Seconds of the work clock a resumption may run without yielding
    limit: f64,
    /// Host time that fires the timeout too, in multiples of the limit ([`HOST_BACKSTOP`])
    backstop: f64,
    /// Script timeouts after which the process exits (0: never)
    stop_after: u32,
    /// Script timeouts so far
    timeouts: AtomicU32,
    /// Those of them host time fired alone ([`HOST_BACKSTOP`]: the work clock had not reached the limit), which a profile that
    /// slows the run makes likelier: its passes are compared by them
    host_timeouts: AtomicU32,
    /// The run stops: `stop_after` timeouts happened ([`ScriptTimeout::stopped`])
    stopped: AtomicBool,
    /// Whether this VM's check currently looks at every safepoint (its thread only, counted in [`EAGER_COUNT`])
    eager: AtomicBool,
    /// Where the last of them ran
    last: Mutex<String>,
}

struct Registry {
    watches: Vec<Weak<Watch>>,
    /// The watchdog thread, once started (a new VM wakes it: its limit may shorten the tick)
    thread: Option<std::thread::Thread>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry { watches: Vec::new(), thread: None });

/// The next watch's serial
static SERIALS: AtomicU64 = AtomicU64::new(1);

/// Whether the watchdog has armed any VM's timeout yet: until then a profile's interrupt skips the check at once
static ARMED: AtomicBool = AtomicBool::new(false);

/// How many times the watchdog has armed a timeout: the first safepoint after each arming looks at the work clock
static ARMINGS: AtomicU64 = AtomicU64::new(0);

/// What the interrupt measured of the resumption it times (the VM's thread)
#[derive(Clone, Copy)]
struct Timing {
    /// The watch and the resumption it times (a later VM on this thread counts its epochs from 0 again)
    serial: u64,
    epoch: u64,
    work: f64,
    host: Option<Instant>,
    /// The resumption has had its timeout (later raises are the same timeout's)
    raised: bool,
    /// The work its resumption point began with, when it runs in one ([`point`])
    point: Option<f64>,
    /// The timeout it had is its point's ([`POINT_TIMEOUT_MESSAGE`])
    by_point: bool,
}

/// Where the check's looks stood at the last safepoint it saw ([`look_due`]): the VM's counters it read (their address) and,
/// counted from them, the multiples of [`LOOK_STEPS`], [`LOOK_UNITS`] and [`LOOK_ENGINE`] reached, the C and engine calls made
/// (the size and engine counts are read again only after a call), and the watchdog's armings
#[derive(Clone, Copy)]
struct Looked {
    counters: usize,
    steps: u64,
    calls: u64,
    units: u64,
    engine: u64,
    armings: u64,
    /// Raw pointers into the active VM's `Local::work` / `Local::drained` cells (valid until `detach`, which resets `counters: 0`).
    work: *const Cell<f64>,
    drained: *const Cell<f64>,
}

thread_local! {
    /// Safepoints the check has seen on this thread ([`HOST_LOOK_EVERY`])
    static SAFEPOINTS: Cell<u32> = const { Cell::new(0) };
    static TIMING: Cell<Timing> = const { Cell::new(Timing { serial: 0, epoch: 0, work: 0.0, host: None, raised: false, point: None, by_point: false }) };
    static LOOKED: Cell<Looked> = const { Cell::new(Looked { counters: 0, steps: 0, calls: 0, units: 0, engine: 0, armings: 0, work: std::ptr::null(), drained: std::ptr::null() }) };
    /// How many VMs on this thread want the check to look at every safepoint (a timeout is due, or a run stops)
    static EAGER_COUNT: Cell<u32> = const { Cell::new(0) };
    /// This thread's VMs with a timeout, for the interrupt (one or two: a service worker holds a prepared run beside the one
    /// it runs)
    static WATCHES: std::cell::RefCell<Vec<Local>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Updates whether `watch` looks at every safepoint and keeps this thread's [`EAGER_COUNT`] balanced
/// so installing, disarming or detaching another VM on the same thread never clears this VM's eager flag.
#[inline]
fn set_eager(watch: &Watch, on: bool) {
    if watch.eager.swap(on, Ordering::Relaxed) != on {
        EAGER_COUNT.with(|count| {
            let current = count.get();
            count.set(if on { current.saturating_add(1) } else { current.saturating_sub(1) });
        });
    }
}

/// A VM's timeout on its own thread: its callbacks (the key), its watch and its scheduler's work clock cells
struct Local {
    callbacks: usize,
    /// The watch's serial ([`enter`] finds its VM's by it)
    serial: u64,
    watch: Weak<Watch>,
    /// What the engine calls charged since the last clock read, and what clock reads took out of it so far (Scheduler's
    /// `work` and `work_drained`)
    work: Rc<Cell<f64>>,
    drained: Rc<Cell<f64>>,
    /// The VM's work counters (`lua_workcounters`: they live as long as the VM, which [`ScriptTimeout::detach`] outlives)
    counters: *const crate::cost::VmCounters,
    /// The work the current outermost resumption began with
    start: Start,
    /// The work the running resumption point began with (its first resumption's), priced at its first look
    point: PointStart,
}

/// The work a resumption point began with: the point epoch, its first resumption's [`Start`] copied, its price once looked at,
/// and whether a timeout of the point was counted ([`ScriptTimeout::timeouts`] counts a point once)
struct PointStart {
    epoch: u64,
    engine: f64,
    counters: Box<crate::cost::VmCounters>,
    priced: Option<f64>,
    counted: bool,
}

/// The work a resumption began with ([`enter`]): its epoch, the engine calls' charges so far and a copy of the VM's counters,
/// priced only when the resumption runs long enough for the watchdog ([`start_work`])
struct Start {
    epoch: u64,
    engine: f64,
    counters: Box<crate::cost::VmCounters>,
}

/// The script timeout of one VM: [`ScriptTimeout::install`] makes it, the engine's scheduler holds it.
#[derive(Clone)]
pub struct ScriptTimeout(Arc<Watch>);

impl ScriptTimeout {
    /// Times the resumptions of `engine`'s VM `lua` on this thread from now on, `limit` seconds of the work clock each
    /// (`None` when `limit` is not above 0: no timeout); the run stops at the `stop_after`th timeout (0: never,
    /// [`ScriptTimeout::stopped`]). The engine's scheduler must hold the result ([`crate::scheduler::Scheduler`]'s
    /// `timeout`), and [`ScriptTimeout::detach`] must run before the VM is dropped ([`crate::Emulator`] does).
    pub fn install(lua: &mlua::Lua, engine: &crate::Engine, limit: f64, stop_after: u32) -> Option<ScriptTimeout> {
        Self::install_with(lua, engine, limit, stop_after, HOST_BACKSTOP)
    }

    /// [`ScriptTimeout::install`] with host time past `backstop` times the limit firing the timeout too (infinite: only the work
    /// clock fires it, as a test that weighs work exactly needs on a busy machine)
    pub(crate) fn install_with(lua: &mlua::Lua, engine: &crate::Engine, limit: f64, stop_after: u32, backstop: f64) -> Option<ScriptTimeout> {
        if !(limit > 0.0 && limit.is_finite()) {
            return None;
        }
        let mut callbacks = std::ptr::null_mut();
        let mut counters = std::ptr::null();
        unsafe {
            let _ = lua.exec_raw::<()>((), |state| {
                callbacks = mlua::ffi::lua_callbacks(state);
                counters = crate::cost::lua_workcounters(state);
            });
        }
        if callbacks.is_null() || counters.is_null() {
            return None;
        }
        let watch = Arc::new(Watch {
            serial: SERIALS.fetch_add(1, Ordering::Relaxed),
            epoch: AtomicU64::new(0),
            depth: AtomicU32::new(0),
            due: AtomicU64::new(0),
            point_epoch: AtomicU64::new(0),
            point_depth: AtomicU32::new(0),
            point_armed: AtomicU64::new(0),
            callbacks: AtomicPtr::new(callbacks),
            slot: Mutex::new(()),
            limit,
            backstop,
            stop_after,
            timeouts: AtomicU32::new(0),
            host_timeouts: AtomicU32::new(0),
            stopped: AtomicBool::new(false),
            eager: AtomicBool::new(false),
            last: Mutex::new(String::new()),
        });
        let (work, drained) = {
            let state = engine.state.borrow();
            (state.sched.work.clone(), state.sched.work_drained.clone())
        };
        // (all integers: zero is a valid VmCounters)
        let start = Start { epoch: 0, engine: 0.0, counters: Box::new(unsafe { std::mem::zeroed() }) };
        let point = PointStart { epoch: 0, engine: 0.0, counters: Box::new(unsafe { std::mem::zeroed() }), priced: None, counted: false };
        LOOKED.with(|looked| looked.set(Looked { counters: 0, work: std::ptr::null(), drained: std::ptr::null(), ..looked.get() }));
        WATCHES.with(|watches| {
            let mut watches = watches.borrow_mut();
            watches.retain(|local| local.watch.strong_count() > 0);
            watches.push(Local { callbacks: callbacks as usize, serial: watch.serial, watch: Arc::downgrade(&watch), work, drained, counters, start, point });
        });
        let mut registry = REGISTRY.lock().unwrap_or_else(|poison| poison.into_inner());
        registry.watches.retain(|weak| weak.strong_count() > 0);
        registry.watches.push(Arc::downgrade(&watch));
        match &registry.thread {
            Some(thread) => thread.unpark(),
            None => registry.thread = std::thread::Builder::new().name("ree-script-timeout".into()).spawn(watchdog).ok().map(|handle| handle.thread().clone()),
        }
        Some(ScriptTimeout(watch))
    }

    /// The watchdog leaves the VM alone from now on (before it is dropped: its callbacks go with it).
    pub fn detach(&self) {
        let watch = &self.0;
        let _slot = watch.slot.lock().unwrap_or_else(|poison| poison.into_inner());
        let callbacks = watch.callbacks.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !callbacks.is_null() {
            unsafe { clear_timeout_interrupt(callbacks) };
        }
        WATCHES.with(|watches| watches.borrow_mut().retain(|local| local.callbacks != callbacks as usize));
        LOOKED.with(|looked| looked.set(Looked { counters: 0, work: std::ptr::null(), drained: std::ptr::null(), ..looked.get() }));
        set_eager(watch, false);
    }

    /// The seconds of the work clock a resumption may run without yielding
    pub fn limit(&self) -> f64 {
        self.0.limit
    }

    /// The engine charged `seconds` (scheduler.rs `charge`): a charge of [`ARM_SHARE`] of the limit or more arms the running
    /// resumption's timeout now, as the watchdog thread would once it had run a tick, so its next safepoint weighs its work
    /// however briefly the call ran in host time (the timeout fires only past the limit; arming only makes the check look)
    #[inline]
    pub fn charged(&self, seconds: f64) {
        if seconds >= self.0.limit * ARM_SHARE {
            let watch = &*self.0;
            let epoch = watch.epoch.load(Ordering::Acquire);
            if epoch % 2 == 1 && watch.due.load(Ordering::Acquire) != epoch {
                arm(watch, epoch);
            }
        }
    }

    /// Script timeouts so far
    pub fn timeouts(&self) -> u32 {
        self.0.timeouts.load(Ordering::Relaxed)
    }

    /// Those of them host time fired alone, past [`HOST_BACKSTOP`] times the limit with the work clock short of it
    pub fn host_timeouts(&self) -> u32 {
        self.0.host_timeouts.load(Ordering::Relaxed)
    }

    /// Why the run must stop, once `stop_after` script timeouts happened: every script safepoint raises from then on, and the
    /// frame loop ends the run with this failure and exit code [`EXIT_SCRIPT_TIMEOUTS`].
    pub fn stopped(&self) -> Option<String> {
        let watch = &self.0;
        watch.stopped.load(Ordering::Relaxed).then(|| {
            format!(
                "{} script timeouts: the run stops (each time a script ran {} s of the work clock without yielding; --max-script-timeouts or REE_MAX_SCRIPT_TIMEOUTS sets how many stop it, 0 never). The last: {}",
                watch.timeouts.load(Ordering::Relaxed),
                watch.limit,
                watch.last.lock().unwrap_or_else(|poison| poison.into_inner())
            )
        })
    }
}

/// A resumption is running while this lives ([`enter`]). It points at the watch the scheduler holds, which outlives
/// every resumption of its VM.
pub struct Entered(*const Watch);

/// The engine begins a resumption (`scheduler::resume`): the outermost one of a nest bumps the epoch now and when it ends, and
/// keeps the work it begins with ([`Start`]).
#[inline]
pub fn enter(timeout: Option<&ScriptTimeout>) -> Entered {
    let Some(timeout) = timeout else { return Entered(std::ptr::null()) };
    let watch = &*timeout.0;
    if watch.depth.fetch_add(1, Ordering::Relaxed) == 0 {
        let epoch = watch.epoch.fetch_add(1, Ordering::Release) + 1;
        begin(watch, epoch);
    }
    Entered(watch)
}

/// Keeps the work the outermost resumption `epoch` of `watch` begins with: the engine calls' charges so far and a copy of the
/// VM's counters (6 KB: a fraction of a resumption's own cost; pricing them is the watchdog's, for the few that run long). The
/// first resumption of a resumption point begins the point too ([`point`]: its work is the point's start), and a later one of a
/// point the watchdog armed is armed at once, so its first safepoint weighs the point's work.
fn begin(watch: &Watch, epoch: u64) {
    let mut point = 0;
    if watch.point_depth.load(Ordering::Relaxed) > 0 {
        point = watch.point_epoch.load(Ordering::Relaxed);
        if point % 2 == 0 {
            point += 1;
            watch.point_epoch.store(point, Ordering::Release);
        }
    }
    WATCHES.with(|watches| {
        let Ok(mut watches) = watches.try_borrow_mut() else { return };
        if let Some(local) = watches.iter_mut().find(|local| local.serial == watch.serial) {
            local.start.epoch = epoch;
            local.start.engine = local.work.get() + local.drained.get();
            // (the VM's own thread: nothing else writes its counters)
            unsafe { std::ptr::copy_nonoverlapping(local.counters, &mut *local.start.counters, 1) };
            if point != 0 && local.point.epoch != point {
                let Local { start, point: begun, .. } = local;
                begun.epoch = point;
                begun.engine = start.engine;
                unsafe { std::ptr::copy_nonoverlapping(&*start.counters, &mut *begun.counters, 1) };
                begun.priced = None;
                begun.counted = false;
            }
        }
    });
    if point != 0 && watch.point_armed.load(Ordering::Acquire) == point && watch.due.load(Ordering::Acquire) != epoch {
        arm(watch, epoch);
    }
}

/// A resumption point is open while this lives ([`point`])
pub struct Point(Option<Arc<Watch>>);

/// The engine opens a resumption point (scheduler.rs: a flush of the deferred queue, an engine event's handlers with the flush
/// after them, the Heartbeat step): the resumptions it begins share one timer, counted from the first one's start, as a client's
/// do. Past the limit the running one raises [`POINT_TIMEOUT_MESSAGE`] (or [`TIMEOUT_MESSAGE`] when it has itself run the limit)
/// and each later one of the point at its first safepoint. Measured in roblox-sim's 0.747 headless client and its server
/// (ScriptContext:SetTimeout at identity 4; tools/parity/probes/timeout_points.luau): the threads of one flush of the deferred
/// queue, deferred from anywhere and by each other, the handlers of one engine event (PreAnimation, PostSimulation, RenderStepped;
/// Stepped and PreSimulation together, RenderStepped and PreRender together) and the deferred work they leave share one, and so do
/// the pass after the task.wait / task.delay resumptions, the Heartbeat handlers and the flush after them (a thread the pass
/// deferred, run after the Heartbeat handlers, shares the pass's); each task.wait, task.delay and wait() resumption and each
/// render-step binding has its own. One opened inside another counts with the outermost; the resumptions of one opened inside a
/// resumption count with that resumption.
#[inline]
pub fn point(timeout: Option<&ScriptTimeout>) -> Point {
    let Some(timeout) = timeout else { return Point(None) };
    timeout.0.point_depth.fetch_add(1, Ordering::Relaxed);
    Point(Some(timeout.0.clone()))
}

impl Drop for Point {
    #[inline]
    fn drop(&mut self) {
        if let Some(watch) = &self.0 {
            if watch.point_depth.fetch_sub(1, Ordering::Relaxed) == 1 {
                let point = watch.point_epoch.load(Ordering::Relaxed);
                if point % 2 == 1 {
                    watch.point_epoch.store(point + 1, Ordering::Release);
                }
            }
        }
    }
}

/// The work clock at the start of the running resumption point `point` (an odd point epoch) of the VM with these `callbacks`,
/// priced once a point
fn point_work(callbacks: usize, point: u64) -> Option<f64> {
    if point % 2 == 0 {
        return None;
    }
    WATCHES.with(|watches| {
        let mut watches = watches.try_borrow_mut().ok()?;
        let local = watches.iter_mut().find(|local| local.callbacks == callbacks)?;
        let begun = &mut local.point;
        if begun.epoch != point {
            return None;
        }
        if begun.priced.is_none() {
            begun.priced = Some(begun.engine + crate::cost::luau(&begun.counters));
        }
        begun.priced
    })
}

/// Whether the timeout of the resumption point `point` is the point's first (its later resumptions raise it too: counted once)
fn first_of_point(callbacks: usize, point: u64) -> bool {
    WATCHES.with(|watches| {
        let Ok(mut watches) = watches.try_borrow_mut() else { return true };
        let Some(local) = watches.iter_mut().find(|local| local.callbacks == callbacks) else { return true };
        if local.point.epoch != point {
            return true;
        }
        !std::mem::replace(&mut local.point.counted, true)
    })
}

/// The work clock at the start of the outermost resumption `epoch` of the VM with these `callbacks`, when [`begin`] kept it
fn start_work(callbacks: usize, epoch: u64) -> Option<f64> {
    WATCHES.with(|watches| {
        let watches = watches.try_borrow().ok()?;
        let local = watches.iter().find(|local| local.callbacks == callbacks)?;
        (local.start.epoch == epoch).then(|| local.start.engine + crate::cost::luau(&local.start.counters))
    })
}

impl Drop for Entered {
    #[inline]
    fn drop(&mut self) {
        if let Some(watch) = unsafe { self.0.as_ref() } {
            if watch.depth.fetch_sub(1, Ordering::Relaxed) == 1 {
                let ended = watch.epoch.fetch_add(1, Ordering::Release);
                let due = watch.due.load(Ordering::Acquire);
                if due != 0 && due <= ended && !watch.stopped.load(Ordering::Relaxed) {
                    disarm_if_stale(watch, due);
                }
            }
        }
    }
}

/// Disarms a timeout interrupt immediately when the long resumption it was armed for ends,
/// so subsequent short resumptions do not keep trapping on every safepoint until a look fires.
#[cold]
#[inline(never)]
fn disarm_if_stale(watch: &Watch, due: u64) {
    set_eager(watch, false);
    let _slot = watch.slot.lock().unwrap_or_else(|poison| poison.into_inner());
    let pointer = watch.callbacks.load(Ordering::Acquire);
    if watch.due.compare_exchange(due, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok() && !pointer.is_null() {
        unsafe { clear_timeout_interrupt(pointer) };
    }
}

/// The interrupt slot as the atomic it is written through (a function pointer or null)
unsafe fn interrupt_slot(callbacks: *mut lua_Callbacks) -> &'static AtomicUsize {
    unsafe { AtomicUsize::from_ptr(std::ptr::addr_of_mut!((*callbacks).interrupt).cast::<usize>()) }
}

/// Removes [`timeout_interrupt`] from the slot when it is there (another interrupt, a profile's or a host's, stays)
unsafe fn clear_timeout_interrupt(callbacks: *mut lua_Callbacks) {
    let slot = unsafe { interrupt_slot(callbacks) };
    let _ = slot.compare_exchange(timeout_interrupt as usize, 0, Ordering::AcqRel, Ordering::Relaxed);
}

/// Arms `watch`'s timeout for its resumption `epoch`: the check looks at its next safepoint, from the timeout's own interrupt
/// (installed now unless another, a profile's or a host's, holds the slot: that one calls the check itself)
fn arm(watch: &Watch, epoch: u64) {
    // (under the slot's lock: the VM's thread disarms under it too, so it never clears a fresh arming)
    let _slot = watch.slot.lock().unwrap_or_else(|poison| poison.into_inner());
    watch.due.store(epoch, Ordering::Release);
    ARMINGS.fetch_add(1, Ordering::Release);
    ARMED.store(true, Ordering::Release);
    let callbacks = watch.callbacks.load(Ordering::Acquire);
    if !callbacks.is_null() {
        // (a profile's interrupt, already there, checks `due` itself: luau_profile)
        let slot = unsafe { interrupt_slot(callbacks) };
        let _ = slot.compare_exchange(0, timeout_interrupt as usize, Ordering::AcqRel, Ordering::Relaxed);
    }
}

/// The watchdog thread: arms the interrupt of a VM whose resumption has run a whole tick of its own ([`tick_of`]).
fn watchdog() {
    // each watch's serial, the epoch it was at and when the thread first saw it there; the same for its resumption points
    let mut seen: Vec<(u64, u64, Instant)> = Vec::new();
    let mut seen_points: Vec<(u64, u64, Instant)> = Vec::new();
    loop {
        let watches: Vec<Arc<Watch>> = {
            let mut registry = REGISTRY.lock().unwrap_or_else(|poison| poison.into_inner());
            registry.watches.retain(|weak| weak.strong_count() > 0);
            registry.watches.iter().filter_map(Weak::upgrade).collect()
        };
        let now = Instant::now();
        let mut next = Vec::with_capacity(watches.len());
        let mut next_points = Vec::new();
        for watch in &watches {
            let key = watch.serial;
            let epoch = watch.epoch.load(Ordering::Acquire);
            let since = seen.iter().find(|(k, e, _)| *k == key && *e == epoch).map_or(now, |(_, _, since)| *since);
            let mut ran = now.duration_since(since).as_secs_f64();
            // a resumption point that ran a tick: its later resumptions are armed as they begin (begin), the running one now
            let point = watch.point_epoch.load(Ordering::Acquire);
            if point % 2 == 1 {
                let point_since = seen_points.iter().find(|(k, e, _)| *k == key && *e == point).map_or(now, |(_, _, since)| *since);
                let point_ran = now.duration_since(point_since).as_secs_f64();
                if point_ran >= tick_of(watch.limit) {
                    watch.point_armed.store(point, Ordering::Release);
                }
                ran = ran.max(point_ran);
                next_points.push((key, point, point_since));
            }
            if epoch % 2 == 1 && ran >= tick_of(watch.limit) && watch.due.load(Ordering::Acquire) != epoch {
                arm(watch, epoch);
            }
            next.push((key, epoch, since));
        }
        seen = next;
        seen_points = next_points;
        // (a new VM unparks it early, so a shorter limit's tick applies at once)
        std::thread::park_timeout(tick(watches.iter().map(|watch| watch.limit)));
    }
}

unsafe extern "C-unwind" fn timeout_interrupt(state: *mut lua_State, gc: c_int) {
    // never raise from a GC interrupt (as mlua and rur's cancellation interrupt)
    if gc >= 0 {
        return;
    }
    unsafe { check(state) }
}

/// The timeout's check at a safepoint of `state` (the timeout's own interrupt, or a profile's that holds the slot):
/// raises [`TIMEOUT_MESSAGE`] when the resumption the watchdog armed it for has run past the limit.
///
/// # Safety
/// `state` is the running thread of a VM, called from its interrupt at a non-GC safepoint.
#[inline]
pub unsafe fn check(state: *mut lua_State) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    // every safepoint of an armed VM comes here (an empty loop's every step): only the ones look_due names look further,
    // unless a timeout is due and waits for a script frame (a loop's period can keep every looked-at safepoint in the
    // engine's Luau)
    let looked = if EAGER_COUNT.with(Cell::get) != 0 {
        None
    } else {
        match unsafe { look_due(state) } {
            Some(full) => Some(full),
            None => return,
        }
    };
    let callbacks = unsafe { mlua::ffi::lua_callbacks(state) } as usize;
    let found = WATCHES.with(|watches| {
        let watches = watches.borrow();
        let local = watches.iter().find(|local| local.callbacks == callbacks)?;
        Some((local.watch.upgrade()?, local.work.clone(), local.drained.clone()))
    });
    let Some((watch, charged, drained)) = found else {
        return;
    };
    // the work clock so far: the VM's own work and what the engine calls charged (cost.rs)
    let work = |state| work(state) + charged.get() + drained.get();
    if watch.stopped.load(Ordering::Relaxed) {
        // the run stops: every script safepoint raises until the frame loop ends it (record/run.rs)
        set_eager(&watch, true);
        if !in_engine_luau(state) {
            drop(watch);
            unsafe { raise(state, false) }
        }
        return;
    }
    let due = watch.due.load(Ordering::Acquire);
    if due == 0 {
        set_eager(&watch, false);
        return;
    }
    if watch.epoch.load(Ordering::Acquire) != due {
        // the resumption it was armed for has ended: back to no interrupt
        set_eager(&watch, false);
        let _slot = watch.slot.lock().unwrap_or_else(|poison| poison.into_inner());
        let pointer = watch.callbacks.load(Ordering::Acquire);
        if watch.due.compare_exchange(due, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok() && !pointer.is_null() {
            unsafe { clear_timeout_interrupt(pointer) };
        }
        return;
    }
    let full = if watch.eager.load(Ordering::Relaxed) {
        true
    } else if let Some(full) = looked {
        full
    } else {
        match unsafe { look_due(state) } {
            Some(full) => full,
            None => return,
        }
    };
    let mut timing = TIMING.with(Cell::get);
    if timing.epoch != due || timing.serial != watch.serial {
        // the first look at this resumption: its work counts from its start (enter), the calls before the watchdog came too,
        // and is weighed at once (a long call at its start is followed by the timeout at the first safepoint after it)
        let start = start_work(callbacks, due).unwrap_or_else(|| work(state));
        let point = point_work(callbacks, watch.point_epoch.load(Ordering::Acquire));
        timing = Timing { serial: watch.serial, epoch: due, work: start, host: Some(Instant::now()), raised: false, point, by_point: false };
        TIMING.with(|cell| cell.set(timing));
        // (an earlier resumption's timeout, due or raised, left the check looking at every safepoint)
        set_eager(&watch, false);
    }
    let mut by_host = false;
    if !timing.raised {
        // (the resumption's own work first: one that ran the limit itself gets the plain message, as a point's first one does)
        let now = if full { Some(work(state)) } else { None };
        let over_work = now.is_some_and(|now| now - timing.work >= watch.limit);
        let over_point = !over_work && now.is_some_and(|now| timing.point.is_some_and(|point| now - point >= watch.limit));
        let over_host = timing.host.is_some_and(|host| host.elapsed().as_secs_f64() >= watch.limit * watch.backstop);
        if !(over_work || over_point || over_host) {
            return;
        }
        by_host = !over_work && !over_point;
        timing.by_point = over_point;
    }
    // due (or raised already and still running: a pcall caught it): raised at the first safepoint in a script's own code
    set_eager(&watch, true);
    if in_engine_luau(state) {
        return;
    }
    if !timing.raised {
        TIMING.with(|cell| cell.set(Timing { raised: true, ..timing }));
        // (a resumption point's later resumptions raise its timeout too: counted once)
        if !timing.by_point || first_of_point(callbacks, watch.point_epoch.load(Ordering::Acquire)) {
            let count = watch.timeouts.fetch_add(1, Ordering::Relaxed) + 1;
            if by_host {
                watch.host_timeouts.fetch_add(1, Ordering::Relaxed);
            }
            let place = script_stack(state);
            crate::output::diagnostic(&format!("ree: script timeout {count} at {place}"));
            if watch.stop_after > 0 && count >= watch.stop_after {
                *watch.last.lock().unwrap_or_else(|poison| poison.into_inner()) = place;
                watch.stopped.store(true, Ordering::Relaxed);
            }
        }
    }
    drop(watch);
    unsafe { raise(state, timing.by_point) }
}

/// Whether the check looks at this safepoint of `state`: `Some(true)` to weigh the work clock, at the first safepoint after the
/// watchdog armed a timeout, one where the script's calls and loop steps ([`STEP_OPS`]) reach a multiple of [`LOOK_STEPS`], or
/// the first after a C or engine call that took the size work ([`size_units`]) or the engine calls' charges past a multiple of
/// [`LOOK_UNITS`] or [`LOOK_ENGINE`]; `Some(false)` at every [`HOST_LOOK_EVERY`]th safepoint, for host time alone. The counts are
/// the VM's and the engine's, the same in every run of a script and with or without a profile's interrupt, so the work clock is
/// weighed at the same safepoints; a long call is followed by a look.
///
/// # Safety
/// As [`check`].
#[inline]
unsafe fn look_due(state: *mut lua_State) -> Option<bool> {
    let seen = SAFEPOINTS.with(|seen| {
        let count = seen.get().wrapping_add(1);
        seen.set(count);
        count
    });
    let counters = unsafe { &*crate::cost::lua_workcounters(state) };
    let key = counters as *const crate::cost::VmCounters as usize;
    let armings = ARMINGS.load(Ordering::Acquire);
    let steps = STEP_OPS.iter().fold(0u64, |sum, &op| sum.wrapping_add(counters.ops[op]).wrapping_add(counters.nativeops[op])) / LOOK_STEPS;
    let calls = counters.ccalls.wrapping_add(counters.tmcalls).wrapping_add(counters.hostcalls);
    let last = LOOKED.with(Cell::get);
    let fresh = last.counters != key;
    let mut look = fresh || last.armings != armings || last.steps != steps;
    let mut next = Looked { counters: key, steps, calls, armings, ..last };
    if fresh {
        let (work, drained) = engine_work_ptrs(state);
        next.work = work;
        next.drained = drained;
    }
    if fresh || calls != last.calls {
        next.units = size_units(counters) / LOOK_UNITS;
        next.engine = if !next.work.is_null() && !next.drained.is_null() {
            unsafe { (((*next.work).get() + (*next.drained).get()) / LOOK_ENGINE) as u64 }
        } else {
            0
        };
        look |= next.units != last.units || next.engine != last.engine;
    }
    LOOKED.with(|looked| looked.set(next));
    if look {
        Some(true)
    } else {
        seen.is_multiple_of(HOST_LOOK_EVERY).then_some(false)
    }
}

/// What a script's work did besides its instructions, in the VM's own counts summed as they come (elements, bytes, pattern
/// steps, objects, nanoseconds of native datatype work): a library call that runs long moves them by its input's size
fn size_units(c: &crate::cost::VmCounters) -> u64 {
    let scalars = [
        c.allocated, c.objects, c.stored, c.rehashed, c.hostnanos as u64, c.numtostr, c.sortcompares, c.sortcalls, c.createitems, c.cloneitems,
        c.concatitems, c.splitparts, c.formats, c.formatspecs, c.newstrings,
    ];
    let special = &c.hostops[crate::cost::FIRST_SPECIAL_SLOT..];
    scalars.iter().chain(special).fold(0u64, |sum, &count| sum.wrapping_add(count))
}

/// Raw pointers to the engine calls' charge cells for the VM with `state`'s callbacks.
fn engine_work_ptrs(state: *mut lua_State) -> (*const Cell<f64>, *const Cell<f64>) {
    let callbacks = unsafe { mlua::ffi::lua_callbacks(state) } as usize;
    WATCHES.with(|watches| {
        let watches = watches.try_borrow().ok()?;
        let local = watches.iter().find(|local| local.callbacks == callbacks)?;
        Some((Rc::as_ptr(&local.work), Rc::as_ptr(&local.drained)))
    })
    .unwrap_or((std::ptr::null(), std::ptr::null()))
}

/// Where `state` runs, innermost first: `name chunk:line` for up to 6 Luau frames
fn script_stack(state: *mut lua_State) -> String {
    let mut frames = Vec::new();
    unsafe {
        let mut debug: mlua::ffi::lua_Debug = std::mem::zeroed();
        let mut level = 0;
        while frames.len() < 6 && level < 64 && mlua::ffi::lua_getinfo(state, level, c"sln".as_ptr(), &mut debug) != 0 {
            level += 1;
            if debug.currentline < 0 {
                continue;
            }
            let source = if debug.short_src.is_null() { "?".into() } else { CStr::from_ptr(debug.short_src).to_string_lossy() };
            let name = if debug.name.is_null() { "anonymous".into() } else { CStr::from_ptr(debug.name).to_string_lossy() };
            frames.push(format!("{name} {source}:{}", debug.currentline));
        }
    }
    frames.join(" < ")
}

/// What the VM's own script work has cost so far, in Studio seconds (cost.rs `luau`)
fn work(state: *mut lua_State) -> f64 {
    unsafe { crate::cost::luau(&*crate::cost::lua_workcounters(state)) }
}

/// Whether the running function is the engine's own Luau (`=rur.*` chunks): Roblox's timeout never fires inside the engine,
/// so it waits for the script's next safepoint
fn in_engine_luau(state: *mut lua_State) -> bool {
    unsafe {
        let mut debug: mlua::ffi::lua_Debug = std::mem::zeroed();
        if mlua::ffi::lua_getinfo(state, 0, c"s".as_ptr(), &mut debug) == 0 || debug.source.is_null() {
            return false;
        }
        CStr::from_ptr(debug.source).to_bytes().starts_with(b"=rur.")
    }
}

/// Raises the timeout's error in `state` ([`POINT_TIMEOUT_MESSAGE`] for its resumption point's), after the position of the running
/// function's caller as luaL_where(1) gives it when that caller is a script's Luau, as a playing client and a server do (roblox-sim
/// 0.747): a busy loop in a thread's own function raises the message alone, one in a function it called names the calling line
/// (`<chunk>:<line>: Script timeout: ...`; called from that one, its own caller's line), and so does one inside a C function a
/// script called (a backtracking string.find); tools/parity/probes/timeout_points.luau
unsafe fn raise(state: *mut lua_State, by_point: bool) -> ! {
    let message = if by_point { POINT_TIMEOUT_MESSAGE } else { TIMEOUT_MESSAGE };
    unsafe {
        mlua::ffi::lua_rawcheckstack(state, 2);
        if caller_is_script(state) {
            mlua::ffi::luaL_where(state, 1);
        } else {
            mlua::ffi::lua_pushlstring(state, c"".as_ptr(), 0);
        }
        mlua::ffi::lua_pushlstring(state, message.as_ptr().cast(), message.len());
        mlua::ffi::lua_concat(state, 2);
        mlua::ffi::lua_error(state)
    }
}

/// Whether the caller of the running function (level 1) is a script's Luau with a line: not a C function, not the engine's own Luau
fn caller_is_script(state: *mut lua_State) -> bool {
    unsafe {
        let mut debug: mlua::ffi::lua_Debug = std::mem::zeroed();
        if mlua::ffi::lua_getinfo(state, 1, c"sl".as_ptr(), &mut debug) == 0 || debug.currentline <= 0 || debug.source.is_null() {
            return false;
        }
        !CStr::from_ptr(debug.source).to_bytes().starts_with(b"=rur.")
    }
}

/// The script timeout a run asked for: `--script-timeout <seconds>`, else `REE_SCRIPT_TIMEOUT`, else Studio's default (0
/// turns it off).
pub fn script_timeout_seconds(flag: Option<f64>) -> f64 {
    flag.or_else(|| std::env::var("REE_SCRIPT_TIMEOUT").ok().and_then(|text| text.trim().parse().ok())).unwrap_or(DEFAULT_SCRIPT_TIMEOUT)
}

/// How many script timeouts stop a run: `--max-script-timeouts <n>`, else `REE_MAX_SCRIPT_TIMEOUTS`, else
/// [`DEFAULT_STOP_AFTER`] (0: never).
pub fn stop_after(flag: Option<u32>) -> u32 {
    flag.or_else(|| std::env::var("REE_MAX_SCRIPT_TIMEOUTS").ok().and_then(|text| text.trim().parse().ok())).unwrap_or(DEFAULT_STOP_AFTER)
}

/// The default wall-clock cap of a run of `seconds` virtual seconds: 10 minutes for the mount and the harness's setup plus
/// 20 host seconds a virtual second. The heaviest scenes measured run at 0.5-3 host seconds a virtual second.
pub fn default_cap(seconds: f64) -> f64 {
    600.0 + 20.0 * seconds.max(0.0)
}

/// The wall-clock cap a run asked for: `--max-wall-seconds <n>`, else `REE_MAX_WALL_SECONDS`, else [`default_cap`] (0 turns
/// it off).
pub fn cap_seconds(flag: Option<f64>, seconds: f64) -> f64 {
    flag.or_else(|| std::env::var("REE_MAX_WALL_SECONDS").ok().and_then(|text| text.trim().parse().ok())).unwrap_or_else(|| default_cap(seconds))
}

/// Ends the process with `code` at once after writing `message` to stderr: no destructors, atexit handlers or buffered
/// stdout (the main thread may be anywhere: in the VM, holding the heap's lock).
pub fn terminate(message: &str, code: i32) -> ! {
    use std::io::Write;
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{message}");
    let _ = stderr.flush();
    #[cfg(windows)]
    unsafe {
        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn TerminateProcess(process: *mut std::ffi::c_void, code: u32) -> i32;
        }
        TerminateProcess(GetCurrentProcess(), code as u32);
    }
    #[cfg(unix)]
    unsafe {
        unsafe extern "C" {
            fn _exit(code: c_int) -> !;
        }
        _exit(code)
    }
    #[allow(unreachable_code)]
    std::process::exit(code)
}

/// Ends the process with [`EXIT_CAP`] once `cap` seconds of wall-clock time have passed since `started` (nothing when `cap`
/// is not above 0). `command` and `seconds` name the run in the message.
pub fn cap_run(command: &str, cap: f64, seconds: f64, started: Instant) {
    if !(cap > 0.0 && cap.is_finite()) {
        return;
    }
    let message = format!(
        "ree {command}: wall-clock cap reached after {cap:.0} s (the run asked for {seconds} virtual seconds; --max-wall-seconds or REE_MAX_WALL_SECONDS sets the cap, 0 turns it off); the run did not finish"
    );
    let deadline = started + Duration::from_secs_f64(cap);
    let _ = std::thread::Builder::new().name("ree-wall-cap".into()).spawn(move || {
        loop {
            let now = Instant::now();
            if now >= deadline {
                terminate(&message, EXIT_CAP);
            }
            std::thread::sleep(deadline - now);
        }
    });
}

/// Ends the process with [`EXIT_ORPHANED`] when the process that started it exits (a thread waits on its handle), or at
/// once when it is gone already (no process has its id, or a later one does). Nothing with `REE_PARENT_WATCH=0` (a launcher
/// that detaches on purpose), or when the parent may not be watched (access denied).
pub fn watch_parent(command: &str) {
    if std::env::var("REE_PARENT_WATCH").as_deref() == Ok("0") {
        return;
    }
    platform::watch_parent(command);
}

/// Why a `ree serve` job stopped before its run ended: its caller left ([`caller_gone`])
pub const CALLER_GONE: &str = "its caller has gone (the client that asked for this job exited or gave up waiting): the job stops";

/// How often a job's frames ask whether its caller has gone
const CALLER_EVERY: Duration = Duration::from_millis(250);

/// The caller of the job running on a thread ([`for_caller`])
struct Caller {
    gone: Box<dyn Fn() -> bool>,
    asked: Cell<Option<Instant>>,
    left: Cell<bool>,
}

thread_local! {
    static CALLER: std::cell::RefCell<Option<Rc<Caller>>> = const { std::cell::RefCell::new(None) };
}

/// Runs `run` (a `ree serve` job) for a caller that `gone` says has left: its frames stop once it has ([`EXIT_ORPHANED`])
pub fn for_caller<R>(gone: impl Fn() -> bool + 'static, run: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rc<Caller>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            CALLER.with(|caller| *caller.borrow_mut() = previous);
        }
    }
    let caller = Rc::new(Caller { gone: Box::new(gone), asked: Cell::new(None), left: Cell::new(false) });
    let _restore = Restore(CALLER.with(|current| current.replace(Some(caller))));
    run()
}

/// Whether the caller of this thread's job has gone (never outside [`for_caller`]), asked at most every [`CALLER_EVERY`]
pub fn caller_gone() -> bool {
    let Some(caller) = CALLER.with(|caller| caller.borrow().clone()) else { return false };
    if caller.left.get() {
        return true;
    }
    let now = Instant::now();
    if caller.asked.get().is_some_and(|asked| now.duration_since(asked) < CALLER_EVERY) {
        return false;
    }
    caller.asked.set(Some(now));
    let left = (caller.gone)();
    caller.left.set(left);
    left
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    const TH32CS_SNAPPROCESS: u32 = 0x2;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const INFINITE: u32 = 0xFFFF_FFFF;
    const WAIT_OBJECT_0: u32 = 0;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    struct ProcessEntry32W {
        size: u32,
        usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        threads: u32,
        parent_process_id: u32,
        priority_base: i32,
        flags: u32,
        exe_file: [u16; 260],
    }

    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, process: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetCurrentProcess() -> *mut c_void;
        fn GetCurrentProcessId() -> u32;
        fn GetProcessTimes(process: *mut c_void, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }

    const ERROR_INVALID_PARAMETER: u32 = 87;

    fn parent_id() -> Option<u32> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot as isize == INVALID_HANDLE_VALUE || snapshot.is_null() {
                return None;
            }
            let me = GetCurrentProcessId();
            let mut entry: ProcessEntry32W = std::mem::zeroed();
            entry.size = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut found = None;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                if entry.process_id == me {
                    found = Some(entry.parent_process_id);
                    break;
                }
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            found
        }
    }

    fn created(process: *mut c_void) -> Option<u64> {
        let (mut creation, mut exit, mut kernel, mut user) = (0, 0, 0, 0);
        (unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } != 0).then_some(creation)
    }

    pub fn watch_parent(command: &str) {
        let Some(parent) = parent_id().filter(|&pid| pid != 0) else { return };
        let message = format!("ree {command}: its parent process (pid {parent}) is gone; exiting");
        let handle = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, parent) };
        if handle.is_null() {
            // no such process: the parent ended before this one looked (access denied: one we may not watch)
            if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                super::terminate(&message, super::EXIT_ORPHANED);
            }
            return;
        }
        // a process made after this one has only reused the id of a parent that is gone
        match (created(handle), created(unsafe { GetCurrentProcess() })) {
            (Some(theirs), Some(mine)) if theirs <= mine => {}
            (Some(_), Some(_)) => super::terminate(&message, super::EXIT_ORPHANED),
            _ => {
                unsafe { CloseHandle(handle) };
                return;
            }
        }
        let handle = handle as usize;
        let _ = std::thread::Builder::new().name("ree-parent-watch".into()).spawn(move || {
            if unsafe { WaitForSingleObject(handle as *mut c_void, INFINITE) } == WAIT_OBJECT_0 {
                super::terminate(&message, super::EXIT_ORPHANED);
            }
        });
    }
}

#[cfg(unix)]
mod platform {
    pub fn watch_parent(command: &str) {
        let parent = std::os::unix::process::parent_id();
        if parent <= 1 {
            return;
        }
        let message = format!("ree {command}: its parent process (pid {parent}) is gone; exiting");
        let _ = std::thread::Builder::new().name("ree-parent-watch".into()).spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if std::os::unix::process::parent_id() != parent {
                    super::terminate(&message, super::EXIT_ORPHANED);
                }
            }
        });
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    pub fn watch_parent(_command: &str) {}
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use mlua::prelude::*;

    use super::{ScriptTimeout, TIMEOUT_MESSAGE};
    use crate::{
        Engine, Options,
        lua::{self as engine_lua, globals::CallOutcome},
    };

    fn start(limit: f64, stop_after: u32) -> (Lua, Rc<Engine>, ScriptTimeout) {
        start_with(limit, stop_after, super::HOST_BACKSTOP)
    }

    /// [`start`] with only the work clock firing the timeout, on interpreted scripts: a test that weighs work exactly, which a
    /// busy machine's host time must not decide
    fn start_exact(limit: f64) -> (Lua, Rc<Engine>, ScriptTimeout) {
        start_with(limit, 0, f64::INFINITY)
    }

    fn start_with(limit: f64, stop_after: u32, backstop: f64) -> (Lua, Rc<Engine>, ScriptTimeout) {
        let engine = Engine::new(Options::default()).expect("engine");
        let lua = Lua::new();
        engine_lua::start(&lua, &engine).expect("start");
        if backstop.is_infinite() {
            // (scripts interpreted, as `ree run` and `ree record` run them: the VM counts their instructions and the work clock
            // charges them; natively compiled, a loop's work is not counted and only host time ends it)
            engine_lua::modules::interpret_all(&lua, &engine);
        }
        let timeout = ScriptTimeout::install_with(&lua, &engine, limit, stop_after, backstop).expect("a timeout");
        engine.state.borrow_mut().sched.timeout = Some(timeout.clone());
        (lua, engine, timeout)
    }

    /// Runs `source` as a script on a scheduler thread; how its first resumption ended: "finished", "pending" or the error
    fn call(lua: &Lua, engine: &Rc<Engine>, source: &str) -> String {
        let script = {
            let mut state = engine.state.borrow_mut();
            let class = engine.reflection.class_id("ModuleScript").unwrap();
            crate::instance::create_named(&mut state, &engine.reflection, class, "Test")
        };
        let env = engine_lua::modules::environment(lua, engine, script).expect("environment");
        let function = lua.load(source).set_name("=Test").set_environment(env).into_function().expect("compile");
        match engine_lua::globals::call(lua, engine, function, LuaMultiValue::new()).expect("call").outcome().expect("outcome") {
            CallOutcome::Finished(_) => "finished".into(),
            CallOutcome::Pending => "pending".into(),
            CallOutcome::Failed(message, _) => message,
        }
    }

    fn timed_out(outcome: &str) -> bool {
        outcome.contains(TIMEOUT_MESSAGE)
    }

    /// The `shared` table every script of the engine sees
    fn shared(engine: &Rc<Engine>) -> LuaTable {
        engine.lua.borrow().shared.clone().expect("shared globals").get("shared").expect("shared")
    }

    /// How a call of `source` ended ([`call`]) and the work it charged (seconds of the work clock at desktop prices)
    fn charged(lua: &Lua, engine: &Rc<Engine>, source: &str) -> (String, f64) {
        let before = engine.state.borrow().sched.work_total();
        let outcome = call(lua, engine, source);
        (outcome, engine.state.borrow().sched.work_total() - before)
    }

    /// The timeout's check runs at every safepoint from now on, as from a profile's interrupt (luau_profile's
    /// `sampling_interrupt`, which holds the slot from the start), without the profile's own work
    fn check_every_safepoint(lua: &Lua) {
        unsafe extern "C-unwind" fn every(state: *mut mlua::ffi::lua_State, gc: std::ffi::c_int) {
            if gc < 0 {
                unsafe { super::check(state) }
            }
        }
        unsafe {
            let _ = lua.exec_raw::<()>((), |state| (*mlua::ffi::lua_callbacks(state)).interrupt = Some(every));
        }
    }

    /// `shared.Make(N)`: a table of N numbers, 1,003 values over and over (a sort of it charges the same in every run), made with
    /// a few library calls
    const MAKE: &str = "shared.Make = function(N)
        local Base = table.create(1000)
        for I = 1, 1000 do Base[I] = (I * 7919) % 1003 end
        local Made = table.create(N)
        for K = 0, N // 1000 - 1 do table.move(Base, 1, 1000, K * 1000 + 1, Made) end
        return Made
    end";

    /// What a sort of a fresh `shared.Make(1e6)` charges, and what making it does
    fn sort_charge() -> (f64, f64) {
        let (lua, engine, timeout) = start_exact(1000.0);
        assert_eq!(call(&lua, &engine, MAKE), "finished");
        let (_, made) = charged(&lua, &engine, "shared.Data = shared.Make(1e6)");
        let (outcome, sorted) = charged(&lua, &engine, "table.sort(shared.Data)");
        assert_eq!(outcome, "finished");
        timeout.detach();
        (sorted, made)
    }

    /// A call that alone charges past the limit, after some script work in its resumption (here before the first frame, a
    /// build, where os.clock takes no charges) or at the start of one after the frames have begun: the timeout's error comes
    /// at the first safepoint after the call, from the timeout's own interrupt and from a profile's alike. A client stops such
    /// a script there too (roblox-sim, Studio 0.747, ScriptContext:SetTimeout at half a 3,000,000-number sort's time: a sort
    /// at a resumption's start, or after a third of the limit of busy work, raised the timeout's error at the next safepoint).
    /// The watchdog's count began at its first look, after the call, so a plain run went on, while under a profile, whose
    /// interrupt had looked before the call, the same script timed out.
    #[test]
    fn a_long_call_is_followed_by_the_timeout_at_the_next_safepoint() {
        let (sort, made) = sort_charge();
        assert!(made < 0.3 * sort, "making the table charges {made} s, its sort {sort} s");
        for (frames, source) in [
            (false, "local Sum = 0 for I = 1, 1e4 do Sum += I end shared.Stage = 1 table.sort(shared.Data) shared.Stage = 2 print('after') shared.Stage = 3"),
            (true, "table.sort(shared.Data) shared.Stage = 2 print('after') shared.Stage = 3"),
        ] {
            for profile in [false, true] {
                let (lua, engine, timeout) = start_exact(0.8 * sort);
                if profile {
                    check_every_safepoint(&lua);
                }
                if frames {
                    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
                }
                assert_eq!(call(&lua, &engine, MAKE), "finished");
                assert_eq!(call(&lua, &engine, "shared.Data = shared.Make(1e6)"), "finished");
                let outcome = call(&lua, &engine, source);
                let stage: i64 = shared(&engine).get("Stage").unwrap();
                assert!(timed_out(&outcome) && stage == 2, "frames {frames}, profile {profile}: {outcome}, stage {stage}");
                assert_eq!(timeout.timeouts(), 1);
                assert!(!engine.state.borrow().output.entries.iter().any(|entry| entry.message == "after"));
                timeout.detach();
            }
        }
    }

    /// Script work and a call each under the limit and together past it, in one resumption: the timeout's error comes at the
    /// first safepoint after the call, as a client's (roblox-sim, Studio 0.747, a 0.5 s limit: 0.3 s busy, then a 0.43 s sort);
    /// the call alone runs to its end
    #[test]
    fn script_work_and_a_call_each_under_the_limit_time_out_together() {
        let (sort, _) = sort_charge();
        let limit = sort / 0.85;
        let turns = {
            let (lua, engine, timeout) = start_exact(1000.0);
            let (_, looped) = charged(&lua, &engine, "local X = 1 for I = 1, 1e5 do X = (X * 7 + 3) % 1000003 end");
            timeout.detach();
            (0.6 * limit / (looped / 1e5)).round()
        };
        for profile in [false, true] {
            let (lua, engine, timeout) = start_exact(limit);
            if profile {
                check_every_safepoint(&lua);
            }
            assert_eq!(call(&lua, &engine, MAKE), "finished");
            assert_eq!(call(&lua, &engine, "shared.Data = shared.Make(1e6)"), "finished");
            assert_eq!(call(&lua, &engine, "table.sort(shared.Data)"), "finished", "profile {profile}: the sort alone");
            assert_eq!(call(&lua, &engine, "shared.Data = shared.Make(1e6)"), "finished");
            let source = format!("local X = 1 for I = 1, {turns} do X = (X * 7 + 3) % 1000003 end shared.Stage = 1 table.sort(shared.Data) shared.Stage = 2 print('after') shared.Stage = 3");
            let outcome = call(&lua, &engine, &source);
            let stage: i64 = shared(&engine).get("Stage").unwrap();
            assert!(timed_out(&outcome) && stage == 2, "profile {profile}: {outcome}, stage {stage}");
            assert_eq!(timeout.timeouts(), 1);
            timeout.detach();
        }
    }

    /// A timeout falls on the same safepoint in every run and with a profile's interrupt: the check looks at safepoints the VM
    /// counts alike (here a busy loop's turns when it stopped). Where it fell depended on when the watchdog's thread came and
    /// on how many safepoints the check had seen (a profile's interrupt calls it at every one).
    #[test]
    fn a_timeout_falls_on_the_same_safepoint_in_every_run_and_with_a_profiles_interrupt() {
        let mut counts = Vec::new();
        for profile in [false, false, false, true, true] {
            let (lua, engine, timeout) = start_exact(0.05);
            if profile {
                check_every_safepoint(&lua);
            }
            let outcome = call(&lua, &engine, "shared.Count = 0 local X = 1 while true do shared.Count += 1 X = (X * 7 + 3) % 1000003 end");
            assert!(timed_out(&outcome), "{outcome}");
            counts.push(shared(&engine).get::<i64>("Count").unwrap());
            timeout.detach();
        }
        assert!(counts.iter().all(|&count| count == counts[0]), "turns before the timeout, plain then with a profile's interrupt: {counts:?}");
    }

    /// os.clock across a call reads what the call charged, after script work in the same resumption and at the start of a
    /// fresh one alike, once the frames have begun; before the first frame (a build) the clock takes no charges and reads 0
    /// across it, while the timeout counts that work (above)
    #[test]
    fn os_clock_across_a_call_reads_its_charge_wherever_it_sits_in_a_resumption() {
        let (lua, engine, timeout) = start_exact(1000.0);
        assert_eq!(call(&lua, &engine, MAKE), "finished");
        let measured = "local Before = os.clock() table.sort(shared.Data) shared.Took = os.clock() - Before";
        let took = |source: &str| -> f64 {
            assert_eq!(call(&lua, &engine, "shared.Data = shared.Make(1e6)"), "finished");
            assert_eq!(call(&lua, &engine, source), "finished");
            shared(&engine).get("Took").unwrap()
        };
        assert_eq!(took(measured), 0.0, "before the first frame");
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        assert_eq!(call(&lua, &engine, "shared.Data = shared.Make(1e6)"), "finished");
        let (_, sort) = charged(&lua, &engine, "table.sort(shared.Data)");
        let fresh = took(measured);
        let after_work = took(&format!("local Sum = 0 for I = 1, 1e4 do Sum += I end {measured}"));
        for (name, took) in [("fresh", fresh), ("after script work", after_work)] {
            assert!((took - sort).abs() < 1e-5, "{name}: os.clock read {took} s across a sort that charged {sort} s");
        }
        timeout.detach();
    }

    fn interrupt_installed(lua: &Lua) -> bool {
        let mut installed = false;
        unsafe {
            let _ = lua.exec_raw::<()>((), |state| installed = (*mlua::ffi::lua_callbacks(state)).interrupt.is_some());
        }
        installed
    }

    /// JSONEncode prices its values on the work clock (cost.rs `json`) as it writes them: one whose own price passes the
    /// script timeout raises the timeout's error then, its work so far charged, instead of writing on past what a script may
    /// run, and its resumption, now past the limit, has the timeout's error at its next safepoint (the charge arms the check
    /// however briefly the encode ran); one under it writes as before
    #[test]
    fn a_json_encode_whose_own_price_passes_the_timeout_raises_the_timeouts_error() {
        let (lua, engine, timeout) = start_exact(0.01);
        let outcome = call(
            &lua,
            &engine,
            r#"
            local HttpService = game:GetService("HttpService")
            local Long = table.create(300000, 0)
            local Ok, Message = pcall(function() return #HttpService:JSONEncode(Long) end)
            shared.Long = { Ok, Message }
            print("after")
            "#,
        );
        assert!(timed_out(&outcome), "{outcome}");
        let long: LuaTable = shared(&engine).get("Long").unwrap();
        assert_eq!((long.get::<bool>(1).unwrap(), long.get::<String>(2).unwrap()), (false, TIMEOUT_MESSAGE.to_string()));
        let outcome = call(&lua, &engine, r#"print("short", pcall(function() return #game:GetService("HttpService"):JSONEncode(table.create(100000, 0)) end))"#);
        assert_eq!(outcome, "finished");
        let lines: Vec<String> = engine.state.borrow().output.entries.iter().map(|entry| entry.message.clone()).collect();
        assert_eq!(lines, ["short true 200001"]);
        timeout.detach();
    }

    #[test]
    fn a_script_that_never_yields_errors_with_robloxs_timeout_and_a_pcall_does_not_keep_it_running() {
        let (lua, engine, timeout) = start(0.02, 0);
        let outcome = call(&lua, &engine, "local ok, message = pcall(function() while true do end end) print('caught', ok, message) while true do end");
        assert!(timed_out(&outcome), "{outcome}");
        assert_eq!(timeout.timeouts(), 1);
        // the next resumption runs as any other
        let outcome = call(&lua, &engine, "local n = 0 for i = 1, 1000 do n += i end print(n)");
        assert_eq!(outcome, "finished");
        timeout.detach();
    }

    #[test]
    fn a_busy_wait_on_a_frozen_clock_ends_by_the_timeout() {
        // REE_FROZEN_CLOCK=1: work moves no clock reading, so a loop waiting for os.clock, tick or os.time to pass a time
        // only drifts: Roblox's timeout ends it
        let (lua, engine, timeout) = start(0.02, 0);
        engine.state.borrow_mut().sched.freeze_wall_clock();
        for wait in [
            "local t = os.clock() while os.clock() < t + 1e9 do end",
            "local t = tick() while tick() < t + 1e9 do end",
            "local t = os.time() while os.time() < t + 1e9 do end",
        ] {
            let outcome = call(&lua, &engine, wait);
            assert!(timed_out(&outcome), "{wait}: {outcome}");
        }
        assert_eq!(timeout.timeouts(), 3);
        timeout.detach();
    }

    #[test]
    fn scripts_that_yield_are_not_timed_out_and_the_interrupt_goes_with_the_long_resumption() {
        let (lua, engine, timeout) = start(10.0, 0);
        // ~0.3 s of host time in one resumption (the watchdog arms the interrupt) but ~0.1 s of the work clock
        let outcome = call(&lua, &engine, "local n = 0 for i = 1, 3e7 do n += i end print(n)");
        assert_eq!(outcome, "finished");
        let outcome = call(&lua, &engine, "local n = 0 for i = 1, 5000 do n += i end");
        assert_eq!(outcome, "finished");
        assert!(!interrupt_installed(&lua), "the interrupt stayed after the resumption it timed ended");
        assert_eq!(timeout.timeouts(), 0);
        timeout.detach();
    }

    /// A service worker runs VM after VM on one thread, and the allocator often gives a later VM an earlier one's callbacks
    #[test]
    fn a_later_vm_on_the_thread_is_timed_from_its_own_resumption() {
        let long = "local n = 0 for i = 1, 1.5e8 do n += i end print(n)";
        let mut earlier = None;
        for _ in 0..6 {
            let (lua, engine, timeout) = start(10.0, 0);
            let mut callbacks = 0usize;
            unsafe {
                let _ = lua.exec_raw::<()>((), |state| callbacks = mlua::ffi::lua_callbacks(state) as usize);
            }
            // the thread's timing as it stands two minutes after the earlier VM's resumption, which had this one's epoch
            super::TIMING.with(|cell| {
                let timing = cell.get();
                cell.set(super::Timing { host: timing.host.map(|host| host.checked_sub(std::time::Duration::from_secs(120)).unwrap_or(host)), ..timing });
            });
            let outcome = call(&lua, &engine, long);
            timeout.detach();
            assert_eq!(outcome, "finished", "a VM's resumption was timed from an earlier VM's (same callbacks: {})", earlier == Some(callbacks));
            assert!(super::TIMING.with(std::cell::Cell::get).host.is_some(), "the watchdog timed the long resumption");
            if earlier == Some(callbacks) {
                break;
            }
            earlier = Some(callbacks);
        }
    }

    #[test]
    fn script_timeouts_stop_the_run_after_the_limit() {
        let (lua, engine, timeout) = start(0.02, 2);
        let outcome = call(&lua, &engine, "game:GetService('RunService').Heartbeat:Connect(function() while true do end end)");
        assert_eq!(outcome, "finished");
        let mut frames = 0;
        while timeout.stopped().is_none() && frames < 40 {
            crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
            frames += 1;
        }
        let why = timeout.stopped().expect("the run stops");
        // (a step of the default edit mode holds several Heartbeats)
        assert!(frames <= 2 && timeout.timeouts() == 2, "{frames} frames: {why}");
        assert!(why.starts_with("2 script timeouts") && why.contains("Test:1"), "{why}");
        // from now on every script safepoint raises: a fresh script cannot run on
        let outcome = call(&lua, &engine, "local n = 0 for i = 1, 1e6 do n += i end");
        assert!(timed_out(&outcome), "{outcome}");
        timeout.detach();
    }

    /// Two VMs on the same thread (e.g. loopback): installing, disarming or detaching the second VM
    /// must not clear the first VM's eager flag.
    #[test]
    fn disarming_one_vm_leaves_another_vms_eager_flag_intact() {
        use std::{cell::Cell, sync::atomic::Ordering};
        let (lua_a, engine_a, timeout_a) = start(0.02, 1);
        let outcome = call(&lua_a, &engine_a, "while true do end");
        assert!(timed_out(&outcome), "{outcome}");
        assert!(timeout_a.stopped().is_some());
        assert!(timeout_a.0.eager.load(Ordering::Relaxed));
        assert_eq!(super::EAGER_COUNT.with(Cell::get), 1);

        let (lua_b, engine_b, timeout_b) = start(10.0, 0);
        assert!(timeout_a.0.eager.load(Ordering::Relaxed));
        assert_eq!(super::EAGER_COUNT.with(Cell::get), 1);
        // Simulate a stale arming on VM B followed by its resumption ending (`disarm_if_stale`).
        {
            let _entered = super::enter(Some(&timeout_b));
            let epoch = timeout_b.0.epoch.load(Ordering::Acquire);
            super::arm(&timeout_b.0, epoch);
            super::set_eager(&timeout_b.0, true);
            assert_eq!(super::EAGER_COUNT.with(Cell::get), 2);
        }
        assert!(!timeout_b.0.eager.load(Ordering::Relaxed));
        assert!(timeout_a.0.eager.load(Ordering::Relaxed));
        assert_eq!(super::EAGER_COUNT.with(Cell::get), 1);
        assert_eq!(call(&lua_b, &engine_b, "local n = 0 for i = 1, 100 do n += i end"), "finished");
        timeout_b.detach();
        drop(lua_b);
        engine_b.teardown();
        assert!(timeout_a.0.eager.load(Ordering::Relaxed));
        assert_eq!(super::EAGER_COUNT.with(Cell::get), 1);

        let outcome = call(&lua_a, &engine_a, "local n = 0 for i = 1, 5 do n += i end");
        assert!(timed_out(&outcome), "{outcome}");
        timeout_a.detach();
        assert_eq!(super::EAGER_COUNT.with(Cell::get), 0);
    }
}
