-- parity: loopback players=2
-- Player's friends methods on each side of a play session (public issue 65): IsFriendsWith and IsFriendsWithAsync,
-- GetFriendsOnline and GetFriendsOnlineAsync, GetFriendsInServerAsync and GetFriendsInUniverseAsync. Whether each is a member
-- (and in ReflectionService's list), which side and which player may call it, the order of its checks and the texts they
-- raise, how its argument is cast (which user ids are asked about and which answer false at once, which counts are refused as
-- negative, the warning past 200 friends), and which calls yield: a player's first question of a kind (its friends, its
-- online friends) asks the web, and later ones of that kind answer at once. What the web answers (the "web" lines) depends on
-- the account, and the capture's player is signed in to none. A second player is in the server (the real engine's second
-- client, which runs nothing; ree's server makes it, as a harness does).
local Players = game:GetService("Players")
local LogService = game:GetService("LogService")
local Names = { "IsFriendsWith", "IsFriendsWithAsync", "GetFriendsOnline", "GetFriendsOnlineAsync", "GetFriendsInServerAsync", "GetFriendsInUniverseAsync" }
local Warnings = {}
LogService.MessageOut:Connect(function(Message, Kind)
	if Kind == Enum.MessageType.MessageWarning and string.find(Message, "Friends", 1, true) then
		table.insert(Warnings, Message)
	end
end)
-- (the member is read inside the pcall: an engine without it raises there)
local function Invoke(Player, Name, Self, ...)
	return Player[Name](Self, ...)
end
-- a call's outcome as text (a list's entries by their keys), and whether it yielded (a thread deferred before it has run
-- when it returns)
local function Call(Player, Name, Self, ...)
	local Ran = false
	task.defer(function()
		Ran = true
	end)
	local Results = table.pack(pcall(Invoke, Player, Name, Self, ...))
	local Answer
	if not Results[1] then
		Answer = "error " .. P.err(Results[2])
	elseif type(Results[2]) == "table" then
		local Parts = {}
		for _, Entry in Results[2] do
			if typeof(Entry) == "Instance" then
				table.insert(Parts, Entry.ClassName)
			elseif type(Entry) == "table" then
				local Keys = {}
				for Key in Entry do
					table.insert(Keys, tostring(Key))
				end
				table.sort(Keys)
				table.insert(Parts, "{" .. table.concat(Keys, ",") .. "}")
			else
				table.insert(Parts, typeof(Entry))
			end
		end
		Answer = "ok list " .. #Results[2] .. (if #Parts > 0 then " " .. table.concat(Parts, " ") else "")
	else
		Answer = "ok " .. tostring(Results[2])
	end
	return Answer, if Ran then "yielded" else "now"
end
-- a call's outcome where only the checks are compared: an error it raised, else "passes" (an answer, or the web's refusal of
-- a player signed in to no account)
local function Checked(Player, Name, Self, ...)
	local Answer = Call(Player, Name, Self, ...)
	return if Answer == "error HTTP 401" or string.sub(Answer, 1, 3) == "ok " then "passes" else Answer
