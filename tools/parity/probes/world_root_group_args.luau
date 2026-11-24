-- WorldRoot's collision-group methods with arguments of other types and names at the edges, on a WorldModel and on the
-- Workspace: casts, which error wins, ids and masks, the 32-group limit, wrong selves. Removes every group it makes.
local RunService = game:GetService("RunService")
print("side " .. (if RunService:IsServer() then "server" else "client"))
print("start " .. P.try(function()
	return #workspace:GetRegisteredCollisionGroups()
end))

local function Run(Label, Root)
	local function Names()
		local Ok, List = pcall(function()
			return Root:GetRegisteredCollisionGroups()
		end)
		if not Ok or type(List) ~= "table" then
			return {}
		end
		local Out = {}
		for _, Entry in List do
			table.insert(Out, if type(Entry) == "table" then Entry.name else Entry)
		end
		return Out
	end
	local function Call(Name, ...)
		local Arguments = table.pack(...)
		return P.try(function()
			return Root[Name](Root, table.unpack(Arguments, 1, Arguments.n))
		end)
	end
	local function Show(Name, Method, ...)
		print(Label .. " " .. Name .. " " .. Call(Method, ...))
	end
	local function Clean()
		pcall(function()
			for _, Name in Names() do
				if Name ~= "Default" then
					Root:UnregisterCollisionGroup(Name)
				end
			end
		end)
	end

	-- 1. the name a group gets from a name argument of another type
	for _, Input in { { "integer", 5 }, { "fraction", 2.5 }, { "tenth", 0.1 }, { "nan", 0 / 0 }, { "inf", math.huge }, { "true", true }, { "empty-table", {} }, { "dictionary", { a = 1 } }, { "Vector3", Vector3.new(1, 2, 3) }, { "Instance", workspace }, { "EnumItem", Enum.Material.Plastic }, { "function", function() end }, { "buffer", buffer.create(2) }, { "Enum", Enum.Material }, { "userdata", newproxy() }, { "Color3", Color3.new(1, 0, 0) } } do
		local Before = #Names()
		local Outcome = Call("RegisterCollisionGroup", Input[2])
		local After = Names()
		local Made = if #After > Before then string.format("%q", After[#After]) else "none"
		print(Label .. " name-from-" .. Input[1] .. " " .. Outcome .. " | " .. P.text(Made) .. " | registered " .. Call("IsCollisionGroupRegistered", Input[2]))
		Clean()
	end

	-- 2. the collidable argument
	Call("RegisterCollisionGroup", "ArgA")
	for _, Input in { { "0", 0 }, { "half", 0.5 }, { "nan", 0 / 0 }, { "true-string", "true" }, { "empty-string", "" }, { "table", {} }, { "Instance", workspace }, { "Vector3", Vector3.zero }, { "EnumItem", Enum.Material.Plastic }, { "False-string", "False" }, { "one-string", "1" }, { "function", function() end }, { "buffer", buffer.create(1) }, { "false", false } } do
		Call("CollisionGroupSetCollidable", "Default", "ArgA", true)
		local Outcome = Call("CollisionGroupSetCollidable", "Default", "ArgA", Input[2])
		print(Label .. " collidable-from-" .. Input[1] .. " " .. Outcome .. " | " .. Call("CollisionGroupsAreCollidable", "Default", "ArgA"))
	end
	Call("CollisionGroupSetCollidable", "Default", "ArgA", true)

	-- 3. names at the edges
	Show("register-default-lower", "RegisterCollisionGroup", "default")
	Show("register-Default", "RegisterCollisionGroup", "Default")
	Show("register-empty", "RegisterCollisionGroup", "")
	Show("register-nil", "RegisterCollisionGroup", nil)
	Show("register-100", "RegisterCollisionGroup", string.rep("L", 100))
	Show("register-101", "RegisterCollisionGroup", string.rep("L", 101))
	Show("register-102-bytes", "RegisterCollisionGroup", string.rep("\u{00FC}", 51))
	Show("register-nul", "RegisterCollisionGroup", "A\0B")
	Show("registered-nul-prefix", "IsCollisionGroupRegistered", "A")
	Show("registered-nil", "IsCollisionGroupRegistered", nil)
	Show("registered-none", "IsCollisionGroupRegistered")
	print(Label .. " edge-names " .. P.text(table.concat(Names(), ",")))
	Clean()
	Show("collidable-empty", "CollisionGroupsAreCollidable", "", "Default")
	Show("collidable-nil-nil", "CollisionGroupsAreCollidable", nil, nil)
	Show("collidable-x-nil", "CollisionGroupsAreCollidable", "Default", nil)
	Show("set-Default-Default", "CollisionGroupSetCollidable", "Default", "Default", false)
	print(Label .. " default-mask " .. P.try(function()
		return Root:GetRegisteredCollisionGroups()[1].mask
	end))
	Show("collidable-Default-Default", "CollisionGroupsAreCollidable", "Default", "Default")
	Show("set-Default-Default-back", "CollisionGroupSetCollidable", "Default", "Default", true)

	-- 4. which error wins
	Show("set-empty-known", "CollisionGroupSetCollidable", "", "Default", false)
	Show("set-unknown-bad-bool", "CollisionGroupSetCollidable", "Nope", "Default", "no")
	Show("set-nil-nil", "CollisionGroupSetCollidable", nil, nil, nil)
	Show("set-known-nil-bool", "CollisionGroupSetCollidable", "Default", nil, "no")
	Show("set-unknown-known", "CollisionGroupSetCollidable", "Nope", "Default", false)
	Show("rename-unknown-empty", "RenameCollisionGroup", "Nope", "")
	Show("rename-empty-x", "RenameCollisionGroup", "", "ArgX")
	Show("rename-long-x", "RenameCollisionGroup", string.rep("L", 101), "ArgX")
	Show("rename-Default-x", "RenameCollisionGroup", "Default", "ArgX")
	Show("rename-x-Default", "RenameCollisionGroup", "ArgA", "Default")
	Show("rename-nil-nil", "RenameCollisionGroup", nil, nil)
	Show("rename-empty-nil", "RenameCollisionGroup", "", nil)
	Show("rename-unknown", "RenameCollisionGroup", "Nope", "ArgX")
	Show("rename-onto-registered", "RenameCollisionGroup", "ArgA", "Default")
	Call("RegisterCollisionGroup", "ArgB")
	Show("rename-onto-ArgB", "RenameCollisionGroup", "ArgA", "ArgB")
	Show("rename-same", "RenameCollisionGroup", "ArgA", "ArgA")
	Show("unregister-Default", "UnregisterCollisionGroup", "Default")
	Show("unregister-empty", "UnregisterCollisionGroup", "")
	Show("unregister-long", "UnregisterCollisionGroup", string.rep("L", 101))
	Show("unregister-nil", "UnregisterCollisionGroup", nil)
	Show("unregister-unknown", "UnregisterCollisionGroup", "Nope")
	print(Label .. " after-errors " .. P.text(table.concat(Names(), ",")))
	Clean()

	-- 5. ids and masks
	Call("RegisterCollisionGroup", "ArgA")
	Call("RegisterCollisionGroup", "ArgB")
	Call("RegisterCollisionGroup", "ArgC")
	Call("UnregisterCollisionGroup", "ArgB")
	Call("CollisionGroupSetCollidable", "ArgA", "ArgC", false)
	Call("RegisterCollisionGroup", "ArgD")
	Call("RenameCollisionGroup", "ArgA", "ArgE")
	print(Label .. " order " .. P.try(function()
		local Out = {}
		for _, Entry in Root:GetRegisteredCollisionGroups() do
			table.insert(Out, Entry.name .. "=" .. P.num(Entry.mask))
		end
		return table.concat(Out, ",")
	end))
	Clean()

	-- 6. at the limit
	for Index = 1, 31 do
		Call("RegisterCollisionGroup", "ArgLimit" .. Index)
	end
	print(Label .. " full " .. #Names())
	Show("full-existing", "RegisterCollisionGroup", "ArgLimit1")
	Show("full-Default", "RegisterCollisionGroup", "Default")
	Show("full-empty", "RegisterCollisionGroup", "")
	Show("full-new", "RegisterCollisionGroup", "ArgOver")
	Show("full-rename", "RenameCollisionGroup", "ArgLimit31", "ArgRenamed")
	print(Label .. " full-names " .. P.text(table.concat(Names(), ",", 30)))
	Clean()
	print(Label .. " end " .. P.text(table.concat(Names(), ",")))
end

local Model = Instance.new("WorldModel")
Run("model", Model)
Run("workspace", workspace)

-- 7. wrong selves and missing selves
local Methods = { "CollisionGroupSetCollidable", "CollisionGroupsAreCollidable", "GetMaxCollisionGroups", "GetRegisteredCollisionGroups", "IsCollisionGroupRegistered", "RegisterCollisionGroup", "RenameCollisionGroup", "UnregisterCollisionGroup" }
for _, Name in Methods do
	local Function = workspace[Name]
	print("self-nil " .. Name .. " " .. P.try(Function))
	print("self-physics " .. Name .. " " .. P.try(Function, game:GetService("PhysicsService"), "Default", "Default", true))
	print("self-part " .. Name .. " " .. P.try(Function, Instance.new("Part"), "Default", "Default", true))
	print("self-model " .. Name .. " " .. P.try(Function, Model, "Default", "Default", true))
	print("self-string " .. Name .. " " .. P.try(Function, "x", "Default", "Default", true))
	print("self-dot " .. Name .. " " .. P.try(function()
		return workspace[Name]("Default", "Default", true)
	end))
end
for _, Name in Methods do
	print("physics-on-model " .. Name .. " " .. P.try(game:GetService("PhysicsService")[Name], Model, "Default", "Default", true))
end
Model:Destroy()
