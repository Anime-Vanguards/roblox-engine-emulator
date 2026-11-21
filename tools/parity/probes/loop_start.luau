-- A numeric for loop's first index is its start as written, signed zero included, whether the compiler unrolls the loop (a
-- constant start and a short trip count) or the loop runs as written; 1 / i shows the zero's sign. The engine compiles at -O2,
-- where loops this short are unrolled; the cases with a start made at run time never are.
local function Line(Name, Start, Finish, Step)
	local Seen = {}
	for i = Start, Finish, Step do
		table.insert(Seen, tostring(1 / i))
	end
	print(Name .. " " .. table.concat(Seen, " "))
end

local Seen = {}
for i = -0, 1 do
	table.insert(Seen, tostring(1 / i))
end
print("constant " .. table.concat(Seen, " "))

Seen = {}
local Start = -0
for i = Start, 1 do
	table.insert(Seen, tostring(1 / i))
end
print("constant-local " .. table.concat(Seen, " "))

Seen = {}
for i = -0, -0 do
	table.insert(Seen, tostring(1 / i))
end
print("constant-once " .. table.concat(Seen, " "))

Seen = {}
for i = -0, -2, -1 do
	table.insert(Seen, tostring(1 / i))
end
print("constant-down " .. table.concat(Seen, " "))

Line("run-time", tonumber("-0"), 1, 1)
Line("run-time-down", tonumber("-0"), -2, -1)
