-- parity: identity=4
-- require as scripts see it: what a module's chunk finds above it (debug.info and a traceback inside the chunk), the frames of
-- an error a module raises and of require's own errors, and a thread waiting in require for a module another thread loads. A
-- real client lets a script write Source and require the module it made only at identity 4.
local Folder = Instance.new("Folder")
Folder.Name = "ParityRequire"
Folder.Parent = game:GetService("ReplicatedStorage")
local function Make(Name, Source)
	local Module = Instance.new("ModuleScript")
	Module.Name = Name
	Module.Source = Source
	Module.Parent = Folder
	return Module
end
local Prefix = P.chunk() .. ":"
local function Frames(Text)
	local Out = {}
	for Line in string.gmatch(Text, "[^\n]+") do
		if string.sub(Line, 1, #Prefix) == Prefix then
			local Number, Rest = string.match(string.sub(Line, #Prefix + 1), "^(%d+)(.*)$")
			if Number then
				Line = "@" .. (tonumber(Number) - P.base) .. Rest
			end
		end
		table.insert(Out, Line)
		if string.find(Line, "function Req", 1, true) then
			break
		end
	end
	return table.concat(Out, " / ")
end
local Info = Make("Info", "local S, L, N = debug.info(2, 'sln') return tostring(S) .. ' ' .. tostring(L) .. ' ' .. tostring(N)")
local Trace = Make("Trace", "return debug.traceback()")
local Fails = Make("Fails", "error('parity module failed')")
local Waits = Make("Waits", "task.wait(0.2) return 'late'")
local function Req(Module)
	return require(Module)
end
print("module-level2 " .. tostring(Req(Info)))
print("module-trace " .. Frames(Req(Trace)))
local Seen
local Ok = xpcall(Req, function(Message)
	Seen = P.err(Message) .. " | " .. Frames(debug.traceback())
	return Message
end, Fails)
print("module-error " .. tostring(Ok) .. " " .. tostring(Seen))
Seen = nil
xpcall(Req, function(Message)
	Seen = P.err(Message) .. " | " .. Frames(debug.traceback())
	return Message
end, Folder)
print("require-error " .. tostring(Seen))
local First = task.spawn(Req, Waits)
local Second = task.spawn(Req, Waits)
print("waiting first " .. coroutine.status(First) .. " | " .. Frames(debug.traceback(First)))
print("waiting second " .. coroutine.status(Second) .. " | " .. Frames(debug.traceback(Second)))
task.wait(0.4)
Folder:Destroy()
