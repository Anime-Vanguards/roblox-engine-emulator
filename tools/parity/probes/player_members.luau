-- Player members a game script reads that rbx-dom lacks: AgeChecked, DeviceTrustLevel, FrustumStreaming, HasRobloxSubscription and
-- User, each read, written with a value of its type and with others, and given to the property-name methods (a client's own
-- player; a server's in loopback/player_user.luau)
P.sync()
local Players = game:GetService("Players")
local Player = Players.LocalPlayer
print("player", Player ~= nil)
local function show(Value)
	local Kind = typeof(Value)
	if Kind == "Instance" then
		return "Instance " .. Value.ClassName
	end
	return Kind .. " " .. tostring(Value)
end
local Fired = {}
local Changed = Player.Changed:Connect(function(Name)
	table.insert(Fired, Name)
end)
local Writes = {
	AgeChecked = Enum.AgeCheckStatus.Checked,
	DeviceTrustLevel = Enum.PlayerDeviceTrustLevel.Full,
	FrustumStreaming = Enum.FrustumStreamingMode.Enabled,
	HasRobloxSubscription = true,
}
for _, Name in { "AgeChecked", "DeviceTrustLevel", "FrustumStreaming", "HasRobloxSubscription", "User" } do
	print("read", Name, P.try(function() return show((Player :: any)[Name]) end))
	print("lower", Name, P.try(function() return show((Player :: any)[string.lower(string.sub(Name, 1, 1)) .. string.sub(Name, 2)]) end))
	print("signal", Name, P.try(function() return typeof(Player:GetPropertyChangedSignal(Name)) end))
	local Value = Writes[Name]
	if Value ~= nil then
		print("write", Name, P.try(function() (Player :: any)[Name] = Value end), P.try(function() return show((Player :: any)[Name]) end))
	end
	print("write table", Name, P.try(function() (Player :: any)[Name] = {} end))
	print("write string", Name, P.try(function() (Player :: any)[Name] = "x" end))
	print("modified", Name, P.try(function() return Player:IsPropertyModified(Name) end))
	print("reset", Name, P.try(function() return Player:ResetPropertyToDefault(Name) end))
end
local Frustum = Player.FrustumStreaming
print("frustum items", P.try(function()
	local Out = {}
	for _, Item in { Enum.FrustumStreamingMode.Disabled, Enum.FrustumStreamingMode.Automatic, Enum.FrustumStreamingMode.Default, "Enabled", 2, 7 } do
		local Ok, Error = pcall(function() (Player :: any).FrustumStreaming = Item end)
		table.insert(Out, if Ok then tostring(Player.FrustumStreaming) else P.err(Error))
	end
	return table.concat(Out, " | ")
end))
pcall(function() (Player :: any).FrustumStreaming = Frustum end)
print("user members", P.try(function()
	local User = (Player :: any).User
	local Out = {}
	for _, Name in { "UserId", "Name", "DisplayName", "Id", "ClassName" } do
		local Ok, Value = pcall(function() return User[Name] end)
		table.insert(Out, Name .. "=" .. (if Ok then show(Value) else P.err(Value)))
	end
	return table.concat(Out, " ")
end))
print("user equal", P.try(function() return (Player :: any).User == (Player :: any).User end))
task.wait()
Changed:Disconnect()
print("changed", table.concat(Fired, " "))
