-- A harness reads the client profile and changes what is read at call time
return function(Context)
	local P = game:GetService("Players").LocalPlayer
	local MS = game:GetService("MarketplaceService")
	local Http = game:GetService("HttpService")
	print("read", Http:JSONEncode(Context.ClientProfile()))
	print("before", MS:UserOwnsGamePassAsync(P.UserId, 11), game:GetService("PolicyService"):GetPolicyInfoForPlayerAsync(P).AreAdsAllowed)
	Context.SetClientProfile({ ownership = { gamePasses = { 11 }, assets = "all" }, policy = { AreAdsAllowed = false }, localization = { countryRegion = "JP" } })
	print("after", MS:UserOwnsGamePassAsync(P.UserId, 11), game:GetService("PolicyService"):GetPolicyInfoForPlayerAsync(P).AreAdsAllowed, MS:PlayerOwnsAssetAsync(P, 3), game:GetService("LocalizationService"):GetCountryRegionForPlayerAsync(P))
	Context.SetClientProfile('{"ownership": {"gamePasses": null}}')
	print("json", MS:UserOwnsGamePassAsync(P.UserId, 11), Http:JSONEncode(Context.ClientProfile().ownership))
	print("refused", pcall(Context.SetClientProfile, { identity = { name = "X" } }))
	print("bad", pcall(Context.SetClientProfile, { ownership = { gamePasses = "some" } }))
	print("@done")
end
