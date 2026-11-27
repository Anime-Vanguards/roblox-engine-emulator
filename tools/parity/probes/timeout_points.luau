-- parity: identity=4 timeout=0.3
-- Which resumptions share the script timeout's timer (a resumption point), and the error each gets: busy threads of 0.25 s
-- under a 0.3 s limit (ScriptContext:SetTimeout at the capture's identity 4; ree takes the limit from the probe's timeout=).
-- Each case prints which threads ran to their end and which were cut, in the order they began, and the errors logged.
local ScriptContext = game:GetService("ScriptContext")
local LogService = game:GetService("LogService")
local RunService = game:GetService("RunService")
P.sync()
pcall(function()
	ScriptContext:SetTimeout(0.3)
end)
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local Seen = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageError then
		local Text = string.split(Message, "\n")[1]
		Text = string.gsub(Text, Escaped .. ":(%d+):", function(Line)
			return "@" .. (tonumber(Line) - P.base) .. ":"
		end)
		table.insert(Seen, Text)
	end
end)
local Ran = {}
local Order = {}
-- busy for Seconds of os.clock, reading it every 200 reads of an Instance's Name (engine work, which ree's host runs faster than
-- its work clock charges it: a loop of plain additions costs ree's host several times what it charges, so its host backstop would
-- end it first)
local Probe = Instance.new("Folder")
local function Spin(R, Seconds)
	local X
	while true do
		for _ = 1, 200 do
			X = Probe.Name
		end
		if os.clock() - R.Start >= Seconds then
			return X
		end
	end
end
local function Busy(Name, Seconds, After, Nest)
	return function()
		local R = { Start = os.clock(), Done = false }
		Ran[Name] = R
		table.insert(Order, Name)
		if Nest == 1 then
			Spin(R, Seconds)
		elseif Nest == 2 then
			local function Outer()
				Spin(R, Seconds)
			end
			Outer()
		else
			while true do
				for _ = 1, 200 do
					local _ = Probe.Name
				end
				if os.clock() - R.Start >= Seconds then
					break
				end
			end
		end
		R.Done = true
		if After then
			After()
		end
	end
end
local function Report(Case)
	for _ = 1, 8 do
		task.wait()
	end
	local Parts = {}
	for _, Name in Order do
		table.insert(Parts, Name .. " " .. (if Ran[Name].Done then "done" else "cut"))
	end
	print(Case .. ": " .. table.concat(Parts, ", ") .. " [" .. table.concat(Seen, " | ") .. "]")
	table.clear(Ran)
	table.clear(Order)
	table.clear(Seen)
	task.wait()
end
local S, L = 0.25, 0.45

task.defer(Busy("a", L))
Report("deferred, over the limit alone")
task.defer(Busy("a", S))
task.defer(Busy("b", S))
Report("deferred, two")
task.defer(Busy("a", S))
task.defer(Busy("b", S))
task.defer(Busy("c", S))
Report("deferred, three")
task.defer(Busy("a", S, function()
	task.defer(Busy("b", S))
end))
Report("deferred, the second deferred by the first")
task.defer(Busy("a", S, function()
	task.spawn(Busy("b", S))
end))
Report("deferred, the second spawned inside the first")
task.defer(Busy("a", L, nil, 1))
Report("deferred, busy in a function it called")
task.defer(Busy("a", L, nil, 2))
Report("deferred, busy two calls down")
for _, Name in { "a", "b" } do
	task.spawn(function()
		task.wait()
		Busy(Name, S)()
	end)
end
Report("task.wait resumptions")
for _, Name in { "a", "b" } do
	task.delay(0, Busy(Name, S))
end
Report("task.delay resumptions")
for _, Name in { "a", "b" } do
	spawn(function()
		wait()
		Busy(Name, S)()
	end)
end
Report("wait resumptions")
task.spawn(function()
	task.wait()
	Busy("a", S, function()
		task.defer(Busy("b", S))
	end)()
end)
Report("a task.wait resumption, then what it deferred")
RunService.Heartbeat:Once(Busy("a", S))
RunService.Heartbeat:Once(Busy("b", S))
Report("Heartbeat handlers")
task.defer(Busy("a", S))
RunService.Heartbeat:Once(Busy("b", S))
Report("a deferred thread, then a Heartbeat handler")
RunService.Heartbeat:Once(Busy("a", S, function()
	task.defer(Busy("b", S))
end))
Report("a Heartbeat handler, then what it deferred")
RunService.Stepped:Once(Busy("a", S))
RunService.PreSimulation:Once(Busy("b", S))
Report("Stepped, then PreSimulation")
RunService.PreAnimation:Once(Busy("a", S))
RunService.Stepped:Once(Busy("b", S))
Report("PreAnimation, then Stepped")
RunService.Stepped:Once(Busy("a", S))
RunService.PostSimulation:Once(Busy("b", S))
Report("Stepped, then PostSimulation")
RunService.PostSimulation:Once(Busy("a", S))
RunService.Heartbeat:Once(Busy("b", S))
Report("PostSimulation, then Heartbeat")
local Event = Instance.new("BindableEvent")
Event.Event:Once(Busy("a", S))
Event.Event:Once(Busy("b", S))
Event:Fire()
Report("BindableEvent handlers")
if RunService:IsClient() then
	RunService.RenderStepped:Once(Busy("a", S))
	RunService.PreRender:Once(Busy("b", S))
	Report("RenderStepped, then PreRender")
	local Bound = false
	local First, Second = Busy("a", S), Busy("b", S)
	RunService:BindToRenderStep("parity-timeout-a", 100, function()
		if not Bound then
			First()
		end
	end)
	RunService:BindToRenderStep("parity-timeout-b", 200, function()
		if not Bound then
			Bound = true
			Second()
		end
	end)
	Report("render-step bindings")
	RunService:UnbindFromRenderStep("parity-timeout-a")
	RunService:UnbindFromRenderStep("parity-timeout-b")
end
Listener:Disconnect()
pcall(function()
	ScriptContext:SetTimeout(0)
end)
