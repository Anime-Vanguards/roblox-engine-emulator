-- Attachment Axis / SecondaryAxis writes (and their world forms and the old methods): what is refused (an axis shorter than 1e-4,
-- a SecondaryAxis whose cross with the Axis is shorter), not-a-number, an Axis along the SecondaryAxis, and the Changed events
P.sync()
local A = Instance.new("Part")
A.Anchored = true
A.CFrame = CFrame.new(0, 50, 0)
A.Parent = workspace
local function Fresh()
	local Att = Instance.new("Attachment")
	Att.Parent = A
	-- Axis (0, 1, 0), SecondaryAxis (0, 0, 1), back (1, 0, 0)
	Att.CFrame = CFrame.fromMatrix(Vector3.new(1, 2, 3), Vector3.new(0, 1, 0), Vector3.new(0, 0, 1))
	return Att
end
local function Show(Label, F)
	local Att = Fresh()
	local Fired = {}
	Att.Changed:Connect(function(Name)
		table.insert(Fired, Name)
	end)
	local Out = P.try(F, Att)
	task.wait()
	table.sort(Fired)
	print(Label, Out, P.f32s(select(4, Att.CFrame:GetComponents())), "|", table.concat(Fired, ","))
	Att:Destroy()
end
local Cases = {
	{ "zero", Vector3.zero },
	{ "negzero", Vector3.new(-0, -0, -0) },
	{ "tiny", Vector3.new(1e-20, 0, 0) },
	{ "small", Vector3.new(1e-6, 0, 0) },
	{ "nan", Vector3.new(0 / 0, 0, 0) },
	{ "along-axis", Vector3.new(0, 2, 0) },
	{ "along-secondary", Vector3.new(0, 0, 3) },
	{ "against-secondary", Vector3.new(0, 0, -3) },
	{ "back", Vector3.new(1, 0, 0) },
	{ "negback", Vector3.new(-2, 0, 0) },
	{ "a", Vector3.new(0.6, 0.8, 0) },
	{ "e", Vector3.new(0, 0.6, -0.8) },
}
for _, Case in Cases do
	Show("axis " .. Case[1], function(Att) Att.Axis = Case[2] end)
	Show("secondary " .. Case[1], function(Att) Att.SecondaryAxis = Case[2] end)
	Show("worldaxis " .. Case[1], function(Att) Att.WorldAxis = Case[2] end)
	Show("worldsecondary " .. Case[1], function(Att) Att.WorldSecondaryAxis = Case[2] end)
	Show("setaxis " .. Case[1], function(Att) Att:SetAxis(Case[2]) end)
	Show("setsecondary " .. Case[1], function(Att) Att:SetSecondaryAxis(Case[2]) end)
end
-- where a write is refused: an axis's length, a secondary axis's cross with the axis
local function Turned(Label, Name, V)
	local Att = Fresh()
	local Before = Att.CFrame
	Att[Name] = V
	print(Label, if Att.CFrame == Before then "kept" else "turned")
	Att:Destroy()
end
local R = 1 / math.sqrt(2)
for _, L in { 1.5e-5, 5e-5, 9e-5, 9.9e-5, 1.01e-4, 1.2e-4 } do
	Turned("axis length " .. L, "Axis", Vector3.new(0, L * R, L * R))
	Turned("axis length x " .. L, "Axis", Vector3.new(L, 0, 0))
end
for _, S in { 1e-6, 1e-5, 3e-5, 1e-4, 3e-4, 1e-2 } do
	Turned("secondary cross " .. S, "SecondaryAxis", Vector3.new(S, 0, 0))
	Turned("secondary near-axis " .. S, "SecondaryAxis", Vector3.new(0, 1, -S))
end
A:Destroy()
