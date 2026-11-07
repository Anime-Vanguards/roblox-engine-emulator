// This file is part of the Luau programming language and is licensed under MIT License; see LICENSE.txt for details
// A host's profile of the interpreter (lua.h "profiling"; patches/luau.patch). While a profile steps, every thread of its
// VM runs luau_execute's single-step variant, whose dispatch calls luaP_step before each instruction:
// - it counts the instructions each function executes, by instruction;
// - it books what the work counters (lua_workcounters, priced by the host) and the host's own charges moved since the
//   previous instruction to that instruction, so a line's cost is exact and its sum is the host's clock;
// - it keeps a calling-context tree (call counts, inclusive costs, callers and callees) from the frames it sees;
// - it records the outcome of the instructions with an inline cache (GETGLOBAL / SETGLOBAL / GETTABLEKS / SETTABLEKS /
//   NAMECALL slot predictions, GETIMPORT's resolved import, FASTCALL's fast path), read before the instruction runs.
// Native code keeps running natively (its instructions are not dispatched, so not seen): a function's entries into it
// and the work booked while it ran are kept apart. Nothing here allocates on the VM's heap or changes what the
// interpreter does.
#include "lprofile.h"

#include "lua.h"
#include "lstate.h"
#include "lfunc.h"
#include "ltable.h"
#include "ltm.h"
#include "lgc.h"
#include "ldebug.h"
#include "lbytecode.h"
#include "lmem.h"
#include "lstring.h"
#include "ludata.h"
#include "lbuffer.h"

#include <atomic>
#include <string.h>
#include <string>
#include <unordered_map>
#include <vector>

thread_local lua_ProfileState* luaP_state = NULL;
thread_local uint8_t luaP_allockind = 0;
thread_local void* luaP_allocblock = NULL;

namespace
{

enum
{
    CACHE_HIT = 0,
    CACHE_MISS = 1,
    CACHE_SLOW = 2,
    CACHE_OTHER = 3,
};

struct ProfFunction
{
    Proto* proto = NULL;
    std::string source;
    std::string name;
    int linedefined = 0;
    int flags = 0;
    std::vector<uint32_t> code;
    std::vector<int> lines;
    std::vector<uint64_t> counts;
    std::vector<double> work;
    std::vector<uint64_t> caches;
    uint64_t nativeentries = 0;
    double nativework = 0;
};

struct ProfCFunction
{
    lua_CFunction f;
    const char* debugname;
    std::string name;
};

struct ProfNode
{
    int parent;
    int function;
    int callpc;
    uint64_t calls;
    uint64_t insns;
    double work;
};

struct ProfFrame
{
    int depth;
    Closure* cl;
    int node;
    bool returning;
};

struct NodeKey
{
    int parent;
    int function;
    int callpc;

    bool operator==(const NodeKey& other) const
    {
        return parent == other.parent && function == other.function && callpc == other.callpc;
    }
};

struct NodeKeyHash
{
    size_t operator()(const NodeKey& key) const
    {
        uint64_t h = uint64_t(uint32_t(key.parent)) * 0x9e3779b97f4a7c15ull;
        h ^= uint64_t(uint32_t(key.function)) * 0xc2b2ae3d27d4eb4full;
        h ^= uint64_t(uint32_t(key.callpc)) * 0x165667b19e3779f9ull;
        return size_t(h ^ (h >> 29));
    }
};

struct CFunctionKey
{
    lua_CFunction f;
    const char* debugname;

    bool operator==(const CFunctionKey& other) const
    {
        return f == other.f && debugname == other.debugname;
    }
};

struct CFunctionKeyHash
{
    size_t operator()(const CFunctionKey& key) const
    {
        return std::hash<const void*>()((const void*)key.f) ^ (std::hash<const void*>()(key.debugname) * 31);
    }
};

static_assert(sizeof(lua_WorkCounters) % sizeof(uint64_t) == 0, "the work counters are 8-byte slots");
const int kWorkSlots = int(sizeof(lua_WorkCounters) / sizeof(uint64_t));
const int kOpsSlot = int(offsetof(lua_WorkCounters, ops) / sizeof(uint64_t));
const int kHostOpsSlot = int(offsetof(lua_WorkCounters, hostops) / sizeof(uint64_t));
const int kNativeOpsSlot = int(offsetof(lua_WorkCounters, nativeops) / sizeof(uint64_t));

} // namespace

