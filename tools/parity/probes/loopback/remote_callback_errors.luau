-- parity: loopback
-- What an error in a RemoteFunction's callback logs on the side the callback runs on (LogService.MessageOut and
-- ScriptContext.Error), OnServerInvoke and OnClientInvoke alike, and a BindableFunction's OnInvoke on each side, beside what the
-- caller gets: a string, a string at level 0, tables, nil, a number, a Vector3, a runtime error, an error after a yield, a
-- result with a key whose __tostring fails. Then whether a BindableFunction's callback runs on its caller's thread, and what a
-- spawned thread's uncaught errors log, for comparison.
local Storage = game:GetService("ReplicatedStorage")
local LogService = game:GetService("LogService")
local ScriptContext = game:GetService("ScriptContext")
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
-- a logged text with the probe's own positions made comparable (an error line's stack names `Script '<chunk>', Line n`,
-- ScriptContext.Error's trace `<chunk>, line n`)
local function Normal(Text)
	Text = string.gsub(Text, "Script '" .. Escaped .. "', Line (%d+)", function(Line)
		return "Script '<probe>', Line @" .. (tonumber(Line) - P.base)
	end)
	Text = string.gsub(Text, "=?" .. Escaped .. ", line (%d+)", function(Line)
		return "<probe>, line @" .. (tonumber(Line) - P.base)
	end)
	return P.text(P.err((string.gsub(Text, "0x%x+", "ADDR"))))
end
-- what each side logs, filed under the case running (Current)
local Logs = {}
local Current = nil
local function Note(Text)
	if Current then
		Logs[Current] = Logs[Current] or {}
		table.insert(Logs[Current], Text)
	end
end
LogService.MessageOut:Connect(function(Message, Kind)
	if Kind ~= Enum.MessageType.MessageOutput then
		Note(Kind.Name .. ":" .. Normal(Message))
	end
end)
ScriptContext.Error:Connect(function(Message, Trace)
	Note("ScriptContext.Error:" .. Normal(tostring(Message)) .. " trace=" .. Normal(tostring(Trace)))
end)
local Callbacks = {
	{ "string", function() error("callback failed") end },
	{ "level 0", function() error("plain failure", 0) end },
	{ "table", function() error({ code = 1 }) end },
	{ "table shapes", function() error({ 1, "two", { three = 3 }, flag = true }) end },
	{ "table with tostring", function() error(setmetatable({}, { __tostring = function() return "custom text" end })) end },
	{ "nil", function() error(nil) end },
	{ "number", function() error(7) end },
	{ "Vector3", function() error(Vector3.new(1, 2, 3)) end },
	{ "runtime", function() local Missing = nil return Missing.Field end },
	{ "after a wait", function() task.wait() error("failed after a wait") end },
	{ "result with a failing key", function() return { [setmetatable({}, { __tostring = function() return nil end })] = 1 } end },
	{ "fine", function() return "fine" end },
}
local CallbackThread = nil
local function Callback(Index, ...)
	Current = Index
	CallbackThread = coroutine.running()
	return Callbacks[Index][2](...)
end
-- (an error raised in the other side's callback carries that side's script and line)
local function Caller(Message)
	return (string.gsub(tostring(Message), "^[^\n]-:%d+: ", "<position>: "))
end
local function Run(Call, Label)
	for Index, Entry in Callbacks do
		Current = Index
		local Ok, Result = Call(Index)
		for _ = 1, 6 do
			task.wait()
		end
		print(Label .. " " .. Entry[1] .. ": " .. (if Ok then "ok" else "error " .. typeof(Result) .. " " .. Caller(Result)))
	end
	Current = nil
end
local function Report(Label)
	for Index, Entry in Callbacks do
		print(Label .. " " .. Entry[1] .. ": logged " .. (if Logs[Index] then #Logs[Index] .. " " .. table.concat(Logs[Index], " || ") else "0"))
	end
	table.clear(Logs)
	Current = nil
end
local Bindable = Instance.new("BindableFunction")
Bindable.OnInvoke = Callback
local function Controls(Label)
	Bindable:Invoke(#Callbacks)
	print(Label .. " a BindableFunction's callback on its caller's thread: " .. tostring(CallbackThread == coroutine.running()))
	for _, Case in { { "spawned string", function() error("spawned failure") end }, { "spawned table", function() error({}) end } } do
		Current = Case[1]
		task.spawn(Case[2])
		for _ = 1, 6 do
			task.wait()
		end
		print(Label .. " " .. Case[1] .. ": logged " .. (if Logs[Case[1]] then #Logs[Case[1]] .. " " .. table.concat(Logs[Case[1]], " || ") else "0"))
	end
	table.clear(Logs)
	Current = nil
end
--- server
local Player = P.player()
local Function = Instance.new("RemoteFunction")
Function.Name = "Errors"
Function.OnServerInvoke = function(_, Index)
	return Callback(Index)
end
Function.Parent = Storage
P.meet("ready")
P.meet("client called")
Report("server log, OnServerInvoke")
Run(function(Index) return pcall(Function.InvokeClient, Function, Player, Index) end, "InvokeClient")
Report("server log, InvokeClient caller")
P.meet("server called")
Run(function(Index) return pcall(Bindable.Invoke, Bindable, Index) end, "server Bindable")
Report("server log, Bindable")
Controls("server")
--- client
local Function = Storage:WaitForChild("Errors")
Function.OnClientInvoke = Callback
P.meet("ready")
Run(function(Index) return pcall(Function.InvokeServer, Function, Index) end, "InvokeServer")
Report("client log, InvokeServer caller")
P.meet("client called")
P.meet("server called")
Report("client log, OnClientInvoke")
Run(function(Index) return pcall(Bindable.Invoke, Bindable, Index) end, "client Bindable")
Report("client log, Bindable")
Controls("client")
