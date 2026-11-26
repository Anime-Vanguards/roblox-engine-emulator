-- TweenService:Create's casts and their order, a string for its TweenInfo, a tween of no instance; TweenInfo's and GetValue's
-- numbers that are not finite; the GUI tween methods' override and callback casts and their order
local TweenService = game:GetService("TweenService")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
P.sync()
local function Info(T)
	if typeof(T) == "Instance" and T:IsA("Tween") then
		return "Tween info=[" .. tostring(T.TweenInfo) .. "] inst=" .. tostring(T.Instance) .. " state=" .. T.PlaybackState.Name
	end
	return typeof(T) .. " " .. tostring(T)
end
local V = Instance.new("NumberValue")
V.Name = "V"
local function Try(Label, ...)
	local Ok, Result = pcall(TweenService.Create, TweenService, ...)
	print("create " .. Label .. " " .. (if Ok then "ok " .. Info(Result) else "error " .. P.err(Result)))
	return if Ok then Result else nil
end
Try("info-string", V, "info", { Value = 1 })
Try("info-number", V, 5, { Value = 1 })
Try("info-table", V, {}, { Value = 1 })
Try("info-bool", V, true, { Value = 1 })
Try("info-nil", V, nil, { Value = 1 })
Try("info-enum", V, Enum.EasingStyle.Linear, { Value = 1 })
Try("info-instance", V, workspace, { Value = 1 })
Try("info-vector", V, Vector3.one, { Value = 1 })
Try("info-string-badgoal", V, "info", { Value = "x" })
Try("info-string-badkey", V, "info", { Nope = 1 })
Try("target-nil-empty", nil, TweenInfo.new(1), {})
Try("target-nil-goal", nil, TweenInfo.new(1), { Value = 1 })
Try("target-nil-badinfo", nil, "info", {})
Try("target-nil-badgoals", nil, TweenInfo.new(1), "x")
Try("target-nil-nilgoals", nil, TweenInfo.new(1), nil)
Try("target-string", "x", TweenInfo.new(1), {})
Try("target-number", 5, TweenInfo.new(1), {})
Try("target-table", {}, TweenInfo.new(1), {})
Try("target-vector", Vector3.one, TweenInfo.new(1), {})
Try("goals-nil", V, TweenInfo.new(1), nil)
Try("goals-string", V, TweenInfo.new(1), "x")
Try("goals-number", V, TweenInfo.new(1), 5)
Try("goals-instance", V, TweenInfo.new(1), workspace)
Try("order-all-bad", "x", "info", "x")
Try("order-info-goals", V, "info", "x")
Try("order-nil-all", nil, nil, nil)
Try("none")

-- a tween made with a string for its TweenInfo, played
local W = Instance.new("NumberValue")
local Played = Try("play-string", W, "info", { Value = 10 })
if Played then
	local Completed = "no"
	Played.Completed:Connect(function(State)
		Completed = State.Name
	end)
	print("play-string play " .. P.try(Played.Play, Played) .. " " .. Played.PlaybackState.Name)
	task.wait(0.5)
	print("play-string mid " .. (if W.Value == 0 then "start" elseif W.Value == 10 then "end" else "between") .. " " .. Played.PlaybackState.Name)
	task.wait(0.8)
	print("play-string end " .. P.num(W.Value) .. " " .. Played.PlaybackState.Name .. " " .. Completed)
end
-- a tween of no instance, played
local Loose = Try("play-nil", nil, TweenInfo.new(0.1), {})
if Loose then
	local Completed = "no"
	Loose.Completed:Connect(function(State)
		Completed = State.Name
	end)
	print("play-nil play " .. P.try(Loose.Play, Loose) .. " " .. Loose.PlaybackState.Name)
	task.wait(0.3)
	print("play-nil end " .. Loose.PlaybackState.Name .. " " .. Completed)
	print("play-nil pause " .. P.try(Loose.Pause, Loose) .. " cancel " .. P.try(Loose.Cancel, Loose) .. " " .. Loose.PlaybackState.Name)
end

