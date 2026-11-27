-- parity: identity=4
-- The threads a client runs a module's chunk, a BindableFunction's callback and a signal's handlers on: what each sees above it,
-- what the thread that started it sees, which require is recursive and which waits, what a module's error logs, which
-- thread a handler reuses and the memory category each starts with. A real client lets a script write Source and require
-- the module it made only at identity 4. Categories are set explicitly where a line reads them (a client job's
-- resetmemorycategory changes nothing).
local LogService = game:GetService("LogService")
local ScriptContext = game:GetService("ScriptContext")
local RunService = game:GetService("RunService")
local Folder = Instance.new("Folder")
Folder.Name = "ParityThreads"
Folder.Parent = game:GetService("ReplicatedStorage")
local function Make(Name, Source)
	local Module = Instance.new("ModuleScript")
	Module.Name = Name
	Module.Source = Source
	Module.Parent = Folder
	return Module
end
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
-- (the probe's own frames as @<line>, whichever form names them: `<chunk>:12`, `Script '<chunk>', Line 12`, `<chunk>, line 12`)
local function Norm(Text)
	local function Line(Number)
		return "@" .. (tonumber(Number) - P.base)
	end
	Text = string.gsub(tostring(Text), Escaped .. ":(%d+)", Line)
	Text = string.gsub(Text, "'" .. Escaped .. "', Line (%d+)", function(Number)
		return "'<probe>', Line " .. Line(Number)
	end)
	Text = string.gsub(Text, Escaped .. ", line (%d+)", function(Number)
		return "<probe>, line " .. Line(Number)
	end)
	return (string.gsub(Text, "\n", " / "))
end
local Start = debug.getmemorycategory()
local function Cat(Value)
	Value = Value or debug.getmemorycategory()
	if Value == Start then
		return "start"
	end
	return Norm(Value)
end
local Log = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if string.find(Message, "ParityThreads") or string.find(Message, "parity%-") or string.find(Message, "Requested module") or string.find(Message, "Module code") or string.find(Message, "no output from Luau") then
		table.insert(Log, Kind.Name .. ":" .. Norm(Message))
	end
end)
local Errors = {}
local ErrorListener = ScriptContext.Error:Connect(function(Message, Trace, Script)
	local Where = "outside"
	if Script and Script:IsDescendantOf(Folder) then
		Where = Script:GetFullName()
	end
	table.insert(Errors, Norm(Message) .. " <" .. Norm(Trace) .. "> " .. Where)
end)
local function Flush(Label)
	task.wait()
	task.wait()
	print(Label .. "-log " .. table.concat(Log, " | "))
	print(Label .. "-errors " .. table.concat(Errors, " | "))
	table.clear(Log)
	table.clear(Errors)
end
local function Bits(List)
	local Out = {}
	for I = 1, #List do
		for J = I + 1, #List do
			table.insert(Out, if rawequal(List[I], List[J]) then "1" else "0")
		end
	end
	return table.concat(Out)
end
P.sync()
shared.ParityThreads = {}
local E = shared.ParityThreads

-- 1. a module's chunk: its thread, what it sees, its category
local Probe = Make("Probe", [[
local E = shared.ParityThreads
local T = coroutine.running()
E.T = T
E.same = rawequal(T, E.requirer)
E.status = coroutine.status(E.requirer)
E.own = coroutine.status(T)
E.yieldable = coroutine.isyieldable()
E.cat = debug.getmemorycategory()
debug.setmemorycategory("parity-modset")
local S, L, N = debug.info(2, "sln")
E.info2 = tostring(S) .. " " .. tostring(L) .. " " .. tostring(N)
E.trace = debug.traceback()
return "probe"
]])
E.requirer = coroutine.running()
debug.setmemorycategory("parity-req")
local Got = require(Probe)
print("chunk got=" .. tostring(Got) .. " same=" .. tostring(E.same) .. " requirer=" .. E.status .. " own=" .. E.own .. " yieldable=" .. tostring(E.yieldable) .. " after=" .. coroutine.status(E.T))
print("chunk cat=" .. Cat(E.cat) .. " requirer-cat-after=" .. Cat())
print("chunk resume-after=" .. tostring((coroutine.resume(E.T))) .. " status=" .. coroutine.status(E.T))
print("chunk info2=" .. E.info2 .. " trace=" .. Norm(E.trace))
local Ma = Make("Ma", "shared.ParityThreads.ta = coroutine.running() return 1")
local Mb = Make("Mb", "shared.ParityThreads.tb = coroutine.running() return 1")
require(Ma)
require(Mb)
print("chunk threads-shared=" .. tostring(rawequal(E.ta, E.tb)))

