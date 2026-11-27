-- LocalizationService:GetTranslatorForLocaleAsync: its locale's cast, the errors at once, the refusal where the thread cannot
-- yield, and the wait (the experience's table, which an offline client never gets) before the translator comes
P.sync()
local L = game:GetService("LocalizationService")
local function Sorted(Label, F)
	local Line
	table.sort({ 2, 1 }, function(A, B)
		if not Line then
			Line = P.try(F)
		end
		return A < B
	end)
	print("refused " .. Label .. " " .. Line)
end
Sorted("x", function() return L:GetTranslatorForLocaleAsync("x") end)
Sorted("5", function() return L:GetTranslatorForLocaleAsync(5) end)
Sorted("empty", function() return L:GetTranslatorForLocaleAsync("") end)
Sorted("other self", function() return L.GetTranslatorForLocaleAsync(workspace, "fr-fr") end)
Sorted("vector", function() return L:GetTranslatorForLocaleAsync(Vector3.one) end)
print("other self", P.try(function() return L.GetTranslatorForLocaleAsync(workspace, "fr-fr") end))
print("no self", P.try(function() return L.GetTranslatorForLocaleAsync() end))
-- casts, and calls made together
local Frames = 0
local Counter = game:GetService("RunService").Heartbeat:Connect(function()
	Frames += 1
end)
local T0 = time()
local Results = {}
local Cases = {
	{ "fr-fr", "fr-fr" },
	{ "table", {} },
	{ "true", true },
	{ "instance", workspace },
	{ "number", 5 },
	{ "x", "x" },
	{ "EN_us", "EN_us" },
	{ "empty", "" },
	{ "vector", Vector3.one },
}
for Index, Case in Cases do
	task.spawn(function()
		local At = Frames
		local Ok, R = pcall(function()
			return L:GetTranslatorForLocaleAsync(Case[2])
		end)
		local Text
		if Ok then
			Text = "ok " .. typeof(R) .. " " .. (if typeof(R) == "Instance" then R.ClassName .. " " .. R.Name .. " " .. R.LocaleId else tostring(R))
		else
			Text = "error " .. P.err(R)
		end
		Results[Index] = Text .. " | " .. (if Frames == At then "at once" else string.format("after %.1f s", time() - T0))
	end)
end
while #Results < #Cases or Results[#Cases] == nil do
	local All = true
	for Index = 1, #Cases do
		if not Results[Index] then
			All = false
		end
	end
	if All or time() - T0 > 40 then
		break
	end
	task.wait(0.25)
end
for Index, Case in Cases do
	print("locale " .. Case[1], Results[Index] or "pending")
end
-- one after the others answered
local Before = time()
local Ok, R = pcall(function()
	return L:GetTranslatorForLocaleAsync("de-de")
end)
Counter:Disconnect()
print("again", Ok, if Ok then R.LocaleId else P.err(R), string.format("%.1f s", time() - Before), Frames > 300)
