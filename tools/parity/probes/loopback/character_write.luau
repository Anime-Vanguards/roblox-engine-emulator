-- parity: loopback
-- A server script writing its player's Character (src/services/players.rs set_character): to a model in the Workspace,
-- another, the same, one in ReplicatedStorage, nil, a parentless one, one a CharacterRemoving handler puts back, after the old
-- one was destroyed. What the server sees (the events in the order they fire, where the old character is in the writing thread,
-- which old characters are destroyed a second on) and what the player's client sees of each write (the old character's
-- removal, then CharacterRemoving, CharacterAdded and Changed). The server's write also gives the player a new Backpack
-- (StarterPack's tools in it), which ree does not.
local Players = game:GetService("Players")
local Storage = game:GetService("ReplicatedStorage")
--- server
Players.CharacterAutoLoads = false
local Me = P.player()
local Holder = Instance.new("Folder")
Holder.Name = "Holder"
Holder.Parent = Storage
local function Model(Name, Parent)
	local M = Instance.new("Model")
	M.Name = Name
	local Part = Instance.new("Part")
	Part.Anchored = true
	Part.Position = Vector3.new(400, 4, #Name * 10)
	Part.Parent = M
	M.Parent = Parent
	return M
end
local function Name(X)
	return if X == nil then "nil" else X.Name
end
local Order = {}
local Removed, Gone = {}, {}
local function Note(Text)
	table.insert(Order, Text)
end
local function Watch(M)
	M.AncestryChanged:Connect(function(X, Parent)
		if X == M then
			Note(M.Name .. " AncestryChanged " .. Name(Parent))
		end
	end)
	M.Destroying:Connect(function()
		Gone[M.Name] = if Removed[M.Name] then string.format("%.1f", time() - Removed[M.Name]) else "by the script"
	end)
	return M
end
Me.CharacterRemoving:Connect(function(X)
	Removed[X.Name] = time()
	Note("CharacterRemoving " .. Name(X))
end)
Me.CharacterAdded:Connect(function(X)
	Note("CharacterAdded " .. Name(X))
end)
Me.Changed:Connect(function(Property)
	if Property == "Character" then
		Note("Changed Character")
	end
end)
Me:GetPropertyChangedSignal("Character"):Connect(function()
	Note("Character " .. Name(Me.Character))
end)
workspace.ChildRemoved:Connect(function(X)
	if X:IsA("Model") then
		Note("Workspace.ChildRemoved " .. X.Name)
	end
end)
local Cases = {
	{ "nil to M1", function() Me.Character = Watch(Model("M1", workspace)) end },
	{ "M1 to M2", function() Me.Character = Watch(Model("M2", workspace)) end },
	{ "M2 again", function() Me.Character = Me.Character end },
	{ "M2 to M3 in ReplicatedStorage", function() Me.Character = Watch(Model("M3", Holder)) end },
	{ "M3 to nil", function() Me.Character = nil end },
	{ "nil to M4 parentless", function() Me.Character = Watch(Model("M4", nil)) end },
	{ "M4 to M5", function() Me.Character = Watch(Model("M5", workspace)) end },
	{
		"M5 to M6, M5 put back",
		function()
			local Connection
			Connection = Me.CharacterRemoving:Connect(function(X)
				Connection:Disconnect()
				X.Parent = workspace
			end)
			Me.Character = Watch(Model("M6", workspace))
		end,
	},
	{ "M6 destroyed, then M7", function() Me.Character:Destroy() Me.Character = Watch(Model("M7", workspace)) end },
}
for Index, Case in Cases do
	P.meet("case " .. Index)
	local Old = Me.Character
	local Pack = Me:FindFirstChildOfClass("Backpack")
	Case[2]()
	print(Case[1] .. ", in the thread: old.Parent=" .. Name(Old and Old.Parent) .. " Character=" .. Name(Me.Character)
		.. (if Index == 2 then " new Backpack=" .. tostring(Me:FindFirstChildOfClass("Backpack") ~= Pack) else ""))
	task.wait()
	print(Case[1] .. ": " .. table.concat(Order, ", "))
	table.clear(Order)
	P.meet("case " .. Index .. " done")
end
task.wait(1.5)
local Times = {}
for _, Key in { "M1", "M2", "M3", "M4", "M5", "M6" } do
	table.insert(Times, Key .. " " .. (Gone[Key] or "no"))
end
print("destroyed: " .. table.concat(Times, ", "))
--- client
local Me = Players.LocalPlayer
local function Name(X)
	return if X == nil then "nil" else X.Name
end
local Order = {}
local function Note(Text)
	table.insert(Order, Text)
end
Me.CharacterRemoving:Connect(function(X)
	Note("CharacterRemoving " .. Name(X))
end)
Me.CharacterAdded:Connect(function(X)
	Note("CharacterAdded " .. Name(X))
end)
Me:GetPropertyChangedSignal("Character"):Connect(function()
	Note("Character " .. Name(Me.Character))
end)
local Watched = {}
local function Watch(M)
	if Watched[M] then
		return
	end
	Watched[M] = true
	M.AncestryChanged:Connect(function(X, Parent)
		if X == M then
			Note(M.Name .. " AncestryChanged " .. Name(Parent))
		end
	end)
end
workspace.ChildAdded:Connect(function(X)
	if X:IsA("Model") then
		Watch(X)
	end
end)
local Labels = { "nil to M1", "M1 to M2", "M2 again", "M2 to M3 in ReplicatedStorage", "M3 to nil", "nil to M4 parentless", "M4 to M5", "M5 to M6, M5 put back", "M6 destroyed, then M7" }
for Index, Label in Labels do
	P.meet("case " .. Index)
	if Me.Character then
		Watch(Me.Character)
	end
	P.meet("case " .. Index .. " done")
	print(Label .. ": " .. table.concat(Order, ", "))
	table.clear(Order)
end
