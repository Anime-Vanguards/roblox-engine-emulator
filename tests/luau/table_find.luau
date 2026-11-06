local Needle = {}
assert(table.find({ {}, Needle, {} }, Needle) == 2)
assert(table.find({ {}, Needle, {} }, Needle, 3) == nil)
assert(table.find({ {}, Needle, Needle }, Needle, 3) == 3)
assert(table.find({ Needle }, Needle, 100) == nil)
assert(not pcall(table.find, { Needle }, Needle, 0))
assert(table.find({ false, 123, "x", function() end, Needle }, Needle) == 5)
assert(table.find({ [1] = {}, [3] = Needle }, Needle) == nil)
assert(table.find({ [1] = {}, [3] = Needle }, Needle, 3) == 3)
local Sparse = table.create(2)
Sparse[1] = {}
Sparse[2] = {}
Sparse[3] = Needle
assert(table.find(Sparse, Needle) == 3)
assert(table.find({}, Needle) == nil)
local EqualMetatable = { __eq = function() return true end }
local Equivalent = setmetatable({}, EqualMetatable)
local WithEquality = setmetatable({}, EqualMetatable)
assert(table.find({ Equivalent }, WithEquality) == 1)
assert(table.find({ Equivalent }, Needle) == nil)
local WithoutEquality = setmetatable({}, { __index = {} })
assert(table.find({ Equivalent, WithoutEquality }, WithoutEquality) == 2)
local FalseEquality = setmetatable({}, { __eq = function() return false end })
assert(table.find({ FalseEquality }, FalseEquality) == nil)
local SharedEquality = function() return true end
assert(table.find({ setmetatable({}, { __eq = SharedEquality }) }, setmetatable({}, { __eq = SharedEquality })) == 1)
assert(table.find({ 1, 2, 3 }, 2) == 2)
assert(table.find({ true, false }, false) == 2)
assert(table.find({ "x", "y" }, "y") == 2)
assert(table.find({ 1, nil, 3 }, nil) == nil)

-- Frozen result from the unmodified pinned Luau 694 VM, including sparse arrays and __eq.
local Values = { {}, {}, setmetatable({}, {}), false, 1, "x" }
local KeyMetatable = { __eq = function(A, B) return A.key == B.key end }
table.insert(Values, setmetatable({ key = 1 }, KeyMetatable))
table.insert(Values, setmetatable({ key = 1 }, KeyMetatable))
table.insert(Values, setmetatable({ key = 2 }, KeyMetatable))
local Hash = 0
for Case = 0, 255 do
	local List = table.create(Case % 17)
	for Index = 1, 80 do
		if (Case + Index * 7) % 11 ~= 0 then
			List[Index] = Values[(Index * 17 + Case) % #Values + 1]
		end
	end
	for _, Value in Values do
		for Start = 1, 83 do
			local Found = table.find(List, Value, Start)
			Hash = (Hash * 31 + (Found or 0)) % 1000000007
		end
	end
end
assert(Hash == 492383283, "table.find differential corpus changed")
print("table.find: focused semantics and 191232 differential cases passed")
