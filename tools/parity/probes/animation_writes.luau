-- A track's Parent, GetTimeOfKeyframe before the clip arrived and after, TimePosition written (looped or not, playing or
-- not, before the clip arrived) as it reads back at once, and whether a Transform a track or a script writes fires Changed
-- (a Motor6D's, a Bone's). The asset is Roblox's R15 idle by URL, which a server fetches too; parity runs play its stand-in (tools/parity/animations).
P.sync()
local Id = "http://www.roblox.com/asset/?id=507766666"
local Model = Instance.new("Model")
local Root = Instance.new("Part")
Root.Name = "HumanoidRootPart"
Root.Anchored = true
Root.CFrame = CFrame.new(0, 500, 0)
Root.Parent = Model
local Torso = Instance.new("Part")
Torso.Name = "LowerTorso"
Torso.Parent = Model
local Joint = Instance.new("Motor6D")
Joint.Name = "Root"
Joint.Part0, Joint.Part1 = Root, Torso
Joint.Parent = Torso
local Humanoid = Instance.new("Humanoid")
Humanoid.Parent = Model
local Animator = Instance.new("Animator")
Animator.Parent = Humanoid
Model.Parent = workspace
local Animation = Instance.new("Animation")
Animation.Name = "Idle"
Animation.AnimationId = Id
local Track = Animator:LoadAnimation(Animation)
print("parent", Track.Parent, "name", Track.Name, "length at load", P.f32(Track.Length))
print("set before arrival", P.try(function() Track.TimePosition = -0.3 return Track.TimePosition end))
print("time of keyframe before", P.try(function() return Track:GetTimeOfKeyframe("x") end))
local Start = os.clock()
local Frames = 0
while Track.Length == 0 and Frames < 300 do
	task.wait()
	Frames += 1
end
print("arrived", Track.Length > 0, "within a second", os.clock() - Start < 1, "after a frame or more", Frames > 0)
print("time of keyframe after", P.try(function() return Track:GetTimeOfKeyframe("x") end))
print("looped", Track.Looped)
local Changed, Transform = {}, 0
Joint.Changed:Connect(function(Name) table.insert(Changed, Name) end)
Joint:GetPropertyChangedSignal("Transform"):Connect(function() Transform += 1 end)
Track:Play(0)
task.wait(0.3)
print("playing changed", #Changed, table.concat(Changed, ","), "transform signal", Transform, "posed", Joint.Transform ~= CFrame.identity)
Track:Stop(0)
task.wait()
table.clear(Changed)
Transform = 0
Joint.Transform = CFrame.new(0, 1, 0)
task.wait()
print("script write changed", #Changed, table.concat(Changed, ","), "transform signal", Transform)
local Second = Animator:LoadAnimation(Animation)
print("second load length", P.f32(Second.Length))
Second.Looped = true
Second:Play(0)
task.wait()
Second.TimePosition = -0.3
print("negative set reads", P.f32(Second.TimePosition))
task.wait()
print("negative after a frame inside", Second.TimePosition >= 0 and Second.TimePosition < Second.Length, "near the end", Second.TimePosition > Second.Length - 0.5)
Second.TimePosition = -5
task.wait()
print("minus five after a frame inside", Second.TimePosition >= 0 and Second.TimePosition < Second.Length)
Second:Stop(0)
local Cases = { { true, -0.3 }, { true, 5 }, { true, 4.3 }, { true, 8.7 }, { false, -0.3 }, { false, 5 }, { false, 4.3 }, { false, 1.25 } }
for _, Case in Cases do
	Second.Looped = Case[1]
	Second.TimePosition = Case[2]
	print("set", Case[1], Case[2], "reads", P.f32(Second.TimePosition))
end
Second.Looped = true
Second:Play(0)
task.wait()
for _, Case in Cases do
	Second.Looped = Case[1]
	Second.TimePosition = Case[2]
	print("playing set", Case[1], Case[2], "reads", P.f32(Second.TimePosition), Second.IsPlaying)
end
Second:Stop(0)
local Bone = Instance.new("Bone")
Bone.Parent = Root
local Log = {}
Bone.Changed:Connect(function(Name) table.insert(Log, Name) end)
Bone.Transform = CFrame.new(0, 1, 0)
Joint.Transform = CFrame.new(0, 2, 0)
task.wait()
print("bone write changed", table.concat(Log, ","), "joint reads", P.f32(Joint.Transform.Y), "bone reads", P.f32(Bone.Transform.Y))
Model:Destroy()
