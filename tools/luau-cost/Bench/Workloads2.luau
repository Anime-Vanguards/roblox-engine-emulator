-- Game-like workloads: a ground-wave effect's mesh fill and radius (SyntheticWave: the fit was measured on a
-- game's own effect of this shape) and object-style code
local Wave = require(script.Parent.SyntheticWave)
local W, Order = {}, {}
local Sink
local function add(Name, N, Fn)
	W[Name] = { N, Fn }
	table.insert(Order, Name)
end

add("wavefill", 100, Wave.Fill)
add("waveouter", 100000, Wave.Outer)

local Particle = {}
Particle.__index = Particle
function Particle.new(Position, Velocity)
	return setmetatable({ Position = Position, Velocity = Velocity, Age = 0, Alive = true }, Particle)
end
function Particle:Step(Dt)
	self.Age += Dt
	self.Velocity += Vector3.new(0, -9.8, 0) * Dt
	self.Position += self.Velocity * Dt
	if self.Age > 2 then
		self.Alive = false
	end
end
add("particles", 200, function(N)
	for _ = 1, N do
		local List = {}
		for I = 1, 100 do
			List[I] = Particle.new(Vector3.new(I, 0, 0), Vector3.new(0, 10, I * 0.1))
		end
		for _ = 1, 10 do
			for _, P in List do
				P:Step(1 / 60)
			end
		end
	end
end)
add("registry", 300, function(N)
	for _ = 1, N do
		local ByName = {}
		for I = 1, 100 do
			local Name = "Effect" .. (I % 25)
			local Entry = ByName[Name]
			if not Entry then
				Entry = { Name = Name, Count = 0, Items = {} }
				ByName[Name] = Entry
			end
			Entry.Count += 1
			table.insert(Entry.Items, I)
		end
		local Total = 0
		for _, Entry in ByName do
			Total += #Entry.Items
		end
		Sink = Total
	end
end)
add("cframechain", 20000, function(N)
	local Base = CFrame.new(10, 20, 30) * CFrame.Angles(0.1, 0.2, 0.3)
	local Offset = CFrame.new(0, 5, 0)
	local P
	for I = 1, N do
		local C = Base * CFrame.Angles(0, I * 0.01, 0) * Offset
		P = C.Position + C.LookVector * 2
	end
	Sink = P
end)
add("closures", 3000, function(N)
	for _ = 1, N do
		local Callbacks = {}
		for I = 1, 50 do
			Callbacks[I] = function(X)
				return X + I
			end
		end
		local X = 0
		for _, F in Callbacks do
			X = F(X)
		end
		Sink = X
	end
end)
return { W = W, Order = Order }
