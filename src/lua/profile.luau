--!nocheck
-- Rust engine: the client profile's answers in the service stubs (client_profile.rs). Loaded after services.luau and
-- server.luau (lua/stubs.rs) only in a run whose profile sets a field read here (ClientProfile::read_by_luau), or once a
-- harness's Context.SetClientProfile gives it one, so a run without one keeps every stub as it was. Each member here wraps
-- the stub it replaces (Original): it answers from R.Profile for the profile's player and leaves everything else (another
-- player, an unset field, the argument checks and yields) to the stub.
--
-- R.Profile is the profile's set fields in its JSON format, its player's UserId as the engine gives it (set or not), and where
-- the profile sets its identity its Name, DisplayName and HasVerifiedBadge. It is read at each call: Context.SetClientProfile
-- replaces it.

local R, Original = ...
local Method = R.Method
local Getter = R.Getter
local RawGet = R.RawGet

-- The profile when UserId is its player's
local function ProfileOfUser(UserId)
	local Profile = R.Profile
	if Profile and tonumber(UserId) == Profile.UserId then
		return Profile
	end
	return nil
end

-- The profile when Player is its player
local function ProfileOf(Player)
	if R.Profile and typeof(Player) == "Instance" and Player:IsA("Player") then
		return ProfileOfUser(RawGet(Player, "UserId"))
	end
	return nil
end

-- A section's field of the profile, or nil
local function Field(Profile, Section, Name)
	local Found = Profile and Profile[Section]
	return if Found then Found[Name] else nil
end

-- Whether a list of ids (or "all") holds Id
local function Holds(List, Id)
	if List == "all" then
		return true
	end
	for _, Item in List do
		if Item == Id or Item == tonumber(Id) then
			return true
		end
	end
	return false
end

-- The profile's friend whose UserId is UserId
local function Friend(UserId)
	for _, Found in Field(R.Profile, "social", "friends") or {} do
		if Found.userId == tonumber(UserId) then
			return Found
		end
	end
	return nil
end

-- `Class`.`Name` answered by Answer(Stub, ...), Stub being the member it replaces
local function Wrap(Class, Name, Answer)
	local Stub = Original(Class, Name)
	Method(Class, Name, function(...)
		return Answer(Stub, ...)
	end)
end

