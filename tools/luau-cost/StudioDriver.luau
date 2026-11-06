-- The Studio side of the Luau cost calibration (Bench.luau): a LocalScript in StarterPlayerScripts with the
-- Bench modules (Workloads, Workloads2, SyntheticWave) as its children. In the playtest client it runs
-- each workload 7 times (a frame apart) and publishes the per-run os.clock (median, min, max) as JSON in
-- ReplicatedStorage.LuauBench.Value, the studio.json fit.py reads.
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local HttpService = game:GetService("HttpService")
local Sets = { require(script.Workloads), require(script.Workloads2) }
task.wait(3)

local Out = {}
for _, Bench in Sets do
	for _, Name in Bench.Order do
		local N, Fn = Bench.W[Name][1], Bench.W[Name][2]
		Fn(2)
		local Times = {}
		for Rep = 1, 7 do
			task.wait()
			local T0 = os.clock()
			Fn(N)
			Times[Rep] = (os.clock() - T0) / N
		end
		table.sort(Times)
		Out[Name] = { median = Times[4], min = Times[1], max = Times[7] }
	end
end

local Value = Instance.new("StringValue")
Value.Name = "LuauBench"
Value.Value = HttpService:JSONEncode(Out)
Value.Parent = ReplicatedStorage
