//! Counting-allocator integration tests: loopback pair idle allocations and memory footprint (src/loopback) alongside
//! world spatial query and BulkMoveTo peak heap bounds (src/world). Both suites count their own thread's allocations
//! and live/peak heap bytes through one thread-local global allocator so the engine test binary links once.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

// (the engine's service code calls mimalloc's collection, which the CLI links; the heap here is the counting one below)
extern crate mimalloc;

struct Counting;

thread_local! {
    static COUNT: Cell<u64> = const { Cell::new(0) };
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
        let _ = COUNT.try_with(|count| count.set(count.get() + 1));
        grow(layout.size() as i64);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        grow(-(layout.size() as i64));
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let _ = COUNT.try_with(|count| count.set(count.get() + 1));
        grow(size as i64 - layout.size() as i64);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocations() -> u64 {
    COUNT.with(Cell::get)
}

fn live() -> i64 {
    LIVE.with(Cell::get)
}

/// How far the heap rose above where it stood before `work`
fn peak_during(work: impl FnOnce()) -> i64 {
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    work();
    PEAK.with(Cell::get) - before
}

mod loopback_alloc {
    use roblox_engine_emulator::{Emulator, Engine, Options, RunContext, loopback, mount::ProjectSpec};

    use super::{allocations, live};

    fn options(context: RunContext) -> Options {
        Options {
            mode: "play".into(),
            run_context: context,
            local_player_name: if context == RunContext::Server { None } else { Some("Player".into()) },
            ..Options::default()
        }
    }

    fn plain(context: RunContext) -> Emulator {
        Emulator::start(Engine::new(options(context)).unwrap(), &ProjectSpec::default()).unwrap()
    }

    fn pair() -> Emulator {
        let client = loopback::start(&Engine::new, &options(RunContext::Loopback), &ProjectSpec::default(), loopback::Spec::default(), &|_, _| {}).unwrap();
        loopback::join(&client.lua, &client.engine).unwrap();
        client
    }

    /// Allocations over `frames` idle frames of `emulator`, after it settled
    fn idle_allocations(emulator: &Emulator, frames: u32) -> u64 {
        for _ in 0..50 {
            emulator.step(1.0 / 60.0).unwrap();
        }
        let before = allocations();
        for _ in 0..frames {
            emulator.step(1.0 / 60.0).unwrap();
        }
        allocations() - before
    }

    #[test]
    fn an_idle_loopback_frame_allocates_nothing_of_its_own() {
        let (server, client, pair) = (plain(RunContext::Server), plain(RunContext::Client), pair());
        let (server, client, pair) = (idle_allocations(&server, 100), idle_allocations(&client, 100), idle_allocations(&pair, 100));
        eprintln!("allocations over 100 idle frames: server {server}, client {client}, pair {pair}");
        assert!(pair <= server + client, "the pair allocated {pair} where its engines alone allocate {server} + {client}");
    }

    /// The memory rows of the loopback bench: the second engine's heap, what the join's copy holds per instance
    #[test]
    #[ignore]
    fn loopback_memory() {
        // (warm the process: the reflection, the engine Luau's bytecode)
        drop(pair());
        let before = live();
        let server = plain(RunContext::Server);
        let one = live() - before;
        drop(server);
        let before = live();
        let both = pair();
        let two = live() - before;
        eprintln!("| the second engine (and the link) | {:.2} MB (a server alone {:.2} MB) | <= 2 MB |", (two - one) as f64 / 1e6, one as f64 / 1e6);
        drop(both);
        // the join's copy: a server with 10,000 parts in ReplicatedStorage, joined
        let client = loopback::start(&Engine::new, &options(RunContext::Loopback), &ProjectSpec::default(), loopback::Spec::default(), &|_, _| {}).unwrap();
        client.engine.run_on_server("Parts", "local F = Instance.new('Folder') for I = 1, 10000 do local P = Instance.new('Part') P.Name = 'P' .. I P.Parent = F end F.Parent = game:GetService('ReplicatedStorage')").unwrap();
        let (before, instances) = (live(), client.engine.state.borrow().dom.live_count());
        loopback::join(&client.lua, &client.engine).unwrap();
        let (copy, made) = (live() - before, client.engine.state.borrow().dom.live_count() - instances);
        assert!(made >= 10_000, "the join made {made} instances");
        // (against the same parts cloned on the server: what Roblox's own copy of an instance costs here)
        let before = live();
        client.engine.run_on_server("Clone", "_G.Kept = game:GetService('ReplicatedStorage').Folder:Clone()").unwrap();
        let cloned = live() - before;
        eprintln!("| the join's copy, per made instance (10,000 parts, {made} made) | {:.0} B (Clone {:.0} B) | <= 400 B |", copy as f64 / made as f64, cloned as f64 / 10_001.0);
    }
}

mod query_memory {
    use roblox_engine_emulator::{Emulator, Engine, Options, lua::globals::CallOutcome, mount::ProjectSpec};

    use super::peak_during;

    /// Runs `source` on `emulator`; panics with the script's error
    fn run(emulator: &Emulator, name: &str, source: &str) {
        if let CallOutcome::Failed(message, trace) = emulator.run(name, source).unwrap().outcome().unwrap() {
            panic!("{name} failed: {message}\n{}", trace.unwrap_or_default());
        }
    }

    /// GetPartBoundsInBox, GetPartsObscuringTarget and a Raycast over 2,000 parts in a folder at the bottom of a 10,000-deep chain
    /// hold about what they hold over the same parts in a folder of the Workspace (a few hundred kilobytes: the result tables and
    /// the walks' parts), and list them in the same order. Each part's whole list of places, 10,001 of them, took the heap 262 MB
    /// higher here (2 GB for 10,000 parts) when the queries put their parts in the Workspace's order.
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
            -- (at multiples of the top level's cells, so that both scenes' walks meet the same cells)
            shared.Scenes = { Flat = Scene(0, 0), Deep = Scene(1024, 10000) }
            function shared.Query(Name)
                local S = shared.Scenes[Name]
                local Found = workspace:GetPartBoundsInBox(CFrame.new(S.X, 50, -10), Vector3.new(0.5, 0.5, 21))
                assert(#Found == 2000, `the box found {#Found}`)
                local Order = table.create(2000)
                for I, Part in Found do
                    Order[I] = table.find(S.Parts, Part)
                end
                S.Order = table.concat(Order, " ")
                local Other = shared.Scenes[if Name == "Flat" then "Deep" else "Flat"]
                assert(Other.Order == nil or Other.Order == S.Order, "the parts in the same order however deep")
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
}
