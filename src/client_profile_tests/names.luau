-- Both sides of a loopback pair look the profile's player (UserId 5) up by its id
local function Names()
	local Players = game:GetService("Players")
	local Info = game:GetService("UserService"):GetUserInfosByUserIdsAsync({ 5 })[1] or {}
	print("names", Players:GetNameFromUserIdAsync(5), Info.Username, Info.DisplayName, Info.HasVerifiedBadge, Info.Id)
end
return {
	Server = function()
		Names()
	end,
	Client = function()
		Names()
		task.wait(0.1)
		print("@done")
	end,
}
