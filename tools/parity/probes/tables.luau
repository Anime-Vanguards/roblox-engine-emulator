-- table.sort (order of equal keys, bad comparators), the length operator on tables with holes, and the table library.
local Rng = P.rng(4242)

-- 1. sorting records by a key with many ties: the order of equal keys shows the algorithm
for Case = 1, 12 do
	local Count = Rng.pick({ 5, 8, 12, 20, 33, 64, 100 })
	local Records = {}
	for Index = 1, Count do
		Records[Index] = { Key = Rng.int(1, 4), Id = Index }
	end
	table.sort(Records, function(A, B)
		return A.Key < B.Key
	end)
	local Ids = {}
	for _, Record in Records do
		table.insert(Ids, Record.Id)
	end
	print("sort-ties " .. Case .. " " .. table.concat(Ids, ","))
end
-- numbers, strings, the default comparator
for Case = 1, 6 do
	local List = {}
	for Index = 1, Rng.int(5, 40) do
		List[Index] = Rng.int(-50, 50)
	end
	table.sort(List)
	print("sort-numbers " .. Case .. " " .. table.concat(List, ","))
end
local Words = { "b", "a", "B", "aa", "", "é", "Z", "a\0", "10", "9" }
table.sort(Words)
print("sort-strings " .. table.concat(Words, "|"))
print("sort-mixed " .. P.try(table.sort, { 1, "a", 2 }))
print("sort-nan " .. (function()
	local List = { 3, 0 / 0, 1, 0 / 0, 2 }
	local Ok, Message = pcall(table.sort, List)
	local Parts = {}
	for _, Value in List do
		table.insert(Parts, P.num(Value))
	end
	return (if Ok then "ok " else "error " .. P.err(Message) .. " ") .. table.concat(Parts, ",")
end)())
-- comparators that lie
for Case = 1, 6 do
	local List = {}
	for Index = 1, Rng.int(8, 60) do
		List[Index] = Rng.int(1, 10)
	end
	local Ok, Message = pcall(table.sort, List, function(A, B)
		return A <= B
	end)
	print("sort-lte " .. Case .. " " .. (if Ok then "ok " .. table.concat(List, ",") else "error " .. P.err(Message)))
end
local Ok, Message = pcall(table.sort, { 3, 2, 1, 5, 4 }, function()
	return true
end)
print("sort-always-true " .. tostring(Ok) .. " " .. P.err(Message))
print("sort-error-in-comparator " .. P.try(table.sort, { 2, 1 }, function()
	error("cmp")
end))
print("sort-yield " .. P.try(table.sort, { 2, 1, 3 }, function(A, B)
	return A < B
end))

