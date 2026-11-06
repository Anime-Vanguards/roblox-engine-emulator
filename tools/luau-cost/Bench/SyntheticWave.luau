-- A synthetic ground-wave effect for the cost calibration's `wavefill` and `waveouter` workloads in the public
-- tree: the shape of an effect that rebuilds its mesh every frame (a ring of columns by profile rows written into
-- a flat array of Vector3s, with lobes and noise from sines of each column's angle, and curves sampled by age). The
-- fit in src/cost.rs was measured on a game's own effect of this shape; this stand-in has its kind of work, not its
-- exact counts, so studio.json has no Studio times for these two workloads (fit.py leaves them out).
local TAU = math.pi * 2

local Ring = {
	Columns = 144,
	Duration = 1.6,
	FadeIn = 0.1,
	FadeOut = 0.4,
	Noise = 0.3,
	LobeRadius = 1.5,
	LobeHeight = 0.2,
	LobeSpeed = 3,
	-- (age, value) points, linear between them
	Radius = { { 0, 2 }, { 0.4, 18 }, { 1, 30 } },
	Height = { { 0, 0.5 }, { 0.3, 4 }, { 1, 1 } },
	-- (v, radial offset, lift) a row
	Profile = {},
}
for Row = 0, 30 do
	local V = Row / 30
	table.insert(Ring.Profile, { V, -0.5 + V, math.sin(math.pi * V) })
end

local Cosines, Sines = table.create(Ring.Columns), table.create(Ring.Columns)
for Column = 1, Ring.Columns do
	local Angle = TAU * (Column - 1) / Ring.Columns
	Cosines[Column], Sines[Column] = math.cos(Angle), math.sin(Angle)
end

local function Smooth(X)
	X = math.clamp(X, 0, 1)
	return X * X * (3 - 2 * X)
end

local function Curve(Points, Age)
	if Age <= Points[1][1] then
		return Points[1][2]
	end
	for Index = 2, #Points do
		local From, To = Points[Index - 1], Points[Index]
		if Age <= To[1] then
			return From[2] + (To[2] - From[2]) * (Age - From[1]) / (To[1] - From[1])
		end
	end
	return Points[#Points][2]
end

local function Fill(Age, Out)
	local Radius, Height = Curve(Ring.Radius, Age), Curve(Ring.Height, Age)
	local Grow = Smooth(Age / 0.2)
	local Motion = Age * Ring.LobeSpeed
	local Rows = #Ring.Profile
	for Column = 1, Ring.Columns do
		local Angle = TAU * (Column - 1) / Ring.Columns
		local Lobe = Ring.LobeRadius * Grow * (math.sin(3 * Angle + Motion) + 0.35 * math.sin(5 * Angle - Motion))
		local Lift = 1 + Ring.LobeHeight * (math.sin(2 * Angle - Motion) + 0.35 * math.cos(5 * Angle))
		local Wobble1, Wobble2 = math.sin(11 * Angle - 5.3 * Age), math.cos(19 * Angle + 4.1 * Age)
		local Cosine, Sine = Cosines[Column], Sines[Column]
		for Row, Point in Ring.Profile do
			local Distance = math.max(0.05, Radius + 4 * Point[2] + Lobe)
			local Noise = Ring.Noise * Grow * Point[3] * (0.6 * Wobble1 + 0.4 * Wobble2)
			Out[(Column - 1) * Rows + Row] = Vector3.new(Distance * Cosine, Height * Point[3] * Lift + Noise, Distance * Sine)
		end
	end
end

local function OuterRadius(Age, Angle)
	local Motion = Age * Ring.LobeSpeed
	return Curve(Ring.Radius, Age) + Ring.LobeRadius * Smooth(Age / 0.2) * (math.sin(3 * Angle + Motion) + 0.35 * math.sin(5 * Angle - Motion))
end

local function Opacity(Age)
	return Smooth(Age / Ring.FadeIn) * Smooth((Ring.Duration - Age) / Ring.FadeOut)
end

local Positions = table.create(Ring.Columns * #Ring.Profile)
local Wave = { Sink = nil }

-- `wavefill`: N fills of the ring at ages 0.3-1.2
function Wave.Fill(N)
	for I = 1, N do
		Fill(0.3 + (I % 10) * 0.1, Positions)
	end
	Wave.Sink = Positions
end

-- `waveouter`: N reads of the ring's outer radius and opacity
function Wave.Outer(N)
	local X = 0
	for I = 1, N do
		X += OuterRadius((I % 100) * 0.019, I * 0.01) + Opacity(0.5)
	end
	Wave.Sink = X
end

return Wave
