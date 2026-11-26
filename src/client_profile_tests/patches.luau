-- A harness's patches: a null that would remove a field set at the start, ownership removed, empty tables
return function(Context)
	local P = game:GetService("Players").LocalPlayer
	local MS = game:GetService("MarketplaceService")
	local Http = game:GetService("HttpService")
	print("null", pcall(Context.SetClientProfile, '{"localization": null}'))
	print("owned", MS:UserOwnsGamePassAsync(P.UserId, 11), MS:PlayerOwnsAssetAsync(P, 3))
	Context.SetClientProfile({ ownership = Context.Null })
	print("removed", MS:UserOwnsGamePassAsync(P.UserId, 11), (MS:PlayerOwnsAssetAsync(P, 3)))
	local Ok = pcall(Context.SetClientProfile, { policy = {}, social = { blocked = {} } })
	print("empty", Ok, Http:JSONEncode(game:GetService("StarterGui"):GetCore("GetBlockedUserIds")), Http:JSONEncode(Context.ClientProfile().policy))
	Ok = pcall(Context.SetClientProfile, { policy = { AllowedExternalLinkReferences = {} }, ownership = { badges = {} } })
	local Links = game:GetService("PolicyService"):GetPolicyInfoForPlayerAsync(P).AllowedExternalLinkReferences
	print("listed", Ok, #Links, Http:JSONEncode(game:GetService("BadgeService"):CheckUserBadgesAsync(P.UserId, { 1 })))
	print("@done")
end