-- 2. a module that waits: the requiring thread shows its require line alone, the chunk's thread the module
local Slow = Make("Slow", [[
local E = shared.ParityThreads
E.slowT = coroutine.running()
E.slowCat = debug.getmemorycategory()
task.wait(0.1)
E.slowStatusReq = coroutine.status(E.slowReq)
return "slow"
]])
debug.setmemorycategory("parity-slowreq")
local SlowReq = task.spawn(function()
	E.slowReq = coroutine.running()
	E.slowGot = require(Slow)
end)
print("slow-wait req=" .. coroutine.status(SlowReq) .. " chunk=" .. coroutine.status(E.slowT) .. " same=" .. tostring(rawequal(SlowReq, E.slowT)))
print("slow-wait req-trace=" .. Norm(debug.traceback(SlowReq)))
print("slow-wait chunk-trace=" .. Norm(debug.traceback(E.slowT)))
task.wait(0.2)
print("slow-done got=" .. tostring(E.slowGot) .. " req=" .. coroutine.status(SlowReq) .. " chunk=" .. coroutine.status(E.slowT) .. " cat=" .. Cat(E.slowCat) .. " req-in-chunk=" .. tostring(E.slowStatusReq))
Make("Inner", "return debug.traceback()")
local OuterT = Make("OuterT", "return require(script.Parent.Inner)")
print("nested-trace " .. Norm(require(OuterT)))
Make("Slow2", "task.wait(0.05) return 's2'")
local Outer2 = Make("Outer2", "shared.ParityThreads.outer2T = coroutine.running() return require(script.Parent.Slow2)")
local Outer2Thread = task.spawn(function()
	E.outer2 = require(Outer2)
end)
print("nested-wait req=" .. Norm(debug.traceback(Outer2Thread)) .. " | chunk=" .. Norm(debug.traceback(E.outer2T)) .. " | " .. coroutine.status(E.outer2T))
task.wait(0.1)
print("nested-wait done=" .. tostring(E.outer2))

