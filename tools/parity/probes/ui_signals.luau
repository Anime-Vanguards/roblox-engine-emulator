-- parity: device=desktop rendered
-- When the layout's own properties change and fire, against the frame: AbsolutePosition, AbsoluteSize,
-- AbsoluteWindowSize, CanvasPosition, AbsoluteCanvasSize and a UIListLayout's AbsoluteContentSize, after a mount,
-- a scroll, a resize and an added row; and what a script reads right after it writes. Needs a GUI layout (a client
-- with a screen, or a host that adds one).
local RunService = game:GetService("RunService")
local Players = game:GetService("Players")
P.sync()
local Log = {}
local function Note(Text)
	table.insert(Log, Text)
end
local function V2(Value)
	return string.format("%d,%d", math.floor(Value.X + 0.5), math.floor(Value.Y + 0.5))
end
local Marks = {}
for _, Name in { "PreRender", "Heartbeat" } do
	table.insert(Marks, RunService[Name]:Connect(function()
		Note("|" .. string.sub(Name, 1, 4))
	end))
end

local Gui = Instance.new("ScreenGui")
Gui.Name = "ParityUISignals"
Gui.IgnoreGuiInset = true
Gui.ResetOnSpawn = false
local Scroller = Instance.new("ScrollingFrame")
Scroller.Name = "Scroller"
Scroller.Position = UDim2.fromOffset(10, 20)
Scroller.Size = UDim2.fromOffset(300, 200)
Scroller.CanvasSize = UDim2.fromOffset(0, 0)
Scroller.AutomaticCanvasSize = Enum.AutomaticSize.Y
Scroller.ScrollBarThickness = 12
Scroller.Parent = Gui
local List = Instance.new("UIListLayout")
List.SortOrder = Enum.SortOrder.LayoutOrder
List.Parent = Scroller
local Rows = {}
for Index = 1, 12 do
	local Row = Instance.new("Frame")
	Row.Name = "Row" .. Index
	Row.LayoutOrder = Index
	Row.Size = UDim2.new(1, 0, 0, 50)
	Row.Parent = Scroller
	Rows[Index] = Row
end
local Watched = {
	{ "scroller.AbsolutePosition", Scroller, "AbsolutePosition" },
	{ "scroller.AbsoluteSize", Scroller, "AbsoluteSize" },
	{ "scroller.AbsoluteWindowSize", Scroller, "AbsoluteWindowSize" },
	{ "scroller.CanvasPosition", Scroller, "CanvasPosition" },
	{ "scroller.AbsoluteCanvasSize", Scroller, "AbsoluteCanvasSize" },
	{ "list.AbsoluteContentSize", List, "AbsoluteContentSize" },
	{ "row5.AbsolutePosition", Rows[5], "AbsolutePosition" },
	{ "row5.AbsoluteSize", Rows[5], "AbsoluteSize" },
}
local Connections = {}
for _, Entry in Watched do
	table.insert(Connections, Entry[2]:GetPropertyChangedSignal(Entry[3]):Connect(function()
		Note(Entry[1] .. "=" .. V2(Entry[2][Entry[3]]))
	end))
end
local function Window(Label, Frames)
	for _ = 1, Frames do
		task.wait()
	end
	-- from the first change to the last entry, marks trimmed at the end
	while #Log > 0 and string.sub(Log[#Log], 1, 1) == "|" do
		table.remove(Log)
	end
	print(Label .. " " .. table.concat(Log, " "))
	table.clear(Log)
end

-- 1. mount
Gui.Parent = Players.LocalPlayer:WaitForChild("PlayerGui")
print("mount-read " .. V2(Scroller.AbsoluteSize) .. " " .. V2(Rows[5].AbsolutePosition) .. " " .. V2(List.AbsoluteContentSize))
Window("mount", 4)

-- 2. scroll from a script, at the wait resumption point
P.sync()
table.clear(Log)
Scroller.CanvasPosition = Vector2.new(0, 120)
print("scroll-read canvas " .. V2(Scroller.CanvasPosition) .. " row5 " .. V2(Rows[5].AbsolutePosition))
Window("scroll", 4)

-- 3. scroll past the end: the canvas position is clamped when?
P.sync()
table.clear(Log)
Scroller.CanvasPosition = Vector2.new(0, 5000)
print("overscroll-read canvas " .. V2(Scroller.CanvasPosition))
Window("overscroll", 4)
print("overscroll-after canvas " .. V2(Scroller.CanvasPosition) .. " row5 " .. V2(Rows[5].AbsolutePosition))

-- 4. resize the window
P.sync()
table.clear(Log)
Scroller.Size = UDim2.fromOffset(300, 250)
print("resize-read size " .. V2(Scroller.AbsoluteSize) .. " window " .. V2(Scroller.AbsoluteWindowSize))
Window("resize", 4)

-- 5. a row added
P.sync()
table.clear(Log)
local Extra = Instance.new("Frame")
Extra.Name = "Row13"
Extra.LayoutOrder = 13
Extra.Size = UDim2.new(1, 0, 0, 50)
Extra.Parent = Scroller
print("add-read content " .. V2(List.AbsoluteContentSize) .. " canvas " .. V2(Scroller.AbsoluteCanvasSize))
Window("add", 4)

-- 6. a scroll from a Heartbeat handler, and the read in the next PreRender handler
P.sync()
table.clear(Log)
local Seen
RunService.Heartbeat:Once(function()
	Scroller.CanvasPosition = Vector2.new(0, 60)
	Note("hb-write")
	RunService.PreRender:Once(function()
		Seen = V2(Rows[5].AbsolutePosition)
		Note("prerender-read row5 " .. Seen)
	end)
end)
Window("scroll-from-heartbeat", 4)

-- 7. coordinates: AbsolutePosition and AbsoluteSize of ScreenGuis with and without IgnoreGuiInset, and the inset
local Insets = {}
for _, Ignore in { true, false } do
	local Screen = Instance.new("ScreenGui")
	Screen.IgnoreGuiInset = Ignore
	local Box = Instance.new("Frame")
	Box.Position = UDim2.fromOffset(10, 20)
	Box.Size = UDim2.new(0.5, 0, 0.5, 0)
	Box.Parent = Screen
	Screen.Parent = Players.LocalPlayer.PlayerGui
	table.insert(Insets, string.format("ignore=%s screen %s/%s box %s/%s", tostring(Ignore), V2(Screen.AbsolutePosition), V2(Screen.AbsoluteSize), V2(Box.AbsolutePosition), V2(Box.AbsoluteSize)))
	Screen:Destroy()
end
local TopLeft, BottomRight = game:GetService("GuiService"):GetGuiInset()
print("coordinates inset " .. V2(TopLeft) .. "/" .. V2(BottomRight) .. " " .. table.concat(Insets, " | "))

for _, Connection in Connections do
	Connection:Disconnect()
end
for _, Connection in Marks do
	Connection:Disconnect()
end
Gui:Destroy()
