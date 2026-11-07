-- Members by their name with the first letter lower-cased ("getPropertyChangedSignal"): reads and method calls of an
-- instance reach every member that way, after its exact members and its children; writes take exact names only.
-- Datatypes, enums and libraries answer only the lower-case names they list.
local function Line(Label, ...)
	local Parts = { Label }
	for Index = 1, select("#", ...) do
		table.insert(Parts, (select(Index, ...)))
	end
	print(table.concat(Parts, " | "))
end
local function Same(A, B)
	return P.try(function()
		return rawequal(A(), B())
	end)
end

local Holder = Instance.new("Folder")
Holder.Name = "Holder"
local Part = Instance.new("Part")
Part.Name = "P"
Part.Parent = Holder

-- 1. reads: methods (the very function the exact name gives), properties, events (the same signal), inherited members
Line("read-method",
	Same(function() return Part.getPropertyChangedSignal end, function() return Part.GetPropertyChangedSignal end),
	Same(function() return Part.getFullName end, function() return Part.GetFullName end),
	Same(function() return Part.isAncestorOf end, function() return Part.IsAncestorOf end),
	Same(function() return Part.getAttribute end, function() return Part.GetAttribute end),
	Same(function() return Part.getPivot end, function() return Part.GetPivot end),
	P.try(function() return typeof(Part.getConnectedParts) end))
