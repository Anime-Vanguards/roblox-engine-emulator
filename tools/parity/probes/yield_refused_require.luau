-- parity: identity=4
-- require where the thread cannot yield (a table.sort comparator): of a module another thread is still loading, of a
-- module whose chunk waits, of one that does not; what a later require of each says, and what the requiring thread's
-- next wait returns. (Identity 4: only it may write a ModuleScript's Source.)
P.sync()

local Folder = Instance.new("Folder")
Folder.Name = "ParityYieldRequire"
Folder.Parent = workspace
local function Make(Name, Source)
	local Module = Instance.new("ModuleScript")
	Module.Name = Name
	Module.Source = Source
	Module.Parent = Folder
	return Module
end

local function Refused(Body)
	local Line
	table.sort({ 2, 1 }, function(A, B)
		if not Line then
			Line = P.try(Body)
		end
		return A < B
	end)
	return Line
end

local function Follow(Label)
	local Started = os.clock()
	local Value = task.wait(0.3)
	print(`{Label}-next-wait {if os.clock() - Started >= 0.27 then "full" else "early"} {if type(Value) == "number" and Value >= 0.29 then "elapsed-full" else tostring(Value)}`)
end

-- a module another thread is loading (its chunk waits 0.1 s)
local Loading = Make("Loading", "task.wait(0.1) return 'loaded'")
local Loaded = {}
task.spawn(function()
	table.insert(Loaded, P.try(require, Loading))
end)
print("require-loading " .. Refused(function()
	return require(Loading)
end))
Follow("require-loading")
print("require-loading-first " .. table.concat(Loaded, ","))
print("require-loading-again " .. P.try(require, Loading))

-- a module whose chunk waits, first required where the thread cannot yield
local Waits = Make("Waits", "task.wait(0.1) return 'waited'")
print("require-waits " .. Refused(function()
	return require(Waits)
end))
Follow("require-waits")
print("require-waits-again " .. P.try(require, Waits))

-- a module whose chunk does not wait
local Plain = Make("Plain", "return 'plain'")
print("require-plain " .. Refused(function()
	return require(Plain)
end))

Folder:Destroy()
