-- A server's answers for the client profile's player (UserId 5) and another (6), and the stores' starting contents
return function()
	local Players = game:GetService("Players")
	local function Join(Id)
		local Player = Instance.new("Player")
		Player.UserId = Id
		Player.Parent = Players
		return Player
	end
	local Mine, Other = Join(5), Join(6)
	local MS = game:GetService("MarketplaceService")
	print("owns", MS:UserOwnsGamePassAsync(5, 11), MS:UserOwnsGamePassAsync(6, 11))
	local PS = game:GetService("PolicyService")
	print("ads", PS:GetPolicyInfoForPlayerAsync(Mine).AreAdsAllowed, PS:GetPolicyInfoForPlayerAsync(Other).AreAdsAllowed)
	print("rank", Mine:GetRankInGroup(7), Other:GetRankInGroup(7), Mine:GetRoleInGroup(7))
	local LS = game:GetService("LocalizationService")
	print("country", LS:GetCountryRegionForPlayerAsync(Mine), LS:GetCountryRegionForPlayerAsync(Other))
	local DSS = game:GetService("DataStoreService")
	local Saved = DSS:GetDataStore("PlayerData"):GetAsync("Player_5")
	print("saved", Saved.Coins, #Saved.Units, DSS:GetDataStore("PlayerData", "beta"):GetAsync("k"), DSS:GetDataStore("Other"):GetAsync("Player_5"))
	local Page = DSS:GetOrderedDataStore("Wins"):GetSortedAsync(false, 10):GetCurrentPage()
	print("wins", Page[1].key, Page[1].value, Page[2].key, Page[2].value)
	print("global", DSS:GetGlobalDataStore():GetAsync("Motd"))
	local MSS = game:GetService("MemoryStoreService")
	print("lobby", MSS:GetHashMap("Lobby"):GetAsync("open"))
	local Ranks = MSS:GetSortedMap("Ranks"):GetRangeAsync(Enum.SortDirection.Ascending, 10)
	print("ranks", Ranks[1].key, Ranks[1].value, Ranks[1].sortKey, Ranks[2].key)
	print("jobs", table.concat(MSS:GetQueue("Jobs"):ReadAsync(2, false, 0), ","))
	print("@done")
end
