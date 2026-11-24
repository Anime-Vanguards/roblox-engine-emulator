//! What the world's calls hold while they work: the walks that put what a spatial query keeps in the Workspace's order hold
//! memory for the parts and the instances up to where their paths meet, not for how deep the parts are (src/world/broadphase.rs
//! sort_by_order), and BulkMoveTo keeps room for the parts its list holds, not for its holes (src/world/mod.rs bulk_moves). This
//! binary counts its own thread's heap, and its peak, with a global allocator of its own.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

use roblox_engine_emulator::{Emulator, Engine, Options, lua::globals::CallOutcome, mount::ProjectSpec};

// (the engine's service code calls mimalloc's collection, which the CLI links; the heap here is the counting one below)
extern crate mimalloc;

struct Counting;

thread_local! {
    static LIVE: Cell<i64> = const { Cell::new(0) };
    static PEAK: Cell<i64> = const { Cell::new(0) };
}

fn grow(by: i64) {
    let _ = LIVE.try_with(|live| {
        live.set(live.get() + by);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(live.get())));
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grow(layout.size() as i64);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        grow(-(layout.size() as i64));
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        grow(size as i64 - layout.size() as i64);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `source` on `emulator`; panics with the script's error
fn run(emulator: &Emulator, name: &str, source: &str) {
    if let CallOutcome::Failed(message, trace) = emulator.run(name, source).unwrap().outcome().unwrap() {
        panic!("{name} failed: {message}
{}", trace.unwrap_or_default());
    }
}

/// How far the heap rose above where it stood before `work`
fn peak_during(work: impl FnOnce()) -> i64 {
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    work();
    PEAK.with(Cell::get) - before
}

/// GetPartBoundsInBox, GetPartsObscuringTarget and a Raycast over 2,000 parts in a folder at the bottom of a 10,000-deep chain
/// hold about what they hold over the same parts in a folder of the Workspace (a few hundred kilobytes: the result tables and
/// the walks' nodes), and list them in the folder's order. Each part's whole list of places, 10,001 of them, took the
/// heap 262 MB higher here (2 GB for 10,000 parts).
#[test]
fn a_query_holds_memory_for_its_parts_not_for_how_deep_they_are() {
    let emulator = Emulator::start(Engine::new(Options { mode: "play".into(), ..Options::default() }).unwrap(), &ProjectSpec::default()).unwrap();
    run(
        &emulator,
        "Scene",
            r#"
            local function Scene(X, Depth)
                local Holder = Instance.new("Folder")
                local Parts = {}
                for I = 1, 2000 do
                    local Part = Instance.new("Part")
                    Part.Anchored, Part.Size, Part.CFrame = true, Vector3.new(1, 1, 0.5), CFrame.new(X, 50, -I * 0.01)
                    Part.Parent = Holder
                    Parts[I] = Part
                end
                -- (the chain made from the bottom up)
                local Top = Holder
                for _ = 1, Depth do
                    local Folder = Instance.new("Folder")
                    Top.Parent = Folder
                    Top = Folder
                end
                Top.Parent = workspace
                return { X = X, Parts = Parts }
            end
            shared.Scenes = { Flat = Scene(0, 0), Deep = Scene(1000, 10000) }
            function shared.Query(Name)
                local S = shared.Scenes[Name]
                local Found = workspace:GetPartBoundsInBox(CFrame.new(S.X, 50, -10), Vector3.new(0.5, 0.5, 21))
                assert(#Found == 2000, `the box found {#Found}`)
                for I, Part in Found do
                    assert(Part == S.Parts[I], "the parts in their folder's order")
                end
                workspace.CurrentCamera.CFrame = CFrame.new(S.X, 50, 10)
                assert(#workspace.CurrentCamera:GetPartsObscuringTarget({ Vector3.new(S.X, 50, -25) }, {}) == 2000, "every part obscures the point")
                assert(workspace:Raycast(Vector3.new(S.X, 50, 10), Vector3.new(0, 0, -40)).Instance == S.Parts[1], "the ray hits the first part")
            end
            -- (each scene once, so the index has its parts and the members are warm)
            shared.Query("Flat")
            shared.Query("Deep")
            "#,
    );
    let flat = peak_during(|| {
        run(&emulator, "Flat", "shared.Query(\"Flat\")");
    });
    let deep = peak_during(|| {
        run(&emulator, "Deep", "shared.Query(\"Deep\")");
    });
    eprintln!("peak heap over the queries: {flat} B in a folder of the Workspace, {deep} B 10,000 folders deep");
    assert!(flat < 4_000_000, "the queries in the Workspace rose {flat} B");
    assert!(deep < flat + 1_000_000, "the queries 10,000 folders deep rose {deep} B, {flat} B in the Workspace");
}

/// BulkMoveTo keeps room for the parts its list holds, not for its holes: a list of 1,000,000 holes and one part (and 1,000,001
/// frames) moves that part with the heap a few hundred bytes higher, where room for a move for every entry took 80 MB.
#[test]
fn bulk_move_to_keeps_no_room_for_the_holes_in_its_list() {
    let emulator = Emulator::start(Engine::new(Options { mode: "play".into(), ..Options::default() }).unwrap(), &ProjectSpec::default()).unwrap();
    run(
        &emulator,
        "Lists",
        r#"
        local Part = Instance.new("Part")
        Part.Anchored = true
        local List = table.create(1000001)
        List[1000001] = Part
        shared.Part, shared.List, shared.Frames = Part, List, table.create(1000001, CFrame.new(0, 10, 0))
        workspace:BulkMoveTo(shared.List, shared.Frames)
        "#,
    );
    let peak = peak_during(|| {
        run(&emulator, "Move", "workspace:BulkMoveTo(shared.List, shared.Frames) assert(shared.Part.Position.Y == 10, 'the part moved')");
    });
    eprintln!("peak heap over a BulkMoveTo of 1,000,000 holes and a part: {peak} B");
    assert!(peak < 100_000, "the move rose the heap {peak} B");
}