struct lua_ProfileState
{
    global_State* g = NULL;
    // the stepping interpreter counts instructions (else the profile only names functions for lua_profilestack)
    bool stepping = false;
    // work is booked (the host gave prices for the work counters' slots)
    bool pricing = false;
    std::vector<double> prices;
    std::vector<int> scalars;
    const double* hostwork = NULL;
    const double* hostdrained = NULL;
    double lasthost = 0;
    lua_WorkCounters last;
    // the instruction dispatched last, and the counts its dispatch is in: ops for a script's function, nativeops for one a code
    // generator would compile (LUAU_PROTO_WORK_NATIVE), hostops for a host's own
    int lastop = -1;
    int lastbase = 0;
    // where the next booking goes: an instruction of a function (pc -1: its native code) and its tree node, or the
    // engine (target -1: what was charged while no instruction ran)
    int target = -1;
    int targetpc = 0;
    int targetnode = 0;
    double enginework = 0;
    // the frame the last step ran in
    lua_State* lastL = NULL;
    CallInfo* lastci = NULL;
    std::vector<ProfFrame>* lastframes = NULL;
    int lastnode = 0;
    // a host's own function (lua_setworkexempt) books to the script instruction that called it
    int hosttarget = -1;
    int hosttargetpc = 0;
    int hosttargetnode = 0;
    // the node a thread's outermost frame hangs from: the root, or the current phase's node (lua_profilephase)
    int phaseroot = 0;
    // nesting of the stepping interpreter
    int depth = 0;
    // native code ran: any counter may move across a frame change
    bool nativeseen = false;
    // a fastcall whose outcome the next step tells
    lua_State* fastL = NULL;
    CallInfo* fastci = NULL;
    const Instruction* fastnext = NULL;
    int fastfunction = 0;
    int fastpc = 0;
    std::vector<ProfFunction> functions;
    std::vector<ProfCFunction> cfunctions;
    std::unordered_map<CFunctionKey, int, CFunctionKeyHash> cfunctionids;
    std::vector<ProfNode> nodes;
    std::unordered_map<NodeKey, int, NodeKeyHash> children;
    std::unordered_map<lua_State*, std::vector<ProfFrame>> threads;
    // objects marked with lua_profilemark
    void (*freed)(void* ud, void* block) = NULL;
    void* freedud = NULL;
};

static lua_ProfileState* stateof(lua_State* L)
{
    lua_ProfileState* s = luaP_state;
    return s && s->g == L->global ? s : NULL;
}

bool luaP_stepping(lua_State* L)
{
    lua_ProfileState* s = luaP_state;
    return s && s->stepping && s->g == L->global;
}

bool luaP_native(lua_State* L)
{
    return luaP_stepping(L);
}

static int functionof(lua_ProfileState* s, Proto* p)
{
    uintptr_t id = uintptr_t(p->userdata);
    if (id != 0 && id <= s->functions.size() && s->functions[id - 1].proto == p)
        return int(id - 1);

    ProfFunction f;
    f.proto = p;
    char buf[LUA_IDSIZE];
    f.source = p->source ? luaO_chunkid(buf, sizeof(buf), getstr(p->source), p->source->len) : "?";
    if (p->debugname)
        f.name = getstr(p->debugname);
    f.linedefined = p->linedefined;
    f.flags = ((p->flags & LUAU_PROTO_WORK_EXEMPT) ? LUA_PROFILE_EXEMPT : 0) | (p->execdata ? LUA_PROFILE_NATIVE : 0);
    f.code.assign(p->code, p->code + p->sizecode);
    f.lines.resize(p->sizecode);
    for (int pc = 0; pc < p->sizecode; ++pc)
        f.lines[pc] = luaG_getline(p, pc);
    if (s->stepping)
    {
        f.counts.assign(p->sizecode, 0);
        f.work.assign(p->sizecode, 0.0);
    }
    s->functions.push_back(std::move(f));
    p->userdata = (void*)uintptr_t(s->functions.size());
    return int(s->functions.size() - 1);
}

static int cfunctionof(lua_ProfileState* s, Closure* cl)
{
    CFunctionKey key = {cl->c.f, cl->c.debugname};
    auto found = s->cfunctionids.find(key);
    if (found != s->cfunctionids.end())
        return found->second;
    ProfCFunction c = {cl->c.f, cl->c.debugname, cl->c.debugname ? cl->c.debugname : ""};
    s->cfunctions.push_back(c);
    int id = int(s->cfunctions.size() - 1);
    s->cfunctionids.emplace(key, id);
    return id;
}

static int child(lua_ProfileState* s, int parent, int function, int callpc)
{
    NodeKey key = {parent, function, callpc};
    auto found = s->children.find(key);
    if (found != s->children.end())
        return found->second;
    ProfNode node = {parent, function, callpc, 0, 0, 0.0};
    s->nodes.push_back(node);
    int id = int(s->nodes.size() - 1);
    s->children.emplace(key, id);
    return id;
}

// the tree node of L's current frame, from the frames this thread showed before (an error may have unwound frames, a
// C function may have called a function again at the same depth: a frame that returned or holds another function goes)
static void reconcile(lua_ProfileState* s, lua_State* L)
{
    std::vector<ProfFrame>* frames = (L == s->lastL && s->lastframes) ? s->lastframes : &s->threads[L];
    int depth = int(L->ci - L->base_ci);
    while (!frames->empty())
    {
        ProfFrame& top = frames->back();
        if (top.depth > depth || top.returning)
        {
            frames->pop_back();
            continue;
        }
        CallInfo* ci = L->base_ci + top.depth;
        if (!ttisfunction(ci->func) || clvalue(ci->func) != top.cl)
        {
            frames->pop_back();
            continue;
        }
        break;
    }
    int parent = frames->empty() ? s->phaseroot : frames->back().node;
    for (int d = frames->empty() ? 0 : frames->back().depth + 1; d <= depth; ++d)
    {
        CallInfo* ci = L->base_ci + d;
        if (!ttisfunction(ci->func))
            continue;
        Closure* cl = clvalue(ci->func);
        int function = cl->isC ? -2 - cfunctionof(s, cl) : functionof(s, cl->l.p);
        int callpc = -1;
        if (d > 0)
        {
            CallInfo* up = ci - 1;
            if (isLua(up) && !(up->flags & LUA_CALLINFO_NATIVE))
                callpc = pcRel(up->savedpc, ci_func(up)->l.p);
        }
        int node = child(s, parent, function, callpc);
        s->nodes[node].calls++;
        frames->push_back(ProfFrame{d, cl, node, false});
        parent = node;
    }
    s->lastL = L;
    s->lastci = L->ci;
    s->lastframes = frames;
    s->lastnode = frames->empty() ? 0 : frames->back().node;
    s->hosttarget = -1;
    for (auto it = frames->rbegin(); it != frames->rend(); ++it)
    {
        if (it->cl->isC || (it->cl->l.p->flags & LUAU_PROTO_WORK_EXEMPT))
            continue;
        Proto* p = it->cl->l.p;
        CallInfo* ci = L->base_ci + it->depth;
        s->hosttarget = functionof(s, p);
        s->hosttargetpc = (ci->flags & LUA_CALLINFO_NATIVE) ? -1 : pcRel(ci->savedpc, p);
        s->hosttargetnode = it->node;
        break;
    }
}

