//! Script-level checks of the 3D world: part and attachment CFrame views, Workspace.Terrain, the
//! spatial queries and the calls reported as world events.

use std::{cell::RefCell, rc::Rc};

use mlua::prelude::*;

use super::WorldEvent;
use crate::{
    Engine, Options,
    lua::{self as engine_lua, globals::CallOutcome},
};

fn start() -> (Lua, Rc<Engine>) {
    let engine = Engine::new(Options::default()).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    (lua, engine)
}

/// The frames [`run`] steps a waiting script through before it fails the test: a minute of 60 frames a second
const RUN_FRAMES: u32 = 3600;

/// Runs `source` as a script to its end: while it waits (task.wait, an event's Wait) the engine steps frames, one at a time,
/// until it finishes, and the test fails when it is still waiting after [`RUN_FRAMES`] (what follows a wait would otherwise
/// never run, its asserts passing unchecked). Panics with the script's error.
fn run(lua: &Lua, engine: &Rc<Engine>, source: &str) {
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        crate::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(lua, engine, script).expect("environment");
    let function = lua.load(source).set_name("=Test").set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(lua, engine, function, LuaMultiValue::new()).expect("call");
    let mut frames = 0;
    loop {
        match call.outcome().unwrap() {
            CallOutcome::Finished(_) => return,
            CallOutcome::Failed(message, trace) => panic!("test script failed: {message}\n{}", trace.unwrap_or_default()),
            CallOutcome::Pending if frames < RUN_FRAMES => {
                crate::scheduler::step(lua, engine, None).expect("step");
                frames += 1;
            }
            CallOutcome::Pending => panic!("test script did not finish: still waiting after {frames} frames"),
        }
    }
}

fn check(source: &str) {
    let (lua, engine) = start();
    run(&lua, &engine, source);
}

/// What follows a wait in a test script runs and is checked: [`run`] steps frames until the script ends (an assert after a
/// task.wait() used to pass unchecked, the call left waiting).
#[test]
#[should_panic(expected = "checked after the wait")]
fn a_test_script_is_checked_after_it_waits() {
    check("task.wait() assert(false, 'checked after the wait')");
}

/// A test script still waiting after [`RUN_FRAMES`] frames fails its test.
#[test]
#[should_panic(expected = "still waiting")]
fn a_test_script_that_never_ends_fails_its_test() {
    check("while true do task.wait() end");
}

#[test]
fn part_cframe_views() {
    check(
        r#"
        local function near(a, b) return (a - b).Magnitude < 1e-3 end
        local Part = Instance.new("Part")
        Part.CFrame = CFrame.new(1, 2, 3) * CFrame.Angles(0, math.rad(90), 0)
        assert(near(Part.Position, Vector3.new(1, 2, 3)), tostring(Part.Position))
        assert(near(Part.Orientation, Vector3.new(0, 90, 0)), tostring(Part.Orientation))
        -- (XYZ angles lose an axis at 90 degrees about Y: the last one reads 0)
        assert(near(Part.Rotation, Vector3.new(0, 90, 0)), tostring(Part.Rotation))
        Part.Position = Vector3.new(5, 6, 7)
        assert(near(Part.CFrame.Position, Vector3.new(5, 6, 7)))
        assert(near(Part.CFrame.LookVector, Vector3.new(-1, 0, 0)), "rotation kept")
        Part.Orientation = Vector3.new(30, 0, 0)
        assert(near(Part.Position, Vector3.new(5, 6, 7)), "position kept")
        assert(near(Part.Orientation, Vector3.new(30, 0, 0)), tostring(Part.Orientation))
        Part.Rotation = Vector3.new(10, 20, 30)
        assert(near(Part.Rotation, Vector3.new(10, 20, 30)), tostring(Part.Rotation))
        assert(near(Part.CFrame.Position, Vector3.new(5, 6, 7)))
        -- the views report their changes
        local Seen = {}
        Part:GetPropertyChangedSignal("Position"):Connect(function() table.insert(Seen, "Position") end)
        Part.CFrame = CFrame.new(0, 10, 0)
        task.wait()
        assert(Seen[1] == "Position", "Position changed with CFrame")
        "#,
    );
}

#[test]
fn attachment_world_members() {
    check(
        r#"
        local function near(a, b) return (a - b).Magnitude < 1e-3 end
        local Part = Instance.new("Part")
        Part.CFrame = CFrame.new(10, 0, 0) * CFrame.Angles(0, math.rad(90), 0)
        local Attachment = Instance.new("Attachment")
        Attachment.Position = Vector3.new(0, 0, -2)
        Attachment.Parent = Part
        -- -Z of a part turned 90 degrees about Y is -X
        assert(near(Attachment.WorldPosition, Vector3.new(8, 0, 0)), tostring(Attachment.WorldPosition))
        assert(near(Attachment.WorldCFrame.Position, Vector3.new(8, 0, 0)))
        assert(near(Attachment.WorldAxis, Part.CFrame.RightVector))
        Attachment.WorldPosition = Vector3.new(10, 5, 0)
        assert(near(Attachment.Position, Vector3.new(0, 5, 0)), tostring(Attachment.Position))
        Attachment.WorldCFrame = CFrame.new(12, 0, 0)
        assert(near(Attachment.WorldPosition, Vector3.new(12, 0, 0)))
        -- an attachment outside a part is in world space
        local Loose = Instance.new("Attachment")
        Loose.CFrame = CFrame.new(1, 2, 3)
        assert(near(Loose.WorldPosition, Vector3.new(1, 2, 3)))
        Loose.Axis = Vector3.new(0, 0, 1)
        assert(near(Loose.Axis, Vector3.new(0, 0, 1)) and near(Loose.SecondaryAxis, Vector3.new(0, 1, 0)))
        "#,
    );
}

/// Measured in Studio (2026-10-17, the capture Studio): an attachment hangs from its nearest part,
/// attachment or model ancestor past Folders, Configurations and Accessories; a model's frame is its
/// pivot, an empty Model's or a WorldModel's the identity.
#[test]
fn attachments_hang_past_folders() {
    check(
        r#"
        local function near(a, b) return (a - b).Magnitude < 1e-3 end
        local Root = Instance.new("Folder"); Root.Parent = workspace
        local Part = Instance.new("Part"); Part.Anchored = true
        Part.CFrame = CFrame.new(100, 50, 20) * CFrame.Angles(0, math.rad(90), 0); Part.Parent = Root
        local function Att(Parent, Name)
            local A = Instance.new("Attachment"); A.Name = Name; A.CFrame = CFrame.new(1, 2, 3); A.Parent = Parent; return A
        end
        local function In(Class, Parent) local I = Instance.new(Class); I.Parent = Parent; return I end
        local Folder = In("Folder", Part)
        local UnderFolder = Att(Folder, "UnderFolder")
        assert(near(UnderFolder.WorldPosition, Vector3.new(103, 52, 19)), tostring(UnderFolder.WorldPosition))
        assert(near(Att(In("Folder", Folder), "Two").WorldPosition, Vector3.new(103, 52, 19)))
        assert(near(Att(In("Configuration", Part), "Config").WorldPosition, Vector3.new(103, 52, 19)))
        assert(near(Att(In("Accessory", Part), "Accessory").WorldPosition, Vector3.new(103, 52, 19)))
        local Nested = Instance.new("Attachment"); Nested.CFrame = CFrame.new(0, 0, 5); Nested.Parent = UnderFolder
        assert(near(Nested.WorldPosition, Vector3.new(108, 52, 19)), tostring(Nested.WorldPosition))
        local Empty = Att(In("Model", Part), "EmptyModel")
        assert(near(Empty.WorldPosition, Vector3.new(1, 2, 3)), tostring(Empty.WorldPosition) .. " " .. tostring(Empty.Parent:GetPivot().Position))
        local InWorldModel = Att(In("WorldModel", Part), "WorldModel")
        assert(near(InWorldModel.WorldPosition, Vector3.new(1, 2, 3)), tostring(InWorldModel.WorldPosition))
        assert(near(Att(In("Folder", Root), "NoPart").WorldPosition, Vector3.new(1, 2, 3)))
        local Model = In("Model", Root)
        local Inside = Instance.new("Part"); Inside.Anchored = true; Inside.CFrame = CFrame.new(-40, 10, 0); Inside.Parent = Model
        local OnModel = Att(Model, "OnModel")
        assert(near(OnModel.WorldPosition, Vector3.new(-39, 12, 3)), tostring(OnModel.WorldPosition))
        Model.WorldPivot = CFrame.new(500, 0, 0)
        assert(near(OnModel.WorldPosition, Vector3.new(501, 2, 3)), tostring(OnModel.WorldPosition))
        assert(near(Att(In("Folder", Model), "FolderInModel").WorldPosition, Vector3.new(501, 2, 3)))
        Part.CFrame += Vector3.new(0, 100, 0)
        assert(near(UnderFolder.WorldPosition, Vector3.new(103, 152, 19)) and near(Nested.WorldPosition, Vector3.new(108, 152, 19)))
        UnderFolder.WorldPosition = Vector3.new(100, 150, 20)
        assert(near(UnderFolder.Position, Vector3.zero), tostring(UnderFolder.Position))
        "#,
    );
}

#[test]
fn data_model_run_service_is_the_service() {
    // the dump's DataModel.RunService property is the RunService (Studio 0.748), though the service is named "Run Service"
    check(
        r#"
        local RunService = game:GetService("RunService")
        assert(game.RunService == RunService and RunService.Name == "Run Service")
        assert(game:FindFirstChild("RunService") == nil and game["Run Service"] == RunService)
        "#,
    );
}

#[test]
fn workspace_terrain_and_service_names() {
    check(
        r#"
        assert(workspace.Terrain:IsA("Terrain") and workspace:FindFirstChildOfClass("Terrain") == workspace.Terrain and workspace.Terrain.Parent == workspace)
        assert(game["Run Service"] == game:GetService("RunService"))
        local Settings = UserSettings():GetService("UserGameSettings")
        assert(Settings.SavedQualityLevel == Enum.SavedQualitySetting.Automatic and Settings.SavedQualityLevel.Value == 0)
        assert(UserSettings():GetService("UserGameSettings") == Settings and Settings.Parent == UserSettings())
        assert(UserSettings().GameSettings == Settings and Settings.Name == "GameSettings")
        -- names as in Studio 0.748: the root is GlobalBasicSettings, and full names start below it, as below the DataModel
        local Root = UserSettings()
        assert(Root.Name == "GlobalBasicSettings" and tostring(Root) == "GlobalBasicSettings" and Root:GetFullName() == "GlobalBasicSettings")
        assert(Settings:GetFullName() == "GameSettings", Settings:GetFullName())
        assert(workspace.Terrain:GetFullName() == "Workspace.Terrain")
        local Loose = Instance.new("Folder")
        local Inner = Instance.new("Part")
        Inner.Parent = Loose
        assert(Inner:GetFullName() == "Folder.Part", Inner:GetFullName())
        -- what Roblox's PlayerModule calls on it
        Settings:SetCameraYInvertVisible()
        assert(Settings:GetCameraYInvertValue() == 1 and Settings:InFullScreen() == false)
        -- the camera's vertical multiplier is GetCameraYInvertValue, the number 1 by default; Studio 0.748 has no
        -- GetCameraYInvert
        local OldOk, OldError = pcall(function()
            return (Settings :: any).GetCameraYInvert
        end)
        assert(not OldOk and string.find(tostring(OldError), 'GetCameraYInvert is not a valid member of UserGameSettings "GameSettings"', 1, true), tostring(OldError))
        Settings.FullscreenChanged:Connect(function() end)
        -- deprecated CFrame members still answer
        local Frame = CFrame.new(1, 2, 3)
        assert(Frame.p == Frame.Position and Frame:inverse() == Frame:Inverse() and Frame.lookVector == Frame.LookVector)
        local Size = Vector2.new(3, 4)
        assert(Size.x == 3 and Size.y == 4 and Size.magnitude == 5 and Size.unit == Size.Unit and Size + Size == Vector2.new(6, 8))
        "#,
    );
}

#[test]
fn repeated_raycasts_see_what_changed_between_them() {
    check(
        r#"
        local Box = Instance.new("Part")
        Box.Anchored = true
        Box.Size = Vector3.new(4, 4, 4)
        Box.CFrame = CFrame.new(0, 2, 0)
        Box.Parent = workspace
        local function down(x) return workspace:Raycast(Vector3.new(x, 20, 0), Vector3.new(0, -40, 0)) end
        -- the same rays again find the same part
        assert(down(0).Instance == Box and down(0).Instance == Box and down(10) == nil)
        -- moved, between two rays of one frame
        Box.CFrame = CFrame.new(10, 2, 0)
        assert(down(0) == nil and down(10).Instance == Box)
        -- another parented, then CanQuery turned off
        local Other = Instance.new("Part")
        Other.Size = Vector3.new(4, 4, 4)
        Other.CFrame = CFrame.new(0, 2, 0)
        Other.Parent = workspace
        assert(down(0).Instance == Other)
        Other.CanQuery = false
        assert(down(0) == nil)
        -- unparented
        Box.Parent = nil
        assert(down(10) == nil)
        "#,
    );
}

#[test]
fn raycasts_hit_the_nearest_admitted_part() {
    check(
        r#"
        local function near(a, b) return math.abs(a - b) < 1e-3 end
        local Floor = Instance.new("Part")
        Floor.Name = "Floor"
        Floor.Anchored = true
        Floor.Size = Vector3.new(100, 2, 100)
        Floor.CFrame = CFrame.new(0, -1, 0)
        Floor.Parent = workspace
        local Box = Instance.new("Part")
        Box.Name = "Box"
        Box.Size = Vector3.new(4, 4, 4)
        Box.CFrame = CFrame.new(0, 2, 0) * CFrame.Angles(0, math.rad(45), 0)
        Box.Parent = workspace
        local Ball = Instance.new("Part")
        Ball.Name = "Ball"
        Ball.Shape = Enum.PartType.Ball
        Ball.Size = Vector3.new(4, 4, 4)
        Ball.CFrame = CFrame.new(20, 2, 0)
        Ball.Parent = workspace
        local Ghost = Instance.new("Part")
        Ghost.CanCollide = false
        Ghost.CanQuery = false
        Ghost.Size = Vector3.new(10, 1, 10)
        Ghost.CFrame = CFrame.new(0, 10, 0)
        Ghost.Parent = workspace

        -- down onto the box's top face (the CanQuery-off slab above is not hit)
        local Hit = workspace:Raycast(Vector3.new(0, 20, 0), Vector3.new(0, -40, 0))
        assert(typeof(Hit) == "RaycastResult" and Hit.Instance == Box, Hit and Hit.Instance.Name)
        assert(near(Hit.Position.Y, 4) and near(Hit.Normal.Y, 1) and near(Hit.Distance, 16))
        assert(Hit.Material == Enum.Material.Plastic)
        -- excluded, then only the floor included
        local Params = RaycastParams.new()
        Params.FilterDescendantsInstances = {Box}
        Hit = workspace:Raycast(Vector3.new(0, 20, 0), Vector3.new(0, -40, 0), Params)
        assert(Hit.Instance == Floor and near(Hit.Position.Y, 0))
        Params.FilterType = Enum.RaycastFilterType.Include
        Params.FilterDescendantsInstances = {Floor}
        assert(workspace:Raycast(Vector3.new(0, 20, 0), Vector3.new(0, -40, 0), Params).Instance == Floor)
        -- too short, and starting inside a part
        assert(workspace:Raycast(Vector3.new(0, 20, 0), Vector3.new(0, -10, 0)) == nil)
        assert(workspace:Raycast(Vector3.new(0, 2, 0), Vector3.new(0, 10, 0)) == nil)
        -- a ball by its sphere: from the side, through its surface
        Hit = workspace:Raycast(Vector3.new(30, 2, 0), Vector3.new(-20, 0, 0))
        assert(Hit.Instance == Ball and near(Hit.Position.X, 22) and near(Hit.Normal.X, 1), Hit and tostring(Hit.Position))
        -- a ray missing the ball's sphere but inside its box corner misses
        assert(workspace:Raycast(Vector3.new(30, 3.8, 1.8), Vector3.new(-20, 0, 0)) == nil)
        -- legacy FindPartOnRay
        local Part, Position, Normal = workspace:FindPartOnRayWithIgnoreList(Ray.new(Vector3.new(0, 20, 0), Vector3.new(0, -40, 0)), {Box})
        assert(Part == Floor and near(Position.Y, 0) and near(Normal.Y, 1))
        local Nothing, End = workspace:FindPartOnRay(Ray.new(Vector3.new(0, 50, 0), Vector3.new(0, 5, 0)))
        assert(Nothing == nil and near(End.Y, 55))
        "#,
    );
}

/// RaycastParams / OverlapParams:AddToFilter(Instance | {Instance}) adds to FilterDescendantsInstances
/// (documented; effect modules' shockwaves call it).
#[test]
fn add_to_filter_extends_the_filter() {
    check(
        r#"
        local function Part(Name, Y)
            local Made = Instance.new("Part")
            Made.Name, Made.Anchored, Made.Size, Made.CFrame = Name, true, Vector3.new(10, 1, 10), CFrame.new(0, Y, 0)
            Made.Parent = workspace
            return Made
        end
        local Top, Middle, Bottom = Part("Top", 6), Part("Middle", 3), Part("Bottom", 0)
        local Down = Vector3.new(0, -20, 0)
        local Params = RaycastParams.new()
        local Assigned = {}
        Params.FilterDescendantsInstances = Assigned
        Params:AddToFilter(Top)
        assert(#Assigned == 0, "AddToFilter changed the table the script assigned")
        assert(workspace:Raycast(Vector3.new(0, 10, 0), Down, Params).Instance == Middle, "one instance")
        Params:AddToFilter({Middle})
        local List = Params.FilterDescendantsInstances
        assert(#List == 2 and List[1] == Top and List[2] == Middle, "the list")
        assert(workspace:Raycast(Vector3.new(0, 10, 0), Down, Params).Instance == Bottom, "an array")
        assert(not pcall(function() Params:AddToFilter(5) end), "a number is not an instance")
        assert(not pcall(function() Params:AddToFilter({Bottom, 5}) end) and #Params.FilterDescendantsInstances == 2, "a table with an entry that is no Instance adds none")
        local Fresh = RaycastParams.new()
        Fresh:AddToFilter(Top)
        assert(#Fresh.FilterDescendantsInstances == 1 and #RaycastParams.new().FilterDescendantsInstances == 0, "a new object's list is its own")
        local Overlap = OverlapParams.new()
        Overlap:AddToFilter({Top, Bottom})
        local Parts = workspace:GetPartBoundsInBox(CFrame.new(0, 3, 0), Vector3.new(4, 10, 4), Overlap)
        assert(#Parts == 1 and Parts[1] == Middle, "OverlapParams")
        "#,
    );
}

/// RaycastParams / OverlapParams ExcludeInstances and IncludeInstances (`{Instance}?`, nil until
/// assigned; Roblox's spatial query filters of April 2026, devforum.roblox.com/t/4563575 and the
/// reference): the part, then each ancestor in turn, and the first either list names decides,
/// ExcludeInstances first; with none named IncludeInstances = nil lets a part through and `{}` does
/// not. Every query honours them; with FilterDescendantsInstances set too both filters apply (how
/// Roblox combines them is not documented: unmeasured). A list assigned or read back is a copy.
#[test]
fn exclude_and_include_instances_filter_every_query() {
    check(
        r#"
        local function near(a, b) return math.abs(a - b) < 1e-3 end
        local function Part(Name, Parent, Y)
            local Made = Instance.new("Part")
            Made.Name, Made.Anchored, Made.Size, Made.CFrame = Name, true, Vector3.new(10, 1, 10), CFrame.new(0, Y, 0)
            Made.Parent = Parent
            return Made
        end
        local Folder = Instance.new("Folder")
        Folder.Name = "Folder"
        Folder.Parent = workspace
        local Tree = Instance.new("Model")
        Tree.Name = "Tree"
        Tree.Parent = Folder
        local Leaf, Trunk, Ground = Part("Leaf", Tree, 9), Part("Trunk", Folder, 6), Part("Ground", workspace, 3)
        local function Seen(Params)
            local Names = {}
            for _, Found in workspace:GetPartBoundsInBox(CFrame.new(0, 6, 0), Vector3.new(4, 10, 4), Params) do
                table.insert(Names, Found.Name)
            end
            table.sort(Names)
            return table.concat(Names, ",")
        end

        local O = OverlapParams.new()
        assert(O.ExcludeInstances == nil and O.IncludeInstances == nil, "nil until assigned")
        assert(Seen(O) == "Ground,Leaf,Trunk", Seen(O))
        -- the announcement's "whitelist with exceptions": Folder's parts but Tree's
        O.IncludeInstances = {Folder}
        O.ExcludeInstances = {Tree}
        assert(Seen(O) == "Trunk", Seen(O))
        -- and "blacklist with exceptions": Folder left out, Tree inside it kept
        O.IncludeInstances = {Tree}
        O.ExcludeInstances = {Folder}
        assert(Seen(O) == "Leaf", Seen(O))
        -- the nearest listed ancestor decides (workspace out, a model in it in: the thread's example)
        O.ExcludeInstances = {workspace}
        assert(Seen(O) == "Leaf", Seen(O))
        -- listed in both: left out
        O.IncludeInstances = {Folder}
        O.ExcludeInstances = {Folder}
        assert(Seen(O) == "", Seen(O))
        -- {} includes nothing, nil everything; {} excludes nothing
        O.ExcludeInstances = nil
        O.IncludeInstances = {}
        assert(Seen(O) == "", Seen(O))
        O.IncludeInstances = nil
        O.ExcludeInstances = {Folder}
        assert(Seen(O) == "Ground", Seen(O))
        O.ExcludeInstances = {}
        assert(Seen(O) == "Ground,Leaf,Trunk", Seen(O))
        -- a part listed itself
        O.IncludeInstances = {workspace}
        O.ExcludeInstances = {Leaf}
        assert(Seen(O) == "Ground,Trunk", Seen(O))
        -- parts that arrive later under a listed instance follow it
        O.IncludeInstances = nil
        O.ExcludeInstances = {Tree}
        local Sprout = Part("Sprout", Tree, 7.5)
        assert(Seen(O) == "Ground,Trunk", Seen(O))
        Sprout.Parent = Folder
        assert(Seen(O) == "Ground,Sprout,Trunk", Seen(O))
        Sprout:Destroy()

        -- with FilterDescendantsInstances too, a part passes both filters
        O.ExcludeInstances = nil
        O.IncludeInstances = {Folder}
        O.FilterDescendantsInstances = {Trunk}
        assert(Seen(O) == "Leaf", Seen(O))
        O.FilterType = Enum.RaycastFilterType.Include
        O.FilterDescendantsInstances = {Folder}
        O.IncludeInstances = nil
        O.ExcludeInstances = {Tree}
        assert(Seen(O) == "Trunk", Seen(O))
        O.FilterType = Enum.RaycastFilterType.Exclude
        O.FilterDescendantsInstances = {}

        -- a table assigned is copied, and one read back is a copy
        local List = {Folder}
        O.ExcludeInstances = List
        table.insert(List, Ground)
        assert(Seen(O) == "Ground", "a table changed after it was assigned changed the filter")
        local Read = O.ExcludeInstances
        assert(#Read == 1 and Read[1] == Folder and Read ~= List and Read ~= O.ExcludeInstances, "a read gives a copy")
        table.insert(Read, Ground)
        assert(Seen(O) == "Ground", "a table read back changed the filter")
        O.ExcludeInstances = List
        assert(Seen(O) == "", "assigning the table again")
        O.ExcludeInstances = nil
        local Legacy = {Leaf}
        O.FilterDescendantsInstances = Legacy
        table.insert(Legacy, Trunk)
        assert(Seen(O) == "Ground,Trunk", "FilterDescendantsInstances is copied as well: " .. Seen(O))
        O.FilterDescendantsInstances = {}
        O.MaxParts = 1
        assert(#workspace:GetPartBoundsInBox(CFrame.new(0, 6, 0), Vector3.new(4, 10, 4), O) == 1, "MaxParts")
        O.MaxParts = 0

        -- the casts
        local Down = Vector3.new(0, -20, 0)
        local R = RaycastParams.new()
        R.ExcludeInstances = {Tree}
        assert(workspace:Raycast(Vector3.new(0, 15, 0), Down, R).Instance == Trunk, "Raycast")
        assert(workspace:Blockcast(CFrame.new(0, 15, 0), Vector3.new(1, 1, 1), Down, R).Instance == Trunk, "Blockcast")
        assert(workspace:Spherecast(Vector3.new(0, 15, 0), 0.5, Down, R).Instance == Trunk, "Spherecast")
        R.IncludeInstances = {Tree}
        R.ExcludeInstances = nil
        assert(workspace:Raycast(Vector3.new(0, 15, 0), Down, R).Instance == Leaf, "Raycast, included")
        R.IncludeInstances = {}
        assert(workspace:Raycast(Vector3.new(0, 15, 0), Down, R) == nil, "Raycast, nothing included")
        R.IncludeInstances = nil
        R.ExcludeInstances = {Tree}
        local Ball = Instance.new("Part")
        Ball.Shape = Enum.PartType.Ball
        Ball.Anchored, Ball.Size, Ball.CFrame = true, Vector3.new(1, 1, 1), CFrame.new(30, 15, 0)
        Ball.Parent = workspace
        local Hit = workspace:Shapecast(Ball, Down, R, CFrame.new(0, 15, 0))
        assert(Hit and Hit.Instance == Trunk and near(Hit.Distance, 8), "Shapecast")
        local Queried = Instance.new("Part")
        Queried.Anchored, Queried.Size, Queried.CFrame = true, Vector3.new(2, 10, 2), CFrame.new(0, 6, 0)
        Queried.Parent = workspace
        O.ExcludeInstances = {Folder}
        local Inside = workspace:GetPartsInPart(Queried, O)
        assert(#Inside == 1 and Inside[1] == Ground, "GetPartsInPart")
        O.ExcludeInstances = {Folder, Queried}
        local Near = workspace:GetPartBoundsInRadius(Vector3.new(0, 6, 0), 4, O)
        assert(#Near == 1 and Near[1] == Ground, "GetPartBoundsInRadius")
        "#,
    );
}

/// WorldRoot:Shapecast(part, direction, params?, cframeOverride?, sizeOverride?) (Roblox's
/// reference): the part's shape cast from where cframeOverride puts it and at sizeOverride's size,
/// else the part's own; the part itself is never hit, what it holds can be.
#[test]
fn shapecast_takes_a_cframe_and_a_size_override() {
    check(
        r#"
        local function near(a, b) return math.abs(a - b) < 1e-3 end
        local Floor = Instance.new("Part")
        Floor.Anchored, Floor.Size, Floor.CFrame = true, Vector3.new(50, 1, 50), CFrame.new(0, -0.5, 0)
        Floor.Parent = workspace
        local Down = Vector3.new(0, -20, 0)
        local Ball = Instance.new("Part")
        Ball.Shape = Enum.PartType.Ball
        Ball.Anchored, Ball.Size, Ball.CFrame = true, Vector3.new(1, 1, 1), CFrame.new(100, 10, 0)
        Ball.Parent = workspace
        assert(workspace:Shapecast(Ball, Down) == nil, "from where it is it reaches nothing")
        local Hit = workspace:Shapecast(Ball, Down, nil, CFrame.new(0, 10, 0))
        assert(Hit and Hit.Instance == Floor and near(Hit.Distance, 9.5), Hit and Hit.Distance)
        Hit = workspace:Shapecast(Ball, Down, nil, CFrame.new(0, 10, 0), Vector3.new(4, 4, 4))
        assert(Hit and Hit.Instance == Floor and near(Hit.Distance, 8), Hit and Hit.Distance)
        local Box = Instance.new("Part")
        Box.Anchored, Box.Size, Box.CFrame = true, Vector3.new(2, 2, 2), CFrame.new(100, 10, 20)
        Box.Parent = workspace
        Hit = workspace:Shapecast(Box, Down, nil, CFrame.new(0, 10, 0))
        assert(Hit and Hit.Instance == Floor and near(Hit.Distance, 9), Hit and Hit.Distance)
        Hit = workspace:Shapecast(Box, Down, nil, CFrame.new(0, 10, 0), Vector3.new(1, 6, 1))
        assert(Hit and Hit.Instance == Floor and near(Hit.Distance, 7), Hit and Hit.Distance)
        assert(Box.CFrame == CFrame.new(100, 10, 20) and Box.Size == Vector3.new(2, 2, 2), "the part stays as it was")
        -- a part the cast part holds is hit; the cast part never is
        local Held = Instance.new("Part")
        Held.Anchored, Held.Size, Held.CFrame = true, Vector3.new(4, 1, 4), CFrame.new(0, 4, 0)
        Held.Parent = Box
        Hit = workspace:Shapecast(Box, Down, nil, CFrame.new(0, 10, 0))
        assert(Hit and Hit.Instance == Held and near(Hit.Distance, 4.5), Hit and Hit.Instance.Name)
        Hit = workspace:Shapecast(Box, Vector3.new(0, 0, 40), nil, CFrame.new(100, 10, 0))
        assert(Hit == nil, "the cast part itself")
        "#,
    );
}

/// Bone.TransformedCFrame is CFrame * Transform; TransformedWorldCFrame chains it through parent bones
/// to the part; WorldCFrame leaves out the bone's own Transform but carries its parent bones' (measured
/// in Studio 2026-10-16), and an Attachment under a bone hangs from its TransformedWorldCFrame.
#[test]
fn a_bone_reports_its_transformed_cframes() {
    check(
        r#"
        local function near(a, b) return (a.Position - b.Position).Magnitude < 1e-4 and a.XVector:Dot(b.XVector) > 0.99999 and a.YVector:Dot(b.YVector) > 0.99999 end
        local Part = Instance.new("Part")
        Part.CFrame = CFrame.new(10, 0, 0) * CFrame.Angles(0, math.rad(90), 0)
        Part.Parent = workspace
        local Root = Instance.new("Bone")
        Root.CFrame = CFrame.new(0, 1, 0)
        Root.Parent = Part
        local Tip = Instance.new("Bone")
        Tip.CFrame = CFrame.new(0, 2, 0)
        Tip.Parent = Root
        assert(near(Tip.TransformedWorldCFrame, Part.CFrame * CFrame.new(0, 3, 0)), "at rest")
        local Bend = CFrame.Angles(0, 0, math.rad(90))
        Root.Transform = Bend
        Tip.Transform = CFrame.new(0, 0.5, 0)
        assert(near(Root.TransformedCFrame, Root.CFrame * Bend), "TransformedCFrame")
        assert(near(Root.TransformedWorldCFrame, Part.CFrame * Root.CFrame * Bend), "the root's TransformedWorldCFrame")
        assert(near(Tip.TransformedWorldCFrame, Part.CFrame * Root.CFrame * Bend * Tip.CFrame * Tip.Transform), "the tip carries its parent's Transform")
        assert(near(Root.WorldCFrame, Part.CFrame * Root.CFrame), "a bone's WorldCFrame leaves out its own Transform")
        assert(near(Tip.WorldCFrame, Part.CFrame * Root.CFrame * Bend * Tip.CFrame), "and carries its parent bone's")
        "#,
    );
}

/// Studio's own numbers (a Part at (10, 0, 0), bones at (0, 5, 0) and (0, 2, 0) under it, an
/// Attachment 3 studs along Z on the root and one 1 stud along X on the tip, the root's Transform 7
/// along Z and 90 degrees about Z, the tip's 11 along Y and 90 degrees about X): attachments follow
/// their bone's animation.
#[test]
fn attachments_follow_the_bone_they_hang_from() {
    check(
        r#"
        local function at(c, x, y, z) return (c.Position - Vector3.new(x, y, z)).Magnitude < 1e-3 end
        local Part = Instance.new("Part")
        Part.CFrame = CFrame.new(10, 0, 0)
        Part.Parent = workspace
        local Root = Instance.new("Bone")
        Root.CFrame = CFrame.new(0, 5, 0)
        Root.Parent = Part
        local Tip = Instance.new("Bone")
        Tip.CFrame = CFrame.new(0, 2, 0)
        Tip.Parent = Root
        local OnTip = Instance.new("Attachment")
        OnTip.CFrame = CFrame.new(1, 0, 0)
        OnTip.Parent = Tip
        local OnRoot = Instance.new("Attachment")
        OnRoot.CFrame = CFrame.new(0, 0, 3)
        OnRoot.Parent = Root
        Root.Transform = CFrame.new(0, 0, 7) * CFrame.Angles(0, 0, math.rad(90))
        Tip.Transform = CFrame.new(0, 11, 0) * CFrame.Angles(math.rad(90), 0, 0)
        assert(at(Root.WorldCFrame, 10, 5, 0), "root WorldCFrame")
        assert(at(Root.TransformedWorldCFrame, 10, 5, 7), "root TransformedWorldCFrame")
        assert(at(Tip.WorldCFrame, 8, 5, 7), "tip WorldCFrame")
        assert(at(Tip.TransformedWorldCFrame, -3, 5, 7), "tip TransformedWorldCFrame")
        assert(at(OnRoot.WorldCFrame, 10, 5, 10), "the root's attachment")
        assert(at(OnTip.WorldCFrame, -3, 6, 7), "the tip's attachment")
        assert(at(CFrame.new(OnTip.WorldPosition), -3, 6, 7), "WorldPosition alike")
        OnTip.WorldPosition = Vector3.new(-3, 6, 9)
        assert(at(OnTip.WorldCFrame, -3, 6, 9), "a world write lands where it was put")
        "#,
    );
}

#[test]
fn shape_casts_and_overlaps() {
    check(
        r#"
        local function near(a, b) return math.abs(a - b) < 1e-3 end
        local Wall = Instance.new("Part")
        Wall.Name = "Wall"
        Wall.Size = Vector3.new(2, 10, 10)
        Wall.CFrame = CFrame.new(10, 0, 0)
        Wall.Parent = workspace
        -- a 2-stud box moving +X touches the wall's face at x = 9 when its centre is at 8
        local Hit = workspace:Blockcast(CFrame.new(0, 0, 0), Vector3.new(2, 2, 2), Vector3.new(20, 0, 0))
        assert(Hit and Hit.Instance == Wall and near(Hit.Distance, 8) and near(Hit.Normal.X, -1), Hit and Hit.Distance)
        assert(near(Hit.Position.X, 9))
        -- a sphere of radius 1 the same
        Hit = workspace:Spherecast(Vector3.new(0, 0, 0), 1, Vector3.new(20, 0, 0))
        assert(Hit and Hit.Instance == Wall and near(Hit.Distance, 8) and near(Hit.Position.X, 9))
        -- a cast that starts overlapping does not hit
        assert(workspace:Blockcast(CFrame.new(9.5, 0, 0), Vector3.new(2, 2, 2), Vector3.new(5, 0, 0)) == nil)
        -- overlap queries
        local Found = workspace:GetPartBoundsInRadius(Vector3.new(7, 0, 0), 2.5)
        assert(#Found == 1 and Found[1] == Wall)
        assert(#workspace:GetPartBoundsInRadius(Vector3.new(5, 0, 0), 2.5) == 0)
        assert(#workspace:GetPartBoundsInBox(CFrame.new(8.5, 0, 0), Vector3.new(1.2, 1, 1)) == 1)
        assert(#workspace:GetPartBoundsInBox(CFrame.new(8, 0, 0), Vector3.new(1, 1, 1)) == 0)
        local Probe = Instance.new("Part")
        Probe.Size = Vector3.new(3, 3, 3)
        Probe.CFrame = CFrame.new(9, 0, 0)
        Probe.Parent = workspace
        local Inside = workspace:GetPartsInPart(Probe)
        assert(#Inside == 1 and Inside[1] == Wall)
        local Params = OverlapParams.new()
        Params.FilterDescendantsInstances = {Wall}
        assert(#workspace:GetPartsInPart(Probe, Params) == 0)
        "#,
    );
}

#[test]
fn camera_projections() {
    check(
        r#"
        local Camera = workspace.CurrentCamera
        Camera.FieldOfView = 90
        Camera.CFrame = CFrame.lookAt(Vector3.new(0, 0, 10), Vector3.zero)
        local Size = Camera.ViewportSize
        -- the point looked at is the viewport's centre, 10 studs deep
        local Centre, Visible = Camera:WorldToViewportPoint(Vector3.zero)
        assert(Visible and math.abs(Centre.X - Size.X / 2) < 1e-3 and math.abs(Centre.Y - Size.Y / 2) < 1e-3 and math.abs(Centre.Z - 10) < 1e-4, tostring(Centre))
        -- at 90 degrees, a point as far up as it is deep is the top edge
        local Top = Camera:WorldToViewportPoint(Vector3.new(0, 10, 0))
        assert(math.abs(Top.Y) < 1e-2, tostring(Top))
        local _, Behind = Camera:WorldToViewportPoint(Vector3.new(0, 0, 20))
        assert(not Behind)
        -- the ray through the centre points at the origin, starting on the near plane (0.1 studs ahead of the start camera) and
        -- `depth` studs further
        local Ray = Camera:ViewportPointToRay(Size.X / 2, Size.Y / 2, 2)
        assert((Ray.Direction - Vector3.new(0, 0, -1)).Magnitude < 1e-4 and (Ray.Origin - Vector3.new(0, 0, 7.9)).Magnitude < 1e-4, tostring(Ray.Origin))
        local Screen = Camera:ScreenPointToRay(Size.X / 2, Size.Y / 2)
        assert((Screen.Direction - Vector3.new(0, 0, -1)).Magnitude < 1e-4)
        "#,
    );
}

/// The screen forms are measured from below the top inset, as Roblox documents them: WorldToScreenPoint is
/// WorldToViewportPoint less the inset, and ScreenPointToRay(x, y) is ViewportPointToRay(x, y + inset).
#[test]
fn camera_screen_points_are_measured_from_below_the_top_inset() {
    let engine = Engine::new(Options { topbar_inset: 58.0, ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    run(
        &lua,
        &engine,
        r#"
        local Camera = workspace.CurrentCamera
        Camera.CFrame = CFrame.lookAt(Vector3.new(0, 10, 20), Vector3.zero)
        local Inset = game:GetService("GuiService"):GetGuiInset()
        assert(Inset == Vector2.new(0, 58), tostring(Inset))
        local Point = Vector3.new(3, 2, 0)
        local Viewport = Camera:WorldToViewportPoint(Point)
        local Screen, Visible = Camera:WorldToScreenPoint(Point)
        assert(Visible and math.abs(Screen.X - Viewport.X) < 1e-3 and math.abs(Screen.Z - Viewport.Z) < 1e-4, tostring(Screen))
        assert(math.abs(Screen.Y - (Viewport.Y - 58)) < 1e-3, `screen {Screen} for viewport {Viewport}`)
        local Toward = (Point - Camera.CFrame.Position).Unit
        local FromViewport = Camera:ViewportPointToRay(Viewport.X, Viewport.Y)
        assert((FromViewport.Direction - Toward).Magnitude < 1e-4, tostring(FromViewport.Direction))
        local FromScreen = Camera:ScreenPointToRay(Viewport.X, Viewport.Y - 58)
        assert((FromScreen.Direction - Toward).Magnitude < 1e-4, tostring(FromScreen.Direction))
        "#,
    );
}

/// Only the camera with the screen's viewport takes the top inset: a camera in its 1 x 1 viewport (in the Workspace or not)
/// projects its screen forms as its viewport forms (measured on a rendered desktop client under a 58-pixel inset, Studio 0.748,
/// tools/parity camera_fov_wide: these values to the bit), while the start camera keeps the inset.
#[test]
fn only_the_camera_with_the_screen_takes_the_top_inset() {
    let (lua, engine) = start_studio_window();
    run(
        &lua,
        &engine,
        r##"
        local function F(...)
            local T = table.pack(...)
            for I = 1, T.n do
                T[I] = string.format("%.9g", T[I])
            end
            return table.concat(T, " ")
        end
        local function Same(A, B)
            return A.Origin == B.Origin and A.Direction == B.Direction
        end
        local At = CFrame.lookAt(Vector3.new(200, 105, 230), Vector3.new(200, 105, 200))
        local W = Vector3.new(203, 107, 190)
        local Start = workspace.CurrentCamera
        Start.CFrame = At
        assert(Same(Start:ScreenPointToRay(100, 200, 2), Start:ViewportPointToRay(100, 258, 2)))
        local S, V = Start:WorldToScreenPoint(W), Start:WorldToViewportPoint(W)
        assert(S == Vector3.new(V.X, V.Y - 58, V.Z), `start: screen {S} for viewport {V}`)
        local New = Instance.new("Camera")
        New.Parent = workspace
        local Loose = Instance.new("Camera")
        for _, Cam in { New, Loose } do
            Cam.CFrame = At
            local R = Cam:ScreenPointToRay(0.5, 0.5, 2)
            assert(F(R.Origin.X, R.Origin.Y, R.Origin.Z, R.Direction.Z) == "200 105 227.5 -1", tostring(R))
            assert(Same(Cam:ScreenPointToRay(0.25, 0.75, 2), Cam:ViewportPointToRay(0.25, 0.75, 2)))
            local P, Seen = Cam:WorldToScreenPoint(W)
            assert(F(P.X, P.Y, P.Z) == "0.553555667 0.464296341 40" and Seen, F(P.X, P.Y, P.Z))
            assert(P == Cam:WorldToViewportPoint(W))
        end
        "##,
    );
}

/// The Camera's casts name a value as a playing client does (Studio 0.748, tools/parity camera_units, camera_cutoff,
/// bulk_move_args): an Instance "Instance", an EnumItem its enum, a function "Function", a signal "EventInstance", an Enum, Enums,
/// thread or plain userdata "null", in each float, int, Vector3, CoordinateFrame, CameraPanMode and Array cast; the rays check
/// their arguments from the last to the first.
#[test]
fn camera_casts_name_values_as_a_client_does() {
    check(
        r#"
        local function Fails(Message, F, ...)
            local Ok, E = pcall(F, ...)
            if Ok or tostring(E) ~= Message then
                error(`{Message}: got {Ok} {E}`, 2)
            end
        end
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        local A = CFrame.new()
        Fails("Unable to cast Instance to float", Cam.ScreenPointToRay, Cam, workspace, 1)
        Fails("Unable to cast KeyCode to float", Cam.ViewportPointToRay, Cam, 1, Enum.KeyCode.A)
        Fails("Unable to cast Instance to float", Cam.ScreenPointToRay, Cam, 1, 1, workspace)
        Fails("Unable to cast KeyCode to float", Cam.ViewportPointToRay, Cam, workspace, 1, Enum.KeyCode.A)
        Fails("Unable to cast Instance to float", Cam.ViewportPointToRay, Cam, "a", workspace)
        Fails("Argument 2 missing or nil", Cam.ScreenPointToRay, Cam, nil, nil, 1)
        Fails("Unable to cast CameraPanMode to float", Cam.SetRoll, Cam, Enum.CameraPanMode.EdgeBump)
        Fails("Unable to cast Instance to float", Cam.SetRoll, Cam, workspace)
        Fails("Unable to cast KeyCode to Vector3", Cam.WorldToScreenPoint, Cam, Enum.KeyCode.A)
        Fails("Unable to cast Instance to Vector3", Cam.WorldToViewportPoint, Cam, workspace)
        Fails("Unable to cast Instance to CameraPanMode", Cam.SetCameraPanMode, Cam, workspace)
        Fails("Unable to cast KeyCode to CameraPanMode", Cam.SetCameraPanMode, Cam, Enum.KeyCode.A)
        Fails("Unable to cast KeyCode to CoordinateFrame", Cam.Interpolate, Cam, Enum.KeyCode.A, A, 1)
        Fails("Unable to cast Instance to float", Cam.Interpolate, Cam, A, A, workspace)
        Fails("Unable to cast Instance to Vector3", Cam.ZoomToExtents, Cam, A, workspace)
        Fails("Unable to cast Instance to CoordinateFrame", Cam.ZoomToExtents, Cam, workspace, Vector3.one)
        Fails("Unable to cast Function to float", Cam.SetRoll, Cam, print)
        Fails("Unable to cast EventInstance to Vector3", Cam.WorldToScreenPoint, Cam, workspace.Changed)
        Fails("Unable to cast null to float", Cam.SetRoll, Cam, Enum.KeyCode)
        Fails("Unable to cast null to float", Cam.ScreenPointToRay, Cam, 1, Enum)
        Fails("Unable to cast null to int", Cam.PanUnits, Cam, coroutine.create(print))
        Fails("Unable to cast null to CameraPanMode", Cam.SetCameraPanMode, Cam, newproxy())
        Fails("Unable to cast Instance to Array", Cam.GetPartsObscuringTarget, Cam, workspace, {})
        Fails("Unable to cast KeyCode to Array", Cam.GetPartsObscuringTarget, Cam, Enum.KeyCode.A, {})
        Fails("Unable to cast Function to Array", workspace.BulkMoveTo, workspace, {}, print)
        -- other datatypes keep their own names
        Fails("Unable to cast Color3 to float", Cam.SetRoll, Cam, Color3.new())
        Fails("Unable to cast buffer to Vector3", Cam.WorldToScreenPoint, Cam, buffer.create(1))
        Fails("Unable to cast CoordinateFrame to Vector3", Cam.WorldToScreenPoint, Cam, A)
        "#,
    );
}

/// A client engine with Studio's measuring window (908.8 x 895.2) and its 58-pixel top inset.
fn start_studio_window() -> (Lua, Rc<Engine>) {
    let engine = Engine::new(Options { camera_width: Some(908.8), camera_height: Some(895.2), topbar_inset: 58.0, ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    (lua, engine)
}

/// Only the camera a run starts with has the viewport and a near plane 0.1 studs out; any other camera reads ViewportSize 1, 1
/// and NearPlaneZ -0.5, also once it is the CurrentCamera, and the start camera keeps its own (measured in a playing client and
/// its server, Studio 0.748). Both are read-only.
#[test]
fn only_the_start_camera_has_the_viewport_and_its_near_plane() {
    let (lua, engine) = start_studio_window();
    run(
        &lua,
        &engine,
        r##"
        local function Fails(Message, F)
            local Ok, E = pcall(F)
            assert(not Ok and string.gsub(tostring(E), "^[^\n]-:%d+: ", "") == Message, tostring(E))
        end
        local Start = workspace.CurrentCamera
        assert(Start.ViewportSize == Vector2.new(908.8, 895.2) and Start.NearPlaneZ == -0.10000000149011612, tostring(Start.ViewportSize))
        local New = Instance.new("Camera")
        New.Parent = workspace
        assert(New.ViewportSize == Vector2.new(1, 1) and New.NearPlaneZ == -0.5, tostring(New.ViewportSize))
        workspace.CurrentCamera = New
        assert(New.ViewportSize == Vector2.new(1, 1) and New.NearPlaneZ == -0.5 and Start.ViewportSize == Vector2.new(908.8, 895.2))
        workspace.CurrentCamera = Start
        Fails("Unable to assign property NearPlaneZ. Property is read only", function() (New :: any).NearPlaneZ = 1 end)
        Fails("Unable to assign property ViewportSize. Property is read only", function() (New :: any).ViewportSize = Vector2.one end)
        "##,
    );
}

/// The projections as a playing client computes them, to the bit (Studio 0.748, its start camera at (0, 500, 0) looking down -Z
/// with a 70-degree field of view in a 908.8 x 895.2 window under a 58-pixel inset): the viewport rounded to whole pixels, one
/// float projection-view matrix, rays from the near plane; a camera that is not current in its 1 x 1 viewport; the arguments.
#[test]
fn camera_projections_match_a_client_to_the_bit() {
    let (lua, engine) = start_studio_window();
    run(
        &lua,
        &engine,
        r##"
        local function F(...)
            local T = table.pack(...)
            for I = 1, T.n do
                T[I] = string.format("%.9g", T[I])
            end
            return table.concat(T, " ")
        end
        local function V(Vector) return F(Vector.X, Vector.Y, Vector.Z) end
        local function Fails(Message, Function, ...)
            local Ok, E = pcall(Function, ...)
            assert(not Ok and tostring(E) == Message, tostring(E))
        end
        local Camera = workspace.CurrentCamera
        Camera.CFrame = CFrame.lookAt(Vector3.new(0, 500, 0), Vector3.new(0, 500, -100))
        Camera.FieldOfView = 70
        local P = Camera:WorldToViewportPoint(Vector3.new(3, 502, -10))
        assert(V(P) == "646.228882 319.682251 10", V(P))
        P = Camera:WorldToViewportPoint(Vector3.new(-50, 470, -20))
        assert(V(P) == "-1143.24048 1406.14404 20", V(P))
        P = Camera:WorldToScreenPoint(Vector3.new(3, 502, -10))
        assert(V(P) == "646.228882 261.682251 10", V(P))
        local Here, Seen = Camera:WorldToViewportPoint(Vector3.new(0, 500, 0))
        assert(Here.X ~= Here.X and Here.Z == 0 and 1 / Here.Z > 0 and not Seen)
        local R = Camera:ViewportPointToRay(0, 0)
        assert(V(R.Origin) == "-0.07111606 500.070007 -0.100000009", V(R.Origin))
        assert(V(R.Direction) == "-0.50336504 0.495612472 -0.707807839", V(R.Direction))
        R = Camera:ViewportPointToRay(100, 250, 5)
        assert(V(R.Origin) == "-2.39680266 501.335327 -4.3209796", V(R.Origin))
        R = Camera:ScreenPointToRay(0, 0)
        assert(V(R.Origin) == "-0.07111606 500.060944 -0.100000009", V(R.Origin))
        assert(V(R.Direction) == "-0.519056022 0.444823593 -0.72987175", V(R.Direction))
        -- a camera that is not current projects into its 1 x 1 viewport, its rays from 0.5 studs out
        local New = Instance.new("Camera")
        New.CFrame = Camera.CFrame
        P = New:WorldToViewportPoint(Vector3.new(3, 502, -10))
        assert(F(P.X, P.Y) == "0.714222193 0.357186884", F(P.X, P.Y))
        R = New:ViewportPointToRay(0.5, 0.5)
        assert(R.Origin == Vector3.new(0, 500, -0.5) and R.Direction == Vector3.new(0, 0, -1), tostring(R))
        -- the arguments: y is checked first, a numeric string or true is taken
        Fails("Argument 1 missing or nil", Camera.WorldToViewportPoint, Camera)
        Fails("Unable to cast CoordinateFrame to Vector3", Camera.WorldToViewportPoint, Camera, CFrame.new())
        Fails("Argument 2 missing or nil", Camera.ViewportPointToRay, Camera, 1)
        Fails("Unable to cast string to float", Camera.ViewportPointToRay, Camera, "x", 1)
        Camera:ViewportPointToRay("5" :: any, true :: any)
        "##,
    );
}

#[test]
fn bulk_moves_and_reported_calls() {
    let (lua, engine) = start();
    let events: Rc<RefCell<Vec<WorldEvent>>> = Rc::default();
    let seen = events.clone();
    engine.hooks.borrow_mut().world_events.push(Box::new(move |_, event| seen.borrow_mut().push(event.clone())));
    run(
        &lua,
        &engine,
        r#"
        local Part = Instance.new("Part")
        workspace:BulkMoveTo({Part}, {CFrame.new(3, 4, 5)}, Enum.BulkMoveMode.FireCFrameChanged)
        assert(Part.Position == Vector3.new(3, 4, 5))
        local Emitter = Instance.new("ParticleEmitter")
        Emitter.Parent = Part
        Emitter:Emit(7)
        Emitter:Emit()
        Emitter:Clear()
        local Trail = Instance.new("Trail")
        Trail:Clear()
        local Sound = Instance.new("Sound")
        Sound:Play()
        assert(Sound.Playing)
        Sound:Stop()
        game:GetService("SoundService"):PlayLocalSound(Sound)
        "#,
    );
    let events = events.borrow();
    let kinds: Vec<String> = events
        .iter()
        .map(|event| match event {
            WorldEvent::Emit { count, .. } => format!("emit {count}"),
            WorldEvent::Clear { .. } => "clear".into(),
            WorldEvent::SoundPlayed { local, .. } => format!("play {local}"),
            WorldEvent::SoundStopped { .. } => "stop".into(),
        })
        .collect();
    assert_eq!(kinds, ["emit 7", "emit 16", "clear", "clear", "play false", "stop", "play true"]);
}

// Rigid assemblies (joints.rs)

/// Runs `source` in an engine of `mode` for `seconds` of frames; panics unless it finished cleanly.
fn check_running(mode: &str, source: &str, seconds: f64) {
    check_with(Options { mode: mode.into(), ..Options::default() }, source, seconds);
}

/// Runs `source` in an engine of `options` for `seconds` of frames; panics unless it finished cleanly.
fn check_with(options: Options, source: &str, seconds: f64) {
    let engine = Engine::new(options).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        crate::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(&lua, &engine, script).expect("environment");
    let prelude = "local function near(a, b) return (a.Position - b.Position).Magnitude < 1e-3 and a.XVector:Dot(b.XVector) > 0.99999 and a.YVector:Dot(b.YVector) > 0.99999 end\n";
    let function = lua.load(format!("{prelude}{source}")).set_name("=Test").set_environment(env).into_function().expect("compile");
    let call = engine_lua::globals::call(&lua, &engine, function, LuaMultiValue::new()).expect("call");
    crate::scheduler::run_until(&lua, &engine, seconds).expect("run");
    match call.outcome().unwrap() {
        CallOutcome::Failed(message, trace) => panic!("test script failed: {message}\n{}", trace.unwrap_or_default()),
        CallOutcome::Pending => panic!("test script did not finish"),
        CallOutcome::Finished(_) => {}
    }
}

#[test]
fn a_weld_carries_its_part() {
    check_running(
        "play",
        r#"
        local A = Instance.new("Part")
        A.Anchored = true
        A.CFrame = CFrame.new(0, 5, 0)
        A.Parent = workspace
        local B = Instance.new("Part")
        B.CFrame = CFrame.new(50, 0, 0)
        B.Parent = workspace
        local Weld = Instance.new("Weld")
        Weld.Part0, Weld.Part1 = A, B
        Weld.C0 = CFrame.new(0, 2, 0)
        Weld.C1 = CFrame.Angles(0, math.rad(90), 0)
        -- outside the Workspace a joint links nothing
        assert(near(B.CFrame, CFrame.new(50, 0, 0)), "inactive weld moved its part")
        Weld.Parent = A
        -- active: the root (A, anchored) stays and B goes where the weld puts it
        assert(near(A.CFrame, CFrame.new(0, 5, 0)), "the root moved")
        assert(near(B.CFrame, A.CFrame * Weld.C0 * Weld.C1:Inverse()), "B did not join A")
        -- moving A carries B, without Changed events for B
        local Fired = 0
        B:GetPropertyChangedSignal("CFrame"):Connect(function() Fired += 1 end)
        A.CFrame = CFrame.new(10, 0, 0) * CFrame.Angles(0, math.rad(45), 0)
        assert(near(B.CFrame, A.CFrame * Weld.C0 * Weld.C1:Inverse()), "B did not follow A")
        task.wait()
        assert(Fired == 0, "a carried part fired CFrame changes")
        -- moving B carries the whole assembly, its anchored root too
        B.CFrame = CFrame.new(-20, 3, 0)
        assert(near(B.CFrame, CFrame.new(-20, 3, 0)), "B is not where it was set")
        assert(near(A.CFrame, B.CFrame * Weld.C1 * Weld.C0:Inverse()), "A did not follow B")
        -- C0 changes: the root stays, the rest moves
        local Root = A.CFrame
        Weld.C0 = CFrame.new(0, 0, -4)
        assert(near(A.CFrame, Root), "the root moved for C0")
        assert(near(B.CFrame, Root * Weld.C0 * Weld.C1:Inverse()), "B did not take the new C0")
        -- Position moves B alone: the weld's C1 is rewritten to keep it there
        B.Position = Vector3.new(0, 30, 0)
        assert(near(A.CFrame, Root), "Position moved the root")
        assert(B.Position == Vector3.new(0, 30, 0), "Position did not move B")
        assert(near(B.CFrame, A.CFrame * Weld.C0 * Weld.C1:Inverse()), "C1 does not hold B where it is")
        A.CFrame = A.CFrame + Vector3.new(0, 1, 0)
        assert(near(B.CFrame, CFrame.new(0, 31, 0) * B.CFrame.Rotation), "B did not keep its new place")
        -- disabled or unparented: no link
        Weld.Enabled = false
        local Rest = B.CFrame
        A.CFrame = CFrame.new(100, 0, 0)
        assert(near(B.CFrame, Rest), "a disabled weld carried its part")
        Weld.Enabled = true
        assert(near(B.CFrame, A.CFrame * Weld.C0 * Weld.C1:Inverse()), "re-enabling did not join B")
        Weld.Parent = nil
        A.CFrame = CFrame.new(0, 0, 0)
        assert(near(B.CFrame, CFrame.new(100, 0, 0) * Weld.C0 * Weld.C1:Inverse()), "an unparented weld carried its part")
        "#,
        0.2,
    );
}

#[test]
fn a_motor6d_applies_its_transform_after_presimulation() {
    check_running(
        "play",
        r#"
        local RunService = game:GetService("RunService")
        local Torso = Instance.new("Part")
        Torso.Name = "Torso"
        Torso.Anchored = true
        Torso.CFrame = CFrame.new(0, 3, 0)
        Torso.Parent = workspace
        local Arm = Instance.new("Part")
        Arm.Size = Vector3.new(1, 2, 1)
        Arm.Parent = workspace
        local Motor = Instance.new("Motor6D")
        Motor.Part0, Motor.Part1 = Torso, Arm
        Motor.C0 = CFrame.new(1.5, 0.5, 0)
        Motor.C1 = CFrame.new(0, 0.5, 0)
        Motor.Parent = Torso
        local Rest = Torso.CFrame * Motor.C0 * Motor.C1:Inverse()
        assert(near(Arm.CFrame, Rest), "the arm did not join the torso")
        task.wait()
        local Pose = CFrame.Angles(math.rad(90), 0, 0)
        local Posed = Torso.CFrame * Motor.C0 * Pose * Motor.C1:Inverse()
        Motor.Transform = Pose
        -- Transform waits for the simulation step (C0 and C1 apply at once)
        assert(near(Arm.CFrame, Rest), "Transform applied before the simulation step")
        local AtPre, AtPost
        local Pre, Post
        Pre = RunService.PreSimulation:Connect(function() AtPre = Arm.CFrame Pre:Disconnect() end)
        Post = RunService.PostSimulation:Connect(function() AtPost = Arm.CFrame Post:Disconnect() end)
        task.wait()
        assert(AtPre and near(AtPre, Rest), "PreSimulation saw the new pose")
        assert(AtPost and near(AtPost, Posed), "PostSimulation did not see the pose")
        assert(near(Arm.CFrame, Posed), "the pose did not stay")
        -- the torso moving carries the posed arm
        Torso.CFrame = CFrame.new(5, 3, 5)
        assert(near(Arm.CFrame, Torso.CFrame * Motor.C0 * Pose * Motor.C1:Inverse()), "the posed arm did not follow")
        "#,
        0.3,
    );
}

#[test]
fn a_motor6d_transform_applies_in_edit_mode_too() {
    check_running(
        "edit",
        r#"
        local Root = Instance.new("Part")
        Root.Anchored = true
        Root.Parent = workspace
        local Limb = Instance.new("Part")
        Limb.Parent = workspace
        local Motor = Instance.new("Motor6D")
        Motor.Part0, Motor.Part1 = Root, Limb
        Motor.C0 = CFrame.new(0, 2, 0)
        Motor.Parent = Root
        Motor.Transform = CFrame.Angles(0, 0, math.rad(30))
        task.wait()
        assert(near(Limb.CFrame, Root.CFrame * Motor.C0 * Motor.Transform), "edit mode did not apply Transform")
        "#,
        0.2,
    );
}

#[test]
fn a_weld_constraint_keeps_where_it_linked_its_parts() {
    check_running(
        "play",
        r#"
        local A = Instance.new("Part")
        A.Anchored = true
        A.Parent = workspace
        local B = Instance.new("Part")
        B.CFrame = CFrame.new(3, 1, 0) * CFrame.Angles(0, math.rad(30), 0)
        B.Parent = workspace
        local Offset = A.CFrame:ToObjectSpace(B.CFrame)
        local Weld = Instance.new("WeldConstraint")
        Weld.Part0, Weld.Part1 = A, B
        Weld.Parent = A
        -- linking moves nothing; then they move together
        assert(near(B.CFrame, CFrame.new(3, 1, 0) * CFrame.Angles(0, math.rad(30), 0)), "linking moved B")
        A.CFrame = CFrame.new(0, 10, 0) * CFrame.Angles(math.rad(20), 0, 0)
        assert(near(B.CFrame, A.CFrame * Offset), "B did not follow A")
        B.CFrame = CFrame.new(7, 7, 7)
        assert(near(A.CFrame, B.CFrame * Offset:Inverse()), "A did not follow B")
        -- Position moves a part alone and the weld takes the new offset
        B.Position = B.Position + Vector3.new(0, 0, 5)
        local Moved = A.CFrame:ToObjectSpace(B.CFrame)
        A.CFrame = CFrame.new(-4, 0, 0)
        assert(near(B.CFrame, A.CFrame * Moved), "B did not keep its new offset")
        -- Part1 set again: links where B is now
        Weld.Part1 = nil
        B.CFrame = CFrame.new(0, 50, 0)
        Weld.Part1 = B
        A.CFrame = CFrame.new(-4, 1, 0)
        assert(near(B.CFrame, CFrame.new(0, 51, 0)), "relinking did not keep B where it was")

        -- a clone keeps its welds' offsets: moved by its root out of the Workspace, it rejoins it in
        local Model = Instance.new("Model")
        local Root = Instance.new("Part")
        Root.Name = "Root"
        Root.Anchored = true
        Root.Parent = Model
        local Handle = Instance.new("Part")
        Handle.Name = "Handle"
        Handle.CFrame = CFrame.new(0, 4, 0)
        Handle.Parent = Model
        local Link = Instance.new("WeldConstraint")
        Link.Part0, Link.Part1 = Root, Handle
        Link.Parent = Handle
        local Copy = Model:Clone()
        Copy.Root.CFrame = CFrame.new(20, 0, 0)
        assert(near(Copy.Handle.CFrame, CFrame.new(0, 4, 0)), "an inactive weld carried its part")
        Copy.Parent = workspace
        assert(near(Copy.Handle.CFrame, CFrame.new(20, 4, 0)), "the clone's handle did not rejoin its root")
        "#,
        0.2,
    );
}

#[test]
fn a_rig_follows_its_humanoid_root_part() {
    check_running(
        "play",
        r#"
        -- An R6-like rig, nothing anchored: HumanoidRootPart is the root (its 10x size weight)
        local Rig = Instance.new("Model")
        local function Part(Name, Size, At)
            local Made = Instance.new("Part")
            Made.Name, Made.Size, Made.CFrame = Name, Size, At
            Made.Parent = Rig
            return Made
        end
        local Root = Part("HumanoidRootPart", Vector3.new(2, 2, 1), CFrame.new(0, 3, 0))
        local Torso = Part("Torso", Vector3.new(2, 2, 1), CFrame.new(0, 3, 0))
        local Head = Part("Head", Vector3.new(2, 1, 1), CFrame.new(0, 4.5, 0))
        local Arm = Part("Right Arm", Vector3.new(1, 2, 1), CFrame.new(1.5, 3, 0))
        local Hat = Part("Handle", Vector3.new(3, 1, 3), CFrame.new(0, 5.5, 0))
        local function Motor(Name, Part0, Part1, C0, C1)
            local Made = Instance.new("Motor6D")
            Made.Name, Made.Part0, Made.Part1, Made.C0, Made.C1 = Name, Part0, Part1, C0, C1
            Made.Parent = Part0
            return Made
        end
        local RootJoint = Motor("RootJoint", Root, Torso, CFrame.identity, CFrame.identity)
        local Neck = Motor("Neck", Torso, Head, CFrame.new(0, 1, 0), CFrame.new(0, -0.5, 0))
        Motor("Right Shoulder", Torso, Arm, CFrame.new(1, 0.5, 0), CFrame.new(-0.5, 0.5, 0))
        local HatWeld = Instance.new("WeldConstraint")
        HatWeld.Part0, HatWeld.Part1 = Head, Hat
        HatWeld.Parent = Hat
        Rig.PrimaryPart = Root
        Rig.Parent = workspace
        -- a consistent rig does not move when it links up
        assert(near(Head.CFrame, CFrame.new(0, 4.5, 0)) and near(Arm.CFrame, CFrame.new(1.5, 3, 0)), "the rig moved when it linked")

        -- the client moves the root (CFrame, BulkMoveTo): the body comes along
        local Goal = CFrame.new(100, 3, 50) * CFrame.Angles(0, math.rad(120), 0)
        Root.CFrame = Goal
        assert(near(Torso.CFrame, Goal), "the torso stayed")
        assert(near(Head.CFrame, Goal * CFrame.new(0, 1.5, 0)), "the head stayed")
        assert(near(Arm.CFrame, Goal * CFrame.new(1.5, 0, 0)), "the arm stayed")
        assert(near(Hat.CFrame, Goal * CFrame.new(0, 2.5, 0)), "the hat stayed")
        workspace:BulkMoveTo({Root}, {CFrame.new(-30, 3, 0)}, Enum.BulkMoveMode.FireCFrameChanged)
        assert(near(Head.CFrame, CFrame.new(-30, 4.5, 0)) and near(Hat.CFrame, CFrame.new(-30, 5.5, 0)), "BulkMoveTo left the body")

        -- moving a limb moves everything (it is one assembly)
        Arm.CFrame = Arm.CFrame + Vector3.new(0, 10, 0)
        assert(near(Root.CFrame, CFrame.new(-30, 13, 0)), "the root did not follow the arm")

        -- Model:PivotTo moves the rig as one
        Rig:PivotTo(CFrame.new(7, 3, 7))
        assert(near(Root.CFrame, CFrame.new(7, 3, 7)) and near(Head.CFrame, CFrame.new(7, 4.5, 7)), "PivotTo split the rig")

        -- a joint's C0 moves what hangs below it, never the root
        Neck.C0 = CFrame.new(0, 2, 0)
        assert(near(Root.CFrame, CFrame.new(7, 3, 7)), "the root moved for the neck")
        assert(near(Head.CFrame, CFrame.new(7, 5.5, 7)) and near(Hat.CFrame, CFrame.new(7, 6.5, 7)), "the head did not take the neck's C0")

        -- RootPriority picks another root: the RootJoint's C0 now moves the HumanoidRootPart
        Torso.RootPriority = 1
        RootJoint.C0 = CFrame.new(0, -1, 0)
        assert(near(Torso.CFrame, CFrame.new(7, 3, 7)), "the torso (now the root) moved")
        assert(near(Root.CFrame, CFrame.new(7, 4, 7)), "the old root did not move")

        -- ScaleTo scales the joints with the parts, so the rig stays joined
        Torso.RootPriority = 0
        RootJoint.C0 = CFrame.identity
        Rig:ScaleTo(2)
        assert(near(Head.CFrame, Root.CFrame * CFrame.new(0, 5, 0)), "ScaleTo broke the neck")
        assert(near(Hat.CFrame, Root.CFrame * CFrame.new(0, 7, 0)), "ScaleTo broke the hat's weld")
        Root.CFrame = CFrame.new(0, 6, 0)
        assert(near(Hat.CFrame, CFrame.new(0, 13, 0)), "the scaled hat did not follow")
        "#,
        0.2,
    );
}

#[test]
fn an_anchored_root_stays_and_anchored_parts_do_not_link() {
    check_running(
        "play",
        r#"
        -- a small anchored part is the root over a big unanchored one
        local Base = Instance.new("Part")
        Base.Size = Vector3.new(1, 1, 1)
        Base.Anchored = true
        Base.CFrame = CFrame.new(0, 0, 0)
        Base.Parent = workspace
        local Slab = Instance.new("Part")
        Slab.Size = Vector3.new(40, 1, 40)
        Slab.CFrame = CFrame.new(0, 5, 0)
        Slab.Parent = workspace
        local Weld = Instance.new("Weld")
        Weld.Part0, Weld.Part1 = Slab, Base
        Weld.C0 = CFrame.new(0, -5, 0)
        Weld.Parent = Slab
        Weld.C0 = CFrame.new(0, -8, 0)
        assert(near(Base.CFrame, CFrame.new(0, 0, 0)), "the anchored root moved")
        assert(near(Slab.CFrame, CFrame.new(0, 8, 0)), "the slab did not move")
        -- two anchored parts: the weld between them does nothing
        Slab.Anchored = true
        Base.CFrame = CFrame.new(0, -3, 0)
        assert(near(Slab.CFrame, CFrame.new(0, 8, 0)), "an anchored part followed another")
        -- unanchored again: the weld links them, the slab goes where it puts it from its anchored root
        Slab.Anchored = false
        assert(near(Base.CFrame, CFrame.new(0, -3, 0)), "the anchored root moved when the weld linked")
        assert(near(Slab.CFrame, CFrame.new(0, 5, 0)), "the slab did not rejoin its root")
        Base.CFrame = CFrame.new(0, 0, 0)
        assert(near(Slab.CFrame, CFrame.new(0, 8, 0)), "the slab did not follow its root")
        "#,
        0.2,
    );
}

/// A pose landing while the rig moves leaves its anchored root where the moves put it: follower
/// rigs (anchored HumanoidRootPart, Model:PivotTo each frame) whose idle pose was set
/// a defer after they were built lay on their side.
/// BasePart.AssemblyRootPart picks the root as a Studio 0.746 playing client does (each line its answer): an anchored part,
/// else not Massless, the higher RootPriority, the larger largest face (not rounded) times a Seat's, a HumanoidRootPart's
/// or a Torso's weight, then the later made part; nil outside a WorldRoot.
#[test]
fn assembly_root_part_is_the_root_a_playing_client_picks() {
    check(
        r#"
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        local function Part(Name, Size, Props, Parent)
            local P = Instance.new((Props and Props.Class) or "Part")
            P.Name = Name
            P.Size = Size
            P.CanCollide = false
            for Key, Value in Props or {} do
                if Key ~= "Class" then
                    P[Key] = Value
                end
            end
            P.Parent = Parent or Folder
            return P
        end
        local function Join(Kind, A, B)
            local J = Instance.new(Kind)
            J.Part0, J.Part1 = A, B
            J.Parent = A
            return J
        end
        local function Root(P)
            local R = P.AssemblyRootPart
            return R and R.Name or "nil"
        end
        local function Is(Case, A, B, Want)
            local Got = Root(A) .. " " .. Root(B)
            assert(Got == Want .. " " .. Want, `{Case}: {Got}, the client's {Want}`)
        end
        local Two = Vector3.new(2, 2, 2)
        -- equal parts: the later made one, whatever the joint's kind and direction
        for _, Kind in { "Weld", "WeldConstraint", "Motor6D" } do
            local A, B = Part("A", Two), Part("B", Two)
            Join(Kind, A, B)
            Is(Kind, A, B, "B")
            local C, D = Part("C", Two), Part("D", Two)
            Join(Kind, D, C)
            Is(Kind .. " the other way", C, D, "D")
        end
        -- the size: the largest face's area, not rounded
        do
            local A, B = Part("Cube4", Vector3.new(4, 4, 4)), Part("Rod10", Vector3.new(10, 1, 1))
            Join("Weld", B, A)
            Is("a cube and a longer rod", A, B, "Cube4")
            local C, D = Part("Cube3", Vector3.new(3, 3, 3)), Part("Slab", Vector3.new(4, 4, 0.5))
            Join("Weld", C, D)
            Is("a cube and a wider slab", C, D, "Slab")
            local E, F = Part("Older", Vector3.new(4, 4.0001, 1)), Part("Newer", Vector3.new(4, 4, 1))
            Join("Weld", E, F)
            Is("a face 0.0004 larger", E, F, "Older")
        end
        -- Massless, RootPriority, then the names' and seats' weights (each at the plate it beats and the one it ties)
        do
            local A, B = Part("Big", Vector3.new(6, 6, 6), { Massless = true }), Part("Small", Two)
            Join("Weld", A, B)
            Is("massless", A, B, "Small")
            local C, D = Part("Big", Vector3.new(6, 6, 6)), Part("Small", Two, { RootPriority = 1 })
            Join("Weld", C, D)
            Is("root priority", C, D, "Small")
        end
        for _, Case in {
            { "HumanoidRootPart", "Part", 9.9, "HumanoidRootPart" },
            { "HumanoidRootPart", "Part", 10, "Plate" },
            { "Torso", "Part", 4.9, "Torso" },
            { "Torso", "Part", 5, "Plate" },
            { "Seat", "Seat", 19.9, "Seat" },
            { "VehicleSeat", "VehicleSeat", 20, "Plate" },
            { "Head", "Part", 1.1, "Plate" },
        } do
            local A, B = Part(Case[1], Vector3.new(1, 1, 1), { Class = Case[2] }), Part("Plate", Vector3.new(Case[3], 1, 0.1))
            Join("Weld", A, B)
            Is(`{Case[1]} and a plate of {Case[3]}`, A, B, Case[4])
        end
        -- anchored parts are their own roots
        do
            local A, B = Part("Big", Vector3.new(6, 6, 6)), Part("Small", Two, { Anchored = true })
            Join("Weld", A, B)
            Is("an anchored part", A, B, "Small")
            local C, D = Part("C", Two, { Anchored = true }), Part("D", Vector3.new(4, 4, 4), { Anchored = true })
            Join("Weld", C, D)
            assert(Root(C) == "C" and Root(D) == "D", "two anchored parts")
        end
        -- a thrown rock and the glow cloned onto it: the glow while free, the rock while held
        do
            local Rock = Part("Rock", Vector3.new(1, 1, 1))
            local Glow = Rock:Clone()
            Glow.Name = "Glow"
            local W = Instance.new("Weld")
            W.Part0, W.Part1 = Glow, Rock
            W.Parent = Glow
            Glow.Parent = Rock
            Is("the glow", Rock, Glow, "Glow")
            Rock.Anchored = true
            Is("the rock held", Rock, Glow, "Rock")
            Rock.Anchored = false
            Is("the rock let go", Rock, Glow, "Glow")
        end
        -- outside a WorldRoot: nil; a disabled weld joins nothing
        do
            local Storage = game:GetService("ReplicatedStorage")
            local A, B = Part("A", Two, nil, Storage), Part("B", Vector3.new(4, 4, 4), nil, Storage)
            Join("Weld", A, B)
            assert(A.AssemblyRootPart == nil and B.AssemblyRootPart == nil and Instance.new("Part").AssemblyRootPart == nil, "outside the Workspace")
            local C, D = Part("C", Two), Part("D", Vector3.new(4, 4, 4))
            Join("Weld", C, D).Enabled = false
            assert(Root(C) == "C" and Root(D) == "D", "a disabled weld")
        end
        "#,
    );
}

/// Parts link only within one world as they move between worlds: a weld joins two parts in the Workspace, then neither when
/// one is in a WorldModel, both again once the WorldModel holds both (wherever the WorldModel goes), the same from
/// JointsService, nothing outside every world, and the same with the parts and the WorldModel at the bottom of chains of
/// folders 3,000 deep moved in and out whole.
#[test]
fn assemblies_link_within_one_world_as_their_parts_move_between_worlds() {
    check(
        r#"
        local function Part(Name, Size)
            local P = Instance.new("Part")
            P.Name, P.Size, P.CanCollide = Name, Size, false
            return P
        end
        local function Root(P)
            local R = P.AssemblyRootPart
            return R and R.Name or "nil"
        end
        local function Is(Case, A, B, Want)
            local Got = Root(A) .. " " .. Root(B)
            assert(Got == Want, `{Case}: {Got} where {Want}`)
        end
        -- (a chain of folders `Depth` deep holding `Inner`, made from the bottom up)
        local function Wrap(Inner, Depth)
            local Outer = Inner
            for _ = 1, Depth do
                local Folder = Instance.new("Folder")
                Outer.Parent = Folder
                Outer = Folder
            end
            return Outer
        end
        for _, Depth in { 0, 3000 } do
            local A, B = Part("A", Vector3.new(2, 2, 2)), Part("B", Vector3.new(4, 4, 4))
            local Weld = Instance.new("Weld")
            Weld.Part0, Weld.Part1 = A, B
            Weld.Parent = A
            local BoxA, BoxB = Instance.new("Folder"), Instance.new("Folder")
            A.Parent, B.Parent = BoxA, BoxB
            local TopA, TopB = Wrap(BoxA, Depth), Wrap(BoxB, Depth)
            TopA.Parent, TopB.Parent = workspace, workspace
            Is(`{Depth}: both in the Workspace`, A, B, "B B")
            local World = Instance.new("WorldModel")
            local TopWorld = Wrap(World, Depth)
            TopWorld.Parent = workspace
            B.Parent = World
            Is(`{Depth}: B in a WorldModel`, A, B, "A B")
            A.Parent = World
            Is(`{Depth}: both in the WorldModel`, A, B, "B B")
            Weld.Parent = game:GetService("JointsService")
            Is(`{Depth}: the weld in JointsService`, A, B, "B B")
            -- (a WorldModel is a world wherever it is)
            TopWorld.Parent = game:GetService("ReplicatedStorage")
            Is(`{Depth}: the WorldModel out of the Workspace`, A, B, "B B")
            TopWorld.Parent = nil
            Is(`{Depth}: the WorldModel out of the game`, A, B, "B B")
            local Loose = Instance.new("Folder")
            Wrap(Loose, Depth)
            A.Parent, B.Parent = Loose, Loose
            Is(`{Depth}: outside every world`, A, B, "nil nil")
            Weld.Parent = A
            A.Parent, B.Parent = BoxA, BoxB
            Is(`{Depth}: back in their boxes`, A, B, "B B")
            TopB.Parent = nil
            Is(`{Depth}: B's chain out`, A, B, "A nil")
            TopB.Parent = TopA
            Is(`{Depth}: B's chain under A's`, A, B, "B B")
            TopA.Parent = game:GetService("ReplicatedStorage")
            Is(`{Depth}: both chains out`, A, B, "nil nil")
            TopA.Parent = workspace
            Is(`{Depth}: both chains back`, A, B, "B B")
        end
        "#,
    );
}

/// An assembly's lookups (AssemblyRootPart, AssemblyMass, AssemblyCenterOfMass, a velocity write, a Position write, a CFrame
/// write moving the assembly) read which world each part and joint is in from the homes the dom keeps (dom.rs Dom::home),
/// never by walking up the tree: a part welded to 20 parts at the bottom of a chain of 10,000 folders, or each 4,000 deep in a
/// chain of its own, costs about as much host work as the same assembly in the Workspace, and the work clock charges each the
/// same, as a playing client's reads cost the same at any depth (Studio 0.747: 0.04 us an AssemblyRootPart read with 1 to 1,000
/// joints, its parts 0 to 1,000 folders deep). Each lookup used to walk up from every part and joint it met: 800,000 steps a
/// lookup in the first scene.
#[test]
fn assembly_lookups_cost_the_same_however_deep_their_parts_are() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        -- (a chain of folders `Depth` deep holding `Inner`, made from the bottom up so that making it walks nothing)
        local function Wrap(Inner, Depth)
            local Outer = Inner
            for _ = 1, Depth do
                local Folder = Instance.new("Folder")
                Outer.Parent = Folder
                Outer = Folder
            end
            return Outer
        end
        local function Scene(X, Depth, Apart)
            local Top = Instance.new("Folder")
            local Hub = Instance.new("Part")
            Hub.Name, Hub.Size, Hub.CanCollide, Hub.CFrame = "Hub", Vector3.new(4, 4, 4), false, CFrame.new(X, 500, 0)
            Hub.Parent = Top
            local Bottom = Instance.new("Folder")
            local Leaves = {}
            for I = 1, 20 do
                local Leaf = Instance.new("Part")
                Leaf.Size, Leaf.CanCollide, Leaf.CFrame = Vector3.one, false, CFrame.new(X, 503, 0)
                local Weld = Instance.new("Weld")
                Weld.Part0, Weld.Part1, Weld.C0 = Hub, Leaf, CFrame.new(0, 3, 0)
                Weld.Parent = Leaf
                if Apart then
                    local Box = Instance.new("Folder")
                    Leaf.Parent = Box
                    Wrap(Box, Depth).Parent = Top
                else
                    Leaf.Parent = Bottom
                end
                Leaves[I] = Leaf
            end
            if not Apart then
                Wrap(Bottom, Depth).Parent = Top
            end
            Top.Parent = workspace
            return { Hub = Hub, Leaf = Leaves[20] }
        end
        shared.Scenes = { Shallow = Scene(0, 0, false), Deep = Scene(100, 10000, false), Apart = Scene(200, 4000, true) }
        shared.Work = {}
        local function Pass(S)
            local Hub, Leaf = S.Hub, S.Leaf
            for I = 1, 200 do
                assert(Leaf.AssemblyRootPart == Hub and Hub.AssemblyRootPart == Hub, "the hub is the root")
                assert(math.abs(Leaf.AssemblyMass - 0.7 * 84) < 1e-3, "the assembly's mass")
                local _ = Leaf.AssemblyCenterOfMass
                Leaf.AssemblyLinearVelocity = Vector3.new(0, I % 2, 0)
            end
            for I = 1, 20 do
                Hub.CFrame = Hub.CFrame + Vector3.new(0, 0, 1)
                Leaf.Position = Leaf.Position + Vector3.new(0, 0, 0.01)
            end
        end
        -- each scene twice, the second pass timed (the first warms the members and instances the frame touches)
        function shared.Use(Name)
            Pass(shared.Scenes[Name])
            local Start = os.clock()
            Pass(shared.Scenes[Name])
            shared.Work[Name] = os.clock() - Start
        end
        "#,
    );
    let mut host = Vec::new();
    for name in ["Shallow", "Deep", "Apart"] {
        let start = std::time::Instant::now();
        run(&lua, &engine, &format!("shared.Use(\"{name}\")"));
        host.push(start.elapsed().as_secs_f64());
    }
    run(
        &lua,
        &engine,
        r#"
        local Work = shared.Work
        assert(Work.Shallow > 0, "the passes cost work")
        -- (os.clock reads about 10,000 s, where its last bit is 1.8e-12 s: two passes that cost the same can read a bit apart)
        assert(math.abs(Work.Deep - Work.Shallow) < 1e-11, `the deep scene's pass cost {Work.Deep} s of work where the shallow one's {Work.Shallow}`)
        assert(math.abs(Work.Apart - Work.Shallow) < 1e-11, `the scene apart cost {Work.Apart} s of work where the shallow one's {Work.Shallow}`)
        "#,
    );
    let (shallow, deep, apart) = (host[0], host[1], host[2]);
    assert!(deep < 0.15 && apart < 0.15, "two passes of 1,040 lookups took {deep:.3} s 10,000 deep and {apart:.3} s 4,000 deep apart, {shallow:.3} s in the Workspace");
}

/// Indexing a joint under the parts it names (world/joints.rs index_joint) costs the same however many joints name the part:
/// 60,000 welds made to name one part, re-pointed to another and back, then cleared, cost each write about the host time 2,000
/// do (each part's list keeps every joint's place in it). Each write used to look through the part's whole list to add the
/// joint and again to take it out: at 40,000 joints a write took 5 to 8.5 us, at 1,000 0.4 to 0.7.
#[test]
fn indexing_a_joint_costs_the_same_however_many_joints_name_its_part() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r#"
        function shared.Index(N)
            local P, Other = Instance.new("Part"), Instance.new("Part")
            local Welds = table.create(N)
            for I = 1, N do
                Welds[I] = Instance.new("Weld")
            end
            for _, To in { P, Other, P } do
                for I = 1, N do
                    Welds[I].Part0 = To
                end
            end
            -- (taken out from the front of the list)
            for I = 1, N do
                Welds[I].Part0 = nil
            end
        end
        "#,
    );
    let per_weld = |count: usize| {
        let start = std::time::Instant::now();
        run(&lua, &engine, &format!("shared.Index({count})"));
        start.elapsed().as_secs_f64() / count as f64
    };
    per_weld(2_000);
    let (few, many) = (per_weld(2_000), per_weld(60_000));
    assert!(many < 3.0 * few, "four writes of a weld cost {:.2} us of host time with 60,000 naming its part, {:.2} us with 2,000", many * 1e6, few * 1e6);
}

/// A part's joints are taken up in the order they were indexed under it, however many came and went before (each part's list
/// leaves a hole where a joint left and closes its holes, in order, once they outnumber its joints): 300 welds on one part,
/// re-pointed to another part, cleared, pointed back and disabled at random 2,000 times, and after each 100 a Position write
/// to the part rewrites the C0 of each enabled weld still on it in the order of a list that each re-pointing takes the weld
/// out of and appends it to (their Changed events, in order), and the index keeps each joint's place in its parts' lists.
#[test]
fn a_parts_joints_keep_their_order_as_joints_come_and_go() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r#"
        local function Anchored(Name)
            local Part = Instance.new("Part")
            Part.Name, Part.Anchored, Part.CanCollide = Name, true, false
            Part.Parent = workspace
            return Part
        end
        local P, Other = Anchored("P"), Anchored("Other")
        local Welds, Order, Fired = {}, {}, {}
        -- (a change of a weld's parts takes it out of each list it was in and onto the end of its new parts' lists)
        local function Point(W, To)
            if W.Part0 == To then
                return
            end
            local Index = table.find(Order, W)
            if Index then
                table.remove(Order, Index)
            end
            W.Part0 = To
            if To == P then
                table.insert(Order, W)
            end
        end
        for I = 1, 300 do
            local W = Instance.new("Weld")
            W.Name = "W" .. I
            W.Part1 = Anchored("Q" .. I)
            W.Parent = W.Part1
            W.Changed:Connect(function(Name)
                if Name == "C0" then
                    table.insert(Fired, W.Name)
                end
            end)
            Welds[I] = W
            Point(W, P)
        end
        local Dice = Random.new(7)
        function shared.Round()
            for _ = 1, 100 do
                local W = Welds[Dice:NextInteger(1, #Welds)]
                local Roll = Dice:NextInteger(1, 4)
                if Roll == 1 then
                    Point(W, Other)
                elseif Roll == 2 then
                    Point(W, nil)
                elseif Roll == 3 then
                    Point(W, P)
                else
                    W.Enabled = not W.Enabled
                end
            end
            table.clear(Fired)
            P.Position += Vector3.new(0, 0, 1)
        end
        function shared.Check(Round)
            local Want = {}
            for _, W in Order do
                if W.Enabled then
                    table.insert(Want, W.Name)
                end
            end
            local Got, Expected = table.concat(Fired, " "), table.concat(Want, " ")
            assert(#Want > 0, `round {Round}: no weld left on the part`)
            assert(Got == Expected, `round {Round}: the welds were taken up as {Got} where {Expected}`)
        end
        "#,
    );
    for round in 1..=20 {
        // (the Changed events fire as the call ends)
        run(&lua, &engine, "shared.Round()");
        run(&lua, &engine, &format!("shared.Check({round})"));
    }
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// A part entering a world pays for its joints (world/joints.rs reparented, cost.rs joints_entering), as a playing client's move
/// does (Studio 0.747): beyond the same part's move with none, 1,000 welds in the world that cannot link (disabled) cost the
/// Parent write 0.05 us each (looked at), 1,000 that link it to anchored parts 0.7 us each (looked at from both their parts and
/// linked), 1,000 outside every world nothing, and 1,000 carried in under it at least 0.4 us each (looked at from both parts and
/// brought into the world, besides what the write pays for carrying them), into the Workspace and into a WorldModel alike. The
/// write used to cost the same with any number of joints.
#[test]
fn a_part_entering_a_world_pays_for_its_joints() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let (looked, linked, entered) = (crate::cost::JOINT_LOOKED_AT, crate::cost::JOINT_LINKED, crate::cost::JOINT_ENTERED);
    run(
        &lua,
        &engine,
        &format!(
            r#"
        local LOOKED, LINKED, ENTERED, N = {looked:e}, {linked:e}, {entered:e}, 1000
        local function Part(Name, Anchored)
            local P = Instance.new("Part")
            P.Name, P.Anchored, P.CanCollide, P.CFrame = Name, Anchored, false, CFrame.new(0, 50, 0)
            return P
        end
        local World = Instance.new("WorldModel")
        World.Parent = game:GetService("ReplicatedStorage")
        for _, Into in {{ workspace, World }} do
            local Work = {{}}
            for _, Kind in {{ "none", "outside", "inactive", "linked", "carried" }} do
                local Box = Instance.new("Folder")
                Box.Parent = Into
                local P = Part("P", false)
                for _ = 1, if Kind == "none" then 0 else N do
                    local Q = Part("Q", true)
                    Q.Parent = Box
                    local W = Instance.new("Weld")
                    W.Part0, W.Part1, W.Enabled = P, Q, Kind ~= "inactive" and Kind ~= "carried"
                    W.Parent = if Kind == "outside" then nil elseif Kind == "carried" then P else Q
                end
                -- (the second move timed: the first takes the frame's first entry)
                P.Parent = Into
                P.Parent = nil
                local Start = os.clock()
                P.Parent = Into
                Work[Kind] = os.clock() - Start
                P.Parent = nil
                Box:Destroy()
            end
            local function Is(Kind, Want)
                local Got = Work[Kind] - Work.none
                assert(math.abs(Got - Want) < 1e-9, `{{Into.Name}}: {{Kind}} welds cost the move {{Got * 1e6}} us beyond none, where {{Want * 1e6}}`)
            end
            Is("outside", 0)
            Is("inactive", N * LOOKED)
            Is("linked", N * (2 * LOOKED + LINKED))
            local Carried = Work.carried - Work.none
            assert(Carried >= N * (2 * LOOKED + ENTERED), `{{Into.Name}}: welds carried in cost the move {{Carried * 1e6}} us beyond none`)
        end
        "#
        ),
    );
}

/// A part entering a world looks only at its joints in a world (world/joints.rs PartJoints), as a client does (Studio 0.747: a
/// part named by 10,000 welds outside every world entered as fast as one named by none): a part named by 50,000 welds outside
/// every world, moved into the Workspace and out 200 times and into a WorldModel and out 200 times, costs about the host time of
/// a part named by none, and the work clock charges the two the same. Each move used to look at each of its welds: at 10,000,
/// 0.13 to 0.23 ms a move.
#[test]
fn a_part_entering_a_world_skips_its_joints_outside_every_world() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Part()
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide = true, false
            return P
        end
        local Lone, Named = Part(), Part()
        for _ = 1, 50000 do
            local W = Instance.new("Weld")
            W.Part0, W.Part1 = Named, Part()
        end
        local World = Instance.new("WorldModel")
        World.Parent = game:GetService("ReplicatedStorage")
        shared.Parts, shared.Work = { Lone = Lone, Named = Named }, {}
        local function Moves(P)
            for _ = 1, 200 do
                P.Parent = workspace
                P.Parent = nil
                P.Parent = World
                P.Parent = nil
            end
        end
        function shared.Move(Name)
            local P = shared.Parts[Name]
            Moves(P)
            local Start = os.clock()
            Moves(P)
            shared.Work[Name] = os.clock() - Start
        end
        "#,
    );
    let mut host = Vec::new();
    for name in ["Lone", "Named"] {
        let start = std::time::Instant::now();
        run(&lua, &engine, &format!("shared.Move(\"{name}\")"));
        host.push(start.elapsed().as_secs_f64());
    }
    // (os.clock reads about 10,000 s, where its last bit is 1.8e-12 s)
    run(&lua, &engine, r#"assert(math.abs(shared.Work.Named - shared.Work.Lone) < 1e-11, `the named part's moves cost {shared.Work.Named} s of work where the lone one's {shared.Work.Lone}`)"#);
    let (lone, named) = (host[0], host[1]);
    assert!(named < 2.0 * lone + 0.05, "1,600 moves took {named:.3} s of host time for a part named by 50,000 welds outside every world, {lone:.3} s for one named by none");
}

/// Which of a part's joints are in a world is marked in its list as the dom logs their homes (dom.rs Dom::log_homes), whatever
/// moves them: 60 welds, 40 parts, folders and WorldModels moved between the Workspace, WorldModels, JointsService, storage and
/// nil, re-pointed, models of them cloned and destroyed, 2,000 times at random, and each joint in each list is marked as in
/// a world exactly when it has a home, its walks taking up exactly those, in order.
#[test]
fn a_joints_mark_follows_its_home_however_it_moves() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r#"
        local Storage, Service = game:GetService("ReplicatedStorage"), game:GetService("JointsService")
        local Holders, Parts, Welds, Movable = { workspace, Storage, Service }, {}, {}, {}
        local Dice = Random.new(11)
        local function Pick(List)
            return List[Dice:NextInteger(1, #List)]
        end
        for I = 1, 6 do
            local Holder = Instance.new(if I % 2 == 0 then "WorldModel" else "Folder")
            Holder.Parent = Pick(Holders)
            table.insert(Holders, Holder)
            table.insert(Movable, Holder)
        end
        for I = 1, 40 do
            local P = Instance.new("Part")
            P.Anchored = I % 3 == 0
            P.CanCollide = false
            P.Parent = Pick(Holders)
            table.insert(Parts, P)
            table.insert(Movable, P)
        end
        for _ = 1, 60 do
            local W = Instance.new(if Dice:NextInteger(1, 3) == 1 then "WeldConstraint" else "Weld")
            W.Part0, W.Part1 = Pick(Parts), Pick(Parts)
            W.Parent = if Dice:NextInteger(1, 4) == 1 then nil else Pick(Holders)
            table.insert(Welds, W)
            table.insert(Movable, W)
        end
        -- (a model of two parts and their weld, cloned with what it holds)
        local Template = Instance.new("Model")
        local A, B = Instance.new("Part"), Instance.new("Part")
        A.Parent, B.Parent = Template, Template
        local Joint = Instance.new("Weld")
        Joint.Part0, Joint.Part1, Joint.Parent = A, B, A
        function shared.Round()
            for _ = 1, 100 do
                local Roll = Dice:NextInteger(1, 10)
                if Roll <= 5 then
                    local Moved = Pick(Movable)
                    local To = if Dice:NextInteger(1, 6) == 1 then nil else Pick(Holders)
                    pcall(function()
                        Moved.Parent = To
                    end)
                elseif Roll <= 7 then
                    local W = Pick(Welds)
                    if Dice:NextInteger(1, 2) == 1 then
                        W.Part0 = Pick(Parts)
                    else
                        W.Part1 = if Dice:NextInteger(1, 5) == 1 then nil else Pick(Parts)
                    end
                elseif Roll == 8 then
                    local Copy = Template:Clone()
                    Copy.Parent = if Dice:NextInteger(1, 4) == 1 then nil else Pick(Holders)
                    table.insert(Movable, Copy)
                    for _, Item in Copy:GetDescendants() do
                        table.insert(if Item:IsA("BasePart") then Parts else Welds, Item)
                        table.insert(Movable, Item)
                    end
                elseif Roll == 9 then
                    local W = Pick(Welds)
                    W:Destroy()
                else
                    local _ = Pick(Parts).AssemblyRootPart
                end
            end
        end
        "#,
    );
    for round in 1..=20 {
        run(&lua, &engine, "shared.Round()");
        if let Err(message) = super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection) {
            panic!("round {round}: {message}");
        }
    }
}

/// The dom's log of the joints' homes (dom.rs Dom::log_homes) keeps twice the dom's slots and 4,096 more; past that it says so,
/// and the next walk marks every indexed joint again: a weld out of the Workspace (marked so), moved in and out 100,000 times
/// straight through the dom, never by a Parent write, and left in, links its parts at the next lookup, and its marks match its
/// home.
#[test]
fn a_joints_mark_follows_its_home_when_the_log_overflows() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r#"
        local A, B = Instance.new("Part"), Instance.new("Part")
        A.Anchored, A.CanCollide, B.CanCollide = true, false, false
        A.Parent, B.Parent = workspace, workspace
        local Weld = Instance.new("Weld")
        Weld.Part0, Weld.Part1 = A, B
        Weld.Parent = workspace
        assert(B.AssemblyRootPart == A, "the weld links its parts")
        Weld.Parent = nil
        assert(B.AssemblyRootPart == B, "out of the Workspace the weld links nothing")
        shared.A, shared.B = A, B
        "#,
    );
    {
        let mut state = engine.state.borrow_mut();
        let class = |name: &str| engine.reflection.class_id(name).expect("class");
        let (weld, workspace) = (state.dom.instances_of_class(class("Weld"))[0], state.dom.instances_of_class(class("Workspace"))[0]);
        for _ in 0..100_000 {
            state.dom.reparent(weld, Some(workspace));
            state.dom.reparent(weld, None);
        }
        state.dom.reparent(weld, Some(workspace));
    }
    run(&lua, &engine, r#"assert(shared.B.AssemblyRootPart == shared.A, "the weld moved back into the Workspace links its parts")"#);
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// A part's assembly reads walk no joint while nothing changed, and a part's joints that cannot link cost its writes and reads
/// nothing: 2,000 AssemblyRootPart, AssemblyMass and AssemblyCenterOfMass reads of one of 2,000 parts welded to an anchored hub,
/// and 2,000 CFrame and Anchored writes and AssemblyRootPart and AssemblyMass reads of an unanchored part named by 10,000 welds in
/// the Workspace whose Part1 is nil, take about the host time they take for a lone part, as a playing client's cost the same at
/// any joint count. Each read of the first used to walk the hub's welds (0.13 ms at 1,000), each of the second the part's (8 us
/// at 1,000).
#[test]
fn assembly_reads_and_joints_that_cannot_link_walk_nothing() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function Part(Anchored)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = Anchored, false, CFrame.new(0, 300, 0)
            P.Parent = workspace
            return P
        end
        local Hub, Leaf = Part(true), nil
        for _ = 1, 2000 do
            local W = Instance.new("Weld")
            W.Part0, W.Part1 = Hub, Part(false)
            W.Parent = W.Part1
            Leaf = Leaf or W.Part1
        end
        local Named = Part(false)
        for _ = 1, 10000 do
            local W = Instance.new("Weld")
            W.Part0 = Named
            W.Parent = Named
        end
        shared.Parts = { Leaf = Leaf, Lone = Part(false), Named = Named }
        function shared.Reads(Name)
            local P = shared.Parts[Name]
            for _ = 1, 2000 do
                local _ = P.AssemblyRootPart
                local _ = P.AssemblyMass
                local _ = P.AssemblyCenterOfMass
            end
        end
        function shared.Writes(Name)
            local P = shared.Parts[Name]
            local At = P.CFrame
            for Index = 1, 2000 do
                P.CFrame = At + Vector3.new(0, Index % 2, 0)
                P.Anchored = Index % 2 == 0
                local _ = P.AssemblyRootPart
                local _ = P.AssemblyMass
            end
            P.Anchored = false
        end
        "#,
    );
    let time = |source: &str| {
        run(&lua, &engine, source);
        let start = std::time::Instant::now();
        run(&lua, &engine, source);
        start.elapsed().as_secs_f64()
    };
    let (leaf, lone) = (time(r#"shared.Reads("Leaf")"#), time(r#"shared.Reads("Lone")"#));
    assert!(leaf < 2.0 * lone + 0.05, "6,000 reads took {leaf:.3} s of host time for a part of an assembly of 2,001, {lone:.3} s for a lone part");
    let (named, lone) = (time(r#"shared.Writes("Named")"#), time(r#"shared.Writes("Lone")"#));
    assert!(named < 2.0 * lone + 0.05, "8,000 writes and reads took {named:.3} s of host time for a part named by 10,000 welds that cannot link, {lone:.3} s for a lone part");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// What an assembly's writes cost beyond the same write to a lone part (measured in a playing client, cost.rs): a CFrame write
/// each part it carries and each joint between its assembly and another (assembly_moved), BulkMoveTo the parts alone, a Position
/// write each joint it rewrites and, for those inside the part's assembly, each part of it (joints_rewritten), an Anchored write
/// each other part whose root or the root's anchoring changes and each joint linking the part (joints_reanchored), and the link
/// change (LINK_CHANGED: each write is timed after two others of the same assembly in its frame), and an AssemblyLinearVelocity
/// write each other part of the assembly (ASSEMBLY_VELOCITY_PART). An anchored hub welded to 100 unanchored parts, and an
/// unanchored part welded to 100 anchored ones (it joins the first one's assembly); each write paid what a lone part's did.
#[test]
fn assembly_writes_pay_for_the_parts_and_joints_they_change() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function Part(Anchored, Size)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.Size, P.CFrame = Anchored, false, Size or Vector3.one, CFrame.new(0, 300, 0)
            P.Parent = workspace
            return P
        end
        local Big = Vector3.new(4, 1, 4)
        local Parts = { Hub = Part(true, Big), Free = Part(false, Big), Lone = Part(true, Big), LoneFree = Part(false, Big) }
        for _ = 1, 100 do
            local W = Instance.new("Weld")
            W.Part0, W.Part1 = Parts.Hub, Part(false)
            W.Parent = workspace
            local V = Instance.new("Weld")
            V.Part0, V.Part1 = Parts.Free, Part(true)
            V.Parent = workspace
        end
        -- (each write's third call: the first two warm its members)
        local function Work(F)
            F(true)
            F(false)
            local Start = os.clock()
            F(true)
            local Spent = os.clock() - Start
            F(false)
            return Spent
        end
        shared.Work = {}
        for Name, P in Parts do
            local At = P.CFrame
            local Was = P.Anchored
            shared.Work[Name] = {
                CFrame = Work(function(On) P.CFrame = if On then At + Vector3.yAxis else At end),
                Bulk = Work(function(On) workspace:BulkMoveTo({ P }, { if On then At + Vector3.yAxis else At }) end),
                Position = Work(function(On) P.Position = At.Position + (if On then Vector3.yAxis else Vector3.zero) end),
                Anchored = Work(function(On) P.Anchored = if On then not Was else Was end),
                Velocity = Work(function(On) P.AssemblyLinearVelocity = if On then Vector3.xAxis else Vector3.zero end),
            }
        end
        "#,
    );
    let cost = |what: &str| -> f64 {
        match what {
            "Hub.CFrame" => crate::cost::assembly_moved(100, 0),
            "Free.CFrame" => crate::cost::assembly_moved(1, 99),
            "Hub.Bulk" => crate::cost::assembly_moved(100, 0),
            "Free.Bulk" => crate::cost::assembly_moved(1, 0),
            "Hub.Position" => crate::cost::joints_rewritten(100, 100, 101),
            "Free.Position" => crate::cost::joints_rewritten(100, 1, 2),
            "Hub.Anchored" => crate::cost::joints_reanchored(100, 100) + crate::cost::LINK_CHANGED,
            "Free.Anchored" => crate::cost::joints_reanchored(0, 100) + crate::cost::LINK_CHANGED,
            "Hub.Velocity" => 100.0 * crate::cost::ASSEMBLY_VELOCITY_PART,
            "Free.Velocity" => crate::cost::ASSEMBLY_VELOCITY_PART,
            _ => unreachable!(),
        }
    };
    for (part, lone) in [("Hub", "Lone"), ("Free", "LoneFree")] {
        for write in ["CFrame", "Bulk", "Position", "Anchored", "Velocity"] {
            let want = cost(&format!("{part}.{write}"));
            assert!(want > 0.0);
            run(&lua, &engine, &format!("local Extra = shared.Work.{part}.{write} - shared.Work.{lone}.{write} assert(math.abs(Extra - {want:e}) < 1e-9, `{part}'s {write} write cost {{Extra}} s more than a lone part's, not {want:e}`)"));
        }
    }
}

/// A link change pays for its assembly once a frame, as a playing client's does (cost.rs LINK_*, BODY_REBUILT_PART): an anchored
/// hub welded to 1,000 parts against one welded to 10, a leaf's weld disabled at a frame's start, enabled again, the hub's
/// AssemblyCenterOfMass read twice, and in the next frame the leaf anchored and unanchored. The first change of each frame costs
/// LINK_PART more for each part beyond the small hub's, and the first centre read after it BODY_REBUILT_PART each; the later change
/// and read cost the same at either size. The small hub's first change costs LINK_FRAME and the first change of its two pieces
/// (link_assembly) more than its second. Each of those cost the same at any size before.
#[test]
fn link_changes_pay_for_their_assembly_once_a_frame() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function Part(Anchored)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = Anchored, false, CFrame.new(0, 300, 0)
            P.Parent = workspace
            return P
        end
        local function Spend(F)
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        local function Changes(Count)
            local Hub, Welds = Part(true), {}
            for Index = 1, Count do
                local W = Instance.new("Weld")
                W.Part0, W.Part1 = Hub, Part(false)
                W.Parent = W.Part1
                Welds[Index] = W
            end
            local Weld, Leaf = Welds[Count], Welds[Count].Part1
            -- (each frame reads the weld and its parts and writes the members first, a weld that links nothing and a lone part: the
            -- first touch and use of the frame are not what is compared)
            local Spare, Lone = Instance.new("Weld"), Part(false)
            Spare.Parent = workspace
            local function Touch()
                local _ = Weld.Enabled, Leaf.Anchored, Hub.Anchored
                Spare.Enabled = not Spare.Enabled
                Lone.Anchored = not Lone.Anchored
            end
            task.wait()
            Touch()
            local Out = {}
            Out.First = Spend(function() Weld.Enabled = false end)
            Out.Again = Spend(function() Weld.Enabled = true end)
            Out.Centre = Spend(function() local _ = Hub.AssemblyCenterOfMass end)
            Out.CentreAgain = Spend(function() local _ = Hub.AssemblyCenterOfMass end)
            task.wait()
            Touch()
            Out.Anchor = Spend(function() Leaf.Anchored = true end)
            Out.Unanchor = Spend(function() Leaf.Anchored = false end)
            task.wait()
            return Out
        end
        -- (a first round runs the code once: its first run's cold code is not what is compared)
        Changes(10)
        shared.Big = Changes(1000)
        shared.Small = Changes(10)
        "#,
    );
    let more = 990.0;
    for (name, want) in [
        ("First", more * crate::cost::LINK_PART),
        ("Again", 0.0),
        ("Centre", more * crate::cost::BODY_REBUILT_PART),
        ("CentreAgain", 0.0),
        ("Anchor", more * crate::cost::LINK_PART),
        ("Unanchor", 0.0),
    ] {
        run(&lua, &engine, &format!("local Extra = shared.Big.{name} - shared.Small.{name} assert(math.abs(Extra - {want:e}) < 1e-9, `{name}: {{Extra}} s more for 1,001 parts than 11, not {want:e}`)"));
    }
    // (the weld disabled leaves the hub's 10 parts and the leaf alone, the first change of each in the frame)
    let first = crate::cost::LINK_FRAME + crate::cost::link_assembly(10) + crate::cost::link_assembly(1);
    run(&lua, &engine, &format!("local Extra = shared.Small.First - shared.Small.Again assert(math.abs(Extra - {first:e}) < 1e-9, `the frame's first change cost {{Extra}} s more than the second, not {first:e}`)"));
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// Link changes in a large assembly walk none of it: 500 rounds of disabling and enabling a leaf's weld (each a different leaf),
/// anchoring and unanchoring it and reading the hub's and the leaf's AssemblyRootPart, on an anchored hub welded to 5,000 parts
/// and the weld toggles and reads on an unanchored one, take about the host time they take on a hub welded to one part, as a
/// playing client's later changes in a frame cost the same at any size. Each change used to walk the whole assembly again (1 to
/// 2.5 ms at 5,000).
#[test]
fn link_changes_in_a_large_assembly_walk_none_of_it() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function Part(Anchored, Size)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.Size, P.CFrame = Anchored, false, Size or Vector3.one, CFrame.new(0, 300, 0)
            P.Parent = workspace
            return P
        end
        local function Star(Anchored, Count)
            local Hub, Welds = Part(Anchored, Vector3.new(6, 6, 6)), {}
            for Index = 1, Count do
                local W = Instance.new("Weld")
                W.Part0, W.Part1, W.C0 = Hub, Part(false), CFrame.new(Index % 7, 4, 0)
                W.Parent = W.Part1
                Welds[Index] = W
            end
            return { Hub = Hub, Welds = Welds, Anchored = Anchored }
        end
        shared.Stars = { Big = Star(true, 5000), Small = Star(true, 1), BigFree = Star(false, 5000), SmallFree = Star(false, 1) }
        function shared.Changes(Name)
            local Star = shared.Stars[Name]
            for Index = 1, 500 do
                local W = Star.Welds[Index % #Star.Welds + 1]
                W.Enabled = false
                W.Enabled = true
                if Star.Anchored then
                    W.Part1.Anchored = true
                    W.Part1.Anchored = false
                end
                local _ = Star.Hub.AssemblyRootPart
                local _ = W.Part1.AssemblyRootPart
            end
        end
        "#,
    );
    let time = |source: &str| {
        run(&lua, &engine, source);
        let start = std::time::Instant::now();
        run(&lua, &engine, source);
        start.elapsed().as_secs_f64()
    };
    for (big, small) in [("Big", "Small"), ("BigFree", "SmallFree")] {
        let (large, lone) = (time(&format!("shared.Changes('{big}')")), time(&format!("shared.Changes('{small}')")));
        assert!(large < 2.0 * lone + 0.05, "{big}: 500 rounds of link changes took {large:.3} s of host time with 5,001 parts, {lone:.3} s with 2");
    }
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// Every kind of link change pays for its assembly once a frame, as a playing client's does (cost.rs LINK_*): a leaf's weld
/// re-pointed to a spare part (Part1) and back, its Part1 set to nil and back, its Part0 re-pointed and back, the weld's Parent set
/// to nil, the weld destroyed, the leaf's Parent set to nil and the leaf destroyed (then another leaf's weld or leaf alike), each
/// at a frame's start on an anchored hub welded to 1,000 parts against one welded to 10: the first change costs LINK_PART more for
/// each part beyond the small hub's, the second the same at either size; a weld taken back into the world and out again, the second
/// the same at either size. Only Enabled was priced as a link change before (a client spends 874 to 982 us on each of these at a
/// frame's start with 10,000 parts, 10 to 43 us on a later one; 18 to 35 us on a weld taken out right after it was taken in).
#[test]
fn link_changes_of_every_kind_pay_for_their_assembly_once_a_frame() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function Part(Anchored)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = Anchored, false, CFrame.new(0, 300, 0)
            P.Parent = workspace
            return P
        end
        local function Spend(F)
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        -- (each kind's first change and the change after it in the frame, of the star's last leaf W.Part1 and the one before it)
        local Kinds = {
            Part1 = function(S) return function() S.W.Part1 = S.Spare end, function() S.W.Part1 = S.L end end,
            Nil = function(S) return function() S.W.Part1 = nil end, function() S.W.Part1 = S.L end end,
            Part0 = function(S) return function() S.W.Part0 = S.Spare end, function() S.W.Part0 = S.Hub end end,
            WeldOut = function(S) return function() S.W.Parent = nil end, function() S.W2.Parent = nil end end,
            WeldDestroyed = function(S) return function() S.W:Destroy() end, function() S.W2:Destroy() end end,
            LeafOut = function(S) return function() S.L.Parent = nil end, function() S.L2.Parent = nil end end,
            LeafDestroyed = function(S) return function() S.L:Destroy() end, function() S.L2:Destroy() end end,
            -- (a weld taken back into the world, the frame before taken out: priced by its joints, then a later change)
            WeldIn = function(S) return function() S.W.Parent = S.L end, function() S.W.Parent = nil end, function() S.W.Parent = nil end end,
        }
        local function Changes(Count, Kind)
            local Hub, Welds = Part(true), {}
            for Index = 1, Count do
                local W = Instance.new("Weld")
                W.Part0, W.Part1 = Hub, Part(false)
                W.Parent = W.Part1
                Welds[Index] = W
            end
            local S = { Hub = Hub, W = Welds[Count], L = Welds[Count].Part1, W2 = Welds[Count - 1], L2 = Welds[Count - 1].Part1, Spare = Part(false) }
            -- (the changed leaves in a folder of their own: taking one out of its parent costs a search of that parent's children)
            local Holder = Instance.new("Folder")
            Holder.Parent = workspace
            S.L.Parent, S.L2.Parent = Holder, Holder
            -- (the frame reads the changed instances and writes the members first, a weld that links nothing and a lone part: the
            -- first touch and use of the frame are not what is compared)
            local Spare, Lone = Instance.new("Weld"), Part(false)
            Spare.Parent = workspace
            local First, Later, Before = Kinds[Kind](S)
            if Before then
                Before()
            end
            task.wait()
            local _ = S.W.Enabled, S.W2.Enabled, S.L.Anchored, S.L2.Anchored, S.Hub.Anchored, S.Spare.Anchored, S.W.Parent, S.L.Parent
            Spare.Enabled = not Spare.Enabled
            Lone.Anchored = not Lone.Anchored
            local Out = { First = Spend(First), Later = Spend(Later) }
            task.wait()
            return Out
        end
        shared.Big, shared.Small = {}, {}
        for Kind in Kinds do
            -- (a first round runs the code once: its first run's cold code is not what is compared)
            Changes(10, Kind)
            shared.Big[Kind] = Changes(1000, Kind)
            shared.Small[Kind] = Changes(10, Kind)
        end
        "#,
    );
    let more = 990.0 * crate::cost::LINK_PART;
    // (a weld taken in pays for the joints its walks look at, a later change in the frame the same at either size)
    run(&lua, &engine, "local Extra = shared.Big.WeldIn.Later - shared.Small.WeldIn.Later assert(math.abs(Extra) < 1e-9, `WeldIn Later: {Extra} s more for 1,001 parts than 11`)");
    for kind in ["Part1", "Nil", "Part0", "WeldOut", "WeldDestroyed", "LeafOut", "LeafDestroyed"] {
        for (name, want) in [("First", more), ("Later", 0.0)] {
            run(&lua, &engine, &format!("local Extra = shared.Big.{kind}.{name} - shared.Small.{kind}.{name} assert(math.abs(Extra - {want:e}) < 1e-9, `{kind} {name}: {{Extra}} s more for 1,001 parts than 11, not {want:e}`)"));
        }
        // (the frame's first change pays LINK_FRAME and the assemblies it changes, a later one LINK_CHANGED)
        let least = crate::cost::LINK_FRAME;
        run(&lua, &engine, &format!("local Extra = shared.Small.{kind}.First - shared.Small.{kind}.Later assert(Extra > {least:e}, `{kind}: the frame's first change cost {{Extra}} s more than the second`)"));
    }
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// Link changes of every kind in a large assembly walk none of it: 300 rounds on an anchored hub welded to 5,000 parts of a leaf's
/// weld re-pointed to a spare part and back (Part1, then Part0), its Part1 set to nil and back, another leaf destroyed and another's
/// Parent set to nil, each change followed by a third leaf's AssemblyRootPart read, take about the host time of the same rounds on
/// hubs welded to four parts (a hub of its own each round), as a client's later changes cost the same at any size. A re-point
/// dropped the components of its parts and walked them again (2.2 to 3.1 ms at 10,000), and a part taken out of its world dropped
/// its component for the next read to walk again.
#[test]
fn link_changes_of_every_kind_in_a_large_assembly_walk_none_of_it() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function Part(Anchored)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = Anchored, false, CFrame.new(0, 300, 0)
            P.Parent = workspace
            return P
        end
        local function Star(Count)
            local Hub, Welds = Part(true), {}
            for Index = 1, Count do
                local W = Instance.new("Weld")
                W.Part0, W.Part1, W.C0 = Hub, Part(false), CFrame.new(Index % 7, 4, 0)
                W.Parent = W.Part1
                Welds[Index] = W
            end
            return { Hub = Hub, Welds = Welds }
        end
        local Big, Small = Star(5000), {}
        for Index = 1, 400 do
            Small[Index] = Star(4)
        end
        local Spare = Part(false)
        -- (round `Index`'s weld to re-point, leaf to destroy, leaf to take out and leaf to read)
        local function Round(Index, Big)
            local Star, At = if Big then Big else Small[Index], if Big then { Index, Index + 1000, Index + 2000, Index + 3000 } else { 1, 2, 3, 4 }
            local W, Gone, Out, Read = Star.Welds[At[1]], Star.Welds[At[2]].Part1, Star.Welds[At[3]].Part1, Star.Welds[At[4]].Part1
            local L = W.Part1
            W.Part1 = Spare
            local _ = Read.AssemblyRootPart
            W.Part1 = L
            local _ = Read.AssemblyRootPart
            W.Part0 = Spare
            local _ = Read.AssemblyRootPart
            W.Part0 = Star.Hub
            local _ = Read.AssemblyRootPart
            W.Part1 = nil
            local _ = Read.AssemblyRootPart
            W.Part1 = L
            local _ = Read.AssemblyRootPart
            Gone:Destroy()
            local _ = Read.AssemblyRootPart
            Out.Parent = nil
            local _ = Read.AssemblyRootPart
        end
        function shared.Rounds(Which, From, To)
            for Index = From, To do
                Round(Index, if Which == "Big" then Big else nil)
            end
        end
        "#,
    );
    let time = |which: &str| {
        run(&lua, &engine, &format!("shared.Rounds('{which}', 1, 50)"));
        let start = std::time::Instant::now();
        run(&lua, &engine, &format!("shared.Rounds('{which}', 51, 350)"));
        start.elapsed().as_secs_f64()
    };
    let (large, small) = (time("Big"), time("Small"));
    assert!(large < 2.0 * small + 0.05, "300 rounds of link changes took {large:.3} s of host time on a hub with 5,000 parts, {small:.3} s on hubs with 4");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// An AssemblyCenterOfMass read right after a move works nothing out: 500 moves of an anchored hub welded to 5,000 parts, each
/// followed by a read of its centre, the reads taking about the host time a lone part's take (the medians), as a playing client's
/// read after a move costs the same at any size (the move works the centres read since the last one out again, from the masses
/// kept). Each read used to work the centre out from every part (0.2 to 0.3 ms at 5,000).
#[test]
fn centre_reads_after_a_move_work_nothing_out() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function Part(Anchored)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = Anchored, false, CFrame.new(0, 300, 0)
            P.Parent = workspace
            return P
        end
        local Hub = Part(true)
        for Index = 1, 5000 do
            local W = Instance.new("Weld")
            W.Part0, W.Part1, W.C0 = Hub, Part(false), CFrame.new(Index % 11, 2, Index % 13)
            W.Parent = W.Part1
        end
        shared.Parts = { Big = Hub, Lone = Part(true) }
        shared.Moves = 0
        function shared.Move(Name)
            local P = shared.Parts[Name]
            shared.Moves += 1
            P.CFrame = CFrame.new(0, 300 + shared.Moves % 2, 0) * CFrame.Angles(0, shared.Moves % 3, 0)
        end
        function shared.Read(Name)
            local _ = shared.Parts[Name].AssemblyCenterOfMass
        end
        "#,
    );
    // (the two parts' moves and reads in turn, each read timed alone: the medians, whatever else the host is doing)
    let mut times: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    for _ in 0..500 {
        for (index, name) in ["Big", "Lone"].into_iter().enumerate() {
            run(&lua, &engine, &format!("shared.Move('{name}')"));
            let start = std::time::Instant::now();
            run(&lua, &engine, &format!("shared.Read('{name}')"));
            times[index].push(start.elapsed().as_secs_f64());
        }
    }
    let [big, lone] = times.map(|mut list| {
        list.sort_by(f64::total_cmp);
        list[list.len() / 2]
    });
    assert!(big < 2.0 * lone + 50e-6, "a centre read after a move took {:.1} us of host time (the median of 500) for an assembly of 5,001 parts, {:.1} us for a lone part", big * 1e6, lone * 1e6);
}

/// Kept components answer as fresh walks do through link changes: 800 random changes to an anchored hub and an unanchored one each
/// welded to 12 parts (Welds and WeldConstraints), a chain of 10 parts hanging from an anchored one and a free chain of 8 (joints
/// enabled, disabled, destroyed and made again, moved to nil and back; parts anchored and unanchored, moved by CFrame, PivotTo
/// and the physics step, Motor6D.Transform written; a frame now and then), with every part's AssemblyRootPart, AssemblyMass,
/// AssemblyCenterOfMass and CFrame and each change's work read after each change, read the same with the components kept (split
/// and joined as links change) as with every walk made afresh, and the index and the kept components hold together.
#[test]
fn link_changes_keep_components_as_fresh_walks_do() {
    let script = r#"
        workspace.Gravity = 0
        local Dice = Random.new(41)
        local Model = Instance.new("Model")
        Model.Parent = workspace
        local Parts, Joints = {}, {}
        local function Part(Anchored, At, Size)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.Size, P.CFrame = Anchored, false, Size or Vector3.one, At
            P.Parent = if Dice:NextInteger(1, 4) == 1 then Model else workspace
            table.insert(Parts, P)
            return P
        end
        local function Join(Kind, A, B, Offset)
            local J = Instance.new(Kind)
            J.Part0, J.Part1 = A, B
            if J:IsA("JointInstance") then
                J.C0 = Offset
            end
            J.Parent = B
            table.insert(Joints, J)
            return J
        end
        for Hubs = 1, 2 do
            local Hub = Part(Hubs == 1, CFrame.new(Hubs * 40, 50, 0), Vector3.new(6, 6, 6))
            for Index = 1, 12 do
                local Leaf = Part(false, Hub.CFrame * CFrame.new(Index % 5 - 2, 4, Index // 5))
                Join(if Index % 3 == 0 then "WeldConstraint" elseif Index % 4 == 0 then "Motor6D" else "Weld", Hub, Leaf, Hub.CFrame:ToObjectSpace(Leaf.CFrame))
            end
        end
        for Chains = 1, 2 do
            local Prev = Part(Chains == 1, CFrame.new(-40 * Chains, 50, 0))
            for Index = 1, if Chains == 1 then 10 else 8 do
                local Next = Part(false, Prev.CFrame * CFrame.new(2, 0, 0) * CFrame.Angles(0, 0.1, 0))
                Join(if Index % 2 == 0 then "WeldConstraint" else "Weld", Prev, Next, Prev.CFrame:ToObjectSpace(Next.CFrame))
                Prev = Next
            end
        end
        local function Pick(List)
            return List[Dice:NextInteger(1, #List)]
        end
        local function Numbers(...)
            local Out = {}
            for Index, Value in { ... } do
                Out[Index] = string.format("%.9g", Value)
            end
            return table.concat(Out, ",")
        end
        local function Digest(Op, Work)
            local Line = { Op, string.format("%.17g", Work) }
            for Index, P in Parts do
                local Root = P.AssemblyRootPart
                local Centre = P.AssemblyCenterOfMass
                table.insert(Line, string.format("%d:%d:%.9g:%s:%s", Index, Root and table.find(Parts, Root) or 0, P.AssemblyMass, Numbers(Centre.X, Centre.Y, Centre.Z), Numbers(P.CFrame:GetComponents())))
            end
            print(table.concat(Line, " "))
        end
        local Ops = {
            function()
                local J = Pick(Joints)
                J.Enabled = not J.Enabled
                return "enabled"
            end,
            function()
                local J = Pick(Joints)
                J.Enabled = not J.Enabled
                J.Enabled = not J.Enabled
                return "toggled"
            end,
            function()
                local P = Pick(Parts)
                P.Anchored = not P.Anchored
                return "anchored"
            end,
            function()
                local J = Pick(Joints)
                J.Parent = if J.Parent then nil else J.Part1
                return "parent"
            end,
            function()
                local Index = Dice:NextInteger(1, #Joints)
                local J = Joints[Index]
                local Kind, A, B = J.ClassName, J.Part0, J.Part1
                J:Destroy()
                if A and B then
                    Joints[Index] = Join(Kind, A, B, A.CFrame:ToObjectSpace(B.CFrame))
                end
                return "remade"
            end,
            function()
                local J = Pick(Joints)
                local Side = if Dice:NextInteger(1, 2) == 1 then "Part0" else "Part1"
                J[Side] = if Dice:NextInteger(1, 4) == 1 then nil else Pick(Parts)
                return "repointed"
            end,
            function()
                local P = Pick(Parts)
                P.Parent = if P.Parent then nil elseif Dice:NextInteger(1, 4) == 1 then Model else workspace
                return "part parent"
            end,
            function()
                -- (a part destroyed, with the joints parented to it, and a new one made where it was)
                local Index = Dice:NextInteger(1, #Parts)
                local P = Parts[Index]
                local At, Anchored, Size = P.CFrame, P.Anchored, P.Size
                P:Destroy()
                local New = Instance.new("Part")
                New.Anchored, New.CanCollide, New.Size, New.CFrame = Anchored, false, Size, At
                New.Parent = workspace
                Parts[Index] = New
                return "destroyed"
            end,
            function()
                local P = Pick(Parts)
                P.CFrame = P.CFrame * CFrame.new(Dice:NextNumber(-1, 1), 0, 0) * CFrame.Angles(0, Dice:NextNumber(-0.3, 0.3), 0)
                return "cframe"
            end,
            function()
                Model:PivotTo(Model:GetPivot() * CFrame.new(0, Dice:NextNumber(-1, 1), 0))
                return "pivot"
            end,
            function()
                local J = Pick(Joints)
                if J:IsA("Motor6D") then
                    J.Transform = CFrame.Angles(Dice:NextNumber(-0.5, 0.5), 0, 0)
                end
                return "transform"
            end,
            function()
                Pick(Parts).AssemblyLinearVelocity = Vector3.new(0, Dice:NextNumber(-1, 1), 0)
                return "velocity"
            end,
            function()
                task.wait()
                return "frame"
            end,
        }
        function shared.Round()
            for _ = 1, 80 do
                local Op = Pick(Ops)
                local Start = os.clock()
                local Ok, Name = pcall(Op)
                local Work = os.clock() - Start
                Digest(if Ok then Name else "raised", Work)
            end
        end
    "#;
    let mut logs: Vec<Vec<String>> = Vec::new();
    for keeping in [true, false] {
        super::joints::set_keeping(keeping);
        let (lua, engine) = start_play();
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        run(&lua, &engine, script);
        for round in 1..=10 {
            run(&lua, &engine, "shared.Round()");
            if let Err(message) = super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection) {
                panic!("keeping {keeping}, round {round}: {message}");
            }
        }
        logs.push(engine.state.borrow().output.entries.iter().map(|line| format!("{}: {}", line.kind, line.message)).collect());
    }
    super::joints::set_keeping(true);
    assert_eq!(logs[0].len(), 800, "a line a change: {:?}", logs[0].iter().find(|line| !line.starts_with("print")));
    for (index, (kept, fresh)) in logs[0].iter().zip(&logs[1]).enumerate() {
        assert_eq!(kept, fresh, "change {}: what the kept components answered, then what fresh walks did", index + 1);
    }
}

/// Model:PivotTo pays for each descendant it looks at that is no part (cost.rs PIVOT_DESCENDANT, a client's 11.3 ns), beyond what
/// it pays for the parts it moves: a Model holding a part with 1,000 Welds, Folders and Attachments under it, against one holding
/// a lone part.
#[test]
fn pivot_to_pays_for_the_descendants_it_looks_at() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Model(Children)
            local M = Instance.new("Model")
            local P = Instance.new("Part")
            P.Anchored, P.Parent = true, M
            for Index = 1, Children do
                local Child = Instance.new(if Index % 3 == 0 then "Weld" elseif Index % 3 == 1 then "Folder" else "Attachment")
                if Child:IsA("Weld") then
                    Child.Part0 = P
                end
                Child.Parent = P
            end
            M.Parent = workspace
            return M
        end
        local function Work(M)
            local At = M:GetPivot()
            M:PivotTo(At + Vector3.yAxis)
            M:PivotTo(At)
            local Start = os.clock()
            M:PivotTo(At + Vector3.yAxis)
            return os.clock() - Start
        end
        shared.Extra = Work(Model(1000)) - Work(Model(0))
        "#,
    );
    let want = 1000.0 * crate::cost::PIVOT_DESCENDANT;
    run(&lua, &engine, &format!("assert(math.abs(shared.Extra - {want:e}) < 1e-9, `PivotTo cost {{shared.Extra}} s more with 1,000 descendants, not {want:e}`)"));
}

/// Kept components answer as fresh walks do: 1,000 random changes to a scene of 30 parts and 45 rigid joints (moves between the
/// Workspace, a Folder, a Model, a WorldModel in it and one in storage, storage, JointsService and nil; joints re-pointed, enabled,
/// disabled, made and destroyed, parts cloned and destroyed; Anchored, Size, Massless, RootPriority, Name, Material and C0 writes;
/// CFrame, Position, PivotTo, BulkMoveTo and velocity writes), with every part's AssemblyRootPart, AssemblyMass,
/// AssemblyCenterOfMass, CFrame and velocity and each change's work read after each change, read the same with the components
/// kept as with every walk made afresh (joints.rs keeping), and the index and the kept components hold together.
#[test]
fn kept_components_answer_as_fresh_walks_do() {
    let script = r#"
        local Storage, Service = game:GetService("ReplicatedStorage"), game:GetService("JointsService")
        local World = Instance.new("WorldModel")
        World.Parent = workspace
        local Stored = Instance.new("WorldModel")
        Stored.Parent = Storage
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        local Model = Instance.new("Model")
        Model.Parent = workspace
        local Holders = { workspace, workspace, Folder, Model, Model, World, Stored, Storage, Service }
        local Dice = Random.new(23)
        local function Pick(List)
            return List[Dice:NextInteger(1, #List)]
        end
        local Parts, Joints = {}, {}
        local Names = { "Part", "HumanoidRootPart", "Torso", "Leg" }
        for I = 1, 30 do
            local P = Instance.new(if I % 10 == 0 then "Seat" elseif I % 7 == 0 then "WedgePart" else "Part")
            P.Name = Pick(Names)
            P.Anchored = Dice:NextInteger(1, 4) == 1
            P.CanCollide = false
            P.Size = Vector3.new(Dice:NextInteger(1, 4), Dice:NextInteger(1, 4), Dice:NextInteger(1, 4))
            P.CFrame = CFrame.new(Dice:NextNumber(-50, 50), Dice:NextNumber(0, 50), Dice:NextNumber(-50, 50)) * CFrame.Angles(Dice:NextNumber(0, 3), Dice:NextNumber(0, 3), 0)
            P.Parent = Pick(Holders)
            table.insert(Parts, P)
        end
        local Kinds = { "Weld", "Weld", "WeldConstraint", "Motor6D", "Snap" }
        local function Join()
            local J = Instance.new(Pick(Kinds))
            J.Part0, J.Part1 = Pick(Parts), Pick(Parts)
            if J:IsA("JointInstance") then
                J.C0 = CFrame.new(Dice:NextNumber(-3, 3), 1, 0)
            end
            J.Parent = if Dice:NextInteger(1, 5) == 1 then nil else Pick(Holders)
            table.insert(Joints, J)
        end
        for _ = 1, 45 do
            Join()
        end
        local function Numbers(...)
            local Out = {}
            for Index, Value in { ... } do
                Out[Index] = string.format("%.9g", Value)
            end
            return table.concat(Out, ",")
        end
        local function Digest(Op, Work)
            local Line = { Op, string.format("%.17g", Work) }
            for Index, P in Parts do
                local Root = P.AssemblyRootPart
                local Centre, Velocity = P.AssemblyCenterOfMass, P.AssemblyLinearVelocity
                table.insert(Line, string.format("%d:%d:%.9g:%s:%s:%s", Index, Root and table.find(Parts, Root) or 0, P.AssemblyMass,
                    Numbers(Centre.X, Centre.Y, Centre.Z), Numbers(P.CFrame:GetComponents()), Numbers(Velocity.X, Velocity.Y, Velocity.Z)))
            end
            print(table.concat(Line, " "))
        end
        local Ops = {
            function()
                local Moved = Pick(if Dice:NextInteger(1, 2) == 1 then Parts else Joints)
                local To = if Dice:NextInteger(1, 6) == 1 then nil else Pick(Holders)
                Moved.Parent = To
                return "parent"
            end,
            function()
                local J = Pick(Joints)
                if Dice:NextInteger(1, 2) == 1 then
                    J.Part0 = Pick(Parts)
                else
                    J.Part1 = if Dice:NextInteger(1, 5) == 1 then nil else Pick(Parts)
                end
                return "repoint"
            end,
            function()
                local J = Pick(Joints)
                J.Enabled = not J.Enabled
                return "enabled"
            end,
            function()
                local P = Pick(Parts)
                P.Anchored = not P.Anchored
                return "anchored"
            end,
            function()
                local P = Pick(Parts)
                local Roll = Dice:NextInteger(1, 5)
                if Roll == 1 then
                    P.Size = Vector3.new(Dice:NextInteger(1, 5), 1, Dice:NextInteger(1, 5))
                elseif Roll == 2 then
                    P.Massless = not P.Massless
                elseif Roll == 3 then
                    P.RootPriority = Dice:NextInteger(-2, 2)
                elseif Roll == 4 then
                    P.Name = Pick(Names)
                else
                    P.Material = if P.Material == Enum.Material.Plastic then Enum.Material.Metal else Enum.Material.Plastic
                end
                return "property"
            end,
            function()
                local P = Pick(Parts)
                P.CFrame = P.CFrame * CFrame.new(Dice:NextNumber(-2, 2), Dice:NextNumber(-2, 2), 0) * CFrame.Angles(0, Dice:NextNumber(-1, 1), 0)
                return "cframe"
            end,
            function()
                local P = Pick(Parts)
                P.Position += Vector3.new(Dice:NextNumber(-2, 2), 0, 1)
                return "position"
            end,
            function()
                Model:PivotTo(Model:GetPivot() * CFrame.new(1, 0, Dice:NextNumber(-1, 1)))
                return "pivot"
            end,
            function()
                local List, Frames = { Pick(Parts), Pick(Parts), Pick(Parts) }, {}
                for Index, P in List do
                    Frames[Index] = P.CFrame + Vector3.new(0, 1, 0)
                end
                workspace:BulkMoveTo(List, Frames)
                return "bulk"
            end,
            function()
                Pick(Parts).AssemblyLinearVelocity = Vector3.new(Dice:NextNumber(-1, 1), 0, 0)
                return "velocity"
            end,
            function()
                local J = Pick(Joints)
                if J:IsA("JointInstance") then
                    J.C0 = CFrame.new(Dice:NextNumber(-3, 3), 1, 0)
                end
                return "c0"
            end,
            function()
                local Roll = Dice:NextInteger(1, 4)
                if Roll == 1 then
                    Join()
                elseif Roll == 2 then
                    Pick(Joints):Destroy()
                elseif Roll == 3 then
                    Pick(Parts):Destroy()
                else
                    local Copy = Pick(Parts):Clone()
                    Copy.Parent = Pick(Holders)
                    table.insert(Parts, Copy)
                end
                return "make"
            end,
        }
        function shared.Round()
            for _ = 1, 100 do
                local Op = Pick(Ops)
                local Start = os.clock()
                local Ok, Name = pcall(Op)
                local Work = os.clock() - Start
                Digest(if Ok then Name else "raised", Work)
            end
        end
    "#;
    let mut logs: Vec<Vec<String>> = Vec::new();
    for keeping in [true, false] {
        super::joints::set_keeping(keeping);
        let (lua, engine) = start_play();
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        run(&lua, &engine, script);
        for round in 1..=10 {
            run(&lua, &engine, "shared.Round()");
            if let Err(message) = super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection) {
                panic!("keeping {keeping}, round {round}: {message}");
            }
        }
        logs.push(engine.state.borrow().output.entries.iter().map(|line| format!("{}: {}", line.kind, line.message)).collect());
    }
    super::joints::set_keeping(true);
    assert_eq!(logs[0].len(), 1000, "a line a change: {:?}", logs[0].iter().find(|line| !line.starts_with("print")));
    for (index, (kept, fresh)) in logs[0].iter().zip(&logs[1]).enumerate() {
        assert_eq!(kept, fresh, "change {}: what the kept components answered, then what fresh walks did", index + 1);
    }
}

/// A write that changes a part's property is priced by whether the part is in the Workspace (cost.rs write), read from the
/// homes the dom keeps (dom.rs Dom::is_descendant_of_home), so writes to a part 10,000 folders deep cost about as much host work
/// as to one right in its parent, and the work clock charges the same: Transparency, Position (a view's setter), CFrame and
/// PivotTo, to parts in the Workspace, in a WorldModel in it (which charges the same for the WorldModel above each: cost.rs
/// world_levels) and out of it. Each write used to walk up from the part to the Workspace, or to the top of the tree when it
/// was not in it: 20 us at that depth.
#[test]
fn part_writes_cost_the_same_however_deep_the_part_is() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    // (each pass's 2,000 rounds change the part 16 times: Transparency 1, Position, CFrame and PivotTo's 5 each)
    let world_level = 2000.0 * crate::cost::world_levels(1, 16);
    run(&lua, &engine, &format!("shared.WorldLevel = {world_level:e}"));
    run(
        &lua,
        &engine,
        r#"
        -- (a chain of folders `Depth` deep holding `Inner`, made from the bottom up so that making it walks nothing)
        local function Wrap(Inner, Depth)
            local Outer = Inner
            for _ = 1, Depth do
                local Folder = Instance.new("Folder")
                Outer.Parent = Folder
                Outer = Folder
            end
            return Outer
        end
        local function Scene(X, Depth, Where)
            local Part = Instance.new("Part")
            Part.Anchored, Part.CanCollide, Part.Size, Part.CFrame = true, false, Vector3.one, CFrame.new(X, 10, 0)
            Wrap(Part, Depth).Parent = Where
            return Part
        end
        local World = Instance.new("WorldModel")
        World.Parent = workspace
        local Storage = game:GetService("ReplicatedStorage")
        shared.Parts = {
            Shallow = Scene(0, 0, workspace), Deep = Scene(10, 10000, workspace),
            WorldShallow = Scene(20, 0, World), World = Scene(30, 10000, World),
            LooseShallow = Scene(40, 0, Storage), Loose = Scene(50, 10000, Storage),
        }
        shared.Work = {}
        local function Pass(Part)
            local X = Part.Position.X
            for I = 1, 2000 do
                Part.Transparency = I % 2 * 0.5
                Part.Position = Vector3.new(X, 10 + I % 2, 0)
                Part.CFrame = CFrame.new(X, 12 + I % 2, 0)
                Part:PivotTo(CFrame.new(X, 14 + I % 2, 0))
            end
        end
        -- each part twice, the second pass timed (the first warms the members and the part the frame touches)
        function shared.Use(Name)
            Pass(shared.Parts[Name])
            local Start = os.clock()
            Pass(shared.Parts[Name])
            shared.Work[Name] = os.clock() - Start
        end
        "#,
    );
    let mut host = std::collections::HashMap::new();
    for name in ["Shallow", "Deep", "WorldShallow", "World", "LooseShallow", "Loose"] {
        let start = std::time::Instant::now();
        run(&lua, &engine, &format!("shared.Use(\"{name}\")"));
        host.insert(name, start.elapsed().as_secs_f64());
    }
    run(
        &lua,
        &engine,
        r#"
        local Work = shared.Work
        for _, Pair in { { "Deep", "Shallow" }, { "World", "WorldShallow" }, { "Loose", "LooseShallow" } } do
            local Deep, Shallow = Work[Pair[1]], Work[Pair[2]]
            assert(Shallow > 0 and math.abs(Deep - Shallow) < 1e-9, `{Pair[1]}'s pass cost {Deep} s of work where {Pair[2]}'s {Shallow}`)
        end
        assert(math.abs(Work.World - Work.Deep - shared.WorldLevel) < 1e-9, `a part in a WorldModel in the Workspace pays {Work.World} s where one in the Workspace {Work.Deep}`)
        assert(Work.Loose < Work.Deep - 8000 * 0.1e-6, `a part out of the Workspace pays {Work.Loose} s, one in it {Work.Deep}`)
        "#,
    );
    for (deep, shallow) in [("Deep", "Shallow"), ("World", "WorldShallow"), ("Loose", "LooseShallow")] {
        let (deep_s, shallow_s) = (host[deep], host[shallow]);
        assert!(deep_s < 2.0 * shallow_s + 0.05, "two passes of 8,000 writes took {deep_s:.3} s of host time to a part 10,000 folders deep ({deep}), {shallow_s:.3} s right in its parent");
    }
}

/// A change to a part below WorldModels nested one in another costs it, beyond the write's price, what the WorldModels above it
/// cost a playing client (cost.rs world_levels and part_changes: 0.81 to 1.22x a 0.747 client's from 30 to 3,000 levels), read
/// from what the dom keeps above the part's home (dom.rs Dom::homes_above), never a walk: below 300 WorldModels, in the
/// Workspace and out of it, Transparency, CFrame and Parent writes, a part's PivotTo and a Model's (one change a part) pay exactly
/// that more than below 300 Folders, and a write that changes nothing and a PivotTo that moves nothing pay nothing more; such a
/// PivotTo below 10,000 WorldModels takes about the host time it takes below none. Each check used to walk up through every
/// WorldModel to the top for the call's flat price: a PivotTo that moved nothing walked them twice (28 us at 10,000).
#[test]
fn a_part_below_worldmodels_pays_for_them_in_one_lookup() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let levels = |changes: u32| crate::cost::world_levels(300, changes);
    run(&lua, &engine, &format!("shared.Levels = {{ [0] = 0, {:e}, {:e}, {:e}, {:e}, {:e} }}", levels(1), levels(2), levels(3), levels(4), levels(5)));
    run(
        &lua,
        &engine,
        r#"
        -- (a part, a Folder and a Model of 4 parts at the bottom of `Levels` instances of `Kind` nested one in another, made from
        -- the top down in `Where`)
        local function Chain(Kind, Levels, Where)
            local Top = Instance.new("Folder")
            Top.Parent = Where
            local Deepest = Top
            for _ = 1, Levels do
                local Level = Instance.new(Kind)
                Level.Parent = Deepest
                Deepest = Level
            end
            local function Part(Parent, X)
                local Made = Instance.new("Part")
                Made.Anchored, Made.CanCollide, Made.Size, Made.CFrame = true, false, Vector3.one, CFrame.new(300 + X, 300, 300)
                Made.Parent = Parent
                return Made
            end
            local Model = Instance.new("Model")
            for I = 1, 4 do
                Part(Model, I * 5)
            end
            Model.Parent = Deepest
            local Inner = Instance.new("Folder")
            Inner.Parent = Deepest
            return { Part = Part(Deepest, 0), Model = Model, Deepest = Deepest, Inner = Inner }
        end
        local Storage = game:GetService("ReplicatedStorage")
        shared.Scenes = {
            Worlds = Chain("WorldModel", 300, workspace), Folders = Chain("Folder", 300, workspace),
            LooseWorlds = Chain("WorldModel", 300, Storage), LooseFolders = Chain("Folder", 300, Storage),
            Deep = Chain("WorldModel", 10000, Storage), Shallow = Chain("WorldModel", 0, Storage),
        }
        -- what 10 changes of each kind cost the work clock (and how many changes a client makes of a part for one)
        local Ops = {
            { "Transparency", 1, function(S, I) S.Part.Transparency = I % 2 end },
            { "CFrame", 5, function(S, I) S.Part.CFrame = CFrame.new(300, 300 + I % 2, 300) end },
            { "Parent", 1, function(S, I) S.Part.Parent = if I % 2 == 1 then S.Inner else S.Deepest end },
            { "PivotTo", 5, function(S, I) S.Part:PivotTo(CFrame.new(300, 302 + I % 2, 300)) end },
            { "ModelPivot", 4, function(S, I) S.Model:PivotTo(CFrame.new(310, 300 + I % 2, 300)) end },
            { "Unchanged", 0, function(S) S.Part.Transparency = S.Part.Transparency end },
            { "PivotSame", 0, function(S) S.Part:PivotTo(S.Part:GetPivot()) end },
        }
        -- (after one change, so that the frame's first change of each kind, which runs the engine's code for it cold, is not timed)
        local function Cost(Scene, Op)
            Op[3](Scene, 1)
            local Start = os.clock()
            for I = 2, 11 do
                Op[3](Scene, I)
            end
            return os.clock() - Start
        end
        for _, Pair in { { "Worlds", "Folders" }, { "LooseWorlds", "LooseFolders" } } do
            for _, Op in Ops do
                local Worlds, Folders = Cost(shared.Scenes[Pair[1]], Op), Cost(shared.Scenes[Pair[2]], Op)
                local Expected = 10 * shared.Levels[Op[2]]
                assert(math.abs(Worlds - Folders - Expected) < 1e-9, `{Op[1]} below 300 WorldModels ({Pair[1]}) cost {Worlds} s where below Folders {Folders}: {Expected} s more expected`)
            end
        end
        function shared.PivotSame(Name)
            local Part = shared.Scenes[Name].Part
            for _ = 1, 2000 do
                Part:PivotTo(Part:GetPivot())
            end
        end
        "#,
    );
    let mut host = std::collections::HashMap::new();
    for name in ["Shallow", "Deep", "Shallow", "Deep"] {
        let start = std::time::Instant::now();
        run(&lua, &engine, &format!("shared.PivotSame(\"{name}\")"));
        host.insert(name, start.elapsed().as_secs_f64());
    }
    let (deep, shallow) = (host["Deep"], host["Shallow"]);
    assert!(deep < 2.0 * shallow + 0.02, "2,000 PivotTo calls that moved nothing took {deep:.3} s of host time to a part below 10,000 WorldModels, {shallow:.3} s below none");
}

/// The physics step asks which world each loose part is in from the homes the dom keeps, and whether a parent holds a Humanoid
/// once a step for each parent: 200 loose parts at the bottom of a chain of 20,000 folders fly for 60 frames, and 5,000 loose
/// parts in one folder for 30, on little host work. The step used to walk up from every loose part to the Workspace each frame
/// (4 million steps a frame in the first case), the first step from every instance in the Workspace (200 million), and look
/// through every sibling of every loose part for a Humanoid each frame (25 million in the second).
#[test]
fn free_flight_steps_cost_the_same_however_deep_or_wide_the_parts_are() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r#"
        local Outer = Instance.new("Folder")
        for I = 1, 200 do
            local Part = Instance.new("Part")
            Part.Size, Part.CanCollide, Part.CFrame = Vector3.one, false, CFrame.new(I * 3, 1000, 0)
            Part.Parent = Outer
        end
        shared.First = Outer:GetChildren()[1]
        for _ = 1, 20000 do
            local Folder = Instance.new("Folder")
            Outer.Parent = Folder
            Outer = Folder
        end
        Outer.Parent = workspace
        "#,
    );
    let start = std::time::Instant::now();
    for _ in 0..60 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let host = start.elapsed().as_secs_f64();
    run(&lua, &engine, r#"assert(shared.First.Position.Y < 950, `the part fell to {shared.First.Position.Y}`)"#);
    assert!(host < 0.25, "60 frames of 200 parts flying 20,000 deep took {host:.3} s of host time");

    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r#"
        local Folder = Instance.new("Folder")
        for I = 1, 5000 do
            local Part = Instance.new("Part")
            Part.Size, Part.CanCollide, Part.CFrame = Vector3.one, false, CFrame.new(I * 3, 1000, 0)
            Part.Parent = Folder
        end
        shared.First = Folder:GetChildren()[1]
        Folder.Parent = workspace
        "#,
    );
    let start = std::time::Instant::now();
    for _ in 0..30 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    let host = start.elapsed().as_secs_f64();
    run(&lua, &engine, r#"assert(shared.First.Position.Y < 990, `the part fell to {shared.First.Position.Y}`)"#);
    assert!(host < 0.2, "30 frames of 5,000 parts flying in one folder took {host:.3} s of host time");
}

/// A script cannot overflow the engine's stack with a deep tree: the engine's walks of the tree keep stacks of their own. On a
/// thread with libtest's 2 MiB stack (a recursion over these chains overflows it at 20 bytes a level, 200 for the one destroyed):
/// a chain of 100,000 Folders with a Part at the bottom holding 50,000 nested Attachments and 50,000 nested Bones below them, a
/// CurveAnimation of 100,000 nested Folders, and a chain of 10,000 Folders, each moved into ReplicatedStorage, and a model of
/// 100,000 nested Folders imported there lazily (its instances made as a script reaches them), with a Studio place's hierarchy
/// loaded. The script first lists the bottom part's children (its Studio node worked out up the whole chain), then searches the chain
/// (FindFirstChild and FindFirstChildWhichIsA recursive, QueryDescendants) and the import's plan, clones the chain, reads the bottom bone's and
/// attachment's world frames and moves the chain into the Workspace (AncestryChanged's order, the move's price), where it is stepped
/// (the physics step's first walk) and queried (the spatial index), then out; it reads the clip's length and the KeyframeSequence
/// made of it, and destroys the last chain (a destroy unparents each child in turn, so it takes as long as the chain's depth
/// squared). Nothing lists the deep chains: wrapping an instance for a script walks up to its root.
#[test]
fn a_tree_deeper_than_a_stack_holds_frames_for_runs() {
    const DEPTH: usize = 100_000;
    const HALF: usize = DEPTH / 2;
    const DESTROYED: usize = 10_000;
    let body = || {
        let (lua, engine) = start_play();
        {
            // (built here, a level at a time: a script's Parent write under the bottom of a chain walks up it)
            let reflection = engine.reflection.clone();
            let class = |name: &str| reflection.class_id(name).unwrap();
            let storage = crate::services::get_service(&engine, engine.world().game, "ReplicatedStorage").expect("ReplicatedStorage");
            let chain = |top: crate::InstanceId, class_name: &str, depth: usize, last: &str| {
                let mut state = engine.state.borrow_mut();
                let mut at = top;
                for level in 1..=depth {
                    let made = crate::instance::create_named(&mut state, &reflection, class(class_name), if level == depth { last } else { class_name });
                    state.dom.attach(made, at);
                    at = made;
                }
                at
            };
            let root = |class_name: &str, name: &str| crate::instance::create_named(&mut engine.state.borrow_mut(), &reflection, class(class_name), name);
            let tree = root("Folder", "DeepTree");
            let bottom = chain(chain(tree, "Folder", DEPTH, "Folder"), "Part", 1, "Bottom");
            chain(chain(bottom, "Attachment", HALF, "LastAttachment"), "Bone", HALF, "LastBone");
            let clip = root("CurveAnimation", "DeepClip");
            chain(clip, "Folder", DEPTH, "BottomFolder");
            let doomed = root("Folder", "DeepDestroy");
            chain(doomed, "Folder", DESTROYED, "Folder");
            for top in [tree, clip, doomed] {
                engine_lua::instance::set_parent(&engine, top, Some(storage)).expect("moved into ReplicatedStorage");
            }
            let mut model = rbx_dom_weak::WeakDom::new(rbx_dom_weak::InstanceBuilder::new("DataModel"));
            let import = model.insert(model.root_ref(), rbx_dom_weak::InstanceBuilder::new("Folder").with_name("DeepImport"));
            let mut at = import;
            for _ in 0..DEPTH {
                at = model.insert(at, rbx_dom_weak::InstanceBuilder::new("Folder"));
            }
            model.insert(at, rbx_dom_weak::InstanceBuilder::new("Part").with_name("ImportBottom"));
            let plan = std::sync::Arc::new(crate::project::ModelPlan::new(&reflection, &model, &[import]));
            plan.instantiate_lazy(&mut engine.state.borrow_mut(), &reflection, storage);
            // a place's Studio hierarchy (the game and its ReplicatedStorage), and an ObjectValue that hands the script the bottom part
            // before anything above it was looked at
            let place = std::env::temp_dir().join(format!("ree-deep-tree-{}", std::process::id()));
            std::fs::create_dir_all(place.join("place")).unwrap();
            std::fs::create_dir_all(place.join(crate::project::SNAPSHOTS_DIR)).unwrap();
            std::fs::write(place.join(crate::project::PLACES_FILE), r#"{"environments":[{"name":"Deep","projectFile":"place/default.project.json"}]}"#).unwrap();
            std::fs::write(
                place.join(crate::project::SNAPSHOTS_DIR).join("Deep.json"),
                r#"{"nodes":[{"name":"game","className":"DataModel","parent":-1},{"name":"ReplicatedStorage","className":"ReplicatedStorage","parent":0}]}"#,
            )
            .unwrap();
            let roots = [place.join("place").to_string_lossy().replace('\\', "/")];
            assert!(crate::project::use_studio_hierarchy(&engine, &roots).is_some(), "the place's hierarchy is loaded");
            let _ = std::fs::remove_dir_all(&place);
            let pointer = root("ObjectValue", "BottomRef");
            crate::instance::raw_set_silent(&mut engine.state.borrow_mut(), &reflection, pointer, reflection.name_id("Value").unwrap(), crate::value::Value::Instance(bottom));
            engine_lua::instance::set_parent(&engine, pointer, Some(storage)).expect("moved into ReplicatedStorage");
        }
        run(
            &lua,
            &engine,
            r#"
            local Storage = game:GetService("ReplicatedStorage")
            local Tree, Clip, Doomed, Import = Storage.DeepTree, Storage.DeepClip, Storage.DeepDestroy, Storage.DeepImport
            assert(#Storage.BottomRef.Value:GetChildren() == 1, "the bottom part holds the first attachment")
            -- (the import's plan searched before any of its instances is made)
            assert(Import:FindFirstChild("Missing", true) == nil, "nothing in the import is named Missing")
            local Imported = Import:FindFirstChildWhichIsA("BasePart", true)
            assert(Imported and Imported.Name == "ImportBottom" and Imported:IsDescendantOf(Import), "the import's part is found and made")
            assert(Tree:FindFirstChild("Missing", true) == nil, "nothing is named Missing")
            local Bottom = Tree:FindFirstChild("Bottom", true)
            assert(Bottom and Bottom:IsA("Part"), "the part at the bottom is found by name")
            local Bone = Tree:FindFirstChildWhichIsA("Bone", true)
            assert(Bone and Bone.Parent.Name == "LastAttachment", "the first bone is found by class")
            local Parts = Tree:QueryDescendants("Part")
            assert(#Parts == 1 and Parts[1] == Bottom, "the query finds the part")
            local Copy = Tree:Clone()
            local Copied = Copy:FindFirstChild("LastBone", true)
            assert(Copied and Copied ~= Bottom:FindFirstChild("LastBone", true), "the clone holds the whole chain")
            local LastBone, LastAttachment = Bottom:FindFirstChild("LastBone", true), Bottom:FindFirstChild("LastAttachment", true)
            assert((LastBone.TransformedWorldCFrame.Position - Bottom.Position).Magnitude < 1e-3, "the bottom bone hangs where the part is")
            assert((LastAttachment.WorldPosition - Bottom.Position).Magnitude < 1e-3, "the bottom attachment hangs where the part is")
            -- (a part that can collide is held where it is: world/gravity.rs)
            Bottom.CanCollide = false
            shared.Start, shared.Bottom = Bottom.Position.Y, Bottom
            Tree.Parent = workspace
            -- the clip: one key at its start, so its KeyframeSequence is one keyframe of 100,000 nested Poses
            local Position = Instance.new("Vector3Curve")
            Position.Name = "Position"
            Position.Parent = Clip:FindFirstChild("BottomFolder", true)
            local X = Instance.new("FloatCurve")
            X.Name = "X"
            X:SetKeys({ FloatCurveKey.new(0, 2, Enum.KeyInterpolationMode.Linear) })
            X.Parent = Position
            assert(Clip.Length == 0, `the clip's length {Clip.Length}`)
            local Id = game:GetService("AnimationClipProvider"):RegisterAnimationClip(Clip)
            task.spawn(function()
                shared.Made = game:GetService("KeyframeSequenceProvider"):GetKeyframeSequenceAsync(Id)
            end)
            Doomed:Destroy()
            assert(Doomed.Parent == nil and #Doomed:GetChildren() == 0, "the chain is destroyed")
            "#,
        );
        for _ in 0..10 {
            crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        }
        run(
            &lua,
            &engine,
            r#"
            local Bottom, Made = shared.Bottom, shared.Made
            assert(Bottom.Position.Y < shared.Start, `the part fell to {Bottom.Position.Y}`)
            local Near = workspace:GetPartBoundsInRadius(Bottom.Position, 5)
            assert(table.find(Near, Bottom), "the spatial index holds the part")
            workspace.DeepTree.Parent = nil
            assert(not Bottom:IsDescendantOf(workspace) and Bottom:FindFirstChild("LastBone", true), "the chain left the Workspace whole")
            assert(Made and #Made:GetChildren() == 1, "the KeyframeSequence has one keyframe")
            local Pose = Made:FindFirstChild("BottomFolder", true)
            assert(Pose and Pose.CFrame.X == 2 and Pose.Weight == 1, "the bottom Folder's Pose is posed, at the bottom of the keyframe's")
            "#,
        );
    };
    let thread = std::thread::Builder::new().name("a deep tree on a 2 MiB stack".into()).stack_size(2 << 20).spawn(body).expect("thread");
    thread.join().expect("the deep tree ran");
}

/// WorldRoot:BulkMoveTo checks its lists as a Studio 0.746 playing client does, with its messages, before any part moves; a
/// nil in the part list keeps the lists paired by index; destroyed, parentless and stored parts move.
#[test]
fn bulk_move_to_checks_its_lists_as_a_playing_client_does() {
    check(
        r#"
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        local function Part(X, Parent)
            local P = Instance.new("Part")
            P.Anchored = true
            P.CFrame = CFrame.new(X, 50, 0)
            P.Parent = Parent or Folder
            return P
        end
        local function Fails(Want, Function)
            local Ok, Message = pcall(Function)
            assert(not Ok and Message == Want, `{Want}: got {Ok} {Message}`)
        end
        local A, B, C = Part(0), Part(10), Part(20)
        -- lists of different lengths throw before any part moves
        Fails("Length of part list (2) and length of CFrame list (1) do not match.", function()
            workspace:BulkMoveTo({ A, B }, { CFrame.new(1, 60, 0) })
        end)
        assert(A.Position == Vector3.new(0, 50, 0) and B.Position == Vector3.new(10, 50, 0), "a part moved before the error")
        Fails("Length of part list (1) and length of CFrame list (2) do not match.", function()
            workspace:BulkMoveTo({ A }, { CFrame.new(2, 60, 0), CFrame.new(3, 60, 0) })
        end)
        Fails("Length of part list (3) and length of CFrame list (2) do not match.", function()
            workspace:BulkMoveTo({ A, B, C }, { CFrame.new(), CFrame.new() }, Enum.BulkMoveMode.FireCFrameChanged)
        end)
        Fails("Length of part list (0) and length of CFrame list (1) do not match.", function()
            workspace:BulkMoveTo({}, { CFrame.new() })
        end)
        workspace:BulkMoveTo({}, {})
        -- the entries and the arguments
        Fails("Can only call BulkMoveTo on BaseParts, partList[1] is a Folder.", function()
            workspace:BulkMoveTo({ Folder }, { CFrame.new() })
        end)
        Fails("Unable to cast value to Object", function()
            workspace:BulkMoveTo({ 5 } :: any, { CFrame.new() })
        end)
        Fails("CFrame list must only contain CFrames, cframeList[1] is a Vector3.", function()
            workspace:BulkMoveTo({ A }, { Vector3.new() } :: any)
        end)
        Fails("Unable to cast CoordinateFrame to Array", function()
            workspace:BulkMoveTo(A :: any, CFrame.new() :: any)
        end)
        Fails("Argument 2 missing or nil", function()
            (workspace :: any):BulkMoveTo({ A })
        end)
        Fails("Unable to cast Dictionary to Array", function()
            workspace:BulkMoveTo({ x = A } :: any, { x = CFrame.new() } :: any)
        end)
        -- (Studio 0.748, tools/parity bulk_move_args) the mode is cast first, then the frame list; the part list is a list of
        -- Instances (named as type() names a value, a dictionary an empty list) whose entries are cast before the lengths compare
        Fails("Unable to cast string to BulkMoveMode", function()
            workspace:BulkMoveTo("x" :: any, "y" :: any, "z" :: any)
        end)
        Fails("Unable to cast KeyCode to BulkMoveMode", function()
            workspace:BulkMoveTo({ A }, { CFrame.new() }, Enum.KeyCode.A :: any)
        end)
        Fails("Expected table argument, got userdata", function()
            workspace:BulkMoveTo(A :: any, {})
        end)
        Fails("Expected table argument, got nil", function()
            workspace:BulkMoveTo(nil :: any, {})
        end)
        Fails("Unable to cast Instance to Array", function()
            workspace:BulkMoveTo({}, A :: any)
        end)
        Fails("Unable to cast value to Object", function()
            workspace:BulkMoveTo({ Vector3.one } :: any, {})
        end)
        Fails("CFrame list must only contain CFrames, cframeList[1] is a double.", function()
            workspace:BulkMoveTo({ A }, { 5 } :: any)
        end)
        workspace:BulkMoveTo({ x = A } :: any, {})
        workspace:BulkMoveTo({ A }, { A.CFrame }, "FireCFrameChanged" :: any)
        -- a nil in the part list leaves its frame unused: the lists pair by index
        workspace:BulkMoveTo({ A, nil, B }, { CFrame.new(14, 60, 0), CFrame.new(15, 60, 0), CFrame.new(16, 60, 0) })
        assert(A.Position == Vector3.new(14, 60, 0) and B.Position == Vector3.new(16, 60, 0), "the lists did not pair by index")
        -- destroyed parts, parts with no parent and parts outside the Workspace move
        local Gone = Part(30)
        Gone:Destroy()
        local Loose = Instance.new("Part")
        local Stored = Part(50, game:GetService("ReplicatedStorage"))
        workspace:BulkMoveTo({ Gone, Loose, Stored, A }, { CFrame.new(9, 60, 0), CFrame.new(10, 60, 0), CFrame.new(11, 60, 0), CFrame.new(13, 60, 0) })
        assert(Gone.Position == Vector3.new(9, 60, 0), "the destroyed part did not move")
        assert(Loose.Position == Vector3.new(10, 60, 0), "the part with no parent did not move")
        assert(Stored.Position == Vector3.new(11, 60, 0), "the part outside the Workspace did not move")
        assert(A.Position == Vector3.new(13, 60, 0), "the part beside them did not move")
        "#,
    );
}

/// A list of Instances costs the work clock what a playing client spends reading it, also when the call then raises (world/mod.rs
/// ignore_list and bulk_moves; cost.rs LIST_CAST_FAILED, BULK_LISTED, BULK_HOLE, IGNORE_LISTED and IGNORE_HOLE, Studio 0.747): a
/// BulkMoveTo whose lists' lengths differ pays for each of its entries, one whose last entry is a number for each entry
/// before it, and so do GetPartsObscuringTarget and GetLargestCutoffDistance before a number; their lists cost each entry and
/// each hole when the call goes on, and a BulkMoveTo that moves each of its holes. Each of these read its whole list for nothing.
#[test]
fn list_reads_cost_what_a_client_spends_reading_them() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Pool = {}
        for I = 1, 10000 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Pool[I] = Part
        end
        local function Copy(Count, Tail)
            local List = table.move(Pool, 1, Count, 1, table.create(Count + 1))
            if Tail ~= nil then
                table.insert(List, Tail)
            end
            return List
        end
        local function Holes(Count, Last)
            local List = table.create(Count + 1)
            List[Count + 1] = Last
            return List
        end
        local Part = Pool[1]
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CFrame = CFrame.new(0, 5000, 0)
        Cam.Focus = CFrame.new(0, 5000, -10)
        local Many, ManyBad, ManyHoles, Lots = Copy(10000), Copy(10000, 5), Holes(10000, Part), Holes(100000, Part)
        local Frames, LotsFrames, One = table.create(10001, CFrame.new()), table.create(100001, CFrame.new()), { CFrame.new() }
        -- (lengths of as many digits, so that both errors are as long)
        local Nine, NineFrames, Thousand, ThousandFrames = Copy(9000), table.create(9001, CFrame.new()), Copy(1000), table.create(1001, CFrame.new())
        local Fast = Enum.BulkMoveMode.FireCFrameChanged
        -- (every list made here: a list made in the timed call costs its instructions)
        local None, Bad, Single, NoPoints = {}, { 5 }, { Part }, {}
        -- (each call once before it is timed: a frame's first call of an engine function is priced cold)
        local function Cost(Call)
            pcall(Call)
            local Start = os.clock()
            pcall(Call)
            return os.clock() - Start
        end
        local function Extra(Big, Small)
            return Cost(Big) - Cost(Small)
        end
        shared.Extra = {
            BulkLengths = Extra(function() workspace:BulkMoveTo(Nine, NineFrames, Fast) end, function() workspace:BulkMoveTo(Thousand, ThousandFrames, Fast) end),
            BulkCast = Extra(function() workspace:BulkMoveTo(ManyBad, Frames, Fast) end, function() workspace:BulkMoveTo(Bad, One, Fast) end),
            BulkHoles = Extra(function() workspace:BulkMoveTo(Lots, LotsFrames, Fast) end, function() workspace:BulkMoveTo(Single, One, Fast) end),
            ObscuringCast = Extra(function() Cam:GetPartsObscuringTarget(NoPoints, ManyBad) end, function() Cam:GetPartsObscuringTarget(NoPoints, Bad) end),
            ObscuringListed = Extra(function() Cam:GetPartsObscuringTarget(NoPoints, Many) end, function() Cam:GetPartsObscuringTarget(NoPoints, None) end),
            ObscuringHoles = Extra(function() Cam:GetPartsObscuringTarget(NoPoints, ManyHoles) end, function() Cam:GetPartsObscuringTarget(NoPoints, Single) end),
            CutoffCast = Extra(function() Cam:GetLargestCutoffDistance(ManyBad) end, function() Cam:GetLargestCutoffDistance(Bad) end),
            CutoffListed = Extra(function() Cam:GetLargestCutoffDistance(Many) end, function() Cam:GetLargestCutoffDistance(None) end),
            CutoffHoles = Extra(function() Cam:GetLargestCutoffDistance(ManyHoles) end, function() Cam:GetLargestCutoffDistance(Single) end),
        }
        "##,
    );
    let cost = |name: &str, seconds: f64| format!("assert(math.abs(shared.Extra.{name} - {seconds:e}) < 1e-9, `{name}: {{shared.Extra.{name}}} s more work, not {seconds:e}`)\n");
    let mut checks = String::new();
    checks += &cost("BulkLengths", 8_000.0 * crate::cost::BULK_LISTED);
    checks += &cost("BulkCast", 10_000.0 * crate::cost::LIST_CAST_FAILED);
    checks += &cost("BulkHoles", 100_000.0 * crate::cost::BULK_HOLE);
    checks += &cost("ObscuringCast", 10_000.0 * crate::cost::LIST_CAST_FAILED);
    checks += &cost("ObscuringListed", 10_000.0 * crate::cost::IGNORE_LISTED);
    checks += &cost("ObscuringHoles", 10_000.0 * crate::cost::IGNORE_HOLE);
    checks += &cost("CutoffCast", 10_000.0 * crate::cost::LIST_CAST_FAILED);
    checks += &cost("CutoffListed", 10_000.0 * crate::cost::IGNORE_LISTED);
    checks += &cost("CutoffHoles", 10_000.0 * crate::cost::IGNORE_HOLE);
    run(&lua, &engine, &checks);
}

#[test]
fn an_anchored_root_stays_when_a_pose_lands_while_it_moves() {
    check_running(
        "play",
        r#"
        local RunService = game:GetService("RunService")
        task.wait(0.1)
        local Rig = Instance.new("Model")
        local function Part(Name, Size, At)
            local Made = Instance.new("Part")
            Made.Name, Made.Size, Made.CFrame = Name, Size, At
            Made.Anchored = Name == "HumanoidRootPart"
            Made.Massless = Name ~= "HumanoidRootPart"
            Made.Parent = Rig
            return Made
        end
        local Root = Part("HumanoidRootPart", Vector3.new(2, 2, 1), CFrame.new(0, 3, 0))
        local Torso = Part("Torso", Vector3.new(2, 2, 1), CFrame.new(0, 3, 0))
        local Head = Part("Head", Vector3.new(2, 1, 1), CFrame.new(0, 4.5, 0))
        local Arm = Part("Right Arm", Vector3.new(1, 2, 1), CFrame.new(1.5, 3, 0))
        local function Motor(Name, Part0, Part1, C0, C1)
            local Made = Instance.new("Motor6D")
            Made.Name, Made.Part0, Made.Part1, Made.C0, Made.C1 = Name, Part0, Part1, C0, C1
            Made.Parent = Part0
            return Made
        end
        local RootJoint = Motor("RootJoint", Root, Torso, CFrame.identity, CFrame.identity)
        local Neck = Motor("Neck", Torso, Head, CFrame.new(0, 1, 0), CFrame.new(0, -0.5, 0))
        Motor("Right Shoulder", Torso, Arm, CFrame.new(1, 0.5, 0), CFrame.new(-0.5, 0.5, 0))
        Rig.PrimaryPart = Root
        Rig.Parent = workspace

        -- the pose lands a defer later, and the rig is scaled about its root in the same frame: the
        -- scale writes the root where it is (it counts as written) and moves the rest
        local Lean = CFrame.Angles(0, 0, math.rad(80))
        local Done = false
        task.defer(function()
            RootJoint.Transform = Lean
            Neck.Transform = CFrame.Angles(math.rad(40), 0, 0)
            Rig:ScaleTo(2)
            Done = true
        end)
        task.wait()
        assert(Done, "the defer did not run")
        assert(near(Root.CFrame, CFrame.new(0, 3, 0)), "the pose moved the anchored root")
        assert(near(Torso.CFrame, Root.CFrame * Lean), "the pose did not lean the torso")

        -- a limb's CFrame written while a Transform waits moves the root as the limb moved, never
        -- swung about the limb by the waiting Transform
        RootJoint.Transform = CFrame.Angles(0, math.rad(90), 0)
        Head.CFrame = Head.CFrame + Vector3.new(5, 0, 0)
        -- (within the float noise of the posed head's rotation: no swing is 1e-4 of a radian)
        local Upright = Root.CFrame.UpVector:Dot(Vector3.yAxis) > 0.9999 and Root.CFrame.LookVector:Dot(-Vector3.zAxis) > 0.9999
        assert(Upright and (Root.Position - Vector3.new(5, 3, 0)).Magnitude < 1e-3, "the waiting Transform swung the root")
        Root.CFrame = CFrame.new(5, 3, 0)

        -- moved each frame by Model:PivotTo, it stays upright
        local Frames = 0
        local Connection = RunService.Heartbeat:Connect(function()
            Frames += 1
            Rig:PivotTo(CFrame.new(5, 3, -0.25 * Frames))
            assert(near(Root.CFrame, CFrame.new(5, 3, -0.25 * Frames)), "PivotTo tilted the anchored root")
        end)
        task.wait(0.1)
        Connection:Disconnect()
        assert(Frames > 3, "the rig did not move")
        assert(near(Torso.CFrame, Root.CFrame * CFrame.Angles(0, math.rad(90), 0)), "the torso lost its pose")
        "#,
        0.5,
    );
}

// Free fall (gravity.rs)

#[test]
fn a_free_assembly_falls_under_gravity() {
    check_running(
        "play",
        r#"
        local function part(name, anchored, collide, y)
            local P = Instance.new("Part")
            P.Name = name
            P.Anchored = anchored
            P.CanCollide = collide
            P.CFrame = CFrame.new(0, y, 0)
            return P
        end
        local Free = part("Free", false, false, 500)
        local Welded = part("Welded", false, false, 510)
        local Weld = Instance.new("WeldConstraint")
        Weld.Part0, Weld.Part1 = Free, Welded
        Weld.Parent = Free
        local Colliding = part("Colliding", false, true, 500)
        local Anchored = part("Anchored", true, false, 500)
        local Held = part("Held", false, false, 500)
        local HeldTo = part("HeldTo", true, false, 600)
        local Joint = Instance.new("WeldConstraint")
        Joint.Part0, Joint.Part1 = Held, HeldTo
        Joint.Parent = Held
        local Elsewhere = part("Elsewhere", false, false, 500)
        local World = Instance.new("WorldModel")
        World.Parent = workspace
        Elsewhere.Parent = World
        for _, P in { Free, Welded, Colliding, Anchored, Held, HeldTo } do
            P.Parent = workspace
        end
        local Fired = 0
        Free:GetPropertyChangedSignal("CFrame"):Connect(function() Fired += 1 end)
        local Start = os.clock()
        task.wait(1)
        local T = os.clock() - Start
        local Drop = 500 - Free.Position.Y
        -- ½ g t² within a frame's fall (Studio: 37.5 studs at 0.62 s, 99.8 at 0.99)
        assert(math.abs(Drop - 0.5 * workspace.Gravity * T * T) < 4, `fell {Drop} in {T}`)
        assert(math.abs((510 - Welded.Position.Y) - Drop) < 1e-2, "the welded part did not fall with it")
        assert(Free.AssemblyLinearVelocity.Y < -150 or Free.Velocity.Y < -150, "no velocity")
        assert(Fired == 0, "physics fired CFrame changes")
        for _, P in { Colliding, Anchored, Held, Elsewhere } do
            assert(P.Position.Y == 500, P.Name .. " moved")
        end
        Anchored.Anchored = false
        task.wait(0.1)
        assert(Anchored.Position.Y < 500, "unanchoring did not free it")
        "#,
        1.5,
    );
}

#[test]
fn nothing_falls_in_edit_mode() {
    check_running(
        "edit",
        r#"
        local P = Instance.new("Part")
        P.Anchored = false
        P.CanCollide = false
        P.CFrame = CFrame.new(0, 500, 0)
        P.Parent = workspace
        task.wait(0.5)
        assert(P.Position.Y == 500, "a part fell in edit mode")
        "#,
        1.0,
    );
}

#[test]
fn a_thrown_part_flies_as_a_playing_client_steps_it() {
    // Measured in a playing client (notes/macro flyprobe.luau): velocity first, then the move by it, four 1/240 s steps on
    // the first simulated frame and one 1/60 s step a frame after; a velocity set by AssemblyLinearVelocity, the legacy
    // Velocity, ApplyImpulse or on a welded partner flies the same path
    check_running(
        "play",
        r#"
        local RunService = game:GetService("RunService")
        local function part(x)
            local P = Instance.new("Part")
            P.Size = Vector3.new(2, 2, 2)
            P.Anchored = false
            P.CanCollide = false
            P.CFrame = CFrame.new(x, 500, 0)
            P.Parent = workspace
            return P
        end
        local Throw = Vector3.new(30, 60, -20)
        local Thrown, Legacy, Pushed, Dropped = part(0), part(10), part(20), part(30)
        Thrown.AssemblyLinearVelocity = Throw
        Legacy.Velocity = Throw
        Pushed:ApplyImpulse(Throw * Pushed:GetMass())
        assert(Pushed.AssemblyLinearVelocity == Vector3.zero, "an impulse acts at the next physics step")
        local A, B = part(40), part(43)
        local Joint = Instance.new("WeldConstraint")
        Joint.Part0, Joint.Part1 = A, B
        Joint.Parent = A
        B.AssemblyLinearVelocity = Throw
        assert(A.AssemblyLinearVelocity == Throw and A.Velocity == Throw, "a welded partner reads the velocity")
        local function path(v0, frames)
            local y, v = 0, v0
            for frame = 1, frames do
                local steps = frame == 1 and 4 or 1
                for _ = 1, steps do
                    v -= workspace.Gravity / (60 * steps)
                    y += v / (60 * steps)
                end
            end
            return y, v
        end
        local From = {}
        for _, P in { Thrown, Legacy, Pushed, Dropped, A } do
            From[P] = P.Position
        end
        for _ = 1, 10 do
            RunService.Heartbeat:Wait()
        end
        local Y, V = path(60, 10)
        for _, P in { Thrown, Legacy, Pushed, A } do
            local Moved = P.Position - From[P]
            assert(math.abs(Moved.X - 5) < 1e-3 and math.abs(Moved.Z + 10 / 3) < 1e-3, `{P.Name} sideways {Moved}`)
            assert(math.abs(Moved.Y - Y) < 2e-3, `{P.Name} rose {Moved.Y}, the client {Y}`)
            assert(math.abs(P.AssemblyLinearVelocity.Y - V) < 1e-3, `{P.Name} speed {P.AssemblyLinearVelocity.Y}, the client {V}`)
        end
        local Drop = path(0, 10)
        assert(math.abs((Dropped.Position - From[Dropped]).Y - Drop) < 2e-3, `dropped {(Dropped.Position - From[Dropped]).Y}, the client {Drop}`)
        "#,
        1.0,
    );
}

#[test]
fn a_spinning_part_turns_about_its_centre_and_slows() {
    // Measured (notes/macro flyprobe.luau, bodyprobe.luau): a 2-stud cube spun at 6 rad/s about Y reads 5.995 after one frame
    // and 5.800 after 39; ApplyAngularImpulse divides by the inertia (a cube's m * 2/3), so m * (0, 4, 0) spins it at 6
    check_running(
        "play",
        r#"
        local RunService = game:GetService("RunService")
        local function cube(x)
            local P = Instance.new("Part")
            P.Size = Vector3.new(2, 2, 2)
            P.Anchored = false
            P.CanCollide = false
            P.CFrame = CFrame.new(x, 500, 0)
            P.Parent = workspace
            return P
        end
        local Spun, Twisted = cube(0), cube(10)
        Spun.AssemblyAngularVelocity = Vector3.new(0, 6, 0)
        Twisted:ApplyAngularImpulse(Vector3.new(0, 4, 0) * Twisted.AssemblyMass)
        RunService.Heartbeat:Wait()
        assert(math.abs(Spun.AssemblyAngularVelocity.Y - 5.995) < 1e-3, `spin {Spun.AssemblyAngularVelocity}`)
        assert(math.abs(Twisted.AssemblyAngularVelocity.Y - 5.995) < 1e-3, `impulse spin {Twisted.AssemblyAngularVelocity}`)
        assert(math.abs(Spun.CFrame.LookVector:Dot(Vector3.new(math.sin(-0.1), 0, -math.cos(0.1)))) > 0.9999, "it turned a frame's worth")
        for _ = 1, 38 do
            RunService.Heartbeat:Wait()
        end
        assert(math.abs(Spun.AssemblyAngularVelocity.Y - 5.8) < 2e-3, `spin after 39 frames {Spun.AssemblyAngularVelocity}`)
        assert(Spun.AssemblyAngularVelocity.X == 0 and Spun.AssemblyAngularVelocity.Z == 0, "a cube does not wobble")
        "#,
        1.0,
    );
}

#[test]
fn a_parts_mass_is_its_volume_times_its_density() {
    // Measured in a playing client (notes/macro bodyprobe.luau)
    check_running(
        "play",
        r#"
        local function part(size, material)
            local P = Instance.new("Part")
            P.Size = size
            P.Material = material or Enum.Material.Plastic
            P.Anchored = false
            P.CanCollide = false
            P.CFrame = CFrame.new(0, 500, 0)
            P.Parent = workspace
            return P
        end
        local function near(a, b) return math.abs(a - b) < 1e-4 end
        assert(near(part(Vector3.new(2, 2, 2)).Mass, 5.6))
        assert(near(part(Vector3.new(2, 2, 2), Enum.Material.Slate):GetMass(), 21.528))
        assert(near(part(Vector3.new(0.1, 0.1, 0.1)).Mass, 0.0007), "no minimum")
        local Ball = part(Vector3.new(2, 2, 2))
        Ball.Shape = Enum.PartType.Ball
        assert(near(Ball.Mass, 2.932153), `ball {Ball.Mass}`)
        local Custom = part(Vector3.new(2, 2, 2))
        Custom.CustomPhysicalProperties = PhysicalProperties.new(3, 0.3, 0.5)
        assert(near(Custom.Mass, 24), `custom {Custom.Mass}`)
        local Base = part(Vector3.new(2, 2, 2))
        local Light = part(Vector3.new(1, 1, 1))
        Light.Massless = true
        Light.CFrame = Base.CFrame * CFrame.new(3, 0, 0)
        local Heavy = part(Vector3.new(1, 1, 1))
        Heavy.CFrame = Base.CFrame * CFrame.new(0, 0, 3)
        for _, Other in { Light, Heavy } do
            local Joint = Instance.new("WeldConstraint")
            Joint.Part0, Joint.Part1 = Base, Other
            Joint.Parent = Base
        end
        assert(Light.Mass == 0 and Light:GetMass() == 0, "a Massless part weighs nothing")
        assert(near(Light.AssemblyMass, 6.3) and near(Base.AssemblyMass, 6.3), `assembly {Base.AssemblyMass}`)
        local Offset = Base.AssemblyCenterOfMass - Base.Position
        assert(near(Offset.Z, 0.7 * 3 / 6.3) and near(Offset.X, 0) and near(Offset.Y, 0), `centre {Offset}`)
        "#,
        0.2,
    );
}

#[test]
fn cframe_and_vector3_properties_keep_their_floats() {
    // A playing client keeps a property's float32s (notes/macro bodyprobe.luau: CFrame.Angles(0.001, 0, 0) on an
    // Attachment reads back sin 0.001000000); lune's file conversion rounded them to 1/65536 (0.001007)
    check_running(
        "play",
        r#"
        local Where = CFrame.new(40.123456, 30.987654, -25.5) * CFrame.Angles(0.3, 1.1, -0.2)
        local P = Instance.new("Part")
        P.Anchored = true
        P.CFrame = Where
        P.Parent = workspace
        assert(P.CFrame == Where, "a part's CFrame")
        local A = Instance.new("Attachment")
        A.CFrame = CFrame.Angles(0.001, 0, 0)
        assert(A.CFrame == CFrame.Angles(0.001, 0, 0), `an attachment's {A.CFrame}`)
        local V = Instance.new("CFrameValue")
        V.Value = Where
        assert(V.Value == Where, "a CFrameValue's")
        local M = Instance.new("Motor6D")
        M.C0 = Where
        assert(M.C0 == Where, "a Motor6D's C0")
        "#,
        0.1,
    );
}

#[test]
fn a_streaming_place_simulates_near_its_foci() {
    check_with(
        Options { mode: "play".into(), streaming_target_radius: Some(400.0), ..Options::default() },
        r#"
        assert(workspace.StreamingEnabled, "the place does not stream")
        local function part(at)
            local P = Instance.new("Part")
            P.Anchored = false
            P.CanCollide = false
            P.CFrame = CFrame.new(at)
            P.Parent = workspace
            return P
        end
        -- no focus: nothing is held
        local Free = part(Vector3.new(5000, 500, 0))
        task.wait(0.2)
        assert(Free.Position.Y < 500, "held without a focus")
        local Focus = Instance.new("Part")
        Focus.Anchored = true
        Focus.CFrame = CFrame.new(0, 0, 0)
        Focus.Parent = workspace
        game:GetService("Players").LocalPlayer.ReplicationFocus = Focus
        local Near = part(Vector3.new(300, 100, 0))
        local Far = part(Vector3.new(2048, 2048, 2048))
        task.wait(0.3)
        assert(Near.Position.Y < 100, "a part within the radius did not fall")
        assert(Far.Position.Y == 2048, "a part beyond the radius fell")
        -- the focus moves onto it: it falls from rest
        Focus.CFrame = CFrame.new(2048, 2048, 2048)
        task.wait(0.5)
        local Drop = 2048 - Far.Position.Y
        assert(Drop > 15 and Drop < 30, `fell {Drop} in 0.5 s from rest`)
        "#,
        1.5,
    );
}

/// An assembly starting about a focus is held to its reach (StreamingTargetRadius cut to a whole 64,
/// 32 less for an added focus) and then stands still keeping its velocity; one clear of every focus
/// falls on (Studio, target radius 400: a 400-stud cube about an added focus fell 5.9 studs in 0.21 s).
#[test]
fn a_streaming_place_holds_an_assembly_to_its_focus_reach() {
    check_with(
        Options { mode: "play".into(), streaming_target_radius: Some(400.0), ..Options::default() },
        r#"
        local function part(size, at)
            local P = Instance.new("Part")
            P.Anchored = false
            P.CanCollide = false
            P.Size = Vector3.one * size
            P.CFrame = CFrame.new(at)
            P.Parent = workspace
            return P
        end
        local function anchor(at)
            local A = Instance.new("Part")
            A.Anchored = true
            A.CFrame = CFrame.new(at)
            A.Parent = workspace
            return A
        end
        local Player = game:GetService("Players").LocalPlayer
        Player.ReplicationFocus = anchor(Vector3.zero)
        Player:AddReplicationFocus(anchor(Vector3.new(2048, 2048, 2048)))
        local Cage = part(400, Vector3.new(2048, 2048, 2048))
        local Cube = part(300, Vector3.zero)
        local Small = part(4, Vector3.new(300, 0, 0))
        local Beyond = part(4, Vector3.new(390, 0, 0))
        task.wait(1.6)
        local CageDrop = 2048 - Cage.Position.Y
        assert(CageDrop > 5.5 and CageDrop < 6.8, `the cage fell {CageDrop}`)
        assert(Cage.Velocity.Y < -40, "the cage lost its velocity")
        local CubeDrop = -Cube.Position.Y
        assert(CubeDrop > 123 and CubeDrop < 130, `the cube fell {CubeDrop}`)
        assert(-Small.Position.Y > 200, `the small part stopped at {-Small.Position.Y}`)
        assert(Beyond.Position.Y == 0, "a part past the character's reach fell")
        "#,
        2.0,
    );
}

/// A model pivoted every frame, as an attack effect's spin pivots one (`PivotTo(originalPivot * CFrame.fromOrientation(0, yaw, 0))`),
/// keeps its rotation: the pivot is the target after each PivotTo, as a client keeps it. ree stored `Delta * Stored`, whose float
/// error the transposed inverse squared each call, and such an effect lost its rotation in about 25 frames: its start part read
/// [0 0 0; 0 1 0; 0 0 0] and the bezier parts it emits went NaN. Measured in a playing client (roblox-sim, Studio 0.746,
/// 2026-11-07): the pivot equals the target after all 120 steps, the parts stay orthonormal within 1.2e-7.
#[test]
fn a_model_pivoted_every_frame_keeps_its_rotation() {
    check(
        r#"
        local Model = Instance.new("Model")
        -- the effect's start part as its bundle stores it (rotation id 0: a quarter turn about Y in floats)
        local Start = Instance.new("Part")
        Start.Anchored = true
        Start.CFrame = CFrame.new(-9.346, 0.5, -121.796, -4.37113883e-08, 0, 1, 0, 1, 0, -1, 0, -4.37113883e-08)
        Start.Parent = Model
        local Other = Instance.new("Part")
        Other.Anchored = true
        Other.CFrame = CFrame.new(-9.346, 0.5, -121.796)
        Other.Parent = Model
        Model.WorldPivot = CFrame.new(-9.346, 0.5, -121.796)
        Model.Parent = workspace
        local Origin = Model:GetPivot()
        for Step = 1, 120 do
            local Target = Origin * CFrame.fromOrientation(0, math.rad(720) * Step / 60, 0)
            Model:PivotTo(Target)
            assert(Model:GetPivot() == Target, `step {Step}: the pivot is not the target`)
            local Right = Start.CFrame.RightVector.Magnitude
            assert(math.abs(Right - 1) < 1e-6, `step {Step}: Start's rotation scaled to {Right}`)
        end
        assert(math.abs(Start.Position.Y - 0.5) < 1e-3, `Start moved to {Start.Position}`)
        -- a part pivoted every frame: its pivot is the target, its rotation stays one
        local Part = Instance.new("Part")
        Part.PivotOffset = CFrame.new(0, 1, 0)
        Part.Parent = workspace
        for Step = 1, 120 do
            local Target = CFrame.new(1, 2, 3) * CFrame.fromOrientation(0, math.rad(720) * Step / 60, 0)
            Part:PivotTo(Target)
            assert((Part:GetPivot().Position - Target.Position).Magnitude < 1e-4, `step {Step}: {Part:GetPivot()}`)
            assert(math.abs(Part.CFrame.RightVector.Magnitude - 1) < 1e-6, `step {Step}: the part's rotation scaled`)
        end
        "#,
    );
}

/// A jointed rig moved by BulkMoveTo, every part given its CFrame times one rigid move (a scene moved from one spot to
/// another), keeps its root's rotation and its shape. Each part's write moves the root as that part moved, `Target *
/// Before:Inverse()`; the inverse is the transpose, so a root scaled by s came back scaled by s squared and the error
/// doubled with each part written. A 15-part rig's root read |RightVector| 0.978 after one such move and a rotation of
/// determinant 0 after the second, and the parts welded to it collapsed with it; a move and a move back left a rig NaN.
/// The root is stored as a client stores a part's CFrame (`frame::stored_rotation`), so it stays orthonormal.
#[test]
fn a_jointed_rig_moved_part_by_part_keeps_its_roots_rotation() {
    check(
        r#"
        local Rig = Instance.new("Model")
        local Root = Instance.new("Part")
        Root.Name = "Root"
        Root.Anchored = true
        Root.Size = Vector3.new(2, 2, 1)
        Root.CFrame = CFrame.new(-24, 3, -132) * CFrame.fromOrientation(0.3, math.rad(40), -0.2)
        Root.Parent = Rig
        local Parts = { Root }
        for Index = 1, 14 do
            local Part = Instance.new("Part")
            Part.Name = `Part{Index}`
            Part.Size = Vector3.new(1, 1, 1)
            Part.Massless = true
            Part.Parent = Rig
            local Joint = Instance.new(if Index % 2 == 0 then "Motor6D" else "Weld")
            Joint.Part0, Joint.Part1 = Parts[(Index + 1) // 2], Part
            Joint.C0 = CFrame.new(0.5, 1, -0.25 * Index) * CFrame.fromOrientation(0.1 * Index, 0.7, -0.2 * Index)
            Joint.C1 = CFrame.fromOrientation(0, -0.3 * Index, 0.4)
            Joint.Parent = Part
            table.insert(Parts, Part)
        end
        Rig.Parent = workspace
        local Start = Root.CFrame
        local Offsets = {}
        for Index, Part in Parts do
            Offsets[Index] = Root.CFrame:ToObjectSpace(Part.CFrame)
        end
        local function Off(A, B)
            local a, b = { A:GetComponents() }, { B:GetComponents() }
            local Worst = 0
            for Index = 1, 12 do
                Worst = math.max(Worst, math.abs(a[Index] - b[Index]))
            end
            return Worst
        end
        local function Check(What, Want)
            local At = Root.CFrame
            local X, Y, Z = At.XVector, At.YVector, At.ZVector
            local Worst = math.max(math.abs(X:Dot(X) - 1), math.abs(Y:Dot(Y) - 1), math.abs(Z:Dot(Z) - 1), math.abs(X:Dot(Y)), math.abs(X:Dot(Z)), math.abs(Y:Dot(Z)))
            assert(Worst < 1e-5, `{What}: the root's rotation is {Worst} off orthonormal (|RightVector| {X.Magnitude})`)
            assert(Off(At, Want) < 1e-3, `{What}: the root is at {At}, the move puts it at {Want}`)
            for Index, Part in Parts do
                assert(Off(At:ToObjectSpace(Part.CFrame), Offsets[Index]) < 1e-3, `{What}: {Part.Name} left its joints`)
            end
        end
        local function Move(Delta)
            local Targets = {}
            for Index, Part in Parts do
                Targets[Index] = Delta * Part.CFrame
            end
            workspace:BulkMoveTo(Parts, Targets, Enum.BulkMoveMode.FireCFrameChanged)
        end
        local There = CFrame.new(2, 0, -108) * CFrame.fromOrientation(0, math.rad(240), 0) * (CFrame.new(-24, 0, -132) * CFrame.fromOrientation(0, math.rad(40), 0)):Inverse()
        Move(There)
        Check("the first move", There * Start)
        local Again = CFrame.new(30, 0, 5) * CFrame.fromOrientation(0, math.rad(-75), 0)
        Move(Again)
        Check("the second move", Again * There * Start)
        Move((Again * There):Inverse())
        Check("the move back", Start)
        "#,
    );
}

/// What a script writes is stored as a client stores it: Vector3s and CFrames exactly (lune's own conversion rounded each
/// fraction to 1/65536), and a part's CFrame, a joint's C0 and a BulkMoveTo orthonormalized as `CFrame:Orthonormalize()` gives
/// it once it is off by more than 1e-5 (a mirror kept), identity for a rotation with a zero column; a model's WorldPivot keeps
/// what is written. The expected values are what a playing client read back (roblox-sim, Studio 0.746, 2026-11-07).
#[test]
fn a_parts_cframe_is_orthonormalized_as_a_client_stores_it() {
    check(
        r#"
        local Part = Instance.new("Part")
        Part.Anchored = true
        Part.Parent = workspace
        local function rows(cf)
            local c = { cf:GetComponents() }
            return { table.unpack(c, 4, 12) }
        end
        local function near(a, b)
            for i = 1, 9 do
                if math.abs(a[i] - b[i]) > 2e-7 then
                    return false
                end
            end
            return true
        end
        -- written exactly: a Vector3, and a CFrame already orthonormal (to the bit, its position too)
        Part.Size = Vector3.new(0.123456789, 1.987654321, 2.3)
        assert(Part.Size == Vector3.new(0.123456789, 1.987654321, 2.3), `size {Part.Size}`)
        local Exact = CFrame.new(0.123456789, 0.987654321, -3.1) * CFrame.fromOrientation(0.3, 1.1, -0.7)
        Part.CFrame = Exact
        assert(Part.CFrame == Exact, `a written CFrame read back {Part.CFrame}`)
        -- the tolerance: a column stretched by 2e-6 or leaning 5e-6 is kept, by 5e-6 or 2e-5 orthonormalized
        local r = { CFrame.fromOrientation(0.3, 1.1, -0.7):GetComponents() }
        local function kept(m)
            local cf = CFrame.new(1, 2, 3, table.unpack(m))
            Part.CFrame = cf
            return Part.CFrame == cf
        end
        local function stretched(d)
            return { r[4] * (1 + d), r[5], r[6], r[7] * (1 + d), r[8], r[9], r[10] * (1 + d), r[11], r[12] }
        end
        local function leaning(d)
            return { r[4], r[5] + d * r[4], r[6], r[7], r[8] + d * r[7], r[9], r[10], r[11] + d * r[10], r[12] }
        end
        assert(kept(stretched(2e-6)) and not kept(stretched(5e-6)), "a stretched column")
        assert(kept(leaning(5e-6)) and not kept(leaning(2e-5)), "a leaning column")
        local function written(...)
            Part.CFrame = CFrame.new(1, 2, 3, ...)
            assert(Part.CFrame.Position == Vector3.new(1, 2, 3), "the position moved")
            return rows(Part.CFrame)
        end
        local function expect(got, want, what)
            assert(near(got, want), `{what}: {table.concat(got, " ")}`)
        end
        expect(written(0.5, 0, 0, 0, 0.5, 0, 0, 0, 0.5), { 1, 0, 0, 0, 1, 0, 0, 0, 1 }, "scaled")
        expect(written(0.9, 0.3, -0.2, 0.1, 1.2, 0.4, -0.3, 0.2, 0.8), {
            0.9434563517570496, -0.0218996349722147, 0.3307726979255676,
            0.10482849180698395, 0.9663204550743103, -0.23502254486083984,
            -0.31448549032211304, 0.2564079761505127, 0.9139769673347473,
        }, "skewed")
        expect(written(1, 0, 0, 0, 1, 0, 0.5, 0.5, 1), {
            0.8944271802902222, -0.18257418274879456, -0.40824827551841736,
            0, 0.9128710031509399, -0.4082483649253845,
            0.4472135901451111, 0.3651483952999115, 0.8164965510368347,
        }, "sheared")
        expect(written(1, 0, 0, 0, 1, 0, 0, 0, -2), { 1, 0, 0, 0, 1, 0, 0, 0, -1 }, "mirrored")
        expect(written(0, 0, 0, 0, 1, 0, 0, 0, 0), { 1, 0, 0, 0, 1, 0, 0, 0, 1 }, "degenerate")
        expect(written(0, 0, 1, 0, 1, 0, 0, 0, 0), { 1, 0, 0, 0, 1, 0, 0, 0, 1 }, "no right vector")
        -- a Motor6D's C0 and a BulkMoveTo store it as the part does
        local Skewed = CFrame.new(1, 2, 3, 0.9, 0.3, -0.2, 0.1, 1.2, 0.4, -0.3, 0.2, 0.8)
        local Client = {
            0.9434563517570496, -0.0218996349722147, 0.3307726979255676,
            0.10482849180698395, 0.9663204550743103, -0.23502254486083984,
            -0.31448549032211304, 0.2564079761505127, 0.9139769673347473,
        }
        local Motor = Instance.new("Motor6D")
        Motor.C0 = Skewed
        expect(rows(Motor.C0), Client, "a Motor6D's C0")
        local Other = Instance.new("Part")
        Other.Anchored = true
        Other.Parent = workspace
        workspace:BulkMoveTo({ Other }, { Skewed }, Enum.BulkMoveMode.FireCFrameChanged)
        expect(rows(Other.CFrame), Client, "BulkMoveTo")
        -- a model's pivot keeps a scaled rotation
        local Model = Instance.new("Model")
        Model.WorldPivot = CFrame.new(0, 0, 0, 0.5, 0, 0, 0, 0.5, 0, 0, 0, 0.5)
        expect(rows(Model:GetPivot()), { 0.5, 0, 0, 0, 0.5, 0, 0, 0, 0.5 }, "the pivot")
        "#,
    );
}

/// A Capsule part (Enum.PartType.Capsule = 5, new in Studio 0.747's dump) is a box there for now: it keeps its Size, weighs
/// what a Block of that size weighs (6 x 2 x 4 of Plastic: 33.6, where a Cylinder weighs 13.19) and a ray meets its box
/// (hits at the corners 2.9, 1.9 from the centre, misses past the faces). Measured in Studio 0.747.0.7471481.
#[test]
fn a_capsule_part_is_its_box() {
    check(
        r#"
        local function near(a, b) return math.abs(a - b) < 1e-3 end
        local Capsule = Instance.new("Part")
        Capsule.Shape = Enum.PartType.Capsule
        assert(Capsule.Shape == Enum.PartType.Capsule and Capsule.Shape.Value == 5 and Capsule.Size == Vector3.new(4, 1.2, 2))
        Capsule.Size = Vector3.new(6, 2, 4)
        Capsule.Anchored, Capsule.CFrame = true, CFrame.new(1000, 500, 1000)
        Capsule.Parent = workspace
        assert(near(Capsule.Mass, 33.6) and near(Capsule:GetMass(), 33.6), Capsule.Mass)
        local Cylinder = Instance.new("Part")
        Cylinder.Shape, Cylinder.Size = Enum.PartType.Cylinder, Vector3.new(6, 2, 4)
        assert(near(Cylinder.Mass, 13.1947), Cylinder.Mass)
        local Params = RaycastParams.new()
        Params.FilterType = Enum.RaycastFilterType.Include
        Params.FilterDescendantsInstances = { Capsule }
        local function ray(x, z)
            local Hit = workspace:Raycast(Vector3.new(1000 + x, 510, 1000 + z), Vector3.new(0, -20, 0), Params)
            return Hit and Hit.Distance
        end
        assert(near(ray(0, 0), 9) and near(ray(2.9, 1.9), 9) and near(ray(-2.9, -1.9), 9), "the box's top")
        assert(ray(3.1, 0) == nil and ray(0, 2.1) == nil, "past the box")
        local Ok, Problem = pcall(function() Capsule.Shape = 6 end)
        assert(not Ok and Problem == "Unable to assign property Shape. Invalid value 6 for enum PartType", Problem)
        "#,
    );
}

#[test]
fn teleport_service_is_named_with_a_space() {
    // Studio 0.748 names it "Teleport Service", so game["Teleport Service"] finds it and game.TeleportService is no member
    check(
        r#"
        local Teleport = game:GetService("TeleportService")
        assert(Teleport.Name == "Teleport Service" and game["Teleport Service"] == Teleport, Teleport.Name)
        local Ok, Problem = pcall(function()
            return (game :: any).TeleportService
        end)
        assert(not Ok and string.find(tostring(Problem), "TeleportService is not a valid member of DataModel", 1, true), tostring(Problem))
        "#,
    );
}

#[test]
fn player_mouse_is_an_instance_whose_rays_follow_the_camera() {
    // Studio 0.748, a playing client: GetMouse() is a PlayerMouse Instance, no table; UnitRay is the CurrentCamera's
    // ViewportPointToRay at the mouse, Origin CFrame.new(o, o + d), Hit and Target the first part along it
    check(
        r##"
        local function fails(message, f, ...)
            local ok, err = pcall(f, ...)
            local text = string.gsub(tostring(err), "^[^:]*:%d+: ", "")
            assert(not ok and text == message, tostring(err))
        end
        local Player = game:GetService("Players").LocalPlayer
        local M = Player:GetMouse()
        assert(type(M) == "userdata" and typeof(M) == "Instance" and M.ClassName == "PlayerMouse" and M.Name == "Instance")
        assert(M:IsA("Mouse") and Player:GetMouse() == M and M.Parent == nil and M:GetFullName() == "Instance" and tostring(M) == "Instance")
        fails("invalid argument #1 to 'next' (table expected, got Instance)", next, M)
        fails("invalid argument #1 to 'rawget' (table expected, got Instance)", rawget, M, "X")
        fails("attempt to get length of a Instance value", function() return #M end)
        fails("NotAMember is not a valid member of PlayerMouse \"Instance\"", function() return (M :: any).NotAMember end)
        fails("Unable to assign property X. Property is read only", function() (M :: any).X = 5 end)
        fails("Unable to assign property Hit. Property is read only", function() (M :: any).Hit = CFrame.new() end)
        fails("The Parent property of Instance is locked, current parent: NULL, new parent Workspace", function() M.Parent = workspace end)
        fails("Instance cannot be cloned", M.Clone, M)
        M.Archivable = false
        assert(M:Clone() == nil)
        assert(typeof(M.Button1Down) == "RBXScriptSignal" and typeof(M.Changed) == "RBXScriptSignal" and M.Icon == "")
        -- the rays, from the mouse location (-1000, -1000 without a host)
        assert(M.X == -1000 and M.Y == -1000)
        local Camera = workspace.CurrentCamera
        Camera.CFrame = CFrame.lookAt(Vector3.new(0, 500, 0), Vector3.new(0, 500, -100))
        local Ray = Camera:ViewportPointToRay(M.X, M.Y)
        assert(M.UnitRay.Origin == Ray.Origin and M.UnitRay.Direction == Ray.Direction)
        assert(M.Origin == CFrame.new(Ray.Origin, Ray.Origin + Ray.Direction))
        assert(M.Hit.Position == Ray.Origin + Ray.Direction * 10000 and M.Target == nil and M.TargetSurface == Enum.NormalId.Right)
        -- a wall across the view, whatever its CanQuery, is the target, hit on its Back face, once the spatial index has it
        -- (a part that just entered the Workspace is not seen until a spatial query or the physics step takes it in)
        local Wall = Instance.new("Part")
        Wall.Anchored = true
        Wall.CanQuery = false
        Wall.Transparency = 1
        Wall.Size = Vector3.new(1e6, 1e6, 4)
        Wall.CFrame = CFrame.new(0, 500, -60)
        Wall.Parent = workspace
        assert(M.Target == nil and M.TargetSurface == Enum.NormalId.Right and M.Hit.Position == Ray.Origin + Ray.Direction * 10000, tostring(M.Target))
        workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.yAxis)
        assert(M.Target == Wall and M.target == Wall and M.TargetSurface == Enum.NormalId.Back, tostring(M.Target))
        assert(math.abs(M.Hit.Position.Z + 58) < 1e-3 and M.Hit.Rotation == M.Origin.Rotation, tostring(M.Hit))
        M.TargetFilter = Wall
        assert(M.Target == nil and M.TargetSurface == Enum.NormalId.Right)
        -- computed when read: the camera's move shows at once
        Camera.CFrame = CFrame.lookAt(Vector3.new(0, 600, 0), Vector3.new(0, 600, -100))
        assert(M.UnitRay.Origin == Camera:ViewportPointToRay(M.X, M.Y).Origin)
        "##,
    );
}

/// The PlayerMouse sees the parts the spatial index has (Studio 0.747, a playing client: tools/parity/probes/mouse_index.luau):
/// a part that just entered the Workspace is no Target until a spatial query (a Raycast) or the physics step takes it in, and
/// no mouse read takes it in; a part moved or turned counts at once, one removed is gone at once, and one put back waits for
/// the index again, whatever its CanQuery.
#[test]
fn player_mouse_sees_a_new_part_once_the_spatial_index_has_it() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r##"
        local M = game:GetService("Players").LocalPlayer:GetMouse()
        workspace.CurrentCamera.CFrame = CFrame.lookAt(Vector3.new(200, 500, 200), Vector3.new(200, 500, 100))
        local Ray = M.UnitRay
        local function Across(Distance)
            local Made = Instance.new("Part")
            Made.Anchored = true
            Made.Size = Vector3.new(100, 100, 1)
            Made.CFrame = CFrame.lookAt(Ray.Origin + Ray.Direction * Distance, Ray.Origin + Ray.Direction * (Distance + 1))
            Made.Parent = workspace
            return Made
        end
        local Part = Across(50)
        assert(M.Target == nil and M.TargetSurface == Enum.NormalId.Right)
        local _ = M.Hit, M.hit, M.target, M.TargetSurface
        assert(M.Target == nil and M.Hit.Position == Ray.Origin + Ray.Direction * 10000)
        workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.yAxis)
        assert(M.Target == Part and M.TargetSurface == Enum.NormalId.Back, tostring(M.Target))
        Part.CFrame = CFrame.lookAt(Ray.Origin + Ray.Direction * 40, Ray.Origin + Ray.Direction * 39)
        assert(M.Target == Part and M.TargetSurface == Enum.NormalId.Front)
        Part.Parent = nil
        assert(M.Target == nil)
        Part.Parent = workspace
        assert(M.Target == nil)
        Part.CanQuery = false
        assert(M.Target == nil)
        shared.MouseIndexPart = Part
        "##,
    );
    // the physics step takes it in
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, r##"assert(game:GetService("Players").LocalPlayer:GetMouse().Target == shared.MouseIndexPart)"##);
}

#[test]
fn the_hosts_mouse_record_moves_and_fires_the_player_mouse() {
    // roblox-ui-renderer's input layer writes X / Y / ViewSizeX / ViewSizeY into shared.__rurMouse and fires the signals it
    // holds: they reach the PlayerMouse
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r##"
        local M = game:GetService("Players").LocalPlayer:GetMouse()
        M.Button1Down:Connect(function() M:SetAttribute("Clicks", (M:GetAttribute("Clicks") or 0) + 1) end)
        M.KeyDown:Connect(function(Key) M:SetAttribute("Key", Key) end)
        "##,
    );
    let record: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared").raw_get("__rurMouse").expect("the host's record");
    let signal = |name: &str| match record.raw_get::<LuaValue>(name).expect(name) {
        LuaValue::UserData(ud) => ud.borrow::<crate::lua::types::LSignal>().expect("a signal").0,
        other => panic!("{name}: {other:?}"),
    };
    crate::instance::fire(&mut engine.state.borrow_mut(), signal("Button1Down"), Vec::new());
    crate::instance::fire(&mut engine.state.borrow_mut(), signal("KeyDown"), vec![crate::Value::String(crate::Str::from("a"))]);
    record.raw_set("X", 120.0).unwrap();
    record.raw_set("Y", 80.0).unwrap();
    record.raw_set("ViewSizeX", 1920.0).unwrap();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local M = game:GetService("Players").LocalPlayer:GetMouse()
        assert(M:GetAttribute("Clicks") == 1 and M:GetAttribute("Key") == "a", tostring(M:GetAttribute("Clicks")))
        assert(M.X == 120 and M.Y == 80 and M.ViewSizeX == 1920)
        local Ray = workspace.CurrentCamera:ViewportPointToRay(120, 80)
        assert(M.UnitRay.Direction == Ray.Direction)
        "##,
    );
}

#[test]
fn the_player_mouse_is_made_on_the_first_get_mouse() {
    // one made at start would take an instance serial before every other instance's (GetDebugId, a host's node ids)
    check(
        r##"
        local Player = game:GetService("Players").LocalPlayer
        local function Serial(Object) return tonumber((string.gsub(Object:GetDebugId(), "%D", ""))) end
        local Before = Serial(Instance.new("Part"))
        local Mouse = Player:GetMouse()
        local After = Serial(Instance.new("Part"))
        assert(Serial(Mouse) == Before + 1 and After == Before + 2, `{Before} {Serial(Mouse)} {After}`)
        assert(Player:GetMouse() == Mouse and Serial(Instance.new("Part")) == After + 1)
        "##,
    );
}

/// The PlayerMouse's ray reads cost the work clock what a playing client spent on them beyond a plain read (roblox-sim, Studio
/// 0.747, cost.rs MOUSE_RAY): a miss 0.44 us for Target and 0.64 for Hit, whatever lies off the ray; a hit 0.68 and 0.90; 24 parts
/// under TargetFilter across the ray 3.13 and 3.76 (each scene's parts taken into the spatial index first: the mouse sees no others). A
/// read takes nothing into the spatial index: GetLargestCutoffDistance misses a part that just entered the Workspace after one, and
/// sees it after a Raycast (tools/parity/probes/mouse_index.luau).
#[test]
fn player_mouse_rays_cost_what_a_client_spent() {
    let (lua, engine) = start();
    crate::lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local M = game:GetService("Players").LocalPlayer:GetMouse()
        workspace.CurrentCamera.CFrame = CFrame.lookAt(Vector3.new(0, 500, 0), Vector3.new(0, 500, -100))
        local Ray = M.UnitRay
        local function Beyond(Read)
            local N, V = 20, nil
            local Start = os.clock()
            Read(N)
            local Middle = os.clock()
            for _ = 1, N do V = M.X end
            return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
        end
        local function Hit(N) local V for _ = 1, N do V = M.Hit end return V end
        local function Target(N) local V for _ = 1, N do V = M.Target end return V end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        local function Part(Size, At, Parent)
            local P = Instance.new("Part")
            P.Anchored = true
            P.Size = Size
            P.CFrame = At
            P.Parent = Parent or workspace
            return P
        end
        local function Across(Distance, Parent)
            return Part(Vector3.new(40, 40, 0.5), CFrame.lookAt(Ray.Origin + Ray.Direction * Distance, Ray.Origin + Ray.Direction * (Distance + 1)), Parent)
        end
        -- (a spatial query takes what entered the Workspace into the index)
        local function Index()
            workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.yAxis)
        end
        -- 500 parts off the ray
        local Off = Instance.new("Folder")
        Off.Parent = workspace
        for I = 1, 500 do
            Part(Vector3.one, CFrame.new(500 + I * 3, 5, -500), Off)
        end
        Index()
        assert(M.Target == nil)
        Within("Target, a miss", Beyond(Target), 0.40, 0.52)
        Within("Hit, a miss", Beyond(Hit), 0.58, 0.72)
        local Wall = Across(50)
        Index()
        assert(M.Target == Wall)
        Within("Target, a hit", Beyond(Target), 0.64, 0.80)
        Within("Hit, a hit", Beyond(Hit), 0.84, 1.0)
        Wall:Destroy()
        local Filtered = Instance.new("Folder")
        Filtered.Parent = workspace
        for I = 1, 24 do
            Across(50 + I * 2, Filtered)
        end
        Index()
        M.TargetFilter = Filtered
        assert(M.Target == nil)
        Within("Target, 24 filtered", Beyond(Target), 2.9, 3.5)
        Within("Hit, 24 filtered", Beyond(Hit), 3.3, 4.0)
        M.TargetFilter = nil
        Filtered:Destroy()
        Off:Destroy()

        local O = Vector3.new(200, 100, 200)
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CFrame = CFrame.lookAt(O + Vector3.new(0, 5, 30), O + Vector3.new(0, 5, 0))
        Cam.Focus = CFrame.new(O + Vector3.new(0, 5, 0))
        Part(Vector3.new(60, 60, 1), CFrame.new(O + Vector3.new(0, 5, 12)))
        local _ = M.Hit, M.Target, M.TargetSurface
        assert(Cam:GetLargestCutoffDistance({}) == 0)
        workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.yAxis)
        assert(math.abs(Cam:GetLargestCutoffDistance({}) - 18) < 1e-3)
        "##,
    );
}

/// Reading the PlayerMouse's Icon, UnitRay and Origin costs the work clock what a playing client spent on them beyond a plain read of
/// the mouse (X) (roblox-sim, Studio 0.747, a headless client; cost.rs MOUSE_ICON, MOUSE_UNIT_RAY, MOUSE_ORIGIN): Icon 4.49 us, with an
/// Icon set or not, UnitRay 0.06 and Origin 0.09.
#[test]
fn player_mouse_icon_unit_ray_and_origin_cost_what_a_client_spent() {
    let (lua, engine) = start();
    crate::lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local M = game:GetService("Players").LocalPlayer:GetMouse()
        -- (the frame's first read of a member costs more)
        local _ = M.X, M.Icon, M.UnitRay, M.Origin
        local function Beyond(Read)
            local N, V = 100, nil
            local Start = os.clock()
            Read(N)
            local Middle = os.clock()
            for _ = 1, N do V = M.X end
            return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
        end
        local function Icon(N) local V for _ = 1, N do V = M.Icon end return V end
        local function UnitRay(N) local V for _ = 1, N do V = M.UnitRay end return V end
        local function Origin(N) local V for _ = 1, N do V = M.Origin end return V end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        assert(M.Icon == "")
        Within("Icon", Beyond(Icon), 4.47, 4.51)
        Within("UnitRay", Beyond(UnitRay), 0.055, 0.065)
        Within("Origin", Beyond(Origin), 0.085, 0.095)
        M.Icon = "rbxasset://textures/ArrowFarCursor.png"
        assert(M.Icon == "rbxasset://textures/ArrowFarCursor.png")
        Within("Icon, set", Beyond(Icon), 4.47, 4.51)
        "##,
    );
}

/// The Camera's simpler methods as a playing client answers them (Studio 0.748, new cameras and the CurrentCamera; the values to
/// the bit, the errors as raised): SetRoll's roll, read back and in GetRenderCFrame, held in Scriptable only, kept through a CFrame
/// write and dropped when CameraType leaves Scriptable; GetPanSpeed / GetTiltSpeed; SetCameraPanMode's argument; Zoom and
/// SetImageServerView, which a game script cannot call.
#[test]
fn camera_roll_render_cframe_and_the_legacy_methods() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r##"
        local function C(Frame)
            local T = { Frame:GetComponents() }
            for I, X in T do
                T[I] = string.format("%.9g", X)
            end
            return table.concat(T, " ")
        end
        local function Fails(Message, Function, ...)
            local Ok, E = pcall(Function, ...)
            assert(not Ok and tostring(E) == Message, tostring(E))
        end
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        assert(Cam:GetRoll() == 0 and Cam:GetPanSpeed() == 0 and Cam:GetTiltSpeed() == 0)
        assert(select("#", Cam:GetRoll()) == 1 and select("#", (Cam :: any):GetRoll(1, "x", {})) == 1)
        -- the default camera's -0 terms read 0 in the render frame
        assert(C(Cam:GetRenderCFrame()) == "0 20 20 1 0 0 0 0.707106829 0.707106829 0 -0.707106829 0.707106829", C(Cam:GetRenderCFrame()))
        -- outside Scriptable SetRoll changes nothing (it warns)
        assert(select("#", Cam:SetRoll(0.5)) == 0 and Cam:GetRoll() == 0)
        Cam.CameraType = Enum.CameraType.Scriptable
        Cam.CFrame = CFrame.new(1, 2, 3) * CFrame.Angles(0.1, 0.2, 0.3)
        assert(select("#", Cam:SetRoll(0.5)) == 0 and Cam:GetRoll() == 0.5)
        assert(C(Cam:GetRenderCFrame()) == "1 2 3 0.960530519 0.194709212 0.198669329 -0.178238362 0.979110718 -0.0978434011 -0.213570267 0.058571063 0.975170374", C(Cam:GetRenderCFrame()))
        assert(C(Cam.CFrame) == "1 2 3 0.936293423 -0.289629489 0.198669329 0.312991828 0.944702566 -0.0978434011 -0.159345075 0.153791994 0.975170374", C(Cam.CFrame))
        Cam:SetRoll(10)
        assert(C(Cam:GetRenderCFrame()) == "1 2 3 -0.943181694 -0.266343564 0.198669329 0.251315594 -0.96294713 -0.0978434011 0.217368007 -0.0423553884 0.975170374", C(Cam:GetRenderCFrame()))
        -- the roll is a float32; a numeric string or a boolean is taken
        Cam:SetRoll(0.1)
        assert(Cam:GetRoll() == 0.10000000149011612)
        Cam:SetRoll(-7)
        assert(Cam:GetRoll() == -7)
        Cam:SetRoll(1e30)
        assert(Cam:GetRoll() == 1.0000000150474662e+30)
        Cam:SetRoll(0 / 0)
        assert(Cam:GetRoll() ~= Cam:GetRoll())
        Cam:SetRoll("0.25" :: any)
        assert(Cam:GetRoll() == 0.25)
        Fails("Argument 1 missing or nil", Cam.SetRoll, Cam)
        Fails("Argument 1 missing or nil", Cam.SetRoll, Cam, nil)
        Fails("Unable to cast string to float", Cam.SetRoll, Cam, "x")
        Fails("Unable to cast Array to float", Cam.SetRoll, Cam, {})
        Cam:SetRoll(true :: any)
        assert(Cam:GetRoll() == 1)
        -- kept through a CFrame write, dropped when CameraType leaves Scriptable
        Cam:SetRoll(0.75)
        Cam.CFrame = CFrame.new(5, 5, 5)
        assert(Cam:GetRoll() == 0.75)
        assert(C(Cam:GetRenderCFrame()) == "5 5 5 0.731688857 0.681638777 0 -0.681638777 0.731688857 0 0 0 1", C(Cam:GetRenderCFrame()))
        Cam.CameraType = Enum.CameraType.Fixed
        assert(Cam:GetRoll() == 0)
        Cam.CameraType = Enum.CameraType.Scriptable
        assert(Cam:GetRoll() == 0)
        -- SetCameraPanMode checks a CameraPanMode, nil taking the default
        assert(select("#", Cam:SetCameraPanMode()) == 0 and select("#", Cam:SetCameraPanMode(Enum.CameraPanMode.EdgeBump)) == 0)
        Cam:SetCameraPanMode(1 :: any)
        Cam:SetCameraPanMode("Classic" :: any)
        Fails("Unable to cast double to CameraPanMode", Cam.SetCameraPanMode, Cam, 9)
        Fails("Unable to cast string to CameraPanMode", Cam.SetCameraPanMode, Cam, "Nope")
        Fails("Unable to cast CameraType to CameraPanMode", Cam.SetCameraPanMode, Cam, Enum.CameraType.Fixed)
        Fails("Unable to cast Array to CameraPanMode", Cam.SetCameraPanMode, Cam, {})
        -- RobloxScript members: a call raises, also without the camera or on another instance, while reading them gives a function
        Fails("The current thread cannot call 'Zoom' (lacking capability RobloxScript)", (Cam :: any).Zoom, Cam, 5)
        Fails("The current thread cannot call 'Zoom' (lacking capability RobloxScript)", (Cam :: any).Zoom)
        Fails("The current thread cannot call 'SetImageServerView' (lacking capability RobloxScript)", (Cam :: any).SetImageServerView, Cam, CFrame.new())
        Fails("The current thread cannot call 'SetImageServerView' (lacking capability RobloxScript)", (Cam :: any).SetImageServerView, workspace, {})
        assert(typeof((Cam :: any).Zoom) == "function" and (Cam :: any).Zoom == (workspace.CurrentCamera :: any).Zoom)
        Cam:Destroy()
        assert(Cam:GetRoll() == 0 and C(Cam:GetRenderCFrame()) == "5 5 5 1 0 0 0 1 0 0 0 1", C(Cam:GetRenderCFrame()))
        -- a camera outside the Workspace rolls too
        local Loose = Instance.new("Camera")
        Loose.CameraType = Enum.CameraType.Scriptable
        Loose:SetRoll(0.2)
        assert(Loose:GetRoll() == 0.20000000298023224)
        assert(C(Loose:GetRenderCFrame()) == "0 20 20 0.980066597 0.198669329 0 -0.140480444 0.693011761 0.707106829 0.140480444 -0.693011761 0.707106829", C(Loose:GetRenderCFrame()))
        -- the CurrentCamera, not Scriptable: no roll, the render frame its CFrame
        local Current = workspace.CurrentCamera
        assert(Current.CameraType ~= Enum.CameraType.Scriptable)
        Current:SetRoll(0.3)
        assert(Current:GetRoll() == 0 and Current:GetRenderCFrame() == Current.CFrame and Current:GetPanSpeed() == 0)
        "##,
    );
    let state = engine.state.borrow();
    let warned = state.log_history.iter().filter(|entry| entry.text == "SetRoll can only be used on Camera objects with a CameraType of Scriptable").count();
    assert_eq!(warned, 2, "SetRoll's warnings, on the Fixed camera and the CurrentCamera");
    assert!(!state.log_history.iter().any(|entry| entry.text.contains("is not emulated")));
}

/// A camera's FieldOfView stays within 1 to 120 degrees and ignores a NaN written, on a new camera and the CurrentCamera alike, also
/// at the end of a tween (measured in a playing client, Studio 0.748); a write the limit leaves as it was fires no Changed.
#[test]
fn camera_field_of_view_stays_within_its_limits() {
    check_running(
        "play",
        r#"
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        local function Set(Camera, Value)
            Camera.FieldOfView = 70
            Camera.FieldOfView = Value
            return Camera.FieldOfView
        end
        for _, Camera in { Cam, workspace.CurrentCamera } do
            for _, Case in { { 0.5, 1 }, { 1, 1 }, { 120, 120 }, { 120.5, 120 }, { math.huge, 120 }, { -math.huge, 1 }, { 1e-40, 1 }, { -0, 1 }, { 0 / 0, 70 }, { 200, 120 }, { -5, 1 }, { 33.3, 33.299999237060547 }, { "60", 60 }, { "x", 1 } } do
                local Got = Set(Camera, Case[1] :: any)
                assert(Got == Case[2] and 1 / Got > 0, `{Camera.Name} {Case[1]}: {Got}`)
            end
        end
        Cam.FieldOfView = 120
        local Fired = 0
        Cam:GetPropertyChangedSignal("FieldOfView"):Connect(function()
            Fired += 1
        end)
        Cam.FieldOfView = 200
        task.wait()
        assert(Fired == 0, Fired)
        Cam.FieldOfView = 60
        task.wait()
        assert(Fired == 1, Fired)
        Cam.FieldOfView = 70
        game:GetService("TweenService"):Create(Cam, TweenInfo.new(0.1), { FieldOfView = 200 }):Play()
        task.wait(0.3)
        assert(Cam.FieldOfView == 120, Cam.FieldOfView)
        "#,
        1.0,
    );
}

/// A playing client's engine (one Heartbeat a frame, a playing client's render step).
fn start_play() -> (Lua, Rc<Engine>) {
    let engine = Engine::new(Options { mode: "play".into(), ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    (lua, engine)
}

/// DiagonalFieldOfView and MaxAxisFieldOfView as a playing client reads and writes them (Studio 0.748, to the bit:
/// tools/parity/probes/camera_fov.luau on a 1 x 1 camera, camera_fov_wide.luau on the 1920 x 1080 start camera): views of
/// FieldOfView through the viewport, kept in radians, written through the inverse within 1 and 120 degrees, a NaN ignored
/// and a value that is no number 0; their Changed after FieldOfView's, the member written first.
#[test]
fn camera_diagonal_and_max_axis_fields_of_view() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r##"
        local function N(...)
            local T = table.pack(...)
            for I = 1, T.n do
                T[I] = if T[I] ~= T[I] then "nan" else string.format("%.17g", T[I])
            end
            return table.concat(T, " ")
        end
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        local function All()
            return N(Cam.FieldOfView, Cam.DiagonalFieldOfView, Cam.MaxAxisFieldOfView)
        end
        assert(All() == "70 88.876533508300781 70", All())
        Cam.FieldOfView = 33.3
        assert(N(Cam.DiagonalFieldOfView, Cam.MaxAxisFieldOfView) == "20.283367156982422 33.299999237060547")
        -- (a client reads FieldOfView 119 back as 119.00000762939453 too; ree keeps the degrees written)
        Cam.FieldOfView = 119
        assert(N(Cam.DiagonalFieldOfView, Cam.MaxAxisFieldOfView) == "160.31578063964844 119.00000762939453")
        local function Write(Member, Value)
            Cam.FieldOfView = 70;
            (Cam :: any)[Member] = Value
            return All()
        end
        assert(Write("DiagonalFieldOfView", 30) == "40.207820892333984 30.000003814697266 40.207820892333984")
        assert(Write("DiagonalFieldOfView", 170) == "120 161.07534790039062 120")
        assert(Write("DiagonalFieldOfView", 180) == "nan nan nan")
        assert(Write("DiagonalFieldOfView", 0) == "1 0.017454177141189575 1")
        assert(Write("DiagonalFieldOfView", 0 / 0) == "70 88.876533508300781 70")
        assert(Write("DiagonalFieldOfView", math.huge) == "nan nan nan")
        assert(Write("DiagonalFieldOfView", "50") == "51.548084259033203 50 51.548084259033203")
        assert(Write("DiagonalFieldOfView", true) == "1 0.017454177141189575 1")
        assert(Write("DiagonalFieldOfView", {}) == "1 0.017454177141189575 1")
        assert(Write("MaxAxisFieldOfView", 30) == "30 16.342891693115234 30")
        assert(Write("MaxAxisFieldOfView", 170) == "120 161.07534790039062 120")
        assert(Write("MaxAxisFieldOfView", 180) == "1 0.017454177141189575 1")
        -- a member written its own value reads it back
        Cam.FieldOfView = 60
        Cam.DiagonalFieldOfView = Cam.DiagonalFieldOfView
        assert(All() == "60 67.380126953125 60", All())
        -- the start camera's 1920 x 1080 viewport
        local Start = workspace.CurrentCamera
        assert(N(Start.DiagonalFieldOfView, Start.MaxAxisFieldOfView) == "127.76887512207031 102.44786071777344")
        Start.DiagonalFieldOfView = 50
        assert(N(Start.FieldOfView, Start.DiagonalFieldOfView, Start.MaxAxisFieldOfView) == "37.019374847412109 50.000003814697266 61.51971435546875")
        Start.MaxAxisFieldOfView = 50
        assert(N(Start.FieldOfView, Start.DiagonalFieldOfView, Start.MaxAxisFieldOfView) == "29.394956588745117 31.946813583374023 50.000003814697266")
        -- Changed: the member written first, then FieldOfView, then the third; a write that changes nothing fires nothing
        Cam.FieldOfView = 70
        local Log = {}
        Cam.Changed:Connect(function(Name)
            table.insert(Log, Name)
        end)
        shared.CameraFovLog = Log
        Cam.FieldOfView = 50
        shared.CameraFovCamera = Cam
        "##,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Log, Cam = shared.CameraFovLog, shared.CameraFovCamera
        assert(table.concat(Log, ",") == "FieldOfView,DiagonalFieldOfView,MaxAxisFieldOfView", table.concat(Log, ","))
        table.clear(Log)
        Cam.DiagonalFieldOfView = 60
        Cam.MaxAxisFieldOfView = Cam.MaxAxisFieldOfView
        "##,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Log, Cam = shared.CameraFovLog, shared.CameraFovCamera
        assert(table.concat(Log, ",") == "DiagonalFieldOfView,FieldOfView,MaxAxisFieldOfView", table.concat(Log, ","))
        table.clear(Log)
        Cam.MaxAxisFieldOfView = 60
        "##,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, r##"assert(table.concat(shared.CameraFovLog, ",") == "MaxAxisFieldOfView,FieldOfView,DiagonalFieldOfView", table.concat(shared.CameraFovLog, ","))"##);
}

/// PanUnits and TiltUnits as a playing client answers them (Studio 0.748: tools/parity/probes/camera_units.luau; positions and
/// rotations to 3 decimals, a client's angles being matched to about 1e-6): the yaw snapped to 45 degrees and turned, the pitch
/// snapped to 10 and raised within 81, the camera about Focus with no roll; the int argument; TiltUnits' answer; a look
/// straight down reads yaw -180; the CoordinateFrame alias's Changed.
#[test]
fn camera_pan_and_tilt_units() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r##"
        local function R(...)
            local T = table.pack(...)
            for I = 1, T.n do
                local S = string.format("%.3f", T[I])
                T[I] = if S == "-0.000" then "0.000" else S
            end
            return table.concat(T, " ")
        end
        local function C(Frame)
            return R(Frame:GetComponents())
        end
        local function Fails(Message, Function, ...)
            local Ok, E = pcall(Function, ...)
            assert(not Ok and tostring(E) == Message, tostring(E))
        end
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CFrame = CFrame.lookAt(Vector3.new(3, 4, 20), Vector3.new(0, 0, 0))
        Cam.Focus = CFrame.new(0, 0, 0)
        assert(select("#", Cam:PanUnits(1)) == 0)
        assert(C(Cam.CFrame) == "-14.300 4.000 14.300 0.707 0.137 -0.694 0.000 0.981 0.194 0.707 -0.137 0.694", C(Cam.CFrame))
        Cam:PanUnits(1)
        assert(C(Cam.CFrame) == "-20.224 4.000 0.000 0.000 0.194 -0.981 0.000 0.981 0.194 1.000 0.000 0.000", C(Cam.CFrame))
        assert(Cam.Focus == CFrame.new(0, 0, 0))
        Cam.CFrame = CFrame.lookAt(Vector3.new(3, 4, 20), Vector3.new(0, 0, 0))
        local Steps = {}
        for _ = 1, 11 do
            table.insert(Steps, tostring(Cam:TiltUnits(1)))
        end
        assert(table.concat(Steps, " ") == string.rep("true ", 10) .. "true")
        assert(C(Cam.CFrame) == "0.478 -20.362 3.189 0.989 0.147 0.023 0.000 0.156 -0.988 -0.148 0.977 0.155", C(Cam.CFrame))
        -- the argument: an int, rounded half away from zero, the int's least past its range
        local function Turned(Method, A)
            Cam.CFrame = CFrame.lookAt(Vector3.new(0, 0, 20), Vector3.new(0, 0, 0))
            Cam.Focus = CFrame.new(0, 0, 0);
            (Cam :: any)[Method](Cam, A)
            local Look = Cam.CFrame.LookVector
            return R(math.deg(math.atan2(-Look.X, -Look.Z)), math.deg(math.asin(Look.Y)))
        end
        assert(Turned("PanUnits", 2.5) == "-135.000 0.000" and Turned("PanUnits", -0.5) == "45.000 0.000" and Turned("PanUnits", true) == "-45.000 0.000")
        assert(Turned("PanUnits", 2 ^ 31 - 1) == "-169.163 0.000" and Turned("PanUnits", 0 / 0) == "169.163 0.000")
        assert(Turned("TiltUnits", 2.5) == "0.000 30.000" and Turned("TiltUnits", -0.5) == "0.000 -10.000" and Turned("TiltUnits", "1e1") == "0.000 81.000")
        assert(Turned("TiltUnits", 0 / 0) == "0.000 -81.000")
        Fails("Unable to cast string to int", Cam.PanUnits, Cam, "x")
        Fails("Unable to cast Array to int", Cam.TiltUnits, Cam, {})
        Fails("Unable to cast Dictionary to int", Cam.PanUnits, Cam, { a = 1 })
        Fails("Unable to cast CoordinateFrame to int", Cam.PanUnits, Cam, CFrame.new())
        Fails("Unable to cast CameraType to int", Cam.TiltUnits, Cam, Enum.CameraType.Fixed)
        Fails("Unable to cast Instance to int", Cam.PanUnits, Cam, workspace)
        Fails("Argument 1 missing or nil", Cam.TiltUnits, Cam)
        -- TiltUnits: whether the pitch changed; a level camera keeps its roll
        Cam.CFrame = CFrame.lookAt(Vector3.new(0, 0, 20), Vector3.new(0, 0, 0)) * CFrame.Angles(0, 0, 0.5)
        local Rolled = Cam.CFrame
        assert(Cam:TiltUnits(0) == false and Cam.CFrame == Rolled)
        Cam.CFrame = CFrame.lookAt(Vector3.new(0, 0, 20), Vector3.new(0, 0, 20) + Vector3.new(0, math.sin(math.rad(21)), -math.cos(math.rad(21))))
        assert(Cam:TiltUnits(0) == true)
        assert(C(Cam.CFrame) == "0.000 -6.840 18.794 1.000 0.000 0.000 0.000 0.940 -0.342 0.000 0.342 0.940", C(Cam.CFrame))
        -- a look straight down: yaw -180 (the look's signed zeros as a client's sums leave them)
        Cam.CFrame = CFrame.new(2.58636475, 10.5392456, 2.67944336, 0.115252346, 0.99333626, -0, 0, 0, 1, 0.99333626, -0.115252346, 0)
        Cam.Focus = CFrame.new(2.58636475, -9.38588715, 2.67944336)
        Cam:TiltUnits(1)
        assert(C(Cam.CFrame) == "2.586 10.237 -0.781 -1.000 0.000 0.000 0.000 0.174 0.985 0.000 0.985 -0.174", C(Cam.CFrame))
        -- the deprecated CoordinateFrame and focus read CFrame and Focus
        assert(Cam.CoordinateFrame == Cam.CFrame and Cam.focus == Cam.Focus)
        Cam.CoordinateFrame = CFrame.new(1, 2, 3)
        assert(Cam.CFrame == CFrame.new(1, 2, 3))
        local Log = {}
        Cam.Changed:Connect(function(Name)
            table.insert(Log, Name)
        end)
        shared.CameraUnitsLog = Log
        Cam.Focus = CFrame.new(0, 0, 0)
        Cam:PanUnits(1)
        "##,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, r##"assert(table.concat(shared.CameraUnitsLog, ",") == "Focus,CFrame,CoordinateFrame", table.concat(shared.CameraUnitsLog, ","))"##);
}

/// ZoomToExtents as a playing client answers it (Studio 0.748: camera_units.luau, camera_fov_wide.luau): the size checked
/// before the frame; the rotation kept and the camera backed off from the box's centre by its world-aligned bounding sphere
/// over the sine of the view's wider half angle, the radius at least 0.5; Focus the centre, unrotated.
#[test]
fn camera_zoom_to_extents() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r##"
        local function Fails(Message, Function, ...)
            local Ok, E = pcall(Function, ...)
            assert(not Ok and tostring(E) == Message, tostring(E))
        end
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.FieldOfView = 90
        local function Radius(Size)
            Cam.CFrame = CFrame.new()
            Cam:ZoomToExtents(CFrame.new(), Size)
            return string.format("%.3f", Cam.CFrame.Position.Z * math.sin(math.rad(45)))
        end
        assert(Radius(Vector3.zero) == "0.500" and Radius(Vector3.new(0.57, 0.57, 0.57)) == "0.500" and Radius(Vector3.new(0.58, 0.58, 0.58)) == "0.502")
        assert(Radius(Vector3.new(-0.6, -0.6, -0.6)) == "0.520" and Radius(Vector3.new(1.02, 0, 0)) == "0.510")
        -- a rotated box: |R| times the half size, the size's sign kept (to the bit)
        Cam.CFrame = CFrame.new(13.6367798, 31.8280029, 25.982666, -0.838941455, 0, -0.54422164, -0.435151905, 0.600551844, 0.670805693, 0.326833308, 0.799585819, -0.50382781)
        local Box = CFrame.new(-10.1687622, 27.9455566, -29.9414062, 0.0862564668, -0.389287055, -0.917068899, 0.886146069, 0.450656772, -0.107951567, 0.455307484, -0.803345442, 0.383837283)
        assert(select("#", Cam:ZoomToExtents(Box, Vector3.new(-1.05041504, 8.34655762, 5.02349854))) == 0)
        local P = Cam.CFrame.Position
        assert(string.format("%.9g %.9g %.9g", P.X, P.Y, P.Z) == "-14.69244 33.5214272 -34.1293221", tostring(P))
        assert(Cam.CFrame.Rotation == CFrame.new(0, 0, 0, -0.838941455, 0, -0.54422164, -0.435151905, 0.600551844, 0.670805693, 0.326833308, 0.799585819, -0.50382781).Rotation)
        assert(Cam.Focus == CFrame.new(-10.1687622, 27.9455566, -29.9414062))
        -- the start camera's wide viewport: the horizontal half angle
        local Start = workspace.CurrentCamera
        Start.FieldOfView = 70
        Start.CFrame = CFrame.new()
        Start:ZoomToExtents(CFrame.new(), Vector3.new(2, 2, 2))
        local Expected = math.sqrt(3) / math.sin(math.atan(math.tan(math.rad(35)) * 1920 / 1080))
        assert(math.abs(Start.CFrame.Position.Z - Expected) < 1e-4, tostring(Start.CFrame.Position))
        Fails("Argument 2 missing or nil", Cam.ZoomToExtents, Cam)
        Fails("Unable to cast string to Vector3", Cam.ZoomToExtents, Cam, "x", "y")
        Fails("Unable to cast Vector3 to CoordinateFrame", Cam.ZoomToExtents, Cam, Vector3.one, Vector3.one)
        Fails("Unable to cast CoordinateFrame to Vector3", Cam.ZoomToExtents, Cam, CFrame.new(), CFrame.new())
        Fails("Argument 1 missing or nil", Cam.ZoomToExtents, Cam, nil, Vector3.one)
        Fails("Unable to cast Array to Vector3", Cam.ZoomToExtents, Cam, CFrame.new(), { 1, 2, 3 })
        "##,
    );
}

/// GetLargestCutoffDistance as a playing client answers it (Studio 0.748: camera_cutoff.luau, camera_fov_wide.luau): five
/// parallel rays from Focus to the near plane; parts that can collide (CanQuery aside) and are at most 0.95 transparent; the
/// ignore list; a part entering the Workspace unseen until the physics step or another spatial query; the argument's errors.
#[test]
fn camera_largest_cutoff_distance_and_the_spatial_index() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r##"
        local function Fails(Message, Function, ...)
            local Ok, E = pcall(Function, ...)
            assert(not Ok and tostring(E) == Message, tostring(E))
        end
        local function R(Value)
            return string.format("%.3f", Value)
        end
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CFrame = CFrame.lookAt(Vector3.new(0, 5, 30), Vector3.new(0, 5, 0))
        Cam.Focus = CFrame.new(0, 5, 0)
        assert(Cam:GetLargestCutoffDistance({}) == 0)
        local Wall = Instance.new("Part")
        Wall.Anchored = true
        Wall.Size = Vector3.new(60, 60, 1)
        Wall.CFrame = CFrame.new(0, 5, 10)
        Wall.Parent = workspace
        -- not in the spatial index yet, until another query
        assert(Cam:GetLargestCutoffDistance({}) == 0 and Cam:GetLargestCutoffDistance({}) == 0)
        workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.new(0, 1, 0))
        assert(R(Cam:GetLargestCutoffDistance({})) == "20.000")
        assert(Cam:GetLargestCutoffDistance({ Wall }) == 0)
        Wall.Transparency = 0.95
        assert(R(Cam:GetLargestCutoffDistance({})) == "20.000")
        Wall.Transparency = 0.9500001
        assert(Cam:GetLargestCutoffDistance({}) == 0)
        Wall.Transparency = 0
        Wall.CanQuery = false
        assert(R(Cam:GetLargestCutoffDistance({})) == "20.000")
        Wall.CanCollide = false
        assert(Cam:GetLargestCutoffDistance({}) == 0)
        Wall.CanCollide = true
        Wall.CanQuery = true
        -- moved at once; removed at once; back in the Workspace unseen
        Wall.CFrame = CFrame.new(0, 5, 20)
        assert(R(Cam:GetLargestCutoffDistance({})) == "10.000")
        Wall.Parent = nil
        assert(Cam:GetLargestCutoffDistance({}) == 0)
        Wall.Parent = workspace
        assert(Cam:GetLargestCutoffDistance({}) == 0)
        shared.CutoffCamera = Cam
        -- the rays are parallel: a small block on a corner ray near Focus
        local Corner = Instance.new("Camera")
        Corner.Parent = workspace
        Corner.CFrame = CFrame.lookAt(Vector3.new(100, 0, 40), Vector3.new(100, 0, 0))
        Corner.Focus = CFrame.new(100, 0, 0)
        Corner.FieldOfView = 120
        local Block = Instance.new("Part")
        Block.Anchored = true
        Block.Size = Vector3.new(0.1, 0.1, 0.1)
        Block.CFrame = CFrame.new(100.85, 0.85, 1)
        Block.Parent = workspace
        workspace:GetPartBoundsInBox(CFrame.new(1000, 1000, 1000), Vector3.one)
        assert(R(Corner:GetLargestCutoffDistance({})) == "38.550", R(Corner:GetLargestCutoffDistance({})))
        Block.CFrame = CFrame.new(100.6, 0.6, 1)
        assert(Corner:GetLargestCutoffDistance({}) == 0)
        Fails("Argument 1 missing or nil", Cam.GetLargestCutoffDistance, Cam)
        Fails("Expected table argument, got nil", Cam.GetLargestCutoffDistance, Cam, nil)
        Fails("Expected table argument, got number", Cam.GetLargestCutoffDistance, Cam, 5)
        Fails("Expected table argument, got userdata", Cam.GetLargestCutoffDistance, Cam, workspace)
        Fails("Unable to cast value to Object", Cam.GetLargestCutoffDistance, Cam, { workspace, 5 })
        -- (the dictionary's entries are no list; GetPartBoundsInBox above took the wall back in)
        assert(R(Cam:GetLargestCutoffDistance({ a = workspace })) == "10.000")
        Wall.Parent = nil
        Wall.Parent = workspace
        assert(Cam:GetLargestCutoffDistance({}) == 0)
        "##,
    );
    // the physics step takes the wall in
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, r##"assert(string.format("%.3f", shared.CutoffCamera:GetLargestCutoffDistance({})) == "10.000")"##);
}

/// GetLargestCutoffDistance costs the work clock what a playing client spent on it (roblox-sim, Studio 0.747, cost.rs CUTOFF_CALL):
/// 1.12 us for five rays that miss, with 5,000 parts off them as with none, and 1.80 when each meets a wall (it read a flat 0.07). Its
/// rays read the spatial queries' broadphase, so 100 calls walk nothing of the Workspace (each call walked every part of it), and a
/// part written in between is seen at once.
#[test]
fn camera_largest_cutoff_distance_costs_what_a_client_spent() {
    let (lua, engine) = start_play();
    crate::lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local O = Vector3.new(200, 100, 200)
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CFrame = CFrame.lookAt(O + Vector3.new(0, 5, 30), O + Vector3.new(0, 5, 0))
        Cam.Focus = CFrame.new(O + Vector3.new(0, 5, 0))
        local Ignore = {}
        local function Each(N)
            local Start = os.clock()
            for _ = 1, N do
                Cam:GetLargestCutoffDistance(Ignore)
            end
            local Middle = os.clock()
            for _ = 1, N do
            end
            return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
        end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        -- (a spatial query takes what entered the Workspace into the index)
        local function Index()
            workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.yAxis)
        end
        Within("a miss", Each(20), 1.05, 1.20)
        local Off = Instance.new("Folder")
        for I = 1, 5000 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Size = Vector3.one
            Part.CFrame = CFrame.new(-3000 + (I % 100) * 2, 500 + (I // 100) * 2, -3000)
            Part.Parent = Off
        end
        Off.Parent = workspace
        Index()
        Within("a miss beside 5,000 parts", Each(20), 1.05, 1.20)
        local Wall = Instance.new("Part")
        Wall.Anchored = true
        Wall.Size = Vector3.new(60, 60, 1)
        Wall.CFrame = CFrame.new(O + Vector3.new(0, 5, 10))
        Wall.Parent = workspace
        Index()
        Within("each ray meeting a wall", Each(20), 1.72, 1.90)
        assert(string.format("%.3f", Cam:GetLargestCutoffDistance(Ignore)) == "20.000")
        shared.Cam, shared.Wall = Cam, Wall
        "##,
    );
    super::broadphase::WORKSPACE_WALKS.with(|walks| walks.set(0));
    run(&lua, &engine, r##"for _ = 1, 100 do assert(shared.Cam:GetLargestCutoffDistance({}) > 19) end"##);
    assert_eq!(super::broadphase::WORKSPACE_WALKS.with(|walks| walks.get()), 0, "100 calls walk nothing (the setup's queries made the index)");
    run(&lua, &engine, r##"shared.Wall.CFrame = shared.Wall.CFrame + Vector3.new(0, 0, 10) assert(string.format("%.3f", shared.Cam:GetLargestCutoffDistance({})) == "10.000")"##);
}

/// The parts waiting for the spatial index (camera.rs `Cameras::unindexed`, which a PlayerMouse ray and GetLargestCutoffDistance
/// look each part they meet up in) are each held once and only while in the Workspace: a model of 200 parts moved out of the
/// Workspace and back 50 times in one frame leaves 200 waiting (it held 10,200, every entry again), a model that leaves takes its
/// parts out (it kept them), and what waits is still unseen until a spatial query takes it in.
#[test]
fn parts_waiting_for_the_spatial_index_are_held_once_while_in_the_workspace() {
    let (lua, engine) = start_play();
    let waiting = || engine.state.borrow().cameras.unindexed.len();
    run(
        &lua,
        &engine,
        r##"
        local Store = game:GetService("ReplicatedStorage")
        local Model = Instance.new("Model")
        for I = 1, 200 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Size = Vector3.new(60, 60, 1)
            Part.CFrame = CFrame.new(0, 5, 10)
            Part.Parent = Model
        end
        Instance.new("Folder").Parent = Model
        Model.Parent = workspace
        for _ = 1, 50 do
            Model.Parent = Store
            Model.Parent = workspace
        end
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CFrame = CFrame.lookAt(Vector3.new(0, 5, 30), Vector3.new(0, 5, 0))
        Cam.Focus = CFrame.new(0, 5, 0)
        assert(Cam:GetLargestCutoffDistance({}) == 0)
        shared.Model, shared.Cam = Model, Cam
        "##,
    );
    assert_eq!(waiting(), 200, "each waiting part once");
    // leaving by an ancestor, then by itself
    run(&lua, &engine, r##"local Folder = Instance.new("Folder") Folder.Parent = workspace shared.Model.Parent = Folder Folder.Parent = nil"##);
    assert_eq!(waiting(), 0, "a model that leaves the Workspace takes its parts out");
    run(&lua, &engine, r##"shared.Model.Parent = workspace local Part = shared.Model:FindFirstChildWhichIsA("Part") Part.Parent = nil Part:Destroy()"##);
    assert_eq!(waiting(), 199, "a part that leaves alone");
    run(
        &lua,
        &engine,
        r##"
        assert(shared.Cam:GetLargestCutoffDistance({}) == 0)
        workspace:Raycast(Vector3.new(1000, 1000, 1000), Vector3.new(0, 1, 0))
        assert(string.format("%.3f", shared.Cam:GetLargestCutoffDistance({})) == "20.000")
        "##,
    );
    assert_eq!(waiting(), 0, "a spatial query takes them in");
}

/// Interpolate (Studio 0.748: camera_units.luau's checks, camera_interpolate.luau's endings): the checks' order; a Focus write
/// ends a tween at the next render step and InterpolationFinished fires after the PreRender handlers' deferred work; CameraType
/// leaving Scriptable cancels it; a replaced tween never fires. The movement is as reported, not measured: the CurrentCamera
/// alone, from the start of the next render step, linear in the position, the focus point and the up vector, Focus 20 studs
/// along the look, InterpolationFinished once at the end.
#[test]
fn camera_interpolate() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r##"
        local function Fails(Message, Function, ...)
            local Ok, E = pcall(Function, ...)
            assert(not Ok and tostring(E) == Message, tostring(E))
        end
        local Cam = workspace.CurrentCamera
        local A, B = CFrame.new(1, 2, 3), CFrame.new(4, 5, 6)
        Fails("Argument 3 missing or nil", Cam.Interpolate, Cam)
        Fails("Argument 2 missing or nil", Cam.Interpolate, Cam, A, nil, 1)
        Fails("Unable to cast string to float", Cam.Interpolate, Cam, "z", "y", "x")
        Fails("Unable to cast string to CoordinateFrame", Cam.Interpolate, Cam, "z", B, 0)
        Fails("Interpolation time must be positive.", Cam.Interpolate, Cam, A, B, 0)
        Fails("Attempted to use interpolation with a camera mode other than scriptable.", Cam.Interpolate, Cam, A, B, 0 / 0)
        Cam.CameraType = Enum.CameraType.Scriptable
        Fails("Interpolation time must be positive.", Cam.Interpolate, Cam, A, B, -0)
        Cam.CFrame = CFrame.lookAt(Vector3.new(0, 10, 30), Vector3.new(0, 0, 0))
        Cam.Focus = CFrame.new(0, 0, 0)
        local Log = {}
        shared.InterpolateLog = Log
        game:GetService("RunService").PreRender:Connect(function()
            table.insert(Log, "preRender")
            task.defer(table.insert, Log, "deferred")
        end)
        Cam.InterpolationFinished:Connect(function()
            table.insert(Log, "finished")
        end)
        game:GetService("RunService"):BindToRenderStep("probe", 0, function()
            table.insert(Log, "bind " .. string.format("%.3f %.3f %.3f", Cam.CFrame.X, Cam.CFrame.Y, Cam.CFrame.Z))
        end)
        assert(select("#", Cam:Interpolate(CFrame.lookAt(Vector3.new(30, 10, 0), Vector3.new(0, 0, 0)), CFrame.new(0, 0, 0), 4 / 60)) == 0)
        assert(Cam.CFrame.Position == Vector3.new(0, 10, 30))
        "##,
    );
    for _ in 0..5 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    run(
        &lua,
        &engine,
        r##"
        local Log, Cam = shared.InterpolateLog, workspace.CurrentCamera
        local Text = table.concat(Log, ", ")
        -- moved before the bindings, by a quarter a frame; finished once, after the PreRender handlers' deferred work
        assert(Text == "bind 7.500 10.000 22.500, preRender, deferred, bind 15.000 10.000 15.000, preRender, deferred, "
            .. "bind 22.500 10.000 7.500, preRender, deferred, bind 30.000 10.000 0.000, preRender, deferred, finished, "
            .. "bind 30.000 10.000 0.000, preRender, deferred", Text)
        local Look = Cam.CFrame.LookVector
        assert((Cam.Focus.Position - (Cam.CFrame.Position + Look * 20)).Magnitude < 1e-4 and (Look - Vector3.new(-1, -1 / 3, 0).Unit).Magnitude < 1e-5)
        table.clear(Log)
        -- a Focus write ends it at the next render step, Focus as written; a replaced tween never fires
        Cam:Interpolate(CFrame.new(0, 50, 0), CFrame.new(0, 0, 0), 1000)
        Cam:Interpolate(CFrame.new(0, 60, 0), CFrame.new(0, 0, 0), 1000)
        "##,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, r##"table.clear(shared.InterpolateLog) workspace.CurrentCamera.Focus = CFrame.new(3, 3, 3)"##);
    for _ in 0..3 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    run(
        &lua,
        &engine,
        r##"
        local Log, Cam = shared.InterpolateLog, workspace.CurrentCamera
        local Text = table.concat(Log, ", "):gsub("bind [%d%.%- ]+", "bind")
        assert(Text == "bind, preRender, deferred, finished, bind, preRender, deferred, bind, preRender, deferred", Text)
        assert(Cam.Focus == CFrame.new(3, 3, 3))
        table.clear(Log)
        -- CameraType leaving Scriptable cancels it, unfired
        Cam:Interpolate(CFrame.new(0, 50, 0), CFrame.new(0, 0, 0), 1000)
        Cam.CameraType = Enum.CameraType.Fixed
        Cam.CameraType = Enum.CameraType.Scriptable
        Cam.Focus = CFrame.new(5, 5, 5)
        -- another camera's tween does not move, and a Focus write ends it too
        local Other = Instance.new("Camera")
        Other.Parent = workspace
        Other.CameraType = Enum.CameraType.Scriptable
        Other.InterpolationFinished:Connect(function()
            table.insert(Log, "other finished")
        end)
        Other.CFrame = CFrame.new()
        Other:Interpolate(CFrame.new(10, 0, 0), CFrame.new(10, 0, -5), 1 / 60)
        shared.OtherCamera = Other
        "##,
    );
    for _ in 0..3 {
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    }
    run(
        &lua,
        &engine,
        r##"
        local Log = shared.InterpolateLog
        assert(not table.find(Log, "finished") and not table.find(Log, "other finished"), table.concat(Log, ", "))
        assert(shared.OtherCamera.CFrame == CFrame.new())
        shared.OtherCamera.Focus = CFrame.new(1, 1, 1)
        "##,
    );
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(&lua, &engine, r##"assert(table.find(shared.InterpolateLog, "other finished"), table.concat(shared.InterpolateLog, ", "))"##);
}

/// GetPartsObscuringTarget as a playing client answers it (Studio 0.748, a new Scriptable camera and the CurrentCamera): the parts
/// the segment from the camera to each point enters before the point, nearest first and each once, whatever their Transparency,
/// CanCollide or CanQuery; the ignore list with its descendants; a point on a part's near face does not count it, one inside
/// does; entries that are no Vector3 are skipped; the camera's orientation does not matter; the argument errors as raised.
#[test]
fn camera_parts_obscuring_a_target_follow_a_client() {
    let (lua, engine) = start();
    run(
        &lua,
        &engine,
        r##"
        local function Fails(Message, F, ...)
            local Ok, E = pcall(F, ...)
            if Ok or tostring(E) ~= Message then
                error(tostring(E), 2)
            end
        end
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        local function Part(Name, Z, Props)
            local P = Instance.new("Part")
            P.Name = Name
            P.Anchored = true
            P.Size = Vector3.new(4, 4, 1)
            P.CFrame = CFrame.new(0, 5, Z)
            for K, V in Props or {} do
                (P :: any)[K] = V
            end
            P.Parent = Folder
            return P
        end
        local A = Part("A", 20)
        local B = Part("B", 16, { Transparency = 0.5 })
        Part("C", 12, { CanCollide = false })
        Part("D", 10, { CanQuery = false })
        Part("E", 8, { Transparency = 1 })
        Part("H", -10)
        Part("I", 15).CFrame = CFrame.new(12, 5, 15)
        local Model = Instance.new("Model")
        Model.Parent = Folder
        Part("M", 4).Parent = Model
        local Ball = Part("Ball", 22, { Shape = Enum.PartType.Ball, Size = Vector3.new(3, 3, 3) })
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CameraType = Enum.CameraType.Scriptable
        Cam.CFrame = CFrame.lookAt(Vector3.new(0, 5, 30), Vector3.new(0, 5, 0))
        local Target = Vector3.new(0, 5, 0)
        local function Names(Camera, ...)
            local List = Camera:GetPartsObscuringTarget(...)
            local Out = {}
            for I, P in List do
                Out[I] = P.Name
            end
            return table.concat(Out, " ")
        end
        local All = "Ball A B C D E M"
        assert(Names(Cam, { Target }, {}) == All, Names(Cam, { Target }, {}))
        assert(Names(Cam, { Target }, { A }) == "Ball B C D E M")
        assert(Names(Cam, { Target }, { Model }) == "Ball A B C D E")
        assert(Names(Cam, { Target }, { Folder }) == "")
        assert(Names(Cam, { Target }, { A, nil, B }) == "Ball C D E M")
        -- the points in turn, each part once
        assert(Names(Cam, { Target, Vector3.new(0, 5, -20) }, {}) == All .. " H")
        assert(Names(Cam, { Target, Target, "x" :: any, CFrame.new(Target) :: any, vector.create(0, 5, 0) }, {}) == All)
        assert(Names(Cam, { 1, 2 } :: any, {}) == "" and Names(Cam, {}, {}) == "")
        -- a point inside a part counts it, one on its near face does not, nor the camera's own point
        assert(Names(Cam, { Vector3.new(0, 5, 20) }, {}) == "Ball A")
        assert(Names(Cam, { Vector3.new(0, 5, 20.5) }, {}) == "Ball")
        assert(Names(Cam, { Vector3.new(0, 5, 30) }, {}) == "")
        assert(Names(Cam, { Vector3.new(12, 5, 15) }, {}) == "I")
        -- the camera's orientation does not matter; the CurrentCamera answers the same
        Cam.CFrame = CFrame.new(0, 5, 30) * CFrame.Angles(1, 2, 3)
        assert(Names(Cam, { Target }, {}) == All)
        local Current = workspace.CurrentCamera
        Current.CameraType = Enum.CameraType.Scriptable
        Current.CFrame = CFrame.lookAt(Vector3.new(0, 5, 30), Vector3.new(0, 5, 0))
        assert(Names(Current, { Target }, {}) == All)
        Fails("Argument 1 missing or nil", Cam.GetPartsObscuringTarget, Cam, nil, {})
        Fails("Unable to cast string to Array", Cam.GetPartsObscuringTarget, Cam, "x", {})
        Fails("Unable to cast Dictionary to Array", Cam.GetPartsObscuringTarget, Cam, { a = Target }, {})
        Fails("Expected table argument, got nil", Cam.GetPartsObscuringTarget, Cam, { Target }, nil)
        Fails("Expected table argument, got string", Cam.GetPartsObscuringTarget, Cam, { Target }, "x")
        Fails("Unable to cast value to Object", Cam.GetPartsObscuringTarget, Cam, { Target }, { 1 })
        -- the ignore list is checked first, entries and all, and named as type() names a value; a call without one misses argument 2
        Fails("Argument 2 missing or nil", Cam.GetPartsObscuringTarget, Cam)
        Fails("Argument 2 missing or nil", Cam.GetPartsObscuringTarget, Cam, { Target })
        Fails("Expected table argument, got nil", Cam.GetPartsObscuringTarget, Cam, nil, nil)
        Fails("Expected table argument, got string", Cam.GetPartsObscuringTarget, Cam, "x", "y")
        Fails("Unable to cast value to Object", Cam.GetPartsObscuringTarget, Cam, "x", { 1 })
        Fails("Expected table argument, got userdata", Cam.GetPartsObscuringTarget, Cam, { Target }, workspace)
        Fails("Expected table argument, got userdata", Cam.GetPartsObscuringTarget, Cam, { Target }, Enum.KeyCode.A)
        Fails("Expected table argument, got vector", Cam.GetPartsObscuringTarget, Cam, { Target }, Target)
        Fails("Unable to cast Instance to Array", Cam.GetPartsObscuringTarget, Cam, workspace, {})
        Folder:Destroy()
        assert(Names(Cam, { Target }, {}) == "")
        "##,
    );
    assert!(!engine.state.borrow().log_history.iter().any(|entry| entry.text.contains("is not emulated")));
}

/// GetPartsObscuringTarget costs the work clock what a playing client spent on it (cost.rs OBSCURING_CALL to OBSCURING_STEP),
/// whatever else the Workspace holds: one point crossing 10 parts took a client 12.7-13.6 us (Studio 0.748) and 11.6-12.7 (0.747),
/// 1,000 points crossing 10 parts 1,243-1,313 us (0.748; on 0.747 100 such points 117-223), and 1,000 points crossing nothing
/// 172-179 us 300 studs out (0.747; a session at the origin, where the player's character stands, read 811-1,203 on 0.748).
#[test]
fn camera_parts_obscuring_a_target_cost_what_a_client_spent() {
    let (lua, engine) = start();
    crate::lua::modules::interpret_all(&lua, &engine);
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local function Per(Body)
            local N = 5
            local Start = os.clock()
            Body(N)
            local Middle = os.clock()
            for _ = 1, N do end
            return ((Middle - Start) - (os.clock() - Middle)) / N * 1e6
        end
        local Cam = Instance.new("Camera")
        Cam.CFrame = CFrame.lookAt(Vector3.new(0, 5, 60), Vector3.new(0, 5, 0))
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        for I = 1, 500 do
            local P = Instance.new("Part")
            P.Anchored = true
            P.Size = Vector3.new(1, 1, 1)
            P.CFrame = CFrame.new(500 + I * 3, 5, -500)
            P.Parent = Folder
        end
        local Points = table.create(1000)
        for I = 1, 1000 do
            Points[I] = Vector3.new((I % 10) * 0.5, 5 + (I // 10 % 10) * 0.5, 0)
        end
        local function Within(Label, Value, Low, High)
            assert(Value >= Low and Value <= High, `{Label} (us): {Value}`)
        end
        -- (the first query takes the parts just made into the spatial index, at cost.rs QUERY_TAKEN_PART each: not the timed calls')
        Cam:GetPartsObscuringTarget({}, {})
        Within("1,000 points crossing nothing", Per(function(N) for _ = 1, N do Cam:GetPartsObscuringTarget(Points, {}) end end), 150, 200)
        for I = 1, 10 do
            local P = Instance.new("Part")
            P.Anchored = true
            P.Size = Vector3.new(40, 40, 0.5)
            P.CFrame = CFrame.new(0, 5, 2 + I * 4)
            P.Parent = Folder
        end
        Within("1,000 points crossing 10 parts", Per(function(N) for _ = 1, N do Cam:GetPartsObscuringTarget(Points, {}) end end), 850, 1450)
        Within("one point crossing 10 parts", Per(function(N) for _ = 1, N do Cam:GetPartsObscuringTarget({ Points[1] }, {}) end end), 11, 15)
        Folder:Destroy()
        "##,
    );
}

/// A world of random parts for the broadphase's checks (`__SEED__`, `__COUNT__` things made, `__QUERIES__` queries a phase): nested
/// Folders and Models, parts under parts, Block, Ball and Cylinder parts, wedges, trusses and MeshParts, rotated or not, tiny, huge
/// and infinite sizes, NaN, infinite and far positions, twins in the same place (ties), CanCollide and CanQuery off; then phases of
/// random queries of every kind (casts, bounds queries, the legacy rays, GetPartsObscuringTarget, GetLargestCutoffDistance, the
/// PlayerMouse), with random filters, NaN and infinite arguments among them, each printed with what the work clock charged it,
/// between random changes (moves, resizes, flags, parts and models leaving, coming back, destroyed, added): `shared.Run(phase)`
/// and `shared.Change()`. `shared.Out` holds the lines.
const RANDOM_QUERIES: &str = r##"
local Seed = __SEED__
local function R()
	Seed = Seed * 48271 % 2147483647
	return Seed / 2147483647
end
local function Range(A, B) return A + (B - A) * R() end
local function Pick(List) return List[1 + math.floor(R() * #List)] end
local Store = game:GetService("ReplicatedStorage")
local Root = Instance.new("Folder")
Root.Name = "World"
Root.Parent = workspace
local Containers, Parts = { Root }, {}
local function Size()
	local K = R()
	if K < 0.04 then return Vector3.new(math.huge, Range(1, 4), Range(1, 4)) end
	if K < 0.08 then return Vector3.new(Range(300, 3000), Range(300, 3000), Range(0.2, 2)) end
	if K < 0.14 then return Vector3.new(0.001, 0.001, 0.001) end
	return Vector3.new(Range(0.5, 14), Range(0.5, 14), Range(0.5, 14))
end
local function Spot() return Vector3.new(Range(-60, 60), Range(-60, 60), Range(-60, 60)) end
local function Frame()
	local K, P = R(), Spot()
	if K < 0.03 then return CFrame.new(0 / 0, P.Y, P.Z) end
	if K < 0.05 then return CFrame.new(math.huge, P.Y, P.Z) end
	if K < 0.07 then return CFrame.new(P.X, P.Y, 1e30) end
	if K < 0.4 then return CFrame.new(P) end
	if K < 0.5 then return CFrame.new(P) * CFrame.Angles(0, math.pi / 2 * math.floor(R() * 4), 0) end
	return CFrame.new(P) * CFrame.Angles(Range(-3, 3), Range(-3, 3), Range(-3, 3))
end
local Count = 0
local function NewPart(Parent)
	Count += 1
	local Class = Pick({ "Part", "Part", "Part", "Part", "WedgePart", "CornerWedgePart", "TrussPart", "MeshPart" })
	local P = Instance.new(Class)
	P.Name = "P" .. Count
	if Class == "Part" then P.Shape = Pick({ Enum.PartType.Block, Enum.PartType.Ball, Enum.PartType.Cylinder }) end
	P.Anchored = true
	P.Size = Size()
	P.CFrame = Frame()
	if R() < 0.15 then P.CanCollide = false end
	if R() < 0.1 then P.CanQuery = false end
	P.Parent = Parent
	table.insert(Parts, P)
	if R() < 0.12 then
		local Twin = P:Clone()
		Count += 1
		Twin.Name = "P" .. Count
		Twin.Parent = if R() < 0.5 then Parent else Pick(Containers)
		table.insert(Parts, Twin)
	end
end
for _ = 1, __COUNT__ do
	local K = R()
	if K < 0.08 then
		local C = Instance.new(if R() < 0.5 then "Folder" else "Model")
		C.Name = "C" .. #Containers
		C.Parent = Pick(Containers)
		table.insert(Containers, C)
	elseif K < 0.25 and #Parts > 0 then
		NewPart(Pick(Parts))
	else
		NewPart(Pick(Containers))
	end
end

local Out = {}
local function Num(X) return string.format("%.9g", X) end
local function Vec(V) return Num(V.X) .. "," .. Num(V.Y) .. "," .. Num(V.Z) end
local function Named(X) return if X then X.Name else "nil" end
local function Hit(Result)
	if not Result then return "nil" end
	return Result.Instance.Name .. " " .. Vec(Result.Position) .. " " .. Vec(Result.Normal) .. " " .. Num(Result.Distance) .. " " .. tostring(Result.Material)
end
local function Names(List)
	local Seen = {}
	for _, P in List do table.insert(Seen, P.Name) end
	return "[" .. table.concat(Seen, ",") .. "]"
end
local function Point()
	local K = R()
	if K < 0.02 then return Vector3.new(0 / 0, 0, 0) end
	if K < 0.04 then return Vector3.new(math.huge, 0, 0) end
	return Vector3.new(Range(-90, 90), Range(-90, 90), Range(-90, 90))
end
local function Direction()
	local K = R()
	if K < 0.03 then return Vector3.new(0, 0 / 0, 0) end
	if K < 0.05 then return Vector3.zero end
	if K < 0.07 then return Vector3.new(0, 0, 1e30) end
	if K < 0.2 then return Pick({ Vector3.xAxis, Vector3.yAxis, Vector3.zAxis, -Vector3.xAxis, -Vector3.yAxis, -Vector3.zAxis }) * Range(1, 200) end
	return Vector3.new(Range(-1, 1), Range(-1, 1), Range(-1, 1)) * Range(1, 250)
end
local function Some(List, Most)
	local Chosen = {}
	for _ = 1, math.floor(R() * (Most + 1)) do table.insert(Chosen, Pick(List)) end
	return Chosen
end
local function Params(Overlap)
	local P = if Overlap then OverlapParams.new() else RaycastParams.new()
	if R() < 0.35 then
		P.FilterType = if R() < 0.5 then Enum.RaycastFilterType.Include else Enum.RaycastFilterType.Exclude
		local L = Some(Containers, 2)
		for _, X in Some(Parts, 3) do table.insert(L, X) end
		P.FilterDescendantsInstances = L
	end
	if R() < 0.2 then P.RespectCanCollide = true end
	if R() < 0.15 then P.ExcludeInstances = Some(Parts, 3) end
	if R() < 0.1 then P.IncludeInstances = Some(Containers, 2) end
	if Overlap and R() < 0.3 then P.MaxParts = 1 + math.floor(R() * 4) end
	return P
end
local Cam = Instance.new("Camera")
Cam.Parent = workspace
local Mouse = game:GetService("Players").LocalPlayer:GetMouse()
local Kinds = {
	function() local O, D, P = Point(), Direction(), Params(false) return "Raycast " .. Hit(workspace:Raycast(O, D, P)) end,
	function() local O, D, P = Point(), Direction(), Params(false) return "Raycast " .. Hit(workspace:Raycast(O, D, P)) end,
	function() local O, Radius, D, P = Point(), Range(0, 6), Direction(), Params(false) return "Spherecast " .. Hit(workspace:Spherecast(O, Radius, D, P)) end,
	function() local F, S, D, P = Frame(), Size(), Direction(), Params(false) return "Blockcast " .. Hit(workspace:Blockcast(F, S, D, P)) end,
	function()
		local Part, D, P = Pick(Parts), Direction(), Params(false)
		local Override = if R() < 0.3 then Frame() else nil
		local Resize = if R() < 0.3 then Size() else nil
		return "Shapecast " .. Hit(workspace:Shapecast(Part, D, P, Override, Resize))
	end,
	function() local F, S, P = Frame(), Size(), Params(true) return "Box " .. Names(workspace:GetPartBoundsInBox(F, S, P)) end,
	function() local O, Radius, P = Point(), Range(-2, 25), Params(true) return "Radius " .. Names(workspace:GetPartBoundsInRadius(O, Radius, P)) end,
	function() local Part, P = Pick(Parts), Params(true) return "InPart " .. Names(workspace:GetPartsInPart(Part, P)) end,
	function()
		local O, D = Point(), Direction()
		local A, B, C, E = workspace:FindPartOnRayWithIgnoreList(Ray.new(O, D), Some(Containers, 2))
		return "Ignore " .. Named(A) .. " " .. Vec(B) .. " " .. Vec(C) .. " " .. tostring(E)
	end,
	function()
		local O, D = Point(), Direction()
		local A, B, C, E = workspace:FindPartOnRayWithWhitelist(Ray.new(O, D), Some(Containers, 3))
		return "Whitelist " .. Named(A) .. " " .. Vec(B) .. " " .. Vec(C) .. " " .. tostring(E)
	end,
	function()
		Cam.CFrame = CFrame.new(Point())
		local Points = {}
		for _ = 1, 1 + math.floor(R() * 5) do table.insert(Points, Point()) end
		return "Obscuring " .. Names(Cam:GetPartsObscuringTarget(Points, Some(Containers, 2)))
	end,
	function()
		Cam.CFrame = CFrame.lookAt(Spot(), Spot())
		Cam.Focus = CFrame.new(Spot())
		return "Cutoff " .. Num(Cam:GetLargestCutoffDistance(Some(Parts, 3)))
	end,
	function()
		workspace.CurrentCamera.CFrame = CFrame.lookAt(Spot() * 1.5, Spot())
		Mouse.TargetFilter = if R() < 0.4 then Pick(Containers) else nil
		local At = Mouse.Hit
		return "Mouse " .. Named(Mouse.Target) .. " " .. Vec(At.Position) .. " " .. tostring(Mouse.TargetSurface)
	end,
}
local function Run(Phase)
	for I = 1, __QUERIES__ do
		local Kind = Pick(Kinds)
		local T0 = os.clock()
		local Ok, Line = pcall(Kind)
		local Spent = os.clock() - T0
		table.insert(Out, Phase .. "." .. I .. " " .. (if Ok then Line else "error " .. tostring(Line)) .. " | " .. Num(Spent * 1e9))
	end
end
local function Try(F) pcall(F) end
local function Change()
	for _ = 1, 20 do local P = Pick(Parts) Try(function() P.CFrame = Frame() end) end
	for _ = 1, 6 do local P = Pick(Parts) Try(function() P.Size = Size() end) end
	for _ = 1, 4 do local P = Pick(Parts) Try(function() P.CanQuery = not P.CanQuery end) end
	for _ = 1, 4 do local P = Pick(Parts) Try(function() P.CanCollide = not P.CanCollide end) end
	for _ = 1, 3 do local C = Pick(Containers) Try(function() C.Parent = if R() < 0.4 then Store else Pick(Containers) end) end
	for _ = 1, 4 do local P = Pick(Parts) Try(function() P.Parent = if R() < 0.4 then nil else Pick(Containers) end) end
	for _ = 1, 2 do local P = Pick(Parts) Try(function() P:Destroy() end) end
	for _ = 1, 6 do NewPart(Pick(Containers)) end
	-- a model made outside the Workspace, moved in whole
	local M = Instance.new("Model")
	M.Name = "C" .. #Containers
	table.insert(Containers, M)
	for _ = 1, 4 do NewPart(M) end
	M.Parent = Root
end
shared.Run = function(Phase)
	Run(Phase)
	shared.Out = table.concat(Out, "\n")
end
shared.Change = Change
"##;

/// The broadphase answers every spatial query as a test of every part in the Workspace's order does (`broadphase::SCAN`): the
/// same hits, positions, normals and distances to the bit, the same choice among ties, the same lists in the same order (MaxParts
/// included) and the same work-clock charge, on random worlds with NaN, infinite, huge, tiny, rotated and coincident parts, and
/// after random changes taken from the dom's log; after each batch of changes the index holds what a walk of the Workspace finds.
/// The tree stays balanced and its boxes hold their children.
#[test]
fn broadphase_answers_match_a_test_of_every_part() {
    let outputs = |seed: u32, scan: bool| {
        let (lua, engine) = start_play();
        super::broadphase::SCAN.with(|on| on.set(scan));
        let source = RANDOM_QUERIES.replace("__SEED__", &seed.to_string()).replace("__COUNT__", "400").replace("__QUERIES__", "160");
        run(&lua, &engine, &source);
        for (phase, changes) in [(1, 0), (2, 1), (3, 1), (4, 2)] {
            for _ in 0..changes {
                run(&lua, &engine, "shared.Change()");
                super::query::index_matches_a_walk(&engine).unwrap_or_else(|error| panic!("seed {seed}, before phase {phase}: {error}"));
            }
            run(&lua, &engine, &format!("shared.Run({phase})"));
            super::query::index_matches_a_walk(&engine).unwrap_or_else(|error| panic!("seed {seed}, after phase {phase}: {error}"));
        }
        super::broadphase::SCAN.with(|on| on.set(false));
        let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("globals").raw_get("shared").expect("shared");
        let out: String = shared.raw_get("Out").expect("the lines");
        (out, super::query::index_state(&engine))
    };
    let mut hits = 0;
    for seed in [7, 1234, 99991, 31337, 2024, 555] {
        let ((tree, (height, sound, parts)), (scan, _)) = (outputs(seed, false), outputs(seed, true));
        assert!(sound, "seed {seed}: the tree's links and boxes");
        assert!(parts > 150 && height <= 2 * (parts as f64).log2().ceil() as i32, "seed {seed}: {parts} parts, height {height}");
        assert_eq!(tree.lines().count(), scan.lines().count(), "seed {seed}");
        for (a, b) in tree.lines().zip(scan.lines()) {
            assert_eq!(a, b, "seed {seed}");
        }
        hits += tree.lines().filter(|line| !line.contains(" nil ") && !line.contains("[]") && !line.contains("Cutoff 0 ") && !line.contains("error")).count();
    }
    eprintln!("{hits} lines found parts");
    assert!(hits > 1500, "the queries found parts ({hits} lines)");
}

/// The parts far from a query cost it no host work, as a client's (Studio 0.747: a Raycast, Spherecast, Blockcast, Shapecast or
/// bounds query beside 0, 10,000 or 100,000 parts away costs the same): beside 50,000 parts 4,000 studs away every kind of query
/// takes a handful of parts from the broadphase, where each tested all 50,000 (1,000 Raycasts and 1,000 GetPartBoundsInBox calls
/// took about 3 s of host time), and moving, adding or removing parts takes them in without a walk of the Workspace.
#[test]
fn far_parts_cost_a_query_no_host_work() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r##"
        local Far = Instance.new("Folder")
        for I = 0, 49999 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.Size = Vector3.new(4, 4, 4)
            Part.CFrame = CFrame.new(4000 + (I % 50) * 6, ((I // 50) % 40) * 6, 4000 + (I // 2000) * 6)
            Part.Parent = Far
        end
        Far.Parent = workspace
        local Wall = Instance.new("Part")
        Wall.Anchored = true
        Wall.Size = Vector3.new(10, 10, 1)
        Wall.CFrame = CFrame.new(0, 50, 20)
        Wall.Parent = workspace
        local Ball = Instance.new("Part")
        Ball.Anchored = true
        Ball.Shape = Enum.PartType.Ball
        Ball.Size = Vector3.new(2, 2, 2)
        Ball.CFrame = CFrame.new(0, 80, 0)
        Ball.Parent = workspace
        shared.Far, shared.Ball = Far, Ball
        assert(workspace:Raycast(Vector3.new(0, 50, 0), Vector3.new(0, 0, 40)).Instance == Wall)
        "##,
    );
    super::broadphase::TAKEN.with(|taken| taken.set(0));
    super::broadphase::WORKSPACE_WALKS.with(|walks| walks.set(0));
    let start = std::time::Instant::now();
    run(
        &lua,
        &engine,
        r##"
        local O, Up, Ahead = Vector3.new(0, 50, 0), Vector3.new(0, 30, 0), Vector3.new(0, 0, 40)
        for _ = 1, 1000 do
            assert(workspace:Raycast(O, Ahead).Instance.Name == "Part")
            assert(#workspace:GetPartBoundsInBox(CFrame.new(0, 50, 20), Vector3.new(4, 4, 4)) == 1)
        end
        "##,
    );
    let host = start.elapsed().as_secs_f64();
    let taken = super::broadphase::TAKEN.with(|taken| taken.get());
    assert!(taken <= 2 * 1000 * 2, "each query took the wall and little else ({taken} parts for 2,000 queries)");
    assert!(host < 0.5, "2,000 queries beside 50,000 far parts took {host:.3} s of host time");
    super::broadphase::TAKEN.with(|taken| taken.set(0));
    run(
        &lua,
        &engine,
        r##"
        local O, Up = Vector3.new(0, 50, 0), Vector3.new(0, 20, 0)
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CFrame = CFrame.lookAt(Vector3.new(0, 50, 0), Vector3.new(0, 50, 20))
        Cam.Focus = CFrame.new(0, 50, 10)
        workspace.CurrentCamera.CFrame = CFrame.lookAt(Vector3.new(0, 50, -10), Vector3.new(0, 50, 20))
        local Mouse = game:GetService("Players").LocalPlayer:GetMouse()
        for _ = 1, 10 do
            workspace:Spherecast(O, 1, Up)
            workspace:Blockcast(CFrame.new(O), Vector3.new(2, 2, 2), Up)
            workspace:Shapecast(shared.Ball, Up)
            workspace:GetPartBoundsInRadius(O + Up, 2)
            workspace:GetPartsInPart(shared.Ball)
            workspace:FindPartOnRay(Ray.new(O, Up))
            Cam:GetPartsObscuringTarget({ Vector3.new(0, 50, 30) }, {})
            Cam:GetLargestCutoffDistance({})
            local _ = Mouse.Target
        end
        -- a far part moved, one added, one removed: the next query takes those, not the Workspace
        local Moved = shared.Far:GetChildren()[1]
        Moved.CFrame = Moved.CFrame + Vector3.new(0, 1, 0)
        local Added = Instance.new("Part")
        Added.Anchored = true
        Added.CFrame = CFrame.new(4000, 300, 4000)
        Added.Parent = shared.Far
        shared.Far:GetChildren()[2].Parent = nil
        assert(workspace:Raycast(O, Up) == nil)
        assert(#workspace:GetPartBoundsInBox(CFrame.new(4000, 300, 4000), Vector3.one) == 1)
        "##,
    );
    let taken = super::broadphase::TAKEN.with(|taken| taken.get());
    assert!(taken <= 10 * 20 + 10, "each query took a few parts ({taken} for 92 queries)");
    assert_eq!(super::broadphase::WORKSPACE_WALKS.with(|walks| walks.get()), 0, "changes taken from the log, no walk");
    let (_, sound, parts) = super::query::index_state(&engine);
    assert!(sound && parts == 50_002, "{parts} parts");
}

/// The broadphase orders what a query keeps as the Workspace's descendants are listed (`Dom::descendants`), from each instance's
/// attach number: after instances are made, moved within and out of the Workspace and back, destroyed, and one replaced in its
/// place (dom.rs replace_child, as a project mount does), every descendant sorted from a shuffled list comes back in that order.
#[test]
fn query_results_follow_the_order_the_workspace_lists_its_parts() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r##"
        local Seed = 4242
        local function R()
            Seed = Seed * 48271 % 2147483647
            return Seed / 2147483647
        end
        local function Pick(List) return List[1 + math.floor(R() * #List)] end
        local All = { workspace }
        for I = 1, 600 do
            local Made = Instance.new(if R() < 0.6 then "Part" elseif R() < 0.5 then "Folder" else "Model")
            Made.Name = "N" .. I
            pcall(function() Made.Parent = Pick(All) end)
            table.insert(All, Made)
            if R() < 0.2 then
                local Moved = Pick(All)
                pcall(function() Moved.Parent = if R() < 0.2 then game:GetService("ReplicatedStorage") else Pick(All) end)
            end
            if R() < 0.03 then
                pcall(function() Pick(All):Destroy() end)
            end
        end
        "##,
    );
    let workspace = engine.world().workspace;
    let check = |label: &str| {
        let state = engine.state.borrow();
        let listed = state.dom.descendants(workspace);
        assert!(listed.len() > 100, "{label}: {} listed", listed.len());
        let mut shuffled: Vec<_> = listed.iter().rev().copied().collect();
        let half = shuffled.len() / 2;
        shuffled.rotate_left(half);
        super::broadphase::sort_by_order(&state.dom, workspace, &mut shuffled, |id| *id);
        assert!(shuffled == listed, "{label}: sorted as listed");
    };
    check("made and moved");
    {
        let mut state = engine.state.borrow_mut();
        let state = &mut *state;
        let parent = state.dom.descendants(workspace)[10];
        let old = state.dom.at(parent).children.first().copied().or_else(|| state.dom.at(workspace).children.first().copied()).expect("a child");
        let parent = state.dom.parent_of(old).expect("a parent");
        let class = engine.reflection.class_id("Folder").expect("Folder");
        let new = state.dom.create(&engine.reflection, class);
        state.dom.replace_child(parent, old, new);
    }
    check("one replaced in its place");
}

/// broadphase::sort_by_order, which reads the order from a trie that walks up from every part at once build only as far as the
/// parts' paths meet, puts any list of instances in exactly the order its first form did (each one's whole list of attach numbers
/// from below the root, compared as lists in a stable sort: broadphase::sort_by_order_lists): random trees with folder chains up
/// to 3,000 deep, parts under parts, models and folders moved, destroyed and parented out of the Workspace, and a folder
/// replaced in its place (dom.rs replace_child), whose detached old self keeps the attach number its replacement takes, so a list
/// holding parts of both has two tops with one number and the trie with one node for each list (Trie::keyed) orders it; random
/// lists of 2 to 400 instances with repeats, the Workspace itself and instances outside it among them, against the Workspace
/// and the DataModel as the root. Its walks take a step onto each instance on the paths at most, and one more for each part where
/// its walk meets another's; parts in one folder at the bottom of a deep chain take one step each.
#[test]
fn query_order_is_the_order_of_whole_attach_number_lists() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r##"
        local Seed = 777
        local function R()
            Seed = Seed * 48271 % 2147483647
            return Seed / 2147483647
        end
        local function Pick(List) return List[1 + math.floor(R() * #List)] end
        local Outside = Instance.new("Folder")
        Outside.Parent = game:GetService("ReplicatedStorage")
        local All = { workspace, Outside }
        for I = 1, 1500 do
            local Made = Instance.new(if R() < 0.5 then "Part" elseif R() < 0.5 then "Folder" else "Model")
            pcall(function() Made.Parent = Pick(All) end)
            table.insert(All, Made)
            if R() < 0.01 then
                -- (a chain below it, made from the bottom up, with a part at the bottom)
                local Top = Instance.new("Part")
                table.insert(All, Top)
                for _ = 1, math.floor(R() * 3000) do
                    local Folder = Instance.new("Folder")
                    Top.Parent = Folder
                    Top = Folder
                end
                Top.Parent = Made
            end
            if R() < 0.15 then
                local Moved = Pick(All)
                pcall(function() Moved.Parent = if R() < 0.2 then Outside else Pick(All) end)
            end
            if R() < 0.02 then
                pcall(function() Pick(All):Destroy() end)
            end
        end
        -- 50 parts in a folder at the bottom of a chain 2,000 deep
        local Bottom = Instance.new("Folder")
        Bottom.Name = "Bottom"
        for _ = 1, 50 do
            Instance.new("Part").Parent = Bottom
        end
        local Top = Bottom
        for _ = 1, 2000 do
            local Folder = Instance.new("Folder")
            Top.Parent = Folder
            Top = Folder
        end
        Top.Parent = workspace
        shared.Bottom = Bottom
        "##,
    );
    let workspace = engine.world().workspace;
    // a folder replaced in its place, each holding parts
    let replaced = {
        let mut state = engine.state.borrow_mut();
        let (folder, part) = (engine.reflection.class_id("Folder").expect("Folder"), engine.reflection.class_id("Part").expect("Part"));
        let (old, new) = (state.dom.create(&engine.reflection, folder), state.dom.create(&engine.reflection, folder));
        state.dom.reparent(old, Some(workspace));
        for holder in [old, new] {
            for _ in 0..5 {
                let made = state.dom.create(&engine.reflection, part);
                state.dom.reparent(made, Some(holder));
            }
        }
        state.dom.replace_child(workspace, old, new);
        assert_eq!(state.dom.attached_order(old), state.dom.attached_order(new), "the replacement takes the old folder's number");
        old
    };
    let state = engine.state.borrow();
    let game = state.dom.parent_of(workspace).expect("the DataModel");
    let mut pool = vec![game, workspace, replaced];
    pool.extend(state.dom.descendants(game));
    pool.extend(state.dom.descendants(replaced));
    assert!(pool.len() > 10_000, "{} instances", pool.len());
    let on_paths = |items: &[crate::InstanceId], root: crate::InstanceId| {
        let mut seen = rustc_hash::FxHashSet::default();
        for &item in items {
            let mut node = state.dom.parent_of(item);
            while let Some(id) = node.filter(|&id| id != root) {
                if !seen.insert(id) {
                    break;
                }
                node = state.dom.parent_of(id);
            }
        }
        seen.len()
    };
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let keyed = super::broadphase::KEYED.with(|keyed| keyed.get());
    for round in 0..400 {
        let root = if round % 4 == 3 { game } else { workspace };
        let count = 2 + next(399);
        let mut items: Vec<crate::InstanceId> = (0..count).map(|_| pool[next(pool.len())]).collect();
        if round % 10 == 0 {
            // (parts of both folders, the replaced one and its replacement)
            items.extend(state.dom.at(replaced).children.iter().copied());
            items.extend(state.dom.at(state.dom.at(workspace).children.iter().copied().find(|&child| state.dom.attached_order(child) == state.dom.attached_order(replaced)).expect("the replacement")).children.iter().copied());
            let at = next(items.len());
            items.rotate_left(at);
        }
        let mut lists = items.clone();
        super::broadphase::sort_by_order_lists(&state.dom, root, &mut lists, |id| *id);
        let mut sorted = items.clone();
        let steps = super::broadphase::sort_by_order(&state.dom, root, &mut sorted, |id| *id);
        assert!(sorted == lists, "round {round}: {count} instances ordered as their whole lists are");
        assert!(steps <= on_paths(&items, root) + items.len(), "round {round}: {steps} steps for {} instances on the paths", on_paths(&items, root));
    }
    assert!(super::broadphase::KEYED.with(|keyed| keyed.get()) > keyed, "the lists with two tops of one number were ordered by the keyed trie");
    // the folder at the bottom of the chain: one step a part
    let bottom = *pool.iter().find(|&&id| state.dom.get(id).is_some_and(|instance| &*instance.name.to_str_lossy() == "Bottom")).expect("the bottom folder");
    let mut parts: Vec<_> = state.dom.at(bottom).children.iter().rev().copied().collect();
    let steps = super::broadphase::sort_by_order(&state.dom, workspace, &mut parts, |id| *id);
    assert!(parts == state.dom.at(bottom).children.to_vec(), "the parts in their folder's order");
    assert_eq!(steps, 50, "one step for each of the 50 parts, 2,000 folders deep");
}

/// A spatial query puts what it keeps in the Workspace's order with walks up from the parts that stop where the parts' paths meet
/// (world/broadphase.rs sort_by_order), so 2,000 parts in a folder at the bottom of a 10,000-deep chain cost GetPartBoundsInBox,
/// GetPartsObscuringTarget and a Raycast through them about as much host work as the same parts in a folder of the Workspace, in
/// the same order, and the work clock charges the box and the Raycast the same, as a playing client's cost the same at any depth
/// (Studio 0.747: 100 parts 12.1 us in the Workspace, 12.2 1,000 deep, 12.5 3,000 deep); GetPartsObscuringTarget costs more by
/// the walks a client's casts take up its parts (camera.rs parts_obscuring: its ancestors' answers kept for the call, so the
/// host walks them once). Making each part's whole list of attach numbers and comparing them took 0.2 s of host time a query here
/// and 262 MB of heap. Parts whose paths meet only far up (50 parts, each at the bottom of a 1,000-deep chain of its own) still walk
/// every instance between, and pay cost.rs QUERY_ORDER_STEP for each step past QUERY_ORDER_FREE a part.
#[test]
fn query_order_walks_the_tree_only_up_to_where_the_parts_paths_meet() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        -- (a chain of folders `Depth` deep holding `Inner`, made from the bottom up so that making it walks nothing)
        local function Wrap(Inner, Depth)
            local Outer = Inner
            for _ = 1, Depth do
                local Folder = Instance.new("Folder")
                Outer.Parent = Folder
                Outer = Folder
            end
            return Outer
        end
        -- `Count` parts in a stack along Z at X, in a folder `Depth` deep, or each in a chain of its own `Apart` deep
        local function Scene(X, Count, Depth, Apart)
            local Parts = {}
            local Holder = Instance.new("Folder")
            for I = 1, Count do
                local Part = Instance.new("Part")
                Part.Anchored, Part.Size, Part.CFrame = true, Vector3.new(1, 1, 0.5), CFrame.new(X, 50, -I * 0.01)
                if Apart then
                    local Box = Instance.new("Folder")
                    Part.Parent = Box
                    Wrap(Box, Apart - 1).Parent = workspace
                else
                    Part.Parent = Holder
                end
                Parts[I] = Part
            end
            if not Apart then
                Wrap(Holder, Depth).Parent = workspace
            end
            return { X = X, Parts = Parts }
        end
        shared.Scenes = { Flat = Scene(0, 2000, 0), Deep = Scene(1000, 2000, 10000), Few = Scene(2000, 50, 0), Apart = Scene(3000, 50, 0, 1000) }
        workspace:Raycast(Vector3.new(0, 5000, 0), Vector3.new(0, 1, 0))
        shared.Work = {}
        local Camera = workspace.CurrentCamera
        local function Pass(S)
            local X, Count = S.X, #S.Parts
            for _ = 1, 5 do
                local Found = workspace:GetPartBoundsInBox(CFrame.new(X, 50, -Count * 0.005), Vector3.new(0.5, 0.5, Count * 0.01 + 1))
                assert(#Found == Count, `the box found {#Found} of {Count}`)
                for I, Part in Found do
                    assert(Part == S.Parts[I], "the parts in the Workspace's order")
                end
                Camera.CFrame = CFrame.new(X, 50, 10)
                local Before = os.clock()
                local Obscuring = Camera:GetPartsObscuringTarget({ Vector3.new(X, 50, -Count * 0.01 - 5) }, {})
                S.Obscuring += os.clock() - Before
                assert(#Obscuring == Count, `{#Obscuring} parts obscure the point`)
                local Hit = workspace:Raycast(Vector3.new(X, 50, 10), Vector3.new(0, 0, -Count * 0.01 - 20))
                assert(Hit and Hit.Instance == S.Parts[1], "the ray hits the first part")
            end
        end
        -- each scene twice, the second pass timed (the first warms the members and instances the frame touches)
        shared.Obscuring = {}
        function shared.Use(Name)
            local S = shared.Scenes[Name]
            S.Obscuring = 0
            Pass(S)
            S.Obscuring = 0
            local Start = os.clock()
            Pass(S)
            -- (GetPartsObscuringTarget's own work apart: it grows with depth)
            shared.Work[Name] = os.clock() - Start - S.Obscuring
            shared.Obscuring[Name] = S.Obscuring
        end
        "#,
    );
    let mut host = Vec::new();
    for name in ["Flat", "Deep", "Few", "Apart"] {
        let start = std::time::Instant::now();
        run(&lua, &engine, &format!("shared.Use(\"{name}\")"));
        host.push(start.elapsed().as_secs_f64());
    }
    // (each of the 50 parts apart walks its 1,000 instances: 50,000 steps a query, 10 queries but GetPartsObscuringTarget's, whose
    // work is apart)
    let apart = 10.0 * super::broadphase::order_cost(50 * 1000, 50);
    assert!(apart > 1e-3, "the walks apart are charged: {apart} s");
    run(
        &lua,
        &engine,
        &format!(
            r#"
            local Work = shared.Work
            assert(Work.Flat > 0, "the passes cost work")
            assert(math.abs(Work.Deep - Work.Flat) < 1e-9, `the deep scene's pass cost {{Work.Deep}} s of work where the flat one's {{Work.Flat}}`)
            assert(math.abs(Work.Apart - Work.Few - {apart}) < 1e-9, `the parts apart cost {{Work.Apart - Work.Few}} s more work than the few in a folder, not {apart}`)
            "#
        ),
    );
    // GetPartsObscuringTarget's five calls: the wall at place i is checked 2,000 - i + 1 times, each walking up for the i walls listed
    // before it, 3 ancestors in the Workspace's folder and 10,003 at the bottom of the chain
    let count = 2000usize;
    let checks: f64 = (0..count).map(|place| ((count - place + 1) * place) as f64).sum();
    let walks = |ancestors: u32| checks * f64::from(ancestors) * crate::cost::filter_step(ancestors) * crate::cost::OBSCURING_STEP;
    let deeper = 5.0 * (walks(10_003) - walks(3));
    run(&lua, &engine, &format!("assert(math.abs((shared.Obscuring.Deep - shared.Obscuring.Flat) / {deeper:e} - 1) < 1e-9, `GetPartsObscuringTarget deep cost {{shared.Obscuring.Deep - shared.Obscuring.Flat}} s more than in the Workspace, not {deeper:e}`)"));
    let (flat, deep) = (host[0], host[1]);
    assert!(deep < 0.25, "two passes of 15 queries of 2,000 parts 10,000 deep took {deep:.3} s, {flat:.3} s in the Workspace");
}

/// A spatial query pays for the parts it takes into the spatial index as a client's does (cost.rs QUERY_TAKE_IN, QUERY_TAKEN_PART;
/// Studio 0.747: 0.5 us and 1.1 us for each part that entered the Workspace since the last query or the physics step), and nothing
/// for parts moved or taken out since, nor again for parts it took in; the PlayerMouse's ray and GetLargestCutoffDistance take
/// nothing in and pay nothing for it, GetPartsObscuringTarget and the bounds queries do; the physics step takes parts in unpaid.
#[test]
fn a_query_pays_for_the_parts_it_takes_into_the_spatial_index() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Pool = {}
        for I = 1, 100 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.CFrame = CFrame.new(1000 + I * 3, 0, 0)
            Pool[I] = Part
        end
        local O, Up = Vector3.new(0, 50, 0), Vector3.new(0, 10, 0)
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CFrame = CFrame.lookAt(Vector3.new(0, 50, 0), Vector3.new(0, 50, 20))
        Cam.Focus = CFrame.new(0, 50, 10)
        local Mouse = game:GetService("Players").LocalPlayer:GetMouse()
        local function Spent(F)
            local T0 = os.clock()
            F()
            return (os.clock() - T0) * 1e6
        end
        local Calls = {
            Raycast = function() workspace:Raycast(O, Up) end,
            Box = function() workspace:GetPartBoundsInBox(CFrame.new(O), Vector3.one) end,
            Obscuring = function() Cam:GetPartsObscuringTarget({ O + Up }, {}) end,
            Cutoff = function() Cam:GetLargestCutoffDistance({}) end,
            Mouse = function() local _ = Mouse.Target end,
        }
        local Alone = {}
        for Name, Call in Calls do
            Call()
            Alone[Name] = Spent(Call)
        end
        local function Enter(K) for J = 1, K do Pool[J].Parent = workspace end end
        local function Leave(K) for J = 1, K do Pool[J].Parent = nil end end
        local function Paid(Name) return Spent(Calls[Name]) - Alone[Name] end
        local function Near(Got, Want, What) assert(math.abs(Got - Want) < 1e-3, What .. ": " .. Got .. " where " .. Want) end
        for _, K in { 1, 10, 100 } do
            Enter(K)
            Near(Paid("Mouse"), 0, K .. " waiting, a mouse read")
            Near(Paid("Cutoff"), 0, K .. " waiting, GetLargestCutoffDistance")
            Near(Paid("Raycast"), 0.5 + 1.1 * K, K .. " taken in by a Raycast")
            Near(Paid("Raycast"), 0, K .. " already in")
            for J = 1, K do
                Pool[J].CFrame = Pool[J].CFrame + Vector3.new(0, 1, 0)
            end
            Near(Paid("Raycast"), 0, K .. " moved")
            Leave(K)
            Near(Paid("Raycast"), 0, K .. " taken out")
        end
        Enter(10)
        Leave(10)
        Near(Paid("Raycast"), 0, "in and out before the query")
        Enter(10)
        Near(Paid("Box"), 0.5 + 11, "taken in by GetPartBoundsInBox")
        Leave(10)
        Enter(10)
        Near(Paid("Obscuring"), 0.5 + 11, "taken in by GetPartsObscuringTarget")
        Leave(10)
        Enter(10)
        "##,
    );
    assert_eq!(engine.state.borrow().cameras.unindexed.len(), 10, "ten waiting");
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    assert_eq!(engine.state.borrow().cameras.unindexed.len(), 0, "the physics step took them in: no query pays for them");
}

/// Parts that entered the Workspace and wait for the spatial index (camera.rs Cameras::unindexed) stay out of the broadphase's tree
/// until a query or the physics step takes them in, so the PlayerMouse's ray and GetLargestCutoffDistance, which do not see them,
/// are not handed them: with 10,000 waiting parts across the mouse's ray and 10,000 across the cutoff's rays, 2,000 reads of each
/// take from the index what they take beside none, cost about its host work and the same work clock, as a playing client's reads
/// cost the same beside 0 or 10,000 waiting parts (Studio 0.747: Target 0.44 us, GetLargestCutoffDistance 1.26 us). Each read
/// used to test and skip every one of them (2,000 reads took 120 million parts from the index). A Raycast then takes them in, and
/// both see them.
#[test]
fn parts_waiting_for_the_spatial_index_are_not_handed_to_the_mouse_or_the_cutoff() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Mouse = game:GetService("Players").LocalPlayer:GetMouse()
        local Ray = Mouse.UnitRay
        local O = Vector3.new(0, 300, 0)
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CFrame = CFrame.lookAt(O + Vector3.new(0, 5, 30), O + Vector3.new(0, 5, 0))
        Cam.Focus = CFrame.new(O + Vector3.new(0, 5, 0))
        local OnRay = CFrame.lookAt(Ray.Origin + Ray.Direction * 50, Ray.Origin + Ray.Direction * 51)
        local OnCut = CFrame.new(O + Vector3.new(0, 5, 12))
        function shared.Add(Count)
            for I = 1, Count do
                for _, Where in { OnRay, OnCut } do
                    local Part = Instance.new("Part")
                    Part.Anchored, Part.Size, Part.CFrame = true, Vector3.new(4, 4, 1), Where * CFrame.new(0, 0, -I * 0.001)
                    Part.Parent = workspace
                end
            end
        end
        shared.Work = {}
        function shared.Reads(Count, Seen, Label)
            local Start = os.clock()
            for _ = 1, Count do
                assert((Mouse.Target ~= nil) == Seen, "the mouse's target")
                assert((Cam:GetLargestCutoffDistance({}) > 0) == Seen, "the cutoff")
            end
            shared.Work[Label] = os.clock() - Start
        end
        shared.Reads(10, false, "Warm")
        "##,
    );
    let taken = || super::broadphase::TAKEN.with(|taken| taken.get());
    let reads = |label: &str| {
        let (before, start) = (taken(), std::time::Instant::now());
        run(&lua, &engine, &format!("shared.Reads(2000, false, \"{label}\")"));
        (taken() - before, start.elapsed().as_secs_f64())
    };
    let (alone, alone_host) = reads("Alone");
    run(&lua, &engine, "shared.Add(10000)");
    assert_eq!(engine.state.borrow().cameras.unindexed.len(), 20_000, "every new part waits");
    let (waiting, waiting_host) = reads("Waiting");
    assert_eq!(waiting, alone, "the reads took {waiting} parts from the index beside the waiting parts, {alone} beside none");
    run(&lua, &engine, "assert(math.abs(shared.Work.Waiting - shared.Work.Alone) < 1e-9, `the reads cost {shared.Work.Waiting} s of work beside the waiting parts, {shared.Work.Alone} s beside none`)");
    assert!(waiting_host < 0.25, "2,000 reads beside 20,000 waiting parts took {waiting_host:.3} s of host time, {alone_host:.3} s beside none");
    run(&lua, &engine, "workspace:Raycast(Vector3.new(0, 5000, 0), Vector3.new(0, 1, 0))");
    assert!(engine.state.borrow().cameras.unindexed.is_empty(), "the Raycast took them in");
    run(&lua, &engine, "shared.Reads(10, true, \"Seen\")");
}

/// Camera:GetPartsObscuringTarget costs the work clock what a client's casts spend (cost.rs OBSCURING_CALL to OBSCURING_NOT_FINITE,
/// Studio 0.747): a client casts again for each part it lists, each cast checking the parts before its hit against the script's
/// ignore list and the parts listed so far, so ten walls 100 folders deep behind an ignore list of five parts cost eleven casts,
/// 65 checks and their walks; an entry of the points that is no Vector3 and a point that is not finite cost what a client spends
/// skipping them. The prices before charged 0.8 us a point, a ray's price for the walls and 1.1 us a wall listed, and nothing for
/// the depth, the ignore list's walks or the skipped entries.
#[test]
fn obscuring_parts_cost_what_a_clients_casts_spend() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Far = Instance.new("Folder")
        Far.Parent = workspace
        local Ignore = {}
        for I = 1, 5 do
            local Part = Instance.new("Part")
            Part.Anchored, Part.Position = true, Vector3.new(3000 + I * 3, 500, 3000)
            Part.Parent = Far
            Ignore[I] = Part
        end
        -- 100 folders: a wall at the bottom has 102 ancestors (the Workspace and the DataModel)
        local Node = workspace
        for _ = 1, 100 do
            local Folder = Instance.new("Folder")
            Folder.Parent = Node
            Node = Folder
        end
        local O = Vector3.new(0, 2000, 0)
        for I = 1, 10 do
            local Wall = Instance.new("Part")
            Wall.Anchored, Wall.Size, Wall.Position = true, Vector3.new(8, 8, 0.2), O + Vector3.new(0, 0, I * 2)
            Wall.Parent = Node
        end
        local Cam = Instance.new("Camera")
        Cam.CFrame = CFrame.lookAt(O + Vector3.new(0, 0, 250), O)
        Cam.Parent = workspace
        local Behind, Open = { O }, { O + Vector3.new(500, 0, 0) }
        local Skips, Unfinite = table.create(101, 5), table.create(11, Vector3.new(0 / 0, 0, 0))
        Skips[101], Unfinite[11] = Open[1], Open[1]
        local function Cost(Call)
            Call()
            local Start = os.clock()
            Call()
            return os.clock() - Start
        end
        assert(#Cam:GetPartsObscuringTarget(Behind, Ignore) == 10, "the ten walls")
        shared.Obscuring = {
            Walls = Cost(function() Cam:GetPartsObscuringTarget(Behind, Ignore) end) - Cost(function() Cam:GetPartsObscuringTarget(Open, Ignore) end),
            Skipped = Cost(function() Cam:GetPartsObscuringTarget(Skips, Ignore) end) - Cost(function() Cam:GetPartsObscuringTarget(Open, Ignore) end),
            Unfinite = Cost(function() Cam:GetPartsObscuringTarget(Unfinite, Ignore) end) - Cost(function() Cam:GetPartsObscuringTarget(Open, Ignore) end),
        }
        "##,
    );
    // ten walls: ten more casts than the open point's one; the wall at place i (0-based) is checked by the 10 - i casts whose hit
    // is at or past it and the last, each check walking up 102 steps for each of the five ignored parts and the i listed before it
    let (walls, ancestors, ignored) = (10usize, 102.0, 5.0);
    let mut checks = 0.0;
    let mut steps = 0.0;
    for place in 0..walls {
        let times = (walls - place + 1) as f64;
        checks += times;
        steps += times * (ignored + place as f64) * ancestors;
    }
    let walls_cost = walls as f64 * (crate::cost::OBSCURING_CAST + crate::cost::OBSCURING_LISTED)
        + checks * crate::cost::OBSCURING_CHECKED
        + steps * crate::cost::filter_step(102) * crate::cost::OBSCURING_STEP;
    let checks_list = [("Walls", walls_cost), ("Skipped", 100.0 * crate::cost::OBSCURING_SKIPPED), ("Unfinite", 10.0 * crate::cost::OBSCURING_NOT_FINITE)];
    for (name, price) in checks_list {
        run(&lua, &engine, &format!("assert(math.abs(shared.Obscuring.{name} - {price:e}) < 1e-9, `{name}: {{shared.Obscuring.{name}}} s more work, not {price:e}`)"));
    }
}

/// The walks up a query's parts that a client's filters take cost the work clock what a client spends on them (cost.rs
/// filter_step, listed_step, QUERY_FILTERED_PART, MOUSE_FILTER_STEP, CUTOFF_IGNORE_STEP; Studio 0.747): with parts 300 folders
/// deep, a box's FilterDescendantsInstances of 100 parts elsewhere walks each part to the top once for each entry, an Exclude of
/// the chain's top walks each part to it and leaves it out, ExcludeInstances walks each part to the top once; a Raycast's walks
/// the part it hits; the PlayerMouse's TargetFilter walks the parts under it to it and the hit to the top; GetLargestCutoffDistance's
/// ignore list walks each part its rays meet. Each of these walked for nothing.
#[test]
fn filter_walks_cost_what_a_client_spends_on_them() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Far = Instance.new("Folder")
        Far.Parent = workspace
        local Pool = table.create(100)
        for I = 1, 100 do
            local Part = Instance.new("Part")
            Part.Anchored, Part.Position = true, Vector3.new(3000 + I * 3, 500, 3000)
            Part.Parent = Far
            Pool[I] = Part
        end
        -- 300 folders: a part at the bottom has 302 ancestors (the Workspace and the DataModel)
        local function Chain(Parent)
            local Top = Instance.new("Folder")
            Top.Parent = Parent
            local Node = Top
            for _ = 2, 300 do
                local Folder = Instance.new("Folder")
                Folder.Parent = Node
                Node = Folder
            end
            return Top, Node
        end
        local Top, Bottom = Chain(workspace)
        for I = 1, 10 do
            local Part = Instance.new("Part")
            Part.Anchored, Part.Size, Part.Position = true, Vector3.one, Vector3.new(I * 2, 1000, 0)
            Part.Parent = Bottom
        end
        local Wall = Instance.new("Part")
        Wall.Anchored, Wall.Size, Wall.Position = true, Vector3.new(4, 1, 4), Vector3.new(100, 1000, 0)
        Wall.Parent = Bottom
        local function Params(Class, Field, Kind, List)
            local Made = Class.new()
            Made.FilterType = Kind
            Made[Field] = List
            return Made
        end
        local Exclude = Enum.RaycastFilterType.Exclude
        local Box, Size, None = CFrame.new(11, 1000, 0), Vector3.new(24, 2, 2), OverlapParams.new()
        local FarList, Topped = Params(OverlapParams, "FilterDescendantsInstances", Exclude, Pool), Params(OverlapParams, "FilterDescendantsInstances", Exclude, { Top })
        local Excluded = Params(OverlapParams, "ExcludeInstances", Exclude, Pool)
        local Cast, Plain = Params(RaycastParams, "FilterDescendantsInstances", Exclude, Pool), RaycastParams.new()
        local From, Down = Vector3.new(100, 1010, 0), Vector3.new(0, -20, 0)
        local function Cost(Call)
            Call()
            local Start = os.clock()
            Call()
            return os.clock() - Start
        end
        local function Extra(A, B)
            return Cost(A) - Cost(B)
        end
        local function Bounds(P)
            return function() workspace:GetPartBoundsInBox(Box, Size, P) end
        end
        -- the mouse: five walls under a chain of its own across its ray, and a wall behind them in the Workspace
        local Mouse = game:GetService("Players").LocalPlayer:GetMouse()
        local Ray = Mouse.UnitRay
        local MouseTop, MouseBottom = Chain(workspace)
        for I = 1, 6 do
            local At = Ray.Origin + Ray.Direction.Unit * (40 + I * 2)
            local Made = Instance.new("Part")
            Made.Anchored, Made.Size, Made.CFrame = true, Vector3.new(6, 6, 0.2), CFrame.lookAt(At, At + Ray.Direction)
            Made.Parent = if I == 6 then workspace else MouseBottom
        end
        -- the cutoff: three big walls between Focus and the camera, under a chain of their own
        local CutTop, CutBottom = Chain(workspace)
        local O = Vector3.new(0, 3000, 0)
        local Cam = Instance.new("Camera")
        Cam.CFrame, Cam.Focus = CFrame.lookAt(O + Vector3.new(0, 0, 30), O), CFrame.new(O)
        Cam.Parent = workspace
        for I = 1, 3 do
            local Made = Instance.new("Part")
            Made.Anchored, Made.Size, Made.Position = true, Vector3.new(60, 60, 0.2), O + Vector3.new(0, 0, 2 + I * 2)
            Made.Parent = CutBottom
        end
        workspace:Raycast(Vector3.new(0, 9000, 0), Vector3.new(0, 1, 0))
        local NoIgnore, Ignored = {}, { CutTop }
        local function Target(Filter)
            return function()
                Mouse.TargetFilter = Filter
                local _ = Mouse.Target
            end
        end
        shared.Walks = {
            Descendants = Extra(Bounds(FarList), Bounds(None)),
            LeftOut = Extra(Bounds(Topped), Bounds(None)),
            Excluded = Extra(Bounds(Excluded), Bounds(None)),
            Ray = Extra(function() workspace:Raycast(From, Down, Cast) end, function() workspace:Raycast(From, Down, Plain) end),
            Mouse = Extra(Target(MouseTop), Target(nil)),
            Cutoff = Extra(function() Cam:GetLargestCutoffDistance(Ignored) end, function() Cam:GetLargestCutoffDistance(NoIgnore) end),
        }
        "##,
    );
    let cost = crate::cost::filter_step(302);
    let read = |entries: f64| crate::cost::PARAMS_LIST + entries * crate::cost::PARAMS_LISTED;
    let checks = [
        // (each of the 10 parts walked to the top for each of the 100 entries)
        ("Descendants", read(100.0) + 10.0 * 100.0 * 302.0 * cost),
        // (each walked 300 steps to the top folder and left out: no QUERY_BOX_PART)
        ("LeftOut", read(1.0) + 10.0 * (300.0 * cost + crate::cost::QUERY_FILTERED_PART - crate::cost::QUERY_BOX_PART)),
        // (each part and its 302 ancestors looked at in a list of 100)
        ("Excluded", read(100.0) + 10.0 * 303.0 * crate::cost::listed_step(100)),
        ("Ray", read(100.0) + 100.0 * 302.0 * cost),
        // (five walls under the filter, each 300 steps to it, and the hit in the Workspace 2 to the top; the five paid as met)
        ("Mouse", 5.0 * crate::cost::MOUSE_FILTERED + (5.0 * 300.0 + 2.0) * crate::cost::MOUSE_FILTER_STEP),
        // (five rays each meeting three ignored walls, each 300 steps to the top folder, instead of hitting the first)
        ("Cutoff", crate::cost::IGNORE_LISTED + 5.0 * 2.0 * crate::cost::RAY_CROSSED + 5.0 * 3.0 * 300.0 * crate::cost::CUTOFF_IGNORE_STEP),
    ];
    for (name, price) in checks {
        run(&lua, &engine, &format!("assert(math.abs(shared.Walks.{name} - {price:e}) < 1e-9, `{name}: {{shared.Walks.{name}}} s more work, not {price:e}`)"));
    }
}

/// A query's params lists cost the work clock what a client spends reading them at every query (cost.rs PARAMS_LIST,
/// PARAMS_LISTED; Studio 0.747: 7.8 ns an entry and 0.1 us a list that holds any): FilterDescendantsInstances, ExcludeInstances
/// and IncludeInstances of 10,000 parts away from a box query and a Raycast, against the same queries with lists of one. Each
/// query read its lists for nothing.
#[test]
fn params_lists_cost_what_a_client_spends_reading_them_at_every_query() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Far = Instance.new("Folder")
        Far.Parent = workspace
        local Pool = table.create(10000)
        for I = 1, 10000 do
            local Part = Instance.new("Part")
            Part.Anchored, Part.Position = true, Vector3.new(3000 + (I % 100) * 3, 500, 3000 + (I // 100) * 3)
            Part.Parent = Far
            Pool[I] = Part
        end
        local function Params(Class, Field, Count)
            local Made = Class.new()
            Made[Field] = table.move(Pool, 1, Count, 1, table.create(Count))
            return Made
        end
        local Box, Size, V, Down = CFrame.new(0, 2000, 0), Vector3.one, Vector3.new(0, 2000, 0), Vector3.new(0, -10, 0)
        local function Cost(Call)
            Call()
            local Start = os.clock()
            Call()
            return os.clock() - Start
        end
        local function Extra(Class, Field, Query)
            local Many, One = Params(Class, Field, 10000), Params(Class, Field, 1)
            return Cost(function() Query(Many) end) - Cost(function() Query(One) end)
        end
        local function Bounds(P) workspace:GetPartBoundsInBox(Box, Size, P) end
        local function Cast(P) workspace:Raycast(V, Down, P) end
        local Empty, Single = OverlapParams.new(), Params(OverlapParams, "FilterDescendantsInstances", 1)
        shared.Lists = {
            Descendants = Extra(OverlapParams, "FilterDescendantsInstances", Bounds),
            Excluded = Extra(OverlapParams, "ExcludeInstances", Bounds),
            Included = Extra(OverlapParams, "IncludeInstances", Bounds),
            Ray = Extra(RaycastParams, "FilterDescendantsInstances", Cast),
            List = Cost(function() Bounds(Single) end) - Cost(function() Bounds(Empty) end),
        }
        "##,
    );
    let many = 9_999.0 * crate::cost::PARAMS_LISTED;
    let checks = [("Descendants", many), ("Excluded", many), ("Included", many), ("Ray", many), ("List", crate::cost::PARAMS_LIST + crate::cost::PARAMS_LISTED)];
    for (name, price) in checks {
        run(&lua, &engine, &format!("assert(math.abs(shared.Lists.{name} - {price:e}) < 1e-9, `{name}: {{shared.Lists.{name}}} s more work, not {price:e}`)"));
    }
}

/// A params object's instance list costs the work clock what a client spends copying it (cost.rs PARAMS_LIST_WRITE,
/// PARAMS_WRITTEN, PARAMS_FILTER_WRITTEN, PARAMS_LIST_READ, PARAMS_READ; Studio 0.747: 13.6 ns an entry assigned to
/// FilterDescendantsInstances, 11 read): each list assigned 10,000 instances against an empty one, read back, AddToFilter of
/// 1,000, and of one onto a list of 10,000 against onto an empty one (a client appends: what the list holds costs nothing). Each
/// copy cost the proxy's host call alone, whatever its length.
#[test]
fn params_lists_cost_what_a_client_spends_copying_them() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Pool = table.create(10000)
        for I = 1, 10000 do
            Pool[I] = Instance.new("Folder")
        end
        local Thousand, Empty = table.move(Pool, 1, 1000, 1, table.create(1000)), {}
        local function Cost(Call)
            Call()
            local Start = os.clock()
            Call()
            return os.clock() - Start
        end
        local Ray, Overlap = RaycastParams.new(), OverlapParams.new()
        local function Write(Params, Field)
            return Cost(function() Params[Field] = Pool end) - Cost(function() Params[Field] = Empty end)
        end
        local function Read(Params, Field)
            Params[Field] = Pool
            local Many = Cost(function() local _ = Params[Field] end)
            Params[Field] = Empty
            return Many - Cost(function() local _ = Params[Field] end)
        end
        local function Append(List)
            Ray.FilterDescendantsInstances = List
            return Cost(function() Ray:AddToFilter(Pool[1]) end)
        end
        shared.Copies = {
            Descendants = Write(Ray, "FilterDescendantsInstances"),
            Overlap = Write(Overlap, "FilterDescendantsInstances"),
            Excluded = Write(Ray, "ExcludeInstances"),
            Included = Write(Overlap, "IncludeInstances"),
            Read = Read(Ray, "FilterDescendantsInstances"),
            ReadIncluded = Read(Overlap, "IncludeInstances"),
            Written = Cost(function() Ray.FilterDescendantsInstances = Empty end) - Cost(function() Ray.IgnoreWater = true end),
            Copied = Cost(function() local _ = Ray.FilterDescendantsInstances end) - Cost(function() local _ = Ray.IgnoreWater end),
            Added = Cost(function() Ray.FilterDescendantsInstances = Empty Ray:AddToFilter(Thousand) end)
                - Cost(function() Ray.FilterDescendantsInstances = Empty Ray:AddToFilter(Empty) end),
            Appended = Append(Pool) - Append(Empty),
        }
        "##,
    );
    let checks = [
        ("Descendants", 10_000.0 * crate::cost::PARAMS_WRITTEN),
        ("Overlap", 10_000.0 * crate::cost::PARAMS_WRITTEN),
        ("Excluded", 10_000.0 * crate::cost::PARAMS_FILTER_WRITTEN),
        ("Included", 10_000.0 * crate::cost::PARAMS_FILTER_WRITTEN),
        ("Read", 10_000.0 * crate::cost::PARAMS_READ),
        ("ReadIncluded", 10_000.0 * crate::cost::PARAMS_READ),
        ("Written", crate::cost::PARAMS_LIST_WRITE),
        ("Copied", crate::cost::PARAMS_LIST_READ),
        ("Added", 1_000.0 * crate::cost::PARAMS_WRITTEN),
        ("Appended", 0.0),
    ];
    for (name, price) in checks {
        run(&lua, &engine, &format!("assert(math.abs(shared.Copies.{name} - {price:e}) < 1e-9, `{name}: {{shared.Copies.{name}}} s more work, not {price:e}`)"));
    }
}

/// A spatial query that raises costs the work clock what a client spends on the error (cost.rs RAISED, Studio 0.747: 2.9 to
/// 4.1 us past the call's price, whatever raised): a missing or wrong argument, a value that is not finite, a list entry that is
/// no Instance, on the WorldRoot queries, GetPartsObscuringTarget and GetLargestCutoffDistance. Each of these was charged its
/// call alone.
#[test]
fn queries_that_raise_cost_what_a_client_spends_on_the_error() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Cam = Instance.new("Camera")
        Cam.CFrame = CFrame.new(0, 5000, 0)
        Cam.Focus = CFrame.new(0, 5000, -10)
        Cam.Parent = workspace
        local Part = Instance.new("Part")
        Part.Anchored, Part.Position = true, Vector3.new(200, 5000, 0)
        Part.Parent = workspace
        -- (the raising and the passing call alike but for the value that raises: an upvalue each)
        local V, Down, Missing, NaN, One = Vector3.new(50, 5000, 50), Vector3.new(0, -10, 0), nil, 0 / 0, 1
        local Frame, Size, Empty, Bad, Ray0 = CFrame.new(50, 5000, 50), Vector3.one, {}, { 5 }, Ray.new(Vector3.new(50, 5000, 50), Vector3.new(0, -10, 0))
        local function Cost(Call)
            pcall(Call)
            local Start = os.clock()
            pcall(Call)
            return os.clock() - Start
        end
        local function Extra(Raises, Passes)
            return Cost(Raises) - Cost(Passes)
        end
        shared.Raised = {
            Raycast = Extra(function() workspace:Raycast(Missing, Down) end, function() workspace:Raycast(V, Down) end),
            Spherecast = Extra(function() workspace:Spherecast(V, NaN, Down) end, function() workspace:Spherecast(V, One, Down) end),
            Blockcast = Extra(function() workspace:Blockcast(Frame, Missing, Down) end, function() workspace:Blockcast(Frame, Size, Down) end),
            Shapecast = Extra(function() workspace:Shapecast(Missing, Down) end, function() workspace:Shapecast(Part, Down) end),
            Box = Extra(function() workspace:GetPartBoundsInBox(Missing, Size) end, function() workspace:GetPartBoundsInBox(Frame, Size) end),
            Radius = Extra(function() workspace:GetPartBoundsInRadius(V, Missing) end, function() workspace:GetPartBoundsInRadius(V, One) end),
            InPart = Extra(function() workspace:GetPartsInPart(Missing) end, function() workspace:GetPartsInPart(Part) end),
            FindPartOnRay = Extra(function() workspace:FindPartOnRay(Missing) end, function() workspace:FindPartOnRay(Ray0) end),
            Obscuring = Extra(function() Cam:GetPartsObscuringTarget(Empty, Missing) end, function() Cam:GetPartsObscuringTarget(Empty, Empty) end),
            Cutoff = Extra(function() Cam:GetLargestCutoffDistance(Bad) end, function() Cam:GetLargestCutoffDistance(Empty) end),
        }
        "##,
    );
    // (a call that raises skips the work past its arguments, which the passing call pays: GetPartsInPart's by its 4 x 1 x 2 part's area, the camera
    // calls' own prices)
    let raised = crate::cost::RAISED;
    let in_part = 28.0f64.sqrt() * crate::cost::QUERY_PART_SIZE;
    let checks = [("Raycast", raised), ("Spherecast", raised), ("Blockcast", raised), ("Shapecast", raised), ("Box", raised), ("Radius", raised), ("InPart", raised - in_part), ("FindPartOnRay", raised), ("Obscuring", raised - crate::cost::OBSCURING_CALL), ("Cutoff", raised - crate::cost::CUTOFF_CALL)];
    for (name, price) in checks {
        run(&lua, &engine, &format!("assert(math.abs(shared.Raised.{name} - {price:e}) < 0.01e-6, `{name}: {{shared.Raised.{name}}} s more work when it raises, not {price:e}`)"));
    }
}

/// An engine function that raises costs the work clock what a client spends on the error, past the same call going on (cost.rs
/// RAISED, RAISED_AT_CALLER; Studio 0.747, a playing client and a server: a plain error 3.1 to 3.7 us, one at the calling line 2.0
/// to 3.0): plain, a method's, a method called with `.` and no instance, a member read and one written that are not, a circular
/// Parent, GetService of no service, and a service method in the engine's Luau (LogService:Info, which prices its own call: its
/// LOG_CALL is not charged when it raises, and the error once); at the calling line, task.cancel's, a Parent written a number,
/// typeof()'s and Path2DControlPoint.new's (the engine's Luau, its error raised again from its engine function, at the script's
/// line). Each was charged its call alone.
#[test]
fn engine_errors_cost_what_a_client_spends_on_the_error() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        local Part = Instance.new("Part")
        Part.Parent = Folder
        local LogService = game:GetService("LogService")
        local Thread = task.spawn(function() end)
        -- (the raising and the passing call alike but for the value that raises: an upvalue each)
        local Missing, Name, Unknown, Known, NoService, Service, Five, Text, Position = nil, "x", "NotAMember", "Anchored", "NotAService", "Workspace", 5, "x", UDim2.new()
        local function Cost(Call)
            pcall(Call)
            local Start = os.clock()
            pcall(Call)
            return os.clock() - Start
        end
        local function Extra(Raises, Passes)
            return Cost(Raises) - Cost(Passes)
        end
        shared.Errors = {
            Method = Extra(function() Part:FindFirstChild(Missing) end, function() Part:FindFirstChild(Name) end),
            Dot = Extra(function() Part.GetChildren(Missing) end, function() Part.GetChildren(Part) end),
            Read = Extra(function() local _ = Part[Unknown] end, function() local _ = Part[Known] end),
            Write = Extra(function() Part[Unknown] = true end, function() Part[Known] = true end),
            Parent = Extra(function() Part.Parent = Part end, function() Part.Parent = Folder end),
            Service = Extra(function() game:GetService(NoService) end, function() game:GetService(Service) end),
            Log = Extra(function() LogService:Info(Missing) end, function() LogService:Info(Text) end),
            Cancel = Extra(function() task.cancel(Five) end, function() task.cancel(Thread) end),
            ParentNumber = Extra(function() Part.Parent = Five end, function() Part.Parent = Folder end),
            Typeof = Extra(function() typeof() end, function() typeof(Part) end),
            EngineLuau = Extra(function() Path2DControlPoint.new(Five) end, function() Path2DControlPoint.new(Position) end),
        }
        "##,
    );
    let (plain, at_caller) = (crate::cost::RAISED, crate::cost::RAISED_AT_CALLER);
    let checks = [
        ("Method", plain),
        ("Dot", plain),
        ("Read", plain),
        ("Write", plain),
        ("Parent", plain),
        ("Service", plain),
        ("Log", plain - crate::cost::LOG_CALL),
        ("Cancel", at_caller),
        ("ParentNumber", at_caller),
        ("Typeof", at_caller),
        ("EngineLuau", at_caller),
    ];
    // (within what the passing call does past its arguments, which the raising one skips: 0.06 us at most)
    for (name, price) in checks {
        run(&lua, &engine, &format!("assert(math.abs(shared.Errors.{name} - {price:e}) < 0.06e-6, `{name}: {{shared.Errors.{name}}} s more work when it raises, not {price:e}`)"));
    }
}

/// A query whose geometry is not finite tests no part, as a client's spends nothing on the parts (Studio 0.747: a NaN or infinite
/// ray 0.17 us beside 0 or 5,000 parts, a NaN box 3.1, a NaN radius 1.5): rays, shape casts with a NaN direction, boxes with a
/// NaN frame, NaN radii and positions, GetPartsObscuringTarget's points and GetLargestCutoffDistance's Focus that are not finite
/// take nothing from the broadphase beside 20,000 parts, and a box infinite along one axis takes only the parts along it; they
/// answer as a client does (tools/parity/probes/query_finite.luau). Each of these took every part.
#[test]
fn queries_whose_geometry_is_not_finite_test_no_part() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Far = Instance.new("Folder")
        Far.Parent = workspace
        for I = 1, 20000 do
            local Part = Instance.new("Part")
            Part.Anchored, Part.Size, Part.Position = true, Vector3.one, Vector3.new(-3000 + (I % 100) * 3, 400 + (I // 1000) * 3, -3000 + ((I // 100) % 10) * 3)
            Part.Parent = Far
        end
        local Wall = Instance.new("Part")
        Wall.Anchored, Wall.Size, Wall.Position, Wall.Name = true, Vector3.new(10, 1, 10), Vector3.new(300, 90, 300), "Wall"
        Wall.Parent = workspace
        local Caster = Instance.new("Part")
        Caster.Anchored, Caster.Position = true, Vector3.new(300, 110, 300)
        Caster.Parent = workspace
        local Cam = Instance.new("Camera")
        Cam.CFrame = CFrame.new(300, 100, 300)
        Cam.Focus = CFrame.new(0 / 0, 0, 0)
        Cam.Parent = workspace
        workspace:Raycast(Vector3.new(5000, 5000, 5000), Vector3.new(0, 1, 0))
        local NaN, Inf, V, Down = 0 / 0, math.huge, Vector3.new(300, 100, 300), Vector3.new(0, -20, 0)
        shared.Queries = function()
            assert(workspace:Raycast(Vector3.new(NaN, 100, 300), Down) == nil, "a NaN origin")
            assert(workspace:Raycast(V, Vector3.new(0, -Inf, 0)) == nil, "an infinite direction")
            assert(workspace:Raycast(V, Vector3.new(0, -1e20, 0)) == nil, "a direction whose length overflows")
            local Part, End = workspace:FindPartOnRay(Ray.new(V, Vector3.new(0, -Inf, 0)))
            assert(Part == nil and End.Y == -Inf, "FindPartOnRay's end keeps the infinity")
            assert(workspace:Spherecast(V, 1, Vector3.new(0, NaN, 0)) == nil, "a Spherecast with a NaN direction")
            assert(workspace:Blockcast(CFrame.new(V), Vector3.one, Vector3.new(NaN, -5, 0)) == nil, "a Blockcast with a NaN direction")
            assert(workspace:Shapecast(Caster, Vector3.new(0, NaN, 0)) == nil, "a Shapecast with a NaN direction")
            assert(#workspace:GetPartBoundsInBox(CFrame.new(NaN, 90, 300), Vector3.one) == 0, "a NaN frame")
            assert(#workspace:GetPartBoundsInBox(CFrame.new(300, 90, 300, NaN, 0, 0, 0, 1, 0, 0, 0, 1), Vector3.one) == 0, "a NaN rotation")
            local Along = workspace:GetPartBoundsInBox(CFrame.new(320, 90, 300), Vector3.new(Inf, 1, 1))
            assert(#Along == 1 and Along[1] == Wall, "a box infinite along x finds the part on its line")
            assert(#workspace:GetPartBoundsInBox(CFrame.new(300, 90, 300), Vector3.new(NaN, 1, 1)) == 1, "a NaN side is no width")
            assert(#workspace:GetPartBoundsInRadius(V, NaN) == 0, "a NaN radius")
            assert(#workspace:GetPartBoundsInRadius(Vector3.new(NaN, 0, 0), 5) == 0, "a NaN position")
            local Listed = Cam:GetPartsObscuringTarget({ Vector3.new(300, -Inf, 300), Vector3.new(300, 80, 300), Vector3.new(NaN, 0, 0) }, {})
            assert(#Listed == 1 and Listed[1] == Wall, "the finite point lists the wall, the others nothing")
            assert(Cam:GetLargestCutoffDistance({}) == 0, "a NaN Focus")
        end
        shared.Queries()
        -- (each call once before it is timed: a frame's first call of an engine function is priced cold)
        local function Cost(Call)
            Call()
            local Start = os.clock()
            Call()
            return os.clock() - Start
        end
        local Empty, One, Nowhere = CFrame.new(0, -500, 0), Vector3.one, Vector3.new(0, -500, 0)
        local NaNFrame, NaNSide, NaNWhere = CFrame.new(NaN, -500, 0), Vector3.new(NaN, 1, 1), Vector3.new(NaN, -500, 0)
        shared.Extra = {
            Frame = Cost(function() workspace:GetPartBoundsInBox(NaNFrame, One) end) - Cost(function() workspace:GetPartBoundsInBox(Empty, One) end),
            Side = Cost(function() workspace:GetPartBoundsInBox(Empty, NaNSide) end) - Cost(function() workspace:GetPartBoundsInBox(Empty, One) end),
            Radius = Cost(function() workspace:GetPartBoundsInRadius(Nowhere, NaN) end) - Cost(function() workspace:GetPartBoundsInRadius(Nowhere, 1) end),
            Position = Cost(function() workspace:GetPartBoundsInRadius(NaNWhere, 1) end) - Cost(function() workspace:GetPartBoundsInRadius(Nowhere, 1) end),
        }
        "##,
    );
    let taken = || super::broadphase::TAKEN.with(|taken| taken.get());
    let (before, start) = (taken(), std::time::Instant::now());
    run(&lua, &engine, "for _ = 1, 20 do shared.Queries() end");
    let (took, host) = (taken() - before, start.elapsed().as_secs_f64());
    // (the finite point's segment and the box along x take the parts near them: a few)
    assert!(took <= 20 * 4, "20 rounds of the queries took {took} parts from the broadphase beside 20,000");
    assert!(host < 0.25, "20 rounds took {host:.3} s of host time");
    // what a client spends on such geometry beyond the call (cost.rs QUERY_NOT_FINITE, QUERY_SIDE_NOT_FINITE)
    for (name, price) in [("Frame", crate::cost::QUERY_NOT_FINITE), ("Side", crate::cost::QUERY_SIDE_NOT_FINITE), ("Radius", crate::cost::QUERY_SIDE_NOT_FINITE), ("Position", crate::cost::QUERY_NOT_FINITE)] {
        run(&lua, &engine, &format!("assert(math.abs(shared.Extra.{name} - {price:e}) < 1e-9, `{name}: {{shared.Extra.{name}}} s more work, not {price:e}`)"));
    }
}

/// The broadphase follows the dom without walking the Workspace: a part moved, resized and turned off (CanQuery), one moved out and
/// back, a model moved in whole, a part destroyed and collected, an instance given a part's class where it stands (a bundle's root
/// revealed: dom.rs set_class) are each seen by the next query; more changes before a query than the log keeps (dom.rs
/// Watch::limit) make that query walk the Workspace once, and its answers stay right.
#[test]
fn the_broadphase_follows_the_dom_and_starts_over_past_its_log() {
    let (lua, engine) = start_play();
    let walks = || super::broadphase::WORKSPACE_WALKS.with(|walks| walks.get());
    run(
        &lua,
        &engine,
        r##"
        local Part = Instance.new("Part")
        Part.Anchored = true
        Part.Size = Vector3.new(4, 4, 4)
        Part.CFrame = CFrame.new(0, 0, 0)
        Part.Parent = workspace
        local Down = Vector3.new(0, -20, 0)
        assert(workspace:Raycast(Vector3.new(0, 10, 0), Down).Instance == Part)
        Part.CFrame = CFrame.new(30, 0, 0)
        assert(workspace:Raycast(Vector3.new(0, 10, 0), Down) == nil and workspace:Raycast(Vector3.new(30, 10, 0), Down).Instance == Part, "moved")
        Part.Size = Vector3.new(4, 40, 4)
        assert(workspace:Raycast(Vector3.new(30, 30, 0), Vector3.new(0, 20, 0)) == nil, "resized")
        assert(workspace:Raycast(Vector3.new(30, 50, 0), Down * 2).Distance == 30, "resized, hit at its new top")
        Part.CanQuery = false
        assert(workspace:Raycast(Vector3.new(30, 50, 0), Down * 2) == nil, "CanQuery off")
        Part.CanQuery = true
        Part.Parent = nil
        assert(workspace:Raycast(Vector3.new(30, 50, 0), Down * 2) == nil, "out")
        Part.Parent = workspace
        assert(workspace:Raycast(Vector3.new(30, 50, 0), Down * 2).Instance == Part, "back")
        local Model = Instance.new("Model")
        for I = 1, 5 do
            local Piece = Instance.new("Part")
            Piece.Anchored = true
            Piece.CFrame = CFrame.new(-30, I * 3, 0)
            Piece.Parent = Model
        end
        Model.Parent = workspace
        assert(#workspace:GetPartBoundsInBox(CFrame.new(-30, 8, 0), Vector3.new(2, 20, 2)) == 5, "a model moved in")
        Part:Destroy()
        "##,
    );
    assert_eq!(walks(), 1, "the first query made the index; the changes came from the log");
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
    engine_lua::raw::collect(&lua, &engine).unwrap();
    run(&lua, &engine, r##"assert(workspace:Raycast(Vector3.new(30, 50, 0), Vector3.new(0, -40, 0)) == nil, "destroyed")"##);
    // a Folder in the Workspace becomes a Part where it stands
    let folder = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("Folder").expect("Folder");
        let folder = crate::instance::create_named(&mut state, &engine.reflection, class, "Revealed");
        let workspace = engine.world().workspace;
        state.dom.attach(folder, workspace);
        folder
    };
    run(&lua, &engine, r##"assert(workspace:Raycast(Vector3.new(0, 10, 0), Vector3.new(0, -20, 0)) == nil)"##);
    {
        let mut state = engine.state.borrow_mut();
        let part = engine.reflection.class_id("Part").expect("Part");
        state.dom.set_class(&engine.reflection, folder, part);
    }
    run(&lua, &engine, r##"assert(workspace:Raycast(Vector3.new(0, 10, 0), Vector3.new(0, -20, 0)).Instance.Name == "Revealed", "revealed")"##);
    assert_eq!(walks(), 1, "no walk for the reveal");
    // past the log: more distinct instances changed before a query than it keeps, 3,000 parts moved out of the Workspace and
    // back and 4,000 new folders put in it (7,000 entries, past twice the 3,006 parts held)
    run(
        &lua,
        &engine,
        r##"
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        local Many = {}
        for I = 1, 3000 do
            local Piece = Instance.new("Part")
            Piece.Anchored = true
            Piece.CFrame = CFrame.new(100 + (I % 50) * 2, 0, (I // 50) * 2)
            Piece.Parent = Folder
            Many[I] = Piece
        end
        assert(#workspace:GetPartBoundsInBox(CFrame.new(150, 0, 60), Vector3.new(200, 2, 200)) == 3000)
        for _, Piece in Many do
            Piece.Parent = nil
            Piece.Parent = Folder
        end
        for _ = 1, 4000 do
            Instance.new("Folder").Parent = Folder
        end
        Many[1].Parent = nil
        assert(#workspace:GetPartBoundsInBox(CFrame.new(150, 0, 60), Vector3.new(200, 2, 200)) == 2999, "after the overflow")
        "##,
    );
    assert_eq!(walks(), 2, "the overflowed log made one walk");
    let (_, sound, parts) = super::query::index_state(&engine);
    assert!(sound && parts == 1 + 5 + 2999, "{parts} parts");
}

/// Changes to instances the log holds already add nothing to it (dom.rs Watch::logged, flags::LOGGED): a part moved back and forth
/// 10,000 times between two queries, one written 100,000 times, and 3,000 parts moved out of the Workspace and back twice log each
/// instance once, so the next query takes them from the log without walking the Workspace (the moves made 20,000 and 12,000
/// entries, past the log's 6,000, and the query walked it), and its answers are right.
#[test]
fn changes_to_the_same_instances_never_overflow_the_broadphase_log() {
    let (lua, engine) = start_play();
    let walks = || super::broadphase::WORKSPACE_WALKS.with(|walks| walks.get());
    run(
        &lua,
        &engine,
        r##"
        local A, B = Instance.new("Folder"), Instance.new("Folder")
        A.Parent, B.Parent = workspace, workspace
        local Many = {}
        for I = 1, 3000 do
            local Piece = Instance.new("Part")
            Piece.Anchored = true
            Piece.CFrame = CFrame.new(100 + (I % 50) * 2, 0, (I // 50) * 2)
            Piece.Parent = A
            Many[I] = Piece
        end
        local Lone = Instance.new("Part")
        Lone.Anchored = true
        Lone.Size = Vector3.new(4, 4, 4)
        Lone.Parent = A
        assert(#workspace:GetPartBoundsInBox(CFrame.new(150, 0, 60), Vector3.new(200, 2, 200)) == 3000)
        shared.A, shared.B, shared.Many, shared.Lone = A, B, Many, Lone
        "##,
    );
    super::broadphase::WORKSPACE_WALKS.with(|walks| walks.set(0));
    run(
        &lua,
        &engine,
        r##"
        local A, B, Lone = shared.A, shared.B, shared.Lone
        for I = 1, 10000 do
            Lone.Parent = if I % 2 == 1 then B else A
        end
        Lone.CFrame = CFrame.new(0, -50, 0)
        local Hit = workspace:Raycast(Vector3.new(0, -40, 0), Vector3.new(0, -20, 0))
        assert(Hit and Hit.Instance == Lone and Lone.Parent == A, "moved 10,000 times")
        "##,
    );
    assert_eq!(walks(), 0, "a part moved 10,000 times came from the log");
    run(
        &lua,
        &engine,
        r##"
        local Lone = shared.Lone
        for I = 1, 100000 do
            Lone.CFrame = CFrame.new(I % 7, -50, 0)
        end
        local Hit = workspace:Raycast(Vector3.new(100000 % 7, -40, 0), Vector3.new(0, -20, 0))
        assert(Hit and Hit.Instance == Lone, "written 100,000 times")
        "##,
    );
    assert_eq!(walks(), 0, "a part written 100,000 times came from the log");
    run(
        &lua,
        &engine,
        r##"
        for _ = 1, 2 do
            for _, Piece in shared.Many do
                Piece.Parent = nil
                Piece.Parent = shared.B
            end
        end
        shared.Many[1].Parent = nil
        assert(#workspace:GetPartBoundsInBox(CFrame.new(150, 0, 60), Vector3.new(200, 2, 200)) == 2999, "moved out and back twice")
        "##,
    );
    assert_eq!(walks(), 0, "3,000 parts moved out and back twice came from the log");
    super::query::index_matches_a_walk(&engine).unwrap();
}

/// The log keeps as many instances as the last walk of the Workspace visited (broadphase.rs Index::limit), so a walk an overflow
/// forces follows as many changed instances as it visits: beside 20,000 folders and 10 parts, 10,000 folders moved between two
/// queries come from the log (they overflowed a log of 4,096 entries and the query walked the Workspace), and only more instances
/// than the walk visited start the index over.
#[test]
fn the_broadphase_log_keeps_as_many_instances_as_its_walk_visits() {
    let (lua, engine) = start_play();
    let walks = || super::broadphase::WORKSPACE_WALKS.with(|walks| walks.get());
    super::broadphase::WORKSPACE_WALKS.with(|walks| walks.set(0));
    run(
        &lua,
        &engine,
        r##"
        local A, B = Instance.new("Folder"), Instance.new("Folder")
        A.Parent, B.Parent = workspace, workspace
        local Folders = {}
        for I = 1, 20000 do
            Folders[I] = Instance.new("Folder")
            Folders[I].Parent = A
        end
        for I = 1, 10 do
            local Piece = Instance.new("Part")
            Piece.Anchored = true
            Piece.CFrame = CFrame.new(I * 4, 0, 0)
            Piece.Parent = A
        end
        assert(#workspace:GetPartBoundsInBox(CFrame.new(22, 0, 0), Vector3.new(44, 2, 2)) == 10)
        shared.A, shared.B, shared.Folders = A, B, Folders
        "##,
    );
    assert_eq!(walks(), 1, "the first query made the index");
    run(
        &lua,
        &engine,
        r##"
        for I = 1, 10000 do
            shared.Folders[I].Parent = shared.B
        end
        assert(#workspace:GetPartBoundsInBox(CFrame.new(22, 0, 0), Vector3.new(44, 2, 2)) == 10)
        "##,
    );
    assert_eq!(walks(), 1, "10,000 folders moved came from the log");
    run(
        &lua,
        &engine,
        r##"
        local C = Instance.new("Folder")
        C.Parent = workspace
        for _, Folder in shared.Folders do
            Folder.Parent = C
        end
        for _ = 1, 1000 do
            Instance.new("Folder").Parent = C
        end
        assert(#workspace:GetPartBoundsInBox(CFrame.new(22, 0, 0), Vector3.new(44, 2, 2)) == 10)
        "##,
    );
    assert_eq!(walks(), 2, "21,000 instances changed, more than the walk visited: one walk");
    super::query::index_matches_a_walk(&engine).unwrap();
}

/// A chain of parts, each parented under the one before, that moves costs the broadphase's next query a look at each part once:
/// 2,000 parts moved within the Workspace, parented out of it, and built inside it part by part between two queries each take
/// a few thousand looks at instances (broadphase::SYNC_VISITS), where each part's walk went down the rest of the chain again
/// (2 million: 0.45 s of host time at 4,000 parts), and the index and the answers stay right.
#[test]
fn a_chain_of_parts_that_moves_costs_the_broadphase_its_length() {
    let (lua, engine) = start_play();
    let visits = || super::broadphase::SYNC_VISITS.with(|visits| visits.replace(0));
    run(
        &lua,
        &engine,
        r##"
        local Holder, Other = Instance.new("Folder"), Instance.new("Folder")
        Holder.Parent, Other.Parent = workspace, workspace
        function shared.Chain(Count)
            local First, At = nil, Holder
            for I = 1, Count do
                local Piece = Instance.new("Part")
                Piece.Anchored = true
                Piece.Size = Vector3.new(1, 1, 1)
                Piece.CFrame = CFrame.new(I * 2, 0, 0)
                Piece.Parent = At
                First = First or Piece
                At = Piece
            end
            return First
        end
        function shared.Count()
            return #workspace:GetPartBoundsInBox(CFrame.new(2001, 0, 0), Vector3.new(4002, 2, 2))
        end
        shared.First, shared.Other = shared.Chain(2000), Other
        assert(shared.Count() == 2000)
        "##,
    );
    visits();
    run(&lua, &engine, r##"shared.First.Parent = shared.Other assert(shared.Count() == 2000, "moved within the Workspace")"##);
    let moved = visits();
    super::query::index_matches_a_walk(&engine).unwrap();
    run(&lua, &engine, r##"shared.First.Parent = nil assert(shared.Count() == 0, "parented out")"##);
    let out = visits();
    super::query::index_matches_a_walk(&engine).unwrap();
    run(&lua, &engine, r##"shared.Chain(2000) assert(shared.Count() == 2000, "built inside")"##);
    let built = visits();
    super::query::index_matches_a_walk(&engine).unwrap();
    for (what, visits) in [("moved within the Workspace", moved), ("parented out", out), ("built inside", built)] {
        assert!(visits <= 2 * 2000 + 10, "a chain of 2,000 parts {what}: the next query looked at {visits} instances");
    }
}

/// A part whose bounds are not finite (a NaN or infinite position) is in the list every query takes (broadphase.rs Index::loose)
/// and leaves it in a step, by the place the list keeps for it: the queries after 60,000 such parts were each written (twice),
/// moved out of the Workspace and back, and destroyed cost about what they cost after the same changes to 60,000 parts in the
/// tree, where each part came out by a search of the list (the query after 100,000 such parts were written took 0.98 s of host
/// time, after they were destroyed about 1 s), and the index and the list stay sound.
#[test]
fn parts_whose_bounds_are_not_finite_leave_the_index_in_a_step() {
    let timed = |frame: &str| {
        let (lua, engine) = start_play();
        run(
            &lua,
            &engine,
            &r##"
            local NaN, Inf = 0 / 0, math.huge
            local Folder = Instance.new("Folder")
            Folder.Parent = workspace
            local Parts = table.create(60000)
            for I = 1, 60000 do
                local Part = Instance.new("Part")
                Part.Anchored = true
                Part.CFrame = __FRAME__
                Part.Parent = Folder
                Parts[I] = Part
            end
            function shared.Query()
                return #workspace:GetPartBoundsInBox(CFrame.new(0, -500, 0), Vector3.one)
            end
            assert(shared.Query() == 0)
            shared.Parts, shared.Folder = Parts, Folder
            "##
            .replace("__FRAME__", frame),
        );
        // (the host time of each query that takes a change: the change's own work in the dom apart)
        let mut host = 0.0;
        for change in ["for _, Part in shared.Parts do Part.Transparency = 0.25 end", "for _, Part in shared.Parts do Part.Transparency = 0.5 end", "shared.Folder.Parent = nil", "shared.Folder.Parent = workspace", "shared.Folder:Destroy()"] {
            run(&lua, &engine, change);
            let start = std::time::Instant::now();
            run(&lua, &engine, "assert(shared.Query() == 0)");
            host += start.elapsed().as_secs_f64();
            let (_, sound, parts) = super::query::index_state(&engine);
            let held = if change.contains("Destroy") || change.contains("nil") { 0 } else { 60_000 };
            assert!(sound && parts == held, "{frame}, {change}: {parts} parts held, sound {sound}");
        }
        super::query::index_matches_a_walk(&engine).unwrap();
        host
    };
    let tree = timed("CFrame.new((I % 200) * 3, (I // 200) * 3, 0)");
    let loose = timed("if I % 2 == 0 then CFrame.new(NaN, 0, 0) else CFrame.new(Inf, 0, 0)");
    eprintln!("the queries took {loose:.3} s of host time for the parts whose bounds are not finite, {tree:.3} s in the tree");
    assert!(loose < 3.0 * tree + 0.1, "the queries after 60,000 parts whose bounds are not finite were written twice, moved out and back and destroyed took {loose:.3} s of host time, after as many in the tree {tree:.3} s");
}

/// The sets the spatial index fills and empties once a round (dom.rs Watch::logged, the broadphase's walk marks, camera.rs
/// Cameras::unindexed) keep no table a burst grew them to (dom.rs clear_set): after 500,000 folders moved between two queries
/// (from the log, no walk of the Workspace) each later round of one part moved and one query costs about what it cost before
/// the burst, where it cleared two tables of a million slots (32 us of host time a round after a million folders moved, 2 us
/// before them), and after 50,000 parts entered the Workspace at once and a query took them in, the waiting parts' table is
/// no larger than what waits.
#[test]
fn a_burst_of_changes_leaves_the_spatial_index_no_work_after_it() {
    let (lua, engine) = start_play();
    let walks = || super::broadphase::WORKSPACE_WALKS.with(|walks| walks.get());
    run(
        &lua,
        &engine,
        r##"
        local Holders = table.create(500)
        for H = 1, 500 do
            Holders[H] = Instance.new("Folder")
            Holders[H].Parent = workspace
        end
        local Folders = table.create(500000)
        for I = 1, 500000 do
            local Folder = Instance.new("Folder")
            Folder.Parent = Holders[I % 500 + 1]
            Folders[I] = Folder
        end
        local A, B = Instance.new("Folder"), Instance.new("Folder")
        A.Parent, B.Parent = workspace, workspace
        local One = Instance.new("Part")
        One.Anchored = true
        One.Parent = A
        function shared.Rounds(Count)
            for I = 1, Count do
                One.Parent = if I % 2 == 1 then B else A
                assert(#workspace:GetPartBoundsInBox(CFrame.new(0, -500, 0), Vector3.one) == 0)
            end
        end
        function shared.Burst()
            for I, Folder in Folders do
                Folder.Parent = Holders[(I + 1) % 500 + 1]
            end
            assert(#workspace:GetPartBoundsInBox(CFrame.new(0, -500, 0), Vector3.one) == 0)
        end
        shared.Rounds(10)
        "##,
    );
    let time = |source: &str| {
        let start = std::time::Instant::now();
        run(&lua, &engine, source);
        start.elapsed().as_secs_f64()
    };
    let before = time("shared.Rounds(30000)");
    let walked = walks();
    run(&lua, &engine, "shared.Burst() shared.Rounds(10)");
    assert_eq!(walks(), walked, "the burst came from the log");
    let after = time("shared.Rounds(30000)");
    eprintln!("30,000 rounds took {before:.3} s of host time before the burst, {after:.3} s after it");
    assert!(after < 2.0 * before + 0.1, "30,000 rounds of one move and one query took {after:.3} s of host time after 500,000 folders moved, {before:.3} s before");
    run(
        &lua,
        &engine,
        r##"
        local Store = Instance.new("Folder")
        for I = 1, 50000 do
            local Part = Instance.new("Part")
            Part.Anchored = true
            Part.CFrame = CFrame.new((I % 100) * 3, -1000, (I // 100) * 3)
            Part.Parent = Store
        end
        Store.Parent = workspace
        assert(#workspace:GetPartBoundsInBox(CFrame.new(0, -500, 0), Vector3.one) == 0)
        shared.Rounds(4)
        "##,
    );
    let capacity = engine.state.borrow().cameras.unindexed.capacity();
    assert!(capacity < 4096, "after 50,000 parts waited and were taken in, the waiting parts' table holds {capacity}");
    super::query::index_matches_a_walk(&engine).unwrap();
}

/// The broadphase holds what a walk of the Workspace finds after random rounds of parts moved below one another (chains and
/// trees of parts), out of the Workspace and back, destroyed, written and made, several changes to the same instances in a round,
/// before each round's query: its walks of what moved, each instance looked at once, take what each part's own walk took.
#[test]
fn the_broadphase_follows_parts_moved_below_one_another() {
    for seed in [3, 77, 4242] {
        let (lua, engine) = start_play();
        run(
            &lua,
            &engine,
            &r##"
            local Rng = Random.new(__SEED__)
            local Store = Instance.new("Folder")
            Store.Parent = game:GetService("ReplicatedStorage")
            local Holders = { workspace, Store }
            for I = 1, 3 do
                local Holder = Instance.new(if I == 3 then "Model" else "Folder")
                Holder.Parent = workspace
                table.insert(Holders, Holder)
            end
            local Parts = {}
            local function NewPart()
                local Piece = Instance.new("Part")
                Piece.Anchored = true
                Piece.Size = Vector3.new(1, 1, 1)
                Piece.CFrame = CFrame.new(Rng:NextInteger(-50, 50), Rng:NextInteger(-50, 50), 0)
                Piece.Parent = if #Parts > 0 and Rng:NextNumber() < 0.6 then Parts[Rng:NextInteger(1, #Parts)] else Holders[Rng:NextInteger(1, #Holders)]
                table.insert(Parts, Piece)
            end
            for _ = 1, 200 do
                NewPart()
            end
            function shared.Round()
                for _ = 1, 60 do
                    local Piece = Parts[Rng:NextInteger(1, #Parts)]
                    local Roll = Rng:NextNumber()
                    pcall(function()
                        if Roll < 0.45 then
                            Piece.Parent = Parts[Rng:NextInteger(1, #Parts)]
                        elseif Roll < 0.6 then
                            Piece.Parent = Holders[Rng:NextInteger(1, #Holders)]
                        elseif Roll < 0.7 then
                            Piece.Parent = nil
                        elseif Roll < 0.85 then
                            Piece.CFrame = CFrame.new(Rng:NextInteger(-50, 50), Rng:NextInteger(-50, 50), 0)
                        elseif Roll < 0.9 then
                            Piece:Destroy()
                        else
                            NewPart()
                        end
                    end)
                end
                return #workspace:GetPartBoundsInBox(CFrame.new(), Vector3.new(120, 120, 4))
            end
            "##
            .replace("__SEED__", &seed.to_string()),
        );
        for round in 0..40 {
            run(&lua, &engine, "shared.Round()");
            super::query::index_matches_a_walk(&engine).unwrap_or_else(|error| panic!("seed {seed}, round {round}: {error}"));
        }
        let (_, sound, _) = super::query::index_state(&engine);
        assert!(sound, "seed {seed}: the tree's links and boxes hold");
    }
}