-- 3. recursion: a load in the chain of loads running now raises, any other waits (a cycle with a yield waits for ever)
local Self = Make("Self", "return require(script)")
print("self " .. P.try(require, Self))
Flush("self")
Make("CycB", "return require(script.Parent.CycA)")
local CycA = Make("CycA", "return require(script.Parent.CycB)")
print("cycle " .. P.try(require, CycA))
Flush("cycle")
Make("C3B", "return require(script.Parent.C3C)")
Make("C3C", "return require(script.Parent.C3A)")
local C3A = Make("C3A", "return require(script.Parent.C3B)")
print("cycle3 " .. P.try(require, C3A))
Flush("cycle3")
Make("CycYB", "task.wait() return require(script.Parent.CycYA)")
local CycYA = Make("CycYA", "task.wait() return require(script.Parent.CycYB)")
local CycYResult
local CycYThread = task.spawn(function()
	CycYResult = P.try(require, CycYA)
end)
Make("CycZB", "return require(script.Parent.CycZA)")
local CycZA = Make("CycZA", "task.wait() return require(script.Parent.CycZB)")
local CycZResult
local CycZThread = task.spawn(function()
	CycZResult = P.try(require, CycZA)
end)
local XA = Make("XA", "task.wait(0.05) return require(script.Parent.XB)")
local XB = Make("XB", "task.wait(0.05) return require(script.Parent.XA)")
local XAResult, XBResult
local XAThread = task.spawn(function()
	XAResult = P.try(require, XA)
end)
local XBThread = task.spawn(function()
	XBResult = P.try(require, XB)
end)
task.wait(0.3)
print("cycle-yield " .. tostring(CycYResult) .. " " .. coroutine.status(CycYThread) .. " | first " .. tostring(CycZResult) .. " " .. coroutine.status(CycZThread))
print("cross " .. tostring(XAResult) .. " " .. tostring(XBResult) .. " " .. coroutine.status(XAThread) .. " " .. coroutine.status(XBThread))
Flush("cycle-yield")
local Sp = Make("Sp", [[
local E = shared.ParityThreads
task.spawn(function()
	local Ok, R = pcall(require, script)
	E.sp = tostring(Ok) .. " " .. tostring(R)
end)
E.spEarly = E.sp
return "sp"
]])
print("spawn-in-chunk got=" .. P.try(require, Sp) .. " early=" .. tostring(E.spEarly))
local Wr = Make("Wr", [[
local E = shared.ParityThreads
local Ok, R = coroutine.wrap(function()
	return pcall(require, script)
end)()
E.wr = tostring(Ok) .. " " .. tostring(R)
return "wr"
]])
print("wrap-in-chunk got=" .. P.try(require, Wr) .. " wr=" .. tostring(E.wr))
Flush("in-chunk")
local Hold = Make("Hold", "task.wait(0.1) return 'hold'")
local HoldOuter = Make("HoldOuter", [[
local E = shared.ParityThreads
local Inner = coroutine.create(function()
	return pcall(require, script.Parent.Hold)
end)
E.holdA = { coroutine.resume(Inner) }
E.holdStatus = coroutine.status(Inner)
return "ho"
]])
task.spawn(require, Hold)
print("waiting-from-chunk " .. P.try(require, HoldOuter) .. " " .. tostring(E.holdA[1]) .. " " .. tostring(E.holdA[2]) .. " " .. E.holdStatus)
local Res = Make("Res", [[
local E = shared.ParityThreads
task.wait()
local T = coroutine.create(function()
	return pcall(require, script.Parent.ResB)
end)
E.res = { coroutine.resume(T) }
return "res"
]])
Make("ResB", "return require(script.Parent.Res)")
task.spawn(function()
	E.resGot = P.try(require, Res)
end)
task.wait(0.2)
print("loader-resumes " .. tostring(E.resGot) .. " " .. tostring(E.res and E.res[1]) .. " " .. tostring(E.res and E.res[2]))
Flush("loader-resumes")

-- 4. a module's error: logged as its thread's, the requirer gets the wrapper
local Plain = Make("Plain", "local x = 1\nerror('parity-plain')")
print("err-plain " .. P.try(require, Plain))
Flush("err-plain")
local Late = Make("Late", "task.wait()\nerror('parity-late')")
local LateResult
task.spawn(function()
	LateResult = P.try(require, Late)
end)
task.wait(0.1)
print("err-late " .. tostring(LateResult))
Flush("err-late")
local Level2 = Make("Level2", "error('parity-level2', 2)")
print("err-level2 " .. P.try(require, Level2))
Flush("err-level2")
local Deep = Make("Deep", "local function f()\n\terror('parity-deep')\nend\nf()")
print("err-deep " .. P.try(require, Deep))
Flush("err-deep")
local Tab = Make("Tab", "error({})")
print("err-table " .. (string.gsub(P.try(require, Tab), "0x%x+", "ADDR")))
Flush("err-table")
task.spawn(function()
	require(Make("Unprot", "error('parity-unprot')"))
end)
Flush("err-unprotected")
local NothingLate = Make("NothingLate", "task.wait()\nlocal x = 1")
local QA, QB
task.spawn(function()
	QA = P.try(require, NothingLate)
end)
task.spawn(function()
	QB = P.try(require, NothingLate)
end)
task.wait(0.1)
print("nothing-late " .. tostring(QA) .. " | " .. tostring(QB) .. " | later " .. P.try(require, NothingLate))
local ErrLate = Make("ErrLate", "task.wait()\nerror('parity-late2')")
task.spawn(function()
	QA = P.try(require, ErrLate)
end)
task.spawn(function()
	QB = P.try(require, ErrLate)
end)
task.wait(0.1)
print("err-late-waiters " .. tostring(QA) .. " | " .. tostring(QB) .. " | later " .. P.try(require, ErrLate))
Flush("err-late-waiters")