static void add(lua_ProfileState* s, double delta)
{
    if (s->target >= 0)
    {
        ProfFunction& f = s->functions[s->target];
        if (s->targetpc >= 0 && size_t(s->targetpc) < f.work.size())
            f.work[s->targetpc] += delta;
        else
            f.nativework += delta;
        s->nodes[s->targetnode].work += delta;
    }
    else
    {
        s->enginework += delta;
    }
}

// books what the work counters and the host's charges moved since the last booking: the instruction dispatched since
// (one count in its function's instruction array) and what it did (allocations, calls, the host's charges)
static void book(lua_ProfileState* s, lua_State* L)
{
    if (!s->pricing)
        return;
    double delta = 0;
    if (s->hostwork && s->hostdrained)
    {
        double host = *s->hostwork + *s->hostdrained;
        delta = host - s->lasthost;
        s->lasthost = host;
    }
    const uint64_t* now = reinterpret_cast<const uint64_t*>(&s->g->work);
    uint64_t* last = reinterpret_cast<uint64_t*>(&s->last);
    if (s->nativeseen && (L != s->lastL || L->ci != s->lastci))
    {
        // (native code may have run since the last step and moved any instruction count)
        for (int i = 0; i < 256; ++i)
        {
            for (int base : {kOpsSlot, kHostOpsSlot, kNativeOpsSlot})
            {
                int slot = base + i;
                if (now[slot] != last[slot])
                {
                    delta += s->prices[slot] * double(int64_t(now[slot] - last[slot]));
                    last[slot] = now[slot];
                }
            }
        }
    }
    else if (s->lastop >= 0)
    {
        int slot = s->lastbase + s->lastop;
        if (now[slot] != last[slot])
        {
            delta += s->prices[slot] * double(int64_t(now[slot] - last[slot]));
            last[slot] = now[slot];
        }
    }
    for (int slot : s->scalars)
    {
        if (now[slot] != last[slot])
        {
            delta += s->prices[slot] * double(int64_t(now[slot] - last[slot]));
            last[slot] = now[slot];
        }
    }
    if (delta != 0)
        add(s, delta);
}

static void cacheoutcome(ProfFunction& f, int pc, int outcome)
{
    if (f.caches.empty())
        f.caches.assign(f.code.size() * LUA_PROFILE_CACHES, 0);
    f.caches[size_t(pc) * LUA_PROFILE_CACHES + outcome]++;
}

static bool slothit(LuaNode* n, TValue* kv)
{
    return ttisstring(gkey(n)) && tsvalue(gkey(n)) == tsvalue(kv) && !ttisnil(gval(n));
}

