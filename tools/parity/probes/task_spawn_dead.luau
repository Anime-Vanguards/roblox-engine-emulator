-- The task library given a coroutine that cannot be resumed: dead (returned or errored), running, normal (it resumed
-- another), cancelled, and a wrapped function's: what the call answers, and the errors the engine logs when it
-- resumes one later.
local LogService = game:GetService("LogService")
P.sync()
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local Seen = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageError or Kind == Enum.MessageType.MessageWarning then
		local Text = string.split(Message, "\n")[1]
		Text = string.gsub(Text, Escaped .. ":(%d+):", function(Line)
			return "@" .. (tonumber(Line) - P.base) .. ":"
		end)
		table.insert(Seen, Text)
	end
end)
local function Logged()
	for _ = 1, 4 do
		task.wait()
	end
	local Text = table.concat(Seen, " | ")
	table.clear(Seen)
	return "[" .. Text .. "]"
end
local function Dead()
	local Thread = coroutine.create(function()
		return 1
	end)
	coroutine.resume(Thread)
	return Thread
end
local function Errored()
	local Thread = coroutine.create(function()
		error("inner", 0)
	end)
	coroutine.resume(Thread)
	return Thread
end
local function Cancelled()
	local Thread = task.delay(10, function() end)
	task.cancel(Thread)
	return Thread
end

for _, Case in { { "dead", Dead }, { "errored", Errored }, { "cancelled", Cancelled } } do
	local Thread = Case[2]()
	print(Case[1] .. " status " .. coroutine.status(Thread))
	print(Case[1] .. " spawn " .. P.try(task.spawn, Thread) .. " " .. Logged())
	print(Case[1] .. " status-after " .. coroutine.status(Thread))
	Thread = Case[2]()
	print(Case[1] .. " defer " .. P.try(function()
		return typeof(task.defer(Thread))
	end) .. " " .. Logged())
	Thread = Case[2]()
	print(Case[1] .. " delay " .. P.try(function()
		return typeof(task.delay(0, Thread))
	end) .. " " .. Logged())
	Thread = Case[2]()
	print(Case[1] .. " spawn-args " .. P.try(task.spawn, Thread, 1, 2) .. " " .. Logged())
	Thread = Case[2]()
	print(Case[1] .. " resume " .. P.try(coroutine.resume, Thread) .. " " .. Logged())
	Thread = Case[2]()
	print(Case[1] .. " cancel " .. P.try(task.cancel, Thread) .. " " .. Logged())
	print(Case[1] .. " close " .. P.try(coroutine.close, Case[2]()) .. " " .. Logged())
end

-- the running thread, and a normal one
print("running spawn " .. P.try(function()
	return task.spawn(coroutine.running())
end) .. " " .. Logged())
print("running spawn-args " .. P.try(function()
	return task.spawn(coroutine.running(), 1)
end) .. " " .. Logged())
-- a thread that defers itself and then ends
local Deferred = "pending"
task.spawn(function()
	Deferred = P.try(function()
		return typeof(task.defer(coroutine.running()))
	end)
end)
print("running defer " .. Deferred .. " " .. Logged())
local Outer
Outer = coroutine.create(function()
	local Inner = coroutine.create(function()
		return P.try(task.spawn, Outer)
	end)
	local _, Text = coroutine.resume(Inner)
	return Text
end)
local _, NormalText = coroutine.resume(Outer)
print("normal spawn " .. tostring(NormalText) .. " " .. Logged())

-- a thread that waits, resumed early by task.spawn and then by its wait
local Waiting = task.spawn(function()
	local Got = table.pack(task.wait(0.5))
	print("waiter resumed " .. Got.n .. " " .. typeof(Got[1]))
end)
print("waiting spawn " .. P.try(function()
	return typeof(task.spawn(Waiting, "early"))
end) .. " " .. Logged())
task.wait(0.7)
print("waiting after " .. coroutine.status(Waiting) .. " " .. Logged())

-- coroutine.wrap's function once it returned
local Wrapped = coroutine.wrap(function()
	return 1
end)
Wrapped()
print("wrap-dead " .. P.try(Wrapped) .. " " .. Logged())
Listener:Disconnect()
