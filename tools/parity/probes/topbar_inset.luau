-- parity: topbar
-- GuiService.TopbarInset, GetGuiInset and GetInsetArea on a client with Roblox's top bar (its core scripts running: a 58 px inset on a
-- 1280 x 720 screen), and how StarterGui:SetCoreGuiEnabled and SetCore move the top bar's band, frame by frame. A client builds its
-- bar a while after it joins: the probe waits for the band to start right of Roblox's buttons.
local GuiService = game:GetService("GuiService")
local StarterGui = game:GetService("StarterGui")
local RunService = game:GetService("RunService")
local Waited = 0
while GuiService.TopbarInset.Min.X == 0 and Waited < 1800 do
	RunService.Heartbeat:Wait()
	Waited += 1
end
for _ = 1, 120 do
	RunService.Heartbeat:Wait()
end
local function rect(Value)
	return `{P.num(Value.Min.X)},{P.num(Value.Min.Y)},{P.num(Value.Max.X)},{P.num(Value.Max.Y)}`
end
local function insets(Tag)
	local TopLeft, BottomRight = GuiService:GetGuiInset()
	print(Tag, "TopbarInset", rect(GuiService.TopbarInset), "GetGuiInset", P.nums(TopLeft.X, TopLeft.Y, BottomRight.X, BottomRight.Y))
	for _, Item in Enum.ScreenInsets:GetEnumItems() do
		print(Tag, "GetInsetArea", Item.Name, P.try(function()
			return rect(GuiService:GetInsetArea(Item))
		end))
	end
end
insets("start")
for _, Value in { "None", "TopbarSafeInsets", 0, 3, "x", true, {} } do
	print("GetInsetArea of", typeof(Value), if type(Value) == "table" then "{}" else tostring(Value), P.try(function()
		return rect(GuiService:GetInsetArea(Value :: any))
	end))
end
print("GetInsetArea of nothing", P.try(function()
	return rect((GuiService :: any):GetInsetArea())
end))
local Changed = 0
GuiService:GetPropertyChangedSignal("TopbarInset"):Connect(function()
	Changed += 1
end)
-- each change from a thread a Heartbeat resumed, then the band's start after each of the next 40 Heartbeats where it moved
local function watch(Tag, Apply)
	RunService.Heartbeat:Wait()
	Changed = 0
	local Last = rect(GuiService.TopbarInset)
	local Ok, Error = pcall(Apply)
	local Same = rect(GuiService.TopbarInset)
	local Seen = {}
	for Frame = 1, 40 do
		RunService.Heartbeat:Wait()
		local Now = rect(GuiService.TopbarInset)
		if Now ~= Last then
			table.insert(Seen, `+{Frame}={Now}`)
			Last = Now
		end
	end
	print(Tag, if Ok then "ok" else "error " .. P.err(Error), "same frame", Same, "moves", table.concat(Seen, " "), "Changed", Changed)
end
watch("chat off", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, false)
end)
insets("chat off")
print("chat enabled", StarterGui:GetCoreGuiEnabled(Enum.CoreGuiType.Chat), StarterGui:GetCoreGuiEnabled(Enum.CoreGuiType.All))
watch("chat on", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, true)
end)
watch("all off", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.All, false)
end)
watch("all on", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.All, true)
end)
for _, Item in { Enum.CoreGuiType.PlayerList, Enum.CoreGuiType.Health, Enum.CoreGuiType.Backpack, Enum.CoreGuiType.EmotesMenu } do
	watch(Item.Name .. " off", function()
		StarterGui:SetCoreGuiEnabled(Item, false)
	end)
	StarterGui:SetCoreGuiEnabled(Item, true)
end
watch("chat off, on, off", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, false)
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, true)
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, false)
end)
watch("chat on again", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, true)
end)
watch("all off, chat on", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.All, false)
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, true)
end)
watch("all on again", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.All, true)
end)
watch("topbar off", function()
	StarterGui:SetCore("TopbarEnabled", false)
end)
watch("topbar on", function()
	StarterGui:SetCore("TopbarEnabled", true)
end)
insets("end")