// what the instruction's inline cache will do, read as the interpreter reads it (the same fasttm calls in the same order,
// so the metatables' cached absence of a metamethod ends as it would)
static int classify(lua_State* L, Closure* cl, const Instruction* pc, int op)
{
    Instruction insn = *pc;
    TValue* k = cl->l.p->k;
    StkId base = L->base;
    switch (op)
    {
    case LOP_GETGLOBAL:
    case LOP_SETGLOBAL:
    {
        LuaTable* h = cl->env;
        LuaNode* n = &h->node[LUAU_INSN_C(insn) & h->nodemask8];
        bool hit = slothit(n, &k[pc[1]]) && (op == LOP_GETGLOBAL || !h->readonly);
        return hit ? CACHE_HIT : CACHE_MISS;
    }
    case LOP_GETIMPORT:
    {
        TValue* kv = &k[LUAU_INSN_D(insn)];
        if (!ttisnil(kv) && cl->env->safeenv)
            return CACHE_HIT;
        return cl->env->safeenv ? CACHE_MISS : CACHE_SLOW;
    }
    case LOP_GETTABLEKS:
    case LOP_SETTABLEKS:
    {
        StkId rb = base + LUAU_INSN_B(insn);
        if (!ttistable(rb))
            return CACHE_OTHER;
        LuaTable* h = hvalue(rb);
        LuaNode* n = &h->node[LUAU_INSN_C(insn) & h->nodemask8];
        if (slothit(n, &k[pc[1]]) && (op == LOP_GETTABLEKS || !h->readonly))
            return CACHE_HIT;
        if (op == LOP_GETTABLEKS)
            return h->metatable ? CACHE_SLOW : CACHE_MISS;
        return fastnotm(h->metatable, TM_NEWINDEX) && !h->readonly ? CACHE_MISS : CACHE_SLOW;
    }
    case LOP_NAMECALL:
    {
        StkId rb = base + LUAU_INSN_B(insn);
        TValue* kv = &k[pc[1]];
        if (ttistable(rb))
        {
            LuaTable* h = hvalue(rb);
            LuaNode* n = &h->node[tsvalue(kv)->hash & (sizenode(h) - 1)];
            if (slothit(n, kv))
                return CACHE_HIT;
            const TValue* mt = 0;
            if (gnext(n) == 0 && (mt = fasttm(L, h->metatable, TM_INDEX)) && ttistable(mt))
            {
                LuaNode* mtn = &hvalue(mt)->node[LUAU_INSN_C(insn) & hvalue(mt)->nodemask8];
                if (slothit(mtn, kv))
                    return CACHE_HIT;
            }
            return CACHE_MISS;
        }
        LuaTable* mt = ttisuserdata(rb) ? uvalue(rb)->metatable : L->global->mt[ttype(rb)];
        if (fasttm(L, mt, TM_NAMECALL))
            return CACHE_OTHER;
        const TValue* tmi = fasttm(L, mt, TM_INDEX);
        if (tmi && ttistable(tmi))
        {
            LuaTable* h = hvalue(tmi);
            return slothit(&h->node[LUAU_INSN_C(insn) & h->nodemask8], kv) ? CACHE_HIT : CACHE_MISS;
        }
        return CACHE_SLOW;
    }
    }
    return -1;
}

void luaP_step(lua_State* L, Closure* cl, const Instruction* pc)
{
    lua_ProfileState* s = luaP_state;
    if (!s->stepping || s->g != L->global)
        return;
    Proto* p = cl->l.p;
    int op = LUAU_INSN_OP(*pc);

    if (s->fastL)
    {
        bool fell = L == s->fastL && L->ci == s->fastci && pc == s->fastnext;
        cacheoutcome(s->functions[s->fastfunction], s->fastpc, fell ? CACHE_MISS : CACHE_HIT);
        s->fastL = NULL;
    }

    book(s, L);

    int function = functionof(s, p);
    if (L != s->lastL || L->ci != s->lastci)
        reconcile(s, L);
    ProfFunction& f = s->functions[function];
    int node = s->lastnode;
    bool exempt = (p->flags & LUAU_PROTO_WORK_EXEMPT) != 0;
    int at = -1;

    if (op == LOP_NATIVECALL)
    {
        // entering the function's native code, which no step sees: the work booked until the next step is its
        f.nativeentries++;
        f.flags |= LUA_PROFILE_NATIVE;
        s->nativeseen = true;
    }
    else
    {
        at = int(pc - p->code);
        if (unsigned(at) >= unsigned(f.counts.size()))
            return;
        f.counts[at]++;
        s->nodes[node].insns++;

        switch (op)
        {
        case LOP_GETGLOBAL:
        case LOP_SETGLOBAL:
        case LOP_GETIMPORT:
        case LOP_GETTABLEKS:
        case LOP_SETTABLEKS:
        case LOP_NAMECALL:
            cacheoutcome(f, at, classify(L, cl, pc, op));
            break;
        case LOP_FASTCALL:
        case LOP_FASTCALL1:
        case LOP_FASTCALL2:
        case LOP_FASTCALL2K:
        case LOP_FASTCALL3:
            if (!cl->env->safeenv)
            {
                // (an unsafe environment never takes the fast path)
                cacheoutcome(f, at, CACHE_SLOW);
            }
            else
            {
                s->fastL = L;
                s->fastci = L->ci;
                s->fastnext = pc + ((op == LOP_FASTCALL || op == LOP_FASTCALL1) ? 1 : 2);
                s->fastfunction = function;
                s->fastpc = at;
            }
            break;
        case LOP_RETURN:
            if (s->lastframes && !s->lastframes->empty())
                s->lastframes->back().returning = true;
            break;
        }
    }

    if (exempt)
    {
        s->target = s->hosttarget;
        s->targetpc = s->hosttargetpc;
        s->targetnode = s->hosttargetnode;
    }
    else
    {
        s->target = function;
        s->targetpc = at;
        s->targetnode = node;
    }
    s->lastop = op;
    s->lastbase = exempt ? kHostOpsSlot : (p->flags & LUAU_PROTO_WORK_NATIVE) ? kNativeOpsSlot : kOpsSlot;
}

void luaP_nativeenter(lua_State* L, Closure* cl)
{
    lua_ProfileState* s = luaP_state;
    if (!s || !s->stepping || s->g != L->global || cl->isC)
        return;
    book(s, L);
    int function = functionof(s, cl->l.p);
    if (L != s->lastL || L->ci != s->lastci)
        reconcile(s, L);
    ProfFunction& f = s->functions[function];
    f.nativeentries++;
    f.flags |= LUA_PROFILE_NATIVE;
    s->nativeseen = true;
    if (cl->l.p->flags & LUAU_PROTO_WORK_EXEMPT)
    {
        s->target = s->hosttarget;
        s->targetpc = s->hosttargetpc;
        s->targetnode = s->hosttargetnode;
    }
    else
    {
        s->target = function;
        s->targetpc = -1;
        s->targetnode = s->lastnode;
    }
    // (native code dispatches no instruction: none is priced at the next booking)
    s->lastop = -1;
}

