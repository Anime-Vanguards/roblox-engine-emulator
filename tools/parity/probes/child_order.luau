-- The order of an instance's children after one is taken out: GetChildren, GetDescendants, FindFirstChild among duplicate
-- names, ClearAllChildren's and Destroy's order of removal, a re-add, a Clone; lists of 19 to 23 (where the rule changes),
-- removals from the start, the middle and the end, several in a row, and in each kind of container. Every list is built and
-- read with no yield in between, so nothing else joins it.
P.sync()
local function Names(Parent, Prefix)
	local Out = {}
	for _, Child in Parent:GetChildren() do
		if string.sub(Child.Name, 1, #Prefix) == Prefix then
			table.insert(Out, string.sub(Child.Name, #Prefix + 1))
		end
	end
	return table.concat(Out, " ")
end
local function Fill(Parent, Count, Prefix, Class)
	local Made = {}
	for Index = 1, Count do
		local Child = Instance.new(Class or "Folder")
		Child.Name = Prefix .. Index
		Child.Parent = Parent
		Made[Index] = Child
	end
	return Made
end
local function Box(Count)
	local Folder = Instance.new("Folder")
	return Folder, Fill(Folder, Count, "c")
end

-- 1. one taken out (Parent = nil) of lists of 19 to 23 and 40: the first, the second, the middle, the second last and the last
for _, Count in { 19, 20, 21, 22, 23, 40 } do
	for _, At in { 1, 2, math.ceil(Count / 2), Count - 1, Count } do
		local Folder, Made = Box(Count)
		Made[At].Parent = nil
		print(`take {Count} {At}: {Names(Folder, "c")}`)
		Folder:Destroy()
	end
end

-- 2. the ways of taking one out: a move to another parent, Destroy, Remove, a move to a descendant of the list's parent
for _, Way in { "move", "destroy", "remove", "nested" } do
	local Folder, Made = Box(24)
	local Other = Instance.new("Folder")
	if Way == "move" then
		Made[3].Parent = Other
	elseif Way == "destroy" then
		Made[3]:Destroy()
	elseif Way == "remove" then
		Made[3]:Remove()
	else
		Made[3].Parent = Made[10]
	end
	print(`way {Way}: {Names(Folder, "c")}`)
	Folder:Destroy()
	Other:Destroy()
end

-- 3. several in a row: the first each time from 23 down to 17 (the rule changes as the list shrinks), set places from 24
do
	local Folder = Box(23)
	local Steps = {}
	for _ = 1, 6 do
		Folder:GetChildren()[1].Parent = nil
		table.insert(Steps, Names(Folder, "c"))
	end
	print("first-each-time " .. table.concat(Steps, " | "))
	Folder:Destroy()
end
do
	local Folder, Made = Box(24)
	local Steps = {}
	for _, At in { 3, 5, 7, 9 } do
		Made[At].Parent = nil
		table.insert(Steps, Names(Folder, "c"))
	end
	print("set-places " .. table.concat(Steps, " | "))
	Folder:Destroy()
end
do
	-- the middle one each time from 22 to 18
	local Folder = Box(22)
	local Steps = {}
	for _ = 1, 4 do
		local Children = Folder:GetChildren()
		Children[math.ceil(#Children / 2)].Parent = nil
		table.insert(Steps, Names(Folder, "c"))
	end
	print("middle-each-time " .. table.concat(Steps, " | "))
	Folder:Destroy()
end
do
	-- the last each time from 23 to 19
	local Folder = Box(23)
	local Steps = {}
	for _ = 1, 4 do
		local Children = Folder:GetChildren()
		Children[#Children].Parent = nil
		table.insert(Steps, Names(Folder, "c"))
	end
	print("last-each-time " .. table.concat(Steps, " | "))
	Folder:Destroy()
end
do
	-- one by one, the first each time, all of 25: the order they go in
	local Folder = Box(25)
	local Gone = {}
	while #Folder:GetChildren() > 0 do
		local First = Folder:GetChildren()[1]
		table.insert(Gone, string.sub(First.Name, 2))
		First.Parent = nil
	end
	print("all-first-each-time " .. table.concat(Gone, " "))
	Folder:Destroy()
end

-- 4. a re-add after a removal: the taken one put back, a new one, one moved in from elsewhere, and a Parent write to the parent it has
do
	local Folder, Made = Box(22)
	Made[5].Parent = nil
	Made[5].Parent = Folder
	print("readd " .. Names(Folder, "c"))
	local New = Instance.new("Folder")
	New.Name = "c99"
	New.Parent = Folder
	print("readd-new " .. Names(Folder, "c"))
	Made[2].Parent = Folder
	print("readd-same-parent " .. Names(Folder, "c"))
	Made[22].Parent = nil
	print("readd-then-last " .. Names(Folder, "c"))
	Folder:Destroy()
end
do
	-- back below 21 and up again: a list of 21 loses one (20 left), gains one, loses one in the middle
	local Folder, Made = Box(21)
	Made[1].Parent = nil
	local New = Instance.new("Folder")
	New.Name = "c50"
	New.Parent = Folder
	print("shrink-grow " .. Names(Folder, "c"))
	Made[6].Parent = nil
	print("shrink-grow-take " .. Names(Folder, "c"))
	Folder:Destroy()
end

-- 5. ClearAllChildren and Destroy: the order the children go in, straight and after a removal
local function Order(Folder, Made, Run)
	local Gone = {}
	for _, Child in Made do
		Child.Destroying:Connect(function()
			table.insert(Gone, string.sub(Child.Name, 2))
		end)
	end
	Run()
	-- (Destroying is deferred: its handlers run at the next resumption point, in the order the children went)
	task.wait()
	return table.concat(Gone, " ")
end
for _, Count in { 20, 25 } do
	for _, Taken in { 0, 3 } do
		local Folder, Made = Box(Count)
		if Taken > 0 then
			Made[Taken].Parent = nil
		end
		print(`destroy {Count} taken {Taken}: {Order(Folder, Made, function()
			Folder:Destroy()
		end)}`)
	end
end
do
	-- ClearAllChildren: the order of Destroying and of ChildRemoved
	for _, Count in { 20, 25 } do
		for _, Taken in { 0, 3 } do
			local Folder, Made = Box(Count)
			if Taken > 0 then
				Made[Taken].Parent = nil
			end
			local Seen = {}
			local Connection = Folder.ChildRemoved:Connect(function(Child)
				table.insert(Seen, string.sub(Child.Name, 2))
			end)
			print(`clear {Count} taken {Taken}: {Order(Folder, Made, function()
				Folder:ClearAllChildren()
			end)}`)
			task.wait()
			print(`clear {Count} taken {Taken} removed: {table.concat(Seen, " ")}`)
			Connection:Disconnect()
			Folder:Destroy()
		end
	end
end
do
	-- Destroy of a child that has children of its own, in a long list
	local Folder, Made = Box(22)
	Fill(Made[4], 22, "g")
	local Inner = Made[4]:GetChildren()
	local Gone = {}
	for _, Child in Inner do
		Child.Destroying:Connect(function()
			table.insert(Gone, string.sub(Child.Name, 2))
		end)
	end
	Inner[2].Parent = nil
	Made[4]:Destroy()
	task.wait()
	print("destroy-nested " .. table.concat(Gone, " ") .. " | " .. Names(Folder, "c"))
	Folder:Destroy()
end

-- 6. GetDescendants after removals at two levels
do
	local Folder, Made = Box(22)
	Fill(Made[2], 3, "g")
	Fill(Made[22], 22, "h")
	Made[1].Parent = nil
	Made[22]:FindFirstChild("h4").Parent = nil
	local Out = {}
	for _, Child in Folder:GetDescendants() do
		table.insert(Out, Child.Name)
	end
	print("descendants " .. table.concat(Out, " "))
	Folder:Destroy()
end

-- 7. which of several children of one name FindFirstChild and the rest find after a removal
for _, Count in { 20, 21, 25 } do
	local Folder = Instance.new("Folder")
	local Made = {}
	for Index = 1, Count do
		local Child = Instance.new(if Index % 2 == 0 then "Configuration" else "Folder")
		Child.Name = "dup"
		Child:SetAttribute("Index", Index)
		Child.Parent = Folder
		Made[Index] = Child
	end
	Made[1].Parent = nil
	local function Index(Child)
		return if Child then tostring(Child:GetAttribute("Index")) else "nil"
	end
	print(`duplicates {Count}: first {Index(Folder:FindFirstChild("dup"))} dot {Index(Folder.dup)} wait {Index(Folder:WaitForChild("dup"))} class {Index(Folder:FindFirstChildOfClass("Folder"))} isa {Index(Folder:FindFirstChildWhichIsA("Configuration"))}`)
	Made[2].Parent = nil
	print(`duplicates {Count} again: first {Index(Folder:FindFirstChild("dup"))} class {Index(Folder:FindFirstChildOfClass("Configuration"))}`)
	Folder:Destroy()
end

-- 8. a Clone of a list a removal reordered
do
	local Folder, Made = Box(22)
	Made[3].Parent = nil
	local Copy = Folder:Clone()
	print("clone " .. Names(Copy, "c"))
	Copy:Destroy()
	Folder:Destroy()
end

-- 9. each kind of container: in the Workspace, a Folder and a Model in it, a Part, ReplicatedStorage, Lighting, and the Workspace
-- itself (its own children count towards the list, so only the probe's are printed)
local Containers = {
	{ "folder-in-workspace", function()
		local Folder = Instance.new("Folder")
		Folder.Parent = workspace
		return Folder, true
	end },
	{ "model-in-workspace", function()
		local Model = Instance.new("Model")
		Model.Parent = workspace
		return Model, true
	end },
	{ "part", function()
		local Part = Instance.new("Part")
		Part.Anchored = true
		Part.Position = Vector3.new(0, 300, 0)
		Part.Parent = workspace
		return Part, true
	end },
	{ "workspace", function()
		return workspace, false
	end },
	{ "replicated-storage", function()
		return game:GetService("ReplicatedStorage"), false
	end },
	{ "lighting", function()
		return game:GetService("Lighting"), false
	end },
}
for _, Entry in Containers do
	local Label, Make = Entry[1], Entry[2]
	-- (a service's own children differ between places: only lists that are long without them)
	for _, Count in { 20, 25 } do
		local Container, Own = Make()
		if Own or Count > 20 then
			local Made = Fill(Container, Count, "q", if Label == "part" then "Attachment" else "Folder")
			Made[2].Parent = nil
			print(`{Label} {Count}: {Names(Container, "q")}`)
			for _, Child in Made do
				Child:Destroy()
			end
		end
		if Own then
			Container:Destroy()
		end
	end
end

-- 10. the order spatial queries list parts in, and which of equal hits a ray takes, after a removal
for _, Count in { 20, 25 } do
	local Folder = Instance.new("Folder")
	Folder.Parent = workspace
	local Made = {}
	for Index = 1, Count do
		local Part = Instance.new("Part")
		Part.Name = "p" .. Index
		Part.Anchored = true
		Part.Size = Vector3.new(2, 2, 2)
		Part.Position = Vector3.new(300, 300, 300)
		Part.Parent = Folder
		Made[Index] = Part
	end
	local function Listed(Parts)
		local Out = {}
		for _, Part in Parts do
			if Part.Parent == Folder then
				table.insert(Out, string.sub(Part.Name, 2))
			end
		end
		return table.concat(Out, " ")
	end
	local Box = CFrame.new(300, 300, 300)
	print(`query {Count} before: {Listed(workspace:GetPartBoundsInBox(Box, Vector3.new(4, 4, 4)))}`)
	Made[3].Parent = nil
	Made[1].Parent = nil
	print(`query {Count} box: {Listed(workspace:GetPartBoundsInBox(Box, Vector3.new(4, 4, 4)))}`)
	print(`query {Count} radius: {Listed(workspace:GetPartBoundsInRadius(Vector3.new(300, 300, 300), 3))}`)
	local Probe = Instance.new("Part")
	Probe.Anchored = true
	Probe.Size = Vector3.new(4, 4, 4)
	Probe.CFrame = Box
	print(`query {Count} part: {Listed(workspace:GetPartsInPart(Probe))}`)
	local Hit = workspace:Raycast(Vector3.new(300, 320, 300), Vector3.new(0, -40, 0))
	print(`query {Count} ray: {if Hit then string.sub(Hit.Instance.Name, 2) else "nil"}`)
	Probe:Destroy()
	Folder:Destroy()
end
