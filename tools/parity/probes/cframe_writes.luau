-- What a CFrame property keeps of a rotation that is not orthonormal (a part in the Workspace, anchored and not, a loose
-- part, an attachment, a joint's C0, a CFrameValue, a model's pivot), at several sizes of error; and the order of the
-- change events a write of CFrame, Position, Orientation or Rotation fires (Changed and each property's signal) on a part
-- and on an attachment.
local function CF(Value)
	return P.f32s(Value:GetComponents())
end
workspace.Gravity = 0
local Origin = Vector3.new(300, 40, 300)
local Errors = { 0, 1e-7, 1e-6, 5e-6, 1e-5, 2e-5, 1e-4, 1e-3, 0.1 }
local function Skewed(Error, Kind)
	local C = math.cos(0.4)
	local S = math.sin(0.4)
	if Kind == "scaled" then
		local K = 1 + Error
		return CFrame.new(Origin.X, Origin.Y, Origin.Z, C * K, -S * K, 0, S * K, C * K, 0, 0, 0, K)
	elseif Kind == "skew" then
		return CFrame.new(Origin.X, Origin.Y, Origin.Z, C, -S + Error, 0, S, C, Error, 0, 0, 1)
	end
	return CFrame.new(Origin.X, Origin.Y, Origin.Z, C, -S, Error, S, C, 0, 0, 0, 1)
end

local Anchored = Instance.new("Part")
Anchored.Anchored = true
Anchored.Parent = workspace
local Free = Instance.new("Part")
Free.Anchored = false
Free.Parent = workspace
local Loose = Instance.new("Part")
local Attachment = Instance.new("Attachment")
Attachment.Parent = Anchored
local Weld = Instance.new("Weld")
local Value = Instance.new("CFrameValue")
local Model = Instance.new("Model")
local Member = Instance.new("Part")
Member.Anchored = true
Member.Parent = Model
Model.PrimaryPart = Member
Model.Parent = workspace
local Targets = {
	{ "anchored", function(C)
		Anchored.CFrame = C
		return Anchored.CFrame
	end },
	{ "free", function(C)
		Free.CFrame = C
		return Free.CFrame
	end },
	{ "loose", function(C)
		Loose.CFrame = C
		return Loose.CFrame
	end },
	{ "attachment", function(C)
		Attachment.CFrame = C
		return Attachment.CFrame
	end },
	{ "weld-c0", function(C)
		Weld.C0 = C
		return Weld.C0
	end },
	{ "value", function(C)
		Value.Value = C
		return Value.Value
	end },
	{ "pivot-to", function(C)
		Model:PivotTo(C)
		return Member.CFrame
	end },
	{ "world-pivot", function(C)
		Model.WorldPivot = C
		return Model.WorldPivot
	end },
	{ "camera", function(C)
		local Camera = workspace.CurrentCamera
		Camera.CFrame = C
		return Camera.CFrame
	end },
}
for _, Kind in { "column", "scaled", "skew" } do
	for _, Error in Errors do
		local Written = Skewed(Error, Kind)
		local Parts = {}
		local Seen = {}
		for _, Target in Targets do
			local Ok, Read = pcall(Target[2], Written)
			local Text = if Ok then (if Read == Written then "same" else CF(Read)) else "E(" .. P.err(Read) .. ")"
			-- each distinct reading once, by the first target that gave it
			if Seen[Text] then
				table.insert(Parts, Target[1] .. "=" .. Seen[Text])
			else
				Seen[Text] = Target[1]
				table.insert(Parts, Target[1] .. ":" .. Text)
			end
		end
		print("write " .. Kind .. " " .. P.num(Error) .. " " .. table.concat(Parts, " | "))
	end
end

-- change events
local Events = {}
local function Watch(Instance, Label, Names)
	Instance.Changed:Connect(function(Name)
		if table.find(Names, Name) then
			table.insert(Events, Label .. ".Changed(" .. Name .. ")")
		end
	end)
	for _, Name in Names do
		Instance:GetPropertyChangedSignal(Name):Connect(function()
			table.insert(Events, Label .. "." .. Name)
		end)
	end
end
local Watched = Instance.new("Part")
Watched.Anchored = true
Watched.Parent = workspace
local PartNames = { "CFrame", "Position", "Orientation", "Rotation", "PivotOffset" }
Watch(Watched, "part", PartNames)
local Hung = Instance.new("Attachment")
Hung.Parent = Watched
local AttachmentNames = { "CFrame", "Position", "Orientation", "Rotation", "Axis", "SecondaryAxis", "WorldCFrame", "WorldPosition", "WorldOrientation", "WorldAxis", "WorldSecondaryAxis", "WorldRotation" }
Watch(Hung, "attachment", AttachmentNames)
task.wait()
local function Step(Label, Action)
	table.clear(Events)
	Action()
	task.wait()
	print("events " .. Label .. " " .. table.concat(Events, ","))
end
Step("part-cframe", function()
	Watched.CFrame = CFrame.new(310, 40, 300) * CFrame.Angles(0.1, 0.2, 0.3)
end)
Step("part-position", function()
	Watched.Position = Vector3.new(311, 41, 301)
end)
Step("part-orientation", function()
	Watched.Orientation = Vector3.new(10, 20, 30)
end)
Step("part-rotation", function()
	Watched.Rotation = Vector3.new(5, 6, 7)
end)
Step("part-cframe-translation", function()
	Watched.CFrame = Watched.CFrame + Vector3.new(1, 0, 0)
end)
Step("part-cframe-same", function()
	Watched.CFrame = Watched.CFrame
end)
Step("part-pivot", function()
	Watched:PivotTo(CFrame.new(320, 40, 300))
end)
Step("part-two-writes", function()
	Watched.Position = Vector3.new(321, 40, 300)
	Watched.CFrame = CFrame.new(322, 40, 300)
end)
Step("attachment-cframe", function()
	Hung.CFrame = CFrame.new(1, 2, 3) * CFrame.Angles(0.3, 0.2, 0.1)
end)
Step("attachment-position", function()
	Hung.Position = Vector3.new(2, 3, 4)
end)
Step("attachment-orientation", function()
	Hung.Orientation = Vector3.new(30, 20, 10)
end)
Step("attachment-axis", function()
	Hung.Axis = Vector3.new(0, 1, 0)
end)
Step("attachment-world-position", function()
	Hung.WorldPosition = Vector3.new(330, 40, 300)
end)
Step("attachment-parent-moves", function()
	Watched.CFrame = CFrame.new(340, 40, 300)
end)
Anchored:Destroy()
Free:Destroy()
Model:Destroy()
Watched:Destroy()
