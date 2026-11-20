// This file is part of the Luau programming language and is licensed under MIT License; see LICENSE.txt for details
#include "luacode.h"

#include "Luau/Compiler.h"
#include "Luau/ClientFlags.h"
#include "Luau/FloatMode.h"

#include <stdlib.h>
#include <string.h>

// Whether a compile folds constants in the flushing float mode (Luau/FloatMode.h): the host lists this name among the mutable globals,
// since the options' layout is the binding's to fill. No source can name it (it is no identifier), so it marks no global mutable.
static bool foldsflushed(const char* const* mutableGlobals)
{
    for (const char* const* name = mutableGlobals; name && *name; ++name)
        if (strcmp(*name, LUAU_COMPILE_FLOATMODE_FLUSH) == 0)
            return true;
    return false;
}

char* luau_compile(const char* source, size_t size, lua_CompileOptions* options, size_t* outsize)
{
    LUAU_ASSERT(outsize);

    // (the flags a live client compiles with: ClientFlags.h)
    Luau::applyClientFlags();

    Luau::CompileOptions opts;

    if (options)
    {
        static_assert(sizeof(lua_CompileOptions) == sizeof(Luau::CompileOptions), "C and C++ interface must match");
        memcpy(static_cast<void*>(&opts), options, sizeof(opts));
    }

    // (constant folding runs in the float mode the host's scripts run in: a folded `2 ^ -1074` is zero where their arithmetic flushes)
    Luau::FloatModeScope floatmode(options && foldsflushed(options->mutableGlobals) ? Luau::FloatMode::Flush : Luau::FloatMode::Thread);
    std::string result = compile(std::string(source, size), opts);

    char* copy = static_cast<char*>(malloc(result.size()));
    if (!copy)
        return nullptr;

    memcpy(copy, result.data(), result.size());
    *outsize = result.size();
    return copy;
}

void luau_set_compile_constant_nil(lua_CompileConstant* constant)
{
    Luau::setCompileConstantNil(constant);
}

void luau_set_compile_constant_boolean(lua_CompileConstant* constant, int b)
{
    Luau::setCompileConstantBoolean(constant, b != 0);
}

void luau_set_compile_constant_number(lua_CompileConstant* constant, double n)
{
    Luau::setCompileConstantNumber(constant, n);
}

void luau_set_compile_constant_vector(lua_CompileConstant* constant, float x, float y, float z, float w)
{
    Luau::setCompileConstantVector(constant, x, y, z, w);
}

void luau_set_compile_constant_vectord(lua_CompileConstant* constant, double x, double y, double z, double w)
{
    Luau::setCompileConstantVectord(constant, x, y, z, w);
}

void luau_set_compile_constant_string(lua_CompileConstant* constant, const char* s, size_t l)
{
    Luau::setCompileConstantString(constant, s, l);
}
