-- The rigid joints' algebra and when a joint links (Roblox's Assemblies guide until now): where each joint class puts its
-- second part (Weld, Snap, ManualWeld, Motor, Motor6D with a Transform, WeldConstraint) from C0 and C1, which part moves
-- when the other is anchored, what a Position write does to the joint's C0 / C1, which joints do not link (disabled,
-- a missing or the same part, outside a world, parts in two worlds, both anchored), a model taken into the Workspace with
-- its welds, a weld that ends, and the order a query lists the parts of each.
local function CF(Value)
	return P.f32s(Value:GetComponents())
end
local function Close(A, B)
	return (A.Position - B.Position).Magnitude < 1e-3 and A.LookVector:Dot(B.LookVector) > 0.99999 and A.UpVector:Dot(B.UpVector) > 0.99999
end
workspace.Gravity = 0
local Base = CFrame.new(200, 50, -200)
local Count = 0
local function NewPart(Name, Anchored, Where)
	Count += 1
	local Part = Instance.new("Part")
	Part.Name = Name
	Part.Size = Vector3.new(2, 1, 1)
	Part.Anchored = Anchored
	Part.CFrame = Base * CFrame.new(Count * 10, 0, 0) * CFrame.Angles(0.1 * Count, 0.2, 0)
	Part.Parent = Where or workspace
	return Part
end
local function Root(Part)
	local Found = Part.AssemblyRootPart
	return if Found then Found.Name else "nil"
end
local C0 = CFrame.new(1, 2, 3) * CFrame.Angles(0.3, 0.5, 0.7)
local C1 = CFrame.new(-0.5, 0.25, 1) * CFrame.Angles(-0.2, 0.1, 0.4)
local Transform = CFrame.Angles(0, 0.6, 0) + Vector3.new(0, 0.5, 0)

-- each joint class, second part free
for _, Class in { "Weld", "Snap", "ManualWeld", "Motor", "Motor6D" } do
	local A = NewPart("A", true)
	local B = NewPart("B", false)
	local Joint = Instance.new(Class)
	Joint.Part0 = A
	Joint.Part1 = B
	Joint.C0 = C0
	Joint.C1 = C1
	Joint.Parent = A
	local Expected = A.CFrame * C0 * C1:Inverse()
	print("joint " .. Class .. " now " .. tostring(Close(B.CFrame, Expected)) .. " root " .. Root(B))
	if Class == "Motor6D" then
		Joint.Transform = Transform
		print("joint Motor6D transform-now " .. tostring(Close(B.CFrame, Expected)))
		task.wait()
		print("joint Motor6D transform-frame " .. tostring(Close(B.CFrame, A.CFrame * C0 * Transform * C1:Inverse())) .. " " .. tostring(Close(B.CFrame, A.CFrame * C0 * C1:Inverse() * Transform)))
	elseif Class == "Motor" then
		Joint.CurrentAngle = 0.5
		task.wait()
		print("joint Motor angle " .. P.f32(Joint.CurrentAngle) .. " plain " .. tostring(Close(B.CFrame, Expected)) .. " z " .. tostring(Close(B.CFrame, A.CFrame * C0 * CFrame.Angles(0, 0, 0.5) * C1:Inverse())))
	else
		task.wait()
		print("joint " .. Class .. " frame " .. tostring(Close(B.CFrame, Expected)))
	end
	-- the anchored side moves, the free one follows
	A.CFrame = A.CFrame * CFrame.new(0, 3, 0)
	print("joint " .. Class .. " follow " .. tostring(Close(B.CFrame, A.CFrame * C0 * (if Class == "Motor6D" then Transform else CFrame.new()) * C1:Inverse())))
	-- a Position write of the second part rewrites the joint
	B.Position += Vector3.new(0, 0, 4)
	print("joint " .. Class .. " position-write c0 " .. tostring(Joint.C0 == C0) .. " c1 " .. tostring(Joint.C1 == C1) .. " holds " .. tostring(Close(B.CFrame, A.CFrame * Joint.C0 * (if Class == "Motor6D" then Joint.Transform else CFrame.new()) * Joint.C1:Inverse())))
	A:Destroy()
	B:Destroy()
end

-- the first part free, the second anchored
do
	local A = NewPart("A", false)
	local B = NewPart("B", true)
	local Weld = Instance.new("Weld")
	Weld.Part0 = A
	Weld.Part1 = B
	Weld.C0 = C0
	Weld.C1 = C1
	Weld.Parent = B
	print("reverse now " .. tostring(Close(A.CFrame, B.CFrame * C1 * C0:Inverse())) .. " root " .. Root(A))
	A:Destroy()
	B:Destroy()
end

