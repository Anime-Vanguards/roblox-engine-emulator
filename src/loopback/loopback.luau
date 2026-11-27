-- A loopback run's RemoteFunction calls (src/loopback/remotes.rs): Call sends the call and yields until the answer comes
-- back as (true, results...) or (false, message); a callback's error is raised in the caller as its text, which carries
-- the callback's script and line (measured: tools/parity/probes/loopback/remote_function.luau). A server's InvokeClient
-- of a player that is not the loopback one waits for that player to leave and then raises, as a server run's does
-- (server.luau). Run as the engine's own Luau (not charged to scripts). A callback runs as a BindableFunction's does
-- (lua/bindable.rs run_own, from remotes.rs).
local Call, Players, Resume = ...

-- (results pass as arguments, never packed into a table: a call allocates nothing of its own here)
local function Answer(Ok, ...)
	if Ok then
		return ...
	end
	error((...), 0)
end

local function InvokeServer(Self, ...)
	return Answer(Call(Self, true, nil, ...))
end

local function Answered(Self, Player, Ok, ...)
	if Ok == "fake" then
		-- a harness's player: no client answers it (server.luau)
		if Player.Parent == Players then
			local Thread = coroutine.running()
			local Connection
			Connection = Players.PlayerRemoving:Connect(function(Leaving)
				if Leaving == Player then
					Connection:Disconnect()
					Resume(Thread)
				end
			end)
			coroutine.yield()
		end
		error(`Player {Player.Name} disconnected during remote call to {Self:GetFullName()}`, 0)
	end
	return Answer(Ok, ...)
end

local function InvokeClient(Self, Player, ...)
	return Answered(Self, Player, Call(Self, false, Player, ...))
end

return { InvokeServer = InvokeServer, InvokeClient = InvokeClient }
