-- Which Animators a frame steps: a rig with joints and one without, in the Workspace, under a Folder, in a WorldModel, in
-- ReplicatedStorage and outside the DataModel, each playing a track of an asset (the URL form, which a server fetches too).
-- Printed: whether its time moved, IsPlaying and WeightCurrent, and whether GetTrackByAnimationId finds it after Stop(0). Parity runs play the asset's stand-in (tools/parity/animations).
P.sync()
local Rigs = {}
local function Rig(Label, Parent, Jointed)
	local Model = Instance.new("Model")
	Model.Name = Label
	local Base = Instance.new("Part")
	Base.Name = "Base"
	Base.Anchored = true
	Base.CanCollide = false
	Base.CFrame = CFrame.new(0, 400 + #Rigs * 10, 0)
	Base.Parent = Model
	if Jointed then
		local Limb = Instance.new("Part")
		Limb.Name = "Limb"
		Limb.CanCollide = false
		Limb.Parent = Model
		local Joint = Instance.new("Motor6D")
		Joint.Part0, Joint.Part1 = Base, Limb
		Joint.Parent = Base
	end
	local Controller = Instance.new("AnimationController")
	Controller.Parent = Model
	local Animator = Instance.new("Animator")
	Animator.Parent = Controller
	Model.Parent = Parent
	local Animation = Instance.new("Animation")
	Animation.AnimationId = "http://www.roblox.com/asset/?id=507766666"
	local Ok, Track = pcall(function() return Animator:LoadAnimation(Animation) end)
	if not Ok then
		print("rig", Label, "load error", P.err(Track))
		return
	end
	Track:Play(0)
	table.insert(Rigs, { Label, Model, Animator, Track })
end
local Folder = Instance.new("Folder")
Folder.Parent = workspace
Rig("workspace", workspace, true)
Rig("workspace no joints", workspace, false)
Rig("folder", Folder, true)
Rig("replicated storage", game:GetService("ReplicatedStorage"), true)
local World = Instance.new("WorldModel")
World.Parent = game:GetService("ReplicatedStorage")
Rig("world model", World, true)
Rig("outside", nil, true)
Rig("outside no joints", nil, false)
task.wait(1.5)
for _, Entry in Rigs do
	local Track = Entry[4]
	print("rig", Entry[1], "length", P.f32(Track.Length), "moved", Track.TimePosition > 0, "playing", Track.IsPlaying, "weight", P.f32(Track.WeightCurrent))
	Track:Stop(0)
end
task.wait()
task.wait()
for _, Entry in Rigs do
	local Track = Entry[4]
	print("rig", Entry[1], "after stop", Entry[3]:GetTrackByAnimationId("http://www.roblox.com/asset/?id=507766666") == Track, "weight", P.f32(Track.WeightCurrent))
	Entry[2]:Destroy()
end
Folder:Destroy()
World:Destroy()