void luaP_enter(lua_State* L)
{
    if (lua_ProfileState* s = stateof(L))
        s->depth++;
}

void luaP_leave(lua_State* L)
{
    lua_ProfileState* s = stateof(L);
    if (!s || s->depth == 0)
        return;
    if (--s->depth == 0 && s->stepping)
    {
        // the last instruction's charges are its own; what is charged until the next one runs is the engine's
        book(s, L);
        s->target = -1;
    }
}

void luaP_threadgone(lua_State* L, lua_State* L1)
{
    lua_ProfileState* s = stateof(L);
    if (!s)
        return;
    if (s->lastL == L1)
    {
        s->lastL = NULL;
        s->lastci = NULL;
        s->lastframes = NULL;
    }
    if (s->fastL == L1)
        s->fastL = NULL;
    s->threads.erase(L1);
}

void luaP_freed(lua_State* L, GCObject* o)
{
    lua_ProfileState* s = stateof(L);
    if (s && s->freed)
        s->freed(s->freedud, o);
}

int lua_profilestart(lua_State* L, int stepping, const double* prices, int nprices, const double* hostwork, const double* hostdrained)
{
    if (luaP_state)
        return 0;
    lua_ProfileState* s = new lua_ProfileState();
    s->g = L->global;
    s->stepping = stepping != 0;
    if (s->stepping && prices && nprices == kWorkSlots)
    {
        s->pricing = true;
        s->prices.assign(prices, prices + nprices);
        for (int slot = 0; slot < kWorkSlots; ++slot)
        {
            bool ops = (slot >= kOpsSlot && slot < kOpsSlot + 256) || (slot >= kHostOpsSlot && slot < kHostOpsSlot + 256) ||
                       (slot >= kNativeOpsSlot && slot < kNativeOpsSlot + 256);
            if (!ops)
                s->scalars.push_back(slot);
        }
        s->hostwork = hostwork;
        s->hostdrained = hostdrained;
        s->lasthost = hostwork && hostdrained ? *hostwork + *hostdrained : 0;
    }
    s->last = L->global->work;
    ProfNode root = {-1, -1, -1, 0, 0, 0.0};
    s->nodes.push_back(root);
    luaP_allockind = 0;
    luaP_allocblock = NULL;
    luaP_state = s;
    return 1;
}

void lua_profilestop(lua_State* L)
{
    lua_ProfileState* s = stateof(L);
    if (!s || !s->stepping)
        return;
    book(s, L);
    s->stepping = false;
}

int lua_profilefunctions(lua_State* L, void (*each)(void* ud, int id, const lua_ProfileFunction* f), void* ud)
{
    lua_ProfileState* s = stateof(L);
    if (!s)
        return 0;
    for (size_t i = 0; i < s->functions.size(); ++i)
    {
        const ProfFunction& f = s->functions[i];
        lua_ProfileFunction out;
        out.source = f.source.c_str();
        out.name = f.name.c_str();
        out.linedefined = f.linedefined;
        out.flags = f.flags;
        out.sizecode = int(f.code.size());
        out.code = f.code.data();
        out.lines = f.lines.data();
        out.counts = f.counts.empty() ? NULL : f.counts.data();
        out.work = f.work.empty() ? NULL : f.work.data();
        out.caches = f.caches.empty() ? NULL : f.caches.data();
        out.nativeentries = f.nativeentries;
        out.nativework = f.nativework;
        each(ud, int(i), &out);
    }
    return int(s->functions.size());
}

int lua_profilenodes(lua_State* L, void (*each)(void* ud, int id, const lua_ProfileNode* n), void* ud)
{
    lua_ProfileState* s = stateof(L);
    if (!s)
        return 0;
    for (size_t i = 0; i < s->nodes.size(); ++i)
    {
        const ProfNode& n = s->nodes[i];
        lua_ProfileNode out = {n.parent, n.function, n.callpc, n.calls, n.insns, n.work};
        each(ud, int(i), &out);
    }
    return int(s->nodes.size());
}

const char* lua_profilecfunction(lua_State* L, int index)
{
    lua_ProfileState* s = stateof(L);
    if (!s || index < 0 || size_t(index) >= s->cfunctions.size())
        return NULL;
    return s->cfunctions[index].name.c_str();
}

void lua_profilephase(lua_State* L, int phase)
{
    lua_ProfileState* s = stateof(L);
    if (!s || !s->stepping)
        return;
    // (what was charged so far is the instruction's that made it; the frames are followed again under the new phase)
    book(s, L);
    s->phaseroot = phase <= 0 ? 0 : child(s, 0, -1, phase);
    s->threads.clear();
    s->lastL = NULL;
    s->lastci = NULL;
    s->lastframes = NULL;
    s->fastL = NULL;
}

double lua_profileengine(lua_State* L)
{
    lua_ProfileState* s = stateof(L);
    return s ? s->enginework : 0;
}