-- 2. the length of tables with holes
local function Length(Builder)
	local Table = Builder()
	return tostring(#Table)
end
local Shapes = {
	{ "constructor-hole", function() return { 1, nil, 3 } end },
	{ "constructor-trailing-nil", function() return { 1, 2, nil } end },
	{ "constructor-holes", function() return { 1, nil, 3, nil, 5, nil, nil, 8 } end },
	{ "assigned-gap", function() local T = {} T[1] = 1 T[2] = 2 T[4] = 4 return T end },
	{ "removed-middle", function() local T = { 1, 2, 3, 4, 5 } T[3] = nil return T end },
	{ "removed-end", function() local T = { 1, 2, 3, 4, 5 } T[5] = nil return T end },
	{ "hash-only", function() local T = {} T[2] = 2 T[3] = 3 return T end },
	{ "sparse-big", function() local T = {} for I = 1, 10 do T[I] = I end T[20] = 20 return T end },
	{ "after-clear", function() local T = { 1, 2, 3 } for I = 1, 3 do T[I] = nil end return T end },
	{ "insert-gap", function() local T = { 1, 2 } T[4] = 4 table.insert(T, 9) return T end },
	{ "create", function() return table.create(5) end },
	{ "create-filled", function() return table.create(5, 0) end },
	{ "pack-nils", function() return table.pack(1, nil, nil) end },
}
for _, Shape in Shapes do
	print("len " .. Shape[1] .. " " .. Length(Shape[2]))
end
for Case = 1, 10 do
	local T = {}
	for _ = 1, Rng.int(1, 30) do
		T[Rng.int(1, 20)] = true
	end
	for _ = 1, Rng.int(0, 5) do
		T[Rng.int(1, 20)] = nil
	end
	print("len-generated " .. Case .. " " .. #T)
end

-- 3. the rest of the table library
local T = { 1, 2, 3 }
table.insert(T, 2, "x")
print("insert-middle " .. table.concat(T, ","))
print("insert-out-of-range " .. P.try(table.insert, { 1, 2 }, 5, "y"))
print("insert-zero " .. P.try(table.insert, { 1, 2 }, 0, "y"))
print("remove " .. tostring(table.remove(T)) .. " " .. tostring(table.remove(T, 1)) .. " " .. table.concat(T, ","))
print("remove-empty " .. select("#", table.remove({})) .. " remove-past " .. P.try(table.remove, { 1 }, 5))
print("concat " .. table.concat({ 1, 2.5, "x" }, "-") .. " " .. table.concat({}, ",") .. "| " .. table.concat({ 1, 2, 3 }, ",", 2, 3))
print("concat-bad " .. P.try(table.concat, { 1, {}, 3 }))
print("concat-nil " .. P.try(table.concat, { 1, nil, 3 }, ",", 1, 3))
print("unpack " .. select("#", table.unpack({ 1, nil, 3 })) .. " " .. select("#", table.unpack({ 1, 2, 3 }, 2)) .. " " .. select("#", unpack({}, 1, 3)))
print("unpack-too-many " .. P.try(function()
	return select("#", table.unpack({}, 1, 1e7))
end))
print("find " .. tostring(table.find({ "a", "b", "a" }, "a", 2)) .. " " .. tostring(table.find({ 1, 2 }, 3)) .. " " .. tostring(table.find({ 0 / 0 }, 0 / 0)))
print("move " .. table.concat(table.move({ 1, 2, 3 }, 1, 3, 2), ","))
print("clone " .. (function()
	local Original = setmetatable({ 1, { 2 } }, { __index = { Extra = 1 } })
	local Copy = table.clone(Original)
	return tostring(Copy[2] == Original[2]) .. " " .. tostring(getmetatable(Copy) == getmetatable(Original)) .. " " .. tostring(Copy.Extra)
end)())
print("clone-locked " .. P.try(table.clone, setmetatable({}, { __metatable = "locked" })))
local Frozen = table.freeze({ 1, Inner = {} })
print("freeze " .. tostring(table.isfrozen(Frozen)) .. " " .. tostring(table.isfrozen(Frozen.Inner)) .. " write " .. P.try(function()
	Frozen[1] = 2
end) .. " rawset " .. P.try(rawset, Frozen, 2, 2) .. " setmetatable " .. P.try(setmetatable, Frozen, {}))
print("freeze-twice " .. P.try(table.freeze, Frozen))
print("clear " .. (function()
	local Cleared = { 1, 2, A = 3 }
	table.clear(Cleared)
	return tostring(next(Cleared)) .. " " .. #Cleared
end)())
print("maxn " .. tostring(table.maxn and table.maxn({ 1, 2, [10] = 3 })))
print("getn " .. tostring(table.getn and table.getn({ 1, 2 })))
print("foreach " .. tostring(table.foreach ~= nil) .. " foreachi " .. tostring(table.foreachi ~= nil))
print("pack " .. (function()
	local Packed = table.pack(1, nil, 3)
	return Packed.n .. " " .. tostring(Packed[2])
end)())
print("next-order " .. (function()
	local Array = { 10, 20, 30, 40 }
	local Keys = {}
	for Key in pairs(Array) do
		table.insert(Keys, Key)
	end
	return table.concat(Keys, ",")
end)())
print("ipairs-stops " .. (function()
	local Count = 0
	for _ in ipairs({ 1, 2, nil, 4 }) do
		Count += 1
	end
	return Count
end)())
print("generalized-iteration " .. (function()
	local Keys = {}
	for Key in { 5, 6, 7 } do
		table.insert(Keys, Key)
	end
	return table.concat(Keys, ",")
end)())
print("modify-during-next " .. P.try(function()
	local Map = { A = 1, B = 2, C = 3 }
	for Key in pairs(Map) do
		Map[Key .. "x"] = 1
	end
	return "no error"
end))
-- the order pairs visits string and number keys in (hash order: the same VM gives the same order)
for Case = 1, 8 do
	local Map = {}
	local Count = Rng.int(3, 24)
	for Index = 1, Count do
		local Kind = Rng.int(1, 3)
		if Kind == 1 then
			Map["key" .. Rng.int(1, 999)] = Index
		elseif Kind == 2 then
			Map[Rng.pick({ "Health", "Damage", "Speed", "Range", "Cost", "Name", "Id", "Level", "a", "b", "zz", "Position", "Size" })] = Index
		else
			Map[Rng.int(1, 64) * Rng.pick({ 1, -1, 0.5 })] = Index
		end
	end
	local Keys = {}
	for Key in pairs(Map) do
		table.insert(Keys, tostring(Key))
	end
	print("pairs-order " .. Case .. " " .. table.concat(Keys, ","))
end
local Removed = { a = 1, b = 2, c = 3, d = 4, e = 5, f = 6 }
Removed.c = nil
Removed.g = 7
local RemovedKeys = {}
for Key in pairs(Removed) do
	table.insert(RemovedKeys, Key)
end
print("pairs-after-remove " .. table.concat(RemovedKeys, ","))
