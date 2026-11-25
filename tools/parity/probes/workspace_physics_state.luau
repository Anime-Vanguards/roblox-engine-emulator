-- Workspace:PGSIsEnabled and GetPhysicsThrottling (services/mod.rs), and UnjoinFromOutsiders' object list (world/mod.rs):
-- what each reads as and returns (count, typeof, value), what a call on another instance or with other arguments raises, and
-- that the list is cast whole, its array part read raw, before any joint goes. unjoin_from_outsiders.luau is what it takes out.
local W = workspace
local function Describe(Value)
	if type(Value) == "number" then
		return "number:" .. P.num(Value)
	end
	return typeof(Value) .. ":" .. (string.gsub(tostring(Value), "0x%x+", "ADDR"))
end
local function Returns(F, ...)
	local Results = table.pack(pcall(F, ...))
	if not Results[1] then
		return "error " .. P.err(Results[2])
	end
	local Parts = { "n=" .. tostring(Results.n - 1) }
	for Index = 2, Results.n do
		table.insert(Parts, Describe(Results[Index]))
	end
	return "ok " .. table.concat(Parts, " ")
end
local function Show(Label, F, ...)
	print(Label, "->", Returns(F, ...))
end
local function Method(Name)
	return function(...)
		return (W :: any)[Name](...)
	end
end
local NAMES = { "PGSIsEnabled", "GetPhysicsThrottling", "UnjoinFromOutsiders" }

for _, Name in NAMES do
	Show("index " .. Name, function()
		return typeof((W :: any)[Name])
	end)
end
Show("PGSIsEnabled()", Method("PGSIsEnabled"), W)
Show("PGSIsEnabled(1, 'x')", Method("PGSIsEnabled"), W, 1, "x")
Show("GetPhysicsThrottling()", Method("GetPhysicsThrottling"), W)
Show("GetPhysicsThrottling(true)", Method("GetPhysicsThrottling"), W, true)

-- the dot call and other selves
local Model = Instance.new("Model")
local WorldModel = Instance.new("WorldModel")
local Part = Instance.new("Part")
for _, Name in NAMES do
	Show(Name .. " dot", function()
		return (W :: any)[Name]()
	end)
	for _, Self in { { "game", game }, { "Part", Part }, { "Model", Model }, { "WorldModel", WorldModel }, { "nil", nil }, { "1", 1 }, { "table", {} } } do
		Show(Name .. " self=" .. Self[1], Method(Name), Self[2], {})
	end
	Show("WorldModel." .. Name, function()
		return typeof((WorldModel :: any)[Name])
	end)
end

-- UnjoinFromOutsiders' list
local Root = Instance.new("Folder")
Root.Name = "PhysicsStateProbe"
Root.Parent = W
local A = Instance.new("Part")
A.Name = "A"
A.Anchored = true
A.CFrame = CFrame.new(200, 100, 200)
A.Parent = Root
local Lists = {
	{ "none", n = 1 },
	{ "nil", nil },
	{ "empty", {} },
	{ "{A}", { A } },
	{ "A", A },
	{ "number", 5 },
	{ "string", "x" },
	{ "Vector3", Vector3.one },
	{ "Enum", Enum.JointCreationMode.All },
	{ "{1}", { 1 } },
	{ "{'x'}", { "x" } },
	{ "{A,1}", { A, 1 } },
	{ "{1,A}", { 1, A } },
	{ "{game}", { game } },
	{ "{Folder}", { Instance.new("Folder") } },
	{ "{[2]=A}", { [2] = A } },
	{ "{x=A}", { x = A } },
	{ "{A,nil,A}", { A, nil, A } },
	{ "{Vector3}", { Vector3.one } },
}
for _, Case in Lists do
	Show("UnjoinFromOutsiders(" .. Case[1] .. ")", Method("UnjoinFromOutsiders"), W, table.unpack(Case, 2, Case.n or 2))
end
local Ok, Problem = pcall(function()
	W:UnjoinFromOutsiders(5)
end)
print("position UnjoinFromOutsiders ->", P.err(Problem))

-- the whole list is cast before any joint goes, and only its array part is read
local function Pair()
	Root:ClearAllChildren()
	local First = Instance.new("Part")
	First.Name = "A"
	First.Anchored = true
	First.CFrame = CFrame.new(200, 100, 200)
	First.Parent = Root
	local Second = First:Clone()
	Second.Name = "B"
	Second.CFrame = CFrame.new(200, 101, 200)
	Second.Parent = Root
	local Weld = Instance.new("Weld")
	Weld.Part0 = Second
	Weld.Part1 = First
	Weld.Parent = Second
	return First, Second, Weld
end
local function Case(Label, Make)
	local First, Second, Weld = Pair()
	local Result = P.try(function()
		W:UnjoinFromOutsiders(Make(First, Second))
	end)
	print("cast", Label, "->", Result, "weld", if Weld.Parent then "kept" else "removed")
end
Case("{B}", function(_, B)
	return { B }
end)
Case("{B,1}", function(_, B)
	return { B, 1 }
end)
Case("{B,nil,1}", function(_, B)
	return { B, nil, 1 }
end)
Case("{B,nil,nil,1}", function(_, B)
	return { B, nil, nil, 1 }
end)
Case("{B,x=1}", function(_, B)
	return { B, x = 1 }
end)
Case("{[2]=B}", function(_, B)
	return { [2] = B }
end)
Case("{[1]=B,[3]=B}", function(_, B)
	return { [1] = B, [3] = B }
end)
Case("{nil,B}", function(_, B)
	return { nil, B }
end)
Case("{B} after holes", function(_, B)
	local T = { 1, 2, 3, B }
	T[1], T[2], T[3] = nil, nil, nil
	return T
end)
Case("{B} metatable", function(_, B)
	return setmetatable({}, {
		__index = function(_, K)
			return if K == 1 then B else nil
		end,
		__len = function()
			return 1
		end,
	})
end)
Root:Destroy()
Part:Destroy()
