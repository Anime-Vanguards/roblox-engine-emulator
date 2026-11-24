-- How a client reads a content id that names an asset, before it fetches anything: properties keep the id as written
-- (Roblox's pinned spelling rbxassetid://N&version=V included), and the ids it cannot read raise from the Animator, the
-- animation providers and AssetService, and fail in PreloadAsync (src/content_id.rs). A capture runs offline and ree
-- downloads what it can read, so only ids nothing is fetched for are loaded here.
local Pinned = {
	"rbxassetid://4848&version=1",
	"rbxassetid://4848&Version=2&foo=bar",
	"rbxassetid://4848?version=1",
	"rbxassetid://4848&id=1",
	"rbxassetid:// 4848&version=1",
	"http://www.roblox.com/asset/?id=4848&version=2",
}
for _, Id in Pinned do
	local Animation = Instance.new("Animation")
	Animation.AnimationId = Id
	local Image = Instance.new("ImageLabel")
	Image.Image = Id
	local Sound = Instance.new("Sound")
	Sound.SoundId = Id
	print(
		"kept",
		Id,
		Animation.AnimationId == Id,
		Animation.AnimationContent.Uri == Id,
		Image.Image == Id,
		Image.ImageContent.Uri == Id,
		Sound.SoundId == Id,
		Content.fromUri(Id).Uri == Id
	)
end

local Unreadable = {
	"",
	" ",
	"4848",
	"4848&version=1",
	" rbxassetid://4848",
	"RBXASSETID://4848",
	"Rbxassetid://4848&version=1",
	"rbxassetid:///4848&version=1",
	"rbxassetid:// 4848",
	"rbxassetid://abc",
	"rbxassetid://-5",
	"www.roblox.com/asset/?id=4848",
	"rbxthumb://type=Asset&id=4848&w=150&h=150",
	"RBXHTTP://asset/?id=4848",
	"http://www.roblox.com/asset/?id=4848&foo=bar",
	"http://www.roblox.com/asset/?id=4848&version=abc",
	"http://www.roblox.com/asset/?id=4848&version=-1",
	"http://www.roblox.com/asset/?id=abc",
	"http://www.roblox.com/asset/?id= 4848",
	"https://assetdelivery.roblox.com/v1/assetId/4848",
	"rbxasset://animations/none.rbxm",
	"rbxtemp://12",
	"file:///C:/none.rbxm",
}
local Model = Instance.new("Model")
local Controller = Instance.new("AnimationController")
Controller.Parent = Model
local Animator = Instance.new("Animator")
Animator.Parent = Controller
Model.Parent = workspace
local KeyframeSequences = game:GetService("KeyframeSequenceProvider")
local AnimationClips = game:GetService("AnimationClipProvider")
for _, Id in Unreadable do
	local Animation = Instance.new("Animation")
	Animation.AnimationId = Id
	print("LoadAnimation", Id, P.try(Animator.LoadAnimation, Animator, Animation))
	print("GetKeyframeSequenceAsync", Id, P.try(KeyframeSequences.GetKeyframeSequenceAsync, KeyframeSequences, Id))
	print("GetAnimationClipAsync", Id, P.try(AnimationClips.GetAnimationClipAsync, AnimationClips, Id))
end

-- PreloadAsync calls back once for each Animation that has an id (in the order they finish, so sorted here), and not for
-- one without
local Folder = Instance.new("Folder")
for _, Id in Unreadable do
	local Animation = Instance.new("Animation")
	Animation.AnimationId = Id
	Animation.Parent = Folder
end
local Reported = {}
local Done = false
task.spawn(function()
	print("PreloadAsync", P.try(game:GetService("ContentProvider").PreloadAsync, game:GetService("ContentProvider"), { Folder }, function(Id, Status)
		table.insert(Reported, Id .. " " .. tostring(Status))
	end))
	Done = true
end)
local Started = os.clock()
while not Done and os.clock() - Started < 10 do
	task.wait()
end
table.sort(Reported)
print("PreloadAsync returned", Done, #Reported)
for _, Line in Reported do
	print("preloaded", Line)
end

-- AssetService refuses a mesh id of no protocol it knows before it fetches
local AssetService = game:GetService("AssetService")
for _, Id in { " rbxassetid://4848", "RBXASSETID://4848", "4848", "www.roblox.com/asset/?id=4848" } do
	print("CreateEditableMeshAsync", Id, P.try(AssetService.CreateEditableMeshAsync, AssetService, Content.fromUri(Id)))
	local Ok, Message = pcall(AssetService.CreateMeshPartAsync, AssetService, Content.fromUri(Id))
	print("CreateMeshPartAsync", Id, Ok, string.find(tostring(Message), "Failed to load mesh asset.", 1, true) == 1)
end
