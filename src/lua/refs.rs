//! Room for the Lua values the engine holds for scripts.
//!
//! mlua keeps every Lua value Rust holds (a queued, delayed or waiting thread, a deferred job's table or function argument,
//! a timer's thread) in one auxiliary thread's stack, which Luau caps at LUAI_MAXCSTACK slots (a million, [`SLOTS`]). Past
//! it mlua panics ("cannot create a Lua reference, out of auxiliary stack space") and, from a resumption that no Luau pcall
//! wraps (a PlayerAdded handler, a Heartbeat handler), the panic ended the run: exit 101 and no summary. Measured
//! 2026-10-30 on a server script that queued a task.defer per hit in a loop that never yielded. Roblox holds threads in memory with no
//! fixed count, so such a loop runs there until its script timeout; here the slots run out first, so the engine turns them
//! into a script error instead, as Roblox does for a resource a script exhausts: the offending script errors with a message
//! and its traceback, and the rest of the game goes on.
//!
//! - [`admit`]: the script calls that make the engine hold more for later raise a string error at the caller once
//!   [`WORK_LIMIT`] slots are held (task.spawn, task.defer, task.delay, spawn, delay, Debris:AddItem, BindableEvent:Fire)
//!   or [`WAIT_LIMIT`] (task.wait, wait, Signal:Wait, WaitForChild: a thread that waits holds one slot, and a thread made
//!   with coroutine.create never passed an admission). What they filled drains as the scheduler runs it, and the
//!   difference keeps the other scripts waiting and the frame loop's own conversions going meanwhile.
//! - [`is_exhaustion`]: what still reaches mlua's cap (one call with a million string arguments) errors in the script
//!   whose resumption hit it (`scheduler::resume`, `raw.rs` guard) instead of panicking out of the run, and the panic hook
//!   stays quiet about it there ([`quiet_panics`]).
//!
//! The count: mlua writes nil into a freed slot and reuses it, and never lowers the stack's top, so the top bounds what is
//! held and the non-nil slots are what is held. Below a limit the top answers (one call); past it the slots are counted,
//! and counted again only once the admissions since, each assumed to add [`PER_ADMISSION`], could have reached the limit,
//! so a refusal is always a fresh count (what queued work let go of meanwhile is free again).

use std::{cell::Cell, ffi::c_int, sync::Once};

use mlua::{ffi, prelude::*};

use super::{Flow, raise_at_caller};
use crate::Engine;

/// mlua's slots for held values: Luau's LUAI_MAXCSTACK as mlua-sys builds it (its LUA_REGISTRYINDEX is
/// -LUAI_MAXCSTACK - 2000)
pub const SLOTS: c_int = -(ffi::LUA_REGISTRYINDEX + 2000);

/// Held slots past which a call that holds more for later is refused
pub const WORK_LIMIT: c_int = SLOTS / 10 * 9;

/// Held slots past which a thread may not wait
pub const WAIT_LIMIT: c_int = SLOTS / 20 * 19;

/// Slots one admission is assumed to add between counts: the thread and a few arguments
const PER_ADMISSION: c_int = 4;

/// What an admitted call holds: more work for later, or a waiting thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hold {
    Work,
    Wait,
}

/// The held slots of one VM: its mlua reference thread, found by [`Gauge::find`].
pub struct Gauge {
    thread: *mut ffi::lua_State,
    /// Slots held at the last count plus what admissions since may have added
    held: Cell<c_int>,
    /// [`WORK_LIMIT`] and [`WAIT_LIMIT`] (tests lower them)
    limits: Cell<(c_int, c_int)>,
}

