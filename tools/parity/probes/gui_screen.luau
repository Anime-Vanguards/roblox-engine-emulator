-- GuiService:GetScreenResolution (a core script's: a game script is refused), GuiService.TouchControlsEnabled (its start value;
-- a write takes the value's truth, Changed and the property signal fire when it changes) and UserInputService.ModalEnabled
P.sync()
local GS = game:GetService("GuiService")
local UIS = game:GetService("UserInputService")
print("resolution", P.try(function() return GS:GetScreenResolution() end))
print("resolution dot", P.try(function() return GS.GetScreenResolution(GS) end))
print("resolution other self", P.try(function() return GS.GetScreenResolution(workspace) end))
print("resolution argument", P.try(function() return GS:GetScreenResolution(5) end))
print("resolution value", typeof(GS.GetScreenResolution))
print("touch controls", GS.TouchControlsEnabled, "modal", UIS.ModalEnabled)
local Log = {}
local A = GS:GetPropertyChangedSignal("TouchControlsEnabled"):Connect(function()
	table.insert(Log, "prop " .. tostring(GS.TouchControlsEnabled))
end)
local B = GS.Changed:Connect(function(Name)
	if Name == "TouchControlsEnabled" then
		table.insert(Log, "changed")
	end
end)
local function Step(Label, Value)
	table.clear(Log)
	local Out = P.try(function()
		GS.TouchControlsEnabled = Value
		return GS.TouchControlsEnabled
	end)
	task.wait()
	print("write " .. Label, Out, "|", table.concat(Log, ","))
end
Step("false", false)
Step("false again", false)
Step("true", true)
Step("true again", true)
Step("zero", 0)
Step("one", 1)
Step("string", "x")
Step("empty string", "")
Step("nil", nil)
Step("table", {})
Step("true", true)
-- several writes in one frame
table.clear(Log)
GS.TouchControlsEnabled = false
GS.TouchControlsEnabled = true
GS.TouchControlsEnabled = false
GS.TouchControlsEnabled = true
task.wait()
print("one frame", table.concat(Log, ","))
-- ModalEnabled beside it
table.clear(Log)
print("modal on", P.try(function()
	UIS.ModalEnabled = true
	return UIS.ModalEnabled
end), GS.TouchControlsEnabled)
task.wait()
print("modal on events", table.concat(Log, ","), GS.TouchControlsEnabled)
table.clear(Log)
print("modal off", P.try(function()
	UIS.ModalEnabled = false
	return UIS.ModalEnabled
end), GS.TouchControlsEnabled)
task.wait()
print("modal off events", table.concat(Log, ","), GS.TouchControlsEnabled)
A:Disconnect()
B:Disconnect()
GS.TouchControlsEnabled = true
UIS.ModalEnabled = false
