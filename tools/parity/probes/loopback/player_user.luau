-- parity: loopback
-- Player.User on each side (its type, text and Id, and whether it equals itself read again), and on the server the
-- other members a game script reads that rbx-dom lacks (AgeChecked, DeviceTrustLevel, FrustumStreaming, HasRobloxSubscription)
local Players = game:GetService("Players")
local function describe(Tag, Player)
	print(Tag, "UserId", P.num(Player.UserId), P.try(function()
		local User = (Player :: any).User
		return typeof(User), tostring(User), P.num(User.Id)
	end))
end
local function equal(Tag, Player)
	print(Tag, "equal", P.try(function() return (Player :: any).User == (Player :: any).User end), P.try(function() return rawequal((Player :: any).User, (Player :: any).User) end))
end
local function members(Tag, Player)
	for _, Name in { "AgeChecked", "DeviceTrustLevel", "FrustumStreaming", "HasRobloxSubscription" } do
		print(Tag, Name, P.try(function() return tostring((Player :: any)[Name]) end), P.try(function() (Player :: any)[Name] = {} end))
	end
	print(Tag, "write", P.try(function() (Player :: any).AgeChecked = Enum.AgeCheckStatus.Checked end), P.try(function() (Player :: any).DeviceTrustLevel = Enum.PlayerDeviceTrustLevel.Full end), P.try(function() (Player :: any).HasRobloxSubscription = true end))
	print(Tag, "user write", P.try(function() (Player :: any).User = (Player :: any).User end))
	print(Tag, "frustum", P.try(function() (Player :: any).FrustumStreaming = Enum.FrustumStreamingMode.Enabled end), P.try(function() return tostring((Player :: any).FrustumStreaming) end), P.try(function() return Player:IsPropertyModified("FrustumStreaming") end), P.try(function() Player:ResetPropertyToDefault("FrustumStreaming") end))
	pcall(function() (Player :: any).FrustumStreaming = Enum.FrustumStreamingMode.Default end)
end
--- server
local Me = P.player()
describe("server", Me)
equal("server", Me)
members("server", Me)
P.meet("joined")
--- client
P.meet("joined")
local Me = P.player()
describe("client", Me)
equal("client", Me)
