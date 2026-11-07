-- The API dump's own lower-case member names (deprecated aliases such as getService, brickColor, childAdded): members of
-- their own, found before any other spelling, that do what the member they stand for does.
local function Line(Label, ...)
	local Parts = { Label }
	for Index = 1, select("#", ...) do
		table.insert(Parts, (select(Index, ...)))
	end
	print(table.concat(Parts, " | "))
end
P.sync()
local Players = game:GetService("Players")
local Lighting = game:GetService("Lighting")

-- 1. methods: the same work under the alias, through a function of its own
local Part = Instance.new("Part")
Line("method-service",
	P.try(function() return game:getService("Players") == Players, game:service("Players") == Players end),
	P.try(function() return typeof(Players:getPlayers()), typeof(Players:players()) end))
Line("method-identity", P.try(function() return rawequal(Part.findFirstChild, Part.FindFirstChild), rawequal(game.getService, game.GetService) end))
Line("method-lighting", P.try(function() return Lighting:getMinutesAfterMidnight() == Lighting:GetMinutesAfterMidnight() end))
local Humanoid = Instance.new("Humanoid")
Line("method-humanoid", P.try(function() return typeof(Humanoid.takeDamage), rawequal(Humanoid.takeDamage, Humanoid.TakeDamage) end))
local Model = Instance.new("Model")
local Inner = Instance.new("Part")
Inner.Parent = Model
Model.PrimaryPart = Inner
local Exact = Instance.new("Model")
local ExactInner = Instance.new("Part")
ExactInner.Parent = Exact
Exact.PrimaryPart = ExactInner
Line("method-model",
	P.try(function() Model:moveTo(Vector3.new(0, 50, 0)) return Model:GetPivot().Position.Y > 0 end),
	P.try(function() Exact:MoveTo(Vector3.new(0, 50, 0)) return Exact:GetPivot().Position.Y > 0 end))
local Doomed = Instance.new("Folder")
Doomed.Parent = workspace
Line("method-debris", P.try(function() game:GetService("Debris"):addItem(Doomed, 0) end))

-- 2. properties: the alias reads and writes the member it stands for
Line("property-shared",
	P.try(function() Part.brickColor = BrickColor.new("Really red") return Part.BrickColor.Name end),
	P.try(function() Part.BrickColor = BrickColor.new("Lime green") return Part.brickColor.Name end))
Line("property-humanoid", P.try(function() Humanoid.maxHealth = 50 return Humanoid.MaxHealth, Humanoid.maxHealth end))
Line("property-archivable", P.try(function() local Folder = Instance.new("Folder") Folder.archivable = false return Folder.Archivable end))
Line("property-datamodel",
	P.try(function() return game.workspace == workspace, game.lighting == Lighting end),
	P.try(function() return Players.localPlayer == Players.LocalPlayer, Players.numPlayers == Players.NumPlayers end))
local Watched = Instance.new("Part")
local Names = {}
Watched.Changed:Connect(function(Name)
	table.insert(Names, Name)
end)
Watched.brickColor = BrickColor.new("Really blue")
local Control = Instance.new("Part")
local ControlNames = {}
Control.Changed:Connect(function(Name)
	table.insert(ControlNames, Name)
end)
Control.BrickColor = BrickColor.new("Really blue")

-- 3. events: a signal of its own that fires with the member it stands for
local Folder = Instance.new("Folder")
local Added = {}
Folder.childAdded:Connect(function(Child)
	table.insert(Added, Child.Name)
end)
local Value = Instance.new("IntValue")
local Values = {}
Value.changed:Connect(function(...)
	table.insert(Values, P.nums(...))
end)
Line("event-signal",
	P.try(function() return tostring(Folder.childAdded), Folder.childAdded == Folder.ChildAdded end),
	P.try(function() return tostring(Value.changed), Value.changed == Value.Changed end))
local Child = Instance.new("Folder")
Child.Name = "Kid"
Child.Parent = Folder
Value.Value = 7
task.wait()
task.wait()
Line("event-fired", table.concat(Added, ","), table.concat(Values, ","))
Line("changed-names", table.concat(Names, ","), table.concat(ControlNames, ","))
Line("debris-done", P.try(function() return Doomed.Parent end))
