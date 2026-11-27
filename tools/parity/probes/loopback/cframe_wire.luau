-- parity: loopback
-- The rotation a CFrame arrives with, each way and by every road: a RemoteEvent's argument (both ways), one inside a
-- table, a RemoteFunction's result, and replicated properties (CFrameValue, a part's CFrame and PivotOffset, an
-- attachment's CFrame, a weld's C0, a model's pivot, an attribute) at the join and as a change after it.
local Storage = game:GetService("ReplicatedStorage")
local function Frames()
	local List = {
		CFrame.Angles(0.5, 0, 0),
		CFrame.Angles(0.1, 0.2, 0.3),
		CFrame.Angles(1, 2, 3),
		CFrame.fromAxisAngle(Vector3.new(1, 1, 1).Unit, 2),
		CFrame.Angles(math.pi, 0, 0),
		CFrame.Angles(0, math.pi / 2, 0),
		CFrame.new(),
		CFrame.fromMatrix(Vector3.zero, Vector3.yAxis, Vector3.zAxis),
		CFrame.Angles(1e-6, 0, 0),
		CFrame.Angles(2e-4, 0, 0),
		CFrame.Angles(3e-4, 0, 0),
		CFrame.Angles(0, math.pi / 2, 0) * CFrame.Angles(0, 0, 2e-4),
		CFrame.Angles(0, math.pi / 2, 0) * CFrame.Angles(0, 0, 3e-4),
		CFrame.Angles(math.pi / 2, 0, 0) * CFrame.Angles(0, 1e-5, 0),
		CFrame.new(0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 1, 0.1, 0, 0, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 1, 0, 0.5, 0, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 1, 2, 0, 0, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 1, 0, 0, 0, 0.5, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
		CFrame.new(0, 0, 0, 0 / 0, 0, 0, 0, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0, 0 / 0),
		CFrame.new(0, 0, 0, math.huge, 0, 0, 0, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, -1, 0, 0, 0, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, -1),
		CFrame.new(0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 0, -1, 0, 1, 0, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, -0, -1, 0, 1, -0, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0),
		CFrame.new(0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, -1, 0, 0, 1, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 1, 0, 0, -1, 1, 0, 0, 0, 1),
		CFrame.new(0, 0, 0, 0, 0, 0, -1, 0, 1, 1, 1, 0),
		CFrame.new(0, 0, 0, 0, 1, 0, 0, 0, 1, -1, 0, 0),
		CFrame.fromAxisAngle(Vector3.new(1, 1, 1).Unit, 2 * math.pi / 3),
		CFrame.Angles(0, math.pi / 4, 0),
		CFrame.fromAxisAngle(Vector3.new(1, 1, 0).Unit, math.pi),
		CFrame.new(123456.789, -0, 1e30) * CFrame.Angles(0.5, 0, 0),
		CFrame.new(0 / 0, math.huge, -math.huge),
	}
	local R = CFrame.Angles(0.5, 0.3, 0.1)
	table.insert(List, CFrame.fromMatrix(Vector3.zero, R.XVector * 0.5, R.YVector, R.ZVector))
	table.insert(List, CFrame.fromMatrix(Vector3.zero, R.XVector, R.YVector * 2, R.ZVector))
	table.insert(List, CFrame.fromMatrix(Vector3.zero, R.XVector, R.YVector, -R.ZVector))
	for Index = 1, 12 do
		table.insert(List, CFrame.new(Index, -Index, Index * 0.5) * CFrame.Angles(Index * 0.37, Index * 0.71, Index * 1.13))
	end
	return List
end
local function Text(Value)
	if typeof(Value) ~= "CFrame" then
		return typeof(Value)
	end
	return P.f32s(Value:GetComponents())
end
local Roads = { "Value", "PartCFrame", "PartPivot", "Attachment", "WeldC0", "ModelPivot", "Attribute" }
local function Make(Tag)
	local Folder = Instance.new("Folder")
	Folder.Name = Tag
	for Index, Value in Frames() do
		local Holder = Instance.new("Folder")
		Holder.Name = tostring(Index)
		local CV = Instance.new("CFrameValue")
		CV.Name = "Value"
		CV.Value = Value
		CV.Parent = Holder
		local Part = Instance.new("Part")
		Part.Name = "Part"
		Part.Anchored = true
		Part.CFrame = Value
		Part.PivotOffset = Value
		local A = Instance.new("Attachment")
		A.CFrame = Value
		A.Parent = Part
		local W = Instance.new("Weld")
		W.C0 = Value
		W.Parent = Part
		Part.Parent = Holder
		local M = Instance.new("Model")
		M.Name = "Model"
		M.WorldPivot = Value
		M.Parent = Holder
		Holder:SetAttribute("C", Value)
		Holder.Parent = Folder
	end
	return Folder
end
local function Read(Holder, Road)
	if Road == "Value" then
		return Holder.Value.Value
	elseif Road == "PartCFrame" then
		return Holder.Part.CFrame
	elseif Road == "PartPivot" then
		return Holder.Part.PivotOffset
	elseif Road == "Attachment" then
		return Holder.Part.Attachment.CFrame
	elseif Road == "WeldC0" then
		return Holder.Part.Weld.C0
	elseif Road == "ModelPivot" then
		return Holder.Model.WorldPivot
	elseif Road == "Attribute" then
		return Holder:GetAttribute("C")
	end
end
--- server
local Before = Make("Before")
Before.Parent = Storage
local Player = P.player()
local Event = Instance.new("RemoteEvent")
Event.Name = "Frames"
Event.Parent = Storage
local Call = Instance.new("RemoteFunction")
Call.Name = "Call"
Call.Parent = Storage
Call.OnServerInvoke = function(_, Index)
	return Frames()[Index]
end
local Got = {}
Event.OnServerEvent:Connect(function(_, Index, Value, Boxed)
	Got[Index] = "from-client " .. Index .. " arg " .. Text(Value) .. " | boxed " .. Text(Boxed[1])
end)
-- the server's own reads of what it stored (a write may change a rotation before it is sent)
for Index in Frames() do
	local Holder = Before[tostring(Index)]
	local Parts = {}
	for _, Road in Roads do
		table.insert(Parts, Road .. " " .. Text(Read(Holder, Road)))
	end
	print("stored " .. Index .. " " .. table.concat(Parts, " | "))
end
P.meet("ready")
for Index, Value in Frames() do
	Event:FireClient(Player, Index, Value, { Value })
end
local After = Make("After")
After.Parent = Storage
P.meet("sent")
P.meet("client sent")
for _ = 1, 10 do
	task.wait()
end
for Index in Frames() do
	print(Got[Index] or ("from-client " .. Index .. " none"))
end
--- client
local Event = Storage:WaitForChild("Frames")
local Call = Storage:WaitForChild("Call")
local Got = {}
Event.OnClientEvent:Connect(function(Index, Value, Boxed)
	Got[Index] = "from-server " .. Index .. " arg " .. Text(Value) .. " | boxed " .. Text(Boxed[1])
end)
P.meet("ready")
P.meet("sent")
local After = Storage:WaitForChild("After")
for _ = 1, 10 do
	task.wait()
end
for Index in Frames() do
	print(Got[Index] or ("from-server " .. Index .. " none"))
end
for _, Tag in { "Before", "After" } do
	local Folder = Storage:WaitForChild(Tag)
	for Index in Frames() do
		local Holder = Folder:WaitForChild(tostring(Index))
		local Parts = {}
		for _, Road in Roads do
			table.insert(Parts, Road .. " " .. Text(Read(Holder, Road)))
		end
		print(Tag .. " " .. Index .. " " .. table.concat(Parts, " | "))
	end
end
for Index in Frames() do
	print("invoke " .. Index .. " " .. Text(Call:InvokeServer(Index)))
end
for Index, Value in Frames() do
	Event:FireServer(Index, Value, { Value })
end
P.meet("client sent")
