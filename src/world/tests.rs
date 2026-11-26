//! Script-level checks of the 3D world: part and attachment CFrame views, Workspace.Terrain, the
//! spatial queries and the calls reported as world events.

use std::{cell::RefCell, rc::Rc};

use mlua::prelude::*;

use super::WorldEvent;
use crate::{
    Engine, Options,
    lua::{self as engine_lua, globals::CallOutcome},
};

#[inline(never)]
pub(super) fn start() -> (Lua, Rc<Engine>) {
    let engine = Engine::new(Options::default()).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    (lua, engine)
}

/// The frames [`run`] steps a waiting script through before it fails the test: a minute of 60 frames a second
const RUN_FRAMES: u32 = 3600;

/// Compiles `source` once into a reusable Luau function in a fresh ModuleScript environment,
/// avoiding per-round DOM creation and Luau parsing when called repeatedly through [`call`].
#[inline(never)]
fn compile(lua: &Lua, engine: &Rc<Engine>, source: &str) -> LuaFunction {
    let script = {
        let mut state = engine.state.borrow_mut();
        let class = engine.reflection.class_id("ModuleScript").unwrap();
        crate::instance::create_named(&mut state, &engine.reflection, class, "Test")
    };
    let env = engine_lua::modules::environment(lua, engine, script).expect("environment");
    lua.load(source).set_name("=Test").set_environment(env).into_function().expect("compile")
}

/// Runs a compiled function to its end through the scheduler, stepping frames while it waits.
fn call(lua: &Lua, engine: &Rc<Engine>, function: &LuaFunction, args: impl IntoLuaMulti) {
    call_multi(lua, engine, function, args.into_lua_multi(lua).expect("args"));
}

