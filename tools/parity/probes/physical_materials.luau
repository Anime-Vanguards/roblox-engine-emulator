-- PhysicalProperties.new(material) for every material: density, friction, elasticity, their weights and acoustic absorption.
local Items = Enum.Material:GetEnumItems()
table.sort(Items, function(A, B)
	return A.Value < B.Value
end)
local Line = {}
for _, Material in Items do
	local Ok, Properties = pcall(PhysicalProperties.new, Material)
	table.insert(Line, Material.Name .. "=" .. (if Ok then tostring(Properties) else "E(" .. P.err(Properties) .. ")"))
	if #Line == 6 then
		print("materials " .. table.concat(Line, " | "))
		table.clear(Line)
	end
end
if #Line > 0 then
	print("materials " .. table.concat(Line, " | "))
end
