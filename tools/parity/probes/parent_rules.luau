-- A TextChannelWindow's children, and writes of every kind of value to Faces and Axes properties
P.sync()
local Window = Instance.new("TextChannelWindow")
for _, Class in { "Folder", "Frame", "TextLabel", "UICorner", "UIListLayout", "Configuration", "StringValue", "ModuleScript", "LocalScript", "TextChannel", "ScreenGui", "TextChannelWindow", "ImageLabel", "UIGradient", "Model" } do
	local Child = Instance.new(Class)
	print("window child", Class, P.try(function() Child.Parent = Window end), Child.Parent == Window)
	Child:Destroy()
end
print("window new parent", P.try(function() return typeof(Instance.new("Folder", Window)) end))
local Holder = Instance.new("Folder")
print("window into folder", P.try(function() Window.Parent = Holder end))
local Corner = Instance.new("UICorner")
Corner.Parent = Window
local Deep = Instance.new("Folder")
print("folder under a window's corner", P.try(function() Deep.Parent = Corner end))
print("window clone with a child", P.try(function() return #Window:Clone():GetChildren() end))
local Loose = Instance.new("Folder")
Loose.Parent = Window.Parent
print("window set parent of child", P.try(function() Loose.Parent = Window end), P.try(function() Window:FindFirstChild("x") end))
local Screen = Instance.new("ScreenGui")
print("window into screen", P.try(function() Window.Parent = Screen end))
Window:Destroy()

local Handles = Instance.new("Handles")
local Arc = Instance.new("ArcHandles")
local Fired = {}
Handles.Changed:Connect(function(Name)
	table.insert(Fired, "H." .. Name)
end)
Arc.Changed:Connect(function(Name)
	table.insert(Fired, "A." .. Name)
end)
local Values = {
	{ "faces top", Faces.new(Enum.NormalId.Top) },
	{ "faces none", Faces.new() },
	{ "axes x", Axes.new(Enum.Axis.X) },
	{ "axes faces", Axes.new(Enum.NormalId.Front) },
	{ "number", 5 },
	{ "string", "Top" },
	{ "table", {} },
	{ "bool", true },
	{ "normal", Enum.NormalId.Top },
	{ "axis", Enum.Axis.Y },
	{ "vector", Vector3.new(1, 0, 0) },
	{ "instance", Instance.new("Folder") },
	{ "nil", nil },
}
for _, Pair in Values do
	local Name, Value = Pair[1], Pair[2]
	table.clear(Fired)
	local H = P.try(function() Handles.Faces = Value end)
	local A = P.try(function() Arc.Axes = Value end)
	task.wait()
	print("write", Name, "| Faces", H, tostring(Handles.Faces), "| Axes", A, tostring(Arc.Axes), "|", table.concat(Fired, " "))
end
local Part = Instance.new("Part")
print("resizeable", tostring(Part.ResizeableFaces), P.try(function() (Part :: any).ResizeableFaces = Faces.new() end), P.try(function() (Part :: any).ResizeableFaces = 5 end))
print("attribute faces", P.try(function() Part:SetAttribute("F", Faces.new(Enum.NormalId.Top)) end), P.try(function() Part:SetAttribute("A", Axes.new(Enum.Axis.X)) end))
local Value = Instance.new("ObjectValue")
print("faces in a value", P.try(function() (Value :: any).Value = Faces.new() end))
