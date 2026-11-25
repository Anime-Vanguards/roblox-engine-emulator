-- Selection's capability-gated methods and Terrain:CountCells on a playing client and server (script identity 2).
local Selection = game:GetService("Selection")
local Terrain = workspace.Terrain

local function Info(Fn)
	local S, L, N, A, V = debug.info(Fn, "slna")
	return tostring(S) .. "|" .. tostring(L) .. "|" .. tostring(N) .. "|" .. tostring(A) .. "|" .. tostring(V)
end

local SelectionMethods = {
	"Get",
	"Set",
	"Add",
	"Remove",
	"AddFocusCallback",
	"ClearTerrainSelectionHack",
	"SetTerrainSelectionHack",
}
for _, Name in SelectionMethods do
	local Fn = Selection[Name]
	print("sel " .. Name .. " " .. typeof(Fn) .. " " .. Info(Fn) .. " same=" .. tostring(rawequal(Fn, Selection[Name]))
		.. " | call " .. P.try(function() return Fn(Selection) end)
		.. " | arg " .. P.try(function() return Fn(Selection, {}) end)
		.. " | other " .. P.try(function() return Fn(workspace) end)
		.. " | none " .. P.try(function() return Fn() end))
end
local function Clean(Text)
	local Game = string.gsub(game.Name, "%p", "%%%0")
	return (string.gsub(Text, "current parent: " .. Game .. ",", "current parent: <game>,"))
end

print("sel-lower-get " .. tostring(rawequal(Selection.get, Selection.Get)) .. " | " .. P.try(function() return Selection:get() end))
print("sel-lower-remove " .. Clean(P.try(function() return Selection:remove() end)))
print("sel-is-plugin " .. tostring(pcall(function() return Selection:Get() end)))

print("terrain-read " .. typeof(Terrain.CountCells) .. " " .. Info(Terrain.CountCells) .. " same=" .. tostring(rawequal(Terrain.CountCells, Terrain.countCells)))
print("terrain-calls " .. P.try(function()
	return Terrain:CountCells(), Terrain:countCells(), Terrain:CountCells(1, "extra")
end))
print("terrain-errors "
	.. P.try(function() return Terrain.CountCells() end) .. " | "
	.. P.try(function() return Terrain.CountCells(workspace) end) .. " | "
	.. P.try(function() return Terrain.CountCells(123) end))
