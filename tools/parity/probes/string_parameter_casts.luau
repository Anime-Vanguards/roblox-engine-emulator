-- A Vector3 and numbers given to string parameters: the name a SharedTableRegistry name reaches, the text a MarkerCurve
-- marker holds, the message LogService:Info logs, and the name a collision group gets (PhysicsService registers groups on a
-- server only: a client's call raises). The registry lists no names, so each value's name is tried against texts it may be:
-- a client's cast (a Vector3's components with nine significant digits and NAN / INF, a number as %.20g writes it, a
-- subnormal one with its digits) and what tostring or a flushed string.format gives.
local Registry = game:GetService("SharedTableRegistry")
local LogService = game:GetService("LogService")
local PhysicsService = game:GetService("PhysicsService")
local Nan, Inf = 0 / 0, math.huge
-- (a subnormal made at run time: the compiler folds constants in the flushing float mode)
local Tiny = tonumber("1e-310")
-- (the run is a script's, in the float mode that flushes a subnormal result: docs/engine.md "Float mode"; `* 3`, since native
-- code drops a multiplication by 1)
print("flushes", Tiny * 3 == 0)
local Cases = {
	{ "vector-fraction", Vector3.new(0.1, 1 / 3, 2), { "0.100000001, 0.333333343, 2", "0.10000000149011612, 0.3333333432674408, 2" } },
	{ "vector-large-negzero", Vector3.new(1e20, 123456.789, -0.0), { "1.00000002e+20, 123456.789, -0", "100000002004087730000, 123456.7890625, -0" } },
	{ "vector-nan-inf", Vector3.new(Nan, Inf, -Inf), { "NAN, INF, -INF", "nan, inf, -inf" } },
	{ "number-tenth", 0.1, { "0.10000000000000000555", "0.1" } },
	{ "number-subnormal", Tiny, { "9.9999999999999694493e-311", "1e-310", "0" } },
	{ "number-nan", Nan, { "NAN", "nan" } },
	{ "number-inf", Inf, { "INF", "inf" } },
	{ "number-negative-inf", -Inf, { "-INF", "-inf" } },
	{ "number-2^63", 2 ^ 63, { "9223372036854775808", "9.223372036854776e+18" } },
}

-- SharedTableRegistry: the candidate the value's name reaches
for _, Case in Cases do
	local Table = SharedTable.new()
	Registry:SetSharedTable(Case[2], Table)
	local Reached = "none"
	for _, Text in Case[3] do
		if Registry:GetSharedTable(Text) == Table then
			Reached = Text
			break
		end
	end
	print("registry", Case[1], Reached)
	Registry:SetSharedTable(Case[2], nil)
	for _, Text in Case[3] do
		Registry:SetSharedTable(Text, nil)
	end
end

-- MarkerCurve: the text each marker holds (a marker's time is its case's index)
local Curve = Instance.new("MarkerCurve")
for Index, Case in Cases do
	Curve:InsertMarkerAtTime(Index, Case[2])
end
for _, Marker in Curve:GetMarkers() do
	print("marker", Cases[Marker.Time][1], Marker.Value)
end

-- LogService:Info: the message each logs, as MessageOut gets it. The warning a Vector3 gives is a MessageWarning, which the
-- check on the kind leaves out. Its stack is left out too: Studio 0.748 sent it as MessageInfo lines of their own after the
-- warning (`Stack Begin`, `Script '<name>', Line <n>`, `Stack End`, on a client and a server at script identity 2), where
-- 0.747's capture in logservice_methods.txt has it inside the warning's text.
local Logged = {}
local Connection = LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageInfo and Message ~= "Stack Begin" and Message ~= "Stack End" and string.sub(Message, 1, 8) ~= "Script '" then
		table.insert(Logged, Message)
	end
end)
for _, Case in Cases do
	LogService:Info(Case[2])
end
for _ = 1, 5 do
	task.wait()
end
Connection:Disconnect()
for Index, Case in Cases do
	print("log", Case[1], Logged[Index] or "none")
end

-- PhysicsService: the name each group gets, on a server; a client's call raises
local Registered = 0
for _, Case in Cases do
	local Ok, Problem = pcall(PhysicsService.RegisterCollisionGroup, PhysicsService, Case[2])
	if Ok then
		Registered += 1
	else
		print("group", Case[1], "error", P.err(Problem))
	end
end
if Registered > 0 then
	local Index = 0
	for _, Group in PhysicsService:GetRegisteredCollisionGroups() do
		if Group.name ~= "Default" then
			Index += 1
			print("group", Cases[Index][1], Group.name)
			PhysicsService:UnregisterCollisionGroup(Group.name)
		end
	end
end