void lua_profilefree(lua_State* L)
{
    lua_ProfileState* s = stateof(L);
    if (!s)
        return;
    luaP_state = NULL;
    delete s;
}

int lua_profilestack(lua_State* L, int* functions, int* lines, int max)
{
    lua_ProfileState* s = stateof(L);
    if (!s)
        return 0;
    int n = 0;
    for (CallInfo* ci = L->ci; ci > L->base_ci && n < max; --ci)
    {
        if (!ttisfunction(ci->func))
            continue;
        Closure* cl = clvalue(ci->func);
        if (cl->isC)
        {
            functions[n] = -2 - cfunctionof(s, cl);
            lines[n] = -1;
        }
        else
        {
            Proto* p = cl->l.p;
            functions[n] = functionof(s, p);
            lines[n] = (ci->flags & LUA_CALLINFO_NATIVE) ? -1 : luaG_getline(p, pcRel(ci->savedpc, p));
        }
        n++;
    }
    return n;
}

void* lua_allocblock(lua_State* L, int* kind)
{
    (void)L;
    if (kind)
        *kind = luaP_allocblock ? LUA_ALLOC_OBJECT : luaP_allockind;
    return luaP_allocblock;
}

int lua_profileobject(void* block, int* tag, void** data, const char** type)
{
    GCObject* o = (GCObject*)block;
    if (tag)
        *tag = -1;
    if (data)
        *data = NULL;
    if (type)
        *type = NULL;
    if (o->gch.tt != LUA_TUSERDATA)
        return o->gch.tt;
    Udata* u = gco2u(o);
    if (tag)
        *tag = u->tag;
    if (data)
        *data = u->data;
    if (type && u->metatable && u->metatable->node != &luaH_dummynode)
    {
        LuaTable* h = u->metatable;
        for (int i = 0; i < sizenode(h); ++i)
        {
            const LuaNode& n = h->node[i];
            if (ttisstring(&n.key) && ttisstring(&n.val) && strcmp(svalue(&n.key), "__type") == 0)
            {
                *type = svalue(&n.val);
                break;
            }
        }
    }
    return LUA_TUSERDATA;
}

size_t lua_profileudataoffset(void)
{
    return offsetof(Udata, data);
}

void lua_profilemark(void* block)
{
    GCObject* o = (GCObject*)block;
    o->gch.marked |= bitmask(PROFBIT);
}

void lua_setprofilefreed(lua_State* L, void (*freed)(void* ud, void* block), void* ud)
{
    if (lua_ProfileState* s = stateof(L))
    {
        s->freed = freed;
        s->freedud = ud;
    }
}

