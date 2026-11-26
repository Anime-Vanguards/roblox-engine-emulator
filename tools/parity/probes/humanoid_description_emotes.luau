-- HumanoidDescription's emotes: SetEmotes, AddEmote, RemoveEmote, GetEmotes, SetEquippedEmotes, GetEquippedEmotes, their casts,
-- what each keeps and the events they fire
P.sync()
local Description = Instance.new("HumanoidDescription")
local Events = {}
Description.EmotesChanged:Connect(function(Value)
	table.insert(Events, "EmotesChanged(" .. typeof(Value) .. ")")
end)
Description.EquippedEmotesChanged:Connect(function(Value)
	table.insert(Events, "EquippedEmotesChanged(" .. typeof(Value) .. ")")
end)
local function Show(Value, Depth)
	Depth = Depth or 0
	if type(Value) ~= "table" then
		return typeof(Value) .. ":" .. tostring(Value)
	end
	local Keys = {}
	for Key in Value do
		table.insert(Keys, Key)
	end
	table.sort(Keys, function(X, Y)
		return tostring(X) < tostring(Y)
	end)
	local Parts = {}
	for _, Key in Keys do
		table.insert(Parts, tostring(Key) .. "=" .. (if Depth < 3 then Show(Value[Key], Depth + 1) else "..."))
	end
	return "{" .. table.concat(Parts, ",") .. "}"
end
local function Step(Label, Call)
	table.clear(Events)
	local Results = table.pack(pcall(Call))
	task.wait()
	local Text
	if Results[1] then
		local Parts = {}
		for Index = 2, Results.n do
			table.insert(Parts, Show(Results[Index]))
		end
		Text = "ok " .. table.concat(Parts, " ")
	else
		Text = "error " .. P.err(Results[2])
	end
	print("emotes " .. Label .. " " .. Text .. " [" .. table.concat(Events, ",") .. "]")
end
Step("get-start", function() return Description:GetEmotes() end)
Step("get-equipped-start", function() return Description:GetEquippedEmotes() end)
Step("emotes-property", function() return Description.EmotesDataInternal, Description.EquippedEmotesDataInternal end)
Step("set", function() return Description:SetEmotes({ Salute = { 3360689775 } }) end)
Step("get", function() return Description:GetEmotes() end)
Step("get-twice-same", function() return rawequal((Description:GetEmotes()), (Description:GetEmotes())) end)
Step("set-same", function() return Description:SetEmotes({ Salute = { 3360689775 } }) end)
Step("set-two", function() return Description:SetEmotes({ Salute = { 3360689775, 3360692915 }, Wave = { 3576686446 } }) end)
Step("get-two", function() return Description:GetEmotes() end)
Step("set-equipped", function() return Description:SetEquippedEmotes({ "Salute" }) end)
Step("get-equipped", function() return Description:GetEquippedEmotes() end)
Step("set-equipped-tables", function() return Description:SetEquippedEmotes({ { Slot = 3, Name = "Wave" } }) end)
Step("get-equipped-tables", function() return Description:GetEquippedEmotes() end)
Step("set-equipped-unknown", function() return Description:SetEquippedEmotes({ "Nope" }) end)
Step("get-equipped-unknown", function() return Description:GetEquippedEmotes() end)
Step("set-equipped-many", function() return Description:SetEquippedEmotes({ "Salute", "Wave", "Salute", "Wave", "Salute", "Wave", "Salute", "Wave", "Salute" }) end)
Step("get-equipped-many", function() return Description:GetEquippedEmotes() end)
Step("add", function() return Description:AddEmote("Dance", 3576686446) end)
Step("add-again", function() return Description:AddEmote("Dance", 3360692915) end)
Step("get-after-add", function() return Description:GetEmotes() end)
Step("remove", function() return Description:RemoveEmote("Wave") end)
Step("remove-missing", function() return Description:RemoveEmote("Nope") end)
Step("get-after-remove", function() return Description:GetEmotes() end)
Step("get-equipped-after-remove", function() return Description:GetEquippedEmotes() end)
Step("set-nil", function() return Description:SetEmotes(nil) end)
Step("set-string", function() return Description:SetEmotes("x") end)
Step("set-bad-ids", function() return Description:SetEmotes({ Salute = "x" }) end)
Step("set-bad-id", function() return Description:SetEmotes({ Salute = { "x" } }) end)
Step("set-number-key", function() return Description:SetEmotes({ [1] = { 5 } }) end)
Step("set-empty", function() return Description:SetEmotes({}) end)
Step("get-empty", function() return Description:GetEmotes() end)
Step("get-equipped-after-empty", function() return Description:GetEquippedEmotes() end)
Step("set-equipped-nil", function() return Description:SetEquippedEmotes(nil) end)
Step("set-equipped-string", function() return Description:SetEquippedEmotes("x") end)
Step("set-equipped-number", function() return Description:SetEquippedEmotes({ 5 }) end)
Step("set-equipped-bad-slot", function() return Description:SetEquippedEmotes({ { Slot = 9, Name = "Salute" } }) end)
Step("set-equipped-empty", function() return Description:SetEquippedEmotes({}) end)
Step("add-nil", function() return Description:AddEmote(nil, 5) end)
Step("add-noid", function() return Description:AddEmote("Dance") end)
Step("add-string-id", function() return Description:AddEmote("Dance", "x") end)
Step("remove-nil", function() return Description:RemoveEmote(nil) end)
Step("emotes-property-end", function() return Description.EmotesDataInternal, Description.EquippedEmotesDataInternal end)
Description:Destroy()
do
local Description = Instance.new("HumanoidDescription")
local function Show(Value, Depth)
	Depth = Depth or 0
	if type(Value) ~= "table" then
		return typeof(Value) .. ":" .. tostring(Value)
	end
	local Keys = {}
	for Key in Value do
		table.insert(Keys, Key)
	end
	table.sort(Keys, function(X, Y)
		return tostring(X) < tostring(Y)
	end)
	local Parts = {}
	for _, Key in Keys do
		table.insert(Parts, tostring(Key) .. "=" .. (if Depth < 3 then Show(Value[Key], Depth + 1) else "..."))
	end
	return "{" .. table.concat(Parts, ",") .. "}"
