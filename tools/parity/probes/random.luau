-- math.random and Random by seed, math.noise, and the math library's edge cases.
for _, Seed in { 0, 1, 42, 12345, -7, 2 ^ 31, 2 ^ 32 + 5, 1.5, 123456789 } do
	math.randomseed(Seed)
	local Out = {}
	for _ = 1, 4 do
		table.insert(Out, P.num(math.random()))
	end
	for _ = 1, 4 do
		table.insert(Out, tostring(math.random(100)))
	end
	for _ = 1, 4 do
		table.insert(Out, tostring(math.random(-5, 5)))
	end
	table.insert(Out, P.try(math.random, 2 ^ 40))
	print("math.random seed " .. P.num(Seed) .. " " .. table.concat(Out, " "))
end
print("math.random-errors " .. P.try(math.random, 0) .. " | " .. P.try(math.random, 5, 1) .. " | " .. P.try(math.random, 1.5) .. " | " .. P.try(math.random, 1, 2, 3))
print("math.random-bounds " .. P.try(math.random, -2 ^ 53, 2 ^ 53) .. " | " .. P.try(math.random, 2 ^ 63))

for _, Seed in { 0, 1, 42, 12345, -7, 2 ^ 31, 2 ^ 52, 1.5, 0.25, 123456789 } do
	local Generator = Random.new(Seed)
	local Out = {}
	for _ = 1, 3 do
		table.insert(Out, P.num(Generator:NextNumber()))
	end
	for _ = 1, 3 do
		table.insert(Out, P.num(Generator:NextNumber(-10, 10)))
	end
	for _ = 1, 3 do
		table.insert(Out, tostring(Generator:NextInteger(1, 1000)))
	end
	table.insert(Out, tostring(Generator:NextInteger(-2 ^ 40, 2 ^ 40)))
	local Unit = Generator:NextUnitVector()
	table.insert(Out, "u(" .. P.f32s(Unit.X, Unit.Y, Unit.Z) .. ")")
	local List = { 1, 2, 3, 4, 5, 6, 7, 8 }
	Generator:Shuffle(List)
	table.insert(Out, "shuffle(" .. table.concat(List, ",") .. ")")
	local Clone = Generator:Clone()
	table.insert(Out, "clone " .. tostring(Clone:NextInteger(1, 1e6) == Generator:NextInteger(1, 1e6)))
	print("Random seed " .. P.num(Seed) .. " " .. table.concat(Out, " "))
end
local Generator = Random.new(9)
print("Random-errors " .. P.try(Generator.NextInteger, Generator, 5, 1) .. " | " .. P.try(Generator.NextNumber, Generator, 5, 1) .. " | " .. P.try(Generator.NextInteger, Generator, 1.5, 2.5) .. " | " .. P.try(Generator.Shuffle, Generator, { 1, nil, 3 }) .. " | " .. P.try(Random.new, "x"))
print("Random-intrange " .. P.try(Generator.NextInteger, Generator, -2 ^ 63, 2 ^ 63))
print("Random-tostring " .. tostring(Random.new(1)) .. " typeof " .. typeof(Random.new(1)))
-- math.random after Random: independent streams
math.randomseed(5)
local First = math.random(1, 1e6)
math.randomseed(5)
Random.new(5):NextNumber()
print("math.random-independent " .. tostring(First == math.random(1, 1e6)))

-- math.noise
local Rng = P.rng(99)
local Out = {}
for Index = 1, 48 do
	local X, Y, Z = Rng.real(-50, 50), Rng.real(-50, 50), Rng.real(-50, 50)
	local Kind = Index % 3
	local Value = if Kind == 0 then math.noise(X) elseif Kind == 1 then math.noise(X, Y) else math.noise(X, Y, Z)
	table.insert(Out, P.num(Value))
	if #Out == 12 then
		print("noise " .. table.concat(Out, " "))
		table.clear(Out)
	end
end
print("noise-integers " .. P.nums(math.noise(0), math.noise(1), math.noise(1, 2), math.noise(1, 2, 3), math.noise(0.5), math.noise(-0.5, 0.25), math.noise(1e6 + 0.5), math.noise(1e9 + 0.25)))
print("noise-errors " .. P.try(math.noise) .. " | " .. P.try(math.noise, "x"))

-- the math library
print("math " .. P.nums(math.floor(-0.5), math.ceil(-0.5), math.round(0.5), math.round(-0.5), math.round(2.5), math.round(-2.5), math.sign(-3), math.sign(0), math.sign(-0), math.clamp(5, 1, 3), math.clamp(0 / 0, 1, 3)))
print("math-clamp-error " .. P.try(math.clamp, 1, 3, 1))
print("math-more " .. P.nums(math.fmod(7, -3), math.fmod(-7, 3), 7 % -3, -7 % 3, 5.5 % 2, -5.5 % 2, 1 % 0, math.fmod(1, 0), 2 ^ 0.5, (-8) ^ (1 / 3), 0 ^ 0, math.huge - math.huge))
print("math-trig " .. P.nums(math.sin(1), math.cos(1), math.tan(1), math.asin(0.5), math.acos(0.5), math.atan(1), math.atan2(1, -1), math.atan(1, -1), math.sinh(1), math.cosh(1), math.tanh(1)))
print("math-exp " .. P.nums(math.exp(1), math.log(10), math.log(8, 2), math.log(100, 10), math.log10(1000), math.sqrt(2), math.pow(2, 10), math.log(0), math.log(-1)))
print("math-int " .. P.nums(math.abs(-0), math.max(1, 0 / 0, 3), math.min(0 / 0, 1), math.max(-0, 0), math.min(0, -0), math.modf(-3.7)))
print("math-frexp " .. P.nums(math.frexp(8)) .. " ldexp " .. P.num(math.ldexp(0.5, 4)) .. " map " .. P.try(function()
	return math.map(5, 0, 10, 0, 100)
end) .. " lerp " .. P.try(function()
	return math.lerp(1, 3, 0.25)
end))
print("math-constants " .. P.nums(math.pi, math.huge) .. " " .. tostring(math.maxinteger) .. " " .. tostring(math.mininteger))
print("integer-division " .. P.nums(7 // 2, -7 // 2, 7 // -2, 7.5 // 2, 1 // 0, -1 // 0, 0 // 0))
print("bit32 " .. P.nums(bit32.band(0xFF, 0x0F), bit32.bor(1, 2), bit32.bxor(-1, 0), bit32.lshift(1, 31), bit32.rshift(-1, 28), bit32.arshift(-16, 2), bit32.lrotate(1, 33), bit32.extract(0xF0, 4, 4), bit32.replace(0, 1, 31), bit32.bnot(0), bit32.btest(1, 2), bit32.countlz(1), bit32.countrz(8), bit32.byteswap(0x01020304)))
print("bit32-range " .. P.nums(bit32.band(2 ^ 32 + 1), bit32.band(-1.5), bit32.band(1e20)))
print("bit32-errors " .. P.try(bit32.extract, 1, 30, 4) .. " | " .. P.try(bit32.lshift, 1, "x"))
