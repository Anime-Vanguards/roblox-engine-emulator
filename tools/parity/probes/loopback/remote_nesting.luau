-- parity: loopback
-- How the depth a remote takes depends on the table's shape and on the call's other values: each side bisects the deepest
-- dictionary nesting its FireServer / FireClient accepts, the deepest array after another argument, FireAllClients', a
-- RemoteFunction's arguments; and the client asks for returned tables at and past the limits a callback's results have.
local Storage = game:GetService("ReplicatedStorage")
local function Nest(Depth, Key)
	local Root = {}
	local At = Root
	for _ = 2, Depth do
		local Inner = {}
		At[Key or 1] = Inner
		At = Inner
	end
	return Root
end
local function Height(Value, Key)
	local Count = 0
	while type(Value) == "table" do
		Count += 1
		Value = Value[Key or 1]
	end
	return Count
end
local function Deepest(Fire, Key)
	local Low, High = 1, 2000
	while Low < High do
		local Middle = (Low + High + 1) // 2
		if pcall(Fire, Nest(Middle, Key)) then
			Low = Middle
		else
			High = Middle - 1
		end
	end
	return Low
end
--- server
local Player = P.player()
local Event = Instance.new("RemoteEvent")
Event.Name = "Nesting"
Event.Parent = Storage
local Call = Instance.new("RemoteFunction")
Call.Name = "NestingCall"
Call.Parent = Storage
Call.OnServerInvoke = function(_, Depth, Key)
	if Depth == "args" then
		return 0
	end
	return Nest(Depth, Key)
end
P.meet("ready")
local function Report(Name, Fire, Key)
	local Deep = Deepest(Fire, Key)
	print("server " .. Name .. " deepest " .. Deep .. ", one more: " .. P.try(Fire, Nest(Deep + 1, Key)))
end
Report("fire-client-dict", function(Value) Event:FireClient(Player, Value) end, "a")
Report("fire-client-2nd", function(Value) Event:FireClient(Player, 1, Value) end)
Report("fire-all", function(Value) Event:FireAllClients(Value) end)
Report("fire-all-dict", function(Value) Event:FireAllClients(Value) end, "a")
P.meet("server sent")
P.meet("client sent")
--- client
local Event = Storage:WaitForChild("Nesting")
local Call = Storage:WaitForChild("NestingCall")
P.meet("ready")
P.meet("server sent")
local function Report(Name, Fire, Key)
	local Deep = Deepest(Fire, Key)
	print("client " .. Name .. " deepest " .. Deep .. ", one more: " .. P.try(Fire, Nest(Deep + 1, Key)))
end
Report("fire-server-dict", function(Value) Event:FireServer(Value) end, "a")
Report("fire-server-2nd", function(Value) Event:FireServer(1, Value) end)
Report("invoke-server-arg", function(Value) Call:InvokeServer("args", Value) end)
for _, Case in { { 299 }, { 300 }, { 301 }, { 150, "a" }, { 151, "a" } } do
	print("client invoke returning " .. Case[1] .. (if Case[2] then " dict" else "") .. " " .. P.try(function()
		return Height(Call:InvokeServer(Case[1], Case[2]), Case[2])
	end))
end
P.meet("client sent")
