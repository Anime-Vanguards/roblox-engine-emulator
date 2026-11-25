-- Lowercase `:lerp` and `.lerp` on Color3 and the other datatypes that list it (Vector3, Vector2, UDim2, CFrame):
-- library tables expose neither `lerp` nor `Lerp`, instance reads return a `[C]` function named `lerp` distinct from
-- `Lerp`, and Color3 / Vector3 calls and argument errors match the client.
local function Info(Fn)
	local S, L, N, A, V = debug.info(Fn, "slna")
	return tostring(S) .. "|" .. tostring(L) .. "|" .. tostring(N) .. "|" .. tostring(A) .. "|" .. tostring(V)
end

print("lib-color3 " .. P.try(function()
	return Color3.lerp, Color3.Lerp, typeof(Color3.toHSV), Info(Color3.toHSV), Color3.ToHSV
end))
print("lib-others " .. P.try(function()
	return Vector3.lerp, Vector3.Lerp, Vector2.lerp, UDim2.lerp, CFrame.lerp
end))

local C1, C2 = Color3.new(0.25, 0.5, 0.75), Color3.new(0.75, 0.25, 0.5)
local V1, V2 = Vector3.new(1, 2, 3), Vector3.new(4, 5, 6)
local Samples = {
	{ "Color3", C1 },
	{ "Vector3", V1 },
	{ "Vector2", Vector2.new(1, 2) },
	{ "UDim2", UDim2.new(0, 1, 0, 2) },
	{ "CFrame", CFrame.new(1, 2, 3) },
}
for _, Entry in Samples do
	local Name, Value = Entry[1], Entry[2]
	print("read " .. Name .. " " .. Info(Value.lerp) .. " " .. Info(Value.Lerp) .. " same=" .. tostring(rawequal(Value.lerp, Value.Lerp)))
end

print("c3-values " .. P.try(function()
	return C1:lerp(C2, 0.5) == C1:Lerp(C2, 0.5),
		C1.lerp(C1, C2, 0.5) == C1:Lerp(C2, 0.5),
		tostring(C1:lerp(C2, -0.5)),
		tostring(C1:lerp(C2, 1.5)),
		C1:lerp(C2, "0.5") == C1:Lerp(C2, 0.5),
		C1:lerp(C2, true) == C2,
		C1:lerp(C2, false) == C1,
		C1:lerp(C2, 0.5, "extra") == C1:Lerp(C2, 0.5)
end))
print("c3-errors "
	.. P.try(function() return C1:lerp() end) .. " | "
	.. P.try(function() return C1:lerp(C2) end) .. " | "
	.. P.try(function() return C1:lerp(1, 0.5) end) .. " | "
	.. P.try(function() return C1:lerp(C2, "bad") end) .. " | "
	.. P.try(function() return C1.lerp() end) .. " | "
	.. P.try(function() C1.lerp = 1 end) .. " | "
	.. P.try(function() C1.r = 1 end))

print("v3-values " .. P.try(function()
	return V1:lerp(V2, 0.5) == V1:Lerp(V2, 0.5), V1.lerp(V1, V2, 0.5) == V1:Lerp(V2, 0.5)
end))
print("v3-dot-errors "
	.. P.try(function() return V1.lerp() end) .. " | "
	.. P.try(function() return V1.lerp(1) end) .. " | "
	.. P.try(function() return V1.lerp(V1) end) .. " | "
	.. P.try(function() return V1.lerp(V1, 1) end) .. " | "
	.. P.try(function() return V1.lerp(V1, V2) end) .. " | "
	.. P.try(function() return V1.lerp(V1, V2, "x") end))
print("v3-col-errors "
	.. P.try(function() return V1:lerp() end) .. " | "
	.. P.try(function() return V1:lerp(1) end) .. " | "
	.. P.try(function() return V1:lerp(V2) end) .. " | "
	.. P.try(function() return V1:lerp(V2, "x") end))
