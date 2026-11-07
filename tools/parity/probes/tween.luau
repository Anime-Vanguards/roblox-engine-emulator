-- TweenService: GetValue for every style and direction at sample times, and a running tween's values and events.
local TweenService = game:GetService("TweenService")
local Alphas = { 0, 0.05, 0.1, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.7, 0.75, 0.8, 0.9, 0.95, 1, -0.1, 1.1 }
for _, Style in Enum.EasingStyle:GetEnumItems() do
	for _, Direction in Enum.EasingDirection:GetEnumItems() do
		local Out = {}
		for _, Alpha in Alphas do
			local Ok, Value = pcall(TweenService.GetValue, TweenService, Alpha, Style, Direction)
			table.insert(Out, if Ok then P.num(Value) else "E(" .. P.err(Value) .. ")")
		end
		print("ease " .. Style.Name .. " " .. Direction.Name .. " " .. table.concat(Out, " "))
	end
end
print("getvalue-errors " .. P.try(TweenService.GetValue, TweenService, "x", Enum.EasingStyle.Linear, Enum.EasingDirection.In) .. " | " .. P.try(TweenService.GetValue, TweenService, 0.5, "Linear", "In"))

-- a tween played over frames: the values it writes at each Heartbeat and its events
P.sync()
local RunService = game:GetService("RunService")
local Value = Instance.new("NumberValue")
local Part = Instance.new("Part")
Part.Anchored = true
Part.Parent = workspace
local Log = {}
local Tween = TweenService:Create(Value, TweenInfo.new(0.09, Enum.EasingStyle.Quad, Enum.EasingDirection.Out), { Value = 10 })
local PartTween = TweenService:Create(Part, TweenInfo.new(0.09, Enum.EasingStyle.Linear), { Transparency = 1, Size = Vector3.new(10, 10, 10), CFrame = CFrame.new(10, 0, 0) * CFrame.Angles(0, math.pi, 0) })
print("tween-initial " .. tostring(Tween.PlaybackState) .. " " .. typeof(Tween) .. " " .. Tween.ClassName .. " instance " .. tostring(Tween.Instance == Value))
local Frames = 0
local Counter = RunService.Heartbeat:Connect(function()
	Frames += 1
	-- where the values stand (start, between, end): exact values follow the frame times, which jitter in a real client
	local function Class(X, From, To)
		return if X == From then "s" elseif X == To then "e" else "m"
	end
	table.insert(Log, "hb" .. Frames .. "=" .. Class(Value.Value, 0, 10) .. Class(Part.Transparency, 0, 1))
end)
Tween.Completed:Connect(function(State)
	table.insert(Log, "completed " .. tostring(State) .. " at hb" .. Frames)
end)
Value.Changed:Connect(function()
	-- count only: values come from Heartbeat reads
end)
Tween:Play()
PartTween:Play()
table.insert(Log, "after-play " .. tostring(Tween.PlaybackState) .. " value " .. P.f32(Value.Value) .. " part " .. P.f32(Part.Transparency))
for _ = 1, 10 do
	task.wait()
end
Counter:Disconnect()
print("tween-frames " .. table.concat(Log, ","))
print("tween-final " .. P.f32(Value.Value) .. " " .. P.f32s(Part.Size.X, Part.Transparency) .. " " .. P.f32s(Part.CFrame:GetComponents()) .. " " .. tostring(Tween.PlaybackState))

-- pause, cancel, replay, a second tween on the same property
local Second = Instance.new("NumberValue")
local A = TweenService:Create(Second, TweenInfo.new(0.2), { Value = 100 })
local B = TweenService:Create(Second, TweenInfo.new(0.2), { Value = -100 })
local States = {}
A.Completed:Connect(function(State)
	table.insert(States, "A " .. tostring(State))
end)
B.Completed:Connect(function(State)
	table.insert(States, "B " .. tostring(State))
end)
A:Play()
task.wait()
task.wait()
B:Play()
task.wait()
table.insert(States, "A-state " .. tostring(A.PlaybackState) .. " B-state " .. tostring(B.PlaybackState))
B:Pause()
local Paused = Second.Value
task.wait()
task.wait()
table.insert(States, "paused-holds " .. tostring(Second.Value == Paused))
B:Cancel()
table.insert(States, "cancelled " .. tostring(B.PlaybackState))
task.wait()
A:Play()
for _ = 1, 20 do
	task.wait()
end
print("tween-control " .. table.concat(States, ",") .. " final " .. P.f32(Second.Value))

-- repeats, reverses and delay
local Third = Instance.new("NumberValue")
local Repeating = TweenService:Create(Third, TweenInfo.new(0.045, Enum.EasingStyle.Linear, Enum.EasingDirection.In, 1, true, 0.025), { Value = 1 })
local Samples = {}
local Count = 0
local Sampler = RunService.Heartbeat:Connect(function()
	Count += 1
	table.insert(Samples, if Third.Value == 0 then "s" elseif Third.Value == 1 then "e" else "m")
end)
local Done = false
Repeating.Completed:Connect(function()
	Done = true
	table.insert(Samples, "done@" .. Count)
end)
Repeating:Play()
for _ = 1, 30 do
	task.wait()
	if Done then
		break
	end
end
task.wait()
Sampler:Disconnect()
print("tween-repeat " .. table.concat(Samples, ","))
print("tween-errors " .. P.try(TweenService.Create, TweenService, Value, TweenInfo.new(1), { Value = "x" }) .. " | " .. P.try(TweenService.Create, TweenService, Value, "info", {}) .. " | " .. P.try(TweenService.Create, TweenService, nil, TweenInfo.new(1), {}))
Part:Destroy()
