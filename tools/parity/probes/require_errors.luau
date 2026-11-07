-- parity: identity=4
-- What a client logs (LogService.MessageOut) and raises when a required module fails: an error while loading (pcall'd
-- or not), a module that returns nothing, a recursive require; and whether modules with the same source share one chunk
-- name (the first one compiled) when the first is renamed, destroyed or never required.
local LogService = game:GetService("LogService")
local Folder = Instance.new("Folder")
Folder.Name = "ParityRequireLog"
Folder.Parent = game:GetService("ReplicatedStorage")
local function Make(Name, Source, Parent)
	local Module = Instance.new("ModuleScript")
	Module.Name = Name
	Module.Source = Source
	Module.Parent = Parent or Folder
	return Module
end
P.sync()
local Log = {}
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local Listener = LogService.MessageOut:Connect(function(Message, Kind)
	local First = (string.split(Message, "\n"))[1]
	if string.find(First, "ParityRequireLog") or string.find(First, "parity%-req") or string.find(First, "Requested module") or string.find(First, "Module code") then
		First = string.gsub(First, Escaped .. ":(%d+)", function(Line)
			return "@" .. (tonumber(Line) - P.base)
		end)
		table.insert(Log, Kind.Name .. ":" .. First)
	elseif Kind == Enum.MessageType.MessageInfo and (First == "Stack Begin" or First == "Stack End" or string.find(First, "^Script '")) then
		table.insert(Log, "info:" .. (if string.find(First, "^Script '") then (string.gsub(string.gsub(First, Escaped, "<probe>"), "Line %d+", "Line n")) else First))
	end
end)
local function Flush(Label)
	task.wait()
	task.wait()
	print(Label .. " " .. table.concat(Log, " | "))
	table.clear(Log)
end

local Broken = Make("Broken", "local x = 1\nerror('parity-req failed')")
print("pcall-broken " .. P.try(require, Broken))
Flush("pcall-broken-log")
print("pcall-broken-again " .. P.try(require, Broken))
Flush("pcall-broken-again-log")
local Thrower = Make("Thrower", "error({ 'parity-req table' })")
print("pcall-table " .. (string.gsub(P.try(require, Thrower), "0x%x+", "ADDR")))
Flush("pcall-table-log")
local Indexer = Make("Indexer", "local Missing = nil\nreturn Missing['parity-req index']")
print("pcall-index " .. P.try(require, Indexer))
Flush("pcall-index-log")
local Nothing = Make("Nothing", "local x = 'parity-req nothing'")
print("pcall-nothing " .. P.try(require, Nothing))
Flush("pcall-nothing-log")
local Cycle = Make("Cycle", "return require(script)")
print("pcall-cycle " .. P.try(require, Cycle))
Flush("pcall-cycle-log")
-- an unprotected require in a spawned thread
task.spawn(function()
	require(Make("Spawned", "error('parity-req spawned')"))
end)
Flush("spawned-log")
-- a module that requires a broken one
local Outer = Make("Outer", "return require(script.Parent.Broken)")
print("pcall-outer " .. P.try(require, Outer))
Flush("pcall-outer-log")

-- the same source, several modules
local Shared = "return { where = debug.info(1, 's'), line = function() error('parity-req shared') end }"
local First = Make("First", Shared)
local Second = Make("Second", Shared)
print("shared-before " .. require(Second).where .. " | " .. P.try(require(Second).line))
local Third = Make("Third", Shared)
print("shared-after " .. require(First).where .. " | " .. require(Third).where)
First.Name = "Renamed"
local Fourth = Make("Fourth", Shared)
print("shared-renamed " .. require(Fourth).where)
Second:Destroy()
local Fifth = Make("Fifth", Shared)
print("shared-destroyed " .. require(Fifth).where)
local Spaced = Make("Spaced", Shared .. " ")
print("shared-other-text " .. require(Spaced).where)
local Clone = Third:Clone()
Clone.Name = "Cloned"
Clone.Parent = Folder
print("shared-clone " .. require(Clone).where)
Flush("shared-log")
Listener:Disconnect()
Folder:Destroy()