end
local Fired = {}
Description.EmotesChanged:Connect(function(Value)
	table.insert(Fired, "E" .. Show(Value))
end)
Description.EquippedEmotesChanged:Connect(function(Value)
	table.insert(Fired, "Q" .. Show(Value))
end)
local function Step(Label, Call, Read)
	table.clear(Fired)
	local Result = P.try(Call)
	task.wait()
	print("emotes " .. Label .. " " .. Result .. " | " .. Show(Description:GetEmotes()) .. " | " .. Show(Description:GetEquippedEmotes()) .. " | " .. table.concat(Fired, " "))
end
Step("add-order", function()
	Description:AddEmote("X", 2)
	Description:AddEmote("X", 1)
	Description:AddEmote("X", 3)
end)
Step("add-dupe", function() return Description:AddEmote("X", 2) end)
Step("add-float", function() return Description:AddEmote("Y", 1.5) end)
Step("add-numstring", function() return Description:AddEmote("Z", "12") end)
Step("add-neg", function() return Description:AddEmote("N", -4) end)
Step("add-number-name", function() return Description:AddEmote(5, 6) end)
Step("set-order", function() return Description:SetEmotes({ X = { 3, 1, 2 } }) end)
Step("set-strings", function() return Description:SetEmotes({ S = "x" }) end)
Step("set-string-ids", function() return Description:SetEmotes({ S = { "x", "7" } }) end)
Step("set-floats", function() return Description:SetEmotes({ F = { 1.5, -2.5 } }) end)
Step("set-dict-ids", function() return Description:SetEmotes({ D = { a = 1 } }) end)
Step("set-holes", function() return Description:SetEmotes({ H = { 1, nil, 3 } }) end)
Step("set-nested", function() return Description:SetEmotes({ T = { { 1 } } }) end)
Step("set-bool", function() return Description:SetEmotes({ B = true }) end)
Step("set-mixed-keys", function() return Description:SetEmotes({ [1] = { 1 }, K = { 2 } }) end)
Step("set-empty-list", function() return Description:SetEmotes({ E = {} }) end)
Step("set-instance", function() return Description:SetEmotes(workspace) end)
Step("equip-number", function() return Description:SetEquippedEmotes({ 5 }) end)
Step("equip-bad-slot", function() return Description:SetEquippedEmotes({ { Slot = 9, Name = "S" } }) end)
Step("equip-slot-string", function() return Description:SetEquippedEmotes({ { Slot = "2", Name = "S" } }) end)
Step("equip-slot-float", function() return Description:SetEquippedEmotes({ { Slot = 2.5, Name = "S" } }) end)
Step("equip-no-slot", function() return Description:SetEquippedEmotes({ { Name = "S" } }) end)
Step("equip-no-name", function() return Description:SetEquippedEmotes({ { Slot = 2 } }) end)
Step("equip-name-number", function() return Description:SetEquippedEmotes({ { Slot = 2, Name = 7 } }) end)
Step("equip-same-slot", function() return Description:SetEquippedEmotes({ { Slot = 2, Name = "A" }, { Slot = 2, Name = "B" } }) end)
Step("equip-mixed", function() return Description:SetEquippedEmotes({ "A", { Slot = 5, Name = "B" }, "C" }) end)
Step("equip-holes", function() return Description:SetEquippedEmotes({ "A", nil, "C" }) end)
Step("equip-dict", function() return Description:SetEquippedEmotes({ a = "A" }) end)
Step("equip-bool", function() return Description:SetEquippedEmotes({ true }) end)
Step("equip-zero-slot", function() return Description:SetEquippedEmotes({ { Slot = 0, Name = "Z" } }) end)
Step("equip-instance", function() return Description:SetEquippedEmotes(workspace) end)
Step("remove-number", function() return Description:RemoveEmote(5) end)
Step("event-table-copy", function()
	local Seen
	local Connection = Description.EmotesChanged:Connect(function(Value)
		Seen = Value
	end)
	local Given = { W = { 1 } }
	Description:SetEmotes(Given)
	task.wait()
	Connection:Disconnect()
	return rawequal(Seen, Given), rawequal(Seen, Description:GetEmotes())
end)
local Clone = Description:Clone()
print("emotes clone " .. Show(Clone:GetEmotes()) .. " | " .. Show(Clone:GetEquippedEmotes()))
Description:Destroy()
end
do
-- emotes: names and ids of other types
local Description = Instance.new("HumanoidDescription")
local function Keys(T)
	local Out = {}
	for Key, Value in T do
		table.insert(Out, typeof(Key) .. ":" .. tostring(Key) .. "=" .. (if type(Value) == "table" then table.concat(Value, ",") else tostring(Value)))
	end
	table.sort(Out)
	return table.concat(Out, " ")
