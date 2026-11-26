-- Which part orients a primary-less model's default pivot (the part whose box across the world's axes covers the most ground).
P.sync()
local Turn1, Turn2, Turn3 = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7), CFrame.fromEulerAnglesXYZ(-0.2, 1.1, 0.4), CFrame.fromEulerAnglesXYZ(0.9, -0.3, 0.2)
local Turns = { Turn1, Turn2, Turn3 }
local function Pivot(Label, Sizes, Order)
	local M = Instance.new("Model")
	local Made = {}
	for Index, Size in Sizes do
		local P = Instance.new("Part")
		P.Name = "P" .. Index
		P.Size = Size
		P.Anchored = true
		P.CFrame = CFrame.new(800 + 8 * Index, 50, 40) * Turns[Index]
		Made[Index] = P
	end
	for _, Index in Order or { 1, 2, 3 } do
		if Made[Index] then
			Made[Index].Parent = M
		end
	end
	M.Parent = workspace
	local R = M:GetPivot().Rotation
	local Which = "none"
	for Index, Turn in Turns do
		if R == Made[Index] and Made[Index].CFrame.Rotation or R == Turn.Rotation then
			Which = "P" .. Index
		end
	end
	print("pivot-part", Label, Which)
	M:Destroy()
end
for _, E in { 0.0001, 0.001, 0.01, 0.05, 0.1, 0.5 } do
	Pivot("grow-" .. E, { Vector3.new(2, 2, 2), Vector3.new(2, 2, 2 + E) })
	Pivot("grow-y-" .. E, { Vector3.new(2, 2, 2), Vector3.new(2, 2 + E, 2) })
	Pivot("shrink-" .. E, { Vector3.new(2, 2, 2 + E), Vector3.new(2, 2, 2) })
end
Pivot("a", { Vector3.new(6, 1, 1), Vector3.new(2.5, 2.5, 2.5) })
Pivot("a-rev", { Vector3.new(2.5, 2.5, 2.5), Vector3.new(6, 1, 1) })
Pivot("b", { Vector3.new(4, 4, 0.1), Vector3.new(5, 1, 1) })
Pivot("b-rev", { Vector3.new(5, 1, 1), Vector3.new(4, 4, 0.1) })
Pivot("c", { Vector3.new(3, 3, 0.1), Vector3.new(3.5, 1.5, 1.5) })
Pivot("c-rev", { Vector3.new(3.5, 1.5, 1.5), Vector3.new(3, 3, 0.1) })
Pivot("d", { Vector3.new(2, 1, 2), Vector3.new(2, 2, 2) })
Pivot("d-rev", { Vector3.new(2, 2, 2), Vector3.new(2, 1, 2) })
Pivot("e", { Vector3.new(1, 1, 1), Vector3.new(1, 1, 1) })
Pivot("three", { Vector3.new(2, 2, 2), Vector3.new(3, 3, 3), Vector3.new(2.5, 2.5, 2.5) })
Pivot("three-order", { Vector3.new(2, 2, 2), Vector3.new(3, 3, 3), Vector3.new(2.5, 2.5, 2.5) }, { 3, 1, 2 })
Pivot("made-vs-parented", { Vector3.new(2, 2, 2), Vector3.new(2, 2, 2) }, { 2, 1 })
