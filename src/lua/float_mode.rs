//! The float mode scripts run in. Roblox's engine runs its scripts with x86's SSE flush-to-zero on and denormals-are-zero off
//! (MXCSR bits 15 and 6): an operation whose result is subnormal (below 2^-1022 for a double, 2^-126 for a float32) gives zero,
//! while a subnormal operand is still read as itself, so reads, comparisons and scaling keep one. It folds constants at compile time
//! in the same mode. A client and edit mode run their render step without it: RenderStepped, PreRender and the render-step
//! bindings, the threads those signals resume, the tween step and the deferred work flushed in the step keep subnormals.
//!
//! Measured (Studio 0.747; tools/parity/probes/float_flush.luau, client and server; and the render step's handlers, binds, waits,
//! deferred work and tween events on a client, with Heartbeat, Stepped, Pre/PostSimulation, PreAnimation and resumptions beside
//! them): a server flushes everywhere; a playing client flushes everywhere but its render step; a plugin in Studio's edit DataModel
//! (Studio's own main loop) flushes in its resumptions, deferred threads and Heartbeat handlers and keeps subnormals in its
//! RenderStepped and PreRender handlers; constants fold to zero (`2 ^ -1074`, `5e-324 * 1`) in all three.
//!
//! Where it is set: in the Luau VM, not here (patches/luau/19-float-mode.patch). The VM applies its mode at its outermost entry from
//! C (a call or a resume: a nested entry only counts itself, as reading the mode register waits for the float operations in flight)
//! and puts the caller's back when that returns or once an error is caught, and luau_compile folds in it when asked (a name among
//! the compiler's mutable globals, [`COMPILE_FLUSH`]). Rust's compiler assumes the default float environment (why `_mm_setcsr` is
//! deprecated), so no Rust code changes it: the engine's own Rust outside a script's call (the scheduler, a host's math) never runs
//! in the mode, and the Rust a script calls (CFrame math, property conversions) does, as a real engine's own code runs in its
//! scripts' mode; at worst a Rust expression the compiler folded keeps a subnormal the hardware would have flushed. The mode belongs
//! to a VM, so the threads that run or compile scripts need nothing of their own: a command's main thread, `ree serve`'s workers,
//! the precompile pool (whose compiles carry the mode in their options) and a loopback run's two engines on one thread, each in its
//! own VM. Every run context folds constants in the same mode, so bytecode compiled for one serves all (lua/modules.rs).
//!
//! On a target without such a mode (all but x86-64: AArch64's FPCR.FZ also reads subnormal operands as zero, unless FEAT_AFP's
//! FIZ/AH controls are there and set, which also change NaN and min/max results; WebAssembly has none) nothing is set and scripts
//! keep subnormals everywhere, as before ([`supported`]).

use mlua::Lua;
use std::ffi::c_int;

unsafe extern "C-unwind" {
    fn lua_setfloatmode(state: *mut mlua::ffi::lua_State, mode: c_int) -> c_int;
}

const FLUSH: c_int = 1;
const KEEP: c_int = 2;

/// The compiler's mutable global that asks luau_compile to fold constants in the flushing mode (luacode.h
/// LUAU_COMPILE_FLOATMODE_FLUSH; no source can name it, so it makes no global mutable)
pub const COMPILE_FLUSH: &str = "@floatmode:flush";

fn set(lua: &Lua, mode: c_int) -> bool {
    unsafe { lua_setfloatmode(lua.state(), mode) != 0 }
}

/// Engine.Start: the VM's scripts flush subnormal results from now on.
pub fn start(lua: &Lua) {
    set(lua, FLUSH);
}

/// Whether this build can flush subnormal results without reading subnormal operands as zero (x86-64).
pub fn supported() -> bool {
    cfg!(target_arch = "x86_64")
}

/// A client's or edit mode's render step, while it lives: its scripts keep subnormals (scheduler.rs render); the flushing mode again
/// when it ends, also when the step fails.
pub struct RenderStep<'a> {
    lua: &'a Lua,
}

impl<'a> RenderStep<'a> {
    pub fn enter(lua: &'a Lua) -> RenderStep<'a> {
        set(lua, KEEP);
        RenderStep { lua }
    }
}

impl Drop for RenderStep<'_> {
    fn drop(&mut self) {
        set(self.lua, FLUSH);
    }
}