impl Gauge {
    /// The VM's reference thread: the registry's thread whose stack holds a table only Rust holds.
    pub fn find(lua: &Lua) -> Option<Gauge> {
        let marker = lua.create_table().ok()?;
        let target = marker.to_pointer();
        let mut found: *mut ffi::lua_State = std::ptr::null_mut();
        unsafe {
            lua.exec_raw::<()>((), |state| {
                ffi::lua_pushnil(state);
                while ffi::lua_next(state, ffi::LUA_REGISTRYINDEX) != 0 {
                    if ffi::lua_type(state, -1) == ffi::LUA_TTHREAD {
                        let thread = ffi::lua_tothread(state, -1);
                        if thread != state && (1..=ffi::lua_gettop(thread)).any(|index| ffi::lua_topointer(thread, index) == target) {
                            found = thread;
                        }
                    }
                    ffi::lua_pop(state, 1);
                }
            })
            .ok()?;
        }
        drop(marker);
        (!found.is_null()).then(|| Gauge { thread: found, held: Cell::new(SLOTS), limits: Cell::new((WORK_LIMIT, WAIT_LIMIT)) })
    }

    /// Other limits than [`WORK_LIMIT`] and [`WAIT_LIMIT`] (a test's, to reach them with a few thousand threads).
    pub fn set_limits(&self, work: c_int, wait: c_int) {
        self.limits.set((work, wait));
        self.held.set(SLOTS);
    }

    /// The slots held now.
    pub fn count(&self) -> c_int {
        unsafe {
            let top = ffi::lua_gettop(self.thread);
            (1..=top).filter(|&index| ffi::lua_type(self.thread, index) != ffi::LUA_TNIL).count() as c_int
        }
    }

    /// Whether one more admission stays within `limit` held slots.
    pub fn admits(&self, limit: c_int) -> bool {
        if unsafe { ffi::lua_gettop(self.thread) } < limit {
            return true;
        }
        let mut held = self.held.get();
        if held + PER_ADMISSION > limit {
            held = self.count();
        }
        let admitted = held + PER_ADMISSION <= limit;
        self.held.set(if admitted { held + PER_ADMISSION } else { held });
        admitted
    }
}

/// A call that will hold more for scripts (`call` names it in the error): a string error at the caller once the VM holds
/// its limit.
pub fn admit(engine: &Engine, hold: Hold, call: &str) -> Result<(), Flow> {
    let refs = engine.lua.borrow();
    let Some(gauge) = refs.gauge.as_ref() else { return Ok(()) };
    let (work, wait) = gauge.limits.get();
    let limit = match hold {
        Hold::Work => work,
        Hold::Wait => wait,
    };
    if gauge.admits(limit) {
        return Ok(());
    }
    let held = gauge.held.get();
    drop(refs);
    // (a task-library-style error: at the calling script line)
    raise_at_caller(format!("{call}: too many threads and values held for scripts ({held}; ree holds at most {SLOTS})"))
}

/// The error of a script whose resumption ran mlua's reference stack full.
pub fn message() -> String {
    format!("too many threads and values held for scripts (ree holds at most {SLOTS})")
}

/// Whether a panic is mlua's for a full reference stack.
pub fn is_exhaustion(payload: &(dyn std::any::Any + Send)) -> bool {
    const TEXT: &str = "cannot create a Lua reference, out of auxiliary stack space";
    payload.downcast_ref::<String>().is_some_and(|text| text.starts_with(TEXT))
        || payload.downcast_ref::<&str>().is_some_and(|text| text.starts_with(TEXT))
}

thread_local! {
    /// Resumptions the engine is inside on this thread (`scheduler::resume`)
    static RESUMING: Cell<u32> = const { Cell::new(0) };
}

/// The engine is inside a resumption while this lives: a full reference stack there becomes the script's error.
pub struct Resuming;

impl Resuming {
    pub fn enter() -> Resuming {
        RESUMING.with(|depth| depth.set(depth.get() + 1));
        Resuming
    }
}

impl Drop for Resuming {
    fn drop(&mut self) {
        RESUMING.with(|depth| depth.set(depth.get() - 1));
    }
}

/// The panic hook prints nothing for a full reference stack inside a resumption, which reports it as the script's error;
/// every other panic goes to the hook there was.
pub fn quiet_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !(is_exhaustion(info.payload()) && RESUMING.with(Cell::get) > 0) {
                previous(info);
            }
        }));
    });
}
