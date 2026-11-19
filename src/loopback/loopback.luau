-- A loopback run's RemoteFunction calls (src/loopback/remotes.rs): Call sends the call and yields until the answer comes
-- back as (true, results...) or (false, message); a callback's error is raised in the caller as its text, which carries
-- the callback's script and line (measured: tools/parity/probes/loopback/remote_function.luau). A server's InvokeClient
-- of a player that is not the loopback one waits for that player to leave and then raises, as a server run's does
-- (server.luau). Run as the engine's own Luau (not charged to scripts).
local Call, Players = ...

local function Answer(Results)
	if Results[1] then
		return table.unpack(Results, 2, Results.n)
	end
	error(Results[2], 0)
end

local function InvokeServer(Self, ...)
	return Answer(table.pack(Call(Self, true, nil, ...)))
end

local function InvokeClient(Self, Player, ...)
	local Results = table.pack(Call(Self, false, Player, ...))
	if Results[1] == "fake" then
		-- a harness's player: no client answers it (server.luau)
		if Player.Parent == Players then
			local Thread = coroutine.running()
			local Connection
			Connection = Players.PlayerRemoving:Connect(function(Leaving)
				if Leaving == Player then
					Connection:Disconnect()
					task.spawn(Thread)
				end
			end)
			coroutine.yield()
		end
		error(`InvokeClient: {Player.Name} left the game before {Self:GetFullName()} returned`, 0)
	end
	return Answer(Results)
end

-- The callback of an invoke that arrived: its outcome goes back to the caller (Done)
local function Run(Callback, Done, CallId, ...)
	Done(CallId, table.pack(pcall(Callback, ...)))
end

return { InvokeServer = InvokeServer, InvokeClient = InvokeClient, Run = Run }
