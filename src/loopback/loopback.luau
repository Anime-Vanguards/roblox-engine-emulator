-- A loopback run's RemoteFunction calls (src/loopback/remotes.rs): Call sends the call and yields until the answer comes
-- back as (true, results...) or (false, message); a callback's error is raised in the caller as its text, which carries
-- the callback's script and line (measured: tools/parity/probes/loopback/remote_function.luau). A server's InvokeClient
-- of a player that is not the loopback one waits for that player to leave and then raises, as a server run's does
-- (server.luau). Run as the engine's own Luau (not charged to scripts).
local Call, Players, Report, CreateThread, Resume = ...

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

-- The callback of an invoke that arrived, on a thread of its own: its outcome goes back to the caller (Done). An error it raises
-- before it first yields is also logged on this side as an uncaught error is (Report: ScriptContext.Error and an error line with
-- the callback's own stack, a table's as "Error occurred, no output from Luau."); one raised after a yield is not. Measured in a
-- 0.747 play session's server and client, OnServerInvoke and OnClientInvoke alike:
-- tools/parity/probes/loopback/remote_callback_errors.luau.
local function Run(Callback, Done, CallId, ...)
	local Fresh = true
	local function Failed(Problem)
		if Fresh then
			Report(Problem)
		end
		return Problem
	end
	-- (the VM's own coroutine.create: the scripts' charges a thread's price)
	local Handler = CreateThread(function(...)
		Done(CallId, xpcall(Callback, Failed, ...))
	end)
	coroutine.resume(Handler, ...)
	Fresh = false
end

return { InvokeServer = InvokeServer, InvokeClient = InvokeClient, Run = Run }
