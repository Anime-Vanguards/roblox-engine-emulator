//! world/parts.rs and the legacy motors (world/joints.rs) as a Studio 0.747 playing client answers them
//! (tools/parity/probes/part_joints.luau, part_methods.luau, model_methods.luau, legacy_motors.luau).

use std::rc::Rc;

use mlua::Lua;

use crate::{Engine, Options, RunContext, lua as engine_lua};

use super::super::tests::{run, start_play};

/// The helpers each script below starts with: parts in the Workspace (or none), joints, names and an assert that prints both sides.
const HELPERS: &str = r#"
    workspace.Gravity = 0
    local Spot = 0
    local function Part(Name, Anchored, Size, Parent)
        Spot += 1
        local P = Instance.new("Part")
        P.Name, P.Anchored, P.CanCollide, P.CanTouch, P.CanQuery = Name, Anchored or false, false, false, false
        P.Size = Size or Vector3.one
        P.CFrame = CFrame.new(300 + 3 * Spot, 60, 40)
        if Parent ~= "none" then
            P.Parent = Parent or workspace
        end
        return P
    end
    local function Join(Kind, A, B, Name)
        local W = Instance.new(Kind)
        W.Name = Name or (A.Name .. B.Name)
        W.Part0, W.Part1 = A, B
        W.Parent = A
        return W
    end
    local function Names(List)
        local Out = {}
        for _, Each in List do
            table.insert(Out, Each.Name)
        end
        return table.concat(Out, ",")
    end
    local function Is(Got, Want, Case)
        assert(Got == Want, `{Case}: {Got}, the client's {Want}`)
    end
    local function Raises(F, Want, Case)
        local Ok, Error = pcall(F)
        assert(not Ok and string.find(tostring(Error), Want, 1, true), `{Case}: {Ok} {Error}, the client raises {Want}`)
    end
"#;

fn check_play(source: &str) {
    let (lua, engine) = start_play();
    run(&lua, &engine, &format!("{HELPERS}\n{source}"));
}

fn start_server() -> (Lua, Rc<Engine>) {
    let engine = Engine::new(Options { mode: "play".into(), run_context: RunContext::Server, ..Options::default() }).expect("engine");
    let lua = Lua::new();
    engine_lua::start(&lua, &engine).expect("start");
    (lua, engine)
}

/// GetRootPart is AssemblyRootPart; GetConnectedParts() the part and each of its joints' parts (one for each joint), GetConnectedParts(true)
/// a free assembly's tree from its root (joints touching the better ranked part first, then the newer; each part before its children,
/// the newest joint's child first) and a component holding an anchored part a stack walk from the part; outside every world the part
/// alone and an empty list. Each line is a 0.747 client's (and its server's) answer.
#[test]
fn connected_parts_list_and_walk_as_a_client_does() {
    check_play(
        r#"
        local Out = Part("Out", false, nil, "none")
        Is(Out:GetRootPart(), nil, "out root")
        Is(Names(Out:GetConnectedParts()), "Out", "out")
        Is(Names(Out:GetConnectedParts(true)), "", "out recursive")
        local A, B, C, D = Part("A"), Part("B"), Part("C"), Part("D")
        Join("Weld", A, B) Join("Weld", B, C) Join("Weld", C, D)
        Is(A:GetRootPart().Name, "D", "chain root")
        Is(Names(B:GetConnectedParts()), "B,A,C", "chain B")
        Is(Names(A:GetConnectedParts(true)), "D,C,B,A", "chain from A")
        Is(Names(A:GetConnectedParts(1)), "D,C,B,A", "a number is a flag")
        Raises(function() A:GetConnectedParts("x") end, "Unable to cast string to bool", "a string")
        Raises(function() A:GetConnectedParts({}) end, "Unable to cast Array to bool", "a table")
        Join("WeldConstraint", A, B, "AB2")
        Is(Names(A:GetConnectedParts()), "A,B,B", "two joints to one part")
        -- free graphs, H the biggest: rings drop their oldest joint not touching the root, squares the older of the two far ones
        local function Graph(Parts, Edges)
            local By = {}
            for Name in string.gmatch(Parts, "%S+") do
                By[Name] = Part(Name, false, if Name == "H" then Vector3.new(3, 3, 3) else nil)
            end
            for Pair in string.gmatch(Edges, "%S+") do
                Join("Weld", By[string.sub(Pair, 1, 1)], By[string.sub(Pair, 2, 2)])
            end
            return By
        end
        Is(Names(Graph("H A B C D E", "HA AB HC CD DE BE").H:GetConnectedParts(true)), "H,C,D,E,B,A", "ring")
        Is(Names(Graph("H A X B", "HA HB AX BX").H:GetConnectedParts(true)), "H,B,X,A", "square")
        Is(Names(Graph("H A X B", "HA HB BX AX").H:GetConnectedParts(true)), "H,B,A,X", "square, the far welds the other way")
        Is(Names(Graph("H A B C D", "HA HB BC BD DA").H:GetConnectedParts(true)), "H,B,C,A,D", "chord")
        Is(Names(Graph("H A B C D E F", "HA HB AC AD BE EF").A:GetConnectedParts(true)), "H,B,E,F,A,D,C", "tree")
        -- an anchored component: a stack walk from the part
        local X = Part("X", true)
        local XA, XB, XC, XD = Part("XA"), Part("XB"), Part("XC"), Part("XD")
        Join("Weld", X, XA) Join("Weld", X, XB) Join("Weld", XA, XC) Join("Weld", XB, XD)
        Is(Names(X:GetConnectedParts(true)), "X,XA,XB,XD,XC", "anchored tree from its root")
        Is(Names(XC:GetConnectedParts(true)), "XC,XA,X,XB,XD", "anchored tree from a leaf")
        "#,
    );
}

