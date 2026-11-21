-- parity: identity=4 context=edit
-- How settings() and UserSettings() and their services refuse a Parent write, as a plugin's code sees them (captured at
-- script identity 4, which has the Plugin capability settings() needs: parent_locks.luau has the rest). A root passes the
-- parent it has (nil) and raises its capability error for another, but "its own parent" made its own parent; every settings
-- service refuses by class.
local Target = Instance.new("Folder")
Target.Name = "Target"
local function Writes(Label, Object)
	local Old = Object.Parent
	local Same = P.try(function()
		Object.Parent = Old
	end)
	local New = if Old == nil then Target else nil
	local Other = P.try(function()
		Object.Parent = New
	end)
	if Object.Parent ~= Old then
		Other ..= " (moved: put back " .. P.try(function()
			Object.Parent = Old
		end) .. ")"
	end
	local Placed = P.try(function()
		Object.Parent = workspace
	end)
	if Object.Parent ~= Old then
		Placed ..= " (moved: put back " .. P.try(function()
			Object.Parent = Old
		end) .. ")"
	end
	local Self = P.try(function()
		Object.Parent = Object
	end)
	print(Label .. " | same " .. Same .. " | other " .. Other .. " | workspace " .. Placed .. " | self " .. Self)
end
local Root = settings()
Writes("root " .. Root.Name, Root)
for _, Child in Root:GetChildren() do
	Writes("service " .. Child.ClassName .. " (" .. Child.Name .. ")", Child)
end
local User = UserSettings()
Writes("user " .. User.Name, User)
for _, Child in User:GetChildren() do
	Writes("user service " .. Child.ClassName .. " (" .. Child.Name .. ")", Child)
end
