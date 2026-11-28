-- GuiService.TopbarInset and GetInsetArea on a client without Roblox's top bar (no core scripts, no inset): the band is empty and
-- every inset's area is the whole screen; the argument's casts
local GuiService = game:GetService("GuiService")
local TopLeft, BottomRight = GuiService:GetGuiInset()
print("TopbarInset", GuiService.TopbarInset, "GetGuiInset", TopLeft, BottomRight)
local Whole = GuiService:GetInsetArea(Enum.ScreenInsets.None)
print("whole", P.nums(Whole.Min.X, Whole.Min.Y), Whole.Width > 0, Whole.Height > 0)
for _, Item in Enum.ScreenInsets:GetEnumItems() do
	local Area = GuiService:GetInsetArea(Item)
	print("GetInsetArea", Item.Name, P.nums(Area.Min.X, Area.Min.Y), Area.Max == Whole.Max)
end
for _, Value in { "None", "TopbarSafeInsets", 0, 3, 9, "x", true, {}, Enum.Material.Plastic } do
	print("GetInsetArea of", typeof(Value), if type(Value) == "table" then "{}" else tostring(Value), P.try(function()
		local Area = GuiService:GetInsetArea(Value :: any)
		return P.nums(Area.Min.X, Area.Min.Y), Area.Max == Whole.Max
	end))
end
print("GetInsetArea of nothing", P.try(function()
	return (GuiService :: any):GetInsetArea()
end))
