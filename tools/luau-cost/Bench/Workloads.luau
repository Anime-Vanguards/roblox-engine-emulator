-- Luau cost workloads: each W[name] = { n, function(n) } runs its body n times
local W = {}
local Sink = 0
local Order = {}
local function add(Name, N, Fn)
	W[Name] = { N, Fn }
	table.insert(Order, Name)
end

local Arr = table.create(1000, 1.5)
for I = 1, 1000 do
	Arr[I] = I * 0.5
end
local Hash = { alpha = 1, beta = 2, gamma = 3, delta = 4 }
local Strs = {}
for I = 1, 100 do
	Strs[I] = "item" .. I
end
local Obj = { Value = 0 }
function Obj:Bump(X)
	self.Value += X
	return self.Value
end
local Mt = { __index = Obj }
local Funcs = {}
function Funcs.Add(A, B)
	return A + B
end

add("emptyloop", 2000000, function(N)
	for I = 1, N do
	end
end)
add("arith", 1000000, function(N)
	local X = 1
	for I = 1, N do
		X = X * 1.0000001 + I % 7 - (I // 3) * 0.5
	end
	Sink = X
end)
add("callfn", 500000, function(N)
	local F = Funcs.Add
	local X = 0
	for I = 1, N do
		X = Funcs.Add(X, I)
	end
	Sink = X
end)
add("method", 500000, function(N)
	local O = setmetatable({ Value = 0 }, Mt)
	for I = 1, N do
		O:Bump(1)
	end
	Sink = O.Value
end)
add("arrread", 1000000, function(N)
	local A, X = Arr, 0
	for I = 1, N do
		X += A[I % 1000 + 1]
	end
	Sink = X
end)
add("arrwrite", 1000000, function(N)
	local A = Arr
	for I = 1, N do
		A[I % 1000 + 1] = I
	end
end)
add("hashread", 1000000, function(N)
	local H, X = Hash, 0
	for I = 1, N do
		X += H.alpha + H.gamma
	end
	Sink = X
end)
add("hashnew", 200000, function(N)
	local T = {}
	for I = 1, N do
		T[Strs[I % 100 + 1] .. "x"] = I
	end
	Sink = T
end)
add("newtable", 300000, function(N)
	local T
	for I = 1, N do
		T = {}
	end
	Sink = T
end)
add("newrecord", 300000, function(N)
	local T
	for I = 1, N do
		T = { A = I, B = I, C = I }
	end
	Sink = T
end)
add("newarray", 300000, function(N)
	local T
	for I = 1, N do
		T = { I, I, I, I }
	end
	Sink = T
end)
add("closure", 300000, function(N)
	local F
	for I = 1, N do
		F = function()
			return I
		end
	end
	Sink = F
end)
add("concat", 200000, function(N)
	local S
	for I = 1, N do
		S = "a" .. I
	end
	Sink = S
end)
add("format", 100000, function(N)
	local S
	for I = 1, N do
		S = string.format("%d:%s %.2f", I, "x", I * 0.5)
	end
	Sink = S
end)
add("strsub", 300000, function(N)
	local S, X = "hello world example", 0
	for I = 1, N do
		X += #string.sub(S, 2, 7) + string.byte(S, 3)
	end
	Sink = X
end)
add("insert", 300000, function(N)
	local T = {}
	for I = 1, N do
		table.insert(T, I)
	end
	Sink = T
end)
add("ipairs", 10000, function(N)
	local X = 0
	for _ = 1, N do
		for _, V in ipairs(Arr) do
			X += V
		end
	end
	Sink = X
end)
add("pairs", 10000, function(N)
	local X = 0
	for _ = 1, N do
		for _, V in Arr do
			X += V
		end
	end
	Sink = X
end)
add("vector", 500000, function(N)
	local V = Vector3.new(1, 2, 3)
	local D = Vector3.new(0.1, 0.2, 0.3)
	for I = 1, N do
		V = V + D * 0.5
	end
	Sink = V
end)
add("vectornew", 500000, function(N)
	local V
	for I = 1, N do
		V = Vector3.new(I, 2, 3)
	end
	Sink = V
end)
add("vectormag", 500000, function(N)
	local V, X = Vector3.new(1, 2, 3), 0
	for I = 1, N do
		X += V.Magnitude + V.Unit.X + V:Dot(V)
	end
	Sink = X
end)
add("cframemul", 200000, function(N)
	local C = CFrame.new(1, 2, 3)
	local D = CFrame.Angles(0.01, 0.02, 0)
	for I = 1, N do
		C = C * D
	end
	Sink = C
end)
add("cframenew", 200000, function(N)
	local C
	for I = 1, N do
		C = CFrame.new(I, 2, 3)
	end
	Sink = C
end)
add("cframepoint", 200000, function(N)
	local C, V = CFrame.Angles(0.1, 0.2, 0.3) + Vector3.new(1, 2, 3), Vector3.new(4, 5, 6)
	local X
	for I = 1, N do
		X = C * V
	end
	Sink = X
end)
add("cframemethods", 100000, function(N)
	local C, V = CFrame.Angles(0.1, 0.2, 0.3) + Vector3.new(1, 2, 3), Vector3.new(4, 5, 6)
	local X
	for I = 1, N do
		X = C:Inverse():PointToWorldSpace(V)
	end
	Sink = X
end)
add("cframeprops", 200000, function(N)
	local C, X = CFrame.Angles(0.1, 0.2, 0.3) + Vector3.new(1, 2, 3), 0
	for I = 1, N do
		X += C.Position.X + C.LookVector.Y
	end
	Sink = X
end)
add("color3", 200000, function(N)
	local C
	for I = 1, N do
		C = Color3.new(0.1, 0.2, I % 2)
	end
	Sink = C
end)
add("mathlib", 500000, function(N)
	local X = 0
	for I = 1, N do
		X += math.floor(I * 0.3) + math.sin(I) + math.max(I, 5)
	end
	Sink = X
end)
add("objects", 200000, function(N)
	local O
	for I = 1, N do
		O = setmetatable({ Value = I, Other = 1 }, Mt)
	end
	Sink = O
end)
add("sort", 2000, function(N)
	for _ = 1, N do
		local T = table.create(100)
		for I = 1, 100 do
			T[I] = (I * 7919) % 101
		end
		table.sort(T)
	end
end)
add("tconcat", 20000, function(N)
	local S
	for _ = 1, N do
		S = table.concat(Strs, ",", 1, 20)
	end
	Sink = S
end)
add("pcall", 300000, function(N)
	local F, X = Funcs.Add, 0
	for I = 1, N do
		local _, R = pcall(F, I, 1)
		X += R
	end
	Sink = X
end)
add("upvalues", 1000000, function(N)
	local A = 0
	local function Step()
		A += 1
	end
	for I = 1, N do
		Step()
	end
	Sink = A
end)
add("branches", 1000000, function(N)
	local X = 0
	for I = 1, N do
		local M = I % 4
		if M == 0 then
			X += 1
		elseif M == 1 and I > 5 then
			X -= 1
		elseif not (M == 2) then
			X *= 1
		end
	end
	Sink = X
end)
add("typeof", 500000, function(N)
	local X = 0
	local V = Vector3.zero
	for I = 1, N do
		if typeof(V) == "Vector3" and type(I) == "number" then
			X += 1
		end
	end
	Sink = X
end)
add("strfind", 200000, function(N)
	local S, X = "Effect_Part_Glow_12", 0
	for I = 1, N do
		if string.find(S, "Glow", 1, true) then
			X += 1
		end
		if S:match("^Effect") then
			X += 1
		end
	end
	Sink = X
end)
add("whileloop", 1000000, function(N)
	local I, X = 0, 0
	while I < N do
		I += 1
		X += I
	end
	Sink = X
end)
add("vararg", 300000, function(N)
	local function Count(...)
		return select("#", ...)
	end
	local X = 0
	for I = 1, N do
		X += Count(I, 2, 3)
	end
	Sink = X
end)
add("tclone", 100000, function(N)
	local T
	for I = 1, N do
		T = table.clone(Hash)
	end
	Sink = T
end)
add("meshslice", 3000, function(N)
	-- a game-like slice: per-vertex lookups, vector math, a table of results
	local Out
	for _ = 1, N do
		Out = {}
		for I = 1, 100 do
			local P = Vector3.new(Arr[I], Arr[I + 1], Arr[I + 2])
			local D = (P - Vector3.one).Magnitude
			if D > 1 then
				Out[#Out + 1] = { Position = P, Distance = D }
			end
		end
	end
	Sink = Out
end)

return { W = W, Order = Order }
