-- parity: identity=4 flag=FFlagEnableLoadModule
-- debug.loadmodule's chunks and require's: a loaded chunk is named by its module at each load, takes no earlier chunk's name and
-- gives none to a later require of the same source text (chunk_sharing.luau has require's own rule).
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local Folder = Instance.new("Folder")
Folder.Name = "ParityShareLoad"
Folder.Parent = ReplicatedStorage
local function Make(Name, Source)
	local Module = Instance.new("ModuleScript")
	Module.Name = Name
	Module.Source = Source
	Module.Parent = Folder
	return Module
end
local Source = "return debug.info(1, 's') -- share-load-a"
local Loaded = debug.loadmodule(Make("LoadedFirst", Source))
print("loaded first " .. Loaded() .. " | require " .. require(Make("RequiredAfter", Source)) .. " | load again " .. debug.loadmodule(Make("LoadedAgain", Source))())
local Other = "return debug.info(1, 's') -- share-load-b"
print("required first " .. require(Make("RequiredFirst", Other)) .. " | load " .. debug.loadmodule(Make("LoadedAfter", Other))() .. " | require again " .. require(Make("RequiredAgain", Other)))
local Held = Make("HeldLoad", "return debug.info(1, 's') -- share-load-c")
local First = debug.loadmodule(Held)
Held.Name = "HeldRenamed"
print("load renamed " .. First() .. " | " .. debug.loadmodule(Held)())
Folder:Destroy()
