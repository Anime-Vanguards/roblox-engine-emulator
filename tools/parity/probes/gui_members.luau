-- GuiService.CoreGuiNavigationEnabled (a game script reads and writes it, though its Read permit names RobloxScript) and
-- VoiceChatInternal.VoiceChatState (refused in words of its own)
P.sync()
local function show(Value)
	local Kind = typeof(Value)
	if Kind == "Instance" then
		return "Instance " .. Value.ClassName
	end
	return Kind .. " " .. tostring(Value)
end
local GuiService = game:GetService("GuiService")
local GuiFired = {}
local GuiChanged = GuiService.Changed:Connect(function(Name)
	if Name == "CoreGuiNavigationEnabled" then
		table.insert(GuiFired, Name)
	end
end)
print("nav read", P.try(function() return show((GuiService :: any).CoreGuiNavigationEnabled) end))
print("nav signal", P.try(function() return typeof(GuiService:GetPropertyChangedSignal("CoreGuiNavigationEnabled")) end))
print("nav write false", P.try(function() (GuiService :: any).CoreGuiNavigationEnabled = false end), P.try(function() return show((GuiService :: any).CoreGuiNavigationEnabled) end))
print("nav write same", P.try(function() (GuiService :: any).CoreGuiNavigationEnabled = false end))
print("nav write 1", P.try(function() (GuiService :: any).CoreGuiNavigationEnabled = 1 end), P.try(function() return show((GuiService :: any).CoreGuiNavigationEnabled) end))
print("nav write string", P.try(function() (GuiService :: any).CoreGuiNavigationEnabled = "x" end))
print("nav write nil", P.try(function() (GuiService :: any).CoreGuiNavigationEnabled = nil end), P.try(function() return show((GuiService :: any).CoreGuiNavigationEnabled) end))
print("nav write true", P.try(function() (GuiService :: any).CoreGuiNavigationEnabled = true end), P.try(function() return show((GuiService :: any).CoreGuiNavigationEnabled) end))
print("nav modified", P.try(function() return GuiService:IsPropertyModified("CoreGuiNavigationEnabled") end))
print("nav lower", P.try(function() return show((GuiService :: any).coreGuiNavigationEnabled) end))
task.wait()
GuiChanged:Disconnect()
print("nav changed", #GuiFired)

-- VoiceChatInternal
print("voice internal", P.try(function() return show(game:GetService("VoiceChatInternal")) end), P.try(function() return show(game:FindService("VoiceChatInternal")) end))
local Ok, Voice = pcall(function() return game:GetService("VoiceChatInternal") end)
if Ok and Voice then
	print("voice state", P.try(function() return show((Voice :: any).VoiceChatState) end))
	print("voice state write", P.try(function() (Voice :: any).VoiceChatState = Enum.VoiceChatState.Idle end))
	print("voice state signal", P.try(function() return typeof(Voice:GetPropertyChangedSignal("VoiceChatState")) end))
	print("voice state lower", P.try(function() return show((Voice :: any).voiceChatState) end))
end
