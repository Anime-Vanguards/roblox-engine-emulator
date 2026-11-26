-- Where each kind of resumed thread runs in a running game's frame and the time() it reads there, a playing client's and a
-- server's alike. wait / delay / spawn threads, and what they defer, fire, invoke or change, run at the frame's start, before
-- PreAnimation, and read the last frame's time() (the time() of the Heartbeat before them), equal to DistributedGameTime;
-- time() moves at PreAnimation. task.wait / task.delay threads, and what they fire, run after PostSimulation and read the
-- frame's (the Heartbeat after them reads the same). What a Heartbeat handler defers runs right after it. Each kind's place is
-- the simulation event before it and the one after it (a headless client renders on a cadence of its own, so the render step
-- is left out); a frame's own clock (os.clock) runs on at the frame's start, so a wait() that took two frames returns their
-- length.
local RunService = game:GetService("RunService")
local Kinds = {
	"wait",
	"delay",
	"spawn",
	"task.wait",
	"task.delay",
	"wait: task.defer",
	"wait: BindableEvent",
	"wait: BindableFunction",
	"wait: Changed",
	"task.wait: BindableEvent",
	"Heartbeat: task.defer",
}
local Seen = {}
for _, Kind in Kinds do
	Seen[Kind] = { Count = 0, Before = 0, After = 0, Game = 0, Places = {} }
end
local Recording, Running = false, true
local LastStep, LastBeat = "none", nil
local Placing, Timing = {}, {}
local function Note(Kind)
	if not Recording then
		return
	end
	local Entry = Seen[Kind]
	local Now = time()
	Entry.Count += 1
	if Now == LastBeat then
		Entry.Before += 1
	end
	if workspace.DistributedGameTime == Now then
		Entry.Game += 1
	end
	table.insert(Placing, { Entry, LastStep })
	table.insert(Timing, { Entry, Now })
end
local function Step(Name)
	for _, Waiting in Placing do
		Waiting[1].Places[Waiting[2] .. " > " .. Name] = true
	end
	table.clear(Placing)
	LastStep = Name
end
local Connections = {}
for _, Name in { "PreAnimation", "Stepped", "PreSimulation", "PostSimulation" } do
	table.insert(Connections, RunService[Name]:Connect(function()
		Step(Name)
	end))
end
local Beats = 0
table.insert(Connections, RunService.Heartbeat:Connect(function()
	Step("Heartbeat")
	local Now = time()
	for _, Waiting in Timing do
		if Waiting[2] == Now then
			Waiting[1].After += 1
		end
	end
	table.clear(Timing)
	LastBeat = Now
	Beats += 1
	-- (twelve frames recorded from the third, then two to place and time the last notes)
	Recording = Beats >= 3 and Beats < 15
	Running = Beats < 17
	task.defer(Note, "Heartbeat: task.defer")
end))
local Bindable = Instance.new("BindableEvent")
Bindable.Event:Connect(Note)
local Invoked = Instance.new("BindableFunction")
Invoked.OnInvoke = function(Kind)
	Note(Kind)
	return true
end
local Value = Instance.new("IntValue")
Value.Changed:Connect(function()
	Note("wait: Changed")
end)
local Returned = {}
task.spawn(function()
	while Running do
		local Elapsed, Second = wait()
		Note("wait")
		if Recording then
			Returned.wait = { Elapsed, Second }
			task.defer(Note, "wait: task.defer")
			Bindable:Fire("wait: BindableEvent")
			Invoked:Invoke("wait: BindableFunction")
			Value.Value += 1
		end
	end
end)
local function Delayed(...)
	Note("delay")
	if Recording then
		Returned.delay = table.pack(...)
	end
	if Running then
		delay(0, Delayed)
	end
end
delay(0, Delayed)
local function Spawned(...)
	Note("spawn")
	if Recording then
		Returned.spawn = table.pack(...)
	end
	if Running then
		spawn(Spawned)
	end
end
spawn(Spawned)
task.spawn(function()
	while Running do
		task.wait()
		Note("task.wait")
		if Recording then
			Bindable:Fire("task.wait: BindableEvent")
		end
	end
end)
local function TaskDelayed()
	Note("task.delay")
	if Running then
		task.delay(0, TaskDelayed)
	end
end
task.delay(0, TaskDelayed)
local Deadline = os.clock() + 10
while Running and os.clock() < Deadline do
	RunService.Heartbeat:Wait()
end
for _, Connection in Connections do
	Connection:Disconnect()
end
for _, Kind in Kinds do
	local Entry = Seen[Kind]
	if Entry.Count == 0 then
		print(Kind .. ": never ran")
		continue
	end
	local Places = {}
	for Place in Entry.Places do
		table.insert(Places, Place)
	end
	table.sort(Places)
	print(string.format("%s: between %s; time() of the Heartbeat before %s, of the Heartbeat after %s; DistributedGameTime equal %s", Kind,
		table.concat(Places, " | "), tostring(Entry.Before == Entry.Count), tostring(Entry.After == Entry.Count), tostring(Entry.Game == Entry.Count)))
end
local Wait = Returned.wait
print("wait() returned", Wait and typeof(Wait[1]), Wait and typeof(Wait[2]), Wait and Wait[1] >= 0.03 and Wait[1] < 0.1)
for _, Kind in { "delay", "spawn" } do
	local Got = Returned[Kind]
	print(Kind .. "'s function got", Got and Got.n, Got and typeof(Got[1]), Got and typeof(Got[2]), Got and Got.n >= 1 and Got[1] >= 0.03 and Got[1] < 0.1)
end