-- 5. a chunk's thread cancelled, or yielded by coroutine.yield and resumed by a script: its requirers wait for ever
local Cn = Make("Cn", [[
local E = shared.ParityThreads
E.cnT = coroutine.running()
task.wait(0.1)
E.cnDone = true
return "cn"
]])
local CnThread = task.spawn(function()
	E.cnGot = P.try(require, Cn)
end)
print("cancel same=" .. tostring(rawequal(CnThread, E.cnT)) .. " " .. P.try(task.cancel, E.cnT))
task.wait(0.2)
local CnThread2 = task.spawn(function()
	E.cnGot2 = P.try(require, Cn)
end)
task.wait(0.05)
print("cancel req=" .. coroutine.status(CnThread) .. " got=" .. tostring(E.cnGot) .. " done=" .. tostring(E.cnDone) .. " | again " .. coroutine.status(CnThread2) .. " " .. tostring(E.cnGot2))
local Yields = Make("Yields", "shared.ParityThreads.y = coroutine.running()\ncoroutine.yield()\nreturn 'y'")
local YT = task.spawn(function()
	QA = P.try(require, Yields)
end)
QA = nil
task.wait(0.1)
print("coroutine-yield " .. tostring(QA) .. " " .. coroutine.status(YT) .. " " .. coroutine.status(E.y))
print("coroutine-yield resumed " .. tostring(coroutine.resume(E.y)) .. " " .. coroutine.status(E.y))
task.wait()
print("coroutine-yield after " .. tostring(QA) .. " " .. coroutine.status(YT))

-- 6. a BindableFunction's callback
local Function = Instance.new("BindableFunction")
local Invoker
local Threads = {}
Function.OnInvoke = function(Tag)
	local T = coroutine.running()
	Threads[Tag] = T
	local S2, L2 = debug.info(2, "sl")
	local S3 = debug.info(3, "s")
	local Out = { same = rawequal(T, Invoker), status = coroutine.status(Invoker), yieldable = coroutine.isyieldable(), cat = Cat(), info2 = Norm(tostring(S2) .. " " .. tostring(L2)), info3 = tostring(S3), trace = Norm(debug.traceback()) }
	debug.setmemorycategory("parity-handler-" .. Tag)
	return Out
end
Invoker = coroutine.running()
debug.setmemorycategory("parity-inv")
local A = Function:Invoke("a")
debug.setmemorycategory("parity-inv2")
local B = Function:Invoke("b")
print("invoke same=" .. tostring(A.same) .. " status=" .. A.status .. " yieldable=" .. tostring(A.yieldable) .. " info2=" .. A.info2 .. " info3=" .. A.info3)
print("invoke cat a=" .. A.cat .. " b=" .. B.cat .. " invoker-after=" .. Cat())
print("invoke trace=" .. A.trace)
print("invoke reuse=" .. tostring(rawequal(Threads.a, Threads.b)) .. " status=" .. coroutine.status(Threads.a) .. " resume=" .. tostring((coroutine.resume(Threads.a))))
Function.OnInvoke = function(Tag)
	local Before = Cat()
	task.wait()
	Threads[Tag] = coroutine.running()
	return Before .. "/" .. Cat()
end
debug.setmemorycategory("parity-inv-wait")
local W1 = Function:Invoke("w1")
local W2 = Function:Invoke("w2")
print("invoke-wait " .. W1 .. " " .. W2 .. " reuse=" .. tostring(rawequal(Threads.w1, Threads.w2)) .. " status=" .. coroutine.status(Threads.w1))
Function.OnInvoke = function()
	coroutine.yield("parity-yielded")
	return "resumed"
end
local YieldResult
local YieldThread = task.spawn(function()
	YieldResult = P.try(Function.Invoke, Function)
end)
task.wait(0.1)
print("invoke-yield " .. coroutine.status(YieldThread) .. " " .. tostring(YieldResult))
Function.OnInvoke = function()
	error("parity-inv-err")
