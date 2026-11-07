-- Attributes that hold an Instance (accepted by 0.744): what GetAttribute and GetAttributes hand back, what that value
-- is and does, AttributeChanged, and instances outside the DataModel, destroyed, or the holder itself.
P.sync()
local Holder = Instance.new("Folder")
Holder.Parent = workspace
local Target = Instance.new("Part")
Target.Name = "ParityTarget"
Target.Parent = workspace
local Changes = {}
Holder.AttributeChanged:Connect(function(Name)
	table.insert(Changes, Name)
end)
print("set " .. P.try(Holder.SetAttribute, Holder, "Ref", Target))
local Read = Holder:GetAttribute("Ref")
print("read typeof=" .. typeof(Read) .. " type=" .. type(Read) .. " tostring=" .. tostring(Read) .. " equal=" .. tostring(Read == Target) .. " rawequal=" .. tostring(rawequal(Read, Target)))
for _, Key in { "Name", "Parent", "ClassName", "Instance", "Value", "Target", "IsA" } do
	print("member " .. Key .. " " .. P.try(function()
		local Value = Read[Key]
		return typeof(Value) .. "(" .. tostring(Value) .. ")"
	end))
end
print("method " .. P.try(function()
	return Read:IsA("Part")
end) .. " | " .. P.try(function()
	return Read:GetFullName()
end))
print("same-read " .. tostring(Holder:GetAttribute("Ref") == Read) .. " " .. tostring(rawequal(Holder:GetAttribute("Ref"), Read)))
local Listed = Holder:GetAttributes()
print("listed typeof=" .. typeof(Listed.Ref) .. " tostring=" .. tostring(Listed.Ref))
print("as-key " .. P.try(function()
	local Map = { [Read] = true }
	return tostring(Map[Holder:GetAttribute("Ref")]) .. " " .. tostring(Map[Target])
end))
task.wait()
print("changed " .. table.concat(Changes, ","))
table.clear(Changes)
pcall(Holder.SetAttribute, Holder, "Ref", Target)
task.wait()
print("same-again " .. table.concat(Changes, ","))
Target.Name = "Renamed"
print("after-rename " .. tostring(Holder:GetAttribute("Ref")))
Target.Parent = nil
print("after-unparent " .. tostring(Holder:GetAttribute("Ref")) .. " " .. typeof(Holder:GetAttribute("Ref")))
Target:Destroy()
task.wait()
print("after-destroy " .. tostring(Holder:GetAttribute("Ref")) .. " " .. typeof(Holder:GetAttribute("Ref")))
local Loose = Instance.new("Folder")
print("loose " .. P.try(Holder.SetAttribute, Holder, "Loose", Loose) .. " " .. tostring(Holder:GetAttribute("Loose")))
print("self " .. P.try(Holder.SetAttribute, Holder, "Self", Holder) .. " " .. tostring(Holder:GetAttribute("Self")))
print("service " .. P.try(Holder.SetAttribute, Holder, "Service", game:GetService("Lighting")) .. " " .. tostring(Holder:GetAttribute("Service")))
local Copy = Holder:Clone()
print("clone " .. tostring(Copy:GetAttribute("Self")) .. " " .. tostring(Copy:GetAttribute("Self") == Copy) .. " " .. tostring(Copy:GetAttribute("Service")))
print("handle-attribute " .. P.try(Holder.SetAttribute, Holder, "Again", Holder:GetAttribute("Service")) .. " " .. typeof(Holder:GetAttribute("Again")))
Holder:Destroy()