/// GetJoints is the part's one list of active joints: a joint goes to its end when it becomes active and leaves by the last one
/// taking its place; disabled ones, ones outside the world or with Part1 nil, and WeldConstraints in JointsService are left out.
#[test]
fn get_joints_keeps_a_clients_list() {
    check_play(
        r#"
        local A, B, C, D = Part("A"), Part("B"), Part("C"), Part("D")
        local J1 = Join("Weld", A, B, "J1")
        local J2 = Join("WeldConstraint", A, D, "J2")
        local J3 = Join("Motor6D", C, A, "J3")
        local J4 = Join("Weld", A, C, "J4")
        Is(Names(A:GetJoints()), "J1,J2,J3,J4", "made")
        J1.Enabled = false
        Is(Names(A:GetJoints()), "J4,J2,J3", "the first disabled")
        Is(Names(A:GetConnectedParts()), "A,C,D,C", "its parts")
        J1.Enabled = true
        Is(Names(A:GetJoints()), "J4,J2,J3,J1", "enabled again")
        J2.Parent = game:GetService("JointsService")
        Is(Names(A:GetJoints()), "J4,J1,J3", "a WeldConstraint in JointsService")
        J4.Parent = game:GetService("JointsService")
        Is(Names(A:GetJoints()), "J4,J1,J3", "a Weld in JointsService keeps its place")
        local Rs = Instance.new("Weld")
        Rs.Part0, Rs.Part1, Rs.Parent = A, B, game:GetService("ReplicatedStorage")
        Is(Names(A:GetJoints()), "J4,J1,J3", "a weld out of the world")
        "#,
    );
}

/// JointInstance.Active and WeldConstraint.Active: an edge of the assembly's tree (the Weld kinds first, then WeldConstraint, Motor and
/// Motor6D, then the joints touching the better ranked part, then the newer).
#[test]
fn active_marks_the_joints_of_an_assemblys_tree() {
    check_play(
        r#"
        local H, A, B, C = Part("H", false, Vector3.new(3, 3, 3)), Part("A"), Part("B"), Part("C")
        local HA, AB, BC, CH = Join("Weld", H, A), Join("Weld", A, B), Join("Weld", B, C), Join("Weld", C, H)
        Is(`{HA.Active} {AB.Active} {BC.Active} {CH.Active}`, "true false true true", "ring")
        HA.Enabled = false
        Is(`{HA.Active} {AB.Active} {BC.Active} {CH.Active}`, "false true true true", "ring cut")
        local P, Q = Part("P"), Part("Q")
        local W, WC, M6, Snap = Join("Weld", P, Q, "W"), Join("WeldConstraint", P, Q, "WC"), Join("Motor6D", Q, P, "M6"), Join("Snap", P, Q, "Snap")
        Is(`{W.Active} {WC.Active} {M6.Active} {Snap.Active}`, "false false false true", "parallel")
        Snap.Enabled = false
        W.Parent = nil
        Is(`{WC.Active} {M6.Active}`, "true false", "a WeldConstraint before a Motor6D")
        local X, U, Y = Part("X", true), Part("U"), Part("Y", true)
        local XU, UY = Join("Weld", X, U), Join("Weld", U, Y)
        Is(`{XU.Active} {UY.Active}`, "false true", "between two anchored parts")
        local Both = Join("Weld", X, Y)
        Is(Both.Active, false, "two anchored parts")
        local Loose = Instance.new("Weld")
        Is(Loose.Active, false, "a loose weld")
        "#,
    );
}