end
print("invoke-err " .. P.try(Function.Invoke, Function))
Flush("invoke-err")
Function.OnInvoke = function()
	task.wait()
	error("parity-inv-late")
end
print("invoke-late " .. P.try(Function.Invoke, Function))
Flush("invoke-late")
Function.OnInvoke = function()
	error("parity-inv-level2", 2)
end
print("invoke-level2 " .. P.try(Function.Invoke, Function))
Flush("invoke-level2")
-- which thread is reused: one per BindableFunction and callback; a nested invoke takes a new one
local Seen = {}
local Fa = Instance.new("BindableFunction")
local Fb = Instance.new("BindableFunction")
local function OnInvoke()
	table.insert(Seen, coroutine.running())
end
Fa.OnInvoke = OnInvoke
Fb.OnInvoke = OnInvoke
Fa:Invoke()
Fb:Invoke()
Fa:Invoke()
Fa.OnInvoke = function()
	table.insert(Seen, coroutine.running())
end
Fa:Invoke()
print("invoke-threads " .. Bits(Seen))
table.clear(Seen)
local Fc = Instance.new("BindableFunction")
local Depth = 0
Fc.OnInvoke = function()
	table.insert(Seen, coroutine.running())
	Depth += 1
	if Depth == 1 then
		Fc:Invoke()
	end
end
Fc:Invoke()
Fc:Invoke()
print("invoke-nested " .. Bits(Seen))
table.clear(Seen)
-- the category: the setting thread's, as it has it at each invoke
local Fd = Instance.new("BindableFunction")
local Setter = coroutine.create(function()
	debug.setmemorycategory("parity-setter")
	Fd.OnInvoke = function(Wait)
		local Was = Cat()
		debug.setmemorycategory("parity-inside")
		if Wait then
			task.wait()
		end
		return Was
	end
	coroutine.yield()
	debug.setmemorycategory("parity-setter2")
	coroutine.yield()
end)
coroutine.resume(Setter)
local InvokerThread = coroutine.create(function()
	debug.setmemorycategory("parity-invoker")
	local First = Fd:Invoke()
	local Second = Fd:Invoke()
	coroutine.resume(Setter)
	local Third = Fd:Invoke(true)
	print("invoke-setter " .. First .. " " .. Second .. " " .. Third .. " after " .. Cat())
end)
coroutine.resume(InvokerThread)
task.wait(0.1)

-- 7. a signal's handlers: a thread per connection, kept while its runs return; a new one takes its connector's category
local Rec = {}
local function Record(Tag)
	table.insert(Rec, Tag .. ":" .. Cat())
end
local function Flushed(Label)
	print(Label .. " " .. table.concat(Rec, " "))
	table.clear(Rec)
end
local Event = Instance.new("BindableEvent")
local Last
Event.Event:Connect(function(Tag)
	local T = coroutine.running()
	table.insert(Rec, Tag .. ":" .. Cat() .. ":" .. tostring(rawequal(T, Last)))
	Last = T
	debug.setmemorycategory("parity-h-" .. Tag)
end)
debug.setmemorycategory("parity-fireA")
Event:Fire("a")
debug.setmemorycategory("parity-fireB")
Event:Fire("b")
debug.setmemorycategory("parity-wait")
task.wait()
Flushed("event")
Event.Event:Connect(function(Tag)
	Record("second-" .. Tag)
end)
debug.setmemorycategory("parity-fireC")
Event:Fire("c")
debug.setmemorycategory("parity-afterC")
task.wait()
Flushed("event-two")
Event.Event:Once(function(Tag)
	Record("once-" .. Tag)
end)
local Waiter = task.spawn(function()
	debug.setmemorycategory("parity-waiter")
	Event.Event:Wait()
	Record("wait")
end)
debug.setmemorycategory("parity-fireD")
Event:Fire("d")
debug.setmemorycategory("parity-afterD")
task.wait()
Flushed("event-once-wait")
local Ev1 = Instance.new("BindableEvent")
local Connector = coroutine.create(function()
	debug.setmemorycategory("parity-connector")
	Ev1.Event:Connect(function()
		Record("m1")
	end)
	coroutine.yield()
end)
coroutine.resume(Connector)
local Firer = coroutine.create(function()
	debug.setmemorycategory("parity-firer")
	Ev1:Fire()
	debug.setmemorycategory("parity-firer-after")
	coroutine.yield()
end)
coroutine.resume(Firer)
debug.setmemorycategory("parity-root")
task.wait()
Flushed("connector")
local HbConnector = coroutine.create(function()
	debug.setmemorycategory("parity-hb-connector")
	local Connection
	Connection = RunService.Heartbeat:Connect(function()
		Record("heartbeat")
		Connection:Disconnect()
	end)
	coroutine.yield()
end)
coroutine.resume(HbConnector)
debug.setmemorycategory("parity-root2")
task.wait()
task.wait()
Flushed("connector-heartbeat")
local Value = Instance.new("IntValue")
Value.Changed:Connect(function()
	Record("changed")
end)
debug.setmemorycategory("parity-write")
Value.Value = 5
debug.setmemorycategory("parity-root3")
task.wait()
Flushed("connector-changed")
local F6Seen = {}
local function F6()
	table.insert(F6Seen, coroutine.running())