// The heap as the collector sees it: luaC_enumheap's nodes (lgcdebug.cpp: the same sizes and names) and every reference
// the collector follows (lgc.cpp's traverse functions and cleartable), which differs from luaC_enumheap's edges in a
// prototype's source, debug name, upvalue and local names, a thread's namecall and open upvalues, the strings of a weak
// table (never cleared) and a weak-valued array part (not followed); and its roots (markroot, atomic) as edges from NULL:
// the main thread, the registry, the basic types' metatables and the fixed objects. So an object no edge reaches is one
// the next collection frees. Objects the collector found dead and has not swept yet go to `dead`, by type and size only:
// during a sweep a dead closure can hold a prototype already freed (luaC_enumheap asserts on it).
namespace
{
struct HeapWalk
{
    lua_State* L;
    void* context;
    void (*node)(void* context, void* ptr, uint8_t tt, uint8_t memcat, size_t size, const char* name);
    void (*edge)(void* context, void* from, void* to, const char* name);
    void (*dead)(void* context, uint8_t tt, size_t size);
};

void* walkpointer(GCObject* gco)
{
    return gco->gch.tt == LUA_TUSERDATA ? (void*)gco2u(gco)->data : (void*)gco;
}

// An object's size, read from its own fields only (a dead object's references may be freed already)
size_t walksize(GCObject* o)
{
    switch (o->gch.tt)
    {
    case LUA_TSTRING:
        return sizestring(gco2ts(o)->len);
    case LUA_TTABLE:
    {
        LuaTable* h = gco2h(o);
        return sizeof(LuaTable) + (h->node == &luaH_dummynode ? 0 : sizenode(h) * sizeof(LuaNode)) + h->sizearray * sizeof(TValue);
    }
    case LUA_TFUNCTION:
    {
        Closure* cl = gco2cl(o);
        return cl->isC ? sizeCclosure(cl->nupvalues) : sizeLclosure(cl->nupvalues);
    }
    case LUA_TUSERDATA:
        return sizeudata(gco2u(o)->len);
    case LUA_TTHREAD:
    {
        lua_State* th = gco2th(o);
        return sizeof(lua_State) + sizeof(TValue) * th->stacksize + sizeof(CallInfo) * th->size_ci;
    }
    case LUA_TBUFFER:
        return sizebuffer(gco2buf(o)->len);
    case LUA_TPROTO:
    {
        Proto* p = gco2p(o);
        return sizeof(Proto) + sizeof(Instruction) * p->sizecode + sizeof(Proto*) * p->sizep + sizeof(TValue) * p->sizek + p->sizelineinfo +
               sizeof(LocVar) * p->sizelocvars + sizeof(TString*) * p->sizeupvalues;
    }
    case LUA_TUPVAL:
        return sizeof(UpVal);
    }
    return 0;
}

void walknode(HeapWalk* w, GCObject* gco, size_t size, const char* name)
{
    w->node(w->context, walkpointer(gco), gco->gch.tt, gco->gch.memcat, size, name);
}

void walkedge(HeapWalk* w, GCObject* from, GCObject* to, const char* name)
{
    w->edge(w->context, walkpointer(from), walkpointer(to), name);
}

void walkedges(HeapWalk* w, GCObject* from, TValue* data, size_t size, const char* name)
{
    for (size_t i = 0; i < size; ++i)
        if (iscollectable(&data[i]))
            walkedge(w, from, gcvalue(&data[i]), name);
}

void walktable(HeapWalk* w, LuaTable* h)
{
    walknode(w, obj2gco(h), walksize(obj2gco(h)), h == hvalue(registry(w->L)) ? "registry" : NULL);
    bool weakkey = false;
    bool weakvalue = false;
    if (const TValue* mode = gfasttm(w->L->global, h->metatable, TM_MODE))
    {
        if (ttisstring(mode))
        {
            weakkey = strchr(svalue(mode), 'k') != NULL;
            weakvalue = strchr(svalue(mode), 'v') != NULL;
        }
    }
    // (a weak reference is no edge, but a weak table keeps its strings: cleartable never clears a string)
    if (h->node != &luaH_dummynode)
    {
        for (int i = 0; i < sizenode(h); ++i)
        {
            const LuaNode& n = h->node[i];
            if (!ttisnil(&n.val) && (iscollectable(&n.key) || iscollectable(&n.val)))
            {
                if (iscollectable(&n.key) && (!weakkey || ttisstring(&n.key)))
                    walkedge(w, obj2gco(h), gcvalue(&n.key), "[key]");
                if (iscollectable(&n.val) && (!weakvalue || ttisstring(&n.val)))
                {
                    char buf[32];
                    const char* name = buf;
                    if (ttisstring(&n.key))
                        name = svalue(&n.key);
                    else if (ttisnumber(&n.key))
                        snprintf(buf, sizeof(buf), "%.14g", nvalue(&n.key));
                    else
                        snprintf(buf, sizeof(buf), "[%s]", getstr(w->L->global->ttname[n.key.tt]));
                    // (in a table weak in both, a string value stays while its key does: cleartable clears the entry of
                    // a freed key without marking the value)
                    if (weakkey && weakvalue && iscollectable(&n.key) && !ttisstring(&n.key))
                        walkedge(w, gcvalue(&n.key), gcvalue(&n.val), name);
                    else
                        walkedge(w, obj2gco(h), gcvalue(&n.val), name);
                }
            }
        }
    }
    for (int i = 0; i < h->sizearray; ++i)
        if (iscollectable(&h->array[i]) && (!weakvalue || ttisstring(&h->array[i])))
            walkedge(w, obj2gco(h), gcvalue(&h->array[i]), "array");
    if (h->metatable)
        walkedge(w, obj2gco(h), obj2gco(h->metatable), "metatable");
}

void walkclosure(HeapWalk* w, Closure* cl)
{
    if (cl->isC)
    {
        walknode(w, obj2gco(cl), walksize(obj2gco(cl)), cl->c.debugname);
    }
    else
    {
        Proto* p = cl->l.p;
        char buf[LUA_IDSIZE];
        if (p->source)
            snprintf(buf, sizeof(buf), "%s:%d %s", p->debugname ? getstr(p->debugname) : "unnamed", p->linedefined, getstr(p->source));
        else
            snprintf(buf, sizeof(buf), "%s:%d", p->debugname ? getstr(p->debugname) : "unnamed", p->linedefined);
        walknode(w, obj2gco(cl), walksize(obj2gco(cl)), buf);
    }
    walkedge(w, obj2gco(cl), obj2gco(cl->env), "env");
    if (cl->isC)
    {
        if (cl->nupvalues)
            walkedges(w, obj2gco(cl), cl->c.upvals, cl->nupvalues, "upvalue");
    }
    else
    {
        walkedge(w, obj2gco(cl), obj2gco(cl->l.p), "proto");
        if (cl->nupvalues)
            walkedges(w, obj2gco(cl), cl->l.uprefs, cl->nupvalues, "upvalue");
    }
}

void walkudata(HeapWalk* w, Udata* u)
{
    const char* name = NULL;
    if (LuaTable* h = u->metatable)
    {
        if (h->node != &luaH_dummynode)
        {
            for (int i = 0; i < sizenode(h); ++i)
            {
                const LuaNode& n = h->node[i];
                if (ttisstring(&n.key) && ttisstring(&n.val) && strcmp(svalue(&n.key), "__type") == 0)
                {
                    name = svalue(&n.val);
                    break;
                }
            }
        }
    }
    walknode(w, obj2gco(u), walksize(obj2gco(u)), name);
    if (u->metatable)
        walkedge(w, obj2gco(u), obj2gco(u->metatable), "metatable");
}

void walkthread(HeapWalk* w, lua_State* th)
{
    size_t size = walksize(obj2gco(th));
    Closure* tcl = NULL;
    for (CallInfo* ci = th->base_ci; ci <= th->ci; ++ci)
    {
        if (ttisfunction(ci->func))
        {
            tcl = clvalue(ci->func);
            break;
        }
    }
    if (tcl && !tcl->isC && tcl->l.p->source)
    {
        Proto* p = tcl->l.p;
        char buf[LUA_IDSIZE];
        snprintf(buf, sizeof(buf), "thread at %s:%d %s", p->debugname ? getstr(p->debugname) : "unnamed", p->linedefined, getstr(p->source));
        walknode(w, obj2gco(th), size, buf);
    }
    else
    {
        walknode(w, obj2gco(th), size, NULL);
    }
    walkedge(w, obj2gco(th), obj2gco(th->gt), "globals");
    if (th->top > th->stack)
        walkedges(w, obj2gco(th), th->stack, th->top - th->stack, "stack");
    // (traversestack marks these too)
    if (th->namecall)
        walkedge(w, obj2gco(th), obj2gco(th->namecall), "namecall");
    for (UpVal* uv = th->openupval; uv; uv = uv->u.open.threadnext)
        walkedge(w, obj2gco(th), obj2gco(uv), "open upvalue");
}

void walkproto(HeapWalk* w, Proto* p)
{
    size_t size = walksize(obj2gco(p));
    char buf[LUA_IDSIZE];
    if (p->source)
        snprintf(buf, sizeof(buf), "proto %s:%d %s", p->debugname ? getstr(p->debugname) : "unnamed", p->linedefined, getstr(p->source));
    else
        snprintf(buf, sizeof(buf), "proto %s:%d", p->debugname ? getstr(p->debugname) : "unnamed", p->linedefined);
    walknode(w, obj2gco(p), size, buf);
    if (p->sizek)
        walkedges(w, obj2gco(p), p->k, p->sizek, "constants");
    for (int i = 0; i < p->sizep; ++i)
        walkedge(w, obj2gco(p), obj2gco(p->p[i]), "protos");
    // (traverseproto marks the names too)
    if (p->source)
        walkedge(w, obj2gco(p), obj2gco(p->source), "source");
    if (p->debugname)
        walkedge(w, obj2gco(p), obj2gco(p->debugname), "debugname");
    for (int i = 0; i < p->sizeupvalues; ++i)
        if (p->upvalues[i])
            walkedge(w, obj2gco(p), obj2gco(p->upvalues[i]), "upvalue name");
    for (int i = 0; i < p->sizelocvars; ++i)
        if (p->locvars[i].varname)
            walkedge(w, obj2gco(p), obj2gco(p->locvars[i].varname), "local name");
}

void walkobject(HeapWalk* w, GCObject* o)
{
    switch (o->gch.tt)
    {
    case LUA_TSTRING:
        return walknode(w, o, walksize(o), NULL);
    case LUA_TTABLE:
        return walktable(w, gco2h(o));
    case LUA_TFUNCTION:
        return walkclosure(w, gco2cl(o));
    case LUA_TUSERDATA:
        return walkudata(w, gco2u(o));
    case LUA_TTHREAD:
        return walkthread(w, gco2th(o));
    case LUA_TBUFFER:
        return walknode(w, o, walksize(o), NULL);
    case LUA_TPROTO:
        return walkproto(w, gco2p(o));
    case LUA_TUPVAL:
    {
        UpVal* uv = gco2uv(o);
        walknode(w, o, walksize(o), NULL);
        if (iscollectable(uv->v))
            walkedge(w, o, gcvalue(uv->v), "value");
        return;
    }
    }
}

bool walkvisit(void* context, lua_Page* page, GCObject* gco)
{
    HeapWalk* w = (HeapWalk*)context;
    // (dead, not swept yet: garbage whose references may already be freed)
    if (isdead(w->L->global, gco))
    {
        if (w->dead)
            w->dead(w->context, gco->gch.tt, walksize(gco));
        return false;
    }
    walkobject(w, gco);
    // (the collector never frees a fixed object: the type and metamethod names, the memory error messages)
    if (isfixed(gco))
        w->edge(w->context, NULL, walkpointer(gco), "fixed");
    return false;
}
} // namespace

void lua_enumheap(
    lua_State* L,
    void* context,
    void (*node)(void* context, void* ptr, uint8_t tt, uint8_t memcat, size_t size, const char* name),
    void (*edge)(void* context, void* from, void* to, const char* name),
    void (*dead)(void* context, uint8_t tt, size_t size)
)
{
    HeapWalk w = {L, context, node, edge, dead};
    global_State* g = L->global;
    walkobject(&w, obj2gco(g->mainthread));
    // the roots markroot and atomic mark
    edge(context, NULL, walkpointer(obj2gco(g->mainthread)), "main thread");
    if (iscollectable(registry(L)))
        edge(context, NULL, walkpointer(gcvalue(registry(L))), "registry");
    for (int i = 0; i < LUA_T_COUNT; ++i)
    {
        if (g->mt[i])
        {
            char buf[64];
            snprintf(buf, sizeof(buf), "metatable of %s", getstr(g->ttname[i]));
            edge(context, NULL, walkpointer(obj2gco(g->mt[i])), buf);
        }
    }
    luaM_visitgco(L, &w, walkvisit);
}

int lua_gcstate(lua_State* L)
{
    return L->global->gcstate;
}