-- WeldConstraint: kept where it was made, re-enabled where its parts are then
do
	local A = NewPart("A", true)
	local B = NewPart("B", false)
	local Offset = A.CFrame:ToObjectSpace(B.CFrame)
	local Weld = Instance.new("WeldConstraint")
	Weld.Part0 = A
	Weld.Part1 = B
	Weld.Parent = A
	A.CFrame *= CFrame.new(0, 2, 0) * CFrame.Angles(0, 0.4, 0)
	print("weldconstraint follow " .. tostring(Close(B.CFrame, A.CFrame * Offset)) .. " root " .. Root(B))
	Weld.Enabled = false
	B.CFrame *= CFrame.new(5, 0, 0)
	local Moved = A.CFrame:ToObjectSpace(B.CFrame)
	Weld.Enabled = true
	A.CFrame *= CFrame.new(0, 1, 0)
	print("weldconstraint re-enabled " .. tostring(Close(B.CFrame, A.CFrame * Moved)) .. " " .. tostring(Close(B.CFrame, A.CFrame * Offset)))
	B.Position += Vector3.new(0, 0, 2)
	local After = A.CFrame:ToObjectSpace(B.CFrame)
	A.CFrame *= CFrame.new(0, 1, 0)
	print("weldconstraint position-write " .. tostring(Close(B.CFrame, A.CFrame * After)))
	A:Destroy()
	B:Destroy()
end

-- joints that do not link
local Spare = Instance.new("WorldModel")
Spare.Parent = workspace
local Cases = {
	{ "disabled", function(Weld, A, B)
		Weld.Enabled = false
	end },
	{ "part1-nil", function(Weld, A, B)
		Weld.Part1 = nil
	end },
	{ "same-part", function(Weld, A, B)
		Weld.Part1 = A
	end },
	{ "weld-loose", function(Weld, A, B)
		Weld.Parent = nil
	end },
	{ "weld-in-storage", function(Weld, A, B)
		Weld.Parent = game:GetService("ReplicatedStorage")
	end },
	{ "weld-in-jointsservice", function(Weld, A, B)
		Weld.Parent = game:GetService("JointsService")
	end },
	{ "weld-in-worldmodel", function(Weld, A, B)
		Weld.Parent = Spare
	end },
	{ "part-in-worldmodel", function(Weld, A, B)
		B.Parent = Spare
	end },
	{ "both-anchored", function(Weld, A, B)
		B.Anchored = true
	end },
	{ "weld-in-part1", function(Weld, A, B)
		Weld.Parent = B
	end },
}
for _, Case in Cases do
	local A = NewPart("A", true)
	local B = NewPart("B", false)
	local Weld = Instance.new("Weld")
	Weld.Part0 = A
	Weld.C0 = C0
	Weld.Part1 = B
	Weld.Parent = A
	local Ok, Message = pcall(Case[2], Weld, A, B)
	local Start = B.CFrame
	A.CFrame *= CFrame.new(0, 4, 0)
	task.wait()
	print("inactive " .. Case[1] .. " " .. (if Ok then "" else "E(" .. P.err(Message) .. ") ") .. "moved " .. tostring(not Close(B.CFrame, Start)) .. " root " .. Root(B) .. " joined " .. tostring(Close(B.CFrame, A.CFrame * Weld.C0 * Weld.C1:Inverse())))
	A:Destroy()
	B:Destroy()
end
Spare:Destroy()

-- a model taken into the Workspace with its welds, and the order a query lists its parts
local Region = CFrame.new(200, 50, -400)
local function Listed()
	local Names = {}
	for _, Part in workspace:GetPartBoundsInBox(Region, Vector3.new(200, 60, 60)) do
		table.insert(Names, Part.Name)
	end
	return table.concat(Names, ",")
end
local Model = Instance.new("Model")
local Made = {}
for Index = 1, 6 do
	local Part = Instance.new("Part")
	Part.Name = "m" .. Index
	Part.Size = Vector3.new(1, 1, 1) * (1 + (Index % 3) * 0.5)
	Part.Anchored = Index == 3
	Part.CFrame = Region * CFrame.new(Index * 4 - 14, 0, 0)
	Part.Parent = Model
	Made[Index] = Part
end
for _, Pair in { { 1, 2 }, { 2, 4 }, { 3, 5 }, { 6, 1 } } do
	local Weld = Instance.new("WeldConstraint")
	Weld.Part0 = Made[Pair[1]]
	Weld.Part1 = Made[Pair[2]]
	Weld.Parent = Made[Pair[1]]
end
Model.Parent = workspace
local Roots = {}
for _, Part in Made do
	table.insert(Roots, Part.Name .. ">" .. Root(Part))
end
print("model-in roots " .. table.concat(Roots, ","))
print("model-in listed " .. Listed())
task.wait()
print("model-in listed-frame " .. Listed())
-- a weld that ends
Made[2]:FindFirstChildOfClass("WeldConstraint"):Destroy()
print("weld-ends roots " .. Root(Made[2]) .. "," .. Root(Made[4]) .. " listed " .. Listed())
task.wait()
print("weld-ends listed-frame " .. Listed())
-- an Anchored write relisting a component
Made[1].Anchored = true
print("anchored-write roots " .. Root(Made[1]) .. "," .. Root(Made[2]) .. "," .. Root(Made[6]) .. " listed " .. Listed())
Made[1].Anchored = false
print("unanchored-write roots " .. Root(Made[1]) .. "," .. Root(Made[2]) .. "," .. Root(Made[6]) .. " listed " .. Listed())
task.wait()
print("unanchored-write listed-frame " .. Listed())
Model:Destroy()