-- TweenInfo with numbers that are not finite
local Linear, In = Enum.EasingStyle.Linear, Enum.EasingDirection.In
local Nan = 0 / 0
for _, Case in {
	{ "time-nan", { Nan } },
	{ "time-negnan", { -Nan } },
	{ "time-inf", { math.huge } },
	{ "time-neginf", { -math.huge } },
	{ "time-neg", { -1 } },
	{ "repeat-nan", { 1, Linear, In, Nan } },
	{ "repeat-inf", { 1, Linear, In, math.huge } },
	{ "repeat-neg", { 1, Linear, In, -3 } },
	{ "repeat-big", { 1, Linear, In, 2 ^ 40 } },
	{ "delay-nan", { 1, Linear, In, 0, false, Nan } },
	{ "delay-negnan", { 1, Linear, In, 0, false, -Nan } },
	{ "delay-inf", { 1, Linear, In, 0, false, math.huge } },
	{ "delay-neg", { 1, Linear, In, 0, false, -2 } },
} do
	local Ok, Made = pcall(TweenInfo.new, table.unpack(Case[2]))
	if Ok then
		print("info " .. Case[1] .. " " .. tostring(Made) .. " | " .. tostring(Made.Time) .. " " .. tostring(Made.RepeatCount) .. " " .. tostring(Made.DelayTime) .. " | " .. P.nums(Made.Time, Made.RepeatCount, Made.DelayTime))
	else
		print("info " .. Case[1] .. " error " .. P.err(Made))
	end
end
-- GetValue with alphas that are not finite
for _, Case in {
	{ "nan", Nan },
	{ "negnan", -Nan },
	{ "inf", math.huge },
	{ "neginf", -math.huge },
	{ "nanstring", "nan" },
	{ "infstring", "inf" },
} do
	for _, Style in { "Linear", "Quad", "Elastic", "Bounce", "Back", "Sine", "Exponential", "Circular", "Cubic", "Quart", "Quint" } do
		local Ok, Value = pcall(TweenService.GetValue, TweenService, Case[2], Enum.EasingStyle[Style], Enum.EasingDirection.InOut)
		print("getvalue " .. Case[1] .. " " .. Style .. " " .. (if Ok then "ok " .. tostring(Value) .. " " .. P.num(Value) else "error " .. P.err(Value)))
	end
end
local Ok, Message = pcall(TweenService.GetValue, TweenService, Nan, Enum.EasingStyle.Spring, In)
print("getvalue nan Spring " .. (if Ok then "ok " .. tostring(Message) else "error " .. P.err(Message)))

-- the GUI tween methods' override and callback
local Frame = Instance.new("Frame")
Frame.Parent = ReplicatedStorage
local Callable = setmetatable({}, { __call = function() end })
local Called = {}
local function Callback(Label)
	return function(Status)
		table.insert(Called, Label .. ":" .. tostring(Status))
	end
