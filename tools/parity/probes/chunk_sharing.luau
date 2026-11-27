-- parity: identity=4
-- Which name a chunk carries when several scripts have the same source: a client keeps the first chunk it compiled from each
-- text, held weakly, and names a later compile of that text by it, until a collection finds that first chunk unreferenced.
-- Writing Source needs identity 4 in a real engine.
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local LogService = game:GetService("LogService")
local Folder = Instance.new("Folder")
Folder.Name = "ParityShare"
Folder.Parent = ReplicatedStorage
local function Make(Name, Source, Parent)
	local Module = Instance.new("ModuleScript")
	Module.Name = Name
	Module.Source = Source
	Module.Parent = Parent or Folder
	return Module
end
-- each source reports its chunk name on its module's attribute Chunk, and keeps its chunk in shared.ParityKeep[script]
local function Held(Tag)
	return "script:SetAttribute('Chunk', debug.info(1, 's')) shared.ParityKeep[script] = debug.info(1, 'f') return 0 -- " .. Tag
end
local function Req(Module)
	local Ok, Problem = pcall(require, Module)
	if not Ok then
		return "error " .. tostring(Problem)
	end
	return tostring(Module:GetAttribute("Chunk"))
end
-- runs until a whole collection cycle went by
local function Cycle()
	local Watch = setmetatable({}, { __mode = "k" })
	Watch[{}] = true
	local Rounds = 0
	while next(Watch) ~= nil and Rounds < 4000 do
		local Junk = {}
		for Index = 1, 5000 do
			Junk[Index] = { Index }
		end
		Rounds += 1
		if Rounds % 50 == 0 then
			task.wait()
		end
	end
	return next(Watch) == nil
end
shared.ParityKeep = {}

-- named when compiled, the twin by the first's name at its compile
local Renamed = Make("BeforeRename", "return { f = function() return debug.info(1, 's') end } -- share-rename")
local RenamedValue = require(Renamed)
Renamed.Name = "AfterRename"
print("renamed " .. RenamedValue.f() .. " | twin " .. require(Make("Twin", "return { f = function() return debug.info(1, 's') end } -- share-rename")).f())
-- the first's chunk held: a collection keeps it
print("held " .. Req(Make("Held1", Held("share-held"))) .. " | cycle " .. tostring(Cycle()) .. " twin " .. Req(Make("Held2", Held("share-held"))))
-- the first's chunk dropped, the twin's held: a collection ends it, and the third names a chunk of its own
local Pair1 = Make("Pair1", Held("share-pair"))
local Pair2 = Make("Pair2", Held("share-pair"))
print("pair " .. Req(Pair1) .. " | " .. Req(Pair2))
shared.ParityKeep[Pair1] = nil
print("pair cycle " .. tostring(Cycle()) .. " third " .. Req(Make("Pair3", Held("share-pair"))) .. " | fourth " .. Req(Make("Pair4", Held("share-pair"))))
-- dropped at once
local None1 = Make("None1", Held("share-none"))
print("none " .. Req(None1))
shared.ParityKeep[None1] = nil
print("none cycle " .. tostring(Cycle()) .. " twin " .. Req(Make("None2", Held("share-none"))))
-- a running chunk is alive: a module requiring its twin while it runs
local Running = Make("Running", "local Name = debug.info(1, 's') local Other = script:FindFirstChild('Inner') return { name = Name, other = Other and require(Other) } -- share-run")
Make("Inner", "local Name = debug.info(1, 's') local Other = script:FindFirstChild('Inner') return { name = Name, other = Other and require(Other) } -- share-run", Running)
local Value = require(Running)
print("running " .. Value.name .. " | " .. Value.other.name)
-- another text (a space more) is another chunk
print("other text " .. Req(Make("Spaced", Held("share-held") .. " ")))
shared.ParityKeep = {}

-- errors: a runtime error carries the first's name; a source that does not compile names no chunk
local Log = {}
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	local First = (string.split(Message, "\n"))[1]
	if string.find(First, "ParityShare") then
		table.insert(Log, Kind.Name .. ":" .. First)
	end
end)
print("syntax " .. Req(Make("Bad1", "local = 1 -- share-syntax")) .. " | " .. Req(Make("Bad2", "local = 1 -- share-syntax")))
print("runtime " .. Req(Make("Err1", "error('share-err') -- share-err")) .. " | " .. Req(Make("Err2", "error('share-err') -- share-err")))
task.wait()
task.wait()
print("log " .. table.concat(Log, " | "))
Listener:Disconnect()
Folder:Destroy()
