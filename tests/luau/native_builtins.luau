-- Ported from runtime/tests/native_builtins.luau (Lune host). The Rust harness in native/tests/luau_vm.rs
-- provides Luau (compile/load/loadNative with Lune's semantics), Serde (encode 'json') and Process (env)
-- as globals in place of @lune/luau, @lune/serde and @lune/process. Luau.loadNative is nil when native
-- codegen is unsupported on this build/CPU; the interpreter run and its assertions still execute.
local Luau = Luau
local Serde = Serde
local Process = Process
assert(Process.env.RUR_REQUIRE_NATIVE ~= '1' or type(Luau.loadNative) == 'function', 'native host capability missing')
local function Run(Native)
	local Shared = { math = math, string = string, table = table, select = select, assert = assert,
		pcall = pcall, error = error, getfenv = getfenv, setfenv = setfenv, setmetatable = setmetatable }
	local Environment = setmetatable({}, { __index = Shared })
	local function Load(Source)
		local Options = { environment = Environment, injectGlobals = false, debugName = 'GuardedBuiltinsProbe' }
		return (if Native then Luau.loadNative(Source, Options) else Luau.load(Luau.compile(Source, { optimizationLevel = 0 }), Options))()
	end
	local Values = {}
	local function Check(Name, Actual, Expected)
		assert(Actual == Expected, Name .. ': ' .. tostring(Actual) .. ' ~= ' .. tostring(Expected))
		table.insert(Values, {Name, Actual})
	end
	local Floor = Load('return function(x) return math.floor(x) end')
	local Captured = Load('local f = math.floor; return function(x) return f(x) end')
	Check('floor number', Floor(3.75), 3)
	Shared.math = { floor = function() return 91 end }
	Check('inherited replacement', Floor(3.75), 91)
	Check('captured original alias', Captured(3.75), 3)
	Shared.math.floor = function() return 92 end
	Check('member replacement', Floor(3.75), 92)
	Shared.math = math
	Check('original restored', Floor(3.75), 3)
	getfenv(Floor).math = { floor = function() return 93 end }
	Check('getfenv replacement', Floor(3.75), 93)
	setfenv(Floor, { math = { floor = function() return 94 end } })
	Check('setfenv replacement', Floor(3.75), 94)
	local Order = Load('return function(Change) return math.floor(Change()) end')
	Check('callee resolved before argument', Order(function() Shared.math = { floor = function() return 95 end }; return 3.75 end), 3)
	Check('next call sees argument mutation', Order(function() return 3.75 end), 95)
	Shared.math = math
	local AliasedArgument = Load('return function(f, ...) return f(...) end')
	Check('callee passed as argument', AliasedArgument(math.floor, 8.9), 8)
	Check('unrelated closure argument', AliasedArgument(function() return 96 end), 96)
	local Suite = Load([[return function()
		local function Count(...) return select('#', ...), ... end
		local a, b, c = math.floor(3.75)
		assert(a == 3 and b == nil and c == nil)
		math.floor(3.75)
		local n, d, e = Count(math.floor(3.75))
		assert(n == 1 and d == 3 and e == nil)
		assert(math.floor('3.75') == 3)
		assert(not pcall(math.floor, false))
		assert(not pcall(math.floor))
		assert(not pcall(math.floor, {}))
		assert(math.floor(3.75, 99) == 3)
		local z = -0.0
		assert(1 / math.floor(z) == -math.huge)
		local bytes = table.pack(string.byte('abc', 1, 3))
		assert(bytes.n == 3 and bytes[1] == 97 and bytes[2] == 98 and bytes[3] == 99)
		local q, r, s, t = string.byte('abc', 1, 3)
		assert(q == 97 and r == 98 and s == 99 and t == nil)
		assert(string.byte('abc', 2) == 98)
		assert(string.byte('abc') == 97)
		assert(string.byte('abc', -1) == 99)
		assert(string.byte('abc', 9) == nil)
		assert(string.byte(123, 1) == 49)
		assert(string.byte('abc', '2') == 98)
		string.byte('abc', 1)
		assert(not pcall(string.byte, false))
		assert(not pcall(string.byte))
		local many = string.rep('x', 300)
		assert(Count(string.byte(many, 1, 300)) == 300)
		local values = {}
		table.insert(values, 'a')
		local u, v = table.insert(values, 'b')
		assert(u == nil and v == nil)
		assert(Count(table.insert(values, 'c')) == 0)
		table.insert(values, 2, 'd')
		assert(table.concat(values) == 'adbc')
		assert(not pcall(table.insert, table.freeze({}), 'x'))
		assert(not pcall(table.insert, false, 'x'))
		assert(not pcall(table.insert))
		local item = { value = 17 }
		for i = 1, 3000 do table.insert(values, {i, item}) end
		assert(values[3004][1] == 3000 and values[3004][2] == item)
		local recursive
		recursive = function(depth)
			if depth == 0 then return math.floor(4.5), string.byte('abc', 2) end
			local values = {}
			for i = 1, 30 do table.insert(values, {depth, i}) end
			local p, q = recursive(depth - 1)
			assert(values[30][1] == depth)
			return p, q
		end
		local p, q = recursive(90)
		assert(p == 4 and q == 98)
		local callable = setmetatable({}, { __call = function(_, x) return x + 1 end })
		assert(callable(8) == 9)
		local callableInsert = setmetatable({}, { __call = table.insert })
		assert(Count(callableInsert(123)) == 0 and callableInsert[1] == 123)
		return 123
	end]])
	for i = 1, 8 do Check('arity coercion errors allocation recursion '..i, Suite(), 123) end
	for _, Expression in {
		'math.floor(false)', 'math.floor()', 'math.floor({})',
		'string.byte(false)', 'string.byte()', 'string.byte("a", {})',
		'table.insert(false, 1)', 'table.insert()', 'table.insert(table.freeze({}), 1)',
	} do
		local Failure = Load('return function() return '..Expression..' end')
		local Ok, Message = pcall(Failure)
		Check('direct native error '..Expression, Ok, false)
		assert(#tostring(Message) > 0)
		table.insert(Values, {'full error '..Expression, tostring(Message)})
		Check('recovery after '..Expression, Captured(5.75), 5)
	end
	local Byte = Load('local b = string.byte; return function(s) return b(s, 1), string.byte(s, 1) end')
	Shared.string = { byte = function() return 97, 98 end }
	local A, B, C = Byte('z')
	Check('byte captured alias', A, 122)
	Check('byte replacement first result', B, 97)
	Check('byte replacement open result', C, 98)
	local Insert = Load('local insert = table.insert; return function(t) insert(t, 1); return table.insert(t, 2) end')
	Shared.table = { insert = function(t, value) return value + 200 end }
	local Target = {}
	Check('insert replacement return', Insert(Target), 202)
	Check('insert captured alias', Target[1], 1)
	return Serde.encode('json', Values)
end
local Reference = Run(false)
if Luau.loadNative then
	assert(Reference == Run(true), 'native builtin differential mismatch')
end
print('native builtins: callee mutation, aliases, argument order, arity, exact errors and allocation stress passed')
