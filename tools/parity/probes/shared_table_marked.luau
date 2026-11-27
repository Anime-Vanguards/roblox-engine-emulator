-- The order of keys whose 64-bit hashes are equal in a SharedTable a client has marked (stored in another SharedTable, held by
-- SharedTableRegistry, set as an attribute, sent through a BindableFunction): every overwrite or increment moves its key first,
-- as in a list a shallow copy shares; a plain table's stays in place. Each step on a fresh table, the order read once at the end
-- (a loop holds the entries it walked, as a copy does).
local Registry = game:GetService("SharedTableRegistry")
local A = { "amacobmcimlpggfd", "gajldeibogfilcdn", "oaomcbcfamgpdfnp" }
local B = { "jadnfnainmlpjeik", "picgfcflgnjiaddi", "okpppjoaaachdced" }
local function colliding(i)
	local s = ""
	for j = 0, 2 do
		s ..= if bit32.extract(i, j) == 1 then B[j + 1] else A[j + 1]
	end
	return s
end
local function order(S)
	local index = {}
	for i = 0, 7 do
		index[colliding(i)] = i
	end
	local out = {}
	for k in S do
		table.insert(out, index[k] or "?")
	end
	return table.concat(out, ",")
end
local STEPS = {
	{ "write 3", function(S) S[colliding(3)] = 33 end },
	{ "increment 5", function(S) SharedTable.increment(S, colliding(5), 1) end },
	{ "remove 1", function(S) S[colliding(1)] = nil end },
	{ "add 1", function(S) S[colliding(1)] = 1 end },
	{ "update 6", function(S) SharedTable.update(S, colliding(6), function(v) return v end) end },
	{ "write 0", function(S) S[colliding(0)] = 0 end },
}
local Holder = Instance.new("Folder")
for _, case in { "plain", "registry", "nested", "attribute", "bindable", "deep copy source", "deep copy" } do
	for k = 0, #STEPS do
		local S = SharedTable.new()
		for i = 0, 7 do
			S[colliding(i)] = i
		end
		local keep
		if case == "registry" then
			Registry:SetSharedTable("shared_table_marked" .. k, S)
		elseif case == "nested" then
			keep = SharedTable.new()
			keep.inner = S
		elseif case == "attribute" then
			pcall(function() Holder:SetAttribute("Marked", S) end)
		elseif case == "bindable" then
			local Bindable = Instance.new("BindableFunction")
			Bindable.OnInvoke = function() end
			Bindable:Invoke(S)
		elseif case == "deep copy source" then
			keep = SharedTable.clone(S, true)
		elseif case == "deep copy" then
			S = SharedTable.clone(S, true)
		end
		for j = 1, k do
			STEPS[j][2](S)
		end
		print(case, if k == 0 then "made" else STEPS[k][1], order(S))
		Registry:SetSharedTable("shared_table_marked" .. k, nil)
	end
end