-- A list as pages (Players:GetFriendsAsync's FriendPages): Size items a page
local function ListPages(Items, Size)
	local Page = 1
	local Pages
	Pages = {
		IsFinished = #Items <= Size,
		GetCurrentPage = function()
			return table.move(Items, (Page - 1) * Size + 1, math.min(Page * Size, #Items), 1, {})
		end,
		AdvanceToNextPageAsync = function()
			if not Pages.IsFinished then
				Page += 1
				Pages.IsFinished = Page * Size >= #Items
			end
		end,
	}
	return Pages
end

-- Identity: the player's verified state, and its name and its friends' (the profile's player by its UserId, on a server as on
-- its client, where the local player is the same)
Wrap("Player", "IsVerified", function(Stub, Self, ...)
	local Verified = Field(ProfileOf(Self), "identity", "verified")
	if Verified ~= nil then
		return Verified
	end
	return Stub(Self, ...)
end)
Wrap("Players", "GetNameFromUserIdAsync", function(Stub, Self, UserId, ...)
	local Profile = ProfileOfUser(UserId)
	if Profile and Profile.Name then
		return Profile.Name
	end
	local Found = Friend(UserId)
	if Found then
		return Found.name
	end
	return Stub(Self, UserId, ...)
end)
Wrap("UserService", "GetUserInfosByUserIdsAsync", function(Stub, Self, UserIds, ...)
	local Result = {}
	for _, UserId in UserIds or {} do
		local Profile = ProfileOfUser(UserId)
		local Found = Friend(UserId)
		if Profile and Profile.Name then
			table.insert(Result, { Id = UserId, Username = Profile.Name, DisplayName = Profile.DisplayName, HasVerifiedBadge = Profile.HasVerifiedBadge })
		elseif Found then
			table.insert(Result, { Id = UserId, Username = Found.name, DisplayName = Found.displayName or Found.name, HasVerifiedBadge = false })
		else
			for _, Info in Stub(Self, { UserId }, ...) do
				table.insert(Result, Info)
			end
		end
	end
	return Result
end)

-- Device: the network ping and VR (UserInputService's inputs are Rust: services/mod.rs)
Wrap("Player", "GetNetworkPing", function(Stub, Self, ...)
	local Ping = Field(ProfileOf(Self), "device", "networkPing")
	if Ping ~= nil then
		return Ping
	end
	return Stub(Self, ...)
end)
Getter("VRService", "VREnabled", function()
	local Inputs = Field(R.Profile, "device", "inputs")
	return Inputs ~= nil and Inputs.vr == true
end)

-- Settings: the camera's vertical multiplier, -1 for a player who inverts it
Wrap("UserGameSettings", "GetCameraYInvertValue", function(Stub, ...)
	if Field(R.Profile, "settings", "cameraYInverted") ~= nil then
		return if Field(R.Profile, "settings", "cameraYInverted") then -1 else 1
	end
	return Stub(...)
end)

-- Policy: the profile's keys over the stub's table (the stub's for any other player)
Wrap("PolicyService", "GetPolicyInfoForPlayerAsync", function(Stub, Self, Player, ...)
	local Answer = Stub(Self, Player, ...)
	local Profile = ProfileOf(Player)
	for Key, Value in if Profile and Profile.policy then Profile.policy else {} do
		Answer[Key] = if type(Value) == "table" then table.clone(Value) else Value
	end
	return Answer
end)

-- Ownership: the profile's lists (ids, or "all") for its player
local function Owned(Kind, Profile, Id)
	local List = Field(Profile, "ownership", Kind)
	if List == nil then
		return nil
	end
	return Holds(List, Id)
end
Wrap("MarketplaceService", "UserOwnsGamePassAsync", function(Stub, Self, UserId, GamePassId, ...)
	local Owns = Owned("gamePasses", ProfileOfUser(UserId), GamePassId)
	if Owns ~= nil then
		return Owns
	end
	return Stub(Self, UserId, GamePassId, ...)
end)
for Name, Kind in { PlayerOwnsAsset = "assets", PlayerOwnsBundle = "bundles" } do
	Wrap("MarketplaceService", Name, function(Stub, Self, Player, Id, ...)
		local Owns = Owned(Kind, ProfileOf(Player), Id)
		if Owns ~= nil then
			return Owns
		end
		return Stub(Self, Player, Id, ...)
	end)
end
Wrap("BadgeService", "UserHasBadgeAsync", function(Stub, Self, UserId, BadgeId, ...)
	local Owns = Owned("badges", ProfileOfUser(UserId), BadgeId)
	if Owns ~= nil then
		return Owns
	end
	return Stub(Self, UserId, BadgeId, ...)
end)

-- Members the stubs leave unemulated that answer once the profile gives what they read: each is added the first time the
-- profile sets its field (now, or after Context.SetClientProfile: R.ProfileMethods). Without its field (before, or after a
-- patch removed it) a call reports itself and returns nothing, as with no profile.
local Added = {}
local function Answering(Class, Name, Kind, Answer)
	Method(Class, Name, function(...)
		local Ownership = R.Profile and R.Profile.ownership
		if not Ownership or Ownership[Kind] == nil then
			R.Missing(Class, Name)
			return
		end
		return Answer(...)
	end)
end
R.ProfileMethods = function()
	local Ownership = R.Profile and R.Profile.ownership
	if not Ownership then
		return
	end
	if Ownership.assets ~= nil and not Added.PlayerOwnsAssetAsync then
		Added.PlayerOwnsAssetAsync = true
		Answering("MarketplaceService", "PlayerOwnsAssetAsync", "assets", function(_, Player, AssetId)
			return Owned("assets", ProfileOf(Player), AssetId) == true
		end)
	end
	-- (an answer as Roblox's documentation gives it: IsSubscribed, IsRenewing; a listed subscription renews)
	if Ownership.subscriptions ~= nil and not Added.GetUserSubscriptionStatusAsync then
		Added.GetUserSubscriptionStatusAsync = true
		Answering("MarketplaceService", "GetUserSubscriptionStatusAsync", "subscriptions", function(_, Player, SubscriptionId)
			local Subscribed = Owned("subscriptions", ProfileOf(Player), SubscriptionId) == true
			return { IsSubscribed = Subscribed, IsRenewing = Subscribed }
		end)
	end
	-- (the ids of BadgeIds the user has, in their order: Roblox's documentation)
	if Ownership.badges ~= nil and not Added.CheckUserBadgesAsync then
		Added.CheckUserBadgesAsync = true
		Answering("BadgeService", "CheckUserBadgesAsync", "badges", function(_, UserId, BadgeIds)
			local Profile = ProfileOfUser(UserId)
			local Has = {}
			for _, BadgeId in BadgeIds or {} do
				if Owned("badges", Profile, BadgeId) then
					table.insert(Has, BadgeId)
				end
			end
			return Has
		end)
	end
end
R.ProfileMethods()

-- Social: the player's groups (rank and role), friends and their pages, its group list
local function Group(Player, Id)
	for _, Found in Field(ProfileOf(Player), "social", "groups") or {} do
		if string.format("%.0f", Found.id) == Id then
			return Found
		end
	end
	return nil
end
for Name, Answer in { IsInGroup = true, IsInGroupAsync = true, GetRankInGroup = "rank", GetRankInGroupAsync = "rank", GetRoleInGroup = "role", GetRoleInGroupAsync = "role" } do
	Wrap("Player", Name, function(Stub, Self, GroupId, ...)
		-- (the stub checks the arguments and yields as a client does the first time a group is named)
		local Stubbed = Stub(Self, GroupId, ...)
		local Found = Group(Self, (R.Int64Argument(GroupId, 1)))
		if Found then
			return if Answer == true then true else Found[Answer]
		end
		return Stubbed
	end)
end
-- Friends: the stub casts the argument, checks the side and waits as a client does for the web's answer (services.luau's
-- friends methods); the profile's friends are the answer for its player
for _, Name in { "IsFriendsWith", "IsFriendsWithAsync" } do
	Wrap("Player", Name, function(Stub, Self, UserId, ...)
		local Stubbed = Stub(Self, UserId, ...)
		if Field(ProfileOf(Self), "social", "friends") then
			return Friend(R.FriendUser(UserId)) ~= nil
		end
		return Stubbed
	end)
end
-- (an entry as Roblox's documentation lists it: VisitorId, UserName, DisplayName, IsOnline, and the keys a friend's
-- `presence` gives: LastLocation, PlaceId, GameId, LocationType, LastOnline; at most maxFriends of them, and 200)
for _, Name in { "GetFriendsOnline", "GetFriendsOnlineAsync" } do
	Wrap("Player", Name, function(Stub, Self, MaxFriends, ...)
		local Stubbed = Stub(Self, MaxFriends, ...)
		local Friends = Field(ProfileOf(Self), "social", "friends")
		if not Friends then
			return Stubbed
		end
		local Most = R.FriendCount(MaxFriends)
		local Online = {}
		for _, Found in Friends do
			if Found.online and #Online < Most then
				local Entry = { VisitorId = Found.userId, UserName = Found.name, DisplayName = Found.displayName or Found.name, IsOnline = true }
				for Key, Value in Found.presence or {} do
					Entry[Key] = Value
				end
				table.insert(Online, Entry)
			end
		end
		return Online
	end)
end
-- (the Players in the server who are the player's friends, in GetPlayers' order, without the player: Roblox's documentation)
Wrap("Player", "GetFriendsInServerAsync", function(Stub, Self, ...)
	local Stubbed = Stub(Self, ...)
	if not Field(ProfileOf(Self), "social", "friends") then
		return Stubbed
	end
	local InServer = {}
	for _, Player in game:GetService("Players"):GetPlayers() do
		if Player ~= Self and Friend(RawGet(Player, "UserId")) then
			table.insert(InServer, Player)
		end
	end
	return InServer
end)
-- (an entry as Roblox's documentation lists it: User, the friend's user id; PlaceId and GameId, the place and the JobId of the
-- server it plays in, from its `presence` (left out where that has none); for each online friend whose universeId is the
-- game's GameId)
Wrap("Player", "GetFriendsInUniverseAsync", function(Stub, Self, ...)
	local Stubbed = Stub(Self, ...)
	local Friends = Field(ProfileOf(Self), "social", "friends")
	if not Friends then
		return Stubbed
	end
	local Universe = game.GameId
	local InUniverse = {}
	for _, Found in Friends do
		if Found.online and Found.universeId == Universe then
			local Presence = Found.presence or {}
			table.insert(InUniverse, { User = Found.userId, PlaceId = Presence.PlaceId, GameId = Presence.GameId })
		end
	end
	return InUniverse
end)
-- (FriendPages entries: Id, Username, DisplayName, IsOnline, Roblox's documentation)
Wrap("Players", "GetFriendsAsync", function(Stub, Self, UserId, ...)
	local Friends = Field(ProfileOfUser(UserId), "social", "friends")
	if not Friends then
		return Stub(Self, UserId, ...)
	end
	local Items = {}
	for _, Found in Friends do
		table.insert(Items, { Id = Found.userId, Username = Found.name, DisplayName = Found.displayName or Found.name, IsOnline = Found.online == true })
	end
	return ListPages(Items, 50)
end)
-- (an entry as Roblox's documentation lists it: Id, Name, Rank, Role, IsPrimary; no emblem or clan keys, which the profile lacks)
Wrap("GroupService", "GetGroupsAsync", function(Stub, Self, UserId, ...)
	local Groups = Field(ProfileOfUser(UserId), "social", "groups")
	if not Groups then
		return Stub(Self, UserId, ...)
	end
	local List = {}
	for _, Found in Groups do
		table.insert(List, { Id = Found.id, Name = Found.name, Rank = Found.rank, Role = Found.role, IsPrimary = Found.isPrimary == true })
	end
	return List
end)

return true
