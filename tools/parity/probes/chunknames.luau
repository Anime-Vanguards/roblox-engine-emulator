-- parity: identity=4
-- What scripts see of a module's chunk name: debug.info, error positions and tracebacks. A real client lets a script
-- write Source and require the module it made only at identity 4.
local Folder = Instance.new("Folder")
Folder.Name = "ParityProbes"
Folder.Parent = game:GetService("ReplicatedStorage")
local function Make(Name, Parent, Source)
	local Module = Instance.new("ModuleScript")
	Module.Name = Name
	Module.Source = Source
	Module.Parent = Parent
	return Module
end
local Source = table.concat({
	"local M = {}",
	"function M.info() return debug.info(1, 's') end",
	"function M.fail() error('inside') end",
	"function M.index() local Missing = nil return Missing.Field end",
	"function M.trace() return debug.traceback('t') end",
	"function M.member() return workspace.NotAMember end",
	"function M.level2() local function Inner() error('up', 2) end Inner() end",
	"return M",
}, "\n")
local function Report(Label, Module)
	local Ok, M = pcall(require, Module)
	if not Ok then
		print(Label .. " require-error " .. tostring(M))
		return
	end
	local Info = M.info()
	print(Label .. " info " .. tostring(Info))
	print(Label .. " fail " .. tostring(select(2, pcall(M.fail))))
	print(Label .. " index " .. tostring(select(2, pcall(M.index))))
	print(Label .. " member " .. tostring(select(2, pcall(M.member))))
	print(Label .. " level2 " .. tostring(select(2, pcall(M.level2))))
	-- the traceback's frames inside the module only (the caller's frames are the test runner's)
	local Frames = {}
	for Line in string.gmatch(M.trace(), "[^\n]+") do
		if string.find(Line, Info, 1, true) or Line == "t" then
			table.insert(Frames, Line)
		end
	end
	print(Label .. " trace " .. table.concat(Frames, " / "))
end
Report("nested", Make("Chunk", Folder, Source))
local Odd = Instance.new("Folder")
Odd.Name = "Odd Name.v2"
Odd.Parent = Folder
Report("odd-names", Make("my module [1]", Odd, Source))
Report("loose", Make("Loose", nil, Source))
local Deep = Folder
for Index = 1, 12 do
	local Next = Instance.new("Folder")
	Next.Name = "Level" .. Index .. string.rep("x", 10)
	Next.Parent = Deep
	Deep = Next
end
Report("long", Make("Chunk", Deep, Source))
Folder:Destroy()

-- require's own errors
local Requires = Instance.new("Folder")
Requires.Name = "ParityRequires"
Requires.Parent = game:GetService("ReplicatedStorage")
local Broken = Make("Broken", Requires, "error('module failed')")
local BrokenZero = Make("BrokenZero", Requires, "error('module failed plainly', 0)")
local Two = Make("Two", Requires, "return 1, 2")
local None = Make("None", Requires, "local x = 1")
local Cycle = Make("Cycle", Requires, "return require(script)")
local Yielding = Make("Yielding", Requires, "task.wait() return 'later'")
for _, Case in { { "broken", Broken }, { "broken-again", Broken }, { "broken-zero", BrokenZero }, { "two", Two }, { "none", None }, { "cycle", Cycle } } do
	local Ok, Result = pcall(require, Case[2])
	print("require-" .. Case[1] .. " " .. tostring(Ok) .. " " .. P.err(tostring(Result)))
end
print("require-yielding " .. tostring(select(2, pcall(require, Yielding))))
print("require-string " .. P.try(require, "ReplicatedStorage.ParityRequires.Two"))
print("require-folder " .. P.try(require, Requires))
Requires:Destroy()