/// The Model methods: GetModelCFrame and GetModelSize are the bounding box (about the pivot itself), a primary-less model's unset
/// pivot is oriented as the part covering the most ground, MoveTo and move keep the rotation and store the pivot measured, and a
/// second PivotTo to the same target changes nothing.
#[test]
fn model_methods_answer_as_a_client_does() {
    check_play(
        r#"
        local Turn1, Turn2 = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7), CFrame.fromEulerAnglesXYZ(-0.2, 1.1, 0.4)
        local function Model(Primary)
            local M = Instance.new("Model")
            local A = Part("A", true, Vector3.new(2, 4, 6), M)
            A.CFrame = CFrame.new(700, 60, 40) * Turn1
            local B = Part("B", true, Vector3.new(1, 1, 1), M)
            B.CFrame = CFrame.new(705, 62, 41)
            if Primary then
                M.PrimaryPart = A
            end
            M.Parent = workspace
            return M, A, B
        end
        local M, A, B = Model(true)
        Is(M:GetModelCFrame(), (M:GetBoundingBox()), "GetModelCFrame")
        Is(M:GetModelSize(), M:GetExtentsSize(), "GetModelSize")
        M:move(Vector3.new(710, 70, 50))
        Is(A.CFrame, CFrame.new(710, 70, 50) * Turn1.Rotation, "move keeps the rotation")
        Is(B.Position, Vector3.new(715, 72, 51), "the other part translated")
        local N, NA = Model(false)
        Is(N:GetPivot().Rotation, NA.CFrame.Rotation, "the unset pivot turned as the part covering the most ground")
        local Old = N:GetPivot().Position
        N:MoveTo(Old + Vector3.new(10, 0, 0))
        assert(N:GetPivot().Position:FuzzyEq(Old + Vector3.new(20, 0, 0), 1e-3), `an unset pivot moves twice: {N:GetPivot().Position}`)
        local Q = Instance.new("Model")
        local Wide = Part("Wide", true, Vector3.new(3, 3, 3), Q)
        Wide.CFrame = CFrame.new(800, 50, 40) * Turn1
        local Big = Part("Big", true, Vector3.new(4, 4, 4), Q)
        Big.CFrame = CFrame.new(810, 50, 40) * Turn2
        Q.Parent = workspace
        Is(Q:GetPivot().Rotation, Big.CFrame.Rotation, "the bigger footprint")
        -- PivotTo: each part at Target * Pivot:ToObjectSpace(it), and again to the same target nothing moves
        local P, PA, PB = Model(true)
        local Pivot = P:GetPivot()
        local Target = CFrame.new(10.1, 20.2, 30.3) * CFrame.Angles(1, 2, 3)
        local WantA, WantB = Target * Pivot:ToObjectSpace(PA.CFrame), Target * Pivot:ToObjectSpace(PB.CFrame)
        P:PivotTo(Target)
        Is(PA.CFrame, WantA, "the primary part")
        Is(PB.CFrame, WantB, "the other part")
        P:PivotTo(Target)
        Is(PA.CFrame, WantA, "the primary part again")
        Is(PB.CFrame, WantB, "the other part again")
        -- an obstacle: MoveTo stands the model on it
        local Floor = Part("Floor", true, Vector3.new(20, 2, 20))
        Floor.CFrame, Floor.CanCollide = CFrame.new(900, 60, 40), true
        local O = Instance.new("Model")
        local OA = Part("OA", true, Vector3.new(2, 2, 2), O)
        OA.CFrame, OA.CanCollide = CFrame.new(1000, 60, 40), true
        O.Parent = workspace
        O:MoveTo(Vector3.new(900, 60, 40))
        Is(OA.Position, Vector3.new(900, 62, 40), "on the floor")
        for _, Name in { "SetIdentityOrientation", "ResetOrientationToIdentity" } do
            local Before = PA.CFrame
            P[Name](P)
            Is(PA.CFrame, Before, Name)
        end
        Is(#P:GetPersistentPlayers(), 0, "no persistent players")
        Raises(function() P:AddPersistentPlayer(game:GetService("Players").LocalPlayer) end, "PersistentPerPlayer API can only be called from the Server.", "a client")
        Raises(function() P:AddPersistentPlayer() end, "PersistentPerPlayer player not specified.", "no player")
        "#,
    );
}

