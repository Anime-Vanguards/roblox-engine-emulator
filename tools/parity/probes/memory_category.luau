-- debug's memory-category functions: which exist, what setmemorycategory / resetmemorycategory answer and take, and
-- whether a category set reads back.
local Names = {}
for Name in debug do
	table.insert(Names, Name)
end
table.sort(Names)
print("debug " .. table.concat(Names, ","))
for _, Name in { "setmemorycategory", "resetmemorycategory", "getmemorycategory", "dumpcodesize", "profilebegin", "profileend" } do
	print("type " .. Name .. " " .. type(debug[Name]))
end
-- (a script's own category may hold its chunk name, which differs between engines: written as <chunk>)
local function Plain(Text)
	local Start, Finish = string.find(Text, P.chunk(), 1, true)
	if Start then
		return string.sub(Text, 1, Start - 1) .. "<chunk>" .. string.sub(Text, Finish + 1)
	end
	return Text
end
local function Get()
	if type(debug.getmemorycategory) ~= "function" then
		return "(none)"
	end
	return Plain(P.try(debug.getmemorycategory))
end
local function Set(...)
	return Plain(P.try(debug.setmemorycategory, ...))
end
print("get-start " .. Get())
print("set " .. Set("abc"))
print("get-after-set " .. Get())
print("set-again " .. Set("def"))
print("reset " .. Plain(P.try(debug.resetmemorycategory)))
print("get-after-reset " .. Get())
local Nothing = newproxy()
for _, Case in {
	{ "none", Nothing },
	{ "nil", nil },
	{ "number", 5 },
	{ "table", {} },
	{ "true", true },
	{ "empty", "" },
	{ "long", string.rep("x", 300) },
	{ "spaces", "a b" },
	{ "vector", Vector3.new() },
} do
	local Result
	if Case[2] == Nothing then
		Result = Set()
	else
		Result = Set(Case[2])
	end
	print("set-" .. Case[1] .. " " .. Result .. " get " .. Get())
end
print("reset-args " .. P.try(debug.resetmemorycategory, "x", 1))
-- a thread's category in another thread
debug.setmemorycategory("outer")
task.spawn(function()
	print("thread-get " .. Get())
	debug.setmemorycategory("inner")
end)
print("after-thread " .. Get())
debug.resetmemorycategory()
