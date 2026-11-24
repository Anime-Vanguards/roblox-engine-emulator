// This file is part of the Luau programming language and is licensed under MIT License; see LICENSE.txt for details
#include "luacodegen.h"

#include "Luau/CodeGen.h"

#include "lapi.h"
#include "lstate.h"

#include "Luau/FloatMode.h"

int luau_codegen_supported()
{
    return Luau::CodeGen::isSupported();
}

void luau_codegen_create(lua_State* L)
{
    Luau::CodeGen::create(L);
}

void luau_codegen_compile(lua_State* L, int idx)
{
    // (the native compiler folds constants in the float mode the code will run in: lua_setfloatmode)
    Luau::FloatModeScope floatmode(Luau::FloatMode(L->global->floatmode));
    Luau::CodeGen::CompilationOptions options;
    Luau::CodeGen::compile(L, idx, options);
}

void luau_codegen_set_tiered(lua_State* L, unsigned int threshold)
{
    Luau::CodeGen::setTieredThreshold(L, threshold);
}

void luau_codegen_set_tiered_load(lua_State* L, int enabled)
{
    Luau::CodeGen::setTieredLoad(L, enabled != 0);
}
