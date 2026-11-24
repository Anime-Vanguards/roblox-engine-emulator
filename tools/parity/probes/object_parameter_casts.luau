-- An Object parameter given a value that is not an Instance: Instance.new's parent and IsAncestorOf / IsDescendantOf, from a
-- function's line and called directly by pcall; nil and no argument at all; and the class name's check against the parent's.
local Folder = Instance.new("Folder")
local Values = {
	{ "nil", function() return nil end },
	{ "number", function() return 5 end },
	{ "string", function() return "x" end },
	{ "table", function() return {} end },
	{ "bool", function() return true end },
	{ "Vector3", function() return Vector3.one end },
	{ "CFrame", function() return CFrame.identity end },
	{ "EnumItem", function() return Enum.Material.Wood end },
	{ "Enum", function() return Enum.Material end },
	{ "function", function() return print end },
	{ "thread", function() return coroutine.create(print) end },
	{ "buffer", function() return buffer.create(1) end },
	{ "newproxy", function() return newproxy() end },
	{ "handle", function() return InstanceHandle.new(workspace) end },
	{ "empty-handle", function() return InstanceHandle.new(nil) end },
	{ "signal", function() return workspace.Changed end },
	{ "RaycastParams", function() return RaycastParams.new() end },
}
for _, Case in Values do
	local Value = Case[2]()
	print("new " .. Case[1] .. " " .. P.try(function()
		return Instance.new("Folder", Value).Parent
	end))
	print("new-direct " .. Case[1] .. " " .. P.try(function()
		local Made = select(2, pcall(Instance.new, "Folder", Value))
		return if typeof(Made) == "Instance" then "Instance" else Made
	end))
	print("ancestor " .. Case[1] .. " " .. P.try(function()
		return workspace:IsAncestorOf(Value)
	end))
	print("descendant " .. Case[1] .. " " .. P.try(function()
		return Folder:IsDescendantOf(Value)
	end))
	print("ancestor-dot " .. Case[1] .. " " .. P.try(workspace.IsAncestorOf, workspace, Value))
	print("descendant-dot " .. Case[1] .. " " .. P.try(Folder.IsDescendantOf, Folder, Value))
end
print("ancestor-none " .. P.try(function()
	return workspace:IsAncestorOf()
end))
print("descendant-none " .. P.try(function()
	return Folder:IsDescendantOf()
end))
print("descendant-lower " .. P.try(function()
	return Folder:isDescendantOf(5)
end))
print("descendant-extra " .. P.try(function()
	return Folder:IsDescendantOf(nil, 5)
end))
print("ancestor-extra " .. P.try(function()
	return workspace:IsAncestorOf(Folder, 5)
end))
print("new-bad-class " .. P.try(function()
	return Instance.new("NoSuchClass", 5)
end))
print("new-number-class " .. P.try(function()
	return Instance.new(5, 5)
end))
print("new-service-class " .. P.try(function()
	return Instance.new("Workspace", 5)
end))
print("new-extra " .. P.try(function()
	return Instance.new("Folder", nil, 5).Parent
end))
print("new-parent-false " .. P.try(function()
	return Instance.new("Folder", false)
end))
