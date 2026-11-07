-- Changed and GetPropertyChangedSignal: which writes fire, how often, with what, and how a frame's writes batch.
P.sync()
local Log = {}
local function Note(Text)
	table.insert(Log, Text)
end
local function Flush(Label)
	print(Label .. " " .. table.concat(Log, ","))
	table.clear(Log)
end
local function Watch(Instance, Label, Properties)
	local Connections = {}
	table.insert(Connections, Instance.Changed:Connect(function(Name)
		Note(Label .. ".Changed(" .. tostring(Name) .. ")")
	end))
	for _, Property in Properties do
		table.insert(Connections, Instance:GetPropertyChangedSignal(Property):Connect(function(...)
			Note(Label .. "." .. Property .. "#" .. select("#", ...))
		end))
	end
	return function()
		for _, Connection in Connections do
			Connection:Disconnect()
		end
	end
end

-- 1. same value, a different value, NaN
local Part = Instance.new("Part")
local Stop = Watch(Part, "part", { "Transparency", "Size", "Name", "Anchored" })
Part.Transparency = 0
task.wait()
Flush("same-value")
Part.Transparency = 0.25
task.wait()
Flush("new-value")
Part.Transparency = 0 / 0
task.wait()
Flush("nan-first")
Part.Transparency = 0 / 0
task.wait()
Flush("nan-again")
print("nan-reads " .. P.num(Part.Transparency))
Part.Size = Part.Size
task.wait()
Flush("same-size")
Part.Size = Vector3.new(4, 1.2, 2)
task.wait()
Flush("size-rounding")
Part.Size = Vector3.new(0, 0, 0)
task.wait()
print("size-min " .. P.f32s(Part.Size.X, Part.Size.Y, Part.Size.Z))
Flush("size-clamped")
Part.Name = "Part"
task.wait()
Flush("same-name")
Part.Name = "Renamed"
Part.Anchored = true
task.wait()
Flush("two-properties")

-- 2. several writes to one property in one batch
Part.Transparency = 0.1
Part.Transparency = 0.2
Part.Transparency = 0.3
Part.Transparency = 0.3
local Seen = {}
local Reader = Part:GetPropertyChangedSignal("Transparency"):Connect(function()
	table.insert(Seen, P.num(Part.Transparency))
end)
Part.Transparency = 0.4
Part.Transparency = 0.5
task.wait()
Reader:Disconnect()
Flush("batched-writes")
print("batched-values-seen " .. table.concat(Seen, ","))
-- a write and a write back within the batch
Part.Transparency = 0.9
Part.Transparency = 0.5
task.wait()
Flush("write-and-back")
Stop()

-- 3. derived properties: one write, which signals?
local Derived = Instance.new("Part")
local StopDerived = Watch(Derived, "d", { "Position", "CFrame", "Orientation", "Rotation", "Color", "BrickColor", "Size" })
Derived.Position = Vector3.new(1, 2, 3)
task.wait()
Flush("write-position")
Derived.CFrame = CFrame.new(1, 2, 3)
task.wait()
Flush("write-same-cframe")
Derived.CFrame = CFrame.Angles(0, math.rad(90), 0)
task.wait()
Flush("write-cframe-rotation")
Derived.Color = Color3.new(1, 0, 0)
task.wait()
Flush("write-color")
Derived.BrickColor = BrickColor.new("Bright blue")
task.wait()
Flush("write-brickcolor")
StopDerived()

-- 4. Value objects: Changed passes the value
local Number = Instance.new("NumberValue")
Number.Changed:Connect(function(...)
	Note("value.Changed " .. select("#", ...) .. " " .. P.num((...)))
end)
Number:GetPropertyChangedSignal("Value"):Connect(function()
	Note("value.signal")
end)
Number.Value = 3
Number.Name = "Renamed"
task.wait()
Flush("numbervalue")
Number.Value = 0 / 0
task.wait()
Number.Value = 0 / 0
task.wait()
Flush("numbervalue-nan")
local Object = Instance.new("ObjectValue")
Object.Changed:Connect(function(Value)
	Note("object.Changed " .. tostring(Value))
end)
Object.Value = Part
Object.Value = Part
task.wait()
Flush("objectvalue")
local Str = Instance.new("StringValue")
Str.Changed:Connect(function(Value)
	Note("string.Changed " .. tostring(Value))
end)
Str.Value = ""
Str.Value = "x"
task.wait()
Flush("stringvalue")
local IntValue = Instance.new("IntValue")
IntValue.Changed:Connect(function(Value)
	Note("int " .. P.num(Value))
end)
IntValue.Value = 2.7
task.wait()
IntValue.Value = -2.7
task.wait()
IntValue.Value = 2 ^ 53 + 1
task.wait()
Flush("intvalue-rounding")
print("intvalue-reads " .. P.num(IntValue.Value))
local BoolValue = Instance.new("BoolValue")
print("bool-assign-number " .. P.try(function()
	BoolValue.Value = 1
end))
print("int-assign-string " .. P.try(function()
	IntValue.Value = "5"
end) .. " -> " .. P.num(IntValue.Value))
print("int-assign-badstring " .. P.try(function()
	IntValue.Value = "five"
end))

-- 5. attributes: AttributeChanged, and does Changed see them?
local Holder = Instance.new("Folder")
Holder.Changed:Connect(function(Name)
	Note("holder.Changed " .. tostring(Name))
end)
Holder.AttributeChanged:Connect(function(Name)
	Note("AttributeChanged " .. tostring(Name))
end)
Holder:GetAttributeChangedSignal("A"):Connect(function()
	Note("A-signal")
end)
Holder:SetAttribute("A", 1)
task.wait()
Flush("attribute-new")
Holder:SetAttribute("A", 1)
task.wait()
Flush("attribute-same")
Holder:SetAttribute("A", nil)
task.wait()
Flush("attribute-removed")
Holder:SetAttribute("B", nil)
task.wait()
Flush("attribute-remove-missing")
Holder:SetAttribute("A", 0 / 0)
task.wait()
Holder:SetAttribute("A", 0 / 0)
task.wait()
Flush("attribute-nan")

-- 6. Parent and tags through Changed
local Child = Instance.new("Folder")
Child.Changed:Connect(function(Name)
	Note("child.Changed " .. tostring(Name))
end)
Child.Parent = Holder
task.wait()
Child:AddTag("T")
task.wait()
Child.Archivable = false
task.wait()
Flush("parent-tag-archivable")
