-- Where a joint's C0 write starts to orthonormalize, against a part's CFrame write: rotations off orthonormal by a column
-- leaning out, a skew, a scale and one stretched column, at errors from 1e-7 to 1.2e-5, each read back kept or changed.
local C = math.cos(0.4)
local S = math.sin(0.4)
local Weld = Instance.new("Weld")
local Part = Instance.new("Part")
local function Try(Kind, Error)
	local Written
	if Kind == "column" then
		Written = CFrame.new(0, 0, 0, C, -S, Error, S, C, 0, 0, 0, 1)
	elseif Kind == "skew" then
		Written = CFrame.new(0, 0, 0, C, -S + Error, 0, S, C, Error, 0, 0, 1)
	elseif Kind == "scaled" then
		local K = 1 + Error
		Written = CFrame.new(0, 0, 0, C * K, -S * K, 0, S * K, C * K, 0, 0, 0, K)
	else
		Written = CFrame.new(0, 0, 0, 1, 0, 0, 0, 1 + Error, 0, 0, 0, 1)
	end
	Weld.C0 = Written
	Part.CFrame = Written
	local A = { Written:GetComponents() }
	local B = { Weld.C0:GetComponents() }
	local D = { Part.CFrame:GetComponents() }
	local Same, PartSame = true, true
	for I = 1, 12 do
		if A[I] ~= B[I] then
			Same = false
		end
		if A[I] ~= D[I] then
			PartSame = false
		end
	end
	print(Kind .. " " .. P.num(Error) .. " c0 " .. (if Same then "kept" else "changed") .. " part " .. (if PartSame then "kept" else "changed"))
end
for _, Kind in { "column", "skew", "scaled", "stretch" } do
	for _, Error in { 1e-7, 1.5e-7, 2e-7, 2.5e-7, 3e-7, 3.5e-7, 4e-7, 4.5e-7, 5e-7, 5.5e-7, 6e-7, 7e-7, 8e-7, 9e-7, 1e-6, 2e-6, 4e-6, 6e-6, 8e-6, 1e-5, 1.2e-5 } do
		Try(Kind, Error)
	end
end
