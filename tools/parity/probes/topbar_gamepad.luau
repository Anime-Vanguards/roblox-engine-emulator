-- parity: topbar=gamepad
-- GuiService.TopbarInset, GetGuiInset and GetInsetArea on a gamepad client with Roblox's top bar (its core scripts running, a
-- gamepad its only input: a 58 px inset on a 1280 x 720 screen), where the bar shows neither the menu button nor the Unibar, and
-- whether StarterGui:SetCoreGuiEnabled moves the band. The probe waits as long as topbar_inset does for the bar to be built.
local GuiService = game:GetService("GuiService")
local StarterGui = game:GetService("StarterGui")
local RunService = game:GetService("RunService")
local UserInputService = game:GetService("UserInputService")
print("device", UserInputService.GamepadEnabled, UserInputService.KeyboardEnabled, UserInputService.MouseEnabled, UserInputService.PreferredInput)
for _ = 1, 360 do
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
	local Seen = {}
	for Frame = 1, 40 do
		RunService.Heartbeat:Wait()
		local Now = rect(GuiService.TopbarInset)
		if Now ~= Last then
			table.insert(Seen, `+{Frame}={Now}`)
			Last = Now
		end
	end
	print(Tag, if Ok then "ok" else "error " .. P.err(Error), "moves", table.concat(Seen, " "), "Changed", Changed)
end
watch("chat off", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, false)
end)
insets("chat off")
watch("chat on", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.Chat, true)
end)
watch("all off", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.All, false)
end)
watch("all on", function()
	StarterGui:SetCoreGuiEnabled(Enum.CoreGuiType.All, true)
end)
insets("end")
