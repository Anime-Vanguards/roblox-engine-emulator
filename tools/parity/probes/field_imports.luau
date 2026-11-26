-- A field read through game or workspace (Game, Workspace) is read when the code runs: the compiler imports the global, not
-- the chain through it, so `workspace.Gravity` reads what a write through a local left, not the value the chunk loaded with.
-- (A chain through Enum is resolved at load: its values never change.)
local W = workspace
local G = game
local Gravity = W.Gravity
W.Gravity = 50
print("Gravity", workspace.Gravity, W.Gravity, game.Workspace.Gravity, Workspace.Gravity, Game.Workspace.Gravity)
W.Gravity = Gravity
print("Gravity again", workspace.Gravity == Gravity)

local Camera = W.CurrentCamera
local Was = Camera.CFrame
Camera.CFrame = CFrame.new(1, 2, 3)
print("CurrentCamera.CFrame", workspace.CurrentCamera.CFrame.Position, game.Workspace.CurrentCamera.CFrame.Position)
Camera.CFrame = Was

local Name = Camera.Name
Camera.Name = "FieldImportsCamera"
print("renamed child", P.try(function()
	return workspace.Camera
end))
Camera.Name = Name

local Folder = Instance.new("Folder")
Folder.Name = "FieldImportsFolder"
Folder.Parent = W
print("new child", workspace.FieldImportsFolder == Folder)
Folder:Destroy()

local Was = G.Name
G.Name = "FieldImportsGame"
print("game.Name", game.Name)
G.Name = Was
print("Enum chain", Enum.KeyCode.A == Enum.KeyCode.A, Enum.KeyCode.A.Name)