end
local To = UDim2.new(0, 10, 0, 10)
local Out = Enum.EasingDirection.Out
for _, Member in { "TweenPosition", "TweenSize", "TweenSizeAndPosition" } do
	local Ends = if Member == "TweenSizeAndPosition" then { To, To } else { To }
	for _, Case in {
		{ "override-string", "x" },
		{ "override-number", 1 },
		{ "override-zero", 0 },
		{ "override-table", {} },
		{ "override-nil", nil, true },
		{ "callback-string", true, "x" },
		{ "callback-number", true, 5 },
		{ "callback-table", true, {} },
		{ "callback-callable", true, Callable },
		{ "callback-bool", true, true },
		{ "callback-instance", true, workspace },
		{ "callback-function", true, true },
	} do
		table.clear(Called)
		local Args = table.clone(Ends)
		table.insert(Args, Out)
		table.insert(Args, Enum.EasingStyle.Linear)
		table.insert(Args, 0.05)
		Args[#Ends + 4] = Case[2]
		if Case[1] == "callback-function" then
			Args[#Ends + 5] = Callback(Case[1])
		else
			Args[#Ends + 5] = Case[3]
		end
		local Result = P.try(Frame[Member], Frame, table.unpack(Args, 1, #Ends + 5))
		task.wait(0.15)
		print("gui " .. Member .. " " .. Case[1] .. " " .. Result .. " " .. table.concat(Called, ","))
	end
end
Frame:Destroy()
-- more casts
local function Try2(Label, ...)
	print("create " .. Label .. " " .. P.try(TweenService.Create, TweenService, ...))
end
Try2("string-number", "x", 5, {})
Try2("string-nil", "x", nil, {})
Try2("number-string", 5, "info", {})
Try2("nil-number", nil, 5, {})
Try2("nil-nil", nil, nil, {})
Try2("nil-number-goal", nil, 5, { Value = 1 })
Try2("info-dict", V, { a = 1 }, {})
Try2("info-array", V, { 1, 2 }, {})
Try2("info-mixed", V, { 1, a = 2 }, {})
Try2("info-empty-string", V, "", {})
Try2("info-userdata", V, newproxy(), {})
Try2("info-function", V, print, {})
Try2("goals-array", V, TweenInfo.new(1), { 1, 2 })
Try2("goals-mixed", V, TweenInfo.new(1), { 1, Value = 2 })
Try2("goals-nonstring-key", V, TweenInfo.new(1), { [true] = 1 })
Try2("goals-number-key", V, TweenInfo.new(1), { [5] = 1 })
Try2("goals-number-key-nil", nil, TweenInfo.new(1), { [5] = 1 })
Try2("target-nil-dictgoal-badvalue", nil, TweenInfo.new(1), { Value = "x" })
Try2("target-nil-badkey", nil, TweenInfo.new(1), { Nope = 1 })
local function Fields(Info)
	local Parts = {}
	for _, Name in { "Time", "DelayTime", "RepeatCount", "Reverses", "EasingStyle", "EasingDirection" } do
		local Ok, Value = pcall(function()
			return Info[Name]
		end)
		table.insert(Parts, Name .. "=" .. (if Ok then (if type(Value) == "number" then P.num(Value) else tostring(Value)) else "error " .. P.err(Value)))
	end
	return table.concat(Parts, " ") .. " [" .. tostring(Info) .. "]"
end
for _, Case in {
	{ "x", V },
	{ "x", nil },
	{ "Time:2 DelayTime:0.5 RepeatCount:3 Reverses:True EasingDirection:In EasingStyle:Sine", V },
	{ "2", V },
	{ " ", V },
} do
	local Ok, Tween = pcall(TweenService.Create, TweenService, Case[2], Case[1], {})
	print("string-info [" .. Case[1] .. "] " .. tostring(Case[2]) .. " " .. (if Ok then Fields(Tween.TweenInfo) else "error " .. P.err(Tween)))
end
-- the GUI tween callback and override: dictionaries and other values
local Frame = Instance.new("Frame")
Frame.Parent = game:GetService("ReplicatedStorage")
local To = UDim2.new(0, 10, 0, 10)
print("gui override-dict " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, nil, { a = 1 }))
print("gui callback-dict " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, nil, nil, { a = 1 }))
print("gui override-vector " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, nil, Vector3.one))
print("gui override-instance " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, nil, workspace))
print("gui override-numstring " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, nil, "1"))
print("gui callback-cfunction " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, nil, true, typeof))
print("gui callback-before-udim " .. P.try(Frame.TweenPosition, Frame, "x", nil, nil, nil, "x", "x"))
print("gui callback-before-override " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, nil, "x", "x"))
print("gui override-before-time " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, "x", "x"))
print("gui loose-callback " .. P.try(Instance.new("Frame").TweenPosition, Instance.new("Frame"), To, nil, nil, nil, nil, "x"))
task.wait(0.1)
Frame:Destroy()
-- the GUI tween methods' cast order
local Frame = Instance.new("Frame")
Frame.Parent = game:GetService("ReplicatedStorage")
local To = UDim2.new(0, 10, 0, 10)
print("order dir-style-time " .. P.try(Frame.TweenPosition, Frame, To, "bad", 13, "x"))
print("order dir-style " .. P.try(Frame.TweenPosition, Frame, To, "bad", 13))
print("order udim-dir " .. P.try(Frame.TweenPosition, Frame, "x", "bad"))
print("order size-pos " .. P.try(Frame.TweenSizeAndPosition, Frame, "x", 5))
print("order pos-dir " .. P.try(Frame.TweenSizeAndPosition, Frame, To, 5, "bad"))
print("order time-override " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, "x", true))
print("order style-spring-time " .. P.try(Frame.TweenPosition, Frame, To, nil, Enum.EasingStyle.Spring, "x"))
print("order override-number-zero " .. P.try(Frame.TweenPosition, Frame, To, nil, nil, 0.01, 0))
-- TweenService:Create: a dictionary key that is no string or number, with a bad TweenInfo
print("create key-before-info " .. P.try(game:GetService("TweenService").Create, game:GetService("TweenService"), Frame, 5, { [true] = 1 }))
print("create string-key-number " .. P.try(game:GetService("TweenService").Create, game:GetService("TweenService"), Frame, 5, { [5] = 1 }))
Frame:Destroy()