Line("read-property",
	P.try(function() return Part.anchored, Part.transparency, Part.name, Part.parent, Part.locked end),
	P.try(function() return Part.position == Part.Position, Part.cFrame == Part.CFrame, Part.material == Part.Material end),
	-- (BasePart.size, the dump's own deprecated name, permits no script: the read finds Size)
	P.try(function() return Part.size == Part.Size end))
Line("read-event",
	P.try(function() return Part.touched == Part.Touched, tostring(Part.touched) end),
	P.try(function() return Part.changed == Part.Changed, tostring(Part.changed) end),
	P.try(function() return tostring(Part.childRemoved), tostring(Part.destroying), tostring(Part.attributeChanged) end))
local Mesh = Instance.new("MeshPart")
local Frame = Instance.new("Frame")
local Model = Instance.new("Model")
Line("read-inherited",
	P.try(function() return Mesh.anchored, Mesh.canCollide, Mesh.className end),
	P.try(function() return Frame.visible, Frame.backgroundTransparency, Frame.zIndex end),
	P.try(function() return Model.primaryPart, typeof(Model.getBoundingBox) end))
Line("read-service",
	P.try(function() return workspace.gravity == workspace.Gravity, workspace.currentCamera == workspace.CurrentCamera end),
	P.try(function() local Lighting = game:GetService("Lighting") return Lighting.brightness == Lighting.Brightness, Lighting.clockTime == Lighting.ClockTime end))

-- 2. callbacks: a read names the callback by its own name; the write needs it
local Function = Instance.new("BindableFunction")
Function.Name = "F"
Line("callback-read", P.try(function() return Function.onInvoke end), P.try(function() return Function.OnInvoke end))
Line("callback-write", P.try(function() Function.onInvoke = function() return 7 end end))
Function.OnInvoke = function()
	return 8
end
Line("callback-invoke", P.try(function() return Function:invoke() end), P.try(function() return Function:Invoke() end))

-- 3. writes take exact names only
Line("write",
	P.try(function() Part.anchored = true end),
	P.try(function() Part.position = Vector3.new(1, 2, 3) end),
	P.try(function() Part.name = "Q" end),
	P.try(function() Part.parent = nil end),
	P.try(function() Part.size = Vector3.new(4, 5, 6) end))
Line("write-left", P.try(function() return Part.Anchored, Part.Name, Part.Parent, Part.Position == Vector3.zero, Part.Size.Y end))

-- 4. a child with the name comes first; children answer only to their own names
local Box = Instance.new("Part")
Box.Name = "Box"
for _, Name in { "anchored", "touched", "getPropertyChangedSignal", "Anchored", "foo" } do
	local Child = Instance.new("Folder")
	Child.Name = Name
	Child.Parent = Box
end
Line("child",
	P.try(function() return Box.anchored.ClassName, Box.touched.ClassName, Box.getPropertyChangedSignal.ClassName end),
	P.try(function() return Box.Anchored end),
	P.try(function() return Box.Foo end))

-- 5. method calls: methods only (children and other members raise), the exact name first
Line("namecall",
	P.try(function() return tostring(Part:getPropertyChangedSignal("Name")) end),
	P.try(function() return Part:getPropertyChangedSignal("Name") == Part:GetPropertyChangedSignal("Name") end),
	P.try(function() return Part:getFullName(), Part:isAncestorOf(Part), Holder:findFirstChildOfClass("Part").Name end),
	P.try(function() return tostring(Box:getPropertyChangedSignal("Name")) end))
Line("namecall-attributes",
	P.try(function() Part:setAttribute("A", 5) return Part:getAttribute("A"), Part:GetAttribute("A") end),
	P.try(function() Part:addTag("T") return Part:hasTag("T"), Part:HasTag("T") end))
Line("namecall-not-method",
	P.try(function() return Part:anchored() end),
	P.try(function() return Part:Anchored() end),
	P.try(function() return Part:touched() end),
	P.try(function() return Part:Touched() end),
	P.try(function() return Box:anchored() end),
	P.try(function() return Box:foo() end),
	P.try(function() return Function:onInvoke() end))

-- 6. the first letter only, and only towards upper case
local Http = game:GetService("HttpService")
Line("first-letter",
	P.try(function() return Http:jSONEncode({ 1 }) end),
	P.try(function() return Http:jsonEncode({ 1 }) end),
	P.try(function() return Part.ANCHORED end),
	P.try(function() return Part.aNCHORED end),
	P.try(function() return Part.cframe end),
	P.try(function() return Part.getpropertychangedsignal end))
Line("unknown",
	P.try(function() return Part.fooBar end),
	P.try(function() return Part.FooBar end),
	P.try(function() return Part:fooBar() end),
	P.try(function() Part.fooBar = 1 end))

-- 7. datatypes: the lower-case names each lists ("-unknown" lines: Roblox's error for the rest)
local V = Vector3.new(1, 2, 3)
Line("vector3", P.try(function() return V.x, V.y, V.z, V.magnitude == V.Magnitude, V.unit == V.Unit, V:lerp(Vector3.zero, 0.5) == V:Lerp(Vector3.zero, 0.5) end))
Line("vector3-unknown", P.try(function() return V:dot(V) end))
local V2 = Vector2.new(3, 4)
Line("vector2", P.try(function() return V2.x, V2.y, V2.magnitude, V2.unit == V2.Unit, V2:lerp(V2, 0) == V2 end))
Line("int16", P.try(function() local I = Vector3int16.new(1, 2, 3) return I.x, I.y, I.z end), P.try(function() local I = Vector2int16.new(4, 5) return I.x, I.y end))
local CF = CFrame.new(1, 2, 3)
Line("cframe",
	P.try(function() return CF.p == CF.Position, CF.x, CF.y, CF.z, CF.lookVector == CF.LookVector, CF.rightVector == CF.RightVector, CF.upVector == CF.UpVector end),
	P.try(function() return CF:inverse() == CF:Inverse(), CF:lerp(CF, 1) == CF, CF:toObjectSpace(CF) == CFrame.identity, CF:pointToObjectSpace(V) == CF:PointToObjectSpace(V) end),
	P.try(function() return P.nums(CF:toOrientation()) == P.nums(CF:ToOrientation()), P.nums(CF:toEulerAnglesYXZ()) == P.nums(CF:ToEulerAnglesYXZ()), P.nums(CF:toEulerAnglesXYZ()) == P.nums(CF:ToEulerAnglesXYZ()), #{ CF:components() } end))
Line("cframe-unknown", P.try(function() return CF.position end), P.try(function() return CF:fuzzyEq(CF) end))
local C3 = Color3.new(0.25, 0.5, 0.75)
Line("color3", P.try(function() return C3.r, C3.g, C3.b, C3:lerp(C3, 1) == C3 end))
Line("color3-unknown", P.try(function() return C3:toHSV() end))
local U2 = UDim2.new(1, 2, 3, 4)
Line("udim2", P.try(function() return U2:lerp(U2, 1) == U2 end))
Line("udim-unknown", P.try(function() return U2.x end), P.try(function() return UDim.new(1, 2).scale end))
local Bricks = BrickColor.new("Bright red")
Line("brickcolor", P.try(function() return Bricks.name, Bricks.number, Bricks.r == Bricks.Color.R end))
Line("brickcolor-unknown", P.try(function() return Bricks.R end), P.try(function() return Bricks.color end))
Line("ray", P.try(function() return Ray.new(Vector3.zero, Vector3.new(0, 0, 2)).unit.Direction.Z end))
Line("ray-unknown", P.try(function() return Ray.new(Vector3.zero, Vector3.one).origin end))
Line("datatype-unknown",
	P.try(function() return Random.new(1):nextNumber() end),
	P.try(function() return TweenInfo.new(1).time end),
	P.try(function() return NumberRange.new(1, 2).min end),
	P.try(function() return Rect.new(0, 0, 1, 1).width end),
	P.try(function() return Faces.new(Enum.NormalId.Top).top end))
local Signal = Part.Changed
Line("signal",
	P.try(function() local C = Signal:connect(function() end) local Was = C.connected C:disconnect() return Was, C.Connected end),
	P.try(function() return typeof(Signal.wait), typeof(Signal.connectParallel) end))
Line("signal-unknown", P.try(function() return Signal:once(function() end) end))
Line("enum-unknown",
	P.try(function() return Enum.KeyCode.A.name end),
	P.try(function() return Enum.KeyCode:getEnumItems() end),
	P.try(function() return Enum.keyCode end))
Line("library",
	P.try(function() return Vector3.New, Instance.New, CFrame.angles, UDim2.FromScale, math.Floor, typeof(BrickColor.New) end))

-- 8. the report's case: a property signal by a lower-case method name, its handler run (newest connection first)
local Report = Instance.new("Part")
local Order = {}
Report:GetPropertyChangedSignal("Position"):Connect(function()
	table.insert(Order, "one")
end)
Line("report-connect", P.try(function()
	Report:getPropertyChangedSignal("Position"):Connect(function()
		table.insert(Order, "two")
	end)
end))
Report.Position += Vector3.new(0, 1, 0)
task.wait()
task.wait()
Line("report-order", table.concat(Order, ","))
