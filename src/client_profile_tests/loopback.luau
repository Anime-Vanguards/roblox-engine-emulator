-- A loopback pair's two sides read the client profile's player: the server's Player for it, and the client's own
local Players = game:GetService("Players")
return {
	Server = function(Context)
		local P = Context.Player
		print("server", P.Name, P.DisplayName, P.UserId, P.AccountAge, P.MembershipType, P.LocaleId)
		print("server owns", game:GetService("MarketplaceService"):UserOwnsGamePassAsync(P.UserId, 11), game:GetService("LocalizationService"):GetCountryRegionForPlayerAsync(P))
		print("server data", game:GetService("DataStoreService"):GetDataStore("PlayerData"):GetAsync("Player_5").Coins)
	end,
	Client = function(Context)
		local P = Players.LocalPlayer
		print("client", P.Name, P.DisplayName, P.UserId, P.AccountAge, P.MembershipType, P.LocaleId, game:GetService("UserInputService").TouchEnabled)
		task.wait(0.1)
		print("@done")
	end,
}