#[inline(never)]
fn call_multi(lua: &Lua, engine: &Rc<Engine>, function: &LuaFunction, args: LuaMultiValue) {
    let running = engine_lua::globals::call(lua, engine, function.clone(), args).expect("call");
    let mut frames = 0;
    loop {
        match running.outcome().unwrap() {
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

/// Exposes `shared.Wrap(Inner, Depth, Kind?)` backed by direct DOM attachment, avoiding tens of
/// thousands of Luau `Instance.new` wrapper allocations when building deep-chain test fixtures.
#[inline(never)]
fn register_wrap(lua: &Lua, engine: &Rc<Engine>) {
    let eng = engine.clone();
    let wrap = lua
        .create_function(move |lua, (inner, depth, kind): (LuaValue, usize, Option<String>)| {
            let mut outer = engine_lua::instance_id(&inner).expect("an instance");
            let class_name = kind.as_deref().unwrap_or("Folder");
            let class_id = eng.reflection.class_id(class_name).unwrap();
            {
                let mut state = eng.state.borrow_mut();
                for _ in 0..depth {
                    let parent = crate::instance::create_named(&mut state, &eng.reflection, class_id, class_name);
                    state.dom.attach(outer, parent);
                    outer = parent;
                }
            }
            engine_lua::instance_value(lua, &eng, outer)
        })
        .expect("wrap");
    let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared globals").get("shared").expect("shared");
    shared.set("Wrap", wrap).expect("shared.Wrap");
}

/// Runs `source` as a script to its end: while it waits (task.wait, an event's Wait) the engine steps frames, one at a time,
/// until it finishes, and the test fails when it is still waiting after [`RUN_FRAMES`] (what follows a wait would otherwise
/// never run, its asserts passing unchecked). Panics with the script's error.
#[inline(never)]
pub(super) fn run(lua: &Lua, engine: &Rc<Engine>, source: &str) {
    let function = compile(lua, engine, source);
    call(lua, engine, &function, ());
}

#[inline(never)]
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
        -- another parented, then CanQuery turned off: still hit while CanCollide is on (measured), not once both are off
        local Other = Instance.new("Part")
        Other.Size = Vector3.new(4, 4, 4)
        Other.CFrame = CFrame.new(0, 2, 0)
        Other.Parent = workspace
        assert(down(0).Instance == Other)
        Other.CanQuery = false
        assert(down(0).Instance == Other)
        Other.CanCollide = false
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
        -- (an entry that is no Instance raises, after the entries before it were added: measured)
        assert(not pcall(function() Params:AddToFilter({Bottom, 5}) end) and #Params.FilterDescendantsInstances == 3, "a table with an entry that is no Instance adds what came before it")
        Params.FilterDescendantsInstances = {Top, Middle}
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

/// A spatial query casts its arguments as a playing client does (Studio 0.747, a client and a server alike:
/// tools/parity/probes/query_args.luau): from the last to the first for the casts and the FindPartOnRay family, from the first to
/// the last for the overlap queries, "Argument <n> missing or nil" for a required one left out or nil, "Unable to cast <type> to
/// <Type>" for a value of the wrong type (a table or another params type given as params too), and the method's own checks after.
#[test]
fn query_arguments_are_cast_as_a_client_casts_them() {
    check(
        r#"
        local function Fails(Message, Method, ...)
            local Ok, E = pcall(workspace[Method], workspace, ...)
            if Ok or tostring(E) ~= Message then
                error(`{Method} {Message}: got {Ok} {E}`, 2)
            end
        end
        local function Passes(Method, ...)
            local Ok, E = pcall(workspace[Method], workspace, ...)
            if not Ok then
                error(`{Method}: {E}`, 2)
            end
        end
        local Part = Instance.new("Part")
        Part.Anchored, Part.Size, Part.Position = true, Vector3.new(10, 1, 10), Vector3.new(300, 90, 300)
        Part.Parent = workspace
        local V, D, CF, S = Vector3.new(300, 100, 300), Vector3.new(0, -20, 0), CFrame.new(300, 100, 300), Vector3.one
        local R = Ray.new(V, D)
        Fails("Argument 2 missing or nil", "Raycast")
        Fails("Argument 2 missing or nil", "Raycast", "x")
        Fails("Argument 1 missing or nil", "Raycast", nil, V)
        Fails("Unable to cast string to Vector3", "Raycast", "x", V)
        Fails("Unable to cast double to RaycastParams", "Raycast", 5, "x", 5)
        Fails("Unable to cast Dictionary to RaycastParams", "Raycast", V, D, { IgnoreWater = true })
        Fails("Unable to cast OverlapParams to RaycastParams", "Raycast", V, D, OverlapParams.new())
        Fails("Unable to cast CoordinateFrame to Vector3", "Raycast", CF, V)
        Fails("Argument 3 missing or nil", "Blockcast", CF, S)
        Fails("Unable to cast Vector3 to CoordinateFrame", "Blockcast", V, S, D)
        Fails("Unable to cast double to RaycastParams", "Blockcast", CF, Vector3.new(600, 1, 1), D, 5)
        Fails("Unable to cast string to float", "Spherecast", V, "x", D)
        Passes("Spherecast", V, "2", D)
        Passes("Spherecast", V, true, D)
        Fails("Unable to cast double to RaycastParams", "Spherecast", V, -1, D, 5)
        Fails("Argument 2 missing or nil", "Shapecast", Part)
        Fails("Shapecast argument #1 expects a BasePart", "Shapecast", nil, D)
        Fails("Shapecast argument #1 expects a BasePart", "Shapecast", workspace, D)
        Fails("Unable to cast value to Object", "Shapecast", "x", D)
        Fails("Unable to cast string to Vector3", "Shapecast", "x", "x", "x", "x", "x")
        Fails("Unable to cast Vector3 to CoordinateFrame", "Shapecast", Part, D, nil, V)
        Fails("Argument 1 missing or nil", "GetPartBoundsInBox")
        Fails("Unable to cast string to CoordinateFrame", "GetPartBoundsInBox", "x", "x", "x")
        Fails("Unable to cast RaycastParams to OverlapParams", "GetPartBoundsInBox", CF, S, RaycastParams.new())
        Fails("Unable to cast string to Vector3", "GetPartBoundsInRadius", "x", "x", "x")
        Fails("Argument 1 missing or nil", "GetPartsInPart")
        Fails("Part parameter must be BasePart", "GetPartsInPart", nil, 5)
        Fails("Unable to cast value to Object", "GetPartsInPart", "x", "x")
        Fails("Unable to cast Array to OverlapParams", "GetPartsInPart", Part, {})
        Fails("Unable to cast string to Ray", "FindPartOnRay", "x")
        Fails("Unable to cast value to Object", "FindPartOnRay", nil, 5)
        Fails("Unable to cast string to bool", "FindPartOnRay", R, nil, "1")
        Fails("Unable to cast Array to bool", "FindPartOnRay", R, nil, false, {})
        Passes("FindPartOnRay", R, nil, "true", 0)
        Fails("Argument 2 missing or nil", "FindPartOnRayWithIgnoreList", R)
        Fails("Expected table argument, got userdata", "FindPartOnRayWithIgnoreList", R, Part)
        Fails("Unable to cast value to Object", "FindPartOnRayWithIgnoreList", R, { { Part } })
        assert(workspace:FindPartOnRayWithIgnoreList(R, { nil, Part }) == nil, "a hole is skipped")
        assert(workspace:FindPartOnRay(R) == Part, "FindPartOnRay with only its ray")
        "#,
    );
}

/// RaycastParams and OverlapParams members cast what a script writes as a playing client does (Studio 0.747, a client and a
/// server alike: tools/parity/probes/query_params.luau): a list takes a table only, nested tables read in their place and nil
/// holes skipped, and keeps what it read before an error; the other members' casts and checks; new() ignores a table.
#[test]
fn params_members_cast_as_a_client_does() {
    check(
        r#"
        -- (the casts' argument errors carry the script's position: the message's end is compared)
        local function Fails(Message, F)
            local Ok, E = pcall(F)
            if Ok or string.sub(tostring(E), -#Message) ~= Message then
                error(`{Message}: got {Ok} {E}`, 2)
            end
        end
        local A, B, C = Instance.new("Folder"), Instance.new("Folder"), Instance.new("Folder")
        local R, O = RaycastParams.new(), OverlapParams.new()
        R.FilterDescendantsInstances = { A, { B, { C } } }
        assert(#R.FilterDescendantsInstances == 3, "nested tables")
        R.FilterDescendantsInstances = { A, nil, B }
        assert(#R.FilterDescendantsInstances == 2, "a hole")
        local List = "Invalid value type; table must only contain Instances and other tables"
        Fails(List, function() R.FilterDescendantsInstances = { A, 1, B } end)
        assert(#R.FilterDescendantsInstances == 1, "what came before the error stays")
        Fails("Invalid value type; expected a table", function() R.FilterDescendantsInstances = A end)
        assert(#R.FilterDescendantsInstances == 0, "no table leaves an empty list")
        Fails("Invalid value type; expected a table", function() R.FilterDescendantsInstances = nil end)
        R.ExcludeInstances = nil
        Fails("Invalid value type; expected a table", function() O.IncludeInstances = 5 end)
        assert(type(O.IncludeInstances) == "table" and #O.IncludeInstances == 0, "an empty Include list")
        local Cycle = { A }
        Cycle[2] = Cycle
        Fails("Tables cannot be cyclic", function() R.ExcludeInstances = Cycle end)
        local Deep = { A }
        for _ = 2, 298 do
            Deep = { Deep }
        end
        Fails("Tables are too deeply nested", function() R.FilterDescendantsInstances = Deep end)
        R:AddToFilter(Deep)
        assert(#R.FilterDescendantsInstances == 1, "AddToFilter reads one level deeper")
        Fails("Expected a table or Instance", function() R:AddToFilter(5) end)
        Fails("invalid argument #1 to 'AddToFilter' (RaycastParams expected, got OverlapParams)", function() R.AddToFilter(O, A) end)
        R.CollisionGroup = 5.5
        assert(R.CollisionGroup == "5.5", "a number group is its text")
        Fails("invalid argument #3 (string expected, got boolean)", function() R.CollisionGroup = true end)
        R.IgnoreWater = 0
        assert(R.IgnoreWater == true, "a value's truth")
        Fails("Invalid value for enum 'RaycastFilterType'", function() R.FilterType = "Include" end)
        Fails("Invalid type 'KeyCode' for enum 'RaycastFilterType'", function() R.FilterType = Enum.KeyCode.A end)
        R.Margin = "2"
        assert(R.Margin == 2, "a number's text")
        R.Margin = 1 / 3
        assert(string.format("%.9g", R.Margin) == "0.333333343", "a float32")
        Fails("Margin must be finite and greater than or equal to 0, got -1.000000", function() R.Margin = -1 end)
        Fails("SizeGradient must be finite and greater than or equal to -1, got inf", function() R.SizeGradient = 1e40 end)
        Fails("Tolerance outside bounds: Expected 0 - 0.050000, got 0.051000", function() O.Tolerance = 0.051 end)
        O.Tolerance = 0.0501
        O.MaxParts = "2.9"
        assert(O.MaxParts == 2, "an int")
        O.MaxParts = 1e10
        assert(O.MaxParts == -2147483648, "an int32 out of range")
        O.MaxParts = true
        assert(O.MaxParts == 0, "no number")
        assert(RaycastParams.new({ IgnoreWater = true }).IgnoreWater == false, "new ignores its table")
        assert(#OverlapParams.new({ FilterDescendantsInstances = { A } }).FilterDescendantsInstances == 0, "new ignores its table")
        "#,
    );
}

/// A playing client queries a part with CanQuery off while CanCollide is on, whichever was written first, and leaves one out
/// only when both are off (Studio 0.747, client and server: tools/parity/probes/query_canquery.luau): every cast and box query,
/// FindPartOnRay, the Region3 queries, an Include filter naming it and GetPartsObscuringTarget alike. A box query pays
/// QUERY_UNQUERYABLE for each part it meets that both leave out (0.45 to 0.53 of a listed part's cost on a client).
#[test]
fn a_part_with_can_query_off_is_queried_while_can_collide_is_on() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local function Part(X, Query, Collide)
            local P = Instance.new("Part")
            P.Anchored, P.Size, P.Position, P.CanQuery, P.CanCollide = true, Vector3.new(10, 1, 10), Vector3.new(X, 90, 300), Query, Collide
            P.Parent = workspace
            return P
        end
        local On = Part(300, false, true)
        local Off = Part(400, false, false)
        local Late = Part(500, true, true)
        Late.CanQuery = false
        Late.CanCollide = false
        Late.CanCollide = true
        local Probe = Instance.new("Part")
        Probe.Anchored, Probe.Size, Probe.CanQuery, Probe.CanCollide = true, Vector3.new(2, 2, 2), false, false
        local function Seen(P)
            local Top, Down = P.Position + Vector3.new(0, 10, 0), Vector3.new(0, -20, 0)
            Probe.CFrame = P.CFrame
            local Region = Region3.new(P.Position - Vector3.one, P.Position + Vector3.one)
            local Include = RaycastParams.new()
            Include.FilterType = Enum.RaycastFilterType.Include
            Include.FilterDescendantsInstances = { P }
            local Hits = {
                workspace:Raycast(Top, Down), workspace:Raycast(Top, Down, Include), workspace:Blockcast(CFrame.new(Top), Vector3.one, Down),
                workspace:Spherecast(Top, 0.5, Down), workspace:FindPartOnRay(Ray.new(Top, Down)),
                workspace:GetPartBoundsInBox(P.CFrame, Vector3.one * 2)[1], workspace:GetPartBoundsInRadius(P.Position, 1)[1],
                workspace:GetPartsInPart(Probe)[1], workspace:FindPartsInRegion3(Region)[1],
            }
            local Count = 0
            for Index = 1, 9 do
                if Hits[Index] then
                    Count += 1
                end
            end
            return Count
        end
        assert(Seen(On) == 9, "CanQuery off, CanCollide on: " .. Seen(On))
        assert(Seen(Off) == 0, "both off: " .. Seen(Off))
        assert(Seen(Late) == 9, "CanCollide on again: " .. Seen(Late))
        local Cam = workspace.CurrentCamera
        Cam.CFrame = CFrame.lookAt(Vector3.new(300, 120, 300), Vector3.new(300, 60, 300))
        assert(#Cam:GetPartsObscuringTarget({ Vector3.new(300, 60, 300) }, {}) == 1, "obscuring, CanCollide on")
        Cam.CFrame = CFrame.lookAt(Vector3.new(400, 120, 300), Vector3.new(400, 60, 300))
        assert(#Cam:GetPartsObscuringTarget({ Vector3.new(400, 60, 300) }, {}) == 0, "obscuring, both off")
        -- what a box query pays for the parts both leave out: 10 such parts against none
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        for I = 1, 10 do
            local P = Instance.new("Part")
            P.Anchored, P.Size, P.Position, P.CanQuery, P.CanCollide = true, Vector3.one, Vector3.new(1000 + I * 1.5, 90, 300), false, false
            P.Parent = Folder
        end
        local function Cost()
            local Start = os.clock()
            workspace:GetPartBoundsInBox(CFrame.new(1009.75, 90, 300), Vector3.new(16, 2, 2))
            return os.clock() - Start
        end
        local _ = Cost()
        local Left = Cost()
        for _, P in Folder:GetChildren() do
            P.CanCollide = true
        end
        local _ = Cost()
        -- (the same walk listing the ten instead)
        shared.Unqueryable = Cost() - Left
        "##,
    );
    let listed = 10.0 * (crate::cost::QUERY_BOX_PART - crate::cost::QUERY_UNQUERYABLE);
    run(&lua, &engine, &format!("assert(math.abs(shared.Unqueryable - {listed:e}) < 1e-9, `{{shared.Unqueryable}} s more for ten listed than left out, not {listed:e}`)"));
}

/// A spatial query leaves out a part whose collision group does not collide with its params' CollisionGroup (Default when none are
/// given), whatever the part's CanCollide; a name that is not registered collides with every group, a change of the registry
/// counts at the next query, FindPartOnRay takes Default's, the Region3 queries take none, and the caster's own group does not
/// matter (Studio 0.747, a server and a loopback client alike: tools/parity/probes/query_collision_groups.luau).
#[test]
fn queries_leave_out_parts_their_params_group_does_not_collide_with() {
    check(
        r#"
        local PhysicsService = game:GetService("PhysicsService")
        for _, Name in { "QA", "QB", "QC" } do
            PhysicsService:RegisterCollisionGroup(Name)
        end
        PhysicsService:CollisionGroupSetCollidable("QA", "QB", false)
        PhysicsService:CollisionGroupSetCollidable("Default", "QC", false)
        local Parts = {}
        for Index, Group in { "Default", "QA", "QB", "QC" } do
            local P = Instance.new("Part")
            P.Anchored, P.Size, P.Position, P.CollisionGroup = true, Vector3.new(10, 1, 10), Vector3.new(300 + 40 * Index, 90, 300), Group
            P.Parent = workspace
            Parts[Group] = P
        end
        Parts.QB.CanCollide = false
        local Probe = Instance.new("Part")
        Probe.Anchored, Probe.Size, Probe.CanQuery, Probe.CanCollide, Probe.CollisionGroup = true, Vector3.new(2, 2, 2), false, false, "QC"
        local function Seen(Group, P)
            local R, O = RaycastParams.new(), OverlapParams.new()
            if Group then
                R.CollisionGroup, O.CollisionGroup = Group, Group
            end
            local Top, Down = P.Position + Vector3.new(0, 10, 0), Vector3.new(0, -20, 0)
            Probe.CFrame = P.CFrame
            local Hits = {
                workspace:Raycast(Top, Down, R), workspace:Blockcast(CFrame.new(Top), Vector3.one, Down, R), workspace:Spherecast(Top, 0.5, Down, R),
                workspace:GetPartBoundsInBox(P.CFrame, Vector3.one, O)[1], workspace:GetPartBoundsInRadius(P.Position, 1, O)[1], workspace:GetPartsInPart(Probe, O)[1],
            }
            local Count = 0
            for Index = 1, 6 do
                if Hits[Index] then
                    Count += 1
                end
            end
            return Count
        end
        local function Row(Group)
            return `{Seen(Group, Parts.Default)} {Seen(Group, Parts.QA)} {Seen(Group, Parts.QB)} {Seen(Group, Parts.QC)}`
        end
        assert(Row(nil) == "6 6 6 0", Row(nil))
        assert(Row("QA") == "6 6 0 6", Row("QA"))
        assert(Row("QB") == "6 0 6 6", Row("QB"))
        assert(Row("Nope") == "6 6 6 6", Row("Nope"))
        PhysicsService:CollisionGroupSetCollidable("QA", "QB", true)
        assert(Row("QA") == "6 6 6 6", "a change counts at once: " .. Row("QA"))
        local C = Parts.QC
        local Top = C.Position + Vector3.new(0, 10, 0)
        assert(workspace:FindPartOnRay(Ray.new(Top, Vector3.new(0, -20, 0))) == nil, "FindPartOnRay takes Default's group")
        assert(#workspace:FindPartsInRegion3(Region3.new(C.Position - Vector3.one, C.Position + Vector3.one)) == 1, "Region3 takes none")
        PhysicsService:UnregisterCollisionGroup("QC")
        assert(Row(nil) == "6 6 6 6", "a part in a group no longer registered collides with all: " .. Row(nil))
        "#,
    );
}

/// Cast edges as a playing client meets them (Studio 0.747, client and server: tools/parity/probes/query_edges.luau): a cast
/// touching a part only at its path's end or only at its start misses it; a Spherecast meets a box's edges and corners rounded;
/// a Blockcast's Position divides the two centres by the boxes' half extents along the hit face, at the face's edge where both
/// boxes' far edges coincide, and at the box's corner for a negative side.
#[test]
fn casts_meet_a_parts_edges_as_a_client_does() {
    check(
        r#"
        local Wall = Instance.new("Part")
        Wall.Anchored, Wall.Size, Wall.Position = true, Vector3.new(10, 1, 10), Vector3.new(300, 90, 300)
        Wall.Parent = workspace
        local Caster = Instance.new("Part")
        Caster.Anchored, Caster.Size, Caster.Position, Caster.CanQuery, Caster.CanCollide = true, Vector3.one, Vector3.new(300, 100, 300), false, false
        Caster.Parent = workspace
        local function R(V)
            return string.format("%.4f %.4f %.4f", V.X, V.Y, V.Z)
        end
        local V, Down = Vector3.new(300, 100, 300), Vector3.new(0, -9.5, 0)
        assert(workspace:Raycast(V, Down) == nil, "a ray ending on the face")
        assert(math.abs(workspace:Raycast(V, Down * 1.000001).Distance - 9.5) < 1e-4, "a ray past it")
        assert(workspace:Raycast(Vector3.new(300, 90.5, 300), Vector3.new(0, -5, 0)) == nil, "a ray starting on the face")
        assert(math.abs(workspace:Raycast(Vector3.new(290, 90.5, 300), Vector3.new(20, 0, 0)).Distance - 5) < 1e-4, "a ray along the face's plane")
        assert(workspace:Spherecast(V, 0.5, Vector3.new(0, -9, 0)) == nil, "a sphere ending touching")
        assert(workspace:Spherecast(Vector3.new(300, 91, 300), 0.5, Vector3.new(0, -5, 0)) == nil, "a sphere starting touching")
        assert(workspace:Blockcast(CFrame.new(V), Vector3.one, Vector3.new(0, -9, 0)) == nil, "a box ending touching")
        assert(workspace:Shapecast(Caster, Vector3.new(0, -9, 0)) == nil, "a shape ending touching")
        -- a sphere past a top edge: where it touches the edge, its normal from the edge to its centre
        local Hit = workspace:Spherecast(Vector3.new(290, 91, 300), 1, Vector3.new(20, 0, 0))
        assert(math.abs(Hit.Distance - (5 - math.sqrt(0.75))) < 1e-4 and R(Hit.Position) == "295.0000 90.5000 300.0000" and R(Hit.Normal) == "-0.8660 0.5000 0.0000", R(Hit.Normal))
        Hit = workspace:Spherecast(Vector3.new(290, 91, 300), 0.5, Vector3.new(20, 0, 0))
        assert(math.abs(Hit.Distance - 5) < 1e-4 and R(Hit.Normal) == "0.0000 1.0000 0.0000", "grazing the top: " .. Hit.Distance)
        -- Blockcast positions
        local function Block(X, Z, Size)
            return R(workspace:Blockcast(CFrame.new(X, 100, Z), Size, Vector3.new(0, -20, 0)).Position)
        end
        assert(Block(304.8, 304.6, Vector3.one) == "304.3636 90.5000 304.1818", Block(304.8, 304.6, Vector3.one))
        assert(Block(302, 301, Vector3.new(30, 2, 12)) == "300.5000 90.5000 300.4546", Block(302, 301, Vector3.new(30, 2, 12)))
        assert(Block(300, 300, Vector3.new(10, 1, 10)) == "305.0000 90.5000 305.0000", "coinciding far edges")
        assert(Block(302, 301, Vector3.new(-1, 1, 1)) == "301.5000 90.5000 300.9091", "a negative side's corner")
        assert(Block(300, 300, Vector3.new(-20, 1, 1)) == "290.0000 90.5000 300.0000", "a negative side across cells")
        assert(workspace:Blockcast(CFrame.new(300, 100, 300), Vector3.new(-600, 1, 1), Vector3.new(0, -20, 0)) == nil, "a negative side past 512")
        local Side = workspace:Blockcast(CFrame.new(290, 90.3, 300), Vector3.one, Vector3.new(10, 0, 0))
        assert(R(Side.Position) == "295.0000 90.1500 300.0000" and R(Side.Normal) == "-1.0000 0.0000 0.0000", R(Side.Position))
        "#,
    );
}

/// RaycastParams / OverlapParams ExcludeInstances and IncludeInstances (`{Instance}?`, nil until
/// assigned; Roblox's spatial query filters of April 2026, devforum.roblox.com/t/4563575 and the
/// reference): the part, then each ancestor in turn, and the first either list names decides,
/// ExcludeInstances first; with none named IncludeInstances = nil lets a part through and `{}` does
/// not. Every query honours them; with FilterDescendantsInstances set too both filters apply (as a client's do:
/// query_filter_combine.luau). A list assigned or read back is a copy.
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
    let engine = Engine::new(Options { profile: crate::client_profile::ClientProfile::parse(r#"{"device":{"topbarInset":58}}"#).unwrap(), ..Options::default() }).expect("engine");
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
#[inline(never)]
fn start_studio_window() -> (Lua, Rc<Engine>) {
    let engine = Engine::new(Options { profile: crate::client_profile::ClientProfile::parse(r#"{"device":{"camera":[908.8,895.2],"topbarInset":58}}"#).unwrap(), ..Options::default() }).expect("engine");
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

/// Workspace.CurrentCamera as a playing client takes it (Studio 0.747, tools/parity/probes/camera_current.luau and
/// camera_current_screen.luau): a write takes at once and parents the Workspace's other Camera children to nil (not a Folder's);
/// the render step gives the current camera the window and a near plane 0.1 out for good; nil reads nil until the simulation
/// step makes the Workspace's first Camera child current; a camera outside the Workspace gives way there to a new Camera that has
/// the window at once and its near plane from the next render step; the errors.
#[test]
fn the_current_camera_takes_a_write_and_the_simulation_step_settles_it() {
    let engine = Engine::new(Options {
        mode: "play".into(),
        profile: crate::client_profile::ClientProfile::parse(r#"{"device":{"camera":[908.8,895.2],"topbarInset":58}}"#).unwrap(),
        ..Options::default()
    })
    .expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    run(
        &lua,
        &engine,
        r##"
        local function Fails(Message, F)
            local Ok, E = pcall(F)
            assert(not Ok and string.gsub(tostring(E), "^[^\n]-:%d+: ", "") == Message, tostring(E))
        end
        local Window, Near = Vector2.new(908.8, 895.2), -0.10000000149011612
        local Log = {}
        local Start = workspace.CurrentCamera
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        local Nested = Instance.new("Camera")
        Nested.Parent = Folder
        local New = Instance.new("Camera")
        New.Name = "New"
        New.Parent = workspace
        workspace.Changed:Connect(function(Prop)
            if Prop == "CurrentCamera" then
                table.insert(Log, "current " .. tostring(workspace.CurrentCamera))
            end
        end)
        workspace.ChildRemoved:Connect(function(Child)
            table.insert(Log, "removed " .. Child.Name)
        end)
        New.Changed:Connect(function(Prop)
            if Prop == "ViewportSize" or Prop == "NearPlaneZ" then
                table.insert(Log, "New " .. Prop)
            end
        end)
        -- (from a frame's resumption point, as a game script writes: the render step comes before the next one)
        task.wait()
        workspace.CurrentCamera = New
        assert(workspace.CurrentCamera == New and Start.Parent == nil and Nested.Parent == Folder)
        assert(New.ViewportSize == Vector2.one and New.NearPlaneZ == -0.5)
        task.wait()
        assert(table.concat(Log, ",") == "current New,removed Camera,New ViewportSize,New NearPlaneZ", table.concat(Log, ","))
        assert(New.ViewportSize == Window and New.NearPlaneZ == Near, tostring(New.ViewportSize))
        assert(Start.ViewportSize == Window and Start.NearPlaneZ == Near)
        Start.Parent = workspace
        workspace.CurrentCamera = Start
        assert(New.Parent == nil and New.ViewportSize == Window and New.NearPlaneZ == Near)
        -- the current camera written again: nothing happens
        New.Parent = workspace
        workspace.CurrentCamera = Start
        assert(New.Parent == workspace)
        -- nil, then the Workspace's first Camera child (the others go)
        workspace.CurrentCamera = nil
        assert(workspace.CurrentCamera == nil)
        task.wait()
        assert(workspace.CurrentCamera == Start and New.Parent == nil and Nested.Parent == Folder, tostring(workspace.CurrentCamera))
        -- a camera in no parent is current until the simulation step puts a new Camera in the Workspace
        local Loose = Instance.new("Camera")
        workspace.CurrentCamera = Loose
        assert(workspace.CurrentCamera == Loose and Loose.Parent == nil and Start.Parent == nil)
        task.wait()
        local Made = workspace.CurrentCamera
        assert(Made ~= Loose and Made.Name == "Camera" and Made.Parent == workspace and Loose.ViewportSize == Window, tostring(Made))
        assert(Made.ViewportSize == Window and Made.NearPlaneZ == -0.5 and Made.FieldOfView == 70 and Made.CFrame == Instance.new("Camera").CFrame)
        task.wait()
        assert(Made.NearPlaneZ == Near)
        -- a camera in a Folder in the Workspace stays current
        workspace.CurrentCamera = Nested
        task.wait()
        assert(workspace.CurrentCamera == Nested and Made.Parent == nil)
        Fails("Expected Camera got Part for Workspace.CurrentCamera.", function() workspace.CurrentCamera = Instance.new("Part") end)
        Fails("Unable to assign property CurrentCamera. Camera expected, got string", function() workspace.CurrentCamera = "x" end)
        Fails("Unable to assign property CurrentCamera. Camera expected, got CFrame", function() workspace.CurrentCamera = CFrame.new() end)
        assert(workspace.CurrentCamera == Nested)
        "##,
    );
}

/// A Camera refuses a CFrame write that is not finite or has a rotation entry of 1.2 or more (Studio 0.747, a client and a server:
/// tools/parity/probes/camera_finite.luau): CFrame and CoordinateFrame keep what they hold and fire nothing, and the write costs
/// what an unchanged one does; Focus takes anything; PanUnits, TiltUnits and ZoomToExtents leave the CFrame as it was when theirs
/// is refused, and ZoomToExtents' Focus is the middle of the box's bounds (NaN for an infinite size, (0, 0, 0) for a huge one).
#[test]
fn a_camera_refuses_a_cframe_that_is_not_finite_or_far_from_a_rotation() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r##"
        local NaN, Inf = 0 / 0, math.huge
        local Cam = Instance.new("Camera")
        Cam.Parent = workspace
        Cam.CameraType = Enum.CameraType.Scriptable
        local Fired = 0
        Cam.Changed:Connect(function(Prop)
            if Prop == "CFrame" or Prop == "CoordinateFrame" then
                Fired += 1
            end
        end)
        local Kept = CFrame.new(5, 6, 7)
        Cam.CFrame = Kept
        task.wait()
        Fired = 0
        for _, Refused in { CFrame.new(NaN, 0, 0), CFrame.new(0, Inf, 0), CFrame.new(1e39, 0, 0), CFrame.new(1, 2, 3, 1.2, 0, 0, 0, 1, 0, 0, 0, 1), CFrame.new(1, 2, 3, 1, 0, 0, 0, 1, 0, 0, -1.5, 1), CFrame.new(1, 2, 3) * CFrame.Angles(NaN, 0, 0) } do
            Cam.CFrame = Refused
            assert(Cam.CFrame == Kept, tostring(Refused))
            Cam.CoordinateFrame = Refused
            assert(Cam.CFrame == Kept, tostring(Refused))
        end
        task.wait()
        assert(Fired == 0, Fired)
        for _, Taken in { CFrame.new(1, 2, 3, 1.199, 0, 0, 0, 1, 0, 0, 0, 1), CFrame.new(1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0), CFrame.new(3e38, 0, 0) } do
            Cam.CFrame = Taken
            assert(Cam.CFrame == Taken, tostring(Taken))
        end
        Cam.Focus = CFrame.new(NaN, 0, 0)
        assert(Cam.Focus.X ~= Cam.Focus.X)
        Cam.Focus = CFrame.new()
        ;(Cam :: any).focus = CFrame.new(NaN, 0, 0)
        assert(Cam.Focus.X ~= Cam.Focus.X)
        Cam.CFrame = Kept
        Cam:PanUnits(1)
        assert(Cam.CFrame == Kept)
        Cam.Focus = CFrame.new(Inf, 0, 0)
        assert(Cam:TiltUnits(1) == true and Cam.CFrame == Kept)
        Cam.Focus = CFrame.new()
        Cam:ZoomToExtents(CFrame.new(1, 2, 3), Vector3.new(Inf, 4, 4))
        local F = Cam.Focus.Position
        assert(Cam.CFrame == Kept and F.X ~= F.X and F.Y ~= F.Y and F.Z ~= F.Z)
        Cam:ZoomToExtents(CFrame.new(NaN, 2, 3), Vector3.new(4, 4, 4))
        F = Cam.Focus.Position
        assert(Cam.CFrame == Kept and F.X ~= F.X and F.Y == 2 and F.Z == 3)
        Cam:ZoomToExtents(CFrame.new(1, 2, 3), Vector3.new(1e38, 1e38, 1e38))
        assert(Cam.CFrame == Kept and Cam.Focus.Position == Vector3.zero, tostring(Cam.Focus))
        -- a refused write costs what an unchanged one does (after the frame's first write, which runs the engine's code cold)
        task.wait()
        Cam.CFrame = Kept
        local T0 = os.clock()
        for _ = 1, 200 do
            Cam.CFrame = Kept
        end
        local T1 = os.clock()
        local Bad = CFrame.new(NaN, 0, 0)
        for _ = 1, 200 do
            Cam.CFrame = Bad
        end
        local T2 = os.clock()
        assert(math.abs((T2 - T1) - (T1 - T0)) < 0.05 * (T1 - T0), `{T1 - T0} {T2 - T1}`)
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
#[inline(never)]
fn check_running(mode: &str, source: &str, seconds: f64) {
    check_with(Options { mode: mode.into(), ..Options::default() }, source, seconds);
}

/// Runs `source` in an engine of `options` for `seconds` of frames; panics unless it finished cleanly.
#[inline(never)]
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

/// Workspace:PGSIsEnabled and GetPhysicsThrottling as a Studio 0.748 playing client and its server answer: true and 100, whatever
/// the arguments (tools/parity/probes/workspace_physics_state.luau); plugin code in Studio's edit DataModel reads the same
/// (identity 5, the probe's calls run there by hand), which the edit run here stands for.
#[test]
fn workspace_physics_state_answers_as_a_playing_client() {
    for mode in ["play", "edit"] {
        check_running(
            mode,
            r##"
            local function Fails(Want, F)
                local Ok, Message = pcall(F)
                assert(not Ok and Message == Want, tostring(Message))
            end
            assert(workspace:PGSIsEnabled() == true and workspace:PGSIsEnabled(1, "x") == true)
            assert(select("#", workspace:PGSIsEnabled()) == 1 and select("#", workspace:GetPhysicsThrottling()) == 1)
            assert(workspace:GetPhysicsThrottling() == 100 and workspace:GetPhysicsThrottling(true) == 100)
            Fails("Expected ':' not '.' calling member function PGSIsEnabled", function() return (workspace :: any).PGSIsEnabled(game) end)
            Fails("Expected ':' not '.' calling member function GetPhysicsThrottling", function() return (workspace :: any).GetPhysicsThrottling(Instance.new("WorldModel")) end)
            "##,
            0.1,
        );
    }
}

/// Workspace:UnjoinFromOutsiders(objects) takes out (Parent = nil) every active joint between a listed part and a part that is not
/// listed, as a Studio 0.748 playing client and its server do: the parts below a listed Instance are listed too; Weld, ManualWeld,
/// Snap, Glue, ManualGlue, Motor and Motor6D count, but not Rotate, RotateP, RotateV, VelocityMotor, a WeldConstraint or a
/// disabled joint, nor one out of the Workspace's world; each part's joints go in the order of its list of active joints, as a
/// client keeps it (the order they were parented, not made); the list is cast whole before anything goes; a nested or repeated
/// entry lists nothing new (tools/parity/probes/unjoin_from_outsiders.luau). Each entry and each instance below the entries (once
/// a call, whether a joint exists yet or not) cost the work clock what a client spends on them (cost.rs UNJOIN_ENTRY,
/// UNJOIN_BELOW); the rest of the place costs nothing. A Weld to an unanchored part taken out pays, beyond what one between two
/// anchored parts pays (which changes no assembly), only the link prices that grow with its assemblies (LINK_FRAME, LINK_PART:
/// world/joints.rs charge_link while Joints::unpriced is up); a Glue pays what a Weld does. A Weld carrying others, ending several
/// links at once, pays every link price, as its Parent = nil does.
#[test]
fn unjoin_from_outsiders_takes_out_the_joints_to_unlisted_parts() {
    check_running(
        "play",
        &r##"
        local Root = Instance.new("Folder")
        Root.Parent = workspace
        local JointsService = game:GetService("JointsService")
        local function Brick(Name, Y, Parent, Anchored)
            local B = Instance.new("Part")
            B.Name = Name
            B.Anchored = Anchored ~= false
            B.CFrame = CFrame.new(0, Y, 0)
            B.Parent = Parent or Root
            return B
        end
        local Made = {}
        local function J(Class, Name, P0, P1, Parent, Enabled)
            local X = Instance.new(Class)
            X.Name = Name
            X.Part0, X.Part1 = P0, P1
            if Enabled == false then
                X.Enabled = false
            end
            X.Parent = Parent
            Made[Name] = X
            return X
        end
        local function Fails(Want, F)
            local Ok, Message = pcall(F)
            assert(not Ok and Message == Want, tostring(Message))
        end
        -- (before any joint is made, a call still looks at what is below its entries, and pays for it; in a frame, where the work
        -- clock runs)
        task.wait()
        do
            local Folders = Instance.new("Folder")
            for _ = 1, 2000 do
                Instance.new("Folder").Parent = Folders
            end
            workspace:UnjoinFromOutsiders({})
            local Start = os.clock()
            workspace:UnjoinFromOutsiders({})
            local Nothing = os.clock() - Start
            Start = os.clock()
            workspace:UnjoinFromOutsiders({ Folders })
            local Walked = os.clock() - Start - Nothing
            -- (an entry and 2,000 instances below it)
            assert(math.abs(Walked - (0.057e-6 + 2000 * 0.017e-6)) < 1e-7, string.format("a walk of 2,001 instances before any joint %.4g", Walked))
        end
        local A, B, C, D = Brick("A", 100), Brick("B", 101), Brick("C", 102), Brick("D", 103)
        J("Weld", "j1", C, D, Root)
        J("Weld", "j2", B, A, A)
        J("ManualWeld", "j3", A, B, JointsService)
        J("Motor6D", "j4", B, D, B)
        J("Snap", "j5", D, C, D)
        J("Glue", "j6", B, C, B)
        J("ManualGlue", "j7", A, C, workspace)
        J("Motor", "j8", D, B, D)
        J("Glue", "j9", C, A, C)
        local Outside = Brick("Outside", 130, game:GetService("ReplicatedStorage"))
        local Kept = {
            J("Weld", "disabled", B, A, B, false),
            J("Weld", "stored", B, A, game:GetService("ReplicatedStorage")),
            J("Weld", "half", B, nil, B),
            J("Weld", "self", B, B, B),
            J("Rotate", "rotate", B, A, B),
            J("RotateP", "rotatep", B, A, B),
            J("RotateV", "rotatev", B, A, B),
            J("VelocityMotor", "velocitymotor", B, A, B),
            J("Weld", "to-outside-part", B, Outside, B),
            J("Glue", "glue-to-outside-part", Outside, C, C),
        }
        local World = Instance.new("WorldModel")
        World.Parent = Root
        local M = Brick("M", 120, World)
        table.insert(Kept, J("Weld", "in-worldmodel", B, A, World))
        table.insert(Kept, J("Weld", "to-worldmodel-part", B, M, B))
        local Constraint = Instance.new("WeldConstraint")
        Constraint.Part0, Constraint.Part1 = B, A
        Constraint.Parent = B
        -- the list is cast whole first: nothing goes for a bad entry after a good one
        Fails("Unable to cast value to Object", function() workspace:UnjoinFromOutsiders({ B, nil, 1 }) end)
        Fails("Argument 1 missing or nil", function() workspace:UnjoinFromOutsiders() end)
        Fails("Expected table argument, got nil", function() workspace:UnjoinFromOutsiders(nil) end)
        Fails("Expected table argument, got userdata", function() workspace:UnjoinFromOutsiders(B) end)
        Fails("Expected ':' not '.' calling member function UnjoinFromOutsiders", function() (workspace :: any).UnjoinFromOutsiders(game, {}) end)
        assert(Made.j2.Parent == A, "a failed call took a joint out")
        -- every part listed: nothing is outside
        workspace:UnjoinFromOutsiders({ game })
        workspace:UnjoinFromOutsiders({ workspace })
        assert(Made.j2.Parent == A and Made.j3.Parent == JointsService, "a joint went with nothing outside")
        task.wait()
        local Order = {}
        local Connections = {}
        for Name, X in Made do
            table.insert(Connections, X.AncestryChanged:Connect(function(_, Parent)
                table.insert(Order, Name .. (if Parent then ">" .. Parent.Name else ""))
            end))
        end
        assert(select("#", workspace:UnjoinFromOutsiders({ B, C, B, x = A })) == 0)
        -- out at once, not destroyed: the parts stay named and the joint can be parented again
        for _, Name in { "j1", "j2", "j3", "j4", "j5", "j7", "j8", "j9" } do
            assert(Made[Name].Parent == nil, Name .. " stayed")
        end
        assert(Made.j6.Parent == B, "a joint between two listed parts went")
        for _, X in Kept do
            assert(X.Parent ~= nil, X.Name .. " went")
        end
        assert(Constraint.Parent == B, "the WeldConstraint went")
        assert(Made.j2.Part0 == B and Made.j2.Part1 == A)
        task.wait()
        -- each listed part's joints in the order they were parented (B's, then C's)
        assert(table.concat(Order, ",") == "j2,j3,j4,j8,j1,j5,j7,j9", table.concat(Order, ","))
        for _, Connection in Connections do
            Connection:Disconnect()
        end
        Made.j2.Parent = A
        -- the parts below a listed Instance are listed, and the assemblies split
        local Model = Instance.new("Model")
        Model.Parent = Root
        local E = Brick("E", 110, Model, false)
        local F = Brick("F", 111, E, false)
        local G = Brick("G", 112)
        J("Weld", "e", E, G, E)
        J("Weld", "f", F, G, F)
        J("Weld", "ef", E, F, E)
        assert(E.AssemblyRootPart == G)
        workspace:UnjoinFromOutsiders({ Model, E })
        assert(Made.e.Parent == nil and Made.f.Parent == nil and Made.ef.Parent == E)
        assert(E.AssemblyRootPart ~= G and E.AssemblyRootPart == F.AssemblyRootPart)
        -- (an entry below one listed before, or above one, or listed again, lists nothing new)
        Made.e.Parent, Made.f.Parent = E, F
        assert(E.AssemblyRootPart == G)
        workspace:UnjoinFromOutsiders({ F, Model, F, E })
        assert(Made.e.Parent == nil and Made.f.Parent == nil and Made.ef.Parent == E)
        -- the work clock: each entry, and each instance below them once a call (a list of 2,000 times the same folder walks
        -- it once and pays for 2,000 entries; 2,000 walks of its 2,001 instances would cost 68 ms)
        local Deep = Instance.new("Folder")
        for _ = 1, 2000 do
            Instance.new("Folder").Parent = Deep
        end
        local Many = table.create(2000, Deep)
        local function Spent(List)
            local Start = os.clock()
            workspace:UnjoinFromOutsiders(List)
            return os.clock() - Start
        end
        local Empty, Once, Repeated = Spent({}), Spent({ Deep }), Spent(Many)
        assert(math.abs(Once - Empty - (0.057e-6 + 2000 * 0.017e-6)) < 1e-8, string.format("one walk %.4g", Once - Empty))
        assert(math.abs(Repeated - Once - 1999 * 0.057e-6) < 1e-8, string.format("1,999 entries more %.4g", Repeated - Once))
        -- (what the rest of the place holds costs nothing: one listed part costs the same beside 5,000 more parts)
        local Lone, Other = Brick("Lone", 150), Brick("Other", 151)
        local function OnePart()
            local Weld = Instance.new("Weld")
            Weld.Part0, Weld.Part1 = Lone, Other
            Weld.Parent = Lone
            local Cost = Spent({ Lone })
            assert(Weld.Parent == nil)
            return Cost
        end
        local Alone = OnePart()
        local Crowd = Instance.new("Folder")
        for _ = 1, 5000 do
            Instance.new("Part").Parent = Crowd
        end
        Crowd.Parent = workspace
        local Beside = OnePart()
        assert(math.abs(Beside - Alone) < 1e-7, string.format("%.3g beside 5,000 parts, %.3g alone", Beside, Alone))
        -- (a Weld to an unanchored part taken out costs the call what one between two anchored parts does but the link prices that
        -- grow with its assemblies: UNJOIN_REMOVED holds the flat ones; 200 parts each joined to a part of its own outside the list,
        -- anchored or not)
        local function OwnParts(Class, Anchored)
            local Listed, Outside = Instance.new("Folder"), Instance.new("Folder")
            Listed.Parent, Outside.Parent = Root, Root
            local Joints = {}
            for Index = 1, 200 do
                table.insert(Joints, J(Class, Class .. Index, Brick("L" .. Index, 300 + Index, Listed), Brick("O" .. Index, 600 + Index, Outside, Anchored), Listed))
            end
            task.wait()
            local Cost = Spent({ Listed })
            for _, Joint in Joints do
                assert(Joint.Parent == nil, Joint.Name .. " stayed")
            end
            Listed:Destroy()
            Outside:Destroy()
            return Cost
        end
        -- (each Weld to an unanchored part taken out at a frame's start leaves two parts of one part each, marked once: the frame's
        -- first change, and each assembly's first change with each part it marks, the unanchored one's dearer, are all it pays beyond
        -- a Weld between two anchored parts, each an assembly of its own, which changes no assembly, as a client's call pays each
        -- split's assembly price; a Glue, a rigid joint, pays what a Weld does)
        local Welded, Fixed = OwnParts("Weld", false), OwnParts("Weld")
        local Want = LINK_FRAME + 400 * (LINK_ASSEMBLY + LINK_PART + LINK_SMALL_PART) + 200 * LINK_FREE_PART
        assert(math.abs(Welded - Fixed - Want) < 1e-7, string.format("200 Welds taken out cost %.4g more than 200 between anchored parts, not %.4g", Welded - Fixed, Want))
        local Glued, FixedGlue = OwnParts("Glue", false), OwnParts("Glue")
        assert(math.abs(Glued - Welded) < 1e-7 and math.abs(FixedGlue - Fixed) < 1e-7, string.format("200 Glue taken out cost %.4g and %.4g more than 200 Welds", Glued - Welded, FixedGlue - Fixed))
        -- (one leaf of an anchored hub welded to 2,000 unanchored leaves: the link change marks both sides, the leaf and the hub with
        -- its 1,999 other leaves and its Glue leaf (Glue joins assemblies as a Weld does); the Glue leaf taken out next costs the call
        -- the same but for the leaf the hub's assembly no longer holds)
        do
            local Star = Instance.new("Folder")
            Star.Parent = Root
            local Hub = Brick("Hub", 1000, Star)
            local Leaves = {}
            for Index = 1, 2000 do
                Leaves[Index] = Brick("Leaf" .. Index, 1001 + Index, Star, false)
                J("Weld", "Spoke" .. Index, Hub, Leaves[Index], Leaves[Index])
            end
            local Glued = Brick("GluedLeaf", 4000, Star, false)
            J("Glue", "GlueSpoke", Hub, Glued, Glued)
            -- (each call at a frame's start, after an empty one there, which takes on what the first call after building the star
            -- would pay besides)
            task.wait()
            Spent({})
            local WeldLeaf = Spent({ Leaves[1] })
            assert(Made.Spoke1.Parent == nil, "the leaf's Weld stayed")
            task.wait()
            Spent({})
            local GlueLeaf = Spent({ Glued })
            assert(Made.GlueSpoke.Parent == nil, "the leaf's Glue stayed")
            assert(math.abs(WeldLeaf - GlueLeaf - LINK_PART) < 1e-7, string.format("a leaf's Weld cost %.4g more than the Glue leaf's next, not %.4g", WeldLeaf - GlueLeaf, LINK_PART))
            Star:Destroy()
        end
        -- (a listed part's Weld to an unlisted part, carrying a Folder of 100 Welds between fresh unanchored pairs, ends 101 links in
        -- one write when it goes, splitting each pair: the call pays every link price, as the same Weld's Parent = nil does, at most
        -- the flat prices of the Weld's own change less)
        do
            local Carry = Instance.new("Folder")
            Carry.Parent = Root
            local Listed, Outside = Brick("CarryListed", 6000, Carry), Brick("CarryOutside", 6001, Carry, false)
            local Pairs = Instance.new("Folder")
            for Index = 1, 100 do
                J("Weld", "Pair" .. Index, Brick("PairA" .. Index, 6100 + 2 * Index, Carry, false), Brick("PairB" .. Index, 6101 + 2 * Index, Carry, false), Pairs)
            end
            local Carrier = J("Weld", "Carrier", Listed, Outside, Listed)
            Pairs.Parent = Carrier
            task.wait()
            Spent({})
            local Call = Spent({ Listed })
            assert(Carrier.Parent == nil, "the carrying Weld stayed")
            Carrier.Parent = Listed
            task.wait()
            local Start = os.clock()
            Carrier.Parent = nil
            local Plain = os.clock() - Start
            local Waived = LINK_CHANGED + LINK_SPLIT_AGAIN
            assert(Call >= Plain - Waived, string.format("the carrying Weld's removal cost the call %.4g, its Parent = nil %.4g", Call, Plain))
            Carry:Destroy()
        end

        -- the order is each part's list of active joints, as a client keeps it (Studio 0.748): a joint goes last when it is
        -- parented with its parts set, not when it is made; one taken out and parented again goes last, and the list's last
        -- joint takes its old place
        local function Order(Build)
            local P0, P1 = Brick("P0", 160), Brick("P1", 161)
            local X = {}
            for Index = 1, 4 do
                X[Index] = Instance.new("Weld")
                X[Index].Name = "x" .. Index
                X[Index].Part0, X[Index].Part1 = P0, P1
            end
            Build(X, P0)
            task.wait()
            local Went = {}
            local Watching = {}
            for _, Weld in X do
                table.insert(Watching, Weld.AncestryChanged:Connect(function(_, Parent)
                    if Parent == nil then
                        table.insert(Went, Weld.Name)
                    end
                end))
            end
            workspace:UnjoinFromOutsiders({ P0 })
            task.wait()
            for _, Connection in Watching do
                Connection:Disconnect()
            end
            P0:Destroy()
            P1:Destroy()
            return table.concat(Went, ",")
        end
        local Parented = Order(function(X, P0)
            for Index = 4, 1, -1 do
                X[Index].Parent = P0
            end
        end)
        assert(Parented == "x4,x3,x2,x1", Parented)
        local Back = Order(function(X, P0)
            for Index = 1, 4 do
                X[Index].Parent = P0
            end
            X[2].Parent = nil
            X[2].Parent = P0
        end)
        assert(Back == "x1,x4,x3,x2", Back)
        "##
        .replace("LINK_FRAME", &format!("{:e}", crate::cost::LINK_FRAME))
        .replace("LINK_FREE_PART", &format!("{:e}", crate::cost::LINK_FREE_PART))
        .replace("LINK_SMALL_PART", &format!("{:e}", crate::cost::LINK_SMALL_PART))
        .replace("LINK_PART", &format!("{:e}", crate::cost::LINK_PART))
        .replace("LINK_CHANGED", &format!("{:e}", crate::cost::LINK_CHANGED))
        .replace("LINK_SPLIT_AGAIN", &format!("{:e}", crate::cost::LINK_SPLIT_AGAIN))
        .replace("LINK_ASSEMBLY", &format!("{:e}", crate::cost::LINK_ASSEMBLY)),
        0.75,
    );
}

/// Workspace:JoinToOutsiders(objects, mode) welds each listed part to each part outside the list whose face its face touches, as a
/// playing client and its server do (roblox-sim, Studio 0.747; tools/parity/probes/join_to_outsiders.luau, whose client lines the
/// strings here are): the mode is cast before the list; a Weld named Weld under the listed part, Part0 the listed part, C0 its face
/// (centre, Z out, negative zeros as a client gives them), C1 Part1.CFrame:Inverse() * (Part0.CFrame * C0); None makes nothing;
/// a gap over 0.01, an overlap of 0.1 or less, a turn of 0.573 degrees, a Ball or a wedge's empty half make none; a joint or
/// constraint between the two stops it, a disabled one does not, nor does a weld out of the Workspace; two listed parts are never
/// welded; a listed part's welds go in the order the other parts came into the Workspace, a subtree's parts the moved one first,
/// then each after its own descendants.
#[test]
fn join_to_outsiders_welds_listed_parts_to_the_faces_they_touch() {
    check_running(
        "play",
        r##"
        local Root = Instance.new("Folder")
        Root.Parent = workspace
        local Storage = game:GetService("ReplicatedStorage")
        local Base = CFrame.new(0, 100, 0)
        local All = Enum.JointCreationMode.All
        local function N(V)
            if V == 0 then
                return if 1 / V < 0 then "-0" else "0"
            end
            return string.format("%.9g", V)
        end
        local function CF(C)
            local Parts = {}
            for _, V in { C:GetComponents() } do
                table.insert(Parts, N(V))
            end
            return table.concat(Parts, ",")
        end
        local function Brick(Name, At, Size, Props, Class, Parent)
            local B = Instance.new(Class or "Part")
            B.Name = Name
            B.Anchored = true
            B.Size = Size or Vector3.new(4, 1, 4)
            for K, V in Props or {} do
                B[K] = V
            end
            B.CFrame = At
            B.Parent = Parent or Root
            return B
        end
        local function Welds(Part)
            local Lines = {}
            for _, X in Part:GetChildren() do
                if X:IsA("JointInstance") then
                    table.insert(Lines, X.ClassName .. " '" .. X.Name .. "' " .. X.Part0.Name .. "-" .. X.Part1.Name .. " C0=" .. CF(X.C0) .. " C1=" .. CF(X.C1))
                end
            end
            return table.concat(Lines, "; ")
        end
        local function Names(Part)
            local Out = {}
            for _, X in Part:GetChildren() do
                table.insert(Out, X.Part1.Name)
            end
            return table.concat(Out, ",")
        end
        local function Fails(Want, F)
            local Ok, Message = pcall(F)
            assert(not Ok and Message == Want, tostring(Message))
        end
        local function Same(Got, Want)
            assert(Got == Want, "\n got " .. tostring(Got) .. "\nwant " .. tostring(Want))
        end
        -- B (listed) at BAt with A under it: B's welds
        local function Case(BAt, BSize, BProps, Mode, ASize, AProps, AClass, AAt)
            Brick("A", AAt or Base, ASize, AProps, AClass)
            local B = Brick("B", BAt or Base * CFrame.new(0, 1, 0), BSize, BProps)
            workspace:JoinToOutsiders({ B }, Mode or All)
            local Got = Welds(B)
            Root:ClearAllChildren()
            return Got
        end

        -- the arguments: the mode first
        Fails("Argument 2 missing or nil", function() workspace:JoinToOutsiders() end)
        Fails("Argument 2 missing or nil", function() workspace:JoinToOutsiders({}) end)
        Fails("Unable to cast string to JointCreationMode", function() workspace:JoinToOutsiders(5, "Bogus") end)
        Fails("Unable to cast double to JointCreationMode", function() workspace:JoinToOutsiders({}, 3) end)
        Fails("Unable to cast double to JointCreationMode", function() workspace:JoinToOutsiders({}, -1) end)
        Fails("Unable to cast null to JointCreationMode", function() workspace:JoinToOutsiders({}, Enum.JointCreationMode) end)
        Fails("Unable to cast Array to JointCreationMode", function() workspace:JoinToOutsiders({}, {}) end)
        Fails("Unable to cast Material to JointCreationMode", function() workspace:JoinToOutsiders({}, Enum.Material.Plastic) end)
        Fails("Expected table argument, got number", function() workspace:JoinToOutsiders(5, All) end)
        Fails("Unable to cast value to Object", function() workspace:JoinToOutsiders({ 1 }, Enum.JointCreationMode.None) end)
        Fails("Expected ':' not '.' calling member function JoinToOutsiders", function() (workspace :: any).JoinToOutsiders({}, All) end)
        assert(select("#", workspace:JoinToOutsiders({}, -0.5)) == 0)

        -- the weld, by mode and face
        local Top = "Weld 'Weld' B-A C0=0,-0.5,0,1,0,0,-0,-0,-1,0,1,0 C1=0,0.5,0,1,0,0,0,0,-1,0,1,0"
        Same(Case(), Top)
        Same(Case(nil, nil, nil, "Surface"), Top)
        Same(Case(nil, nil, nil, 1.5), Top)
        Same(Case(nil, nil, nil, Enum.JointCreationMode.None), "")
        Same(Case(nil, nil, nil, 2.9), "")
        Same(Case(nil, nil, { BottomSurface = Enum.SurfaceType.Hinge }, Enum.JointCreationMode.Surface), Top)
        Same(Case(Base + Vector3.new(-4, 0, 0)), "Weld 'Weld' B-A C0=2,0,0,0,0,1,0,1,0,-1,-0,-0 C1=-2,0,0,0,0,1,0,1,0,-1,0,0")
        Same(Case(Base + Vector3.new(0, -3, 0), Vector3.new(2, 3, 5), nil, nil, Vector3.new(4, 3, 5)), "Weld 'Weld' B-A C0=0,1.5,0,-1,-0,-0,0,0,1,0,1,0 C1=0,-1.5,0,-1,0,0,0,0,1,0,1,0")
        Same(Case(Base * CFrame.new(0, 0.5, 0), Vector3.new(0.7, 0.7, 0.7), nil, nil, Vector3.new(4, 0.3, 4)), "Weld 'Weld' B-A C0=0,-0.349999994,0,1,0,0,-0,-0,-1,0,1,0 C1=0,0.150001526,0,1,0,0,0,0,-1,0,1,0")
        do
            local A = Brick("A", CFrame.new(40, 80, 0) * CFrame.Angles(0.3, 0.7, 0.2))
            local B = Brick("B", A.CFrame * CFrame.new(0.3, 1, -0.6), Vector3.new(2, 1, 2))
            workspace:JoinToOutsiders({ B }, All)
            Same(Welds(B), "Weld 'Weld' B-A C0=0,-0.5,0,1,0,0,-0,-0,-1,0,1,0 C1=0.300003052,0.5,-0.600003719,1.00000012,-5.96046448e-08,0,0,2.98023224e-08,-1.00000012,-5.96046448e-08,1.00000012,-2.98023224e-08")
            local X = B:FindFirstChildOfClass("Weld")
            assert(X.Parent == B and X.Part0 == B and X.Part1 == A and X.Enabled)
            Root:ClearAllChildren()
        end
        -- when faces touch: a float32 gap of 0.01, an overlap of more than 0.1 (a separating axis's), 0.573 degrees, flat faces
        Same(Case(CFrame.new(0, 1.51, 0), nil, nil, nil, nil, nil, nil, CFrame.new(0, 0.5, 0)) ~= "", true)
        Same(Case(Base * CFrame.new(0, 1.0101, 0)), "")
        Same(Case(Base * CFrame.new(3.9, 1, 0)), "")
        Same(Case(Base * CFrame.new(1.92, 0.525, 0), Vector3.new(0.05, 0.05, 0.05)) ~= "", true)
        Same(Case(Base * CFrame.new(1.93, 0.525, 0), Vector3.new(0.05, 0.05, 0.05)), "")
        Same(Case(Base * CFrame.new(0, 1, 0) * CFrame.Angles(0, math.rad(0.572), 0)) ~= "", true)
        Same(Case(Base * CFrame.new(0, 1, 0) * CFrame.Angles(0, math.rad(0.573), 0)), "")
        Same(Case(Base * CFrame.new(0, 1, 0) * CFrame.Angles(math.rad(0.3), 0, 0)), "")
        Same(Case(Base * CFrame.new(0, 2.5, 0), Vector3.new(4, 4, 4), { Shape = Enum.PartType.Ball }), "")
        Same(Case(Base * CFrame.new(0, 2.5, 0), Vector3.new(4, 4, 4), { Shape = Enum.PartType.Cylinder }), "")
        Same(Case(Base * CFrame.new(-1.5, 2.25, -1.5), Vector3.new(0.5, 0.5, 0.5), nil, nil, Vector3.new(4, 4, 4), nil, "WedgePart", Base * CFrame.Angles(0, 0, math.rad(90))), "")
        Same(Case(Base * CFrame.new(1.5, 2.25, 1.5), Vector3.new(0.5, 0.5, 0.5), nil, nil, Vector3.new(4, 4, 4), nil, "WedgePart", Base * CFrame.Angles(0, 0, math.rad(90))) ~= "", true)

        -- what is between them already
        for Kind, Want in { Weld = 0, WeldConstraint = 0, Glue = 0, Rotate = 0, HingeConstraint = 0, VelocityMotor = 1, NoCollisionConstraint = 1, ["disabled Weld"] = 1, ["Weld in ReplicatedStorage"] = 1, twice = 1 } do
            local A = Brick("A", Base)
            local B = Brick("B", Base * CFrame.new(0, 1, 0))
            local Class = if Kind == "disabled Weld" or Kind == "Weld in ReplicatedStorage" then "Weld" else Kind
            if Kind == "twice" then
                workspace:JoinToOutsiders({ B }, All)
            else
                local X = Instance.new(Class)
                if X:IsA("Constraint") then
                    local A0, A1 = Instance.new("Attachment"), Instance.new("Attachment")
                    A0.Parent, A1.Parent = A, B
                    X.Attachment0, X.Attachment1 = A0, A1
                else
                    X.Part0, X.Part1 = A, B
                end
                if Kind == "disabled Weld" then
                    X.Enabled = false
                end
                X.Parent = if Kind == "Weld in ReplicatedStorage" then Storage else A
            end
            workspace:JoinToOutsiders({ B }, All)
            local Made = #Welds(B):split("Weld '") - 1
            assert(Made == Want, Kind .. ": " .. Made .. " welds")
            Root:ClearAllChildren()
            for _, X in Storage:GetChildren() do
                if X:IsA("Weld") then
                    X:Destroy()
                end
            end
        end
        -- which parts
        do
            local A = Brick("A", Base)
            local B = Brick("B", Base * CFrame.new(0, 1, 0))
            workspace:JoinToOutsiders({ Root }, All)
            assert(#A:GetChildren() == 0 and #B:GetChildren() == 0, "both listed")
            Root:ClearAllChildren()
            local World = Instance.new("WorldModel")
            World.Parent = Root
            Brick("A", Base, nil, nil, nil, World)
            B = Brick("B", Base * CFrame.new(0, 1, 0))
            workspace:JoinToOutsiders({ B }, All)
            assert(#B:GetChildren() == 0, "A in a WorldModel")
            Root:ClearAllChildren()
        end
        -- the order: the other parts as they came into the Workspace
        local At = { A = Base, C = Base * CFrame.new(0, 2, 0), D = Base * CFrame.new(4, 1, 0), E = Base * CFrame.new(-4, 1, 0) }
        do
            local B = Brick("B", Base * CFrame.new(0, 1, 0))
            for _, Name in { "D", "C", "A" } do
                Brick(Name, At[Name])
            end
            workspace:JoinToOutsiders({ B }, All)
            Same(Names(B), "D,C,A")
            Root:ClearAllChildren()
            B = Brick("B", Base * CFrame.new(0, 1, 0))
            local A = Brick("A", At.A)
            Brick("C", At.C)
            Brick("D", At.D)
            A.Parent = nil
            A.Parent = Root
            workspace:JoinToOutsiders({ B }, All)
            Same(Names(B), "C,D,A")
            Root:ClearAllChildren()
            local Fo = Instance.new("Folder")
            local D = Brick("D", At.D, nil, nil, nil, Fo)
            Brick("A", At.A, nil, nil, nil, D)
            local Sub = Instance.new("Folder")
            Sub.Parent = Fo
            Brick("E", At.E, nil, nil, nil, Sub)
            Brick("C", At.C, nil, nil, nil, Fo)
            B = Brick("B", Base * CFrame.new(0, 1, 0))
            Fo.Parent = Root
            workspace:JoinToOutsiders({ B }, All)
            Same(Names(B), "A,D,E,C")
            Root:ClearAllChildren()
            local P = Brick("P", At.C, nil, nil, nil, Storage)
            Brick("Q", At.A, nil, nil, nil, P)
            B = Brick("B", Base * CFrame.new(0, 1, 0))
            P.Parent = Root
            workspace:JoinToOutsiders({ B }, All)
            Same(Names(B), "P,Q")
            Root:ClearAllChildren()
        end
        "##,
        0.1,
    );
}

/// What JoinToOutsiders costs the work clock, against a playing client's medians (roblox-sim, Studio 0.747; cost.rs JOIN_CAST and
/// the rest): an empty list 7.6 us, one lone part 21.6, 100 lone 2 x 1 x 2 parts 6 studs apart 185 us, the same each on a part
/// turned 45 degrees 395, each welded already 236, each on a part of its own 1,049 (100 welds), all on one part 967; ree within
/// x0.6 to x1.5 of each (a method call's price alone, about 2 us, before JoinToOutsiders was emulated).
#[test]
fn join_to_outsiders_costs_what_a_client_spends() {
    check_running(
        "play",
        r##"
        workspace.Gravity = 0
        local function Scene(Kind)
            local Root = Instance.new("Folder")
            Root.Parent = workspace
            local Listed = {}
            for I = 1, 100 do
                local At = Vector3.new(300 + (I - 1) * 6, 300, 300)
                local P = Instance.new("Part")
                P.Anchored, P.Size, P.CFrame, P.Parent = true, Vector3.new(2, 1, 2), CFrame.new(At), Root
                Listed[I] = P
                if Kind ~= "lone" and Kind ~= "star" then
                    local Q = Instance.new("Part")
                    Q.Anchored, Q.Size, Q.Parent = true, Vector3.new(2, 1, 2), Root
                    Q.CFrame = CFrame.new(At - Vector3.new(0, 1, 0)) * (if Kind == "near" then CFrame.Angles(0, math.rad(45), 0) else CFrame.identity)
                    if Kind == "blocked" then
                        local X = Instance.new("Weld")
                        X.Part0, X.Part1, X.Parent = Q, P, Q
                    end
                end
            end
            if Kind == "star" then
                local G = Instance.new("Part")
                G.Anchored, G.Size, G.CFrame, G.Parent = true, Vector3.new(596, 1, 2), CFrame.new(300 + 297, 299, 300), Root
            end
            return Root, Listed
        end
        local function Within(Label, Got, Client)
            local Ratio = Got / Client
            assert(Ratio > 0.6 and Ratio < 1.5, string.format("%s: %.1f us, x%.2f of the client's %.1f", Label, Got * 1e6, Ratio, Client * 1e6))
        end
        local function Time(F)
            task.wait()
            task.wait()
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        Within("an empty list", Time(function() workspace:JoinToOutsiders({}, Enum.JointCreationMode.All) end), 7.6e-6)
        for _, Case in { { "lone", 185e-6 }, { "near", 395e-6 }, { "blocked", 236e-6 }, { "weld", 1049e-6 }, { "star", 967e-6 } } do
            local Kind, Client = Case[1], Case[2]
            local Root, Listed = Scene(Kind)
            if Kind == "lone" then
                Within("one lone part", Time(function() workspace:JoinToOutsiders({ Listed[1] }, Enum.JointCreationMode.All) end), 21.6e-6)
            end
            Within(Kind, Time(function() workspace:JoinToOutsiders(Listed, Enum.JointCreationMode.All) end), Client)
            local Made = 0
            for _, P in Listed do
                Made += #P:GetChildren()
            end
            assert(Made == (if Kind == "weld" or Kind == "star" then 100 else 0), Kind .. " made " .. Made)
            Root:Destroy()
        end
        "##,
        0.5,
    );
}

/// A joint leaves its parts' lists of active joints at the same cost however long they are (joints.rs Joints::detach swaps at the
/// place it keeps for the joint): a part holding 30,000 Glue to another leaves the Workspace and comes back 16 times, and then
/// UnjoinFromOutsiders takes the Glue out, in at most twice the host time the same 30,000 Glue take spread over 30 parts of 1,000,
/// each part glued to one of its own, which is the same work (0.29 s and 0.25 s here, the trips the faster of two runs taken in
/// turn). A joint used to be looked for in each list before it left it,
/// about 9e8 comparisons a trip with one part and 3e7 with 30: 2.2 s and 0.30 s.
#[test]
fn a_part_with_many_joints_moves_and_unjoins_on_little_host_time() {
    let rigs = [many_joints(1), many_joints(30)];
    let time = |(lua, engine, _): &(Lua, Rc<Engine>, Vec<crate::InstanceId>), source: &str| {
        let start = std::time::Instant::now();
        run(lua, engine, source);
        start.elapsed().as_secs_f64()
    };
    let trips = r#"
        local Moving = shared.Moving
        for _ = 1, 16 do
            Moving.Parent = game:GetService("ReplicatedStorage")
            Moving.Parent = shared.Box
        end
        "#;
    let mut host = [f64::INFINITY; 2];
    for _ in 0..2 {
        for (arm, rig) in rigs.iter().enumerate() {
            host[arm] = host[arm].min(time(rig, trips));
        }
    }
    for (arm, rig) in rigs.iter().enumerate() {
        host[arm] += time(rig, "workspace:UnjoinFromOutsiders({ shared.Moving })");
        let state = rig.1.state.borrow();
        for &glue in &rig.2 {
            assert!(state.dom.at(glue).parent.is_none(), "a Glue stayed");
        }
    }
    let [one, spread] = host;
    assert!(one < 2.0 * spread + 0.05, "16 trips and an unjoin took {one:.3} s of host time with 30,000 Glue to one part, {spread:.3} s with them spread over 30");
}

/// An engine with 30,000 Glue spread over `parts` parts that move together (`shared.Moving`), each part glued to one of its own
/// that stays.
#[inline(never)]
fn many_joints(parts: usize) -> (Lua, Rc<Engine>, Vec<crate::InstanceId>) {
    let (lua, engine) = start_play();
    let reflection = engine.reflection.clone();
    let workspace = engine.world().workspace;
    let model_class = reflection.class_id("Model").unwrap();
    let folder_class = reflection.class_id("Folder").unwrap();
    let part_class = reflection.class_id("Part").unwrap();
    let glue_class = reflection.class_id("Glue").unwrap();
    let anchored_prop = reflection.name_id("Anchored").unwrap();
    let part0_prop = reflection.name_id("Part0").unwrap();
    let part1_prop = reflection.name_id("Part1").unwrap();
    let (box_id, moving_id, joints) = {
        let mut state = engine.state.borrow_mut();
        // (made out of the DataModel and moved in at once; a hundred Glue to a folder, so that taking one out of its parent's
        // children is not what is timed; the parts that move in a Model of their own)
        let box_id = crate::instance::create_named(&mut state, &reflection, model_class, "Model");
        let moving_id = crate::instance::create_named(&mut state, &reflection, model_class, "Model");
        state.dom.attach(moving_id, box_id);
        let mut joints = Vec::with_capacity(30_000);
        for _ in 0..parts {
            let p = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
            let q = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
            crate::instance::raw_set_silent(&mut state, &reflection, p, anchored_prop, crate::value::Value::Bool(true));
            crate::instance::raw_set_silent(&mut state, &reflection, q, anchored_prop, crate::value::Value::Bool(true));
            state.dom.attach(p, moving_id);
            state.dom.attach(q, box_id);
            for _ in 0..(300 / parts) {
                let holder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                for _ in 0..100 {
                    let glue = crate::instance::create_named(&mut state, &reflection, glue_class, "Glue");
                    crate::instance::raw_set_silent(&mut state, &reflection, glue, part0_prop, crate::value::Value::Instance(p));
                    crate::instance::raw_set_silent(&mut state, &reflection, glue, part1_prop, crate::value::Value::Instance(q));
                    state.dom.attach(glue, holder);
                    super::joints::made_in_place(&mut state, &reflection, glue);
                    joints.push(glue);
                }
                state.dom.attach(holder, box_id);
            }
        }
        assert_eq!(joints.len(), 30_000);
        (box_id, moving_id, joints)
    };
    engine_lua::instance::set_parent(&engine, box_id, Some(workspace)).expect("box in workspace");
    {
        let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared").get("shared").expect("shared");
        shared.set("Box", engine_lua::instance_value(&lua, &engine, box_id).unwrap()).unwrap();
        shared.set("Moving", engine_lua::instance_value(&lua, &engine, moving_id).unwrap()).unwrap();
    }
    (lua, engine, joints)
}

/// UnjoinFromOutsiders looks at each instance below its entries once a call, making it whole as the walk reaches it when it was
/// imported lazily (joints.rs unjoin_from_outsiders): a list holding the same Folder of 10,000 Folders 2,000 times takes at most
/// twice the host time of a list holding it once (0.33 ms and 0.27 ms here). Each entry's subtree used to be made whole before
/// the repeats were dropped, 2,000 walks of its 10,001 instances: 0.16 s.
#[test]
fn a_repeated_entry_unjoins_on_little_host_time() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r#"
        local Deep = Instance.new("Folder")
        for _ = 1, 10000 do
            Instance.new("Folder").Parent = Deep
        end
        Deep.Parent = workspace
        shared.Deep, shared.Many = Deep, table.create(2000, Deep)
        workspace:UnjoinFromOutsiders({})
        "#,
    );
    let time = |source: &str| {
        let start = std::time::Instant::now();
        run(&lua, &engine, source);
        start.elapsed().as_secs_f64()
    };
    let once = time("workspace:UnjoinFromOutsiders({ shared.Deep })");
    let repeated = time("workspace:UnjoinFromOutsiders(shared.Many)");
    assert!(repeated < 2.0 * once + 0.05, "a list of the same Folder 2,000 times took {repeated:.4} s of host time, the Folder once {once:.4} s");
}

/// A WorldRoot that leaves every world takes its world along, so nothing below it is looked at, and an instance that leaves walks
/// down to the WorldRoots below it, not into them (joints.rs left_every_world): a chain of 2,000 nested WorldModels whose bottom
/// part 20,000 welds name is taken apart from the top, one WorldModel at a time, and so is a chain of 2,000 WorldModels each
/// inside a Folder, one Folder at a time, in about the host time the same chains take with 20,000 Folders in place of the welds
/// (most of it the AncestryChanged walk of each removed subtree, instance.rs fire_ancestry_changed), and the welds still link
/// afterwards. Each WorldModel used to walk everything below it as it left, and re-check every weld and each of the bottom
/// parts' joints: 5.7 s for the first chain alone, where the Folders' take 0.2. A Folder that walked below the WorldModel it
/// holds would take the welds out of their parts' lists, which are the bottom WorldModel's still, and charge for it: the first
/// Folder's move costs the same work with the welds enabled as with them disabled. The two chains are timed in turn, up to three
/// tries, so a busy host weighs on both alike.
#[test]
fn nested_world_models_leave_their_worlds_on_little_host_time() {
    for wrapped in [false, true] {
        let (mut welded, mut plain, mut first) = (0.0, 0.0, 0.0);
        for _ in 0..3 {
            (welded, first) = nested_world_models_leave(wrapped, "enabled");
            (plain, _) = nested_world_models_leave(wrapped, "folders");
            if welded < 2.0 * plain + 0.05 {
                break;
            }
        }
        let chain = if wrapped { "2,000 WorldModels each in a Folder" } else { "2,000 nested WorldModels" };
        assert!(welded < 2.0 * plain + 0.05, "{chain} taken apart from the top took {welded:.3} s of host time with 20,000 welds at the bottom, {plain:.3} s with 20,000 Folders");
        if wrapped {
            let (_, disabled) = nested_world_models_leave(wrapped, "disabled");
            assert!((first - disabled).abs() < 1e-6, "the first Folder's move cost {first:.6} s of work, {disabled:.6} s with the welds disabled");
        }
    }
}

/// Builds the chain (`wrapped`: each WorldModel in a Folder of its own, the chain of the Folders; `holds`: what the bottom holds,
/// 20,000 "enabled" or "disabled" welds or "folders") and takes it apart from the top: the host time that took, and the work the
/// first link's move charged.
#[inline(never)]
fn nested_world_models_leave(wrapped: bool, holds: &str) -> (f64, f64) {
    let (lua, engine) = start_play();
    let reflection = engine.reflection.clone();
    let workspace = engine.world().workspace;
    let world_model_class = reflection.class_id("WorldModel").unwrap();
    let folder_class = reflection.class_id("Folder").unwrap();
    let part_class = reflection.class_id("Part").unwrap();
    let weld_class = reflection.class_id("Weld").unwrap();
    let anchored_prop = reflection.name_id("Anchored").unwrap();
    let part0_prop = reflection.name_id("Part0").unwrap();
    let part1_prop = reflection.name_id("Part1").unwrap();
    let enabled_prop = reflection.name_id("Enabled").unwrap();
    let (top, chain, holders, welds, p_id, q_id) = {
        let mut state = engine.state.borrow_mut();
        // (made out of the DataModel and moved in at once: the chain from the top down, the welds named before their parts are in
        // any world, a hundred to a folder. Wrapped, each WorldModel is in a Folder of its own, and the chain is of the Folders)
        let make_link = |state: &mut crate::State, parent: Option<crate::InstanceId>| {
            let world = crate::instance::create_named(state, &reflection, world_model_class, "WorldModel");
            if wrapped {
                let holder = crate::instance::create_named(state, &reflection, folder_class, "Folder");
                state.dom.attach(world, holder);
                if let Some(parent) = parent {
                    state.dom.attach(holder, parent);
                }
                (holder, world)
            } else {
                if let Some(parent) = parent {
                    state.dom.attach(world, parent);
                }
                (world, world)
            }
        };
        let (top, mut inner) = make_link(&mut state, None);
        let mut chain = Vec::with_capacity(2_000);
        chain.push(top);
        for _ in 1..2_000 {
            let (holder, world) = make_link(&mut state, Some(inner));
            inner = world;
            chain.push(holder);
        }
        let p_id = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
        let q_id = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
        crate::instance::raw_set_silent(&mut state, &reflection, p_id, anchored_prop, crate::value::Value::Bool(true));
        crate::instance::raw_set_silent(&mut state, &reflection, q_id, anchored_prop, crate::value::Value::Bool(true));
        let mut holders = Vec::with_capacity(200);
        let mut welds = Vec::with_capacity(if holds == "folders" { 0 } else { 20_000 });
        let enabled = holds == "enabled";
        for _ in 0..200 {
            let holder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
            for _ in 0..100 {
                if holds == "folders" {
                    let f = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                    state.dom.attach(f, holder);
                    continue;
                }
                let weld = crate::instance::create_named(&mut state, &reflection, weld_class, "Weld");
                crate::instance::raw_set_silent(&mut state, &reflection, weld, part0_prop, crate::value::Value::Instance(p_id));
                crate::instance::raw_set_silent(&mut state, &reflection, weld, part1_prop, crate::value::Value::Instance(q_id));
                crate::instance::raw_set_silent(&mut state, &reflection, weld, enabled_prop, crate::value::Value::Bool(enabled));
                state.dom.attach(weld, holder);
                super::joints::made_in_place(&mut state, &reflection, weld);
                welds.push(weld);
            }
            holders.push(holder);
        }
        state.dom.attach(p_id, inner);
        state.dom.attach(q_id, inner);
        for &holder in &holders {
            state.dom.attach(holder, inner);
        }
        (top, chain, holders, welds, p_id, q_id)
    };
    engine_lua::instance::set_parent(&engine, top, Some(workspace)).expect("top in workspace");
    {
        let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared").get("shared").expect("shared");
        let chain_tbl = lua.create_table_with_capacity(chain.len(), 0).unwrap();
        for (i, &id) in chain.iter().enumerate() {
            chain_tbl.raw_set(i + 1, engine_lua::instance_value(&lua, &engine, id).unwrap()).unwrap();
        }
        let holders_tbl = lua.create_table_with_capacity(holders.len(), 0).unwrap();
        for (i, &id) in holders.iter().enumerate() {
            holders_tbl.raw_set(i + 1, engine_lua::instance_value(&lua, &engine, id).unwrap()).unwrap();
        }
        shared.set("Chain", chain_tbl).unwrap();
        shared.set("Holders", holders_tbl).unwrap();
        shared.set("P", engine_lua::instance_value(&lua, &engine, p_id).unwrap()).unwrap();
        shared.set("Q", engine_lua::instance_value(&lua, &engine, q_id).unwrap()).unwrap();
    }
    let work = || engine.state.borrow().sched.work_total();
    let start = std::time::Instant::now();
    let before = work();
    run(&lua, &engine, "shared.Chain[1].Parent = nil");
    let first = work() - before;
    run(
        &lua,
        &engine,
        r#"
        for Index = 2, #shared.Chain do
            shared.Chain[Index].Parent = nil
        end
        "#,
    );
    let host = start.elapsed().as_secs_f64();
    if holds == "enabled" {
        run(
            &lua,
            &engine,
            r#"
            -- (the welds are as they were: moved into the Workspace with their parts, they are its joints, and all of them go)
            shared.P.Parent, shared.Q.Parent = workspace, workspace
            for _, Holder in shared.Holders do
                Holder.Parent = workspace
            end
            workspace:UnjoinFromOutsiders({ shared.P })
            "#,
        );
        let state = engine.state.borrow();
        for &weld in &welds {
            assert!(state.dom.at(weld).parent.is_none(), "a weld stayed");
        }
    }
    (host, first)
}

/// One Parent write that takes several joints out of their parts' lists of active joints, or puts several in, does it in the order
/// a Studio 0.748 client takes the moved subtree up (joints.rs taken_up): the moved instance first, then the rest from the bottom
/// up, each part's joints the last named first, and an instance the walk has not reached yet counts as where it was
/// (tools/parity/probes/unjoin_from_outsiders.luau section 6). The lists show through UnjoinFromOutsiders' order.
#[test]
fn a_move_takes_joints_out_and_puts_them_in_as_a_client_takes_its_subtree_up() {
    check_running(
        "play",
        r##"
        local Root = Instance.new("Folder")
        Root.Parent = workspace
        local Storage = game:GetService("ReplicatedStorage")
        local function Brick(Name, Y)
            local B = Instance.new("Part")
            B.Name, B.Anchored, B.CFrame, B.Parent = Name, true, CFrame.new(0, Y, 0), Root
            return B
        end
        local function Folder(Name, Parent)
            local F = Instance.new("Folder")
            F.Name, F.Parent = Name, Parent
            return F
        end
        -- (welds x1, x2, ... join B to the parts Others names, get their parts in index order or Order's, and are parented in
        -- index order, to B or where Places says; B's list is read back through UnjoinFromOutsiders({B}))
        local function Case(Others, Build)
            task.wait()
            local Q = { A = Brick("A", 1), B = Brick("B", 2), C = Brick("C", 3), D = Brick("D", 4) }
            local X = {}
            for Index = 1, #Others do
                X[Index] = Instance.new("Weld")
                X[Index].Name = "x" .. Index
            end
            local function Name(Order)
                for Position = 1, #X do
                    local Index = if Order then Order[Position] else Position
                    X[Index].Part0, X[Index].Part1 = Q.B, Q[Others[Index]]
                end
            end
            local function Put(Places)
                for Index, Weld in X do
                    if Weld.Parent == nil then
                        Weld.Parent = Places and Places[Index] or Q.B
                    end
                end
            end
            local Made = Build(X, Q, Name, Put)
            task.wait()
            local Went = {}
            for _, Weld in X do
                Weld.AncestryChanged:Connect(function(_, Parent)
                    if Parent == nil then
                        table.insert(Went, Weld.Name)
                    end
                end)
            end
            workspace:UnjoinFromOutsiders({ Q.B })
            task.wait()
            local Order = table.concat(Went, ",")
            for _, Weld in X do
                Weld:Destroy()
            end
            if Made then
                Made:Destroy()
            end
            for _, Part in Q do
                Part:Destroy()
            end
            Root:ClearAllChildren()
            return Order
        end
        local function Check(Label, Want, Got)
            assert(Got == Want, Label .. ": " .. Got .. ", not " .. Want)
        end
        -- (leaving: the part's own joints the last named first, whatever the order they were parented in)
        Check("A out, x1 x3 to A, named 3 1 2 4 5", "x5,x2,x4", Case({ "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
            Name({ 3, 1, 2, 4, 5 })
            Put()
            Q.A.Parent = Storage
            return Q.A
        end))
        -- (below the moved Folder, x1 before the part A that holds it: x1 first, then x3)
        Check("Folder [A [x1]] out, x1 x3 to A", "x5,x2,x4", Case({ "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
            local Fo = Folder("Fo", Root)
            Q.A.Parent = Fo
            Name()
            Put({ Q.A })
            Fo.Parent = Storage
            return Fo
        end))
        -- (x3 below x2: x3 first)
        Check("Folder [x2 [x3]] out", "x1,x4,x5", Case({ "A", "A", "A", "A", "A" }, function(X, Q, Name, Put)
            local Fo = Folder("Fo", Root)
            Name()
            Put({ nil, Fo })
            X[3].Parent = X[2]
            Fo.Parent = nil
        end))
        -- (x3, below A, before A's own x1)
        Check("Folder [A [x3]] out, x1 to A, x3 to C", "x5,x2,x6,x4", Case({ "A", "D", "C", "D", "D", "D" }, function(X, Q, Name, Put)
            local Fo = Folder("Fo", Root)
            Q.A.Parent = Fo
            Name()
            Put({ nil, nil, Q.A })
            Fo.Parent = Storage
            return Fo
        end))
        -- (entering: x3, below A, before A brings back x2)
        Check("Folder [A [x3]] in, x2 to A, x3 to C", "x1,x3,x2", Case({ "D", "A", "C" }, function(X, Q, Name, Put)
            local Fo = Folder("Fo", Storage)
            Q.A.Parent = Fo
            Name()
            Put({ nil, nil, Q.A })
            Fo.Parent = Root
        end))
        -- (A comes back first and brings back x2; x3, below it and not reached yet, joins after)
        Check("A [x3] in, x2 x3 to A", "x1,x2,x3", Case({ "D", "A", "A" }, function(X, Q, Name, Put)
            Q.A.Parent = Storage
            Name()
            Put({ nil, nil, Q.A })
            Q.A.Parent = Root
        end))
        Check("A [x2] in, x2 x3 to A", "x1,x3,x2", Case({ "D", "A", "A" }, function(X, Q, Name, Put)
            Q.A.Parent = Storage
            Name()
            Put({ nil, Q.A })
            Q.A.Parent = Root
        end))
        -- (x1's other part re-pointed and back: x1 still named A before x3, which leaves first; its own side re-pointed: after)
        Check("A out, x1's Part0 to D and back first", "x4,x2,x5", Case({ "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
            Name()
            X[1].Part0 = Q.D
            X[1].Part0 = Q.B
            Put()
            Q.A.Parent = Storage
            return Q.A
        end))
        Check("A out, x1's Part1 to C and back first", "x5,x2,x4", Case({ "A", "C", "A", "C", "C" }, function(X, Q, Name, Put)
            Name()
            X[1].Part1 = Q.C
            X[1].Part1 = Q.A
            Put()
            Q.A.Parent = Storage
            return Q.A
        end))
        Check("Folder [x3, Sub [x2]] in", "x1,x3,x2", Case({ "A", "A", "A" }, function(X, Q, Name, Put)
            local Fo = Folder("Fo", Storage)
            Name()
            X[3].Parent = Fo
            Put({ nil, Folder("Sub", Fo) })
            Fo.Parent = Root
            return Fo
        end))
        "##,
        1.0,
    );
}

/// A part that leaves every world takes its active joints out of their parts' lists, and the work clock pays what a Studio 0.748
/// playing client spends on that (cost.rs JOINT_TAKEN_OUT for each joint, JOINT_LIST_LEFT for each part whose list changes) beyond
/// the Parent write's own price: a part with 1,000 Glue to one other part pays 1,000 joints and 2 lists more than one with 1,000
/// disabled Glue, and one with 1,000 Glue each to a part of its own 1,000 joints and 1,001 lists more, each besides the change of
/// 1,000 links between two anchored parts, which changes no assembly (LINK_CHANGED, LINK_MORE for each further link: joints.rs
/// left_world). Joints that are not active are not looked at, and cost nothing. (A client spends as much on Glue as on Welds.)
#[test]
fn a_part_leaving_every_world_pays_for_the_joints_it_takes_out_of_their_lists() {
    check_running(
        "play",
        r##"
        local function Part(Parent)
            local B = Instance.new("Part")
            B.Anchored = true
            B.Parent = Parent
            return B
        end
        -- (P and the parts its Glue join it to in one Folder of the Workspace, the Glue in another)
        local function Leaving(Count, Mode)
            local Holder, Joints = Instance.new("Folder"), Instance.new("Folder")
            Holder.Parent, Joints.Parent = workspace, workspace
            local P, Q = Part(Holder), Part(Holder)
            for _ = 1, Count do
                local Glue = Instance.new("Glue")
                Glue.Part0, Glue.Part1 = P, if Mode == "many" then Part(Holder) else Q
                Glue.Enabled = Mode ~= "disabled"
                Glue.Parent = if Mode == "out" then nil else Joints
            end
            task.wait()
            local Start = os.clock()
            P.Parent = nil
            local Spent = os.clock() - Start
            Holder:Destroy()
            Joints:Destroy()
            return Spent
        end
        -- (within 0.1 us: the Luau around the write varies by a few ns between scenes)
        local Alone, Disabled, Out = Leaving(0, "one"), Leaving(1000, "disabled"), Leaving(1000, "out")
        assert(math.abs(Disabled - Alone) < 1e-7 and math.abs(Out - Alone) < 1e-7, string.format("%.4g disabled, %.4g out of the DataModel, %.4g alone", Disabled, Out, Alone))
        local One, Many = Leaving(1000, "one"), Leaving(1000, "many")
        local Links = 1.7e-6 + 999 * 0.06e-6
        assert(math.abs(One - Alone - (1000 * 0.25e-6 + 2 * 0.5e-6 + Links)) < 1e-7, string.format("1,000 joints to one part %.4g", One - Alone))
        assert(math.abs(Many - Alone - (1000 * 0.25e-6 + 1001 * 0.5e-6 + Links)) < 1e-7, string.format("1,000 joints to parts of their own %.4g", Many - Alone))
        "##,
        0.5,
    );
}

/// A part that leaves every world looks only at its active joints (joints.rs left_every_world: a joint that is not active cannot
/// become active by leaving), and one that enters a world only at its joints in a world: 100 parts, each named by 1,000 Welds out
/// of the DataModel, moved out of the Workspace and back 50 times while a Weld is active elsewhere, take about the host time 100
/// parts named by none take, and the work clock charges the two the same. Each part used to re-check every joint that named it
/// as it left: 100,000 checks a trip.
#[test]
fn parts_named_by_joints_out_of_the_data_model_leave_on_little_host_time() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        local function Part(Parent)
            local B = Instance.new("Part")
            B.Anchored, B.CanCollide = true, false
            B.Parent = Parent
            return B
        end
        -- (a Weld active elsewhere, so the lists of active joints are not empty)
        local Other = Instance.new("Folder")
        Other.Parent = workspace
        local Active = Instance.new("Weld")
        Active.Part0, Active.Part1 = Part(Other), Part(Other)
        Active.Parent = Other
        -- (each named part's Welds to a part of its own, out of the DataModel)
        shared.Sets, shared.Work = {}, {}
        for _, Name in { "Lone", "Named" } do
            local Holder = Instance.new("Folder")
            Holder.Parent = workspace
            local Parts = {}
            for Index = 1, 100 do
                Parts[Index] = Part(Holder)
                if Name == "Named" then
                    local Partner = Part(nil)
                    for _ = 1, 1000 do
                        local Weld = Instance.new("Weld")
                        Weld.Part0, Weld.Part1 = Parts[Index], Partner
                    end
                end
            end
            shared.Sets[Name] = { Holder = Holder, Parts = Parts }
        end
        local function Trips(Set)
            for _ = 1, 50 do
                for _, P in Set.Parts do
                    P.Parent = nil
                end
                for _, P in Set.Parts do
                    P.Parent = Set.Holder
                end
            end
        end
        function shared.Move(Name)
            local Set = shared.Sets[Name]
            Trips(Set)
            local Start = os.clock()
            Trips(Set)
            shared.Work[Name] = os.clock() - Start
        end
        shared.Active = Active
        "#,
    );
    let mut host = Vec::new();
    for name in ["Lone", "Named"] {
        let start = std::time::Instant::now();
        run(&lua, &engine, &format!("shared.Move(\"{name}\")"));
        host.push(start.elapsed().as_secs_f64());
    }
    run(&lua, &engine, r#"assert(math.abs(shared.Work.Named - shared.Work.Lone) < 1e-9, `the named parts' trips cost {shared.Work.Named} s of work where the lone ones' {shared.Work.Lone}`)"#);
    run(
        &lua,
        &engine,
        r#"
        -- (the active Weld is still in its parts' lists, and goes)
        workspace:UnjoinFromOutsiders({ shared.Active.Part0 })
        assert(shared.Active.Parent == nil, "the active Weld stayed")
        "#,
    );
    let (lone, named) = (host[0], host[1]);
    assert!(named < 2.0 * lone + 0.05, "10,000 moves took {named:.3} s of host time for parts each named by 1,000 Welds out of the DataModel, {lone:.3} s for parts named by none");
}

/// A part moved between two worlds, or out of every world and back, takes its active joints out of their parts' lists and puts them
/// back, Glue and ManualGlue among them, and the work clock pays for Glue what it pays for Welds, as a playing client spends on
/// each (Studio 0.748: a part with 1,000 or 10,000 Welds or Glue to another, moved between the Workspace and a WorldModel in it,
/// into JointsService, out of the DataModel and into ReplicatedStorage, and back, spent the same on Glue as on Welds): the rounds
/// with 20,000 Glue are charged what they are with 20,000 Welds, and the Folders holding them, carried out and back, alike, enabled
/// or disabled. 20,000
/// Glue to one part take at most twice the host time of the same 20,000 spread over 20 parts, each glued to one of its own, which
/// is the same work (0.27 s and 0.26 s here, each the faster of two runs taken in turn; a joint looked for in each list before it
/// left it, 3.1 s and 0.65 s).
#[test]
fn glue_moved_between_worlds_pays_for_its_list_changes() {
    let rigs = [glue_between_worlds(1), glue_between_worlds(20)];
    let mut host = [f64::INFINITY; 2];
    for _ in 0..2 {
        for (arm, (lua, engine)) in rigs.iter().enumerate() {
            let start = std::time::Instant::now();
            run(lua, engine, "shared.Move(\"Glued\")");
            host[arm] = host[arm].min(start.elapsed().as_secs_f64());
        }
    }
    let [one, spread] = host;
    assert!(one < 2.0 * spread + 0.05, "10 rounds of moves took {one:.3} s of host time with 20,000 Glue to one part, {spread:.3} s with them spread over 20");
}

/// An engine where `parts` parts move together, 20,000 Glue spread over them, each part glued to one of its own, between the
/// Workspace, a WorldModel in it and JointsService, and out of the DataModel and into ReplicatedStorage and back, 10 rounds a
/// `shared.Move("Glued")`. Checks the work those rounds are charged beyond the same parts' moves with no joint, and what the Folder
/// holding the Glue, and Folders of as many disabled Glue and Welds, are charged carried out of the DataModel and back.
#[inline(never)]
fn glue_between_worlds(parts: usize) -> (Lua, Rc<Engine>) {
    let count = 20_000usize;
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let reflection = engine.reflection.clone();
    let workspace = engine.world().workspace;
    let world_model_class = reflection.class_id("WorldModel").unwrap();
    let folder_class = reflection.class_id("Folder").unwrap();
    let model_class = reflection.class_id("Model").unwrap();
    let part_class = reflection.class_id("Part").unwrap();
    let glue_class = reflection.class_id("Glue").unwrap();
    let weld_class = reflection.class_id("Weld").unwrap();
    let anchored_prop = reflection.name_id("Anchored").unwrap();
    let collide_prop = reflection.name_id("CanCollide").unwrap();
    let part0_prop = reflection.name_id("Part0").unwrap();
    let part1_prop = reflection.name_id("Part1").unwrap();
    let enabled_prop = reflection.name_id("Enabled").unwrap();

    let world_id = {
        let mut state = engine.state.borrow_mut();
        crate::instance::create_named(&mut state, &reflection, world_model_class, "WorldModel")
    };
    engine_lua::instance::set_parent(&engine, world_id, Some(workspace)).expect("world in workspace");

    let mut sets: Vec<(&str, crate::InstanceId, crate::InstanceId, crate::InstanceId)> = Vec::with_capacity(4);
    for name in ["Lone", "Glued", "Welded", "Idle", "IdleWelds"] {
        let (holder, moving, joints) = {
            let mut state = engine.state.borrow_mut();
            let holder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
            let moving = crate::instance::create_named(&mut state, &reflection, model_class, "Model");
            let joints = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
            state.dom.attach(moving, holder);
            for _ in 0..parts {
                let p = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
                let q = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
                crate::instance::raw_set_silent(&mut state, &reflection, p, anchored_prop, crate::value::Value::Bool(true));
                crate::instance::raw_set_silent(&mut state, &reflection, p, collide_prop, crate::value::Value::Bool(false));
                crate::instance::raw_set_silent(&mut state, &reflection, q, anchored_prop, crate::value::Value::Bool(true));
                crate::instance::raw_set_silent(&mut state, &reflection, q, collide_prop, crate::value::Value::Bool(false));
                state.dom.attach(p, moving);
                state.dom.attach(q, holder);
                if name != "Lone" {
                    let (jclass, jname) = if name == "IdleWelds" || name == "Welded" { (weld_class, "Weld") } else { (glue_class, "Glue") };
                    let enabled = name == "Glued" || name == "Welded";
                    for _ in 0..(count / parts) {
                        let joint = crate::instance::create_named(&mut state, &reflection, jclass, jname);
                        crate::instance::raw_set_silent(&mut state, &reflection, joint, part0_prop, crate::value::Value::Instance(p));
                        crate::instance::raw_set_silent(&mut state, &reflection, joint, part1_prop, crate::value::Value::Instance(q));
                        crate::instance::raw_set_silent(&mut state, &reflection, joint, enabled_prop, crate::value::Value::Bool(enabled));
                        state.dom.attach(joint, joints);
                        super::joints::made_in_place(&mut state, &reflection, joint);
                    }
                }
            }
            (holder, moving, joints)
        };
        engine_lua::instance::set_parent(&engine, holder, Some(workspace)).expect("holder in workspace");
        engine_lua::instance::set_parent(&engine, joints, Some(holder)).expect("joints in holder");
        sets.push((name, holder, moving, joints));
    }
    {
        let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared").get("shared").expect("shared");
        shared.set("World", engine_lua::instance_value(&lua, &engine, world_id).unwrap()).unwrap();
        let sets_tbl = lua.create_table_with_capacity(0, 5).unwrap();
        for (name, holder, moving, joints) in sets {
            let entry = lua.create_table_with_capacity(0, 3).unwrap();
            entry.raw_set("Holder", engine_lua::instance_value(&lua, &engine, holder).unwrap()).unwrap();
            entry.raw_set("Moving", engine_lua::instance_value(&lua, &engine, moving).unwrap()).unwrap();
            entry.raw_set("Joints", engine_lua::instance_value(&lua, &engine, joints).unwrap()).unwrap();
            sets_tbl.raw_set(name, entry).unwrap();
        }
        shared.set("Sets", sets_tbl).unwrap();
        shared.set("Work", lua.create_table().unwrap()).unwrap();
    }
    run(
        &lua,
        &engine,
        r#"
        local World = shared.World
        local Service, Storage = game:GetService("JointsService"), game:GetService("ReplicatedStorage")
        function shared.Move(Name)
            local Set = shared.Sets[Name]
            local Start = os.clock()
            for _ = 1, 10 do
                Set.Moving.Parent = World
                Set.Moving.Parent = Set.Holder
                Set.Moving.Parent = Service
                Set.Moving.Parent = Set.Holder
                Set.Moving.Parent = nil
                Set.Moving.Parent = Set.Holder
                Set.Moving.Parent = Storage
                Set.Moving.Parent = Set.Holder
            end
            shared.Work[Name] = os.clock() - Start
        end
        function shared.Carry(Name)
            local Set = shared.Sets[Name]
            local Start = os.clock()
            Set.Joints.Parent = nil
            Set.Joints.Parent = Set.Holder
            return os.clock() - Start
        end
        "#,
    );
    run(&lua, &engine, "shared.Move(\"Lone\")");
    run(&lua, &engine, "shared.Move(\"Glued\")");
    run(&lua, &engine, "shared.Move(\"Welded\")");
    // (a client spends as much on Glue as on Welds on each of these moves: the rounds with COUNT Glue cost what they cost with COUNT
    // Welds, and the Folders holding them, carried out and back, alike, enabled or disabled)
    run(
        &lua,
        &engine,
        &r#"
        local N = COUNT
        local Glued, Welded = shared.Work.Glued - shared.Work.Lone, shared.Work.Welded - shared.Work.Lone
        assert(Glued > 0 and math.abs(Glued - Welded) < 1e-7 * (1 + N / 1000), `10 rounds charged {Glued} s more than the lone parts with Glue, {Welded} s with Welds`)
        local Idle, IdleWelds = shared.Carry("Idle"), shared.Carry("IdleWelds")
        assert(math.abs(Idle - IdleWelds) < 1e-7 * (1 + N / 1000), `a Folder of disabled Glue carried out and back charged {Idle} s, of disabled Welds {IdleWelds} s`)
        local Carried, CarriedWelds = shared.Carry("Glued"), shared.Carry("Welded")
        assert(math.abs(Carried - CarriedWelds) < 1e-7 * (1 + N / 1000), `the Glue's Folder charged {Carried} s, the Welds' {CarriedWelds} s`)
        "#
        .replace("COUNT", &count.to_string()),
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
    (lua, engine)
}

/// An AssemblyRootPart read right after UnjoinFromOutsiders answers as a fresh walk does: the reads before it keep the assembly
/// (world/joints.rs Component), and each joint UnjoinFromOutsiders takes out drops what was kept, as any joint taken out does.
#[test]
fn an_assembly_read_after_unjoin_from_outsiders_answers_as_a_fresh_walk() {
    let script = r#"
        local function Part(Name, Anchored)
            local B = Instance.new("Part")
            B.Name, B.Anchored, B.CanCollide = Name, Anchored, false
            B.Parent = workspace
            return B
        end
        local A, B, C, D = Part("A", true), Part("B", false), Part("C", false), Part("D", false)
        local function Weld(P0, P1)
            local W = Instance.new("Weld")
            W.Part0, W.Part1 = P0, P1
            W.Parent = workspace
        end
        Weld(A, B)
        Weld(B, C)
        Weld(C, D)
        local function Roots(Label)
            print(Label, B.AssemblyRootPart.Name, C.AssemblyRootPart.Name, D.AssemblyRootPart.Name)
        end
        Roots("before")
        workspace:UnjoinFromOutsiders({ C, D })
        Roots("C and D unjoined")
        workspace:UnjoinFromOutsiders({ B })
        Roots("B unjoined")
    "#;
    let mut logs: Vec<Vec<String>> = Vec::new();
    for keeping in [true, false] {
        super::joints::set_keeping(keeping);
        let (lua, engine) = start_play();
        run(&lua, &engine, script);
        logs.push(engine.state.borrow().output.entries.iter().map(|line| line.message.to_string()).collect());
    }
    super::joints::set_keeping(true);
    assert_eq!(logs[0], ["before A A A", "C and D unjoined A D D", "B unjoined B D D"], "what the kept assemblies answered");
    assert_eq!(logs[0], logs[1], "what the kept assemblies answered, then what fresh walks did");
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

/// Of several anchored parts, each unanchored part hangs from the one a playing client gives it (Studio 0.747, client and server,
/// tools/parity/probes/assembly_roots.luau): its links taken a Weld, Snap or Glue before a WeldConstraint, a WeldConstraint before a
/// Motor, a Motor before a Motor6D, of one kind the later made first, each joining its two sides unless both already hang from an
/// anchored part. So the order joints were made in decides, not the order they were parented or given their parts in.
#[test]
fn a_part_linked_to_several_anchored_parts_hangs_from_the_one_a_client_picks() {
    check(
        r#"
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        local function Part(Name, Anchored)
            local P = Instance.new("Part")
            P.Name, P.Anchored, P.CanCollide = Name, Anchored, false
            P.Parent = Folder
            return P
        end
        local function Join(Kind, A, B, Parent)
            local J = Instance.new(Kind)
            J.Part0, J.Part1 = A, B
            if Parent ~= false then
                J.Parent = Parent or A
            end
            return J
        end
        local function Root(P)
            local R = P.AssemblyRootPart
            return R and R.Name or "nil"
        end
        local function Is(Case, Parts, Want)
            local Got = {}
            for _, P in Parts do
                table.insert(Got, Root(P))
            end
            assert(table.concat(Got, " ") == Want, `{Case}: {table.concat(Got, " ")}, the client's {Want}`)
        end
        -- welds made A, B, C, parented C, B, A: the last made, C, whichever came last
        local U, A, B, C = Part("U", false), Part("A", true), Part("B", true), Part("C", true)
        local WA, WB, WC = Join("Weld", U, A, false), Join("Weld", U, B, false), Join("Weld", U, C, false)
        WC.Parent = U
        WB.Parent = U
        WA.Parent = U
        Is("parented last first", { U }, "C")
        WC.Enabled = false
        Is("the last made disabled", { U }, "B")
        WC.Enabled = true
        Is("enabled again", { U }, "C")
        WA.Part1 = nil
        WA.Part1 = A
        Is("the first re-pointed", { U }, "C")
        Folder:ClearAllChildren()
        -- a chain between two anchored parts hangs whole from the end its last made weld reaches; a bridge hangs one from each end
        local A1, U1, U2, B1 = Part("A", true), Part("U1", false), Part("U2", false), Part("B", true)
        Join("Weld", A1, U1)
        Join("Weld", U1, U2)
        Join("Weld", U2, B1)
        Is("chain", { U1, U2 }, "B B")
        Folder:ClearAllChildren()
        A1, U1, U2, B1 = Part("A", true), Part("U1", false), Part("U2", false), Part("B", true)
        Join("Weld", U1, U2)
        Join("Weld", U2, B1)
        Join("Weld", U1, A1)
        Is("bridge", { U1, U2 }, "A B")
        Folder:ClearAllChildren()
        -- the kinds' order: a Weld made first beats a WeldConstraint, a WeldConstraint a Motor, a Motor a Motor6D; Glue is a Weld
        for _, Case in { { "Weld", "WeldConstraint", "A" }, { "WeldConstraint", "Motor", "A" }, { "Motor", "Motor6D", "A" }, { "Motor6D", "Motor", "B" }, { "Weld", "Glue", "B" }, { "Snap", "ManualWeld", "B" }, { "Rotate", "Weld", "B" }, { "Weld", "VelocityMotor", "A" } } do
            local P, First, Second = Part("U", false), Part("A", true), Part("B", true)
            Join(Case[1], P, First)
            Join(Case[2], P, Second)
            Is(`{Case[1]} then {Case[2]}`, { P }, Case[3])
            Folder:ClearAllChildren()
        end
        -- a free star whose fifth and first leaves are anchored, in either order: the fifth, its weld the last made
        for _, Order in { { 5, 1 }, { 1, 5 } } do
            local Hub = Part("H", false)
            local Leaves = {}
            for Index = 1, 5 do
                Leaves[Index] = Part("L" .. Index, false)
                Join("Weld", Hub, Leaves[Index])
            end
            for _, Index in Order do
                Leaves[Index].Anchored = true
            end
            Is(`star anchored {Order[1]}, {Order[2]}`, { Hub, Leaves[2], Leaves[1] }, "L5 L5 L1")
            Folder:ClearAllChildren()
        end
        -- a clone's welds, made in the order the clone made them: each under its anchored part, the parts made U, C, B, A
        local Model = Instance.new("Model")
        local CU = Instance.new("Part")
        CU.Name, CU.CanCollide, CU.Parent = "U", false, Model
        for _, Name in { "C", "B", "A" } do
            local P = Instance.new("Part")
            P.Name, P.Anchored, P.CanCollide, P.Parent = Name, true, false, Model
        end
        for _, Name in { "A", "B", "C" } do
            Join("Weld", CU, Model[Name], Model[Name])
        end
        local Copy = Model:Clone()
        Model.Parent = Folder
        Copy.Parent = Folder
        Is("the original", { Model.U }, "C")
        Is("its clone", { Copy.U }, "A")
        "#,
    );
}

/// Glue and ManualGlue link and place their parts as a Weld does, as on a playing client (Studio 0.747, client and server,
/// tools/parity/probes/assembly_roots.luau): a part glued to an anchored part reads it as its AssemblyRootPart, takes the place C0
/// and C1 give it, moves with it, and adds to its assembly's mass; disabled, the Glue links nothing.
#[test]
fn glue_links_and_places_its_parts_as_a_weld_does() {
    check(
        r#"
        for _, Kind in { "Glue", "ManualGlue" } do
            local A, B = Instance.new("Part"), Instance.new("Part")
            A.Name, B.Name = "A", "B"
            A.Size, B.Size = Vector3.new(2, 2, 2), Vector3.new(1, 1, 1)
            A.Anchored, A.CanCollide, B.CanCollide = true, false, false
            A.CFrame = CFrame.new(10, 20, 30)
            A.Parent, B.Parent = workspace, workspace
            local G = Instance.new(Kind)
            G.Part0, G.Part1 = A, B
            G.C0 = CFrame.new(0, 4, 0)
            G.Parent = A
            assert(B.AssemblyRootPart == A, `{Kind}: B's root {B.AssemblyRootPart}`)
            assert((B.Position - Vector3.new(10, 24, 30)).Magnitude < 1e-5, `{Kind}: B at {B.Position}`)
            G.Enabled = false
            assert(B.AssemblyRootPart == B, `{Kind}: disabled`)
            G.Enabled = true
            A.Anchored = false
            assert(B.AssemblyRootPart == A and math.abs(B.AssemblyMass - (A.Mass + B.Mass)) < 1e-5, `{Kind}: free, mass {B.AssemblyMass}`)
            A.CFrame = A.CFrame * CFrame.new(0, 3, 0)
            assert((B.Position - Vector3.new(10, 27, 30)).Magnitude < 1e-5, `{Kind}: moved with A, at {B.Position}`)
            A:Destroy()
            B:Destroy()
        end
        "#,
    );
}

/// A JointInstance in JointsService links two parts of the Workspace's own world, however they came in, and no two of a WorldModel
/// (one in the Workspace too); a WeldConstraint in JointsService links nothing: as on a playing client (Studio 0.747, client and
/// server, tools/parity/probes/assembly_roots.luau).
#[test]
fn joints_service_links_joint_instances_between_parts_of_the_workspace() {
    check(
        r#"
        local JS = game:GetService("JointsService")
        local World = Instance.new("WorldModel")
        World.Parent = workspace
        local function Case(Kind, Where, Late)
            local A, B = Instance.new("Part"), Instance.new("Part")
            A.Anchored, A.CanCollide, B.CanCollide = true, false, false
            A.Name, B.Name = "A", "B"
            if not Late then
                A.Parent, B.Parent = Where, Where
            end
            local J = Instance.new(Kind)
            J.Part0, J.Part1 = A, B
            J.Parent = JS
            if Late then
                A.Parent, B.Parent = Where, Where
            end
            local Linked = B.AssemblyRootPart == A
            J:Destroy()
            A:Destroy()
            B:Destroy()
            return Linked
        end
        for _, Kind in { "Weld", "Motor6D", "Snap", "Glue" } do
            assert(Case(Kind, workspace), `{Kind} in JointsService between parts of the Workspace`)
            assert(Case(Kind, workspace, true), `{Kind} in JointsService, the parts entering after it`)
            assert(not Case(Kind, World), `{Kind} in JointsService between parts of a WorldModel`)
            assert(not Case(Kind, World, true), `{Kind} in JointsService, the parts entering a WorldModel`)
        end
        assert(not Case("WeldConstraint", workspace), "a WeldConstraint in JointsService")
        assert(not Case("WeldConstraint", workspace, true), "a WeldConstraint in JointsService, the parts entering after it")
        "#,
    );
}

/// An assembly reads its motion from its root's body, as a playing client keeps it (Studio 0.747, client and server,
/// tools/parity/probes/assembly_motion.luau): parts a weld joins take the root's velocity and spin, the joined centre's velocity with
/// them; a piece split off reads what its new root's body holds (nothing, in the frame a write set the motion of a root it hung
/// from); a part anchored in a moving assembly gives it the motion a join left in it (each part from the root to the end a joint
/// joined a part at); a part moved within it reads its velocity at its new place.
#[test]
fn an_assembly_reads_its_motion_from_its_roots_body() {
    check(
        r#"
        local function Part(Name, X, Size)
            local P = Instance.new("Part")
            P.Name, P.Size, P.CanCollide = Name, Size or Vector3.one, false
            P.CFrame = CFrame.new(300 + X, 60, 40)
            P.Parent = workspace
            return P
        end
        local function Weld(A, B)
            local W = Instance.new("Weld")
            W.Part0, W.Part1 = A, B
            W.C0 = A.CFrame:ToObjectSpace(B.CFrame)
            W.Parent = A
            return W
        end
        local function Is(Case, P, Velocity, Spin)
            assert(P.Velocity == Velocity and P.RotVelocity == Spin and P.AssemblyAngularVelocity == Spin, `{Case}: {P.Name} {P.Velocity} {P.RotVelocity}, the client's {Velocity} {Spin}`)
        end
        -- a join: Y takes X's motion, whatever it had
        local X, Y = Part("X", 0, Vector3.new(2, 2, 2)), Part("Y", 4)
        X.AssemblyLinearVelocity, X.AssemblyAngularVelocity = Vector3.new(1, 0, 0), Vector3.new(0, 0.5, 0)
        Y.AssemblyLinearVelocity, Y.AssemblyAngularVelocity = Vector3.new(0, 0, 3), Vector3.new(0, 1, 0)
        Weld(X, Y)
        Is("join", Y, Vector3.new(1, 0, -2), Vector3.new(0, 0.5, 0))
        Is("join", X, Vector3.new(1, 0, 0), Vector3.new(0, 0.5, 0))
        assert(Y.AssemblyLinearVelocity == X.AssemblyLinearVelocity and Y.AssemblyLinearVelocity ~= Vector3.new(1, 0, 0), `the joined centre's velocity {Y.AssemblyLinearVelocity}`)
        X:Destroy()
        Y:Destroy()
        -- X moving alone, then Y and Z welded on (X-Y, Y-Z): the Y-Z join leaves Y's motion in Y, none in Z
        local function Trio()
            local X = Part("X", 0, Vector3.new(2, 2, 2))
            X.AssemblyLinearVelocity, X.AssemblyAngularVelocity = Vector3.new(1, 0, 0), Vector3.new(0, 1, 0)
            local Y, Z = Part("Y", 2), Part("Z", 4)
            return X, Y, Z, Weld(X, Y), Weld(Y, Z)
        end
        local Y2, Z2, XY, YZ
        X, Y2, Z2, XY, YZ = Trio()
        Is("joined", Z2, Vector3.new(1, 0, -4), Vector3.new(0, 1, 0))
        YZ.Enabled = false
        Is("split off", Z2, Vector3.zero, Vector3.zero)
        assert(Z2.AssemblyLinearVelocity == Vector3.zero, "the piece's centre")
        Is("the root's piece", X, Vector3.new(1, 0, 0), Vector3.new(0, 1, 0))
        Is("the root's piece", Y2, Vector3.new(1, 0, -2), Vector3.new(0, 1, 0))
        YZ.Enabled = true
        Is("joined again", Z2, Vector3.new(1, 0, -4), Vector3.new(0, 1, 0))
        Y2.Anchored = true
        Is("re-rooted", X, Vector3.new(1, 0, 0), Vector3.new(0, 1, 0))
        Is("re-rooted", Z2, Vector3.new(1, 0, -4), Vector3.new(0, 1, 0))
        Y2.Anchored = false
        Z2.Position += Vector3.new(2, 0, 0)
        Is("moved within", Z2, Vector3.new(1, 0, -6), Vector3.new(0, 1, 0))
        X:Destroy()
        Y2:Destroy()
        Z2:Destroy()
        -- the trio's X-Y weld off: the piece {Y, Z} hangs from Z, which held no motion
        X, Y2, Z2, XY, YZ = Trio()
        XY.Enabled = false
        Is("the piece's root", Z2, Vector3.zero, Vector3.zero)
        Is("the piece's root", Y2, Vector3.zero, Vector3.zero)
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
            Is(`{Depth}: the weld in JointsService, its parts in the WorldModel`, A, B, "A B")
            Weld.Parent = World
            Is(`{Depth}: the weld in the WorldModel`, A, B, "B B")
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
    register_wrap(&lua, &engine);
    run(
        &lua,
        &engine,
        r#"
        -- (a chain of folders `Depth` deep holding `Inner`, built directly in the dom so making it walks nothing)
        local Wrap = shared.Wrap
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
    let use_scene = compile(&lua, &engine, "local Name = ...; shared.Use(Name)");
    let mut host = [0.0; 3];
    for _ in 0..3 {
        for (index, name) in ["Shallow", "Deep", "Apart"].into_iter().enumerate() {
            let start = std::time::Instant::now();
            call(&lua, &engine, &use_scene, name);
            host[index] = start.elapsed().as_secs_f64();
        }
        if host[1] < 2.0 * host[0] + 0.05 && host[2] < 2.0 * host[0] + 0.05 {
            break;
        }
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
    let [shallow, deep, apart] = host;
    assert!(deep < 2.0 * shallow + 0.05 && apart < 2.0 * shallow + 0.05, "two passes of 1,040 lookups took {deep:.3} s 10,000 deep and {apart:.3} s 4,000 deep apart, {shallow:.3} s in the Workspace");
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
    let index_fn = compile(&lua, &engine, "local N = ...; shared.Index(N)");
    let per_weld = |count: usize| {
        let start = std::time::Instant::now();
        call(&lua, &engine, &index_fn, count);
        start.elapsed().as_secs_f64() / count as f64
    };
    per_weld(2_000);
    let (mut few, mut many) = (0.0, 0.0);
    for _ in 0..3 {
        (few, many) = (per_weld(2_000), per_weld(60_000));
        if many < 3.0 * few {
            break;
        }
    }
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
/// linked) and a later link change's price (its assembly charged already in the frame, the frame's first link it starts after one
/// it ended, after the frame's first join: LINK_REJOINED_AFTER_JOIN) with each link beyond the first (LINK_MORE), 1,000 outside
/// every world nothing, and 1,000
/// carried in under it at least 0.4 us each (looked at from both parts and
/// brought into the world, besides what the write pays for carrying them), into the Workspace and into a WorldModel alike. The
/// write used to cost the same with any number of joints.
#[test]
fn a_part_entering_a_world_pays_for_its_joints() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let (looked, linked, entered, changed, rejoined, more) = (crate::cost::JOINT_LOOKED_AT, crate::cost::JOINT_LINKED, crate::cost::JOINT_ENTERED, crate::cost::LINK_CHANGED, crate::cost::LINK_REJOINED_AFTER_JOIN, crate::cost::LINK_MORE);
    run(
        &lua,
        &engine,
        &format!(
            r#"
        local LOOKED, LINKED, ENTERED, CHANGED, REJOINED, MORE, N = {looked:e}, {linked:e}, {entered:e}, {changed:e}, {rejoined:e}, {more:e}, 1000
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
                -- (each kind in a frame of its own: the moves' link changes are the frame's)
                task.wait()
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
            Is("linked", N * (2 * LOOKED + LINKED) + CHANGED + REJOINED + (N - 1) * MORE)
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
    let reflection = engine.reflection.clone();
    let part_class = reflection.class_id("Part").unwrap();
    let weld_class = reflection.class_id("Weld").unwrap();
    let anchored_prop = reflection.name_id("Anchored").unwrap();
    let collide_prop = reflection.name_id("CanCollide").unwrap();
    let part0_prop = reflection.name_id("Part0").unwrap();
    let part1_prop = reflection.name_id("Part1").unwrap();
    let (lone_id, named_id) = {
        let mut state = engine.state.borrow_mut();
        let make_part = |state: &mut crate::State| {
            let p = crate::instance::create_named(state, &reflection, part_class, "Part");
            crate::instance::raw_set_silent(state, &reflection, p, anchored_prop, crate::value::Value::Bool(true));
            crate::instance::raw_set_silent(state, &reflection, p, collide_prop, crate::value::Value::Bool(false));
            p
        };
        let lone = make_part(&mut state);
        let named = make_part(&mut state);
        for _ in 0..50_000 {
            let other = make_part(&mut state);
            let w = crate::instance::create_named(&mut state, &reflection, weld_class, "Weld");
            crate::instance::raw_set_silent(&mut state, &reflection, w, part0_prop, crate::value::Value::Instance(named));
            crate::instance::raw_set_silent(&mut state, &reflection, w, part1_prop, crate::value::Value::Instance(other));
            super::joints::made_in_place(&mut state, &reflection, w);
        }
        (lone, named)
    };
    {
        let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared").get("shared").expect("shared");
        let parts = lua.create_table().unwrap();
        parts.set("Lone", engine_lua::instance_value(&lua, &engine, lone_id).unwrap()).unwrap();
        parts.set("Named", engine_lua::instance_value(&lua, &engine, named_id).unwrap()).unwrap();
        shared.set("Parts", parts).unwrap();
    }
    run(
        &lua,
        &engine,
        r#"
        local World = Instance.new("WorldModel")
        World.Parent = game:GetService("ReplicatedStorage")
        shared.Work = {}
        local function Moves(P)
            for _ = 1, 400 do
                P.Parent = workspace
                P.Parent = nil
                P.Parent = World
                P.Parent = nil
            end
        end
        function shared.Warm(Name)
            Moves(shared.Parts[Name])
        end
        function shared.Move(Name)
            local P = shared.Parts[Name]
            local Start = os.clock()
            Moves(P)
            shared.Work[Name] = os.clock() - Start
        end
        "#,
    );
    let warm = compile(&lua, &engine, "local Name = ...; shared.Warm(Name)");
    let move_fn = compile(&lua, &engine, "local Name = ...; shared.Move(Name)");
    call(&lua, &engine, &warm, "Lone");
    call(&lua, &engine, &warm, "Named");
    let (mut lone, mut named) = (0.0, 0.0);
    for _ in 0..3 {
        let start = std::time::Instant::now();
        call(&lua, &engine, &move_fn, "Lone");
        lone = start.elapsed().as_secs_f64();
        let start = std::time::Instant::now();
        call(&lua, &engine, &move_fn, "Named");
        named = start.elapsed().as_secs_f64();
        if named < 2.0 * lone + 0.05 {
            break;
        }
    }
    // (os.clock reads about 10,000 s, where its last bit is 1.8e-12 s)
    run(&lua, &engine, r#"assert(math.abs(shared.Work.Named - shared.Work.Lone) < 1e-11, `the named part's moves cost {shared.Work.Named} s of work where the lone one's {shared.Work.Lone}`)"#);
    assert!(named < 2.0 * lone + 0.05, "1,600 moves took {named:.3} s of host time for a part named by 50,000 welds outside every world, {lone:.3} s for one named by none");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// A part outside every world named by joints that are in one moves between places outside every world without working its joints
/// out again, as a client's move costs nothing for them (Studio 0.747, a playing client: a part in ReplicatedStorage named by 100 to
/// 10,000 welds in the Workspace, enabled or not, moved to JointsService and back, or to nil and back, 38 to 53 us, as one named by
/// none): 200 rounds of a part named by 50,000 such welds moved to JointsService and back take about the host time of a part named
/// by none, the two parts' rounds in turn (up to three tries), and the work clock charges the two the same. Each move worked each
/// joint out again (0.4 to 1.5 ms a move at 10,000).
#[test]
fn a_part_outside_every_world_moves_without_working_its_joints_out() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let reflection = engine.reflection.clone();
    let storage = crate::services::get_service(&engine, engine.world().game, "ReplicatedStorage").expect("ReplicatedStorage");
    let workspace = engine.world().workspace;
    let folder_class = reflection.class_id("Folder").unwrap();
    let part_class = reflection.class_id("Part").unwrap();
    let weld_class = reflection.class_id("Weld").unwrap();
    let anchored_prop = reflection.name_id("Anchored").unwrap();
    let collide_prop = reflection.name_id("CanCollide").unwrap();
    let cframe_prop = reflection.name_id("CFrame").unwrap();
    let part0_prop = reflection.name_id("Part0").unwrap();
    let part1_prop = reflection.name_id("Part1").unwrap();
    let enabled_prop = reflection.name_id("Enabled").unwrap();
    let cf_val = crate::value::Value::Dom(Rc::new(rbx_dom_weak::types::Variant::CFrame(
        rbx_dom_weak::types::CFrame::new(rbx_dom_weak::types::Vector3::new(0.0, 300.0, 0.0), rbx_dom_weak::types::Matrix3::identity()),
    )));
    let (lone_id, named_id, box_id) = {
        let mut state = engine.state.borrow_mut();
        let make_part = |state: &mut crate::State, anchored: bool, parent: crate::InstanceId| {
            let p = crate::instance::create_named(state, &reflection, part_class, "Part");
            crate::instance::raw_set_silent(state, &reflection, p, anchored_prop, crate::value::Value::Bool(anchored));
            crate::instance::raw_set_silent(state, &reflection, p, collide_prop, crate::value::Value::Bool(false));
            crate::instance::raw_set_silent(state, &reflection, p, cframe_prop, cf_val.clone());
            state.dom.attach(p, parent);
            p
        };
        let lone = make_part(&mut state, false, storage);
        let named = make_part(&mut state, false, storage);
        let box_folder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
        for index in 1..=50_000usize {
            let other = make_part(&mut state, true, box_folder);
            let w = crate::instance::create_named(&mut state, &reflection, weld_class, "Weld");
            crate::instance::raw_set_silent(&mut state, &reflection, w, part0_prop, crate::value::Value::Instance(named));
            crate::instance::raw_set_silent(&mut state, &reflection, w, part1_prop, crate::value::Value::Instance(other));
            crate::instance::raw_set_silent(&mut state, &reflection, w, enabled_prop, crate::value::Value::Bool(index.is_multiple_of(2)));
            state.dom.attach(w, box_folder);
            super::joints::made_in_place(&mut state, &reflection, w);
        }
        (lone, named, box_folder)
    };
    engine_lua::instance::set_parent(&engine, box_id, Some(workspace)).expect("box in workspace");
    {
        let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared").get("shared").expect("shared");
        let parts = lua.create_table().unwrap();
        parts.set("Lone", engine_lua::instance_value(&lua, &engine, lone_id).unwrap()).unwrap();
        parts.set("Named", engine_lua::instance_value(&lua, &engine, named_id).unwrap()).unwrap();
        shared.set("Parts", parts).unwrap();
    }
    run(
        &lua,
        &engine,
        r#"
        local Storage, Joints = game:GetService("ReplicatedStorage"), game:GetService("JointsService")
        shared.Work = {}
        function shared.Move(Name)
            local P = shared.Parts[Name]
            local Start = os.clock()
            for _ = 1, 200 do
                P.Parent = Joints
                P.Parent = Storage
            end
            shared.Work[Name] = os.clock() - Start
        end
        "#,
    );
    let move_fn = compile(&lua, &engine, "local Name = ...; shared.Move(Name)");
    let time = |name: &str| {
        let start = std::time::Instant::now();
        call(&lua, &engine, &move_fn, name);
        start.elapsed().as_secs_f64()
    };
    time("Lone");
    time("Named");
    // (the two parts' rounds in turn, so whatever else the host does weighs on both alike; up to three tries)
    let (mut lone, mut named) = (0.0, 0.0);
    for _ in 0..3 {
        (lone, named) = (0.0, 0.0);
        for _ in 0..3 {
            lone += time("Lone");
            named += time("Named");
        }
        if named < 2.0 * lone + 0.05 {
            break;
        }
    }
    // (os.clock reads about 10,000 s, where its last bit is 1.8e-12 s)
    run(&lua, &engine, r#"assert(math.abs(shared.Work.Named - shared.Work.Lone) < 1e-11, `the named part's moves cost {shared.Work.Named} s of work where the lone one's {shared.Work.Lone}`)"#);
    assert!(named < 2.0 * lone + 0.05, "3 x 400 moves took {named:.3} s of host time for a part named by 50,000 welds in the Workspace, {lone:.3} s for one named by none");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
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

/// A part whose only active joint is a Glue, moved out of every world straight through the dom (no Parent
/// write) takes the Glue out of its parts' lists of active joints at the next look (joints.rs take_homes queues the joints of a part
/// that holds some in its list, list_logged), as a part whose joints link does.
#[test]
fn glue_leaves_the_lists_when_the_dom_moves_its_part_out_of_every_world() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r#"
        local A, B = Instance.new("WedgePart"), Instance.new("Part")
        A.Anchored, B.Anchored = true, true
        A.Parent, B.Parent = workspace, workspace
        local Glue = Instance.new("Glue")
        Glue.Part0, Glue.Part1 = A, B
        Glue.Parent = workspace
        "#,
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index, the Glue listed");
    {
        let mut state = engine.state.borrow_mut();
        let wedge = engine.reflection.class_id("WedgePart").expect("class");
        let part = state.dom.instances_of_class(wedge)[0];
        state.dom.reparent(part, None);
    }
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index, the Glue out of the lists");
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
    let reads_fn = compile(&lua, &engine, "local Name = ...; shared.Reads(Name)");
    let writes_fn = compile(&lua, &engine, "local Name = ...; shared.Writes(Name)");
    let time_call = |func: &LuaFunction, name: &str| {
        let start = std::time::Instant::now();
        call(&lua, &engine, func, name);
        start.elapsed().as_secs_f64()
    };
    call(&lua, &engine, &reads_fn, "Leaf");
    call(&lua, &engine, &reads_fn, "Lone");
    let (mut leaf, mut lone) = (0.0, 0.0);
    for _ in 0..3 {
        (leaf, lone) = (time_call(&reads_fn, "Leaf"), time_call(&reads_fn, "Lone"));
        if leaf < 2.0 * lone + 0.05 {
            break;
        }
    }
    assert!(leaf < 2.0 * lone + 0.05, "6,000 reads took {leaf:.3} s of host time for a part of an assembly of 2,001, {lone:.3} s for a lone part");
    call(&lua, &engine, &writes_fn, "Named");
    call(&lua, &engine, &writes_fn, "Lone");
    let (mut named, mut lone) = (0.0, 0.0);
    for _ in 0..3 {
        (named, lone) = (time_call(&writes_fn, "Named"), time_call(&writes_fn, "Lone"));
        if named < 2.0 * lone + 0.05 {
            break;
        }
    }
    assert!(named < 2.0 * lone + 0.05, "8,000 writes and reads took {named:.3} s of host time for a part named by 10,000 welds that cannot link, {lone:.3} s for a lone part");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// Writing C0, C1, Enabled or Part0/Part1 on a weld that does not link two parts (unparented, disabled or one-sided)
/// cannot change any assembly and must not settle the hub's assembly: host time on a 5,001-part assembly matches a
/// 2-part one, work-clock charges match, pending Motor6D transforms stay deferred until the step, and active welds
/// still place Part1 on C0/C1 writes and on parenting.
#[test]
fn c0_and_c1_writes_on_non_linking_joints_walk_nothing() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function MakePart(Anchored, At)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = Anchored, false, At
            P.Parent = workspace
            return P
        end
        local function BuildRig(Leaves, Origin)
            local Hub = MakePart(true, Origin)
            for _ = 1, Leaves do
                local Leaf = MakePart(false, Origin)
                local W = Instance.new("Weld")
                W.Part0, W.Part1 = Hub, Leaf
                W.Parent = Leaf
            end
            local MotorLeaf = MakePart(false, Origin)
            local Motor = Instance.new("Motor6D")
            Motor.Part0, Motor.Part1 = Hub, MotorLeaf
            Motor.Parent = MotorLeaf
            local SpareAt = Origin * CFrame.new(0, 50, 0)
            local Spare = MakePart(false, SpareAt)

            local Unparented = Instance.new("Weld")
            Unparented.Part0, Unparented.Part1 = Hub, Spare

            local Disabled = Instance.new("Weld")
            Disabled.Part0, Disabled.Part1, Disabled.Enabled = Hub, Spare, false
            Disabled.Parent = Spare

            local OneSided = Instance.new("Weld")
            OneSided.Part0 = Hub
            OneSided.Parent = Hub

            local Repoint = Instance.new("Weld")

            return {
                Hub = Hub, Motor = Motor, MotorLeaf = MotorLeaf,
                Spare = Spare, SpareAt = SpareAt,
                Unparented = Unparented, Disabled = Disabled, OneSided = OneSided, Repoint = Repoint,
            }
        end
        shared.Rigs = {
            Big = BuildRig(5000, CFrame.new(0, 300, 0)),
            Small = BuildRig(1, CFrame.new(500, 300, 0)),
        }
        shared.Work = {}
        function shared.WriteNonLinking(Name)
            local R = shared.Rigs[Name]
            local T0 = os.clock()
            for Index = 1, 300 do
                local Shift = CFrame.new(Index, 0, 0)
                local Counter = CFrame.new(0, Index, 0)
                R.Unparented.C0 = Shift
                R.Unparented.C1 = Counter
                R.Disabled.C0 = Shift
                R.Disabled.C1 = Counter
                R.OneSided.C0 = Shift
                R.OneSided.C1 = Counter
            end
            shared.Work[Name] = os.clock() - T0
        end
        "#,
    );
    let write_fn = compile(&lua, &engine, "local Name = ...; shared.WriteNonLinking(Name)");
    let time_call = |name: &str| {
        let start = std::time::Instant::now();
        call(&lua, &engine, &write_fn, name);
        start.elapsed().as_secs_f64()
    };
    call(&lua, &engine, &write_fn, "Big");
    call(&lua, &engine, &write_fn, "Small");
    let (mut big, mut small) = (0.0, 0.0);
    for _ in 0..3 {
        (big, small) = (time_call("Big"), time_call("Small"));
        if big < 2.0 * small + 0.05 {
            break;
        }
    }
    assert!(
        big < 2.0 * small + 0.05,
        "1,800 C0/C1 writes on non-linking welds took {big:.3} s of host time on a 5,001-part assembly, {small:.3} s on a 2-part assembly"
    );
    run(
        &lua,
        &engine,
        r#"
        local function near(A, B) return (A.Position - B.Position).Magnitude < 1e-3 end
        assert(shared.Work.Big > 0, "work clock charged")
        assert(math.abs(shared.Work.Big - shared.Work.Small) < 1e-11, string.format("work clock differed: %.15g vs %.15g", shared.Work.Big, shared.Work.Small))
        -- Non-linking C0/C1 writes must not move Spare
        assert(near(shared.Rigs.Big.Spare.CFrame, shared.Rigs.Big.SpareAt), "Big Spare moved")
        assert(near(shared.Rigs.Small.Spare.CFrame, shared.Rigs.Small.SpareAt), "Small Spare moved")

        -- With a pending Motor6D.Transform (component unsettled), non-linking writes (C0/C1/Enabled/Part0) must not settle it early
        local Big = shared.Rigs.Big
        local BeforeMotor = Big.MotorLeaf.CFrame
        Big.Motor.Transform = CFrame.new(0, 7, 0)
        Big.Unparented.C0 = CFrame.new(9, 0, 0)
        Big.Unparented.C1 = CFrame.new(0, 9, 0)
        Big.Unparented.Enabled = false
        Big.Unparented.Part0 = Big.Hub
        Big.Repoint.Part0 = Big.Hub
        Big.Repoint.Part1 = Big.Spare
        Big.Repoint.Part0 = nil
        assert(near(Big.MotorLeaf.CFrame, BeforeMotor), "non-linking writes settled a pending Motor6D.Transform early")
        task.wait()
        assert(near(Big.MotorLeaf.CFrame, Big.Hub.CFrame * CFrame.new(0, 7, 0)), "Motor6D.Transform settled on step")

        -- Part0/Part1 -> C0/C1 -> Parent weld creation places each leaf exactly on Parent, and active C0/C1 writes move it immediately
        local LastWeld, LastLeaf = nil, nil
        for Index = 1, 50 do
            local Leaf = Instance.new("Part")
            Leaf.Anchored, Leaf.CanCollide, Leaf.CFrame = false, false, CFrame.new(0, 0, 0)
            Leaf.Parent = workspace
            local W = Instance.new("Weld")
            local C0 = CFrame.new(Index, 2, 3)
            local C1 = CFrame.new(0, -1, 0)
            W.Part0, W.Part1 = Big.Hub, Leaf
            W.C0, W.C1 = C0, C1
            W.Parent = Leaf
            assert(near(Leaf.CFrame, Big.Hub.CFrame * C0 * C1:Inverse()), "leaf not placed on Parent")
            LastWeld, LastLeaf = W, Leaf
        end
        local NewC0 = CFrame.new(11, 13, 17)
        local NewC1 = CFrame.new(1, 2, 3)
        LastWeld.C0 = NewC0
        assert(near(LastLeaf.CFrame, Big.Hub.CFrame * NewC0 * LastWeld.C1:Inverse()), "active C0 write moved leaf")
        LastWeld.C1 = NewC1
        assert(near(LastLeaf.CFrame, Big.Hub.CFrame * NewC0 * NewC1:Inverse()), "active C1 write moved leaf")
        "#,
    );
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
            "Hub.Anchored" => crate::cost::joints_reanchored(100, 100),
            "Free.Anchored" => crate::cost::joints_reanchored(0, 100),
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
/// (link_assembly) more than its second, less the second's LINK_REJOINED. Each of those cost the same at any size before.
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
            -- (outside every world, where its Anchored write is no link change)
            Lone.Parent = game:GetService("ReplicatedStorage")
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
    let assembly = crate::cost::link_assembly(1000, false, 0) - crate::cost::link_assembly(10, false, 0);
    for (name, want) in [
        ("First", assembly),
        ("Again", 0.0),
        ("Centre", more * crate::cost::BODY_REBUILT_PART),
        ("CentreAgain", 0.0),
        ("Anchor", assembly),
        ("Unanchor", 0.0),
    ] {
        run(&lua, &engine, &format!("local Extra = shared.Big.{name} - shared.Small.{name} assert(math.abs(Extra - {want:e}) < 1e-9, `{name}: {{Extra}} s more for 1,001 parts than 11, not {want:e}`)"));
    }
    // (the weld disabled leaves the hub's 10 parts and the leaf alone, the first change of each in the frame; the second, which
    // starts the link the first ended, is the frame's first rejoin)
    let first = crate::cost::LINK_FRAME + crate::cost::link_assembly(10, false, 0) + crate::cost::link_assembly(1, true, 0) - crate::cost::LINK_REJOINED;
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
                W.C0, W.Part0, W.Part1 = CFrame.new(Index % 7, 4, 0), Hub, Part(false)
                W.Parent = W.Part1
                Welds[Index] = W
            end
            return { Hub = Hub, Welds = Welds, Anchored = Anchored }
        end
        shared.Stars = { Big = Star(true, 5000), Small = Star(true, 1), BigFree = Star(false, 5000), SmallFree = Star(false, 1) }
        function shared.Changes(Name, Rounds)
            local Star = shared.Stars[Name]
            for Index = 1, Rounds do
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
    let changes = compile(&lua, &engine, "local Name, Rounds = ...; shared.Changes(Name, Rounds)");
    let time = |name: &str, rounds: usize| {
        let start = std::time::Instant::now();
        call(&lua, &engine, &changes, (name, rounds));
        start.elapsed().as_secs_f64()
    };
    for (big, small) in [("Big", "Small"), ("BigFree", "SmallFree")] {
        call(&lua, &engine, &changes, (big, 20usize));
        call(&lua, &engine, &changes, (small, 20usize));
        let (mut large, mut lone) = (0.0, 0.0);
        for _ in 0..3 {
            (large, lone) = (time(big, 500), time(small, 500));
            if large < 2.0 * lone + 0.05 {
                break;
            }
        }
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
/// frame's start with 10,000 parts, 10 to 43 us on a later one; 18 to 35 us on a weld taken out right after it was taken in). Two
/// welds disabled the frame before and enabled in turn: the second, which joins its leaf alone to the assembly the first changed,
/// costs the same at either size (it was charged the whole assembly again: 857 us at 10,000, where a client spends 15.6). Two welds,
/// and two leaves with their welds, taken out the frame before and parented back in turn: the first costs LINK_PART more for each
/// part beyond the small hub's, the second the same at either size (each was charged the joints a walk of the whole assembly looked
/// at, 1.0 ms at 10,000 every time, where a client spends 920 us at a frame's start and 17 to 22 later).
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
            -- (two welds disabled the frame before, enabled in turn: the second joins its leaf alone to the assembly the first changed)
            Enabled = function(S) return function() S.W.Enabled = true end, function() S.W2.Enabled = true end, function() S.W.Enabled, S.W2.Enabled = false, false end end,
            -- (two welds, and two leaves with theirs, taken out the frame before and parented back in turn)
            WeldBack = function(S) return function() S.W.Parent = S.L end, function() S.W2.Parent = S.L2 end, function() S.W.Parent, S.W2.Parent = nil, nil end end,
            LeafBack = function(S) return function() S.L.Parent = S.Holder end, function() S.L2.Parent = S.Holder end, function() S.L.Parent, S.L2.Parent = nil, nil end end,
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
            S.L.Parent, S.L2.Parent, S.Holder = Holder, Holder, Holder
            -- (the frame reads the changed instances and writes the members first, a weld that links nothing and a lone part: the
            -- first touch and use of the frame are not what is compared)
            local Spare, Lone = Instance.new("Weld"), Part(false)
            Spare.Parent = workspace
            -- (outside every world, where its Anchored write is no link change)
            Lone.Parent = game:GetService("ReplicatedStorage")
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
    // (the hub's assembly after the change: 1,000 and 10 parts, 1,001 and 11 when its weld was re-pointed to a spare part)
    let assembly = |parts: usize| crate::cost::link_assembly(parts + 990, false, 0) - crate::cost::link_assembly(parts, false, 0);
    // (a weld taken in pays for the joints its walks look at, a later change in the frame the same at either size)
    run(&lua, &engine, "local Extra = shared.Big.WeldIn.Later - shared.Small.WeldIn.Later assert(math.abs(Extra) < 1e-9, `WeldIn Later: {Extra} s more for 1,001 parts than 11`)");
    for kind in ["Part1", "Nil", "Part0", "WeldOut", "WeldDestroyed", "LeafOut", "LeafDestroyed", "Enabled", "WeldBack", "LeafBack"] {
        for (name, want) in [("First", assembly(if kind == "Part1" { 11 } else { 10 })), ("Later", 0.0)] {
            run(&lua, &engine, &format!("local Extra = shared.Big.{kind}.{name} - shared.Small.{kind}.{name} assert(math.abs(Extra - {want:e}) < 1e-9, `{kind} {name}: {{Extra}} s more for 1,001 parts than 11, not {want:e}`)"));
        }
        // (the frame's first change pays LINK_FRAME and the assemblies it changes, a later one LINK_CHANGED, and LINK_REJOINED when it
        // is the frame's first that starts a link after one ended)
        let least = crate::cost::LINK_FRAME - crate::cost::LINK_REJOINED;
        run(&lua, &engine, &format!("local Extra = shared.Small.{kind}.First - shared.Small.{kind}.Later assert(Extra > {least:e}, `{kind}: the frame's first change cost {{Extra}} s more than the second`)"));
    }
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// One write that ends or starts several links pays for each beyond the first and for each piece it splits off or joins, at a
/// frame's start and later in it, as a playing client's does (cost.rs LINK_MORE, LINK_PIECE, fitted with the joints' list prices
/// charged; Studio 0.747, two sessions: an anchored hub's 100 unanchored leaves joined right after they split off in the frame 162
/// us beyond the same with the welds disabled, 100 anchored leaves split off at a frame's start 55 us and joined 47): beyond the
/// same writes with the welds disabled, an anchored hub welded to 100 unanchored parts taken out at a frame's start, put back and
/// taken out again in that frame; the same with 100 anchored parts, whose links change no assembly (no frame's or assembly's price,
/// as a client's cost it 4 to 8 us for one); and a Folder of 100 welds among a star's leaves taken out at a frame's start, which
/// splits nothing but opens 100 cycles in an anchored assembly (LINK_CYCLE_ENDED), and put back right after, which closes them
/// (LINK_CYCLE_PAIR for each pair: a client's cost grows with their square, 94.5 ms for 999 among 1,000 leaves). The joins read
/// within x0.7 to x1.5 of the client's and the anchored leaves within x2 (x3.2 and x2.5 to x2.6 with the prices fitted before the
/// list prices).
#[test]
fn a_write_that_changes_several_links_pays_for_each_and_each_piece() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    use crate::cost::*;
    let n = 100usize;
    // (each leaf split off at a frame's start a piece and an assembly of one unanchored part: every piece but one pays LINK_PIECE)
    let piece = link_assembly(1, true, 0);
    let more = (n - 1) as f64 * LINK_MORE;
    // (taken out, the welds also leave their parts' lists of active joints: each weld and each part whose list changes, the hub's and
    // the leaves', or the leaves' alone for welds among them)
    let listed = n as f64 * JOINT_TAKEN_OUT + (n + 1) as f64 * JOINT_LIST_LEFT;
    let free_out = LINK_FRAME + LINK_CHANGED + n as f64 * piece + (n - 1) as f64 * LINK_PIECE + more + listed;
    // (taken in, each link is a joint looked at and a link made beyond the joints looked at alike with the welds disabled)
    let free_in = n as f64 * (JOINT_LOOKED_AT + JOINT_LINKED) + LINK_CHANGED + LINK_REJOINED + more + (n - 1) as f64 * LINK_PIECE;
    let free_again = LINK_CHANGED + more + (n - 1) as f64 * LINK_PIECE + listed;
    let fixed_out = LINK_CHANGED + more + listed;
    let fixed_in = n as f64 * (JOINT_LOOKED_AT + JOINT_LINKED) + LINK_CHANGED + more;
    let one_out = LINK_FRAME + LINK_CHANGED + link_assembly(n + 1, false, 0) + more + n as f64 * (JOINT_TAKEN_OUT + JOINT_LIST_LEFT) + link_cycles(n, true);
    // (welds between parts in the world: each a link made beyond the joints looked at alike with the welds disabled)
    let one_in = n as f64 * JOINT_LINKED + LINK_CHANGED + LINK_REJOINED + more + link_cycles(n, false);
    run(
        &lua,
        &engine,
        &format!(
            r#"
        workspace.Gravity = 0
        local N = {n}
        local function Part(Anchored, At)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = Anchored, false, At
            P.Parent = workspace
            return P
        end
        local function Off(I)
            return CFrame.new(I % 10 * 2, 4, I // 10 * 2)
        end
        local function Star(LeavesAnchored, Enabled, Folder)
            local Hub = Part(true, CFrame.new(0, 300, 0))
            local Leaves = {{}}
            for I = 1, N do
                Leaves[I] = Part(LeavesAnchored, Hub.CFrame * Off(I))
                local W = Instance.new("Weld")
                W.Part0, W.Part1, W.C0, W.Enabled = Hub, Leaves[I], Off(I), Folder ~= nil or Enabled
                W.Parent = Leaves[I]
            end
            if Folder then
                for I = 1, N do
                    local J = I % N + 1
                    local W = Instance.new("Weld")
                    W.Part0, W.Part1, W.C0, W.Enabled = Leaves[I], Leaves[J], Off(I):Inverse() * Off(J), Enabled
                    W.Parent = Folder
                end
                Folder.Parent = workspace
            end
            return Folder or Hub
        end
        local function Time(F)
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        -- (each case's writes, at a frame's start and then in turn in that frame: the first wait of the script's start resumes in
        -- the frame it started in)
        local function Rounds(Thing)
            task.wait()
            task.wait()
            local Out = Time(function() Thing.Parent = nil end)
            local In = Time(function() Thing.Parent = workspace end)
            local Again = Time(function() Thing.Parent = nil end)
            task.wait()
            Thing.Parent = workspace
            return {{ Out, In, Again }}
        end
        -- (to 0.1 us: the two scenes' own writes differ by 20 ns, on main too, where each price compared is 59 us or more)
        local function Is(Name, Got, Want)
            assert(math.abs(Got - Want) < 1e-7, `{{Name}} cost {{Got * 1e6}} us beyond the same with its welds disabled, where {{Want * 1e6}}`)
        end
        local Free, FreeOff = Rounds(Star(false, true)), Rounds(Star(false, false))
        Is("free leaves split off", Free[1] - FreeOff[1], {free_out:e})
        Is("free leaves joined later in the frame", Free[2] - FreeOff[2], {free_in:e})
        Is("free leaves split off again", Free[3] - FreeOff[3], {free_again:e})
        local Fixed, FixedOff = Rounds(Star(true, true)), Rounds(Star(true, false))
        Is("anchored leaves split off", Fixed[1] - FixedOff[1], {fixed_out:e})
        Is("anchored leaves joined", Fixed[2] - FixedOff[2], {fixed_in:e})
        -- (against the client's medians: the joins and the anchored leaves)
        local function Near(Name, Got, Client, Low, High)
            local Ratio = Got / Client
            assert(Ratio > Low and Ratio < High, string.format("%s: x%.2f of a client's %.0f us", Name, Ratio, Client * 1e6))
        end
        Near("free leaves joined later in the frame", Free[2] - FreeOff[2], 162e-6, 0.7, 1.5)
        Near("anchored leaves split off", Fixed[1] - FixedOff[1], 55e-6, 0.5, 2)
        Near("anchored leaves joined", Fixed[2] - FixedOff[2], 47e-6, 0.5, 2)
        local One, OneOff = Rounds(Star(false, true, Instance.new("Folder"))), Rounds(Star(false, false, Instance.new("Folder")))
        Is("welds among leaves taken out", One[1] - OneOff[1], {one_out:e})
        Is("welds among leaves put back", One[2] - OneOff[2], {one_in:e})
        "#
        ),
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// An Anchored write of a part that links no other part costs a link change of its assembly of one, as a playing client's does
/// (joints.rs lone_reanchored, cost.rs charge_link; roblox-sim, Studio 0.747, three sessions, the geometric mean of their medians: a
/// lone part's Anchored written at a frame's start 39 us more than its CastShadow, then in turn 4.6, 3.0 and 2.5 more; a part in a
/// WorldModel alike, one in ReplicatedStorage as its CastShadow): beyond the same part's CastShadow, at a frame's start and in turn,
/// in the Workspace and a WorldModel; in ReplicatedStorage nothing. It cost the write's price alone.
#[test]
fn an_anchored_write_of_a_part_that_links_nothing_pays_a_link_change() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    use crate::cost::*;
    let (first, later) = (LINK_FRAME + LINK_CHANGED + link_assembly(1, false, 0), LINK_CHANGED);
    run(
        &lua,
        &engine,
        &format!(
            r#"
        workspace.Gravity = 0
        local Storage = game:GetService("ReplicatedStorage")
        local Model = Instance.new("WorldModel")
        Model.Parent = Storage
        local function Part(Parent)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = false, false, CFrame.new(0, 300, 0)
            P.Parent = Parent
            return P
        end
        local function Time(F)
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        -- (two writes, at a frame's start and in turn; a round first, whose first touches are not what is compared)
        local function Writes(P, Property)
            local Got
            for _ = 1, 2 do
                task.wait()
                task.wait()
                Got = {{ Time(function() P[Property] = not P[Property] end), Time(function() P[Property] = not P[Property] end) }}
            end
            return Got
        end
        for _, Case in {{ {{ "Workspace", workspace, {first:e}, {later:e} }}, {{ "WorldModel", Model, {first:e}, {later:e} }}, {{ "ReplicatedStorage", Storage, 0, 0 }} }} do
            local Anchored, Shadow = Writes(Part(Case[2]), "Anchored"), Writes(Part(Case[2]), "CastShadow")
            for Index = 1, 2 do
                local Extra = Anchored[Index] - Shadow[Index]
                -- (to 0.05 us: the walk's lookups in a WorldModel cost a few ns more)
                assert(math.abs(Extra - Case[2 + Index]) < 5e-8, `{{Case[1]}}: write {{Index}} cost {{Extra * 1e6}} us more than CastShadow, not {{Case[2 + Index] * 1e6}}`)
            end
        end
        "#
        ),
    );
}

/// A link change pays for the parts it gives another root, beyond the first of each side it re-roots, as a playing client's does
/// (cost.rs link_rerooted, joints.rs Shape; roblox-sim, Studio 0.747, three sessions, the geometric mean of their medians: a free
/// star of 1,000 parts alike, whose root is its last made leaf, that leaf's weld disabled at a frame's start 890 us and enabled
/// again 286, where another leaf's cost 186 and 11; an anchored hub's arm of 100 parts disabled at a frame's start 548 us and
/// enabled again in the frame 200, a leaf's 154 and 11): the free star's root leaf against another leaf at 200 leaves, and an
/// anchored hub's arm of 10 parts against a leaf. ree charged none of it.
#[test]
fn a_link_change_pays_for_the_parts_it_roots_elsewhere() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    use crate::cost::*;
    let (split, join) = (199.0 * LINK_REROOTED_SPLIT, 199.0 * LINK_REROOTED_JOIN);
    let arm_out = 9.0 * LINK_PIECE_PART + link_assembly(2, false, 0) + link_assembly(10, true, 0) - link_assembly(11, false, 0) - link_assembly(1, true, 0);
    let arm_in = 9.0 * LINK_PIECE_JOINED_PART;
    run(
        &lua,
        &engine,
        &format!(
            r#"
        workspace.Gravity = 0
        local Spot = 0
        local function Part(Anchored)
            Spot += 1
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.Size = Anchored, false, Vector3.one
            P.CFrame = CFrame.new(300 + Spot % 40 * 2, 300 + Spot // 40 * 2, 300)
            P.Parent = workspace
            return P
        end
        local function Weld(P0, P1)
            local W = Instance.new("Weld")
            W.Part0, W.Part1, W.C0 = P0, P1, P0.CFrame:ToObjectSpace(P1.CFrame)
            W.Parent = P1
            return W
        end
        local function Time(F)
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        -- (a weld disabled at a frame's start and enabled in turn; a round first, whose first touches are not what is compared)
        local function Pair(W)
            local Got
            for _ = 1, 2 do
                task.wait()
                task.wait()
                Got = {{ Time(function() W.Enabled = false end), Time(function() W.Enabled = true end) }}
            end
            return Got
        end
        local function Is(Name, Got, Want)
            assert(math.abs(Got - Want) < 1e-9, `{{Name}}: {{Got * 1e6}} us more, not {{Want * 1e6}}`)
        end
        local Hub, Welds = Part(false), {{}}
        for Index = 1, 200 do
            Welds[Index] = Weld(Hub, Part(false))
        end
        local Root, Other = Pair(Welds[200]), Pair(Welds[1])
        Is("the root leaf split off", Root[1] - Other[1], {split:e})
        Is("the root leaf joined", Root[2] - Other[2], {join:e})
        local Anchor = Part(true)
        local Leaf = Weld(Anchor, Part(false))
        local Arm, Last = nil, Anchor
        for Index = 1, 10 do
            local P = Part(false)
            local W = Weld(Last, P)
            Arm = Arm or W
            Last = P
        end
        local Piece, One = Pair(Arm), Pair(Leaf)
        Is("the arm split off", Piece[1] - One[1], {arm_out:e})
        Is("the arm joined", Piece[2] - One[2], {arm_in:e})
        "#
        ),
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// A link that closes or opens a cycle among parts none of which is anchored changes no assembly and costs a link change alone, as a
/// playing client's does (joints.rs Shape; roblox-sim, Studio 0.747, three sessions, the geometric mean of their medians: a free
/// star of 1,000 leaves, its hub the root, a weld between two leaves enabled at a frame's start 26 us, where a weld with no Part1
/// costs 16; an anchored hub's 136 to 144): beyond a weld with no Part1 enabled at a frame's start, a free star's such weld
/// LINK_CHANGED alone, an anchored star's the frame's and its assembly's prices too. ree charged the free star's assembly.
#[test]
fn a_cycle_among_free_parts_changes_no_assembly() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    use crate::cost::*;
    let (free, fixed) = (LINK_CHANGED, LINK_FRAME + LINK_CHANGED + link_assembly(301, false, 0));
    run(
        &lua,
        &engine,
        &format!(
            r#"
        workspace.Gravity = 0
        local Spot = 0
        local function Part(Anchored, Size)
            Spot += 1
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.Size = Anchored, false, Size or Vector3.one
            P.CFrame = CFrame.new(300 + Spot % 40 * 2, 300 + Spot // 40 * 2, 300)
            P.Parent = workspace
            return P
        end
        local function Weld(P0, P1)
            local W = Instance.new("Weld")
            W.Part0, W.Part1, W.C0 = P0, P1, if P1 then P0.CFrame:ToObjectSpace(P1.CFrame) else CFrame.identity
            W.Enabled = P1 ~= nil
            W.Parent = P0
            return W
        end
        local function Star(Anchored)
            local Hub, Leaves = Part(Anchored, Vector3.new(4, 4, 4)), {{}}
            for Index = 1, 300 do
                Leaves[Index] = Part(false)
                Weld(Hub, Leaves[Index])
            end
            local Cycle = Weld(Leaves[1], Leaves[2])
            Cycle.Enabled = false
            return Cycle
        end
        -- (enabled at a frame's start, disabled after; a round first, whose first touches are not what is compared)
        local function On(W)
            local Got
            for _ = 1, 2 do
                task.wait()
                task.wait()
                local Start = os.clock()
                W.Enabled = true
                Got = os.clock() - Start
                W.Enabled = false
            end
            return Got
        end
        local Free, Fixed, Loose = On(Star(false)), On(Star(true)), On(Weld(Part(false), nil))
        assert(math.abs(Free - Loose - {free:e}) < 1e-9, `a free star's cycle: {{(Free - Loose) * 1e6}} us more than a loose weld, not {{{free:e} * 1e6}}`)
        assert(math.abs(Fixed - Loose - {fixed:e}) < 1e-9, `an anchored star's cycle: {{(Fixed - Loose) * 1e6}} us more than a loose weld, not {{{fixed:e} * 1e6}}`)
        "#
        ),
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// A link change of a two-part assembly costs what a playing client's does (cost.rs LINK_ASSEMBLY, LINK_SPLIT_FIRST,
/// LINK_REJOINED_AFTER_JOIN; roblox-sim, Studio 0.747, three sessions, the geometric mean of their medians, each beyond the same
/// write of a disabled weld): 12 pairs of an anchored part welded to an unanchored one, after another pair's weld enabled first in a
/// frame, each weld taken out in turn and parented back: the first out (the frame's first split) 12.7 us, the third and later 5.1
/// (each splits an assembly the frame had not changed, two pieces of one part), the first back 3.9; 300 such pairs' welds taken out
/// in one loop at a frame's start 1.64 ms, and UnjoinFromOutsiders of their 300 anchored parts 2.03 ms. ree read x2.6 of the later
/// ones, x2.4 of the first back, x2.4 of the loop and x2.1 of the call when each fresh assembly paid 5 us (LINK_ASSEMBLY).
#[test]
fn link_changes_of_two_part_assemblies_cost_what_a_clients_do() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let split_first = crate::cost::LINK_SPLIT_FIRST;
    run(
        &lua,
        &engine,
        &format!(
            r#"
        workspace.Gravity = 0
        local Spot = 0
        local function Part(Anchored)
            Spot += 1
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = Anchored, false, CFrame.new(300 + Spot % 60 * 6, 300 + Spot // 60 * 6, 300)
            P.Parent = workspace
            return P
        end
        local function Pairs(Count, Enabled)
            local Set, Listed = {{}}, {{}}
            for I = 1, Count do
                local A, U = Part(true), Part(false)
                local W = Instance.new("Weld")
                W.Part0, W.Part1, W.C0, W.Enabled = A, U, A.CFrame:ToObjectSpace(U.CFrame), Enabled
                W.Parent = U
                Set[I], Listed[I] = {{ A = A, U = U, W = W }}, A
            end
            return Set, Listed
        end
        local function Time(F)
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        local function Near(Name, Got, Client)
            local Ratio = Got / Client
            assert(Ratio > 0.7 and Ratio < 1.4, string.format("%s: %.2f us, x%.2f of a client's %.2f", Name, Got * 1e6, Ratio, Client * 1e6))
        end
        local Sac = Pairs(1, false)[1]
        local On, Off = Pairs(12, true), Pairs(12, false)
        -- (each set's welds out in turn and back in turn, after another pair's weld enabled first in the frame; a round first, whose
        -- first touches are not what is compared)
        local function Round(Set)
            task.wait()
            task.wait()
            Sac.W.Enabled = true
            local Out, In = {{}}, {{}}
            for I = 1, #Set do
                Out[I] = Time(function() Set[I].W.Parent = nil end)
            end
            for I = 1, #Set do
                In[I] = Time(function() Set[I].W.Parent = Set[I].U end)
            end
            Sac.W.Enabled = false
            return Out, In
        end
        Round(On)
        Round(Off)
        local Out, In = Round(On)
        local COut, CIn = Round(Off)
        Near("the frame's first split", Out[1] - COut[1], 12.7e-6)
        Near("a later split", Out[5] - COut[5], 5.1e-6)
        Near("the first join after the splits", In[1] - CIn[1], 3.9e-6)
        -- (the first split costs LINK_SPLIT_FIRST more than a later one)
        local First = (Out[1] - COut[1]) - (Out[5] - COut[5])
        assert(math.abs(First - {split_first:e}) < 1e-8, `the frame's first split cost {{First * 1e6}} us more than a later one`)
        -- (300 pairs' welds out in one loop at a frame's start, and UnjoinFromOutsiders of their anchored parts)
        local Many, ManyListed = Pairs(300, true)
        local Still, StillListed = Pairs(300, false)
        local function Loop(Set)
            task.wait()
            task.wait()
            local Spent = Time(function()
                for I = 1, #Set do
                    Set[I].W.Parent = nil
                end
            end)
            for I = 1, #Set do
                Set[I].W.Parent = Set[I].U
            end
            return Spent
        end
        local function Unjoin(Listed)
            task.wait()
            task.wait()
            return Time(function() workspace:UnjoinFromOutsiders(Listed) end)
        end
        Near("300 welds out in a loop", Loop(Many) - Loop(Still), 1.64e-3)
        Near("UnjoinFromOutsiders of 300 parts", Unjoin(ManyListed) - Unjoin(StillListed), 2.03e-3)
        "#
        ),
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// A write that splits off or joins many pieces costs what a playing client's does (cost.rs LINK_PIECE, link_assembly, LINK_PARALLEL;
/// roblox-sim, Studio 0.747, three sessions, the geometric mean of their medians, beyond the same writes with the welds disabled):
/// an anchored hub's 100 unanchored leaves split off at a frame's start by their welds' Folder taken out, 300 us (each fresh piece
/// pays LINK_PIECE and its own assembly's price, as a piece split off later in the frame pays LINK_PIECE), and 100 welds between
/// two parts in a Folder parented back right after it was taken out, 235 us (each link beyond the first between parts another of
/// them links pays LINK_PARALLEL). ree read x2.1 and x0.32 when each fresh piece paid its assembly's 5 us alone and parallel links
/// paid as links among a star's leaves do.
#[test]
fn writes_that_split_or_join_many_pieces_cost_what_a_clients_do() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local Spot = 0
        local function Part(Anchored, Size)
            Spot += 1
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.Size = Anchored, false, Size or Vector3.one
            P.CFrame = CFrame.new(300 + Spot % 60 * 6, 300 + Spot // 60 * 6, 300)
            P.Parent = workspace
            return P
        end
        local function Weld(P0, P1, Enabled, Parent)
            local W = Instance.new("Weld")
            W.Part0, W.Part1, W.C0, W.Enabled = P0, P1, P0.CFrame:ToObjectSpace(P1.CFrame), Enabled
            W.Parent = Parent
        end
        local function Leaves(Enabled)
            local F, Hub = Instance.new("Folder"), Part(true, Vector3.new(4, 4, 4))
            for _ = 1, 100 do
                Weld(Hub, Part(false), Enabled, F)
            end
            F.Parent = workspace
            return F
        end
        local function Parallel(Enabled)
            local F, A, U = Instance.new("Folder"), Part(true, Vector3.new(2, 2, 2)), Part(false)
            for _ = 1, 100 do
                Weld(A, U, Enabled, F)
            end
            F.Parent = workspace
            return F
        end
        local function Time(F)
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        -- (the Folder out at a frame's start and back in right after; a round first, whose first touches are not what is compared)
        local function Round(F)
            task.wait()
            task.wait()
            local Out = Time(function() F.Parent = nil end)
            local In = Time(function() F.Parent = workspace end)
            return Out, In
        end
        local function Near(Name, Got, Client)
            local Ratio = Got / Client
            assert(Ratio > 0.7 and Ratio < 1.4, string.format("%s: %.1f us, x%.2f of a client's %.1f", Name, Got * 1e6, Ratio, Client * 1e6))
        end
        local Split, SplitOff = Leaves(true), Leaves(false)
        Round(Split)
        Round(SplitOff)
        local Out = Round(Split)
        local OutOff = Round(SplitOff)
        Near("100 leaves split off at a frame's start", Out - OutOff, 300e-6)
        local Joined, JoinedOff = Parallel(true), Parallel(false)
        Round(Joined)
        Round(JoinedOff)
        local _, In = Round(Joined)
        local _, InOff = Round(JoinedOff)
        Near("100 welds joined between two parts", In - InOff, 235e-6)
        "#,
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// A link change costs the assembly a playing client keeps (world/joints.rs mark_component; cost.rs LINK_PART, LINK_ANCHORED,
/// LINK_KEPT_PARALLEL; roblox-sim, Studio 0.747, two or three sessions, the geometric mean of their medians, the whole write timed):
/// an anchored hub with one unanchored leaf and 1,000 more joints, the leaf's weld disabled after another link change in the frame:
/// 1,000 Glue to unanchored parts 104 us (a client's Glue joins assemblies as a Weld does: 0.08 us a part), Welds to anchored parts
/// 24 us (each an assembly of its own: 0.005 us a weld), Welds between the hub and the leaf made after the leaf's 3.4 us (the link
/// kept: no assembly changes) and made before it 42 us (the client picks the link again, looking at each: 0.025 us a weld), where a
/// hub with none costs 14 us. ree read x0.12, x4.1, x2.2 and x0.18 of those when the price followed the joints' components.
#[test]
fn a_link_change_costs_the_assembly_a_client_keeps() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    use crate::cost::*;
    let (glue, anchored, kept, alone) = (link_assembly(1001, false, 0) - link_assembly(1, false, 0), LINK_ANCHORED, LINK_KEPT_PARALLEL, link_assembly(2, false, 0) - link_assembly(1, false, 0) - link_assembly(1, true, 0));
    run(
        &lua,
        &engine,
        &format!(
            r#"
        workspace.Gravity = 0
        local GLUE, ANCHORED, KEPT, ALONE = {glue:e}, {anchored:e}, {kept:e}, {alone:e}
        local Spot = 0
        local function Part(Anchored, Size)
            Spot += 1
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.Size = Anchored, false, Size or Vector3.one
            P.CFrame = CFrame.new(300 + Spot % 60 * 6, 300 + Spot // 60 * 6, 300)
            P.Parent = workspace
            return P
        end
        local function Joint(Class, P0, P1, Parent)
            local W = Instance.new(Class)
            W.Part0, W.Part1, W.C0 = P0, P1, P0.CFrame:ToObjectSpace(P1.CFrame)
            W.Parent = Parent or P1
            return W
        end
        local SacA, SacU = Part(true), Part(false)
        local Sac = Joint("Weld", SacA, SacU)
        Sac.Enabled = false
        -- (the hub's leaf, its weld made before the 1,000 other joints, or after them)
        local function Hub(Kind, Count, Last)
            local H, Leaf = Part(true, Vector3.new(4, 4, 4)), Part(false)
            local Folder = Instance.new("Folder")
            Folder.Parent = workspace
            local W = if Last then nil else Joint("Weld", H, Leaf)
            for _ = 1, Count do
                if Kind == "glue" then
                    Joint("Glue", H, Part(false))
                elseif Kind == "anchored" then
                    Joint("Weld", H, Part(true))
                elseif Kind == "parallel" then
                    Joint("Weld", H, Leaf, Folder)
                end
            end
            return W or Joint("Weld", H, Leaf)
        end
        -- (the weld disabled after another pair's weld enabled first in the frame; a round first, whose first touches are not what
        -- is compared)
        local function Off(W)
            local Spent
            for _ = 1, 2 do
                task.wait()
                task.wait()
                Sac.Enabled = true
                local Start = os.clock()
                W.Enabled = false
                Spent = os.clock() - Start
                W.Enabled = true
                Sac.Enabled = false
            end
            return Spent
        end
        local function Near(Name, Got, Client)
            local Ratio = Got / Client
            assert(Ratio > 0.5 and Ratio < 2, string.format("%s: %.1f us, x%.2f of a client's %.1f", Name, Got * 1e6, Ratio, Client * 1e6))
        end
        local function Is(Name, Got, Want)
            assert(math.abs(Got - Want) < 1e-8, string.format("%s: %.3f us more than a hub with none, not %.3f", Name, Got * 1e6, Want * 1e6))
        end
        local None = Off(Hub("none", 0))
        local Glue, Anchored = Off(Hub("glue", 1000)), Off(Hub("anchored", 1000))
        local After, Before = Off(Hub("parallel", 1000)), Off(Hub("parallel", 1000, true))
        Near("Glue", Glue, 104e-6)
        Near("anchored leaves", Anchored, 24e-6)
        Near("parallel welds made after", After, 3.4e-6)
        Near("parallel welds made before", Before, 42e-6)
        Is("Glue", Glue - None, GLUE)
        Is("anchored leaves", Anchored - None, 1000 * ANCHORED)
        -- (the leaf stays in the hub's assembly: one assembly of two parts where the hub with none leaves two of one part)
        Is("parallel welds made before", Before - None, 1000 * KEPT + ALONE)
        "#
        ),
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// A single link change finds the other joints between its two parts at the same cost however many there are (world/joints.rs
/// Joints::pairs, parallel_links): 8,000 welds between an anchored hub and an unanchored part, disabled one by one in the order
/// they were made, take about the host time 8,000 welds between two anchored parts take (whose changes look at no parallel joint),
/// each try a fresh pair of scenes. A change used to walk every joint linking the part with fewer links to count the parallel ones
/// (x1.4 to x1.46 of the anchored pair's time here, 8,000 to 10,000 welds).
#[test]
fn parallel_joints_cost_a_link_change_the_same_however_many() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let reflection = engine.reflection.clone();
    let workspace = engine.world().workspace;
    let folder_class = reflection.class_id("Folder").unwrap();
    let part_class = reflection.class_id("Part").unwrap();
    let weld_class = reflection.class_id("Weld").unwrap();
    let anchored_prop = reflection.name_id("Anchored").unwrap();
    let collide_prop = reflection.name_id("CanCollide").unwrap();
    let cframe_prop = reflection.name_id("CFrame").unwrap();
    let part0_prop = reflection.name_id("Part0").unwrap();
    let part1_prop = reflection.name_id("Part1").unwrap();
    let c0_prop = reflection.name_id("C0").unwrap();
    let gravity_prop = reflection.name_id("Gravity").unwrap();
    {
        let mut state = engine.state.borrow_mut();
        crate::instance::raw_set_silent(&mut state, &reflection, workspace, gravity_prop, crate::value::Value::Number(0.0));
    }
    run(
        &lua,
        &engine,
        r#"
        function shared.Off()
            for _, W in shared.Welds do
                W.Enabled = false
            end
        end
        "#,
    );
    let off = compile(&lua, &engine, "shared.Off()");
    let mut spot = 0u32;
    let mut make_scene = |free: bool| {
        let mut next_pos = || {
            spot += 1;
            rbx_dom_weak::types::Vector3::new(
                300.0 + (spot % 60 * 6) as f32,
                300.0 + ((spot / 60) % 60 * 6) as f32,
                300.0,
            )
        };
        let h_pos = next_pos();
        let u_pos = next_pos();
        let h_cf = crate::value::Value::Dom(Rc::new(rbx_dom_weak::types::Variant::CFrame(
            rbx_dom_weak::types::CFrame::new(h_pos, rbx_dom_weak::types::Matrix3::identity()),
        )));
        let u_cf = crate::value::Value::Dom(Rc::new(rbx_dom_weak::types::Variant::CFrame(
            rbx_dom_weak::types::CFrame::new(u_pos, rbx_dom_weak::types::Matrix3::identity()),
        )));
        let c0 = crate::value::Value::Dom(Rc::new(rbx_dom_weak::types::Variant::CFrame(
            rbx_dom_weak::types::CFrame::new(
                rbx_dom_weak::types::Vector3::new(u_pos.x - h_pos.x, u_pos.y - h_pos.y, u_pos.z - h_pos.z),
                rbx_dom_weak::types::Matrix3::identity(),
            ),
        )));
        let (h_id, u_id, folder_id, welds) = {
            let mut state = engine.state.borrow_mut();
            let h_id = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
            crate::instance::raw_set_silent(&mut state, &reflection, h_id, anchored_prop, crate::value::Value::Bool(true));
            crate::instance::raw_set_silent(&mut state, &reflection, h_id, collide_prop, crate::value::Value::Bool(false));
            crate::instance::raw_set_silent(&mut state, &reflection, h_id, cframe_prop, h_cf);
            let u_id = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
            crate::instance::raw_set_silent(&mut state, &reflection, u_id, anchored_prop, crate::value::Value::Bool(!free));
            crate::instance::raw_set_silent(&mut state, &reflection, u_id, collide_prop, crate::value::Value::Bool(false));
            crate::instance::raw_set_silent(&mut state, &reflection, u_id, cframe_prop, u_cf);
            let folder_id = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
            let mut welds = Vec::with_capacity(8_000);
            for _ in 0..8_000 {
                let w = crate::instance::create_named(&mut state, &reflection, weld_class, "Weld");
                crate::instance::raw_set_silent(&mut state, &reflection, w, part0_prop, crate::value::Value::Instance(h_id));
                crate::instance::raw_set_silent(&mut state, &reflection, w, part1_prop, crate::value::Value::Instance(u_id));
                crate::instance::raw_set_silent(&mut state, &reflection, w, c0_prop, c0.clone());
                state.dom.attach(w, folder_id);
                super::joints::made_in_place(&mut state, &reflection, w);
                welds.push(w);
            }
            (h_id, u_id, folder_id, welds)
        };
        engine_lua::instance::set_parent(&engine, h_id, Some(workspace)).expect("h in workspace");
        engine_lua::instance::set_parent(&engine, u_id, Some(workspace)).expect("u in workspace");
        engine_lua::instance::set_parent(&engine, folder_id, Some(workspace)).expect("folder in workspace");
        let welds_tbl = lua.create_table_with_capacity(welds.len(), 0).unwrap();
        for (i, &w) in welds.iter().enumerate() {
            welds_tbl.raw_set(i + 1, engine_lua::instance_value(&lua, &engine, w).unwrap()).unwrap();
        }
        let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared").get("shared").expect("shared");
        shared.set("Welds", welds_tbl).unwrap();
    };
    let mut time = |free: bool| {
        make_scene(free);
        let start = std::time::Instant::now();
        call(&lua, &engine, &off, ());
        start.elapsed().as_secs_f64()
    };
    let (mut free, mut rigid) = (0.0, 0.0);
    for _ in 0..3 {
        (free, rigid) = (time(true), time(false));
        if free < 1.25 * rigid + 0.02 {
            break;
        }
    }
    assert!(free < 1.25 * rigid + 0.02, "8,000 parallel welds to an unanchored part disabled in {free:.3} s of host time, to an anchored one {rigid:.3} s");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// Glue that a Parent write takes out of the world costs what Welds do, as a client spends as much on them (Glue links as a Weld
/// does: world/joints.rs): a Model holding an unanchored part welded to an anchored one and 1,000 Glue from the anchored part to
/// unanchored parts outside it, taken out of the Workspace at a frame's start, costs what the same Model with 1,000 Welds costs.
#[test]
fn glue_that_leaves_costs_what_welds_do() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        &format!(
            r#"
        workspace.Gravity = 0
        local Spot = 0
        local function Part(Anchored, Parent)
            Spot += 1
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide = Anchored, false
            P.CFrame = CFrame.new(300 + Spot % 60 * 6, 300 + Spot // 60 % 60 * 6, 300)
            P.Parent = Parent or workspace
            return P
        end
        local function Scene(Kind)
            local M = Instance.new("Model")
            M.Parent = workspace
            local B, P = Part(true), Part(false, M)
            local W = Instance.new("Weld")
            W.Part0, W.Part1, W.C0 = B, P, B.CFrame:ToObjectSpace(P.CFrame)
            W.Parent = M
            for _ = 1, 1000 do
                local Other = Part(false)
                local G = Instance.new(Kind)
                G.Part0, G.Part1, G.C0 = B, Other, B.CFrame:ToObjectSpace(Other.CFrame)
                G.Parent = M
            end
            return M
        end
        local function Out(M)
            task.wait()
            task.wait()
            local Start = os.clock()
            M.Parent = nil
            return os.clock() - Start
        end
        -- (a round first, whose first touches are not what is compared)
        Out(Scene("Glue"))
        Out(Scene("Weld"))
        local Extra = Out(Scene("Glue")) - Out(Scene("Weld"))
        -- (to 1 us: the two scenes' first touches differ by 0.2 us)
        assert(math.abs(Extra) < 1e-6, `1,000 Glue leaving cost {{Extra * 1e6}} us more than 1,000 Welds`)
        "#
        ),
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// A weld between two parts that later-made welds also link costs a Part1 write what its Enabled write costs (world/joints.rs
/// repointed: the order it named its pair in, from before the write): LINK_CHANGED alone, Part1 set to nil and back, beyond the
/// same writes of a disabled weld between the two, as a playing client's (roblox-sim, Studio 0.747, three sessions: an anchored
/// hub's leaf held by 1,001 welds, the first made re-pointed after another link change in the frame, 4.3 to 4.7 us where a
/// disabled weld's costs 2.7 to 3.0; then disabled, 2.5 to 2.9 as before it was re-pointed). The write was charged a split, the
/// frame's first split and LINK_KEPT_PARALLEL for each weld left (37.9 us), and the weld counted as the last made after it.
#[test]
fn a_parallel_weld_re_pointed_costs_what_disabling_it_costs() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let changed = crate::cost::LINK_CHANGED;
    run(
        &lua,
        &engine,
        &format!(
            r#"
        workspace.Gravity = 0
        local CHANGED = {changed:e}
        local Spot = 0
        local function Part(Anchored, Size)
            Spot += 1
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.Size = Anchored, false, Size or Vector3.one
            P.CFrame = CFrame.new(300 + Spot % 60 * 6, 300 + Spot // 60 % 60 * 6, 300)
            P.Parent = workspace
            return P
        end
        local function Weld(P0, P1, Enabled, Parent)
            local W = Instance.new("Weld")
            W.Part0, W.Part1, W.C0, W.Enabled = P0, P1, P0.CFrame:ToObjectSpace(P1.CFrame), Enabled
            W.Parent = Parent or P1
            return W
        end
        local Sac = Weld(Part(true), Part(false), false)
        local Hub, Leaf = Part(true, Vector3.new(4, 4, 4)), Part(false)
        local W = Weld(Hub, Leaf, true)
        local Folder = Instance.new("Folder")
        Folder.Parent = workspace
        for _ = 1, 1000 do
            Weld(Hub, Leaf, true, Folder)
        end
        local D = Weld(Hub, Leaf, false)
        local function Time(F)
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        -- (each weld's writes in a frame of their own, after another pair's weld enabled first in it; a round first, whose first
        -- touches are not what is compared)
        local function Repoint(X)
            task.wait()
            task.wait()
            Sac.Enabled = true
            local Nil, Back = Time(function() X.Part1 = nil end), Time(function() X.Part1 = Leaf end)
            Sac.Enabled = false
            return Nil, Back
        end
        local function Round()
            local Nil, Back = Repoint(W)
            local DNil, DBack = Repoint(D)
            return Nil - DNil, Back - DBack
        end
        local function Disabled()
            task.wait()
            task.wait()
            Sac.Enabled = true
            local Off = Time(function() W.Enabled = false end)
            W.Enabled = true
            Sac.Enabled = false
            return Off
        end
        Disabled()
        local Before = Disabled()
        Round()
        local Nil, Back = Round()
        local Off = Disabled()
        local function Is(Name, Got, Want)
            assert(math.abs(Got - Want) < 1e-8, string.format("%s: %.3f us, not %.3f", Name, Got * 1e6, Want * 1e6))
        end
        Is("Part1 set to nil, beyond a disabled weld's", Nil, CHANGED)
        Is("Part1 set back, beyond a disabled weld's", Back, CHANGED)
        Is("disabled after it was re-pointed and back, against before", Off, Before)
        "#
        ),
    );
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// Two later link changes in a frame cost more, as a playing client's do (cost.rs LINK_SPLIT_AGAIN, LINK_REJOINED): on an anchored hub
/// welded to 100 parts, two leaves' welds disabled and enabled again in turn in a frame, the second disable costs LINK_SPLIT_AGAIN
/// more than the last enable and the first enable LINK_REJOINED more; with another hub's weld disabled first in the frame, the first
/// hub's disable costs LINK_SPLIT_AGAIN more than it costs at a frame's start and its enable LINK_REJOINED more than the other hub's
/// enable after it. Each cost LINK_CHANGED alone (Studio 0.747: 4.0 to 8.7 us and 7.6 to 14.4, where the others cost 1.8 to 2.9).
#[test]
fn a_frames_first_rejoin_and_second_split_cost_more() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let (again, rejoined, frame) = (crate::cost::LINK_SPLIT_AGAIN, crate::cost::LINK_REJOINED, crate::cost::LINK_FRAME);
    run(
        &lua,
        &engine,
        &format!(
            r#"
        workspace.Gravity = 0
        local AGAIN, REJOINED, FRAME = {again:e}, {rejoined:e}, {frame:e}
        local function Part(Anchored)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.CFrame = Anchored, false, CFrame.new(0, 300, 0)
            P.Parent = workspace
            return P
        end
        local function Star(Count)
            local Hub, Welds = Part(true), {{}}
            for Index = 1, Count do
                local W = Instance.new("Weld")
                W.Part0, W.Part1 = Hub, Part(false)
                W.Parent = W.Part1
                Welds[Index] = W
            end
            return Welds
        end
        local A, B = Star(100), Star(100)
        -- (a weld that links nothing, toggled first in each frame: the frame's first write is not what is compared)
        local Spare = Instance.new("Weld")
        Spare.Parent = workspace
        local function Spend(F)
            local Start = os.clock()
            F()
            return os.clock() - Start
        end
        local function Is(Name, Got, Want)
            assert(math.abs(Got - Want) < 1e-9, `{{Name}}: {{Got * 1e6}} us more, where {{Want * 1e6}}`)
        end
        -- (a first round runs the code once: its first run's cold code is not what is compared)
        for Round = 1, 2 do
            task.wait()
            local _ = A[1].Enabled, A[2].Enabled, B[1].Enabled
            Spare.Enabled = not Spare.Enabled
            local Off1 = Spend(function() A[1].Enabled = false end)
            local Off2 = Spend(function() A[2].Enabled = false end)
            local On1 = Spend(function() A[1].Enabled = true end)
            local On2 = Spend(function() A[2].Enabled = true end)
            task.wait()
            local _ = A[1].Enabled, B[1].Enabled
            Spare.Enabled = not Spare.Enabled
            local First = Spend(function() A[1].Enabled = false end)
            A[1].Enabled = true
            task.wait()
            local _ = A[1].Enabled, B[1].Enabled
            Spare.Enabled = not Spare.Enabled
            B[1].Enabled = false
            local Second = Spend(function() A[1].Enabled = false end)
            local Rejoin = Spend(function() A[1].Enabled = true end)
            local Other = Spend(function() B[1].Enabled = true end)
            if Round == 2 then
                Is("the second disable", Off2 - On2, AGAIN)
                Is("the first enable", On1 - On2, REJOINED)
                Is("a disable after another hub's", Second - (First - FRAME), AGAIN)
                Is("its enable", Rejoin - Other, REJOINED)
            end
        end
        "#
        ),
    );
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
                W.C0, W.Part0, W.Part1 = CFrame.new(Index % 7, 4, 0), Hub, Part(false)
                W.Parent = W.Part1
                Welds[Index] = W
            end
            return { Hub = Hub, Welds = Welds }
        end
        local Big, Small = Star(5000), {}
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
        function shared.PrepSmall(From, To)
            for Index = From, To do
                if not Small[Index] then
                    Small[Index] = Star(4)
                end
            end
        end
        function shared.Rounds(Which, From, To)
            for Index = From, To do
                Round(Index, if Which == "Big" then Big else nil)
            end
        end
        "#,
    );
    let prep_small = compile(&lua, &engine, "local From, To = ...; shared.PrepSmall(From, To)");
    let rounds = compile(&lua, &engine, "local Which, From, To = ...; shared.Rounds(Which, From, To)");
    let time = |which: &str, from: usize, to: usize| {
        let start = std::time::Instant::now();
        call(&lua, &engine, &rounds, (which, from, to));
        start.elapsed().as_secs_f64()
    };
    call(&lua, &engine, &prep_small, (1usize, 20usize));
    call(&lua, &engine, &rounds, ("Big", 1usize, 20usize));
    call(&lua, &engine, &rounds, ("Small", 1usize, 20usize));
    let (mut large, mut small) = (0.0, 0.0);
    for attempt in 0..3 {
        let from = 21 + attempt * 300;
        let to = from + 299;
        call(&lua, &engine, &prep_small, (from, to));
        (large, small) = (time("Big", from, to), time("Small", from, to));
        if large < 2.0 * small + 0.05 {
            break;
        }
    }
    assert!(large < 2.0 * small + 0.05, "300 rounds of link changes took {large:.3} s of host time on a hub with 5,000 parts, {small:.3} s on hubs with 4");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// Joints and parts taken into a world join a large assembly without walking it: 300 rounds on an anchored hub welded to 5,000 parts
/// of a leaf's weld taken out and parented back, another leaf (its weld under it) taken out and parented back, and a Folder holding
/// a third leaf's two welds (to the hub and to a spare part, each taken out of the leaf before) parented into the Workspace and out,
/// each followed by a fourth leaf's AssemblyRootPart read, take about the host time of the same rounds on hubs welded to four parts
/// (a hub of its own each round), as a client's later link changes cost the same at any size. Each such write walked the whole
/// assembly again (5.6 to 7.3 ms at 10,000).
#[test]
fn entering_a_large_assembly_walks_none_of_it() {
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
        local Spare = Part(false)
        local function Star(Count)
            local Hub, Welds = Part(true), {}
            for Index = 1, Count do
                local W = Instance.new("Weld")
                W.C0, W.Part0, W.Part1 = CFrame.new(Index % 7, 4, 0), Hub, Part(false)
                W.Parent = W.Part1
                Welds[Index] = W
            end
            -- (the third leaf's welds, in a Folder of their own outside every world)
            local Box, Third = Instance.new("Folder"), Welds[3].Part1
            local Other = Instance.new("Weld")
            Other.Part0, Other.Part1, Other.Parent = Third, Spare, Box
            Welds[3].Parent = Box
            return { Hub = Hub, Welds = Welds, Box = Box }
        end
        local Big, Small = Star(5000), {}
        local function Round(Index, Big)
            local Star, At = if Big then Big else Small[Index], if Big then { Index + 3, Index + 1003, Index + 2003 } else { 1, 2, 4 }
            local W, L, Read = Star.Welds[At[1]], Star.Welds[At[2]].Part1, Star.Welds[At[3]].Part1
            local Leaf = W.Part1
            W.Parent = nil
            local _ = Read.AssemblyRootPart
            W.Parent = Leaf
            local _ = Read.AssemblyRootPart
            L.Parent = nil
            local _ = Read.AssemblyRootPart
            L.Parent = workspace
            local _ = Read.AssemblyRootPart
            Star.Box.Parent = workspace
            local _ = Read.AssemblyRootPart
            Star.Box.Parent = nil
            local _ = Read.AssemblyRootPart
        end
        function shared.PrepSmall(From, To)
            for Index = From, To do
                if not Small[Index] then
                    Small[Index] = Star(4)
                end
            end
        end
        function shared.Rounds(Which, From, To)
            for Index = From, To do
                Round(Index, if Which == "Big" then Big else nil)
            end
        end
        "#,
    );
    let prep_small = compile(&lua, &engine, "local From, To = ...; shared.PrepSmall(From, To)");
    let rounds = compile(&lua, &engine, "local Which, From, To = ...; shared.Rounds(Which, From, To)");
    let time = |which: &str, from: usize, to: usize| {
        let start = std::time::Instant::now();
        call(&lua, &engine, &rounds, (which, from, to));
        start.elapsed().as_secs_f64()
    };
    call(&lua, &engine, &prep_small, (1usize, 20usize));
    call(&lua, &engine, &rounds, ("Big", 1usize, 20usize));
    call(&lua, &engine, &rounds, ("Small", 1usize, 20usize));
    let (mut large, mut small) = (0.0, 0.0);
    for attempt in 0..3 {
        let from = 21 + attempt * 300;
        let to = from + 299;
        call(&lua, &engine, &prep_small, (from, to));
        (large, small) = (time("Big", from, to), time("Small", from, to));
        if large < 2.0 * small + 0.05 {
            break;
        }
    }
    assert!(large < 2.0 * small + 0.05, "300 rounds of joints and parts taken in took {large:.3} s of host time on a hub with 5,000 parts, {small:.3} s on hubs with 4");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// One write's link changes cost host time by the links it ends or starts, however many of them one part has: 3 rounds of a Folder
/// of 30,000 welds between one part and an anchored hub (made before the hub's 60,000 other welds, so first in its list) taken out of
/// the Workspace and put back, then that part taken out and put back, each followed by a root read, take about the host time of the
/// same rounds with the 30,000 welds spread over 3,000 parts, 10 each, each part welded to a hub's part of its own. Each link taken
/// out of a part's kept list, or put into it, moved the rest of the list: 0.45 to 0.8 s of host time a write here, 0.03 to 0.13 s
/// spread.
#[test]
fn one_writes_link_changes_cost_the_links_however_many_one_part_has() {
    let scene = |spread: bool| {
        let (lua, engine) = start_play();
        crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
        let reflection = engine.reflection.clone();
        let workspace = engine.world().workspace;
        let model_class = reflection.class_id("Model").unwrap();
        let folder_class = reflection.class_id("Folder").unwrap();
        let part_class = reflection.class_id("Part").unwrap();
        let weld_class = reflection.class_id("Weld").unwrap();
        let anchored_prop = reflection.name_id("Anchored").unwrap();
        let collide_prop = reflection.name_id("CanCollide").unwrap();
        let cframe_prop = reflection.name_id("CFrame").unwrap();
        let part0_prop = reflection.name_id("Part0").unwrap();
        let part1_prop = reflection.name_id("Part1").unwrap();
        let c0_prop = reflection.name_id("C0").unwrap();
        let gravity_prop = reflection.name_id("Gravity").unwrap();
        let part_cf = crate::value::Value::Dom(Rc::new(rbx_dom_weak::types::Variant::CFrame(
            rbx_dom_weak::types::CFrame::new(rbx_dom_weak::types::Vector3::new(0.0, 300.0, 0.0), rbx_dom_weak::types::Matrix3::identity()),
        )));
        let weld_c0 = crate::value::Value::Dom(Rc::new(rbx_dom_weak::types::Variant::CFrame(
            rbx_dom_weak::types::CFrame::new(rbx_dom_weak::types::Vector3::new(0.0, 3.0, 0.0), rbx_dom_weak::types::Matrix3::identity()),
        )));
        let (model, welds, box_model, read) = {
            let mut state = engine.state.borrow_mut();
            crate::instance::raw_set_silent(&mut state, &reflection, workspace, gravity_prop, crate::value::Value::Number(0.0));
            let make_part = |state: &mut crate::State, anchored: bool, parent: crate::InstanceId| {
                let p = crate::instance::create_named(state, &reflection, part_class, "Part");
                crate::instance::raw_set_silent(state, &reflection, p, anchored_prop, crate::value::Value::Bool(anchored));
                crate::instance::raw_set_silent(state, &reflection, p, collide_prop, crate::value::Value::Bool(false));
                crate::instance::raw_set_silent(state, &reflection, p, cframe_prop, part_cf.clone());
                state.dom.attach(p, parent);
                p
            };
            let make_weld = |state: &mut crate::State, a: crate::InstanceId, b: crate::InstanceId, parent: crate::InstanceId| {
                let w = crate::instance::create_named(state, &reflection, weld_class, "Weld");
                crate::instance::raw_set_silent(state, &reflection, w, c0_prop, weld_c0.clone());
                crate::instance::raw_set_silent(state, &reflection, w, part0_prop, crate::value::Value::Instance(a));
                crate::instance::raw_set_silent(state, &reflection, w, part1_prop, crate::value::Value::Instance(b));
                state.dom.attach(w, parent);
                super::joints::made_in_place(state, &reflection, w);
            };
            let model = crate::instance::create_named(&mut state, &reflection, model_class, "Model");
            let hub = make_part(&mut state, true, model);
            let box_model = crate::instance::create_named(&mut state, &reflection, model_class, "Model");
            let welds = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
            state.dom.attach(box_model, model);
            state.dom.attach(welds, model);
            let one = make_part(&mut state, false, box_model);
            let mut leaves = Vec::with_capacity(60_000);
            for _ in 0..60_000 {
                leaves.push(make_part(&mut state, false, model));
            }
            let mut groups: Vec<Option<crate::InstanceId>> = vec![None; 3_000];
            for index in 0..30_000usize {
                let group = index / 10;
                if spread {
                    let target = *groups[group].get_or_insert_with(|| make_part(&mut state, false, box_model));
                    make_weld(&mut state, leaves[group], target, welds);
                } else {
                    make_weld(&mut state, hub, one, welds);
                }
            }
            for &leaf in &leaves {
                make_weld(&mut state, hub, leaf, leaf);
            }
            (model, welds, box_model, leaves[59_999])
        };
        engine_lua::instance::set_parent(&engine, model, Some(workspace)).expect("model in workspace");
        {
            let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared").get("shared").expect("shared");
            shared.set("Model", engine_lua::instance_value(&lua, &engine, model).unwrap()).unwrap();
            shared.set("Welds", engine_lua::instance_value(&lua, &engine, welds).unwrap()).unwrap();
            shared.set("Box", engine_lua::instance_value(&lua, &engine, box_model).unwrap()).unwrap();
            shared.set("Read", engine_lua::instance_value(&lua, &engine, read).unwrap()).unwrap();
        }
        run(
            &lua,
            &engine,
            r#"
            local Model, Welds, Box, Read = shared.Model, shared.Welds, shared.Box, shared.Read
            local _ = Read.AssemblyRootPart
            function shared.Round()
                Welds.Parent = nil
                local _ = Read.AssemblyRootPart
                Welds.Parent = Model
                local _ = Read.AssemblyRootPart
                Box.Parent = nil
                local _ = Read.AssemblyRootPart
                Box.Parent = Model
                local _ = Read.AssemblyRootPart
            end
            "#,
        );
        let round = compile(&lua, &engine, "shared.Round()");
        (lua, engine, round)
    };
    // (the two scenes' rounds in turn, so whatever else the host does weighs on both alike; up to three tries, each pair made afresh)
    let mut host = [0.0; 2];
    for _ in 0..3 {
        let scenes = [scene(false), scene(true)];
        for (lua, engine, round) in &scenes {
            call(lua, engine, round, ());
        }
        host = [0.0; 2];
        for _ in 0..3 {
            for (index, (lua, engine, round)) in scenes.iter().enumerate() {
                let start = std::time::Instant::now();
                call(lua, engine, round, ());
                host[index] += start.elapsed().as_secs_f64();
            }
        }
        for (_, engine, _) in &scenes {
            super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
        }
        if host[0] < 2.0 * host[1] + 0.05 {
            break;
        }
    }
    let [one, spread] = host;
    assert!(one < 2.0 * spread + 0.05, "3 rounds of 30,000 links ended and started 4 times took {one:.3} s of host time with one part holding them all, {spread:.3} s with 10 held by each of 3,000 parts");
}

/// Link changes in a component of several assemblies walk none of it: 500 rounds of a leaf's weld disabled and enabled and re-pointed
/// to a spare part and back (each a different leaf), each change followed by AssemblyRootPart reads of the leaf and of a part welded
/// to both anchored parts, on an anchored hub welded to 5,000 parts one of which is also welded to another anchored part, take about
/// the host time they take on such a hub welded to 3 parts, as a client's root read after a link change costs the same at any size
/// (2.1 us with 10,000 parts). Each change settled the whole component from its roots (0.8 ms at 10,000), and a root read after a
/// change worked its tree out.
#[test]
fn link_changes_among_several_anchored_parts_walk_none_of_their_assembly() {
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
                W.C0, W.Part0, W.Part1 = CFrame.new(Index % 7, 4, 0), Hub, Part(false)
                W.Parent = W.Part1
                Welds[Index] = W
            end
            local Other, Both = Part(true), Welds[1].Part1
            local W = Instance.new("Weld")
            W.C0, W.Part0, W.Part1 = CFrame.new(0, 2, 0), Both, Other
            W.Parent = Other
            return { Welds = Welds, Both = Both }
        end
        local Spare = Part(false)
        shared.Stars = { Big = Star(5000), Small = Star(3) }
        function shared.Changes(Name, Rounds)
            local Star = shared.Stars[Name]
            for Index = 1, Rounds do
                local W = Star.Welds[Index % (#Star.Welds - 1) + 2]
                local L = W.Part1
                W.Enabled = false
                local _ = L.AssemblyRootPart, Star.Both.AssemblyRootPart
                W.Enabled = true
                local _ = L.AssemblyRootPart, Star.Both.AssemblyRootPart
                W.Part1 = Spare
                local _ = L.AssemblyRootPart, Star.Both.AssemblyRootPart
                W.Part1 = L
                local _ = L.AssemblyRootPart, Star.Both.AssemblyRootPart
            end
        end
        "#,
    );
    let changes = compile(&lua, &engine, "local Name, Rounds = ...; shared.Changes(Name, Rounds)");
    let time = |name: &str, rounds: usize| {
        let start = std::time::Instant::now();
        call(&lua, &engine, &changes, (name, rounds));
        start.elapsed().as_secs_f64()
    };
    call(&lua, &engine, &changes, ("Big", 20usize));
    call(&lua, &engine, &changes, ("Small", 20usize));
    let (mut large, mut small) = (0.0, 0.0);
    for _ in 0..3 {
        (large, small) = (time("Big", 500), time("Small", 500));
        if large < 2.0 * small + 0.05 {
            break;
        }
    }
    assert!(large < 2.0 * small + 0.05, "500 rounds of link changes took {large:.3} s of host time with 5,002 parts and two anchored, {small:.3} s with 5");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// An AssemblyMass read whose kept walk was made while a leaf's weld was disabled walks nothing after the weld is enabled again,
/// though the leaf joins the middle of the walk's order: on a free hub welded to 10,000 parts (the hub the root), each try
/// disables all 320 target leaves, drops the kept walk once (a weld closing a cycle enabled and disabled), and reads the hub's
/// mass (making its kept walk once with those leaves out); then each of the 300 timed rounds enables another leaf's weld and
/// times the hub's mass read alone (taking about the host time the same reads take on 320 four-part hubs, a big and a small
/// hub's round in turn, up to three tries). The read after the enable walked the whole assembly again (about 0.6 ms at 10,000);
/// it adds the kept masses up again from the leaf on, to the bit (30 to 35 us at 10,001 parts, 0.2 to 0.3 ms with the caches
/// cold on a loaded host).
#[test]
fn a_mass_read_after_a_leaf_rejoins_its_walks_middle_walks_nothing() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function Part(Size)
            local P = Instance.new("Part")
            P.Anchored, P.CanCollide, P.Size, P.CFrame = false, false, Size, CFrame.new(0, 300, 0)
            P.Parent = workspace
            return P
        end
        local function Star(Count)
            local Hub, Welds = Part(Vector3.new(6, 6, 6)), {}
            for Index = 1, Count do
                local W = Instance.new("Weld")
                W.C0, W.Part0, W.Part1 = CFrame.new(Index % 7, 4, 0), Hub, Part(Vector3.new(1, 1 + Index % 3, 1))
                W.Parent = W.Part1
                Welds[Index] = W
            end
            local Cycle = Instance.new("Weld")
            Cycle.Part0, Cycle.Part1, Cycle.Enabled, Cycle.Parent = Welds[1].Part1, Welds[2].Part1, false, workspace
            return { Hub = Hub, Welds = Welds, Cycle = Cycle }
        end
        shared.Stars = { Big = Star(10000), Small = {} }
        for Index = 1, 320 do
            shared.Stars.Small[Index] = Star(4)
        end
        -- Build each kept walk once per try with all 320 target leaves disabled, so every round's
        -- enable inserts a leaf into the middle of the walk order via Body::insert_one without
        -- paying an untimed 10,000-part cycle split and full walk on every single round.
        function shared.ResetTry()
            local Big = shared.Stars.Big
            for Round = 1, 320 do
                Big.Welds[2000 + Round * 7].Enabled = false
            end
            Big.Cycle.Enabled = true
            Big.Cycle.Enabled = false
            local _ = Big.Hub.AssemblyMass
            for Round = 1, 320 do
                local S = shared.Stars.Small[Round]
                S.Welds[3].Enabled = false
                S.Cycle.Enabled = true
                S.Cycle.Enabled = false
                local _ = S.Hub.AssemblyMass
            end
        end
        function shared.Prepare(Name, Round)
            local Star = if Name == "Big" then shared.Stars.Big else shared.Stars.Small[Round]
            local W = Star.Welds[if Name == "Big" then 2000 + Round * 7 else 3]
            W.Enabled = true
            shared.Hub = Star.Hub
        end
        "#,
    );
    let reset_try = compile(&lua, &engine, "shared.ResetTry()");
    let prepare = compile(&lua, &engine, "local Name, Round = ...; shared.Prepare(Name, Round)");
    let read_mass = compile(&lua, &engine, "local _ = shared.Hub.AssemblyMass");
    // (each round's big read and small read in turn, so whatever else the host does weighs on both alike; up to three tries)
    let time = |name: &str, round: usize| {
        call(&lua, &engine, &prepare, (name, round));
        let start = std::time::Instant::now();
        call(&lua, &engine, &read_mass, ());
        start.elapsed().as_secs_f64()
    };
    let (mut large, mut small) = (0.0, 0.0);
    for _ in 0..3 {
        call(&lua, &engine, &reset_try, ());
        (large, small) = (0.0, 0.0);
        for round in 1..=320 {
            let (big, little) = (time("Big", round), time("Small", round));
            if round > 20 {
                large += big;
                small += little;
            }
        }
        if large < 2.0 * small + 0.05 {
            break;
        }
    }
    assert!(large < 2.0 * small + 0.05, "300 mass reads after a leaf joined the middle of their walks took {large:.3} s of host time with 10,001 parts, {small:.3} s with 5");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// AssemblyMass reads after link changes add up nothing again: 500 rounds of a leaf's weld disabled and enabled (each a different
/// leaf, all over the hub's list), each change followed by the hub's and another leaf's AssemblyMass, on an anchored hub welded to
/// 5,000 parts take about the host time they take on a hub welded to 2, as a playing client's mass read after a link change of an
/// anchored assembly costs the same at any size (0.3 to 1.1 us at 1,000 to 10,000 parts), the big hub's rounds and the small hub's
/// in turn (up to three tries). Each read after a change worked the mass out from every part (0.6 ms at 10,000).
#[test]
fn mass_reads_after_link_changes_add_up_nothing_again() {
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
        local function Star(Count)
            local Hub, Welds = Part(true, Vector3.new(6, 6, 6)), {}
            for Index = 1, Count do
                local W = Instance.new("Weld")
                W.C0, W.Part0, W.Part1 = CFrame.new(Index % 7, 4, 0), Hub, Part(false, Vector3.new(1, 1 + Index % 3, 1))
                W.Parent = W.Part1
                Welds[Index] = W
            end
            local _ = Hub.AssemblyMass, Welds[1].Part1.AssemblyMass
            return { Hub = Hub, Welds = Welds }
        end
        shared.Stars = { Big = Star(5000), Small = Star(2) }
        function shared.Changes(Name, Rounds)
            local Star = shared.Stars[Name]
            local Other = Star.Welds[1].Part1
            for Index = 1, Rounds do
                local W = Star.Welds[Index * 7 % (#Star.Welds - 1) + 2]
                W.Enabled = false
                local _ = Star.Hub.AssemblyMass, Other.AssemblyMass
                W.Enabled = true
                local _ = Star.Hub.AssemblyMass, Other.AssemblyMass
            end
        end
        "#,
    );
    let changes = compile(&lua, &engine, "local Name, Rounds = ...; shared.Changes(Name, Rounds)");
    // (the big hub's rounds and the small hub's in turn, so whatever else the host does weighs on both alike; up to three tries)
    let time = |name: &str, rounds: usize| {
        let start = std::time::Instant::now();
        call(&lua, &engine, &changes, (name, rounds));
        start.elapsed().as_secs_f64()
    };
    call(&lua, &engine, &changes, ("Big", 20usize));
    call(&lua, &engine, &changes, ("Small", 20usize));
    let (mut large, mut small) = (0.0, 0.0);
    for _ in 0..3 {
        (large, small) = (time("Big", 500), time("Small", 500));
        if large < 2.0 * small + 0.05 {
            break;
        }
    }
    assert!(large < 2.0 * small + 0.05, "500 rounds of link changes and mass reads took {large:.3} s of host time with 5,001 parts, {small:.3} s with 3");
    super::joints::check_index(&mut engine.state.borrow_mut(), &engine.reflection).expect("the joint index");
}

/// An AssemblyCenterOfMass read right after a move works nothing out: 1,000 moves of an anchored hub welded to 5,000 parts, each
/// followed by a read of its centre or of a lone part's, the hub's reads taking about the host time the lone part's take after
/// the same move (the medians), as a playing client's read after a move costs the same at any size (the move works the centres
/// read since the last one out again, from the masses kept). Each read used to work the centre out from every part (0.2 to 0.3
/// ms at 5,000).
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
            W.C0, W.Part0, W.Part1 = CFrame.new(Index % 11, 2, Index % 13), Hub, Part(false)
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
    let move_big = compile(&lua, &engine, "shared.Move('Big')");
    let read_big = compile(&lua, &engine, "shared.Read('Big')");
    let read_lone = compile(&lua, &engine, "shared.Read('Lone')");
    // (the two parts' reads in turn, each timed alone right after a move of the hub, so what the move leaves for the next script and
    // whatever else the host is doing weigh on both alike: the medians; the hub's centre is read again after the lone part's, so the
    // next move works it out again, as it does a centre read since the last one)
    let mut times: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    for _ in 0..100 {
        for (index, reader) in [&read_big, &read_lone].into_iter().enumerate() {
            call(&lua, &engine, &move_big, ());
            let start = std::time::Instant::now();
            call(&lua, &engine, reader, ());
            times[index].push(start.elapsed().as_secs_f64());
        }
        call(&lua, &engine, &read_big, ());
    }
    let [big, lone] = times.map(|mut list| {
        crate::sort::by(&mut list, f64::total_cmp);
        list[list.len() / 2]
    });
    assert!(big < 2.0 * lone + 50e-6, "a centre read after a move took {:.1} us of host time (the median of 100) for an assembly of 5,001 parts, {:.1} us for a lone part after the same move", big * 1e6, lone * 1e6);
}

/// Kept components answer as fresh walks do through link changes: 800 random changes to two anchored hubs and an unanchored one each
/// welded to 12 parts (Welds and WeldConstraints; one hub's first part also welded to an anchored part), a chain of 10 parts hanging
/// from an anchored one, a free chain of 8 and a chain of 10 anchored at both ends (joints
/// enabled, disabled, destroyed and made again, moved to nil and back, a few at a time into a Folder that then enters the world, a few
/// in a Folder in the world that leaves it and comes back; parts
/// anchored and unanchored, moved by CFrame, PivotTo
/// and the physics step, Motor6D.Transform written; a frame now and then), with every part's AssemblyRootPart, AssemblyMass (half
/// of them before the centre), AssemblyCenterOfMass and CFrame and each change's work read after each change, read the same with the components kept (split
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
        for Hubs = 1, 3 do
            local Hub = Part(Hubs ~= 2, CFrame.new(Hubs * 40, 50, 0), Vector3.new(6, 6, 6))
            local First
            for Index = 1, 12 do
                local Leaf = Part(false, Hub.CFrame * CFrame.new(Index % 5 - 2, 4, Index // 5))
                Join(if Index % 3 == 0 then "WeldConstraint" elseif Index % 4 == 0 then "Motor6D" else "Weld", Hub, Leaf, Hub.CFrame:ToObjectSpace(Leaf.CFrame))
                First = First or Leaf
            end
            -- (the third hub's first leaf also welded to an anchored part: a component of two assemblies)
            if Hubs == 3 then
                local Other = Part(true, First.CFrame * CFrame.new(0, 3, 0))
                Join("Weld", First, Other, First.CFrame:ToObjectSpace(Other.CFrame))
            end
        end
        for Chains = 1, 3 do
            local Prev = Part(Chains ~= 2, CFrame.new(-40 * Chains, 50, 0))
            for Index = 1, if Chains == 2 then 8 else 10 do
                local Next = Part(Chains == 3 and Index == 10, Prev.CFrame * CFrame.new(2, 0, 0) * CFrame.Angles(0, 0.1, 0))
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
                -- (half the parts' masses read before their centres: a mass read alone after a link change)
                local Mass = if Index % 2 == 0 then P.AssemblyMass else nil
                local Centre = P.AssemblyCenterOfMass
                Mass = Mass or P.AssemblyMass
                table.insert(Line, string.format("%d:%d:%.9g:%s:%s", Index, Root and table.find(Parts, Root) or 0, Mass, Numbers(Centre.X, Centre.Y, Centre.Z), Numbers(P.CFrame:GetComponents())))
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
                -- (one to three joints taken into a Folder outside every world, which then enters it: several links at once)
                local Box, Picked = Instance.new("Folder"), {}
                for _ = 1, Dice:NextInteger(1, 3) do
                    local J = Pick(Joints)
                    if not table.find(Picked, J) then
                        J.Parent = Box
                        table.insert(Picked, J)
                    end
                end
                Box.Parent = if Dice:NextInteger(1, 3) == 1 then Model else workspace
                for _, J in Picked do
                    J.Parent = J.Part1
                end
                Box:Destroy()
                return "box"
            end,
            function()
                -- (two to six joints moved into a Folder in the world, which leaves it and comes back: several links ended by one
                -- write, then started by one)
                local Box, Picked = Instance.new("Folder"), {}
                Box.Parent = workspace
                for _ = 1, Dice:NextInteger(2, 6) do
                    local J = Pick(Joints)
                    if not table.find(Picked, J) then
                        J.Parent = Box
                        table.insert(Picked, J)
                    end
                end
                Box.Parent = nil
                Box.Parent = if Dice:NextInteger(1, 3) == 1 then Model else workspace
                for _, J in Picked do
                    J.Parent = J.Part1
                end
                Box:Destroy()
                return "folder"
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
    register_wrap(&lua, &engine);
    // (each pass's 2,000 rounds change the part 16 times: Transparency 1, Position, CFrame and PivotTo's 5 each)
    let world_level = 2000.0 * crate::cost::world_levels(1, 16);
    run(&lua, &engine, &format!("shared.WorldLevel = {world_level:e}"));
    run(
        &lua,
        &engine,
        r#"
        -- (a chain of folders `Depth` deep holding `Inner`, built directly in the dom so making it walks nothing)
        local Wrap = shared.Wrap
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
        local function Pass(Part, Rounds)
            local X = Part.Position.X
            for I = 1, Rounds do
                Part.Transparency = I % 2 * 0.5
                Part.Position = Vector3.new(X, 10 + I % 2, 0)
                Part.CFrame = CFrame.new(X, 12 + I % 2, 0)
                Part:PivotTo(CFrame.new(X, 14 + I % 2, 0))
            end
        end
        -- warm the members and the part the frame touches, then time the full 2,000-round pass
        function shared.Warm(Name)
            Pass(shared.Parts[Name], 20)
        end
        function shared.Use(Name)
            local Start = os.clock()
            Pass(shared.Parts[Name], 2000)
            shared.Work[Name] = os.clock() - Start
        end
        "#,
    );
    let warm = compile(&lua, &engine, "local Name = ...; shared.Warm(Name)");
    let use_part = compile(&lua, &engine, "local Name = ...; shared.Use(Name)");
    for name in ["Shallow", "Deep", "WorldShallow", "World", "LooseShallow", "Loose"] {
        call(&lua, &engine, &warm, name);
    }
    let mut host = std::collections::HashMap::new();
    for (deep, shallow) in [("Deep", "Shallow"), ("World", "WorldShallow"), ("Loose", "LooseShallow")] {
        let (mut deep_s, mut shallow_s) = (0.0, 0.0);
        for _ in 0..3 {
            let start = std::time::Instant::now();
            call(&lua, &engine, &use_part, shallow);
            shallow_s = start.elapsed().as_secs_f64();
            let start = std::time::Instant::now();
            call(&lua, &engine, &use_part, deep);
            deep_s = start.elapsed().as_secs_f64();
            if deep_s < 2.0 * shallow_s + 0.05 {
                break;
            }
        }
        host.insert(shallow, shallow_s);
        host.insert(deep, deep_s);
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
        assert!(deep_s < 2.0 * shallow_s + 0.05, "2,000 rounds of 4 writes took {deep_s:.3} s of host time to a part 10,000 folders deep ({deep}), {shallow_s:.3} s right in its parent");
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
    register_wrap(&lua, &engine);
    let levels = |changes: u32| crate::cost::world_levels(300, changes);
    run(&lua, &engine, &format!("shared.Levels = {{ [0] = 0, {:e}, {:e}, {:e}, {:e}, {:e} }}", levels(1), levels(2), levels(3), levels(4), levels(5)));
    run(
        &lua,
        &engine,
        r#"
        -- (a part, a Folder and a Model of 4 parts at the bottom of `Levels` instances of `Kind` nested one in another, built
        -- directly in the dom and attached to `Where`)
        local function Chain(Kind, Levels, Where)
            local Top = Instance.new("Folder")
            local Deepest = Top
            if Levels > 0 then
                Deepest = Instance.new(Kind)
                shared.Wrap(Deepest, Levels - 1, Kind).Parent = Top
            end
            Top.Parent = Where
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
    let pivot_same = compile(&lua, &engine, "local Name = ...; shared.PivotSame(Name)");
    call(&lua, &engine, &pivot_same, "Shallow");
    call(&lua, &engine, &pivot_same, "Deep");
    let (mut deep, mut shallow) = (0.0, 0.0);
    for _ in 0..3 {
        let start = std::time::Instant::now();
        call(&lua, &engine, &pivot_same, "Shallow");
        shallow = start.elapsed().as_secs_f64();
        let start = std::time::Instant::now();
        call(&lua, &engine, &pivot_same, "Deep");
        deep = start.elapsed().as_secs_f64();
        if deep < 2.0 * shallow + 0.02 {
            break;
        }
    }
    assert!(deep < 2.0 * shallow + 0.02, "2,000 PivotTo calls that moved nothing took {deep:.3} s of host time to a part below 10,000 WorldModels, {shallow:.3} s below none");
}

/// A part's write through a behaviour setter (lua/instance.rs assign) that leaves the part as it was costs what any write that
/// changes nothing costs, below WorldModels too, as a playing client's (Studio 0.747, below 0, 100 and 300 WorldModels in the
/// Workspace and in ReplicatedStorage: 0.05 to 1.2 us a write): CollisionGroup written the name it has, CollisionGroupId, Color
/// and Position written what they hold cost what Transparency written the value it holds does, below 300 WorldModels and below
/// 300 Folders, and CollisionGroup written "" a flat cost.rs COLLISION_GROUP_IGNORED more. Writes that change the part still pay
/// the WorldModels for their changes (2, 3 and 5). Each such write was priced as a change: 76 to 382 us below 300 WorldModels.
#[test]
fn a_parts_setter_write_that_changes_nothing_pays_nothing_more() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    let levels = |changes: u32| crate::cost::world_levels(300, changes);
    run(&lua, &engine, &format!("shared.Levels = {{ [0] = 0, {:e}, {:e}, {:e}, {:e}, {:e} }} shared.Ignored = {:e}", levels(1), levels(2), levels(3), levels(4), levels(5), crate::cost::COLLISION_GROUP_IGNORED));
    run(
        &lua,
        &engine,
        r#"
        -- (a part at the bottom of 300 instances of `Kind` nested one in another, made from the top down in `Where`, with the
        -- values it holds)
        local function Chain(Kind, Where)
            local Deepest = Where
            for _ = 1, 300 do
                local Level = Instance.new(Kind)
                Level.Parent = Deepest
                Deepest = Level
            end
            local Part = Instance.new("Part")
            Part.Anchored, Part.CanCollide, Part.Size, Part.CFrame = true, false, Vector3.one, CFrame.new(300, 300, 300)
            Part.Parent = Deepest
            return { Part = Part, Zero = 0, Name = "Default", Empty = "", Id = Part.CollisionGroupId, Color = Part.Color, At = Part.Position }
        end
        local Storage = game:GetService("ReplicatedStorage")
        local Scenes = {
            Worlds = Chain("WorldModel", workspace), Folders = Chain("Folder", workspace),
            LooseWorlds = Chain("WorldModel", Storage), LooseFolders = Chain("Folder", Storage),
        }
        -- what 10 writes of each kind cost the work clock, after one untimed (the frame's first of a kind runs its code cold)
        local function Cost(Scene, Write)
            Write(Scene, 1)
            local Start = os.clock()
            for I = 2, 11 do
                Write(Scene, I)
            end
            return os.clock() - Start
        end
        local Quiet = function(S) S.Part.Transparency = S.Zero end
        local Same = {
            { "CollisionGroup", 0, function(S) S.Part.CollisionGroup = S.Name end },
            { "CollisionGroup \"\"", shared.Ignored, function(S) S.Part.CollisionGroup = S.Empty end },
            { "CollisionGroupId", 0, function(S) S.Part.CollisionGroupId = S.Id end },
            { "Color", 0, function(S) S.Part.Color = S.Color end },
            { "Position", 0, function(S) S.Part.Position = S.At end },
        }
        -- (a CollisionGroup change out of the Workspace also pays the walk up that finds its WorldModel in the DataModel:
        -- services/physics.rs model_inside)
        local Changes = {
            { "CollisionGroup", 2, function(S, I) S.Part.CollisionGroup = if I % 2 == 0 then "ZzA" else "ZzB" end, true },
            { "Color", 3, function(S, I) S.Part.Color = if I % 2 == 0 then Color3.new(1, 0, 0) else Color3.new(0, 1, 0) end },
            { "Position", 5, function(S, I) S.Part.Position = Vector3.new(300, 300 + I % 2, 300) end },
        }
        for _, Pair in { { "Worlds", "Folders" }, { "LooseWorlds", "LooseFolders" } } do
            local Worlds, Folders = Scenes[Pair[1]], Scenes[Pair[2]]
            for _, Row in Same do
                local Base = Cost(Worlds, Quiet)
                local InWorlds, InFolders = Cost(Worlds, Row[3]), Cost(Folders, Row[3])
                assert(math.abs(InWorlds - InFolders) < 1e-9, `{Row[1]} written what it holds below 300 WorldModels ({Pair[1]}) cost {InWorlds} s where below Folders {InFolders}`)
                assert(math.abs(InWorlds - Base - 10 * Row[2]) < 0.2e-6, `{Row[1]} written what it holds ({Pair[1]}) cost {InWorlds} s for 10 where a Transparency write that changes nothing cost {Base}: {10 * Row[2]} s more expected`)
            end
            for _, Row in Changes do
                if Row[4] and Pair[1] ~= "Worlds" then
                    continue
                end
                local InWorlds, InFolders = Cost(Worlds, Row[3]), Cost(Folders, Row[3])
                local Expected = 10 * shared.Levels[Row[2]]
                assert(math.abs(InWorlds - InFolders - Expected) < 1e-9, `{Row[1]} changed below 300 WorldModels ({Pair[1]}) cost {InWorlds} s where below Folders {InFolders}: {Expected} s more expected`)
            end
        end
        "#,
    );
}

/// The physics step asks which world each loose part is in from the homes the dom keeps, and whether a parent holds a Humanoid
/// once a step for each parent: 200 loose parts at the bottom of a chain of 20,000 folders fly for 60 frames on about the host
/// time of 200 in a folder beside 20,000 others, and 5,000 loose parts in one folder for 30 on about that of 5,000 each in a
/// folder of its own. The step used to walk up from every loose part to the Workspace each frame (4 million steps a frame in the
/// first case), the first step from every instance in the Workspace (200 million), and look through every sibling of every loose
/// part for a Humanoid each frame (25 million in the second).
#[test]
fn free_flight_steps_cost_the_same_however_deep_or_wide_the_parts_are() {
    #[derive(Clone, Copy)]
    enum Layout {
        Deep,
        Flat,
        Wide,
        Spread,
    }
    let scene = |layout: Layout| {
        let (lua, engine) = start_play();
        let reflection = engine.reflection.clone();
        let folder_class = reflection.class_id("Folder").unwrap();
        let part_class = reflection.class_id("Part").unwrap();
        let size_prop = reflection.name_id("Size").unwrap();
        let collide_prop = reflection.name_id("CanCollide").unwrap();
        let cframe_prop = reflection.name_id("CFrame").unwrap();
        let workspace = engine.world().workspace;
        let (outer, first, last) = {
            let mut state = engine.state.borrow_mut();
            let make_part = |state: &mut crate::State, index: usize, parent: crate::InstanceId| {
                let id = crate::instance::create_named(state, &reflection, part_class, "Part");
                crate::instance::raw_set_silent(state, &reflection, id, size_prop, crate::value::Value::Vector3(rbx_dom_weak::types::Vector3::new(1.0, 1.0, 1.0)));
                crate::instance::raw_set_silent(state, &reflection, id, collide_prop, crate::value::Value::Bool(false));
                let cf = rbx_dom_weak::types::CFrame::new(rbx_dom_weak::types::Vector3::new((index as f32) * 3.0, 1000.0, 0.0), rbx_dom_weak::types::Matrix3::identity());
                crate::instance::raw_set_silent(state, &reflection, id, cframe_prop, crate::value::Value::Dom(Rc::new(rbx_dom_weak::types::Variant::CFrame(cf))));
                state.dom.attach(id, parent);
                id
            };
            let make_parts = |state: &mut crate::State, count: usize, parent: crate::InstanceId| {
                let first = make_part(state, 1, parent);
                let mut last = first;
                for index in 2..=count {
                    last = make_part(state, index, parent);
                }
                (first, last)
            };
            match layout {
                Layout::Deep => {
                    let mut outer = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                    let (first, last) = make_parts(&mut state, 200, outer);
                    for _ in 0..20_000 {
                        let folder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                        state.dom.attach(outer, folder);
                        outer = folder;
                    }
                    (outer, first, last)
                }
                Layout::Flat => {
                    let outer = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                    let inner = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                    let (first, last) = make_parts(&mut state, 200, inner);
                    state.dom.attach(inner, outer);
                    for _ in 0..20_000 {
                        let folder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                        state.dom.attach(folder, outer);
                    }
                    (outer, first, last)
                }
                Layout::Wide => {
                    let folder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                    let (first, last) = make_parts(&mut state, 5_000, folder);
                    (folder, first, last)
                }
                Layout::Spread => {
                    let outer = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                    let mut first = None;
                    let mut last = None;
                    for index in 1..=5_000 {
                        let folder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
                        let part = make_part(&mut state, index, folder);
                        first.get_or_insert(part);
                        last = Some(part);
                        state.dom.attach(folder, outer);
                    }
                    (outer, first.unwrap(), last.unwrap())
                }
            }
        };
        engine_lua::instance::set_parent(&engine, outer, Some(workspace)).expect("attach to workspace");
        (lua, engine, first, last)
    };
    // (each pair stepped in turn, a frame each, so whatever else the host does weighs on both alike; up to three tries, each pair
    // made afresh, until one is within the bound: a busy host that takes the thread away for a few milliseconds slows one try's
    // steps, a step that walks the tree slows every try's)
    let within = |[host, base]: [f64; 2]| host < 2.0 * base + 0.05;
    let fly = |layouts: [Layout; 2], frames: usize, below: f64| {
        let mut host = [0.0; 2];
        for _ in 0..3 {
            let scenes = layouts.map(scene);
            host = [0.0; 2];
            for _ in 0..frames {
                for (index, (lua, engine, _, _)) in scenes.iter().enumerate() {
                    let start = std::time::Instant::now();
                    crate::scheduler::step(lua, engine, Some(1.0 / 60.0)).expect("step");
                    host[index] += start.elapsed().as_secs_f64();
                }
            }
            for (_, engine, first, last) in &scenes {
                let state = engine.state.borrow();
                for &part in &[first, last] {
                    let y = super::world_cframe(&state, engine, *part).translation.y as f64;
                    assert!(y < below, "the part fell to {y}");
                }
            }
            if within(host) {
                break;
            }
        }
        host
    };
    let [deep, flat] = fly([Layout::Deep, Layout::Flat], 60, 950.0);
    assert!(within([deep, flat]), "60 frames of 200 parts flying 20,000 deep took {deep:.3} s of host time, {flat:.3} s beside 20,000 folders");

    let [wide, spread] = fly([Layout::Wide, Layout::Spread], 30, 990.0);
    assert!(within([wide, spread]), "30 frames of 5,000 parts flying in one folder took {wide:.3} s of host time, {spread:.3} s each in a folder of its own");
}

/// A script cannot overflow the engine's stack with a deep tree: the engine's walks of the tree keep stacks of their own. On a
/// thread with a 512 KiB stack (a recursion over these chains overflows it at 18 bytes a level, 180 for the one destroyed):
/// a chain of 30,000 Folders with a Part at the bottom holding 15,000 nested Attachments and 15,000 nested Bones below them, a
/// CurveAnimation of 30,000 nested Folders, and a chain of 3,000 Folders, each moved into ReplicatedStorage, and a model of
/// 30,000 nested Folders imported there lazily (its instances made as a script reaches them), with a Studio place's hierarchy
/// loaded. The script first lists the bottom part's children (its Studio node worked out up the whole chain), then searches the chain
/// (FindFirstChild and FindFirstChildWhichIsA recursive, QueryDescendants) and the import's plan, clones the chain, reads the bottom bone's and
/// attachment's world frames and moves the chain into the Workspace (AncestryChanged's order, the move's price), where it is stepped
/// (the physics step's first walk) and queried (the spatial index), then out; it reads the clip's length and the KeyframeSequence
/// made of it, and destroys the last chain (a destroy unparents each child in turn, so it takes as long as the chain's depth
/// squared). Nothing lists the deep chains: wrapping an instance for a script walks up to its root.
#[test]
fn a_tree_deeper_than_a_stack_holds_frames_for_runs() {
    // Run on a 512 KiB stack with 30,000-deep chains (overflows at 18 bytes/level = 540 KiB) and a
    // 3,000-deep destroy chain (overflows at 180 bytes/level = 540 KiB).
    const DEPTH: usize = 30_000;
    const HALF: usize = DEPTH / 2;
    const DESTROYED: usize = 3_000;
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
    let thread = std::thread::Builder::new().name("a deep tree on a 512 KiB stack".into()).stack_size(512 << 10).spawn(body).expect("thread");
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

/// An assembly's mass and velocities read as a playing client's (Studio 0.747, client and server; tools/parity/probes/
/// assembly_mass.luau): inf for an assembly holding an anchored part, from the write's own frame, its centre still its parts';
/// each anchored part its own assembly; a Massless part weighing as its assembly's root or outside every world; and a velocity
/// kept as the root's with the spin, so a velocity written and a spin after it reads back 1.99999988 for 2.
#[test]
fn an_assembly_reads_its_mass_and_velocity_as_a_client_does() {
    check_running(
        "play",
        r#"
        workspace.Gravity = 0
        local function part(name, parent, at, size, anchored, massless)
            local P = Instance.new("Part")
            P.Name, P.Size, P.Anchored, P.Massless, P.CanCollide = name, size, anchored, massless == true, false
            P.CFrame = at
            P.Parent = parent
            return P
        end
        local function weld(a, b)
            local W = Instance.new("Weld")
            W.Part0, W.Part1, W.C0 = a, b, a.CFrame:ToObjectSpace(b.CFrame)
            W.Parent = a
        end
        local At = CFrame.new(300, 60, 40)
        local Torso = part("Torso", workspace, At, Vector3.new(2, 2, 1), true)
        local Arm = part("Arm", workspace, At * CFrame.new(1.5, 0, 0), Vector3.new(1, 2, 1), false)
        local Hand = part("Hand", workspace, At * CFrame.new(1.5, -1.5, 0), Vector3.new(1, 1, 1), false)
        weld(Torso, Arm)
        weld(Arm, Hand)
        for _, P in { Torso, Arm, Hand } do
            assert(P.AssemblyMass == math.huge and tostring(P.AssemblyMass) == "inf", `anchored {P.Name} {P.AssemblyMass}`)
            assert(P.AssemblyCenterOfMass == Torso.AssemblyCenterOfMass, "one centre")
        end
        assert(string.format("%.9g", Torso.AssemblyCenterOfMass.X) == "300.642853", `centre {Torso.AssemblyCenterOfMass}`)
        Torso.Anchored = false
        assert(string.format("%.17g", Hand.AssemblyMass) == "4.8999996185302734", `unanchored {Hand.AssemblyMass}`)
        Hand.Anchored = true
        assert(Arm.AssemblyMass == math.huge and Arm.AssemblyRootPart == Hand, "anchored in the write's frame")
        Hand.Anchored = false
        task.wait()
        Arm.AssemblyLinearVelocity = Vector3.new(0, 0, 2)
        Hand.AssemblyAngularVelocity = Vector3.new(0, 1, 0)
        assert(string.format("%.9g", Torso.AssemblyLinearVelocity.Z) == "1.99999988", `linear {Torso.AssemblyLinearVelocity}`)
        Arm.Velocity = Vector3.new(0, 5, 0)
        assert(Torso.Velocity == Vector3.new(0, 5, 0) and Hand.Velocity ~= Torso.Velocity, "Velocity is the root's, each part its own")
        -- two anchored parts welded together are two assemblies
        local A = part("A", workspace, CFrame.new(320, 60, 40), Vector3.new(2, 2, 2), true)
        local B = part("B", workspace, CFrame.new(323, 60, 40), Vector3.new(4, 2, 2), true)
        local C = part("C", workspace, CFrame.new(320, 62, 40), Vector3.new(1, 2, 1), false)
        weld(A, B)
        weld(A, C)
        assert(C.AssemblyCenterOfMass == A.AssemblyCenterOfMass and B.AssemblyCenterOfMass == B.Position, "each anchored part's own")
        -- Massless: a root's and an unworldly part's count
        local Lone = part("Lone", workspace, CFrame.new(340, 60, 40), Vector3.new(2, 1, 2), false, true)
        assert(math.abs(Lone.Mass - 2.8) < 1e-6 and Lone:GetMass() == Lone.Mass and Lone.AssemblyMass == Lone.Mass, "a lone Massless part")
        local Big = part("Big", workspace, CFrame.new(342, 60, 40), Vector3.new(2, 1, 3), false, true)
        weld(Lone, Big)
        assert(Lone.Mass == 0 and Big.Mass > 0 and Lone.AssemblyMass == Big.Mass, "the all-Massless assembly's root weighs")
        local Loose = part("Loose", nil, CFrame.new(360, 60, 40), Vector3.new(2, 1, 2), true, true)
        assert(Loose.AssemblyRootPart == nil and math.abs(Loose.Mass - 2.8) < 1e-6 and Loose.AssemblyMass == Loose.Mass, "outside every world")
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
        Options { mode: "play".into(), profile: crate::client_profile::ClientProfile::parse(r#"{"game":{"streamingTargetRadius":400}}"#).unwrap(), ..Options::default() },
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
        Options { mode: "play".into(), profile: crate::client_profile::ClientProfile::parse(r#"{"game":{"streamingTargetRadius":400}}"#).unwrap(), ..Options::default() },
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
#[inline(never)]
pub(super) fn start_play() -> (Lua, Rc<Engine>) {
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
        (out, super::query::index_len(&engine))
    };
    let mut hits = 0;
    for seed in [7, 1234, 99991, 31337, 2024, 555] {
        let ((tree, parts), (scan, _)) = (outputs(seed, false), outputs(seed, true));
        assert!(parts > 150, "seed {seed}: {parts} parts");
        assert_eq!(tree.lines().count(), scan.lines().count(), "seed {seed}");
        for (a, b) in tree.lines().zip(scan.lines()) {
            assert_eq!(a, b, "seed {seed}");
        }
        hits += tree.lines().filter(|line| !line.contains(" nil ") && !line.contains("[]") && !line.contains("Cutoff 0 ") && !line.contains("error")).count();
    }
    eprintln!("{hits} lines found parts");
    assert!(hits > 1500, "the queries found parts ({hits} lines)");
}

/// Spatial queries list and pick parts as a client's grid walk gives them (world/grid.rs; Studio 0.747, a playing client and a
/// server alike, tools/parity/probes/grid_*.luau): a cell's unanchored parts before its anchored ones, each in the order they came
/// in; a part leaving gives its place to the cell's last; a part moved out of its cell and back, or anchored and freed, before a
/// query is last; a subtree comes in the moved part first, then each part after its descendants; parts of the levels above the
/// query's come last; a query's low edge on a cell edge walks the cell below; welded parts come in root first and a weld made later
/// files them again; the casts take a cell's anchored parts first among exact ties; a box is clamped to 7,500 studs about its
/// centre and a ray cut at 15,000; nothing is found past the far bound; and a WorldModel walks a grid of its own, which the
/// Workspace's queries never meet.
#[test]
fn queries_list_parts_in_a_clients_grid_order() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        workspace.Gravity = 0
        local Root = Instance.new("Folder")
        Root.Parent = workspace
        local Mine = {}
        local V = Vector3.new
        local function Part(Name, At, Size, Anchored, Parent)
            local Made = Instance.new("Part")
            Made.Name = Name
            Made.Anchored = Anchored ~= false
            Made.Size = Size or V(0.2, 0.2, 0.2)
            Made.CFrame = CFrame.new(At)
            Made.Parent = Parent or Root
            Mine[Made] = true
            return Made
        end
        local function Index() workspace:GetPartBoundsInBox(CFrame.new(0, -10000, 0), Vector3.one) end
        local function Names(List)
            local Out = {}
            for _, Found in List do
                if Mine[Found] then table.insert(Out, Found.Name) end
            end
            return table.concat(Out, " ")
        end
        local function Clear() Root:ClearAllChildren() table.clear(Mine) end
        local C0 = V(2004, 1004, 2004)
        local function Q(Size) return Names(workspace:GetPartBoundsInBox(CFrame.new(C0), Size or V(6, 1, 1))) end
        local function Is(Label, Got, Want) assert(Got == Want, `{Label}: {Got}, a client lists {Want}`) end
        -- one cell: unanchored first, each list in the order the parts came
        Part("a1", C0) Part("u2", C0 + V(0.5, 0, 0), nil, false) Part("a3", C0 + V(1, 0, 0)) Part("u4", C0 + V(1.5, 0, 0), nil, false)
        Index()
        Is("one cell", Q(V(4, 1, 1)), "u2 u4 a1 a3")
        Clear()
        local function Five()
            local A = {}
            for I = 1, 5 do A[I] = Part("a" .. I, C0 + V(-2 + I * 0.6, 0, 0)) end
            Index()
            return A
        end
        local A = Five() A[2]:Destroy() Index() A[1]:Destroy()
        Is("a2 destroyed, then a1", Q(), "a4 a5 a3") Clear()
        A = Five() A[2].CFrame += V(20, 0, 0) A[2].CFrame -= V(20, 0, 0)
        Is("a2 moved out and back, no query between", Q(), "a1 a5 a3 a4 a2") Clear()
        A = Five() A[2].Anchored = false A[2].Anchored = true
        Is("a2 unanchored and anchored again", Q(), "a1 a5 a3 a4 a2") Clear()
        A = Five() A[2].CanCollide = false A[2].CanTouch = false
        Is("a2 CanCollide and CanTouch off", Q(), "a1 a5 a3 a4 a2") Clear()
        A = Five() A[2].CFrame += V(0, 0, 0.3) A[2].Transparency = 0.5 A[2].Parent = Instance.new("Folder", Root)
        Is("a2 moved within its cell, written and moved in the tree", Q(), "a1 a2 a3 a4 a5") Clear()
        -- a subtree comes in each part after its descendants (the moved Folder is no part)
        local T = Instance.new("Folder")
        Part("a", C0 + V(-0.9, 0, 0), nil, true, T)
        local M = Instance.new("Model", T)
        Part("b", C0 + V(-0.5, 0, 0), nil, true, M)
        local c = Part("c", C0 + V(-0.1, 0, 0), nil, true, M)
        Part("d", C0 + V(0.3, 0, 0), nil, true, c) Part("f", C0 + V(0.7, 0, 0), nil, true, c) Part("e", C0 + V(0.9, 0, 0), nil, true, T)
        T.Parent = Root
        Index()
        Is("subtree", Q(V(4, 1, 1)), "a b d f c e") Clear()
        -- welded unanchored parts come in root first (of equal parts the later made); a weld made later files both again
        Part("a1", C0) local U2 = Part("u2", C0 + V(0.5, 0, 0), nil, false) Part("a3", C0 + V(1, 0, 0)) local U4 = Part("u4", C0 + V(1.5, 0, 0), nil, false)
        local W = Instance.new("WeldConstraint") W.Part0, W.Part1 = U2, U4 W.Parent = U2
        Part("u5", C0 + V(-0.5, 0, 0), nil, false)
        Index()
        Is("welded", Q(V(4, 1, 1)), "u4 u2 u5 a1 a3")
        U2.Anchored = true
        Is("then u2 anchored", Q(V(4, 1, 1)), "u5 a1 a3 u2 u4") Clear()
        Part("a1", C0) U2 = Part("u2", C0 + V(0.5, 0, 0), nil, false) U4 = Part("u4", C0 + V(1.5, 0, 0), nil, false) Part("u5", C0 + V(-0.5, 0, 0), nil, false)
        Index()
        W = Instance.new("WeldConstraint") W.Part0, W.Part1 = U2, U4 W.Parent = U2
        Is("welded after the take-in", Q(V(4, 1, 1)), "u5 u4 u2 a1") Clear()
        -- the casts take a cell's anchored parts first among exact ties, the box queries its unanchored ones
        local Tied = { Part("A", C0, V(2, 2, 2)), Part("B", C0, V(2, 2, 2), false), Part("C", C0, V(2, 2, 2)) }
        Index()
        local From = C0 + V(0, 10, 0)
        Is("tied ray", workspace:Raycast(From, V(0, -20, 0)).Instance.Name, "A")
        Is("tied box", Names(workspace:GetPartBoundsInBox(CFrame.new(C0), V(0.5, 0.5, 0.5))), "B A C")
        Tied[1]:Destroy()
        Is("tied ray, A destroyed", workspace:Raycast(From, V(0, -20, 0)).Instance.Name, "C") Clear()
        -- the levels: a 30-stud part (level 1) after the level-0 parts a small query meets first
        Part("L1", C0 + V(0, -14, 0), V(30, 30, 30)) Part("L0", C0, V(2, 2, 2))
        Index()
        Is("levels, box", Names(workspace:GetPartBoundsInBox(CFrame.new(C0), V(0.5, 0.5, 0.5))), "L0 L1")
        Is("levels, ray", workspace:Raycast(From, V(0, -20, 0)).Instance.Name, "L0")
        Is("levels, sphere", workspace:Spherecast(From, 0.5, V(0, -20, 0)).Instance.Name, "L1") Clear()
        -- a query's low edge on a cell edge also walks the cell below
        Part("M", V(2052, 1004, 1004)) Part("P", V(2048, 1004, 1004))
        Index()
        Is("low edge", Names(workspace:GetPartBoundsInBox(CFrame.new(2051, 1004, 1004), V(6, 2, 2))), "P M")
        Is("low edge past it", Names(workspace:GetPartBoundsInBox(CFrame.new(2051.0005, 1004, 1004), V(5.999, 2, 2))), "M P") Clear()
        -- the clamp, the cut and the far bound
        local Cl = V(100004, 1004, 4)
        Part("7500", Cl + V(7500, 0, 0), V(0.01, 1, 1)) Part("7500.5", Cl + V(7500.5, 0, 0), V(0.01, 1, 1))
        Index()
        Is("clamp", Names(workspace:GetPartBoundsInBox(CFrame.new(Cl), V(20000, 4, 4))), "7500")
        Is("radius clamp", Names(workspace:GetPartBoundsInRadius(Cl, 10000)), "7500") Clear()
        local Near, Past = Part("near", Cl + V(14999.9 + 0.1, 0, 0), V(0.2, 4, 4)), nil
        Index()
        Is("ray before the cut", tostring(workspace:Raycast(Cl, V(1e6, 0, 0)) ~= nil), "true") Clear()
        Past = Part("past", Cl + V(15000.1, 0, 0), V(0.2, 4, 4))
        Index()
        Is("ray past the cut", tostring(workspace:Raycast(Cl, V(1e6, 0, 0))), "nil") Clear()
        local T26 = 2 ^ 26
        Part("in", V(T26, 0, 300), V(2, 2, 2)) Part("out", V(T26 + 8, 0, 340), V(2, 2, 2))
        Index()
        Is("far bound", Names(workspace:GetPartBoundsInBox(CFrame.new(T26, 0, 300), Vector3.one)) .. "|" .. Names(workspace:GetPartBoundsInBox(CFrame.new(T26 + 8, 0, 340), Vector3.one)), "in|") Clear()
        -- a WorldModel's own grid
        local World = Instance.new("WorldModel")
        World.Parent = workspace
        for _, S in { 64, 32, 1 } do Part("K" .. S, C0, V(S, S, S), true, World) end
        Is("WorldModel", Names(World:GetPartBoundsInBox(CFrame.new(C0), V(0.01, 0.01, 0.01))), "K1 K32 K64")
        Is("the Workspace beside it", Names(workspace:GetPartBoundsInBox(CFrame.new(C0), V(0.01, 0.01, 0.01))), "")
        Is("its ray", World:Raycast(C0 + V(0, 100, 0), V(0, -200, 0)).Instance.Name, "K64")
        Is("the Workspace's ray", tostring(workspace:Raycast(C0 + V(0, 100, 0), V(0, -200, 0))), "nil")
        World:Destroy()
        "##,
    );
    super::query::index_matches_a_walk(&engine).unwrap();
}

/// A Parent write files the moved subtree's parts into a grid's cells, and out of them, in the order a client takes the subtree
/// up (joints.rs taken_up; Studio 0.747, a playing client and a server alike, tools/parity/probes/grid_subtree.luau): the moved
/// part first, then the rest each after its own descendants. So a part p holding q(r) and s comes into a cell as p r q s, made in
/// nil or coming back, from nil or from ReplicatedStorage, into the Workspace or a WorldModel; a moved Model is no part, so its
/// parts come each after its descendants. It leaves in the same order, each leaving part giving its place to the cell's last:
/// p(q(r), s) leaving x p q r s a b c d e leaves x e c d b a, where each part after its descendants left x b d e c a.
#[test]
fn a_moved_subtree_comes_into_and_leaves_the_grid_as_a_client_takes_it_up() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        workspace.Gravity = 0
        local Storage = game:GetService("ReplicatedStorage")
        local Root = Instance.new("Folder")
        Root.Parent = workspace
        local Mine = {}
        local V = Vector3.new
        local C0 = V(2004, 1004, 2004)
        local Slot = 0
        local function Part(Name, Parent)
            local Made = Instance.new("Part")
            Made.Name, Made.Anchored, Made.Size = Name, true, V(0.2, 0.2, 0.2)
            Slot += 1
            Made.CFrame = CFrame.new(C0 + V(-2.6 + Slot * 0.3, 0, 0))
            Made.Parent = Parent
            Mine[Made] = true
            return Made
        end
        local function Index(World) (World or workspace):GetPartBoundsInBox(CFrame.new(0, -10000, 0), Vector3.one) end
        local function Q(World)
            local Out = {}
            for _, Found in (World or workspace):GetPartBoundsInBox(CFrame.new(C0), V(6, 1, 1)) do
                if Mine[Found] then table.insert(Out, Found.Name) end
            end
            return table.concat(Out, " ")
        end
        local function Clear() Root:ClearAllChildren() table.clear(Mine) Slot = 0 end
        local function Is(Label, Got, Want) assert(Got == Want, `{Label}: {Got}, a client lists {Want}`) end
        -- p(q(r), s), or M(p(q), r), each instance parented as it is made
        local function Tree(Parent)
            local P = Part("p", Parent)
            Part("r", Part("q", P))
            Part("s", P)
            return P
        end
        local function ModelTree(Parent)
            local M = Instance.new("Model")
            M.Parent = Parent
            Part("q", Part("p", M))
            Part("r", M)
            return M
        end
        -- coming in, after x
        local World = Instance.new("WorldModel")
        World.Parent = workspace
        local function In(Label, Make, Move, Want, Into)
            Part("x", Into or Root)
            Index(Into)
            Move(Make(nil), Into or Root)
            Index(Into)
            Is(Label, Q(Into), Want)
            Root:ClearAllChildren() World:ClearAllChildren() table.clear(Mine) Slot = 0
        end
        In("from nil", Tree, function(T, To) T.Parent = To end, "x p r q s")
        In("back from nil, a query between", Tree, function(T, To) T.Parent = To Index() T.Parent = nil Index() T.Parent = To end, "x p r q s")
        In("back from ReplicatedStorage, no query between", Tree, function(T, To) T.Parent = To Index() T.Parent = Storage T.Parent = To end, "x p r q s")
        In("a Model from nil", ModelTree, function(T, To) T.Parent = To end, "x q p r")
        In("into a WorldModel", Tree, function(T, To) T.Parent = To end, "x p r q s", World)
        -- going out, from x, the tree made in place, a b c d e
        local function Out(Label, Make, Move, Want)
            Part("x", Root)
            local T = Make(Root)
            for _, Name in { "a", "b", "c", "d", "e" } do Part(Name, Root) end
            Index()
            Move(T)
            Index()
            Is(Label, Q(), Want)
            T:Destroy()
            Clear()
        end
        Out("to nil", Tree, function(T) T.Parent = nil end, "x e c d b a")
        Out("destroyed", Tree, function(T) T:Destroy() end, "x e c d b a")
        Out("into a WorldModel", Tree, function(T) T.Parent = World end, "x e c d b a")
        Out("a Model to nil", ModelTree, function(T) T.Parent = nil end, "x d e c a b")
        World:Destroy()
        "##,
    );
    super::query::index_matches_a_walk(&engine).unwrap();
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
    let parts = super::query::index_len(&engine);
    assert!(parts == 50_002, "{parts} parts");
}

/// The broadphase orders what a query keeps as the Workspace's descendants are listed (`Dom::descendants`), from each instance's
/// place among its siblings: after instances are made, moved within and out of the Workspace and back, destroyed, and one replaced in its
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
/// parts' paths meet, puts any list of instances in exactly the order its first form did (each one's whole list of places
/// from below the root, compared as lists in a stable sort: broadphase::sort_by_order_lists): random trees with folder chains up
/// to 3,000 deep, parts under parts, models and folders moved, destroyed and parented out of the Workspace, and a folder
/// replaced in its place (dom.rs replace_child), whose detached old self keeps the place its replacement takes, so a list
/// holding parts of both has two tops with one number and the trie with one node for each list (Trie::keyed) orders it; random
/// lists of 2 to 400 instances with repeats, the Workspace itself and instances outside it among them, against the Workspace
/// and the DataModel as the root. Its walks take a step onto each instance on the paths at most, and one more for each part where
/// its walk meets another's; parts in one folder at the bottom of a deep chain take one step each.
#[test]
fn query_order_is_the_order_of_whole_attach_number_lists() {
    let (lua, engine) = start_play();
    register_wrap(&lua, &engine);
    run(
        &lua,
        &engine,
        r##"
        local Wrap = shared.Wrap
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
                Top = Wrap(Top, math.floor(R() * 3000))
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
        Wrap(Bottom, 2000).Parent = workspace
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
        assert_eq!(state.dom.sibling_place(old), state.dom.sibling_place(new), "the replacement takes the old folder's place");
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
            items.extend(state.dom.at(state.dom.at(workspace).children.iter().copied().find(|&child| state.dom.sibling_place(child) == state.dom.sibling_place(replaced)).expect("the replacement")).children.iter().copied());
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

/// A spatial query lists what it keeps in its grid walk's order (world/grid.rs), which knows nothing of the tree, so 2,000 parts in a
/// folder at the bottom of a 10,000-deep chain cost GetPartBoundsInBox, GetPartsObscuringTarget and a Raycast through them about as
/// much host work as the same parts in a folder of the Workspace, in the same order, and the work clock charges the box and the
/// Raycast the same, as a playing client's cost the same at any depth (Studio 0.747: 100 parts 12.1 us in the Workspace, 12.2 1,000
/// deep, 12.5 3,000 deep); GetPartsObscuringTarget costs more by the walks a client's casts take up its parts (camera.rs
/// parts_obscuring: its ancestors' answers kept for the call, so the host walks them once). Parts whose paths meet only far up (50
/// parts, each at the bottom of a 1,000-deep chain of its own) cost what the same parts in a folder cost: the walks up from them that
/// put them in the Workspace's order before (cost.rs QUERY_ORDER_STEP) are gone with that order.
#[test]
fn query_order_walks_the_tree_only_up_to_where_the_parts_paths_meet() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    register_wrap(&lua, &engine);
    run(
        &lua,
        &engine,
        r#"
        local Wrap = shared.Wrap
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
        -- (at multiples of the top level's cells, so that each scene's walks meet the same cells: a client's walk costs by its cells)
        shared.Scenes = { Flat = Scene(0, 2000, 0), Deep = Scene(1024, 2000, 10000), Few = Scene(2048, 50, 0), Apart = Scene(3072, 50, 0, 1000) }
        workspace:Raycast(Vector3.new(0, 5000, 0), Vector3.new(0, 1, 0))
        shared.Work = {}
        local Camera = workspace.CurrentCamera
        local function Pass(S)
            local X, Count = S.X, #S.Parts
            for _ = 1, 5 do
                local Found = workspace:GetPartBoundsInBox(CFrame.new(X, 50, -Count * 0.005), Vector3.new(0.5, 0.5, Count * 0.01 + 1))
                assert(#Found == Count, `the box found {#Found} of {Count}`)
                local Order = {}
                for I, Part in Found do
                    Order[I] = table.find(S.Parts, Part)
                end
                S.Order = table.concat(Order, " ")
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
    let use_scene = compile(&lua, &engine, "local Name = ...; shared.Use(Name)");
    // Pause incremental GC during each warmup+timed pair so a sweep cannot free `table.concat`'s
    // temporary integer strings between the two passes and charge re-interning (`LUAU_NEW_STRING`) to one scene.
    let run_scene = |name: &str| {
        lua.gc_collect().expect("gc");
        lua.gc_stop();
        let start = std::time::Instant::now();
        call(&lua, &engine, &use_scene, name);
        let elapsed = start.elapsed().as_secs_f64();
        lua.gc_restart();
        elapsed
    };
    let (mut flat, mut deep) = (0.0, 0.0);
    for _ in 0..3 {
        flat = run_scene("Flat");
        deep = run_scene("Deep");
        if deep < 2.0 * flat + 0.1 {
            break;
        }
    }
    let _ = run_scene("Few");
    let _ = run_scene("Apart");
    run(
        &lua,
        &engine,
        r#"
        local Work, Scenes = shared.Work, shared.Scenes
        assert(Work.Flat > 0, "the passes cost work")
        assert(math.abs(Work.Deep - Work.Flat) < 1e-9, `the deep scene's pass cost {Work.Deep} s of work where the flat one's {Work.Flat}`)
        assert(math.abs(Work.Apart - Work.Few) < 1e-9, `the parts apart cost {Work.Apart - Work.Few} s more work than the few in a folder`)
        assert(Scenes.Deep.Order == Scenes.Flat.Order and Scenes.Apart.Order == Scenes.Few.Order, "the same order however deep")
        "#,
    );
    // GetPartsObscuringTarget's five calls: the wall at place i is checked 2,000 - i + 1 times, each walking up for the i walls listed
    // before it, 3 ancestors in the Workspace's folder and 10,003 at the bottom of the chain
    let count = 2000usize;
    let checks: f64 = (0..count).map(|place| ((count - place + 1) * place) as f64).sum();
    let walks = |ancestors: u32| checks * f64::from(ancestors) * crate::cost::filter_step(ancestors) * crate::cost::OBSCURING_STEP;
    let deeper = 5.0 * (walks(10_003) - walks(3));
    run(&lua, &engine, &format!("assert(math.abs((shared.Obscuring.Deep - shared.Obscuring.Flat) / {deeper:e} - 1) < 1e-9, `GetPartsObscuringTarget deep cost {{shared.Obscuring.Deep - shared.Obscuring.Flat}} s more than in the Workspace, not {deeper:e}`)"));
    assert!(deep < 2.0 * flat + 0.1, "two passes of 15 queries of 2,000 parts 10,000 deep took {deep:.3} s, {flat:.3} s in the Workspace");
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
        -- (a list refused at an entry after ten Instances: the ten read, and the error)
        local Ten = table.move(Pool, 1, 10, 1, table.create(11))
        local Refused = table.clone(Ten)
        Refused[11] = 5
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
            Refused = Cost(function() pcall(function() Ray.ExcludeInstances = Refused end) end) - Cost(function() pcall(function() Ray.ExcludeInstances = Ten end) end),
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
        ("Refused", crate::cost::RAISED),
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
/// A script's own errors, the datatypes' and the proxy values' errors, refused property writes, failed conversions and the error a
/// Connect logs cost the work clock what a playing client spends on them (cost.rs SCRIPT_ERROR, MEMBER_MISSED, WRITE_REFUSED,
/// WRITE_CAST_REFUSED, CONVERSION_FAILED, CONNECT_FAILED_LOGGED; Studio 0.747, a headless client, its server and a rendered client:
/// error() 1.86-1.93 us, a refused write 4.34-4.87, a wrong-typed one 7.47-7.83, "x" for Transparency 4.58-4.99, Connect(nil) 7.24-7.61
/// past the call), each against the same work going on; an engine error still costs RAISED in all. The VM's error count is
/// patches/luau/29-script-error-work.patch. Before, the script's and datatypes' errors cost nothing and the writes RAISED alone.
#[test]
fn script_errors_refused_writes_and_failed_conversions_cost_what_a_client_spends() {
    let (lua, engine) = start_play();
    crate::scheduler::step(&lua, &engine, Some(1.0 / 60.0)).expect("step");
    run(
        &lua,
        &engine,
        r##"
        local Folder = Instance.new("Folder")
        Folder.Name = "F"
        Folder.Parent = workspace
        local Child = Instance.new("Folder")
        Child.Parent = Folder
        local Part = Instance.new("Part")
        Part.Parent = Folder
        local Value = Instance.new("IntValue")
        local Params = RaycastParams.new()
        local Missing, Text, Number, Zero, Fn, V, Size = nil, "x", 5, 0, function() end, Vector3.new(1, 2, 3), Part.Size
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
            Error = Extra(function() error(Text) end, function() end),
            Assert = Extra(function() assert(false) end, function() end),
            Arithmetic = Extra(function() return Missing + 1 end, function() end),
            Library = Extra(function() return math.floor(Text) end, function() return math.floor(Zero) end),
            Operator = Extra(function() return V + Text end, function() return V + V end),
            VectorMember = Extra(function() return V.Nope end, function() return V.X end),
            ProxyMember = Extra(function() return Params.Nope end, function() return Params.IgnoreWater end),
            ReadOnly = Extra(function() Folder.ClassName = Text end, function() Folder.Name = "F" end),
            EnumItem = Extra(function() Part.Material = Text end, function() Part.Material = Enum.Material.Plastic end),
            Circular = Extra(function() Folder.Parent = Child end, function() Folder.Parent = workspace end),
            WrongType = Extra(function() Part.Size = Text end, function() Part.Size = Size end),
            WrongString = Extra(function() Folder.Name = Fn end, function() Folder.Name = "F" end),
            NumberText = Extra(function() Part.Transparency = Text end, function() Part.Transparency = "0" end),
            IntValueText = Extra(function() Value.Value = Text end, function() Value.Value = Zero end),
            NumberNil = Extra(function() Part.Transparency = Missing end, function() Part.Transparency = Zero end),
            StringParameter = Extra(function() Folder:FindFirstChild({}) end, function() Folder:FindFirstChild(Text) end),
            Connect = Extra(function() Part.Changed:Connect(Missing) end, function() end),
        }
        "##,
    );
    let script = crate::cost::SCRIPT_ERROR;
    let checks = [
        ("Error", script),
        ("Assert", script),
        ("Arithmetic", script),
        ("Library", script),
        ("Operator", script),
        ("VectorMember", crate::cost::MEMBER_MISSED),
        ("ProxyMember", crate::cost::MEMBER_MISSED),
        ("ReadOnly", crate::cost::RAISED + crate::cost::WRITE_REFUSED - crate::cost::WRITE_QUIET),
        ("EnumItem", crate::cost::RAISED + crate::cost::WRITE_REFUSED - crate::cost::WRITE_QUIET),
        ("Circular", crate::cost::RAISED + crate::cost::WRITE_REFUSED),
        ("WrongType", crate::cost::RAISED + crate::cost::WRITE_CAST_REFUSED - crate::cost::WRITE_QUIET),
        ("WrongString", crate::cost::RAISED + crate::cost::WRITE_CAST_REFUSED - crate::cost::WRITE_QUIET),
        ("NumberText", crate::cost::CONVERSION_FAILED),
        ("IntValueText", crate::cost::CONVERSION_FAILED),
        ("NumberNil", crate::cost::CONVERSION_FAILED),
        ("StringParameter", crate::cost::CONVERSION_FAILED),
        ("Connect", crate::cost::CONNECT_FAILED_LOGGED),
    ];
    // (within what the passing work does past its arguments and the messages' own strings: 0.2 us at most)
    for (name, price) in checks {
        run(&lua, &engine, &format!("assert(math.abs(shared.Errors.{name} - {price:e}) < 0.2e-6, `{name}: {{shared.Errors.{name}}} s more work, not {price:e}`)"));
    }
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
    // what a client spends on such geometry beyond the call (cost.rs QUERY_NOT_FINITE, QUERY_SIDE_NOT_FINITE), less what the finite query's
    // walk pays and the one that finds nothing does not (its 4 root cells: QUERY_ROOT_CELL); a NaN side is a box of no width, as many cells
    let walk = 4.0 * crate::cost::QUERY_ROOT_CELL;
    for (name, price) in [("Frame", crate::cost::QUERY_NOT_FINITE - walk), ("Side", crate::cost::QUERY_SIDE_NOT_FINITE), ("Radius", crate::cost::QUERY_SIDE_NOT_FINITE - walk), ("Position", crate::cost::QUERY_NOT_FINITE - walk)] {
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
        assert(workspace:Raycast(Vector3.new(30, 50, 0), Down * 2).Instance == Part, "CanQuery off, CanCollide on")
        Part.CanCollide = false
        assert(workspace:Raycast(Vector3.new(30, 50, 0), Down * 2) == nil, "CanQuery and CanCollide off")
        Part.CanQuery, Part.CanCollide = true, true
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
    super::query::index_matches_a_walk(&engine).unwrap();
    let parts = super::query::index_len(&engine);
    assert!(parts == 1 + 5 + 2999, "{parts} parts");
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
    let make_scene = |non_finite: bool| {
        let (lua, engine) = start_play();
        let reflection = engine.reflection.clone();
        let workspace = engine.world().workspace;
        let folder_class = reflection.class_id("Folder").unwrap();
        let part_class = reflection.class_id("Part").unwrap();
        let anchored_prop = reflection.name_id("Anchored").unwrap();
        let cframe_prop = reflection.name_id("CFrame").unwrap();
        let (folder, parts) = {
            let mut state = engine.state.borrow_mut();
            let folder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
            let mut parts = Vec::with_capacity(60_000);
            for i in 1..=60_000usize {
                let part = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
                crate::instance::raw_set_silent(&mut state, &reflection, part, anchored_prop, crate::value::Value::Bool(true));
                let pos = if non_finite {
                    rbx_dom_weak::types::Vector3::new(if i.is_multiple_of(2) { f32::NAN } else { f32::INFINITY }, 0.0, 0.0)
                } else {
                    rbx_dom_weak::types::Vector3::new(((i % 200) * 3) as f32, ((i / 200) * 3) as f32, 0.0)
                };
                let cf = rbx_dom_weak::types::CFrame::new(pos, rbx_dom_weak::types::Matrix3::identity());
                crate::instance::raw_set_silent(&mut state, &reflection, part, cframe_prop, crate::value::Value::Dom(Rc::new(rbx_dom_weak::types::Variant::CFrame(cf))));
                state.dom.attach(part, folder);
                parts.push(part);
            }
            (folder, parts)
        };
        engine_lua::instance::set_parent(&engine, folder, Some(workspace)).expect("folder in workspace");
        {
            let shared: LuaTable = engine.lua.borrow().shared.as_ref().expect("shared").get("shared").expect("shared");
            shared.set("Folder", engine_lua::instance_value(&lua, &engine, folder).unwrap()).unwrap();
        }
        run(
            &lua,
            &engine,
            r##"
            function shared.Query()
                return #workspace:GetPartBoundsInBox(CFrame.new(0, -500, 0), Vector3.one)
            end
            assert(shared.Query() == 0)
            "##,
        );
        let query = compile(&lua, &engine, "assert(shared.Query() == 0)");
        let destroy = compile(&lua, &engine, "shared.Folder:Destroy()");
        (lua, engine, folder, parts, query, destroy)
    };
    let mut host = [0.0; 2];
    for _ in 0..3 {
        let scenes = [make_scene(false), make_scene(true)];
        host = [0.0; 2];
        // (the host time of each query that takes a change, the two scenes in turn per change)
        for step in 0..5 {
            for (index, (lua, engine, folder, parts, query, destroy)) in scenes.iter().enumerate() {
                let workspace = engine.world().workspace;
                match step {
                    0 | 1 => {
                        let transparency = engine.reflection.name_id("Transparency").unwrap();
                        let value = crate::value::Value::Number(if step == 0 { 0.25 } else { 0.5 });
                        let hooks = engine.hooks.borrow();
                        let mut state = engine.state.borrow_mut();
                        for &part in parts {
                            crate::instance::raw_set(&mut state, &engine.reflection, &hooks, part, transparency, value.clone());
                        }
                    }
                    2 => engine_lua::instance::set_parent(engine, *folder, None).expect("out"),
                    3 => engine_lua::instance::set_parent(engine, *folder, Some(workspace)).expect("back"),
                    _ => call(lua, engine, destroy, ()),
                }
                let start = std::time::Instant::now();
                call(lua, engine, query, ());
                host[index] += start.elapsed().as_secs_f64();
                let count = super::query::index_len(engine);
                let held = if step == 2 || step == 4 { 0 } else { 60_000 };
                assert!(count == held, "non_finite={}, step {step}: {count} parts held", index == 1);
            }
        }
        for (_, engine, ..) in &scenes {
            super::query::index_matches_a_walk(engine).unwrap();
        }
        if host[1] < 3.0 * host[0] + 0.1 {
            break;
        }
    }
    let [tree, loose] = host;
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
    let reflection = engine.reflection.clone();
    let folder_class = reflection.class_id("Folder").unwrap();
    let part_class = reflection.class_id("Part").unwrap();
    let anchored_prop = reflection.name_id("Anchored").unwrap();
    let cframe_prop = reflection.name_id("CFrame").unwrap();
    let workspace = engine.world().workspace;
    let (holders, folders) = {
        let mut state = engine.state.borrow_mut();
        let mut holders = Vec::with_capacity(500);
        for _ in 0..500 {
            holders.push(crate::instance::create_named(&mut state, &reflection, folder_class, "Folder"));
        }
        let mut folders = Vec::with_capacity(500_000);
        for i in 0..500_000usize {
            let folder = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
            state.dom.attach(folder, holders[i % 500]);
            folders.push(folder);
        }
        (holders, folders)
    };
    for &holder in &holders {
        engine_lua::instance::set_parent(&engine, holder, Some(workspace)).expect("holder in workspace");
    }
    run(
        &lua,
        &engine,
        r##"
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
        function shared.QueryEmpty()
            assert(#workspace:GetPartBoundsInBox(CFrame.new(0, -500, 0), Vector3.one) == 0)
        end
        shared.Rounds(10)
        "##,
    );
    let rounds = compile(&lua, &engine, "local Count = ...; shared.Rounds(Count)");
    let query_empty = compile(&lua, &engine, "shared.QueryEmpty()");
    let time = |count: usize| {
        let start = std::time::Instant::now();
        call(&lua, &engine, &rounds, count);
        start.elapsed().as_secs_f64()
    };
    let before = time(30_000);
    let walked = walks();
    for (i, &folder) in folders.iter().enumerate() {
        engine_lua::instance::set_parent(&engine, folder, Some(holders[(i + 1) % 500])).expect("burst move");
    }
    call(&lua, &engine, &query_empty, ());
    call(&lua, &engine, &rounds, 10usize);
    assert_eq!(walks(), walked, "the burst came from the log");
    // Retry up to 3 times so a transient preemption on a loaded host does not flake; a retained
    // bloated table stays bloated across all 3 tries.
    let mut after = 0.0;
    for _ in 0..3 {
        after = time(30_000);
        if after < 2.0 * before + 0.1 {
            break;
        }
    }
    eprintln!("30,000 rounds took {before:.3} s of host time before the burst, {after:.3} s after it");
    assert!(after < 2.0 * before + 0.1, "30,000 rounds of one move and one query took {after:.3} s of host time after 500,000 folders moved, {before:.3} s before");
    let store = {
        let mut state = engine.state.borrow_mut();
        let store = crate::instance::create_named(&mut state, &reflection, folder_class, "Folder");
        for i in 1..=50_000usize {
            let part = crate::instance::create_named(&mut state, &reflection, part_class, "Part");
            crate::instance::raw_set_silent(&mut state, &reflection, part, anchored_prop, crate::value::Value::Bool(true));
            let cf = rbx_dom_weak::types::CFrame::new(
                rbx_dom_weak::types::Vector3::new(((i % 100) * 3) as f32, -1000.0, ((i / 100) * 3) as f32),
                rbx_dom_weak::types::Matrix3::identity(),
            );
            crate::instance::raw_set_silent(&mut state, &reflection, part, cframe_prop, crate::value::Value::Dom(Rc::new(rbx_dom_weak::types::Variant::CFrame(cf))));
            state.dom.attach(part, store);
        }
        store
    };
    engine_lua::instance::set_parent(&engine, store, Some(workspace)).expect("store in workspace");
    call(&lua, &engine, &query_empty, ());
    call(&lua, &engine, &rounds, 4usize);
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
    }
}

/// The legacy motors (world/joints.rs legacy_angle, step_motors; Studio 0.747, a playing client and its server,
/// tools/parity/probes/part_rules.luau and rigid_joints.luau): CurrentAngle turns Part1 about C0's Z at once (from Part1's side the
/// same way), each simulation step turns it toward DesiredAngle in quarter steps of MaxVelocity (none while less than a quarter step
/// is left; a NaN turns it down), a Motor6D's Transform once written replaces its angle, a Position write's C1 keeps the turn, and a
/// loose motor does not turn.
#[test]
fn legacy_motors_turn_toward_their_desired_angle() {
    let (lua, engine) = start_play();
    run(
        &lua,
        &engine,
        r#"
        workspace.Gravity = 0
        local function Part(Name, Anchored, Size, At)
            local P = Instance.new("Part")
            P.Name, P.Anchored, P.CanCollide, P.Size, P.CFrame = Name, Anchored, false, Size, At
            P.Parent = workspace
            return P
        end
        local function Near(A, B, Case)
            assert(A:FuzzyEq(B, 1e-5), `{Case}: {A}, the client's {B}`)
        end
        local function Is(Got, Want, Case)
            assert(Got == Want, `{Case}: {Got}, the client's {Want}`)
        end
        local Base = Part("Base", true, Vector3.new(2, 2, 2), CFrame.new(0, 300, 0))
        local Arm = Part("Arm", false, Vector3.new(1, 1, 4), CFrame.new(0, 300, 3))
        local M = Instance.new("Motor")
        M.Part0, M.Part1, M.C0, M.C1 = Base, Arm, CFrame.new(0, 0, 1.5), CFrame.new(0, 0, -1.5)
        M.Parent = Base
        M.CurrentAngle = 0.25
        Near(Arm.CFrame, Base.CFrame * CFrame.new(0, 0, 1.5) * CFrame.Angles(0, 0, 0.25) * CFrame.new(0, 0, 1.5), "a CurrentAngle write")
        M.DesiredAngle = 1
        task.wait()
        Is(M.CurrentAngle, 0.25, "no MaxVelocity, no turn")
        -- (steps of a quarter of 0.25 land on 1 exactly)
        M.MaxVelocity = 0.25
        task.wait()
        Is(M.CurrentAngle, 0.5, "a frame turns MaxVelocity")
        for _ = 1, 3 do
            task.wait()
        end
        Is(M.CurrentAngle, 1, "it reaches DesiredAngle")
        Near(Arm.CFrame, Base.CFrame * CFrame.new(0, 0, 1.5) * CFrame.Angles(0, 0, 1) * CFrame.new(0, 0, 1.5), "turned there")
        M.DesiredAngle = 1.05
        task.wait()
        Is(M.CurrentAngle, 1, "less than a quarter step left")
        M.MaxVelocity, M.DesiredAngle = 0.1, 0 / 0
        task.wait()
        assert(math.abs(M.CurrentAngle - 0.9) < 1e-6, `a NaN turns it down: {M.CurrentAngle}`)
        -- a Position write of Part1 rewrites C1 keeping the turn
        M.MaxVelocity = 0
        Arm.Position += Vector3.new(0, 0, 4)
        Near(Arm.CFrame, Base.CFrame * M.C0 * CFrame.Angles(0, 0, M.CurrentAngle) * M.C1:Inverse(), "a Position write")
        -- Part0 free, Part1 anchored: Part0 = Part1 * C1 * Rz(angle) * C0:Inverse()
        local Free, Fixed = Part("Free", false, Vector3.new(2, 2, 2), CFrame.new(10, 300, 0)), Part("Fixed", true, Vector3.new(1, 1, 4), CFrame.new(10, 300, 3))
        local R = Instance.new("Motor")
        R.Part0, R.Part1, R.C0, R.C1 = Free, Fixed, CFrame.new(0, 0, 1.5), CFrame.new(0, 0, -1.5)
        R.Parent = Free
        R.CurrentAngle = 0.7
        Near(Free.CFrame, Fixed.CFrame * CFrame.new(0, 0, -1.5) * CFrame.Angles(0, 0, 0.7) * CFrame.new(0, 0, -1.5), "from Part1's side")
        -- a Motor6D: CurrentAngle until its Transform is written
        local Arm2 = Part("Arm2", false, Vector3.new(1, 1, 4), CFrame.new(0, 300, 3))
        local M6 = Instance.new("Motor6D")
        M6.Part0, M6.Part1, M6.C0, M6.C1 = Base, Arm2, CFrame.new(0, 0, 1.5), CFrame.new(0, 0, -1.5)
        M6.Parent = Base
        M6.CurrentAngle = 0.4
        Near(Arm2.CFrame, Base.CFrame * CFrame.new(0, 0, 1.5) * CFrame.Angles(0, 0, 0.4) * CFrame.new(0, 0, 1.5), "a Motor6D's angle")
        M6.Transform = CFrame.Angles(0.3, 0, 0)
        task.wait()
        M6.Transform = CFrame.new()
        task.wait()
        M6.CurrentAngle = 0.6
        Near(Arm2.CFrame, Base.CFrame * CFrame.new(0, 0, 3), "its Transform alone once written")
        -- a loose motor keeps its angle
        local Loose = Instance.new("Motor")
        Loose.MaxVelocity, Loose.DesiredAngle = 0.1, 1
        task.wait()
        Is(Loose.CurrentAngle, 0, "a loose motor")
        "#,
    );
}
