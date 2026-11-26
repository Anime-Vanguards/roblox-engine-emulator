-- parity: loopback
-- A server script's Character write as a spawn (src/services/players.rs): the player's Backpack and PlayerGui around each write
-- of character_write.luau's kinds (a model in the Workspace, another, the same, one in ReplicatedStorage, nil, a parentless one).
-- StarterPack holds a Tool and a Folder, the player's StarterGear a Tool, StarterGui a ScreenGui that resets on spawn, one that
-- does not and a Folder. The server prints, for each write, what was there in the writing thread and the events in the order
-- they fire (the Player's and PlayerGui's children, the old Backpack's and gui copies' removal and Destroying, CharacterAdded
-- and Changed, with what the containers held then); the client prints what it holds once the write reached it.
local Players = game:GetService("Players")
local Storage = game:GetService("ReplicatedStorage")
local StarterPack = game:GetService("StarterPack")
local StarterGui = game:GetService("StarterGui")
local function Kids(X)
	if X == nil then
		return "none"
	end
	local Names = {}
	for _, Child in X:GetChildren() do
		table.insert(Names, Child.Name)
	end
	table.sort(Names)
	return "{" .. table.concat(Names, " ") .. "}"
end
-- (the children in GetChildren's order)
local function Listed(X)
	if X == nil then
		return "none"
	end
	local Names = {}
	for _, Child in X:GetChildren() do
		table.insert(Names, Child.Name)
	end
	return "[" .. table.concat(Names, " ") .. "]"
end
--- server
Players.CharacterAutoLoads = false
local function Make(Class, Name, Parent)
	local X = Instance.new(Class)
	X.Name = Name
	X.Parent = Parent
	return X
end
Make("Tool", "PackTool", StarterPack).RequiresHandle = false
Make("Folder", "PackFolder", StarterPack)
Make("ScreenGui", "Reset", StarterGui)
Make("ScreenGui", "Keep", StarterGui).ResetOnSpawn = false
Make("Folder", "GuiFolder", StarterGui)
local Me = P.player()
local Gear = Me:FindFirstChildOfClass("StarterGear")
Make("Tool", "GearTool", Gear).RequiresHandle = false
local Holder = Make("Folder", "Holder", Storage)
local function Model(Name, Parent)
	local M = Instance.new("Model")
	M.Name = Name
	local Part = Instance.new("Part")
	Part.Anchored = true
	Part.Position = Vector3.new(500, 4, #Name * 10)
	Part.Parent = M
	M.Parent = Parent
	return M
end
local Gui = Me:FindFirstChildOfClass("PlayerGui")
local function Pack()
	return Me:FindFirstChildOfClass("Backpack")
end
local Order = {}
local function Note(Text)
	table.insert(Order, Text)
end
local Watched = {}
local function Watch(X, Label)
	if Watched[X] then
		return
	end
	Watched[X] = true
	X.AncestryChanged:Connect(function(Child, Parent)
		if Child == X then
			Note(Label .. " AncestryChanged " .. (if Parent then Parent.ClassName else "nil"))
		end
	end)
	X.Destroying:Connect(function()
		Note(Label .. " Destroying")
	end)
end
local function WatchAll()
	if Pack() then
		Watch(Pack(), "old Backpack")
	end
	for _, Child in Gui:GetChildren() do
		Watch(Child, "old " .. Child.Name)
	end
end
Me.ChildAdded:Connect(function(X)
	Note("Player.ChildAdded " .. X.ClassName .. " " .. Kids(X))
end)
Me.ChildRemoved:Connect(function(X)
	Note("Player.ChildRemoved " .. X.ClassName)
end)
Gui.ChildAdded:Connect(function(X)
	Note("PlayerGui.ChildAdded " .. X.Name)
end)
Gui.ChildRemoved:Connect(function(X)
	Note("PlayerGui.ChildRemoved " .. X.Name)
end)
Me.CharacterRemoving:Connect(function(X)
	Note("CharacterRemoving " .. X.Name)
end)
Me.CharacterAdded:Connect(function(X)
	Note("CharacterAdded " .. X.Name .. " pack=" .. Kids(Pack()) .. " gui=" .. Kids(Gui))
end)
Me:GetPropertyChangedSignal("Character"):Connect(function()
	Note("Character " .. (if Me.Character then Me.Character.Name else "nil") .. " pack=" .. Kids(Pack()) .. " gui=" .. Kids(Gui))
end)
print("start: pack=" .. Kids(Pack()) .. " gui=" .. Kids(Gui) .. " gear=" .. Kids(Gear))
Make("Folder", "Extra", Pack())
local Cases = {
	{ "nil to M1", function() Me.Character = Model("M1", workspace) end },
	{ "M1 to M2", function() Me.Character = Model("M2", workspace) end },
	{ "M2 again", function() Me.Character = Me.Character end },
	{ "M2 to M3 in ReplicatedStorage", function() Me.Character = Model("M3", Holder) end },
	{ "M3 to nil", function() Me.Character = nil end },
	{ "nil to M4 parentless", function() Me.Character = Model("M4", nil) end },
	{ "M4 to M5", function() Me.Character = Model("M5", workspace) end },
}
for Index, Case in Cases do
	P.meet("case " .. Index)
	WatchAll()
	local Before = Pack()
	if Index == 2 then
		-- a gui copy a script changed, and one more thing in the Backpack
		local Copy = Gui:FindFirstChild("Reset")
		if Copy then
			Make("Folder", "Mark", Copy)
		end
		Make("Folder", "Extra", Pack())
	end
	Case[2]()
	print(Case[1] .. ", in the thread: same Backpack=" .. tostring(Pack() == Before) .. " pack=" .. Kids(Pack()) .. " gui=" .. Kids(Gui))
	task.wait()
	print(Case[1] .. ": " .. table.concat(Order, ", "))
	table.clear(Order)
	local Reset = Gui:FindFirstChild("Reset")
	print(Case[1] .. ", after: pack=" .. Kids(Pack()) .. " gui=" .. Kids(Gui) .. " Reset=" .. (if Reset then Kids(Reset) else "none") .. " gear=" .. Kids(Gear) .. " listed " .. Listed(Pack()) .. " " .. Listed(Gui) .. " " .. Listed(Me))
	P.meet("case " .. Index .. " done")
end
--- client
local Me = Players.LocalPlayer
local Labels = { "nil to M1", "M1 to M2", "M2 again", "M2 to M3 in ReplicatedStorage", "M3 to nil", "nil to M4 parentless", "M4 to M5" }
local function Gui()
	return Me:FindFirstChildOfClass("PlayerGui")
end
for Index, Label in Labels do
	P.meet("case " .. Index)
	P.meet("case " .. Index .. " done")
	task.wait()
	local Reset = Gui() and Gui():FindFirstChild("Reset")
	print(Label .. ": pack=" .. Kids(Me:FindFirstChildOfClass("Backpack")) .. " gui=" .. Kids(Gui()) .. " Reset=" .. (if Reset then Kids(Reset) else "none"))
end
