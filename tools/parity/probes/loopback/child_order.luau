-- parity: loopback
-- The order of a replicated list's children on each side once the server takes some out (a server keeps them as a client
-- does: out of a list of more than 20 its last child takes the taken one's place, out of a shorter one the gap closes).
-- At the join: lists the server reordered before the client joined, before and after they were in a replicated container,
-- parented in reverse, a child moved out and back. After it: removals, a Destroy, a move out of the replicated containers
-- and an addition in a long and a short list, and a reordered list sent whole.
local Storage = game:GetService("ReplicatedStorage")
local Lists = { "Long", "Short", "Early", "Before", "Reverse", "Back", "BackShort", "Out", "Within", "Moved", "Holder.Inside", "Live" }
local function Find(Path)
	local At = Storage
	for Name in string.gmatch(Path, "[^.]+") do
		At = At and At:FindFirstChild(Name)
	end
	return At
end
local function Show(Step)
	for _, Name in Lists do
		local Folder = Find(Name)
		if Folder then
			local Out = {}
			for _, Child in Folder:GetChildren() do
				table.insert(Out, string.sub(Child.Name, 2))
			end
			print(`{Step} {Name}: {table.concat(Out, " ")}`)
		end
	end
end
--- server
local Hidden = game:GetService("ServerStorage")
local function Fill(Name, Count, Parent)
	local Folder = Instance.new("Folder")
	Folder.Name = Name
	for Index = 1, Count do
		local Child = Instance.new("Folder")
		Child.Name = "c" .. Index
		Child.Parent = Folder
	end
	Folder.Parent = Parent
	return Folder
end
local Long = Fill("Long", 25, Storage)
local Short = Fill("Short", 20, Storage)
-- reordered in the container
local Early = Fill("Early", 25, Storage)
Early.c3.Parent = nil
Early.c1.Parent = nil
-- reordered before it is in the container
local Before = Fill("Before", 25, nil)
Before.c3.Parent = nil
Before.c1.Parent = nil
Before.Parent = Storage
-- made in order, parented in reverse
do
	local Reverse = Instance.new("Folder")
	Reverse.Name = "Reverse"
	local Made = {}
	for Index = 1, 25 do
		Made[Index] = Instance.new("Folder")
		Made[Index].Name = "c" .. Index
	end
	for Index = 25, 1, -1 do
		Made[Index].Parent = Reverse
	end
	Reverse.Parent = Storage
end
-- the first taken out and put back, in a long and a short list, and out of the containers and back
for _, Entry in { { "Back", 25, nil }, { "BackShort", 20, nil }, { "Out", 25, Hidden } } do
	local Folder = Fill(Entry[1], Entry[2], Storage)
	local First = Folder.c1
	First.Parent = Entry[3]
	First.Parent = Folder
end
-- the first moved to another list in the containers and back
do
	local Within = Fill("Within", 25, Storage)
	local Other = Fill("Other", 0, Storage)
	local First = Within.c1
	First.Parent = Other
	First.Parent = Within
end
-- a reordered list moved out of the containers and back, and to another place in them
do
	local Moved = Fill("Moved", 25, Storage)
	Moved.c3.Parent = nil
	Moved.Parent = Hidden
	Moved.Parent = Storage
	local Inside = Fill("Inside", 25, Storage)
	Inside.c3.Parent = nil
	Inside.Parent = Fill("Holder", 0, Storage)
end
P.player()
P.meet("ready")
Show("ready")
P.meet("ready seen")
local Taken = { Long.c5, Short.c2, Early.c25, Long.c9, Long.c2 }
Taken[1].Parent = nil
Taken[2].Parent = nil
Taken[3]:Destroy()
Taken[4].Parent = Hidden
local New = Instance.new("Folder")
New.Name = "c99"
New.Parent = Long
Taken[5].Parent = nil
local Live = Fill("Live", 25, nil)
Live.c3.Parent = nil
Live.c1.Parent = nil
Live.Parent = Storage
P.meet("changed")
Show("changed")
P.meet("changed seen")
--- client
for _, Name in Lists do
	if Name ~= "Live" then
		local At = Storage
		for Part in string.gmatch(Name, "[^.]+") do
			At = At:WaitForChild(Part)
		end
	end
end
P.meet("ready")
Show("ready")
P.meet("ready seen")
P.meet("changed")
for _ = 1, 5 do
	task.wait()
end
Show("changed")
P.meet("changed seen")
