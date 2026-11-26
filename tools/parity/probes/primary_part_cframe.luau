-- Model:SetPrimaryPartCFrame and GetPrimaryPartCFrame: casts, a model whose PrimaryPart is unset, gone or destroyed, a primary
-- part with a PivotOffset and a model with a WorldPivot
P.sync()
local function CF(Value)
	if typeof(Value) ~= "CFrame" then
		return tostring(Value)
	end
	return P.f32s(Value:GetComponents())
end
local Model = Instance.new("Model")
local A = Instance.new("Part")
A.Name = "A"
A.Anchored = true
A.Size = Vector3.new(2, 2, 2)
A.CFrame = CFrame.new(1, 2, 3)
A.Parent = Model
local B = Instance.new("Part")
B.Name = "B"
B.Anchored = true
B.CFrame = CFrame.new(5, 2, 3)
B.Parent = Model
Model.Parent = workspace
print("model none set " .. P.try(Model.SetPrimaryPartCFrame, Model, CFrame.new(0, 10, 0)) .. " A " .. CF(A.CFrame))
print("model none get " .. P.try(function() return CF(Model:GetPrimaryPartCFrame()) end))
print("model none set-nil " .. P.try(Model.SetPrimaryPartCFrame, Model, nil))
print("model none set-string " .. P.try(Model.SetPrimaryPartCFrame, Model, "x"))
print("model none set-none " .. P.try(Model.SetPrimaryPartCFrame, Model))
Model.PrimaryPart = A
print("model set-nil " .. P.try(Model.SetPrimaryPartCFrame, Model, nil))
print("model set-string " .. P.try(Model.SetPrimaryPartCFrame, Model, "x"))
print("model set-vector " .. P.try(Model.SetPrimaryPartCFrame, Model, Vector3.one))
print("model set-none " .. P.try(Model.SetPrimaryPartCFrame, Model))
print("model plain set " .. P.try(Model.SetPrimaryPartCFrame, Model, CFrame.new(0, 10, 0)) .. " A " .. CF(A.CFrame) .. " B " .. CF(B.CFrame))
print("model plain get " .. P.try(function() return CF(Model:GetPrimaryPartCFrame()) end) .. " pivot " .. CF(Model:GetPivot()))
A.PivotOffset = CFrame.new(0, 1, 0) * CFrame.Angles(0, math.pi / 2, 0)
print("model offset get " .. P.try(function() return CF(Model:GetPrimaryPartCFrame()) end) .. " pivot " .. CF(Model:GetPivot()))
print("model offset set " .. P.try(Model.SetPrimaryPartCFrame, Model, CFrame.new(0, 20, 0)) .. " A " .. CF(A.CFrame) .. " B " .. CF(B.CFrame))
Model.WorldPivot = CFrame.new(100, 0, 0)
print("model worldpivot get " .. P.try(function() return CF(Model:GetPrimaryPartCFrame()) end) .. " pivot " .. CF(Model:GetPivot()))
print("model worldpivot set " .. P.try(Model.SetPrimaryPartCFrame, Model, CFrame.new(0, 30, 0)) .. " A " .. CF(A.CFrame) .. " B " .. CF(B.CFrame))
-- the primary part taken out of the model
A.Parent = workspace
print("model gone primary " .. tostring(Model.PrimaryPart))
print("model gone set " .. P.try(Model.SetPrimaryPartCFrame, Model, CFrame.new(0, 40, 0)) .. " A " .. CF(A.CFrame) .. " B " .. CF(B.CFrame))
print("model gone get " .. P.try(function() return CF(Model:GetPrimaryPartCFrame()) end))
A.Parent = Model
Model.PrimaryPart = A
A:Destroy()
print("model destroyed primary " .. tostring(Model.PrimaryPart))
print("model destroyed set " .. P.try(Model.SetPrimaryPartCFrame, Model, CFrame.new(0, 50, 0)) .. " B " .. CF(B.CFrame))
-- outside the DataModel
local Loose = Instance.new("Model")
local C = Instance.new("Part")
C.Parent = Loose
Loose.PrimaryPart = C
print("model loose set " .. P.try(Loose.SetPrimaryPartCFrame, Loose, CFrame.new(0, 60, 0)) .. " C " .. CF(C.CFrame))
print("model loose none " .. P.try(Instance.new("Model").SetPrimaryPartCFrame, Instance.new("Model"), CFrame.new()))
print("model loose none get " .. P.try(function() return CF(Instance.new("Model"):GetPrimaryPartCFrame()) end))
Model:Destroy()