end
local function Run(Me, Other, Client)
	for _, Name in Names do
		print("read " .. Name .. " " .. P.try(function()
			return typeof(Me[Name])
		end))
	end
	print("same " .. P.try(function()
		return Me.IsFriendsWithAsync == Me.IsFriendsWithAsync, Me.IsFriendsWithAsync == Me.IsFriendsWith, Me.GetFriendsOnline == Me.GetFriendsOnlineAsync
	end))
	local Listed = {}
	local Read, Methods = pcall(function()
		return game:GetService("ReflectionService"):GetMethodsOfClass("Player")
	end)
	for _, Method in if Read and type(Methods) == "table" then Methods else {} do
		Listed[Method.Name] = true
	end
	for _, Name in Names do
		print("reflection " .. Name .. " " .. (if Listed[Name] then "listed" else "not listed"))
	end

	-- the friend list: user ids that answer at once (negative once cast), then the first one asked about, then the kind's
	-- other questions
	for _, Case in { { "-1", -1 }, { "'-1'", "-1" }, { "-2^31-1", -2 ^ 31 - 1 }, { "nan", 0 / 0 }, { "inf", math.huge }, { "-inf", -math.huge }, { "2^63", 2 ^ 63 }, { "1e20", 1e20 }, { "-2^63", -2 ^ 63 } } do
		local Answer, Yield = Call(Me, "IsFriendsWithAsync", Me, Case[2])
		print("list-fresh IsFriendsWithAsync " .. Case[1] .. " " .. Answer .. " " .. Yield)
	end
	local Answer, Yield = Call(Me, "IsFriendsWithAsync", Me, -0.6)
	print("list-first IsFriendsWithAsync -0.6 " .. Yield)
	print("web IsFriendsWithAsync -0.6 " .. Answer)
	for _, Case in { { "IsFriendsWithAsync", 5 }, { "IsFriendsWith", 6 }, { "GetFriendsInServerAsync" }, { "IsFriendsWithAsync", -0.6 }, { "IsFriendsWith", true } } do
		Answer, Yield = Call(Me, Case[1], Me, Case[2])
		print("list-later " .. Case[1] .. " " .. tostring(Case[2]) .. " " .. Yield)
		print("web " .. Case[1] .. " " .. tostring(Case[2]) .. " " .. Answer)
	end
	-- the online friends: a client's first question of the kind asks, the others answer at once; a server may not ask
	for Index, Case in { { "GetFriendsInUniverseAsync" }, { "GetFriendsOnlineAsync", 3 }, { "GetFriendsOnline" }, { "GetFriendsInUniverseAsync" }, { "GetFriendsOnlineAsync" } } do
		Answer, Yield = Call(Me, Case[1], Me, Case[2])
		if Client then
			print("online-" .. (if Index == 1 then "first " else "later ") .. Case[1] .. " " .. tostring(Case[2]) .. " " .. Yield)
			print("web " .. Case[1] .. " " .. tostring(Case[2]) .. " " .. Answer)
		else
			print("online " .. Case[1] .. " " .. tostring(Case[2]) .. " " .. Answer .. " " .. Yield)
		end
	end

	-- another player: its own friend list (its first question asks again); only the local player's online friends
	print("other " .. (if Other then "found" else "none"))
	if Other then
		Answer, Yield = Call(Me, "IsFriendsWithAsync", Other, 5)
		-- (a server's first question about the other player waited in two captures of three and answered at once in the third,
		-- as if the server had asked already, when the player joined: its wait is not compared)
		print("other-first IsFriendsWithAsync 5 " .. (if Client then Yield else "-"))
		print("web other IsFriendsWithAsync 5 " .. Answer)
		Answer, Yield = Call(Me, "GetFriendsInServerAsync", Other)
		print("other-later GetFriendsInServerAsync " .. Yield)
		print("web other GetFriendsInServerAsync " .. Answer)
		for _, Name in { "GetFriendsOnline", "GetFriendsOnlineAsync", "GetFriendsInUniverseAsync" } do
			local Parts = {}
			for _, Case in { { "5", 5 }, { "-1", -1 }, { "'abc'", "abc" }, { "none" } } do
				Answer, Yield = Call(Me, Name, Other, Case[2])
				table.insert(Parts, Case[1] .. " " .. Answer .. " " .. Yield)
			end
			print("other " .. Name .. " " .. table.concat(Parts, " | "))
		end
	end

	-- the order of the checks: self first
	for _, Name in Names do
		print("self " .. Name .. " workspace,'abc' " .. P.try(Invoke, Me, Name, workspace, "abc") .. " | nil,'abc' " .. P.try(Invoke, Me, Name, nil, "abc")
			.. " | 'x',-1 " .. P.try(Invoke, Me, Name, "x", -1) .. " | Players,5 " .. P.try(Invoke, Me, Name, Players, 5))
	end

	-- the casts (on the player itself, whose questions of both kinds have been asked)
	local Cases = {
		{ "nil", nil }, { "7.5", 7.5 }, { "-1", -1 }, { "0", 0 }, { "-0.4", -0.4 }, { "-0.5", -0.5 }, { "-0.6", -0.6 }, { "0.5", 0.5 },
		{ "2^31-1", 2 ^ 31 - 1 }, { "2^31", 2 ^ 31 }, { "-2^31", -2 ^ 31 }, { "-2^31-1", -2 ^ 31 - 1 }, { "2^32+1", 2 ^ 32 + 1 }, { "2^53", 2 ^ 53 },
		{ "2^62", 2 ^ 62 }, { "2^63", 2 ^ 63 }, { "-2^63", -2 ^ 63 }, { "nan", 0 / 0 }, { "inf", math.huge }, { "-inf", -math.huge }, { "1e20", 1e20 },
		{ "true", true }, { "false", false }, { "'7'", "7" }, { "' 7'", " 7" }, { "'7 '", "7 " }, { "'-1'", "-1" }, { "'-0'", "-0" },
		{ "'2147483648'", "2147483648" }, { "'7.5'", "7.5" }, { "'0x10'", "0x10" }, { "'1e2'", "1e2" }, { "'abc'", "abc" }, { "''", "" },
		{ "{}", {} }, { "{1}", { 1 } }, { "{a=1}", { a = 1 } }, { "Vector3", Vector3.zero }, { "CFrame", CFrame.new() }, { "Color3", Color3.new() },
		{ "function", print }, { "workspace", workspace }, { "Enum", Enum.KeyCode.A }, { "newproxy", newproxy() }, { "thread", coroutine.create(print) },
		{ "buffer", buffer.create(1) },
	}
	for _, Name in Names do
		local Parts = {}
		for _, Case in Cases do
			table.insert(Parts, Case[1] .. " " .. Checked(Me, Name, Me, Case[2]))
		end
		table.insert(Parts, "none " .. Checked(Me, Name, Me))
		table.insert(Parts, "5,'abc' " .. Checked(Me, Name, Me, 5, "abc"))
		print("cast " .. Name .. " " .. table.concat(Parts, " / "))
	end
	-- the answer for a negative user id, after the cast
	print("negative IsFriendsWithAsync -1 " .. Call(Me, "IsFriendsWithAsync", Me, -1) .. " | -0.6 " .. Checked(Me, "IsFriendsWithAsync", Me, -0.6) .. " | nan " .. Call(Me, "IsFriendsWithAsync", Me, 0 / 0))

	-- more than 200 friends asked for: a warning (once a call), then the call goes on (a real engine's MessageOut comes a
	-- while after the call: the earlier casts' warnings are let in before the count starts)
	task.wait(1)
	table.clear(Warnings)
	local Parts = {}
	for _, Case in { { "GetFriendsOnlineAsync", 201 }, { "GetFriendsOnline", 500 }, { "GetFriendsOnlineAsync", 200 }, { "GetFriendsOnline", "300" } } do
		table.insert(Parts, Case[1] .. " " .. tostring(Case[2]) .. " " .. Checked(Me, Case[1], Me, Case[2]))
	end
	if Other then
		table.insert(Parts, "other GetFriendsOnlineAsync 250 " .. Checked(Me, "GetFriendsOnlineAsync", Other, 250))
	end
	task.wait(1)
	print("many " .. table.concat(Parts, " / "))
	print("warnings " .. #Warnings .. " " .. table.concat(Warnings, " | "))
end
--- server
local Me = P.player()
local Made, Second = pcall(Instance.new, "Player")
if Made then
	Second.Name = "Second"
	Second.UserId = 102
	Second.Parent = Players
else
	local Deadline = os.clock() + 60
	while #Players:GetPlayers() < 2 and os.clock() < Deadline do
		task.wait()
	end
	for _, Player in Players:GetPlayers() do
		if Player ~= Me then
			Second = Player
		end
	end
end
P.meet("joined")
Run(Me, Second, false)
--- client
P.meet("joined")
local Me = P.player()
local Other
for _, Player in Players:GetPlayers() do
	if Player ~= Me then
		Other = Player
	end
end
Run(Me, Other, true)
