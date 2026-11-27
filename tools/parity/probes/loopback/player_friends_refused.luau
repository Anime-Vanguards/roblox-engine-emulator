-- parity: loopback
-- Player's friends methods where the thread cannot yield (a table.sort comparator), each side's first questions of the
-- session (public issue 65): what each raises there, which checks come before the refusal (a negative user id, a count, a
-- cast, the side), and that a refused question is still asked: the next question of its kind waits for the answer while it
-- is on its way, and one asked after that answers at once.
local Names = { "IsFriendsWith", "IsFriendsWithAsync", "GetFriendsOnline", "GetFriendsOnlineAsync", "GetFriendsInServerAsync", "GetFriendsInUniverseAsync" }
local function Invoke(Player, Name, Self, ...)
	return Player[Name](Self, ...)
end
local function Refused(Player, Name, ...)
	local Args = table.pack(...)
	local Line
	table.sort({ 2, 1 }, function(A, B)
		if not Line then
			local Answer = P.try(Invoke, Player, Name, Player, table.unpack(Args, 1, Args.n))
			-- (an answer, or the web's refusal of a player signed in to no account)
			Line = (if coroutine.isyieldable() then "(yieldable) " else "") .. (if Answer == "error HTTP 401" or string.sub(Answer, 1, 3) == "ok " then "passes" else Answer)
		end
		return A < B
	end)
	return Line
end
local function Call(Player, Name, ...)
	local Ran = false
	task.defer(function()
		Ran = true
	end)
	local Answer = P.try(Invoke, Player, Name, Player, ...)
	return (if Answer == "error HTTP 401" or string.sub(Answer, 1, 3) == "ok " then "passes" else Answer) .. " " .. (if Ran then "yielded" else "now")
end
local function Run(Me)
	print("refused IsFriendsWithAsync -1 " .. Refused(Me, "IsFriendsWithAsync", -1))
	print("refused IsFriendsWithAsync 'abc' " .. Refused(Me, "IsFriendsWithAsync", "abc"))
	print("refused GetFriendsOnlineAsync -1 " .. Refused(Me, "GetFriendsOnlineAsync", -1))
	print("refused GetFriendsOnlineAsync 'abc' " .. Refused(Me, "GetFriendsOnlineAsync", "abc"))
	for _, Name in Names do
		print("refused " .. Name .. " 5 " .. Refused(Me, Name, 5))
	end
	print("after IsFriendsWithAsync 5 " .. Call(Me, "IsFriendsWithAsync", 5))
	print("after IsFriendsWith 6 " .. Call(Me, "IsFriendsWith", 6))
	task.wait(2)
	print("later GetFriendsInServerAsync " .. Call(Me, "GetFriendsInServerAsync"))
	print("later GetFriendsOnlineAsync " .. Call(Me, "GetFriendsOnlineAsync"))
	print("later GetFriendsInUniverseAsync " .. Call(Me, "GetFriendsInUniverseAsync"))
	print("later refused IsFriendsWithAsync 7 " .. Refused(Me, "IsFriendsWithAsync", 7))
	print("later refused GetFriendsOnlineAsync 7 " .. Refused(Me, "GetFriendsOnlineAsync", 7))
end
--- server
local Me = P.player()
P.meet("joined")
Run(Me)
--- client
P.meet("joined")
Run(P.player())
