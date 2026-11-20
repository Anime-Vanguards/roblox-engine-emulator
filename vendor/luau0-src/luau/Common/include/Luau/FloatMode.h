// This file is part of the Luau programming language and is licensed under MIT License; see LICENSE.txt for details
#pragma once

// A host's float mode for the code it runs (lua_setfloatmode), set on the calling thread for a scope and the caller's mode put back
// when it ends. Only the mode's two bits change, and only here, in C++: the host's own code runs in them only when the scope calls it
// (a C function a script calls).
//
// FloatMode::Flush is x86's SSE flush-to-zero (MXCSR bit 15) with denormals-are-zero (bit 6) off: an operation whose result is
// subnormal gives zero, a subnormal operand is still read as itself (so reads, comparisons and scaling keep it). FloatMode::Keep clears
// both: IEEE 754's gradual underflow. Other targets have no mode that flushes results alone (AArch64's FPCR.FZ also flushes operands,
// unless FEAT_AFP's controls change it), so there nothing changes.
//
// Reading or writing the mode register waits for the float operations in flight, so the interpreter applies the mode at its outermost
// entry only (lvmexecute.cpp). The VM is built without C++ unwind semantics on MSVC (no /EHsc: a destructor does not run while an
// error unwinds its frame), so a scope puts the mode back on a normal exit only, and luaD_rawrunprotected does it once an error is
// caught.
#if (defined(_M_X64) && !defined(_M_ARM64EC)) || defined(__x86_64__)
#include <xmmintrin.h>
#define LUAU_FLOATMODE_SUPPORTED 1
#else
#define LUAU_FLOATMODE_SUPPORTED 0
#endif

namespace Luau
{

// (the values of lua.h's LUA_FLOATMODE_*)
enum class FloatMode : unsigned char
{
    Thread = 0, // the calling thread's mode, unchanged
    Flush = 1,
    Keep = 2,
};

#if LUAU_FLOATMODE_SUPPORTED
constexpr unsigned int kFloatModeBits = 0x8040; // flush-to-zero, denormals-are-zero
constexpr unsigned int kFloatModeFlush = 0x8000;

// Sets `mode` on the calling thread; true when that changed its bits, which `saved` then holds
inline bool floatModeApply(FloatMode mode, unsigned int& saved)
{
    if (mode == FloatMode::Thread)
        return false;
    unsigned int csr = _mm_getcsr();
    unsigned int want = (csr & ~kFloatModeBits) | (mode == FloatMode::Flush ? kFloatModeFlush : 0);
    if (want == csr)
        return false;
    _mm_setcsr(want);
    saved = csr & kFloatModeBits;
    return true;
}

// Puts mode bits `saved` back on the calling thread (when they differ: a write costs more than a read)
inline void floatModeRestore(unsigned int saved)
{
    unsigned int csr = _mm_getcsr();
    if ((csr & kFloatModeBits) != saved)
        _mm_setcsr((csr & ~kFloatModeBits) | saved);
}
#else
inline bool floatModeApply(FloatMode, unsigned int&)
{
    return false;
}

inline void floatModeRestore(unsigned int) {}
#endif

// `mode` for a scope that raises no error past its end (a compile)
struct FloatModeScope
{
    unsigned int saved = 0;
    bool changed;

    explicit FloatModeScope(FloatMode mode)
        : changed(floatModeApply(mode, saved))
    {
    }

    ~FloatModeScope()
    {
        if (changed)
            floatModeRestore(saved);
    }

    FloatModeScope(const FloatModeScope&) = delete;
    FloatModeScope& operator=(const FloatModeScope&) = delete;
};

} // namespace Luau
