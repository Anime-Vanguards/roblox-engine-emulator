-- parity: loopback
-- The thread a RemoteFunction's callback runs on, each side: what it sees above it, whether a run that returned leaves
-- its thread for the next run (and after a yield, an error, a replaced callback, a second remote with the same function, a
-- nested call), and the memory category it starts with. A callback's error reaches the caller as its text (its position is the
-- other side's chunk: left out).
local Storage = game:GetService("ReplicatedStorage")
local Escaped = string.gsub(P.chunk(), "[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")
local function Norm(Text)
	Text = string.gsub(tostring(Text), Escaped .. ":(%d+)", function(Number)
		return "@" .. (tonumber(Number) - P.base)
	end)
	return (string.gsub(Text, "\n", " / "))
end
local Start = debug.getmemorycategory()
local function Cat()
	local Value = debug.getmemorycategory()
	return if Value == Start then "start" else Value
end
local function Bits(List)
	local Out = {}
	for I = 1, #List do
		for J = I + 1, #List do
			table.insert(Out, if rawequal(List[I], List[J]) then "1" else "0")
		end
	end
	return table.concat(Out)
end
-- the host side: Install(Remote, Function) sets the remote's callback; Other is the remote the host calls back for a nested call
local Seen, Tags, Notes = {}, {}, {}
local function Host(Remotes, Install, CallBack)
	local Main
	local Second = function(...)
		local Tag = select(select("#", ...), ...)
		table.insert(Seen, coroutine.running())
		table.insert(Tags, Tag)
		return "second"
	end
	Main = function(...)
		local Tag = select(select("#", ...), ...)
		table.insert(Seen, coroutine.running())
		table.insert(Tags, Tag)
		if Tag == "a" then
			local S2, L2 = debug.info(2, "sl")
			local S3 = debug.info(3, "s")
			table.insert(Notes, "first yieldable=" .. tostring(coroutine.isyieldable()) .. " info2=" .. Norm(tostring(S2) .. " " .. tostring(L2)) .. " info3=" .. tostring(S3) .. " trace=" .. Norm(debug.traceback()))
		end
		table.insert(Notes, Tag .. " cat=" .. Cat())
		debug.setmemorycategory("parity-inside-" .. Tag)
		if string.sub(Tag, 1, 4) == "wait" then
			task.wait()
		elseif string.sub(Tag, 1, 3) == "err" then
			error("parity-" .. Tag)
		elseif Tag == "nest" then
			CallBack("inner")
		elseif Tag == "replace" then
			Install(Remotes[1], Second)
		end
		return Tag
	end
	local Setter = coroutine.create(function()
		debug.setmemorycategory("parity-setter")
		Install(Remotes[1], Main)
		Install(Remotes[2], Main)
		coroutine.yield()
		debug.setmemorycategory("parity-setter2")
		coroutine.yield()
	end)
	coroutine.resume(Setter)
	return function()
		coroutine.resume(Setter)
	end, function()
		Install(Remotes[1], Main)
	end
end
local function Report(Label)
	print(Label .. " tags " .. table.concat(Tags, " "))
	print(Label .. " threads " .. Bits(Seen))
	local Status = {}
	for _, Thread in Seen do
		table.insert(Status, coroutine.status(Thread))
	end
	print(Label .. " status " .. table.concat(Status, " "))
	print(Label .. " notes " .. table.concat(Notes, " | "))
	print(Label .. " resume-first " .. tostring((coroutine.resume(Seen[1]))))
	table.clear(Seen)
	table.clear(Tags)
	table.clear(Notes)
end
-- the invoker side: the calls in order (Switch: after "b", the setter's category moves; Restore: Main back after "replace")
local function Drive(Invoke, Invoke2, Switch, Restore)
	local Out = {}
	local function Try(Tag, Via)
		local Ok, Result = pcall(Via or Invoke, Tag)
		table.insert(Out, Tag .. "=" .. (if Ok then tostring(Result) else "error " .. tostring(string.match(tostring(Result), "parity%-%w+"))))
	end
	Try("a")
	Try("b")
	Switch()
	Try("c")
	Try("wait1")
	Try("d")
	Try("err1")
	Try("e")
	Try("f", Invoke2)
	Try("g")
	Try("nest")
	Try("h")
	Try("replace")
	Try("i")
	Restore()
	Try("j")
	return table.concat(Out, " ")
end
--- server
local Player = P.player()
local F = Instance.new("RemoteFunction")
F.Name = "F"
F.Parent = Storage
local F2 = Instance.new("RemoteFunction")
F2.Name = "F2"
F2.Parent = Storage
local G = Instance.new("RemoteFunction")
G.Name = "G"
G.Parent = Storage
local G2 = Instance.new("RemoteFunction")
G2.Name = "G2"
G2.Parent = Storage
local Control = Instance.new("RemoteFunction")
Control.Name = "Control"
Control.Parent = Storage
local ClientControl = Instance.new("RemoteFunction")
ClientControl.Name = "ClientControl"
ClientControl.Parent = Storage
local Switch, Restore = Host({ F, F2 }, function(Remote, Function)
	Remote.OnServerInvoke = Function
end, function(Tag)
	return G:InvokeClient(Player, Tag)
end)
Control.OnServerInvoke = function(_, What)
	if What == "switch" then
		Switch()
	elseif What == "restore" then
		Restore()
	end
end
P.meet("ready")
P.meet("client drove")
Report("server-host")
debug.setmemorycategory("parity-invoker")
print("server drove " .. Drive(function(Tag)
	return G:InvokeClient(Player, Tag)
end, function(Tag)
	return G2:InvokeClient(Player, Tag)
end, function()
	ClientControl:InvokeClient(Player, "switch")
end, function()
	ClientControl:InvokeClient(Player, "restore")
end))
P.meet("server drove")
--- client
local F = Storage:WaitForChild("F")
local F2 = Storage:WaitForChild("F2")
local G = Storage:WaitForChild("G")
local G2 = Storage:WaitForChild("G2")
local Control = Storage:WaitForChild("Control")
local ClientControl = Storage:WaitForChild("ClientControl")
local Switch, Restore = Host({ G, G2 }, function(Remote, Function)
	Remote.OnClientInvoke = Function
end, function(Tag)
	return F:InvokeServer(Tag)
end)
ClientControl.OnClientInvoke = function(What)
	if What == "switch" then
		Switch()
	elseif What == "restore" then
		Restore()
	end
end
P.meet("ready")
debug.setmemorycategory("parity-invoker")
print("client drove " .. Drive(function(Tag)
	return F:InvokeServer(Tag)
end, function(Tag)
	return F2:InvokeServer(Tag)
end, function()
	Control:InvokeServer("switch")
end, function()
	Control:InvokeServer("restore")
end))
P.meet("client drove")
P.meet("server drove")
Report("client-host")