end
Description:AddEmote(5, 6)
print("emotes number-name " .. Keys(Description:GetEmotes()))
print("emotes set-number-key " .. P.try(Description.SetEmotes, Description, { [5] = { 1 } }) .. " " .. Keys(Description:GetEmotes()))
print("emotes set-mixed-ids " .. P.try(Description.SetEmotes, Description, { M = { 3, "x", 1, 1 } }) .. " " .. Keys(Description:GetEmotes()))
print("emotes add-to-mixed " .. P.try(Description.AddEmote, Description, "M", 2.5) .. " " .. Keys(Description:GetEmotes()))
print("emotes equip-collide " .. P.try(Description.SetEquippedEmotes, Description, { { Slot = 1, Name = "T" }, "S" }) .. " " .. Keys(Description:GetEquippedEmotes()[1] or {}) .. " / " .. Keys(Description:GetEquippedEmotes()[2] or {}))
print("emotes equip-collide-2 " .. P.try(Description.SetEquippedEmotes, Description, { "S", { Slot = 1, Name = "T" } }) .. " " .. Keys(Description:GetEquippedEmotes()[1] or {}) .. " / " .. Keys(Description:GetEquippedEmotes()[2] or {}))
print("emotes equip-order " .. P.try(Description.SetEquippedEmotes, Description, { { Slot = 5, Name = "A" }, { Slot = 2, Name = "B" } }) .. " " .. Keys(Description:GetEquippedEmotes()[1] or {}) .. " / " .. Keys(Description:GetEquippedEmotes()[2] or {}))
Description:Destroy()
end
