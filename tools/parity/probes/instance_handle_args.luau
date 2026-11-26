-- Instance.new's parent, IsAncestorOf / IsDescendantOf and a Parent write given a value that is no Instance: an
-- InstanceHandle (of a part, of an ancestor, of nothing) among values of other kinds.
local Holder = Instance.new("Model")
Holder.Name = "Holder"
local Part = Instance.new("Part")
Part.Name = "Inner"
Part.Parent = Holder
local Nothing = newproxy()
local Values = {
	{ "handle-part", InstanceHandle.new(Part) },
	{ "handle-holder", InstanceHandle.new(Holder) },
	{ "handle-nil", InstanceHandle.new(nil) },
	{ "instance", Holder },
	{ "nil", nil },
	{ "none", Nothing },
	{ "table", {} },
	{ "number", 5 },
	{ "string", "Holder" },
	{ "true", true },
	{ "vector", Vector3.new(1, 2, 3) },
	{ "enumitem", Enum.Material.Wood },
	{ "function", print },
	{ "newproxy", newproxy() },
	{ "signal", workspace.Changed },
}
local function Call(Case, Function)
	if Case[2] == Nothing then
		return P.try(Function)
	end
	return P.try(Function, Case[2])
end
for _, Case in Values do
	print("new " .. Case[1] .. " " .. Call(Case, function(...)
		local Made = Instance.new("Folder", ...)
		return Made.Parent and Made.Parent.Name or "no parent"
	end))
	print("ancestor " .. Case[1] .. " " .. Call(Case, function(...)
		return Holder:IsAncestorOf(...)
	end))
	print("descendant " .. Case[1] .. " " .. Call(Case, function(...)
		return Part:IsDescendantOf(...)
	end))
	print("parent " .. Case[1] .. " " .. Call(Case, function(...)
		local Loose = Instance.new("Folder")
		Loose.Parent = ...
		return Loose.Parent and Loose.Parent.Name or "no parent"
	end))
end
