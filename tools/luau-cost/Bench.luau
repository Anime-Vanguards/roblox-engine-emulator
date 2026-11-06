-- The Luau cost calibration (src/cost.rs `luau`): every workload of Bench/Workloads and Bench/Workloads2 run
-- N times, printed as one "BENCH {json}" line each: os.clock a run (ree's charge) and the VM's work counters a
-- run. In ree: REE_WORK_COUNTERS=1 ree run --harness tools/luau-cost/Bench.luau --seconds 60 > ree.json
-- In Studio: StudioDriver.luau as a LocalScript (playtest client) with the Bench modules under it.
-- Then: python -I tools/luau-cost/fit.py ree.json studio.json
local HttpService = game:GetService("HttpService")
local Sets = { require(script.Workloads), require(script.Workloads2) }

return function()
	for _, Bench in Sets do
		for _, Name in Bench.Order do
			local N, Fn = Bench.W[Name][1], Bench.W[Name][2]
			Fn(2)
			task.wait()
			local A = os.workcounters()
			local T0 = os.clock()
			Fn(N)
			local T1 = os.clock()
			local B = os.workcounters()
			local Ops = {}
			for Op, Count in B.ops do
				local D = Count - (A.ops[Op] or 0)
				if D > 0 then
					Ops[tostring(Op - 1)] = D / N
				end
			end
			local Out = { name = Name, n = N, clock = (T1 - T0) / N, ops = Ops }
			for _, Key in { "allocated", "ccalls", "tmcalls", "hostcalls", "hostops", "objects", "stored", "rehashed" } do
				Out[Key] = (B[Key] - A[Key]) / N
			end
			print("BENCH " .. HttpService:JSONEncode(Out))
		end
	end
end