end
local Ev6 = Instance.new("BindableEvent")
Ev6.Event:Connect(F6)
Ev6.Event:Connect(F6)
Ev6:Fire()
task.wait()
Ev6:Fire()
task.wait()
print("handler-threads two-connections " .. Bits(F6Seen))
table.clear(F6Seen)
local Ev9 = Instance.new("BindableEvent")
Ev9.Event:Connect(function(Mode)
	table.insert(F6Seen, coroutine.running())
	if Mode == "wait" then
		task.wait()
	elseif Mode == "error" then
		error("parity-handler")
	end
end)
for _, Mode in { "plain", "plain", "wait", "plain", "error", "plain", "plain" } do
	Ev9:Fire(Mode)
	task.wait()
	task.wait()
end
print("handler-threads after-wait-and-error " .. Bits(F6Seen))
table.clear(F6Seen)
table.clear(Log)
table.clear(Errors)
-- coroutine.wrap and create, task.defer: the maker's category
debug.setmemorycategory("parity-wrapmaker")
local Wrapped = coroutine.wrap(function()
	return Cat()
end)
local Created = coroutine.create(function()
	return Cat()
end)
debug.setmemorycategory("parity-caller")
print("wrap " .. Wrapped() .. " create " .. tostring(select(2, coroutine.resume(Created))))
debug.setmemorycategory("parity-defer")
task.defer(function()
	Record("defer")
end)
debug.setmemorycategory("parity-after-defer")
task.wait()
Flushed("defer")

-- 8. who goes on when a thread that yielded ends: a module's requirers at once (before work its chunk deferred), the first
-- requirer first; a BindableFunction's invoker at the next deferred point (after work its callback deferred)
local Order = {}
E.order = Order
local Wake = Make("Wake", [[
local Order = shared.ParityThreads.order
task.wait()
table.insert(Order, "chunk-ends")
task.defer(function() table.insert(Order, "chunk-deferred") end)
task.spawn(function() table.insert(Order, "chunk-spawned") end)
return 1
]])
task.spawn(function()
	require(Wake)
	table.insert(Order, "first-requirer")
end)
task.spawn(function()
	require(Wake)
	table.insert(Order, "second-requirer")
end)
task.defer(function()
	table.insert(Order, "outer-deferred")
end)
task.wait(0.1)
print("wake-module " .. table.concat(Order, " "))
table.clear(Order)
local Fw = Instance.new("BindableFunction")
Fw.OnInvoke = function()
	task.wait()
	table.insert(Order, "callback-ends")
	task.defer(function() table.insert(Order, "callback-deferred") end)
	return 1
end
task.spawn(function()
	Fw:Invoke()
	table.insert(Order, "invoker")
end)
task.wait(0.1)
print("wake-invoke " .. table.concat(Order, " "))

Listener:Disconnect()
ErrorListener:Disconnect()
Folder:Destroy()
shared.ParityThreads = nil