/// The part methods that read the world: IsGrounded, GetRenderCFrame, CanCollideWith, GetClosestPointOnSurface, GetVelocityAtPosition,
/// the torque conversions, Resize, BreakJoints, MakeJoints, GetNoCollisionConstraints, the attachment methods and, on a client, the
/// network ownership refusals.
#[test]
fn part_methods_answer_as_a_client_does() {
    check_play(
        r#"
        local Anchor, Welded, Free = Part("Anchor", true), Part("Welded"), Part("Free")
        Join("Weld", Anchor, Welded)
        Is(`{Anchor:IsGrounded()} {Welded:IsGrounded()} {Free:IsGrounded()} {Part("Out", false, nil, "none"):IsGrounded()}`, "true true false false", "IsGrounded")
        Is(Free:GetRenderCFrame(), Free.CFrame, "GetRenderCFrame")
        Free.CanCollide, Anchor.CanCollide, Welded.CanCollide = true, true, true
        Is(`{Free:CanCollideWith(Anchor)} {Welded:CanCollideWith(Anchor)} {Free:CanCollideWith(Free)}`, "true false false", "CanCollideWith")
        local Apart = Instance.new("NoCollisionConstraint")
        Apart.Part0, Apart.Part1, Apart.Parent = Free, Anchor, Free
        Is(Free:CanCollideWith(Anchor), false, "a NoCollisionConstraint")
        Is(Names(Free:GetNoCollisionConstraints()), "NoCollisionConstraint", "GetNoCollisionConstraints")
        Raises(function() Free:CanCollideWith(Instance.new("Folder")) end, "CanCollideWith expects a BasePart argument.", "a Folder")
        Raises(function() Free:CanCollideWith(Part("Away", false, nil, "none")) end, "CanCollideWith expects both parts to be descendants of the same WorldRoot.", "another world")
        Raises(function() Free:GetNetworkOwner() end, "Network Ownership API can only be called from the Server.", "a client's ownership call")
        local Ok, Why = Free:CanSetNetworkOwnership()
        Is(`{Ok} {Why}`, "false Network Ownership API can only be called from the Server.", "CanSetNetworkOwnership")
        -- the closest point: inside answers the point, outside the box's nearest point
        local Box = Part("Box", true, Vector3.new(2, 4, 6))
        Box.CFrame = CFrame.new(400, 60, 40)
        Is(Box:GetClosestPointOnSurface(Vector3.new(400.3, 60.2, 40.1)), Vector3.new(400.3, 60.2, 40.1), "inside")
        Is(Box:GetClosestPointOnSurface(Vector3.new(410, 70, 40)), Vector3.new(401, 62, 40), "outside")
        -- velocities and torques
        Free.AssemblyLinearVelocity = Vector3.new(-1, 0, 2)
        Is(Free:GetVelocityAtPosition(Vector3.new(1, 2, 3)), Vector3.new(-1, 0, 2), "a velocity without spin")
        local Block = Part("Block", false, Vector3.new(2, 4, 6))
        local Torque = Block:AngularAccelerationToTorque(Vector3.new(1, 2, 3))
        assert(Torque:FuzzyEq(Vector3.new(145.6, 224, 168), 1e-3), `the torque {Torque}`)
        Is(Anchor:AngularAccelerationToTorque(Vector3.one).X, math.huge, "an anchored assembly")
        Is(Anchor:TorqueToAngularAcceleration(Vector3.one), Vector3.zero, "an anchored assembly's acceleration")
        -- Resize: the face moves out, the opposite one stays; blocked by a part that can collide
        local Grown = Part("Grown", true, Vector3.new(4, 1, 4))
        Grown.CFrame = CFrame.new(600, 60, 40)
        Is(Grown:Resize(Enum.NormalId.Top, 2), true, "Resize")
        Is(`{Grown.Size} {Grown.Position}`, `{Vector3.new(4, 3, 4)} {Vector3.new(600, 61, 40)}`, "its size and place")
        local Lid = Part("Lid", true, Vector3.new(4, 1, 4))
        Lid.CFrame, Lid.CanCollide, Grown.CanCollide = CFrame.new(600, 63.5, 40), true, true
        Is(Grown:Resize(Enum.NormalId.Top, 3), false, "blocked")
        Is(Grown.Size, Vector3.new(4, 3, 4), "unchanged")
        -- BreakJoints: the part's rigid joints, MakeJoints nothing
        local H, L1, L2 = Part("H"), Part("L1"), Part("L2")
        local W1 = Join("Weld", H, L1)
        local WC = Join("WeldConstraint", H, L2)
        local Off = Join("Weld", H, L2, "Off")
        Off.Enabled = false
        H:BreakJoints()
        Is(`{W1.Parent} {WC.Parent} {Off.Parent}`, "nil nil H", "BreakJoints")
        H:MakeJoints()
        Is(#H:GetJoints(), 0, "MakeJoints")
        -- Motor:SetDesiredAngle writes DesiredAngle, a float argument
        local Motor = Instance.new("Motor")
        Motor:SetDesiredAngle("2")
        Is(Motor.DesiredAngle, 2, "SetDesiredAngle")
        Raises(function() Motor:SetDesiredAngle({}) end, "Unable to cast Array to float", "a table")
        Raises(function() Motor:SetDesiredAngle() end, "Argument 1 missing or nil", "nothing")
        -- attachments
        local Att, Other = Instance.new("Attachment"), Instance.new("Attachment")
        Att.Parent, Other.Parent = H, L1
        Is(Att:GetAxis(), Vector3.new(1, 0, 0), "GetAxis")
        Att:SetAxis(Vector3.new(0, 1, 0))
        Is(`{Att.Axis} {Att:GetSecondaryAxis()}`, `{Vector3.new(0, 1, 0)} {Vector3.new(-1, 0, 0)}`, "SetAxis")
        Raises(function() Att:SetAxis("x") end, "Unable to cast string to Vector3", "SetAxis a string")
        local Rope = Instance.new("RopeConstraint")
        Rope.Attachment0, Rope.Attachment1, Rope.Parent = Other, Att, workspace
        local Hinge = Instance.new("HingeConstraint")
        Hinge.Attachment0, Hinge.Enabled, Hinge.Parent = Att, false, workspace
        local Loose = Instance.new("BallSocketConstraint")
        Loose.Attachment0 = Att
        Is(Names(Att:GetConstraints()), "RopeConstraint,HingeConstraint", "GetConstraints: disabled ones too, none out of the DataModel")
        "#,
    );
}

/// A server's network ownership: nil (the server) and automatic until set; anchored assemblies refused.
#[test]
fn a_server_keeps_network_owners() {
    let (lua, engine) = start_server();
    run(
        &lua,
        &engine,
        &format!(
            "{HELPERS}\n{}",
            r#"
            local Free, Fixed = Part("Free"), Part("Fixed", true)
            Is(`{Free:GetNetworkOwner()} {Free:GetNetworkOwnershipAuto()} {Free:CanSetNetworkOwnership()}`, "nil true true", "unset")
            Free:SetNetworkOwner(nil)
            Is(Free:GetNetworkOwnershipAuto(), false, "set to the server")
            Free:SetNetworkOwnershipAuto()
            Is(Free:GetNetworkOwnershipAuto(), true, "automatic again")
            Raises(function() Fixed:GetNetworkOwner() end, "Network Ownership API cannot be called on Anchored parts or parts welded to Anchored parts.", "anchored")
            Raises(function() Free:SetNetworkOwner("x") end, "Unable to cast value to Object", "a string")
            local Ok, Why = Part("Out", false, nil, "none"):CanSetNetworkOwnership()
            Is(`{Ok} {Why}`, "false Can only call Network Ownership API on a part that is descendent of Workspace", "outside the Workspace")
            Is(#Instance.new("Model"):GetPersistentPlayers(), 0, "a server's persistent players")
            "#
        ),
    );
}
